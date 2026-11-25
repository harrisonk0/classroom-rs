//! Live desktop transport. The short-lived bearer never enters the cache.

use crate::{
    connect_google,
    desktop_data::{
        Attachment, AttachmentKind, Course, CourseMember, CourseRole, CourseState, CourseTheme,
        Coursework, CourseworkKind, Grade, QuestionAnswer, QuestionDetails, Snapshot, SubmissionStatus, Topic,
    },
    query_courses, wire,
};
use iced::futures::{Stream, StreamExt, stream};
use prost::Message;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, atomic::{AtomicBool, Ordering}},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{io::AsyncWriteExt, process::Command, sync::Mutex, time::timeout};
use tonic::{Code, Status, transport::Channel};
use zeroize::Zeroizing;

mod artwork;
mod assignment_fields;
mod class_archive;
mod class_calendar;
mod class_create;
mod class_notifications;
mod class_order;
mod class_visibility;
mod classwork_order;
mod comment_delete;
mod comments;
mod courses;
mod coursework;
mod drive_files;
mod drive_transfer;
mod email_notifications;
mod enrollment;
mod gradebook;
mod mentions;
mod originality;
mod people;
mod personal_copy;
mod rubric;
mod stream_posts;
mod stream_settings;
mod student_muting;
mod submission_attachments;
mod submissions;
mod teacher_assignment;
mod teacher_submissions;
mod teacher_topics;
mod topic_order;
#[cfg(test)]
mod tests;

pub use artwork::{artwork_updates, load_artworks, profile_picture_updates};
pub use assignment_fields::{
    AssignmentDue, AssignmentFieldChanges, parse_assignment_due, parse_assignment_points,
};
pub use class_archive::{read_class_state, set_class_archived};
pub use class_create::{NewClass, can_create_classes, class_input_valid, create_class};
pub use class_notifications::{load_class_notifications, set_class_notifications};
pub use class_order::{CoursePlacement, reorder_class};
pub use class_visibility::{class_visibility_for, set_class_hidden};
pub use classwork_order::{ClassworkOrderDirection, reorder_classwork};
pub use comment_delete::{
    CommentAudience, check_class_comment_target, delete_own_comment, moderate_class_comment,
};
pub use comments::{load_comments, load_private_feedback, post_class_comment, post_private_feedback};
pub use coursework::{
    AssignmentRefresh, CourseContents, DraftPublication, TeacherDraft, assignment_updates,
    load_coursework, load_teacher_drafts,
};
pub use drive_files::{
    DriveFilePage, DriveFolder, DriveLocation, list_drive_files, lookup_drive_file_link,
    preview_drive_file, save_drive_file,
};
pub use drive_transfer::DriveFileAttachment;
pub use email_notifications::{
    EmailNotificationCategory, EmailNotificationGroup, EmailNotificationSettings,
    EmailNotificationTarget, load_email_notifications, set_email_notification,
};
pub use enrollment::{
    accept_invitation, decline_invitation, join_class, leave_class, load_invitations, valid_class_code,
};
pub use gradebook::{gradebook_coverage, load_course_grade};
pub use mentions::{ClassMention, load_class_mentions};
pub use originality::originality_report_url;
pub use people::{
    OwnClassDetails, UserProfile, check_current_user, load_class_folders, load_class_teacher_names,
    load_own_class_details, load_roster, load_stream_profiles, own_work_link,
};
pub use personal_copy::{PersonalCopyPreparation, check_personal_copy_absent, prepare_personal_copy};
pub use rubric::load_rubric;
pub use stream_posts::{
    PublishedAnnouncement, StreamAccess, check_stream_post_lookup, delete_own_stream_post,
    load_stream_access, load_stream_permission, moderate_stream_post, post_stream_announcement,
    upload_stream_file,
};
pub use stream_settings::{
    ClassDetails, CourseStreamSettings, StreamPostingPolicy, TeacherClassSettings,
    load_teacher_class_settings, save_teacher_class_settings,
};
pub use student_muting::{load_student_muting, set_student_muted};
pub use submission_attachments::{
    DriveDocumentKind, attach_submission_drive_file, attach_submission_file, attach_submission_link,
    create_submission_document, remove_submission_attachment,
};
pub use submissions::{
    ShortAnswerRevision, SubmissionAction, can_turn_in_question, change_submission,
    editable_short_answer, load_own_submission_history, load_submissions,
    question_submission_is_current, turn_in_question,
};
pub use teacher_assignment::{
    AssignmentPublication, assignment_draft_text_valid, change_assignment_publication,
    save_assignment_draft, save_assignment_draft_with_fields,
};
pub use teacher_submissions::{
    TeacherFeedbackKey, TeacherGradebook, TeacherGradebookCell, TeacherGradingAction, TeacherSubmission,
    load_teacher_gradebook, load_teacher_private_feedback, load_teacher_submissions,
    post_teacher_private_feedback, return_teacher_submissions, write_teacher_grade,
};
pub use teacher_topics::{
    TeacherTopicAction, TopicItemTarget, change_classwork_topic, check_item_topic_read,
    load_teacher_topics, mutate_teacher_topic, normalized_topic_name,
};
pub use topic_order::{TopicOrderDirection, reorder_topic};

use comments::{insert_comment, query_submission_feedback};
use courses::{course_details_request, decode_courses};
use coursework::{
    coursework, decode_description_bullets, decode_description_styles, load_topics, material,
    teacher_draft,
};
use drive_transfer::{run_drive_helper, upload_drive_file};
use people::{
    course_member_key, current_user_id, load_user_names, own_class_details, require_class_moderator,
};
use submission_attachments::{link_material, material_matches};
use submissions::{own_question_answer, submission_display_status};

#[derive(Clone)]
pub struct LiveSession {
    pub snapshot: Snapshot,
    pub account: String,
    credentials: Arc<Mutex<SessionCredentials>>,
    needs_sign_in: Arc<AtomicBool>,
    channel: Channel,
}

struct SessionCredentials {
    token: ClassroomToken,
    failure: Option<(Instant, String)>,
}

struct ClassroomToken {
    bearer: Arc<Zeroizing<String>>,
    expires_at: Option<u64>,
}

impl ClassroomToken {
    fn expires_within(&self, time: u64, seconds: u64) -> bool {
        self.expires_at.is_some_and(|expiry| expiry <= time.saturating_add(seconds))
    }

    fn invalidate(&mut self, rejected: &Arc<Zeroizing<String>>) -> bool {
        let rejected_current = Arc::ptr_eq(&self.bearer, rejected);
        if rejected_current {
            self.expires_at = Some(0);
        }
        rejected_current
    }
}

#[derive(Clone, Copy)]
enum SignInMode {
    Interactive,
    Ephemeral,
    SavedOnly,
}

struct SignInFailure {
    message: String,
    needs_sign_in: bool,
}

impl From<String> for SignInFailure {
    fn from(message: String) -> Self {
        Self { message, needs_sign_in: false }
    }
}

impl From<&str> for SignInFailure {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

impl fmt::Debug for LiveSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveSession")
            .field("courses", &self.snapshot.courses.len())
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct SignInReply {
    bearer: Option<String>,
    expires_at: Option<u64>,
    stage: Option<String>,
    reason: Option<String>,
}

struct Runtime {
    python: PathBuf,
    sign_in: PathBuf,
    browsers: PathBuf,
}

impl Runtime {
    fn resolve() -> Result<Self, String> {
        if let Ok(executable) = std::env::current_exe()
            && let Some(contents) = executable.parent().and_then(Path::parent)
        {
            let resources = contents.join("Resources");
            let runtime = Self {
                python: resources.join("Runtime/bin/python3.12"),
                sign_in: resources.join("google_sign_in.py"),
                browsers: resources.join("Browsers"),
            };
            if runtime.is_available() {
                return Ok(runtime);
            }
        }

        let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let configured = std::env::var_os("CLASSROOM_RUNTIME_DIR").map(PathBuf::from);
        let development = crate_root.join("desktop/runtime");
        let legacy = crate_root.parent().and_then(Path::parent).map(|workspace| workspace.join("work/notebooklm-test"));
        for tools in configured.into_iter().chain(Some(development)).chain(legacy) {
            let runtime = Self {
                python: tools.join("venv/bin/python"),
                sign_in: crate_root.join("desktop/google_sign_in.py"),
                browsers: tools.join("browsers"),
            };
            if runtime.is_available() {
                return Ok(runtime);
            }
        }

        Err("The Google sign-in runtime is missing from this build.".into())
    }

    fn is_available(&self) -> bool {
        self.python.is_file() && self.sign_in.is_file() && self.browsers.is_dir()
    }
}

pub async fn sign_in(email: String) -> Result<LiveSession, String> {
    start_session(email, SignInMode::Interactive).await
}

pub async fn sign_in_basic(email: String) -> Result<LiveSession, String> {
    start_session(email, SignInMode::Ephemeral).await
}

pub async fn restore_session(email: String) -> Result<LiveSession, String> {
    start_session(email, SignInMode::SavedOnly).await
}

async fn start_session(email: String, mode: SignInMode) -> Result<LiveSession, String> {
    let account = normalize_account(&email)?;
    let token = issue_token(&account, mode).await.map_err(|error| error.message)?;
    let channel = connect_google().await.map_err(|_| "Couldn't connect to Google Classroom.".to_owned())?;
    let snapshot = match mode {
        SignInMode::Ephemeral => query_courses(channel.clone(), &token.bearer).await
            .map(decode_courses).map_err(|error| course_error(error.code()))?,
        _ => {
            let result = classroom_rpc_routed(&channel, &token.bearer, "QueryCourse", course_details_request(), None)
                .await.map_err(|error| course_error(error.code()))?;
            decode_courses(result)
        }
    };
    if matches!(mode, SignInMode::Interactive) {
        save_account_preference(&account)?;
    }
    Ok(LiveSession {
        snapshot,
        account,
        credentials: Arc::new(Mutex::new(SessionCredentials { token, failure: None })),
        needs_sign_in: Arc::new(AtomicBool::new(false)),
        channel,
    })
}

async fn issue_token(account: &str, mode: SignInMode) -> Result<ClassroomToken, SignInFailure> {
    let runtime = Runtime::resolve()?;
    let flags: &[&str] = match mode {
        SignInMode::Interactive => &[],
        SignInMode::Ephemeral => &["--ephemeral"],
        SignInMode::SavedOnly => &["--saved-only"],
    };
    let prompt_timeout = match mode {
        SignInMode::SavedOnly => Duration::from_secs(35),
        _ => Duration::from_secs(380),
    };
    let mut child = Command::new(runtime.python)
        .arg("-I")
        .arg("-B")
        .arg(runtime.sign_in)
        .args(flags)
        .env("PLAYWRIGHT_BROWSERS_PATH", runtime.browsers)
        .env_remove("DEBUG")
        .env_remove("PWDEBUG")
        .env_remove("SSLKEYLOGFILE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "Couldn't start the isolated Google sign-in window.".to_owned())?;

    let mut input = child.stdin.take().ok_or("Couldn't start the sign-in session.")?;
    input
        .write_all(format!("{account}\n").as_bytes())
        .await
        .map_err(|_| "The sign-in window closed before it was ready.".to_owned())?;
    drop(input);

    let output = timeout(prompt_timeout, child.wait_with_output())
        .await
        .map_err(|_| "Google sign-in timed out. Try again when you're ready.".to_owned())?
        .map_err(|_| "The sign-in window closed before it finished.".to_owned())?;
    let payload = Zeroizing::new(output.stdout);
    if payload.len() > 8192 {
        return Err("Google sign-in returned an unexpected result.".into());
    }
    let result: SignInReply = serde_json::from_slice(payload.as_slice())
        .map_err(|_| "Google sign-in didn't finish. Check the sign-in window and try again.".to_owned())?;
    let bearer = Zeroizing::new(result.bearer.unwrap_or_default());
    if !output.status.success() || bearer.is_empty() {
        let stage = result.stage.as_deref();
        let reason = result.reason.as_deref();
        let needs_sign_in = stage == Some("saved-session") || (
            stage == Some("classroom") && matches!(reason,
                Some("BadAuthentication" | "NeedsBrowser" | "InvalidSecondFactor" | "Unauthorized" | "TokenExpired")
            )
        );
        return Err(SignInFailure { message: sign_in_error(stage, reason), needs_sign_in });
    }
    if bearer.len() > 4096
        || !bearer
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._~+/=".contains(&byte))
    {
        return Err("Google returned an unexpected session credential.".into());
    }

    Ok(ClassroomToken { bearer: Arc::new(bearer), expires_at: result.expires_at })
}

fn normalize_account(email: &str) -> Result<String, String> {
    let account = email.trim().to_ascii_lowercase();
    let valid = account.len() <= 254 && account.split_once('@').is_some_and(|(name, domain)| {
        domain.len() <= 253
            && domain.contains('.')
            && domain.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
            && !name.is_empty()
            && name.len() <= 128
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
    });
    valid
        .then_some(account)
        .ok_or_else(|| "Enter a valid Google account email.".into())
}

pub async fn refresh(session: &LiveSession) -> Result<Snapshot, String> {
    session.read_rpc("QueryCourse", course_details_request()).await.map(decode_courses)
}

fn unix_time() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

const SERVICE: &str = "/homeroom.dataservice.HomeroomDataService/";
const RENEW_AHEAD_SECONDS: u64 = 30;
const RENEWAL_BACKOFF: Duration = Duration::from_secs(15);

impl LiveSession {
    pub fn needs_sign_in(&self) -> bool {
        self.needs_sign_in.load(Ordering::Relaxed)
    }

    pub async fn reconnect(&self) -> Result<(), String> {
        let mut credentials = self.credentials.lock().await;
        let token = issue_token(&self.account, SignInMode::Interactive).await.map_err(|error| error.message)?;
        let _: wire::QueryCourseResponse = classroom_rpc_routed(
            &self.channel, &token.bearer, "QueryCourse", course_details_request(), None,
        ).await.map_err(|error| course_error(error.code()))?;
        credentials.token = token;
        credentials.failure = None;
        self.needs_sign_in.store(false, Ordering::Relaxed);
        Ok(())
    }

    async fn bearer_for_request(
        &self, rejected: Option<&Arc<Zeroizing<String>>>,
    ) -> Result<Arc<Zeroizing<String>>, String> {
        let mut credentials = self.credentials.lock().await;
        if let Some(rejected) = rejected {
            credentials.token.invalidate(rejected);
        }
        let now = unix_time();
        if !credentials.token.expires_within(now, RENEW_AHEAD_SECONDS) {
            return Ok(credentials.token.bearer.clone());
        }
        if let Some((last_attempt, message)) = &credentials.failure
            && (last_attempt.elapsed() < RENEWAL_BACKOFF || self.needs_sign_in())
        {
            return if credentials.token.expires_within(now, 0) {
                Err(message.clone())
            } else {
                Ok(credentials.token.bearer.clone())
            };
        }

        // All clones share the lock, including parallel overview requests.
        match issue_token(&self.account, SignInMode::SavedOnly).await {
            Ok(token) => {
                credentials.token = token;
                credentials.failure = None;
                self.needs_sign_in.store(false, Ordering::Relaxed);
                Ok(credentials.token.bearer.clone())
            }
            Err(error) => {
                if error.needs_sign_in {
                    credentials.token.expires_at = Some(0);
                    self.needs_sign_in.store(true, Ordering::Relaxed);
                }
                credentials.failure = Some((Instant::now(), error.message.clone()));
                if credentials.token.expires_within(unix_time(), 0) {
                    Err(error.message)
                } else {
                    Ok(credentials.token.bearer.clone())
                }
            }
        }
    }

    async fn read_rpc<Q, R>(&self, method: &str, request: Q) -> Result<R, String>
    where
        Q: Message + Default + Send + Sync + Clone + 'static,
        R: Message + Default + Send + Sync + 'static,
    {
        self.rpc(method, request, None).await
    }

    async fn rpc<Q, R>(&self, method: &str, request: Q, course_id: Option<i64>) -> Result<R, String>
    where
        Q: Message + Default + Send + Sync + Clone + 'static,
        R: Message + Default + Send + Sync + 'static,
    {
        let bearer = self.bearer_for_request(None).await?;
        let result = classroom_rpc_routed(&self.channel, &bearer, method, request.clone(), course_id).await;
        match result {
            Ok(response) => return Ok(response),
            Err(error) if error.code() != Code::Unauthenticated => return Err(course_error(error.code())),
            Err(_) => {}
        }

        let bearer = self.bearer_for_request(Some(&bearer)).await?;
        let retried = classroom_rpc_routed(&self.channel, &bearer, method, request, course_id).await;
        if retried.as_ref().is_err_and(|error| error.code() == Code::Unauthenticated) {
            self.note_rejected_bearer(&bearer).await;
        }
        retried.map_err(|status| course_error(status.code()))
    }

    async fn note_rejected_bearer(&self, bearer: &Arc<Zeroizing<String>>) {
        let mut credentials = self.credentials.lock().await;
        if credentials.token.invalidate(bearer) {
            credentials.failure = Some((Instant::now(), course_error(Code::Unauthenticated)));
            self.needs_sign_in.store(true, Ordering::Relaxed);
        }
    }

    async fn write_rpc<Q, R>(&self, method: &str, request: Q, course_id: Option<i64>) -> Result<R, String>
    where
        Q: Message + Default + Send + Sync + 'static,
        R: Message + Default + Send + Sync + 'static,
    {
        let bearer = self.bearer_for_request(None).await?;
        let result = classroom_rpc_routed(&self.channel, &bearer, method, request, course_id).await;
        if result.as_ref().is_err_and(|error| error.code() == Code::Unauthenticated) {
            self.credentials.lock().await.token.invalidate(&bearer);
        }
        result.map_err(|status| course_error(status.code()))
    }
}

#[derive(Clone, PartialEq, Message)]
struct ClassroomRouting {
    #[prost(int64, optional, tag = "1")]
    course_id: Option<i64>,
}
#[derive(Clone, PartialEq, Message)]
struct RoutingEnvelope {
    #[prost(message, optional, tag = "444624357")]
    classroom: Option<ClassroomRouting>,
}

async fn classroom_rpc_routed<Q, R>(
    channel: &Channel, bearer: &str, method: &str, request: Q, course_id: Option<i64>,
) -> Result<R, Status>
where
    Q: Message + Default + Send + Sync + 'static,
    R: Message + Default + Send + Sync + 'static,
{
    let mut grpc = tonic::client::Grpc::new(channel.clone()).max_decoding_message_size(8 * 1024 * 1024);
    grpc.ready().await.map_err(|_| Status::unavailable("Classroom channel is unavailable"))?;
    let mut authorization = tonic::metadata::MetadataValue::try_from(format!("Bearer {bearer}"))
        .map_err(|_| Status::internal("Invalid auth metadata"))?;
    authorization.set_sensitive(true);
    let mut request = tonic::Request::new(request);
    request.metadata_mut().insert("authorization", authorization);
    if let Some(course_id) = course_id {
        let envelope = RoutingEnvelope { classroom: Some(ClassroomRouting { course_id: Some(course_id) }) }.encode_to_vec();
        request.metadata_mut().insert_bin("x-goog-ext-53200201-bin", tonic::metadata::MetadataValue::from_bytes(&envelope));
    }
    let service = match method {
        "AddUserToCourse" | "RemoveUserFromCourse" | "DeleteCourseInvitation" | "UpdateCourseUser" | "CreateCourse" =>
            "/homeroom.courses.CoursesService/",
        "ListRubrics" => "/apps.edu.lms.rubric.api.RubricService/",
        _ => SERVICE,
    };
    let path = format!("{service}{method}").parse::<tonic::codegen::http::uri::PathAndQuery>()
        .map_err(|_| Status::internal("Invalid Classroom method"))?;

    grpc.unary(request, path, tonic_prost::ProstCodec::<Q, R>::default())
        .await
        .map(tonic::Response::into_inner)
        .map_err(|status| {
            if std::env::var_os("CLASSROOM_READ_ONLY_DIAGNOSTICS").is_some() {
                let known = ["invalid wire type", "Error deserializing", "Error serializing", "Missing", "Internal error", "Unsupported compression", "protocol error"];
                let label = known.into_iter().find(|word| status.message().contains(word)).unwrap_or("unclassified");
                eprintln!("RPC failure: method={method} code={:?} category={label} details_bytes={}", status.code(), status.details().len());
            }
            status
        })
}

pub(super) fn course_error(code: tonic::Code) -> String {
    match code {
        tonic::Code::Unauthenticated => "Google didn't accept this session. Sign in again to continue.".into(),
        tonic::Code::PermissionDenied => "Google denied this Classroom request.".into(),
        tonic::Code::ResourceExhausted => "Google is limiting Classroom requests. Try again later.".into(),
        tonic::Code::DeadlineExceeded | tonic::Code::Unavailable => {
            "Google Classroom didn't respond. Try again shortly.".into()
        }
        other => format!("Google Classroom couldn't complete this request ({other:?})."),
    }
}

fn sign_in_error(stage: Option<&str>, reason: Option<&str>) -> String {
    match (stage, reason) {
        (Some("saved-session"), Some("missing")) => "Saved Classroom sign-in is unavailable. Sign in with Google to continue.".into(),
        (Some("browser"), Some("closed")) => "Google sign-in was closed.".into(),
        (Some("browser"), Some("timed-out")) => "Google sign-in timed out. Try again when you're ready.".into(),
        (Some("browser"), Some("google-denied")) => "Google didn't allow sign-in in this browser window.".into(),
        (Some(stage @ ("account" | "classroom")), Some(reason)) => {
            match reason {
                "BadAuthentication" | "NeedsBrowser" | "ServiceDisabled" | "InvalidScope"
                | "CaptchaRequired" | "DeviceManagementRequired" | "InvalidSecondFactor"
                | "AccountDisabled" | "AppNotAuthorized" | "Unauthorized" | "TokenExpired" => {
                    format!("Google declined the {stage} sign-in step ({reason}).")
                }
                _ => format!("Google declined the {stage} sign-in step."),
            }
        }
        _ => "Google sign-in couldn't finish. Try again from Classroom.".into(),
    }
}

// Extra recovered shapes stay in the schema as offline evidence.
#[allow(dead_code)]
mod classwork {
    tonic::include_proto!("homeroom.classwork_evidence");
}

pub fn drive_url_reference(url: &str) -> Option<String> {
    let host = https_host(url)?;
    let uri: tonic::codegen::http::Uri = url.parse().ok()?;
    if host == "drive.google.com" && uri.path() == "/open" {
        let mut ids = uri.query()?.split('&').filter_map(|entry| {
            let (key, value) = entry.split_once('=')?;
            (key == "id").then_some(value)
        });
        let id = ids.next()?;
        return (ids.next().is_none() && drive_files::valid_drive_id(id)).then(|| id.to_owned());
    }
    let mut parts = uri.path().split('/').skip(1);
    let category = parts.next()?;
    let marker = match parts.next()? {
        "u" => {
            let account = parts.next()?;
            if account.is_empty() || !account.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            parts.next()?
        }
        marker => marker,
    };
    let id = parts.next()?;
    let known = marker == "d" && match host.as_str() {
        "docs.google.com" => matches!(category, "document" | "spreadsheets" | "presentation" | "drawings")
            || category == "forms" && id != "e",
        "drive.google.com" => category == "file",
        _ => false,
    };
    (known && drive_files::valid_drive_id(id)).then(|| id.to_owned())
}

#[cfg(test)]
#[test]
fn drive_file_urls_keep_the_exact_id_across_google_account_paths() {
    for url in [
        "https://docs.google.com/document/u/1/d/example-file/edit?resourcekey=opaque",
        "https://drive.google.com/file/u/0/d/example-file/view",
        "https://drive.google.com/open?resourcekey=opaque&id=example-file",
    ] {
        assert_eq!(drive_url_reference(url).as_deref(), Some("example-file"));
    }
    for url in [
        "https://docs.google.com.invalid/document/d/example-file/edit",
        "https://docs.google.com/document/u/student/d/example-file/edit",
        "https://drive.google.com/open?id=example-file&id=another-file",
        "https://drive.google.com/open?id=%65xample-file",
        "https://docs.google.com/forms/d/e/public-published-form/viewform",
    ] {
        assert_eq!(drive_url_reference(url), None);
    }
}

#[cfg(test)]
#[test]
fn drive_material_with_an_id_and_no_link_remains_openable_through_metadata() {
    let file = material(classwork::Material {
        reference_type: Some(2), reference: Some("unlinked-file".into()), ..Default::default()
    }).unwrap();
    assert_eq!(file.kind, AttachmentKind::DriveFile);
    assert_eq!(file.title, "Drive file");
    assert_eq!(file.drive_reference.as_deref(), Some("unlinked-file"));
    assert!(file.url.is_none());
}

pub fn https_host(value: &str) -> Option<String> {
    if value.len() > 8192 || value.bytes().any(|byte| byte.is_ascii_control() || byte == b'\\') {
        return None;
    }
    let uri: tonic::codegen::http::Uri = value.parse().ok()?;
    if uri.scheme_str()? != "https" || uri.authority()?.as_str().contains('@')
        || !matches!(uri.port_u16(), None | Some(443)) {
        return None;
    }
    Some(uri.host()?.to_ascii_lowercase())
}

fn check_class(course_id: i64) -> Result<(), String> {
    if course_id > 0 {
        Ok(())
    } else {
        Err("Choose a live class first.".into())
    }
}

fn classroom_event_time(timestamp: Option<i64>) -> Option<i64> {
    timestamp.filter(|time| (946_684_800_000..4_102_444_800_000).contains(time))
}

// The generated mutation types refer to this sibling package by its proto name.
use classwork as classwork_evidence;

#[allow(dead_code)]
mod mutation {
    tonic::include_proto!("homeroom.mutation_evidence");
}

fn artwork_host_is_allowed(url: &str) -> bool {
    let Some(host) = https_host(url) else { return false };
    host == "gstatic.com" || host.ends_with(".gstatic.com")
        || host == "googleusercontent.com" || host.ends_with(".googleusercontent.com")
}

pub async fn forget_session(account: String) -> Result<(), String> {
    let runtime = Runtime::resolve()?;
    let mut child = Command::new(runtime.python).arg("-I").arg("-B")
        .arg(runtime.sign_in).arg("--forget")
        .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null())
        .kill_on_drop(true).spawn().map_err(|_| "Could not open Keychain.".to_owned())?;
    let mut input = child.stdin.take().ok_or("Could not remove session.")?;
    input.write_all(format!("{account}\n").as_bytes()).await.map_err(|_| "Could not remove session.".to_owned())?;
    drop(input);
    let status = timeout(Duration::from_secs(30), child.wait()).await
        .map_err(|_| "Keychain removal timed out.".to_owned())?
        .map_err(|_| "Could not remove session.".to_owned())?;
    if status.success() {
        if let Some(path) = account_preference_path() {
            match std::fs::remove_file(path) {
                Ok(()) => {},
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                Err(_) => return Err("Sign-in removed; could not clear the saved account preference.".into()),
            }
        }
        Ok(())
    } else { Err("Keychain could not remove the saved sign-in. Try again.".into()) }
}

fn account_preference_path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?)
        .join("Library/Application Support/Classroom for Mac/last-account"))
}

pub fn remembered_account() -> Option<String> {
    let path = account_preference_path()?;
    if std::fs::metadata(&path).ok()?.len() > 256 { return None; }
    normalize_account(&std::fs::read_to_string(path).ok()?).ok()
}

fn save_account_preference(account: &str) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::io::Write;
    let Some(path) = account_preference_path() else { return Ok(()) };
    let parent = path.parent().ok_or("Account preference path unavailable.")?;
    std::fs::create_dir_all(parent).map_err(|_| "Could not save the account preference.")?;
    let temp = parent.join("last-account.tmp");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600)
        .open(&temp).map_err(|_| "Could not save the account preference.")?;
    file.write_all(account.as_bytes()).map_err(|_| "Could not save the account preference.")?;
    std::fs::rename(temp, path).map_err(|_| "Could not save the account preference.".into())
}
