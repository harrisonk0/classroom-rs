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

mod email_notifications;
mod mentions;
pub use mentions::{ClassMention, load_class_mentions};
mod stream_posts;
mod stream_settings;
mod student_muting;
mod teacher_topics;
mod topic_order;
mod classwork_order;
pub use classwork_order::{ClassworkOrderDirection, reorder_classwork};
pub use topic_order::{TopicOrderDirection, reorder_topic};
pub use teacher_topics::{TeacherTopicAction, TopicItemTarget, change_classwork_topic, check_item_topic_read,
    load_teacher_topics, mutate_teacher_topic, normalized_topic_name};
pub use student_muting::{load_student_muting, set_student_muted};
pub use stream_settings::{ClassDetails, CourseStreamSettings, StreamPostingPolicy, TeacherClassSettings, load_teacher_class_settings, save_teacher_class_settings};
mod teacher_assignment;
mod assignment_fields;
mod originality;
pub use originality::originality_report_url;
mod teacher_submissions;
pub use assignment_fields::{AssignmentDue, AssignmentFieldChanges, parse_assignment_due, parse_assignment_points};
pub use teacher_assignment::{AssignmentPublication, assignment_draft_text_valid, change_assignment_publication, save_assignment_draft,
    save_assignment_draft_with_fields};
pub use teacher_submissions::{TeacherFeedbackKey, TeacherGradebook, TeacherGradebookCell, TeacherGradingAction, TeacherSubmission,
    load_teacher_gradebook, load_teacher_private_feedback, load_teacher_submissions, post_teacher_private_feedback,
    return_teacher_submissions, write_teacher_grade};
mod class_visibility;
mod class_archive;
pub use class_archive::{read_class_state, set_class_archived};
mod class_create;
pub use class_create::{NewClass, can_create_classes, class_input_valid, create_class};
mod class_order;
mod comment_delete;
mod class_calendar;
pub use comment_delete::{CommentAudience, check_class_comment_target, delete_own_comment, moderate_class_comment};
mod drive_files;
pub use drive_files::{DriveFilePage, DriveFolder, DriveLocation, list_drive_files, lookup_drive_file_link};
pub use class_order::{CoursePlacement, reorder_class};
pub use class_visibility::{class_visibility_for, set_class_hidden};
pub use stream_posts::{
    PublishedAnnouncement, StreamAccess, load_stream_access, load_stream_permission, post_stream_announcement, upload_stream_file,
    check_stream_post_lookup, delete_own_stream_post, moderate_stream_post,
};
pub use email_notifications::{
    EmailNotificationCategory, EmailNotificationGroup, EmailNotificationSettings, EmailNotificationTarget,
    load_email_notifications, set_email_notification,
};

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
        if let Some(workspace) = crate_root.parent().and_then(Path::parent) {
            let tools = workspace.join("work/notebooklm-test");
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
    let valid = account.split_once('@').is_some_and(|(name, domain)| {
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

fn course_details_request() -> wire::QueryCourseRequest {
    let mut request = crate::course_list_candidate();
    if let Some(filter) = &mut request.filter_options {
        filter.latest_enrollment_event = vec![2]; // ADDED
    }
    if let Some(mask) = &mut request.course_read_mask {
        mask.include_course_state = Some(true);
        mask.include_owner_user_id = Some(true);
        mask.include_description = Some(true);
        mask.include_room = Some(true);
        mask.include_calendar_url = Some(true);
        mask.include_reordering_sort_key = Some(true);
        mask.current_user_setting_read_mask = Some(wire::CourseUserSettingReadMask {
            include_course_visibility_setting: Some(true),
            ..Default::default()
        });
        mask.video_call_info_read_mask = Some(wire::VideoCallInfoReadMask {
            include_video_call_url_state: Some(true),
            include_video_call_url: Some(true),
            include_video_call_url_type: None,
        });
        mask.course_setting_read_mask = Some(wire::CourseSettingReadMask {
            include_student_stream_posting_policy: None,
            include_classwork_in_stream_display_type: Some(true),
            course_theme_setting_read_mask: Some(wire::CourseThemeSettingReadMask {
                include_color: Some(true),
                include_color_gm3: Some(true),
                include_secure_photo_url: Some(true),
                include_secure_photo_url_gm3: Some(true),
                include_photo_render_mode: Some(true),
                include_use_gm3_stock_theme: Some(true),
            }),
        });
    }
    request
}

fn decode_courses(response: wire::QueryCourseResponse) -> Snapshot {
    let mut courses = response.course.into_iter().filter_map(|mut wire_course| {
        let style = wire_course.course_setting.as_mut()
            .and_then(|settings| settings.course_theme_setting.take())
            .and_then(course_theme);
        let meet = wire_course.video_call_info.take().and_then(student_meet_url);
        let calendar = wire_course.calendar_url.as_deref().and_then(class_calendar::calendar_url);
        let owners = wire_course.owner_user_id.iter()
            .filter_map(|bytes| classwork::UserId::decode(bytes.as_slice()).ok()?.id).filter(|id| *id > 0).collect();
        let mut course = Course::from_wire(wire_course)?;
        course.theme = style;
        course.meet_url = meet;
        course.calendar_url = calendar;
        course.owner_ids = owners;
        Some(course)
    }).collect::<Vec<_>>();
    class_order::sort_visible_classes(&mut courses);
    Snapshot::live(courses)
}

fn course_theme(theme: wire::CourseThemeSetting) -> Option<CourseTheme> {
    let artwork_url = theme.secure_photo_url_gm3
        .filter(|url| artwork_host_is_allowed(url))
        .or_else(|| theme.secure_photo_url.filter(|url| artwork_host_is_allowed(url)));
    let color_rgb = theme.color_gm3.iter().chain(theme.color.iter()).find_map(|value| parse_color(value));
    if artwork_url.is_none() && color_rgb.is_none() {
        return None;
    }
    Some(CourseTheme { color_rgb, artwork_url })
}

fn parse_color(value: &str) -> Option<[u8; 3]> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some([
        u8::from_str_radix(&hex[..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..], 16).ok()?,
    ])
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

fn classwork_request(course_id: i64) -> wire::StreamItemQueryRequest {
    classwork_query(course_id, false)
}

fn classwork_query(course_id: i64, drafts: bool) -> wire::StreamItemQueryRequest {
    use classwork::*;
    let info = StreamItemInfoReadMask {
        include_title: Some(true),
        include_description: Some(true),
        description_read_method: Some(2),
        include_topic_id: Some(true),
        include_classwork_sort_key: (!drafts).then_some(true),
        publication_info_mask: drafts.then_some(PublicationInfoReadMask {
            include_publication_status: Some(true),
            include_scheduled_timestamp: Some(true),
            include_scheduled_status: Some(true),
            ..Default::default()
        }),
        material_mask: Some(MaterialReadMask {
            include_name: Some(true),
            include_mime_type: Some(true),
            include_open_url: Some(true),
            drive_item_mask: Some(DriveItemReadMask { include_sharing_option: Some(true) }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let task = TaskInfoReadMask {
        include_due_date: Some(true),
        include_user_specified_due_time: Some(true),
        include_grade_denominator: Some(true),
        ..Default::default()
    };
    let mask = StreamReadMask {
        assignment_mask: Some(AssignmentReadMask {
            stream_item_info_mask: Some(info.clone()),
            task_info_mask: Some(task.clone()),
            originality_settings_mask: (!drafts).then_some(OriginalitySettingsReadMask { include_state: Some(true) }),
        }),
        post_mask: Some(PostReadMask { stream_item_info_mask: Some(StreamItemInfoReadMask {
            include_creator_user_id: Some(true), include_classwork_sort_key: None, ..info.clone()
        }) }),
        question_mask: Some(QuestionReadMask {
            stream_item_info_mask: Some(info.clone()),
            task_info_mask: Some(TaskInfoReadMask {
                include_allow_edit_after_turn_in: Some(true),
                submissions_visibility_mask: Some(VisibilityReadMask { include_visibility_type: Some(true) }),
                ..task
            }),
            question_info_mask: Some(QuestionInfoReadMask { include_choices: Some(true) }),
        }),
        supplement_mask: Some(SupplementReadMask { stream_item_info_mask: Some(info) }),
    };
    let filter = StreamFilter {
        allowed_status: vec![2], // ACTIVE
        course_id: vec![CourseId { id: Some(course_id) }],
        publication_status: vec![if drafts { 3 } else { 2 }],
        ..Default::default()
    };
    wire::StreamItemQueryRequest {
        read_mask: Some(mask.encode_to_vec()),
        filter: vec![filter.encode_to_vec()],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftPublication {
    Draft,
    Scheduled(Option<i64>),
    Retrying(Option<i64>),
    Failed(Option<i64>),
}

#[derive(Debug, Clone)]
pub struct TeacherDraft {
    pub item: Coursework,
    pub publication: DraftPublication,
}

pub async fn load_teacher_drafts(session: &LiveSession, course_id: i64) -> Result<Vec<TeacherDraft>, String> {
    check_class(course_id)?;
    let membership = load_own_class_details(session, vec![course_id]).await?;
    if membership.roles.get(&course_id) != Some(&CourseRole::Teacher) {
        return Err("Classroom did not confirm teacher access for this class.".into());
    }
    let response: wire::StreamItemQueryResponse = session.rpc(
        "QueryStreamItem", classwork_query(course_id, true), Some(course_id),
    ).await?;
    let mut drafts = BTreeMap::new();
    for bytes in response.stream_item {
        let item = classwork::StreamItem::decode(bytes.as_slice())
            .map_err(|_| "Classroom returned an unreadable draft.")?;
        if let Some(draft) = teacher_draft(course_id, item) {
            drafts.insert(draft.item.id.clone(), draft);
        }
    }
    Ok(drafts.into_values().collect())
}

fn teacher_draft(course_id: i64, item: classwork::StreamItem) -> Option<TeacherDraft> {
    let info = item.assignment.as_ref().and_then(|row| row.stream_item_info.as_ref())
        .or_else(|| item.post.as_ref()?.stream_item_info.as_ref())
        .or_else(|| item.question.as_ref()?.stream_item_info.as_ref())
        .or_else(|| item.supplement.as_ref()?.stream_item_info.as_ref())?;
    if info.status.is_some_and(|status| status != 2) {
        return None;
    }
    let publication = info.publication_info.as_ref()?;
    if publication.status != Some(3) {
        return None;
    }
    let timestamp = classroom_event_time(publication.scheduled_timestamp);
    let publication = match publication.scheduled_status {
        Some(1) => DraftPublication::Scheduled(timestamp),
        Some(2) => DraftPublication::Retrying(timestamp),
        Some(4) => DraftPublication::Failed(timestamp),
        Some(3) => return None,
        _ => DraftPublication::Draft,
    };
    Some(TeacherDraft { item: coursework(course_id, item)?, publication })
}

#[cfg(test)]
#[test]
fn teacher_drafts_require_an_explicit_draft_status_and_exact_course() {
    use classwork::*;
    let scheduled_at = 1_789_000_000_000;
    let item = |status, course| StreamItem {
        assignment: Some(Assignment {
            stream_item_info: Some(StreamItemInfo {
                stream_item_id: Some(StreamItemId { id: Some(49), course_id: Some(CourseId { id: Some(course) }) }),
                publication_info: Some(PublicationInfo {
                    status: Some(status), scheduled_timestamp: Some(scheduled_at), scheduled_status: Some(1), ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(teacher_draft(9, item(3, 9)).unwrap().publication, DraftPublication::Scheduled(Some(scheduled_at)));
    assert!(teacher_draft(9, item(2, 9)).is_none());
    assert!(teacher_draft(9, item(3, 8)).is_none());
    let filter = |drafts| StreamFilter::decode(classwork_query(9, drafts).filter[0].as_slice()).unwrap().publication_status;
    assert_eq!(filter(false), [2]);
    assert_eq!(filter(true), [3]);
}

fn coursework(course_id: i64, item: classwork::StreamItem) -> Option<Coursework> {
    let originality_enabled = match item.assignment.as_ref().and_then(|assignment| assignment.originality_settings.as_ref())
        .and_then(|settings| settings.state)
    {
        Some(1) => Some(true),
        Some(2) => Some(false),
        _ => None,
    };
    let (info, kind, task, question) = if let Some(assignment) = item.assignment {
        (assignment.stream_item_info?, CourseworkKind::Assignment, assignment.task_info, None)
    } else if let Some(post) = item.post {
        (post.stream_item_info?, CourseworkKind::Announcement, None, None)
    } else if let Some(question) = item.question {
        (question.stream_item_info?, CourseworkKind::Question, question.task_info,
            question.question_info.and_then(question_details))
    } else {
        (item.supplement?.stream_item_info?, CourseworkKind::Material, None, None)
    };
    // Live payloads use Unix milliseconds. Reject unset or out-of-range values.
    let due_at_millis = task.as_ref().and_then(|task| task.due_date)
        .filter(|time| (946_684_800_000..4_102_444_800_000).contains(time));
    let timed = task.as_ref().and_then(|task| task.user_specified_due_time) == Some(true);
    let due_label = due_at_millis.and_then(chrono::DateTime::from_timestamp_millis)
        .map(|date| {
            if timed {
                format!("Due {}", date.with_timezone(&chrono::Local).format("%-d %b, %-I:%M %p"))
            } else {
                format!("Due {}", date.format("%-d %b"))
            }
        });
    let id = info.stream_item_id?;
    if id.course_id.as_ref().and_then(|course| course.id) != Some(course_id) {
        return None;
    }
    let rich = info.description_rich.as_ref();
    let description = rich.and_then(|rich| rich.text.clone()).or(info.description).unwrap_or_default();
    let description_styles = rich.map_or_else(Vec::new, |rich| decode_description_styles(rich, &description));
    let description_bullets = rich.map_or_else(Vec::new, |rich| decode_description_bullets(rich, &description));
    let question_sharing = if question.is_some() {
        question_sharing(task.as_ref().and_then(|task| task.submissions_visibility.as_ref()))
    } else {
        None
    };
    Some(Coursework {
        id: id.id?.to_string(),
        course_id,
        title: info.title.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| kind.label().into()),
        description,
        description_styles,
        description_bullets,
        due_label,
        due_at_millis,
        due_has_time: due_at_millis.map(|_| timed),
        maximum_points: task.as_ref().and_then(|task| task.grade_denominator)
            .filter(|points| points.is_finite() && *points > 0.0),
        kind,
        author_id: (kind == CourseworkKind::Announcement).then(|| info.creator_user_id.and_then(|user| user.id)).flatten(),
        question,
        question_sharing,
        originality_enabled,
        allow_edit_after_turn_in: kind == CourseworkKind::Question
            && task.as_ref().and_then(|task| task.allow_edit_after_turn_in) == Some(true),
        topic_id: info.topic_id.into_iter()
            .find(|topic| topic.course_id.as_ref().and_then(|course| course.id) == Some(course_id))
            .and_then(|topic| topic.id),
        classwork_sort_key: info.classwork_sort_key.filter(|key| !key.is_empty()),
        attachments: info.material.into_iter().filter_map(material).collect(),
        submission: None,
        comments: None,
    })
}

fn decode_description_styles(rich: &classwork::RichText, text: &str) -> Vec<crate::desktop_data::DescriptionStyle> {
    let offsets = utf16_positions(text);
    let mut runs = BTreeMap::new();
    for style in &rich.style {
        let Some(&start) = offsets.get(&style.start.unwrap_or(0)) else { continue };
        runs.insert(start, crate::desktop_data::DescriptionStyle {
            start, bold: style.r#type.contains(&2), italic: style.r#type.contains(&3),
            underline: style.r#type.contains(&4),
        });
    }
    runs.into_values().collect()
}

fn utf16_positions(text: &str) -> BTreeMap<i64, usize> {
    let mut offsets = BTreeMap::new();
    let mut utf16 = 0;
    for (byte, character) in text.char_indices() {
        offsets.insert(utf16, byte);
        utf16 += character.len_utf16() as i64;
    }
    offsets
}

fn decode_description_bullets(rich: &classwork::RichText, text: &str) -> Vec<usize> {
    let offsets = utf16_positions(text);
    rich.block_style.iter().filter_map(|block| {
        if block.r#type != Some(1) {
            return None;
        }
        let &start = offsets.get(&block.start.unwrap_or(0))?;
        (start == 0 || text.as_bytes().get(start - 1) == Some(&b'\n')).then_some(start)
    }).collect::<BTreeSet<_>>().into_iter().collect()
}

fn material(item: classwork::Material) -> Option<Attachment> {
    use crate::desktop_data::DriveSharing;
    let drive_sharing = match item.drive_item.and_then(|drive| drive.sharing_option) {
        Some(1) => Some(DriveSharing::IndividualCopy),
        Some(2) => Some(DriveSharing::View),
        Some(3) => Some(DriveSharing::Edit),
        Some(4) => Some(DriveSharing::CopyToKeep),
        _ => None,
    };
    let url = item.open_url.filter(|value| https_host(value).is_some()).or_else(|| {
        if item.reference_type == Some(4) {
            item.reference.clone().filter(|value| https_host(value).is_some())
        } else {
            None
        }
    });
    let mime = item.mime_type.as_deref().unwrap_or_default();
    let kind = match url.as_deref().and_then(https_host).as_deref() {
        Some("www.youtube.com" | "youtu.be" | "youtube.com") => AttachmentKind::Video,
        Some("forms.gle") => AttachmentKind::Form,
        Some("docs.google.com") if url.as_ref().is_some_and(|url| url.contains("/forms/")) => AttachmentKind::Form,
        Some("drive.google.com" | "docs.google.com") => AttachmentKind::DriveFile,
        _ if mime.starts_with("video/") => AttachmentKind::Video,
        _ if mime.starts_with("application/vnd.google-apps.") || drive_sharing.is_some()
            || item.reference_type == Some(2) => AttachmentKind::DriveFile,
        _ if url.is_some() => AttachmentKind::Link,
        _ => AttachmentKind::Other,
    };
    let title = item.name.filter(|name| !name.trim().is_empty());
    let drive_reference = (item.reference_type == Some(2) || drive_sharing.is_some() || kind == AttachmentKind::DriveFile)
        .then(|| item.reference.filter(|value| drive_files::valid_drive_id(value))).flatten()
        .or_else(|| url.as_deref().and_then(drive_url_reference));
    if title.is_none() && url.is_none() && drive_reference.is_none() {
        return None;
    }
    Some(Attachment {
        remote_id: item.id.filter(|id| *id > 0), title: title.unwrap_or_else(|| kind.label().into()), url, kind,
        drive_sharing, drive_reference,
        drive_parent_reference: item.parent_reference.filter(|value| drive_files::valid_drive_id(value)),
    })
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

#[derive(Debug, Clone)]
pub struct CourseContents {
    pub items: Vec<Coursework>,
    pub topics: Option<Vec<Topic>>,
}

fn check_class(course_id: i64) -> Result<(), String> {
    if course_id > 0 {
        Ok(())
    } else {
        Err("Choose a live class first.".into())
    }
}

pub async fn load_coursework(session: &LiveSession, course_id: i64) -> Result<CourseContents, String> {
    check_class(course_id)?;
    let response: wire::StreamItemQueryResponse = session.rpc(
        "QueryStreamItem", classwork_request(course_id), Some(course_id),
    ).await?;
    let count = response.stream_item.len();
    let mut items = Vec::with_capacity(count);
    for bytes in response.stream_item {
        let item = classwork::StreamItem::decode(bytes.as_slice())
            .map_err(|_| "Google returned a Classwork response this build can't decode.".to_owned())?;
        if let Some(item) = coursework(course_id, item) {
            items.push(item);
        }
    }
    if count != 0 && items.is_empty() {
        return Err("Google returned items without readable IDs for this class.".into());
    }
    let topics = if items.iter().any(|item| item.topic_id.is_some()) {
        load_topics(session, course_id).await.ok()
    } else {
        None
    };
    Ok(CourseContents { items, topics })
}

#[derive(Debug, Clone)]
pub struct AssignmentRefresh {
    pub course_id: i64,
    pub contents: Option<CourseContents>,
    pub submissions: Option<Vec<(String, crate::desktop_data::Submission)>>,
}

pub fn assignment_updates(
    session: Arc<LiveSession>, courses: Vec<i64>,
) -> impl Stream<Item = AssignmentRefresh> + Send + 'static {
    stream::iter(courses).map(move |course_id| {
        let session = session.clone();
        async move {
            let contents = load_coursework(&session, course_id).await.ok();
            let submissions = if contents.as_ref().is_some_and(|result| {
                result.items.iter().any(|item| matches!(item.kind, CourseworkKind::Assignment | CourseworkKind::Question))
            }) {
                load_submissions(&session, course_id).await.ok()
            } else {
                None
            };
            AssignmentRefresh { course_id, contents, submissions }
        }
    }).buffer_unordered(3)
}

async fn load_topics(session: &LiveSession, course_id: i64) -> Result<Vec<Topic>, String> {
    use classwork::*;
    let response: TopicQueryResponse = session.rpc("TopicQuery", TopicQueryRequest {
        request_header: Some(vec![]),
        response_order: Some(2),
        read_mask: Some(TopicReadMask {
            include_name: Some(true),
            include_classwork_sort_key: Some(true),
            ..Default::default()
        }),
        filter: vec![TopicFilter { course_id: vec![CourseId { id: Some(course_id) }] }],
    }, Some(course_id)).await?;
    Ok(response.topic.into_iter().filter_map(|topic| {
        let key = topic.topic_id?;
        (key.course_id?.id? == course_id).then_some(crate::desktop_data::Topic { id: key.id?, title: topic.name? })
    }).collect())
}

fn question_details(info: classwork::QuestionInfo) -> Option<QuestionDetails> {
    match info.question_type? {
        1 => Some(QuestionDetails::ShortAnswer),
        2 => Some(QuestionDetails::MultipleChoice(info.multiple_choice?.choice)),
        _ => None,
    }
}

fn question_sharing(visibility: Option<&classwork::Visibility>) -> Option<crate::desktop_data::QuestionSharing> {
    use crate::desktop_data::QuestionSharing;
    match visibility?.visibility_type? {
        1 => Some(QuestionSharing::Private),
        2 => Some(QuestionSharing::Course),
        _ => None,
    }
}

#[cfg(test)]
#[test]
fn question_sharing_requires_a_known_explicit_visibility() {
    use crate::desktop_data::QuestionSharing::{Course, Private};
    for (value, expected) in [(Some(1), Some(Private)), (Some(2), Some(Course)), (Some(0), None), (Some(99), None), (None, None)] {
        assert_eq!(question_sharing(Some(&classwork::Visibility { visibility_type: value })), expected);
    }
    assert_eq!(question_sharing(None), None);
}

fn own_question_answer(answer: classwork::QuestionSubmission) -> Option<QuestionAnswer> {
    let short = answer.short_answer.filter(|text| !text.trim().is_empty());
    let choice = answer.multiple_choice_answer.filter(|text| !text.trim().is_empty());
    match (answer.question_answer_type, short, choice) {
        (Some(1) | None | Some(0), Some(text), None) => Some(QuestionAnswer::ShortAnswer(text)),
        (Some(2) | None | Some(0), None, Some(text)) => Some(QuestionAnswer::MultipleChoice(text)),
        (Some(1), Some(text), Some(_)) => Some(QuestionAnswer::ShortAnswer(text)),
        (Some(2), Some(_), Some(text)) => Some(QuestionAnswer::MultipleChoice(text)),
        _ => None,
    }
}

pub async fn load_submissions(session: &LiveSession, course_id: i64) -> Result<Vec<(String, crate::desktop_data::Submission)>, String> {
    use classwork::*;
    check_class(course_id)?;
    let response: wire::QuerySubmissionResponse = session.rpc("QuerySubmission", wire::QuerySubmissionRequest {
        submission_read_mask: Some(SubmissionReadMask {
            include_current_display_state_v2: Some(true),
            include_current_state: Some(true),
            include_live_grade_numerator: Some(true),
            include_last_turned_in_timestamp: Some(true),
            question_submission_mask: Some(QuestionSubmissionReadMask { include_answer: Some(true) }),
            live_rubric_score_mask: Some(RubricScoreReadMask {
                criterion_score_mask: Some(CriterionScoreReadMask { include_rating_id: Some(true), include_points: Some(true) }),
            }),
            assignment_submission_mask: Some(AssignmentSubmissionReadMask {
                material_mask: Some(MaterialReadMask {
                    include_name: Some(true),
                    include_mime_type: Some(true),
                    include_open_url: Some(true),
                    include_parent_reference: Some(true),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        }.encode_to_vec()),
        filter_options: Some(SubmissionFilter {
            base_filter_options: Some(FilterOptions {
                user_filter: vec![UserFilter { use_current_user: Some(true), user_id: None }],
            }),
            course_id: vec![CourseId { id: Some(course_id) }],
            ..Default::default()
        }.encode_to_vec()),
    }, Some(course_id)).await?;

    let mut results = BTreeMap::new();
    for bytes in response.submission {
        let wire = classwork::Submission::decode(bytes.as_slice())
            .map_err(|_| "Google returned a submission this build can't decode.".to_owned())?;
        let Some(submission_id) = wire.submission_id else { continue };
        let Some(key) = submission_id.stream_item_id else { continue };
        if key.course_id.and_then(|course| course.id) != Some(course_id) { continue }
        let Some(id) = key.id else { continue };
        let state = wire.current_display_state_v2.unwrap_or_default();
        let status = submission_display_status(state);
        let grade = if matches!(state, 5 | 8 | 10) {
            wire.live_grade_numerator.filter(|value| value.is_finite())
                .map(|earned| Grade { earned, maximum: None })
        } else {
            None
        };
        let entry = crate::desktop_data::Submission {
            remote_id: submission_id.id.filter(|id| *id > 0),
            remote_state: wire.current_state,
            attachments: wire.assignment_submission.into_iter()
                .flat_map(|assignment| assignment.material).filter_map(material).collect(),
            status,
            grade,
            last_turned_in_millis: classroom_event_time(wire.last_turned_in_timestamp),
            question_answer: wire.question_submission.and_then(own_question_answer),
            rubric_scores: rubric::released_scores(status, wire.live_rubric_score),
        };
        results.entry(id.to_string())
            .and_modify(|old: &mut crate::desktop_data::Submission| old.remote_id = None)
            .or_insert(entry);
    }
    Ok(results.into_iter().collect())
}

fn submission_display_status(state: i32) -> SubmissionStatus {
    match state {
        1 => SubmissionStatus::Assigned,
        2 => SubmissionStatus::Missing,
        3 => SubmissionStatus::TurnedIn,
        4 => SubmissionStatus::TurnedInLate,
        5..=8 => SubmissionStatus::Returned,
        9 | 10 => SubmissionStatus::ReturnedLate,
        11 => SubmissionStatus::Excused,
        _ => SubmissionStatus::Unknown,
    }
}

/// Uses the signed-in student filter and reads history for one exact submission.
pub async fn load_own_submission_history(
    session: &LiveSession, course_id: i64, item_id: i64, submission_id: i64,
) -> Result<Vec<crate::desktop_data::SubmissionEvent>, String> {
    use classwork::*;
    check_class(course_id)?;
    if item_id <= 0 || submission_id <= 0 {
        return Err("Classroom has not confirmed a submission for this item.".into());
    }
    let key = SubmissionId {
        id: Some(submission_id),
        stream_item_id: Some(StreamItemId { id: Some(item_id), course_id: Some(CourseId { id: Some(course_id) }) }),
    };
    let response: wire::QuerySubmissionResponse = session.rpc("QuerySubmission", wire::QuerySubmissionRequest {
        submission_read_mask: Some(SubmissionReadMask {
            include_submission_history: Some(true),
            ..Default::default()
        }.encode_to_vec()),
        filter_options: Some(SubmissionFilter {
            base_filter_options: Some(FilterOptions {
                user_filter: vec![UserFilter { use_current_user: Some(true), user_id: None }],
            }),
            submission_id: vec![key.clone()],
            ..Default::default()
        }.encode_to_vec()),
    }, Some(course_id)).await?;

    let mut matching = None;
    for bytes in response.submission {
        let submission = classwork::Submission::decode(bytes.as_slice())
            .map_err(|_| "Google returned an unreadable submission history.")?;
        if submission.submission_id.as_ref() == Some(&key)
            && matching.replace(submission.submission_history).is_some()
        {
            return Err("Google returned more than one history for this submission.".into());
        }
    }
    matching.map(decode_submission_history)
        .ok_or_else(|| "Google did not return the exact submission in the history lookup.".into())
}

fn classroom_event_time(timestamp: Option<i64>) -> Option<i64> {
    timestamp.filter(|time| (946_684_800_000..4_102_444_800_000).contains(time))
}

fn decode_submission_history(history: Vec<classwork::SubmissionHistory>) -> Vec<crate::desktop_data::SubmissionEvent> {
    use crate::desktop_data::SubmissionEvent;
    let mut seen = std::collections::BTreeSet::new();
    let mut events: Vec<_> = history.into_iter().filter_map(|history| {
        let event = history.state_history?;
        let state = event.state?;
        let status = match state {
            2 => SubmissionStatus::TurnedIn,
            3 => SubmissionStatus::Returned,
            5 => SubmissionStatus::Reclaimed,
            6 => SubmissionStatus::Excused,
            _ => return None,
        };
        let timestamp_millis = classroom_event_time(event.state_timestamp)?;
        seen.insert((timestamp_millis, state)).then_some(SubmissionEvent { status, timestamp_millis })
    }).collect();
    events.sort_by_key(|event| std::cmp::Reverse(event.timestamp_millis));
    events
}

#[cfg(test)]
#[test]
fn submission_history_keeps_only_distinct_state_events_with_millisecond_timestamps() {
    use classwork::{SubmissionHistory, SubmissionStateHistory};
    let at = 1_784_600_123_000;
    let event = |state, timestamp| SubmissionHistory {
        state_history: Some(SubmissionStateHistory { state: Some(state), state_timestamp: Some(timestamp) }),
    };
    let decoded = decode_submission_history(vec![
        event(2, at), event(4, at), event(3, at + 1), event(5, at),
        event(2, at), event(2, at / 1000), SubmissionHistory::default(),
    ]);
    assert_eq!(decoded.iter().map(|event| (event.status, event.timestamp_millis)).collect::<Vec<_>>(), [
        (SubmissionStatus::Returned, at + 1), (SubmissionStatus::TurnedIn, at), (SubmissionStatus::Reclaimed, at),
    ]);
}

pub async fn load_roster(session: &LiveSession, course_id: i64) -> Result<Vec<CourseMember>, String> {
    check_class(course_id)?;
    let mut request = crate::course_list_candidate();
    let filter = request.filter_options.as_mut().ok_or("Missing People filter.")?;
    filter.base_filter_options = None;
    filter.course_id = vec![wire::CourseId { id: Some(course_id) }];
    request.course_read_mask = Some(wire::CourseReadMask {
        include_student_user_id: Some(true),
        include_teacher_user_id: Some(true),
        ..Default::default()
    });
    let response: wire::QueryCourseResponse = session.rpc("QueryCourse", request, Some(course_id)).await?;
    let matching = response.course.into_iter()
        .find(|course| course.course_id.as_ref().and_then(|id| id.id) == Some(course_id))
        .ok_or("Google did not return the requested class roster.")?;
    let mut roles = BTreeMap::new();
    for (role, identifiers) in [
        (CourseRole::Student, matching.student_user_id),
        (CourseRole::Teacher, matching.teacher_user_id),
    ] {
        for bytes in identifiers {
            if let Ok(user) = classwork::UserId::decode(bytes.as_slice())
                && let Some(id) = user.id.filter(|id| *id > 0)
            {
                roles.insert(id, role);
            }
        }
    }
    if roles.len() > 500 {
        return Err("This class has too many members for one roster request.".into());
    }
    if roles.is_empty() {
        return Err("Google did not provide a visible roster for this class.".into());
    }
    let names = load_user_names(session, roles.keys().copied().collect()).await?;
    Ok(roles.into_iter().filter_map(|(id, role)| {
        Some(CourseMember { id: id.to_string(), display_name: names.get(&id)?.clone(), role })
    }).collect())
}

async fn load_user_names(session: &LiveSession, ids: BTreeSet<i64>) -> Result<BTreeMap<i64, String>, String> {
    use classwork::*;
    if ids.is_empty() { return Ok(BTreeMap::new()) }
    let response: QueryUserResponse = session.read_rpc("QueryUser", QueryUserRequest {
        user_read_mask: Some(UserReadMask { include_name: Some(true), ..Default::default() }),
        filter_options: Some(UserFilterOptions {
            base_filter_options: Some(FilterOptions {
                user_filter: ids.iter().map(|id| UserFilter {
                    use_current_user: None,
                    user_id: Some(UserId { id: Some(*id) }),
                }).collect(),
            }),
        }),
        ..Default::default()
    }).await?;
    Ok(response.user.into_iter().filter_map(|user| {
        let id = user.user_id?.id?;
        ids.contains(&id).then_some((id, user.name?))
    }).collect())
}

pub async fn load_class_teacher_names(session: &LiveSession, courses: &[(i64, Vec<i64>)]) -> Result<BTreeMap<i64, String>, String> {
    let ids = courses.iter().flat_map(|(_, ids)| ids.iter().copied()).collect();
    let names = load_user_names(session, ids).await?;
    Ok(class_teacher_names(courses, &names))
}

fn class_teacher_names(courses: &[(i64, Vec<i64>)], names: &BTreeMap<i64, String>) -> BTreeMap<i64, String> {
    courses.iter().filter_map(|(course, owners)| {
        owners.iter().filter_map(|owner| names.get(owner).map(|name| name.trim()))
            .find(|name| !name.is_empty()).map(|name| (*course, name.to_owned()))
    }).collect()
}

async fn current_user_id(session: &LiveSession) -> Result<i64, String> {
    let response: classwork::QueryUserResponse = session.read_rpc("QueryUser", classwork::QueryUserRequest {
        user_read_mask: Some(classwork::UserReadMask { include_name: Some(true), ..Default::default() }),
        filter_options: Some(classwork::UserFilterOptions {
            base_filter_options: Some(classwork::FilterOptions {
                user_filter: vec![classwork::UserFilter { use_current_user: Some(true), user_id: None }],
            }),
        }),
        ..Default::default()
    }).await?;
    match response.user.as_slice() {
        [user] => user.user_id.as_ref().and_then(|id| id.id).filter(|id| *id > 0)
            .ok_or_else(|| "Google did not provide the signed-in Classroom user ID.".into()),
        _ => Err("Google did not identify a single signed-in Classroom user.".into()),
    }
}

pub async fn check_current_user(session: &LiveSession) -> Result<(), String> {
    current_user_id(session).await.map(|_| ())
}

async fn require_class_moderator(session: &LiveSession, course: i64) -> Result<(), String> {
    check_class(course)?;
    if !session.snapshot.courses.iter().any(|entry| entry.id == course && entry.state == CourseState::Active) {
        return Err("Choose an active class.".into());
    }
    let details = load_own_class_details(session, vec![course]).await?;
    if details.roles.get(&course) != Some(&CourseRole::Teacher) {
        return Err("Classroom did not confirm teacher access to moderate this class.".into());
    }
    Ok(())
}

fn student_meet_url(info: wire::VideoCallInfo) -> Option<String> {
    // ENABLED_VISIBLE is the Android protocol value for student-visible links.
    (info.video_call_url_state == Some(3)).then_some(info.video_call_url).flatten()
        .filter(|url| https_host(url).as_deref() == Some("meet.google.com"))
}

#[derive(Debug, Clone, Default)]
pub struct OwnClassDetails {
    pub folders: BTreeMap<i64, String>,
    pub roles: BTreeMap<i64, CourseRole>,
}

pub async fn load_class_folders(
    session: &LiveSession, courses: Vec<i64>,
) -> Result<BTreeMap<i64, String>, String> {
    Ok(load_own_class_details(session, courses).await?.folders)
}

pub async fn load_own_class_details(
    session: &LiveSession, courses: Vec<i64>,
) -> Result<OwnClassDetails, String> {
    let courses: BTreeSet<_> = courses.into_iter().filter(|id| *id > 0).collect();
    if courses.is_empty() {
        return Ok(OwnClassDetails::default());
    }
    let own_user = current_user_id(session).await?;
    let ids = courses.iter().map(|id| classwork::CourseUserId {
        course_id: Some(classwork::CourseId { id: Some(*id) }),
        user_id: Some(classwork::UserId { id: Some(own_user) }),
    }).collect();
    let response: classwork::CourseUserLookupResponse = session.read_rpc(
        "LookupCourseUser",
        classwork::CourseUserLookupRequest {
            course_user_ids: ids,
            read_mask: Some(classwork::CourseUserReadMask {
                include_course_role: Some(true),
                course_folder_read_mask: Some(classwork::FolderReadMask { include_open_url: Some(true) }),
                ..Default::default()
            }),
        },
    ).await?;
    Ok(own_class_details(response, &courses, own_user))
}

fn own_class_details(
    response: classwork::CourseUserLookupResponse, requested: &BTreeSet<i64>, own_user: i64,
) -> OwnClassDetails {
    let mut details = OwnClassDetails::default();
    let mut seen = BTreeSet::new();
    for row in response.results {
        let (Some(key), Some(entry)) = (row.course_user_id, row.course_user) else { continue };
        let Some(id) = key.course_id.as_ref().and_then(|key| key.id) else { continue };
        if key.user_id.as_ref().and_then(|user| user.id) != Some(own_user)
            || entry.course_user_id.as_ref() != Some(&key) || !requested.contains(&id)
        {
            continue;
        }
        if !seen.insert(id) {
            details.folders.remove(&id);
            details.roles.remove(&id);
            continue;
        }
        if let Some(role) = match entry.course_role {
            Some(2) => Some(CourseRole::Teacher),
            Some(3) => Some(CourseRole::Student),
            _ => None,
        } {
            details.roles.insert(id, role);
        }
        if let Some(url) = entry.course_folder.and_then(|folder| folder.open_url)
            && https_host(&url).as_deref() == Some("drive.google.com")
        {
            details.folders.insert(id, url);
        }
    }
    details
}

fn notification_read_mask() -> wire::CourseReadMask {
    wire::CourseReadMask {
        include_course_state: Some(true),
        current_user_setting_read_mask: Some(wire::CourseUserSettingReadMask {
            include_notifications_muted: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn course_member_key(course: i64, own_user: i64) -> classwork::CourseUserId {
    classwork::CourseUserId {
        course_id: Some(classwork::CourseId { id: Some(course) }),
        user_id: Some(classwork::UserId { id: Some(own_user) }),
    }
}

pub async fn load_class_notifications(session: &LiveSession, courses: Vec<i64>) -> Result<BTreeMap<i64, bool>, String> {
    let ids: BTreeSet<_> = courses.into_iter().filter(|&id| id > 0).collect();
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut request = crate::course_list_candidate();
    request.course_read_mask = Some(notification_read_mask());
    let response: wire::QueryCourseResponse = session.read_rpc("QueryCourse", request).await?;
    Ok(own_class_notifications(response, &ids))
}

fn own_class_notifications(response: wire::QueryCourseResponse, courses: &BTreeSet<i64>) -> BTreeMap<i64, bool> {
    let mut settings = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for row in response.course {
        let Some(course) = row.course_id.and_then(|id| id.id).filter(|course| courses.contains(course)) else { continue; };
        if row.course_state != Some(1) {
            continue;
        }
        if !seen.insert(course) {
            settings.remove(&course);
            continue;
        }
        // On an active enrolled class, an unset preference keeps notifications enabled.
        settings.insert(course, row.current_user_setting.and_then(|setting| setting.notifications_muted).unwrap_or(false));
    }
    settings
}

fn update_class_notifications_request(course: i64, muted: bool) -> mutation::WriteCourseRequest {
    mutation::WriteCourseRequest {
        request_header: Some(mutation::WriteRequestHeader { operation: Some(3) }),
        course_mutation: vec![mutation::CourseMutation {
            course_id: Some(classwork::CourseId { id: Some(course) }),
            course: Some(wire::Course {
                course_id: Some(wire::CourseId { id: Some(course) }),
                current_user_setting: Some(wire::CourseUserSetting { notifications_muted: Some(muted), ..Default::default() }),
                ..Default::default()
            }.encode_to_vec()),
            course_update_mask: Some(mutation::CourseUpdateMask {
                current_user_setting_update_mask: Some(mutation::CourseUserSettingUpdateMask {
                    update_notifications_muted: Some(true), ..Default::default()
                }),
                ..Default::default()
            }.encode_to_vec()),
            ..Default::default()
        }],
        course_read_mask: Some(notification_read_mask().encode_to_vec()),
    }
}

pub async fn set_class_notifications(
    session: &LiveSession, course: i64, muted: bool,
) -> Result<bool, String> {
    check_class(course)?;
    let before = load_class_notifications(session, vec![course]).await?;
    let Some(&current) = before.get(&course) else {
        return Err("Classroom did not return your notification setting for this class.".into());
    };
    if current == muted {
        return Ok(current);
    }
    let _: mutation::WriteCourseResponse = session.write_rpc(
        "WriteCourse", update_class_notifications_request(course, muted), Some(course),
    ).await?;
    let after = load_class_notifications(session, vec![course]).await?;
    if after.get(&course) != Some(&muted) {
        return Err("Classroom did not confirm the notification change.".into());
    }
    Ok(muted)
}

pub async fn load_comments(session: &LiveSession, course_id: i64, stream_item_id: i64) -> Result<Vec<crate::desktop_data::Comment>, String> {
    use classwork::*;
    check_class(course_id)?;
    if stream_item_id <= 0 { return Err("Choose a published class item first.".into()) }
    let response: QueryCommentResponse = session.read_rpc("QueryComment", QueryCommentRequest {
        comment_read_mask: Some(CommentReadMask {
            include_comment: Some(true),
            include_creator_user_id: Some(true),
            ..Default::default()
        }),
        filter_options: Some(CommentFilterOptions {
            stream_item_id: vec![StreamItemId {
                id: Some(stream_item_id),
                course_id: Some(CourseId { id: Some(course_id) }),
            }],
            ..Default::default()
        }),
    }).await?;
    let received = response.comment.len();
    let entries = response.comment.into_iter().filter_map(|comment| {
        let key = comment.comment_id?;
        if comment.status != Some(2) || key.submission_id.is_some()
            || comment.visibility.is_some_and(|visibility| visibility.visibility_type == Some(1)) { return None }
        let parent = key.stream_item_id?;
        if parent.id != Some(stream_item_id) || parent.course_id?.id != Some(course_id) { return None }
        Some((key.id?, comment.creator_user_id.and_then(|user| user.id), comment.comment?))
    }).collect::<Vec<_>>();
    if received != 0 && entries.is_empty() {
        return Err("None of the returned comments could be verified as active comments on this item.".into());
    }
    // Failure to resolve a display name never authorizes an additional fallback query.
    let authors = load_user_names(session, entries.iter().filter_map(|(_, id, _)| *id).take(500).collect()).await
        .unwrap_or_default();
    Ok(entries.into_iter().map(|(id, author, body)| crate::desktop_data::Comment {
        id: id.to_string(),
        author_id: author,
        author: author.and_then(|id| authors.get(&id).cloned()).unwrap_or_else(|| "Class member".into()),
        body,
        created_label: None,
    }).collect())
}

pub async fn load_private_feedback(
    session: &LiveSession, course_id: i64, item_id: i64,
) -> Result<Vec<crate::desktop_data::Comment>, String> {
    use classwork::*;
    check_class(course_id)?;
    let (_, mine) = load_submissions(session, course_id).await?.into_iter()
        .find(|(id, _)| id == &item_id.to_string())
        .ok_or("Classroom did not return your submission for this assignment.")?;
    let target = SubmissionId {
        id: Some(mine.remote_id.ok_or("The submission could not be identified.")?),
        stream_item_id: Some(StreamItemId {
            id: Some(item_id),
            course_id: Some(CourseId { id: Some(course_id) }),
        }),
    };
    query_submission_feedback(session, course_id, target).await
}

async fn query_submission_feedback(
    session: &LiveSession, course_id: i64, target: classwork::SubmissionId,
) -> Result<Vec<crate::desktop_data::Comment>, String> {
    use classwork::*;
    let result: QueryCommentResponse = session.rpc(
        "QueryComment", QueryCommentRequest {
            comment_read_mask: Some(CommentReadMask {
                include_comment: Some(true),
                include_creator_user_id: Some(true),
                visibility_mask: Some(CommentVisibilityReadMask { include_visibility_type: Some(true) }),
                ..Default::default()
            }),
            filter_options: Some(CommentFilterOptions {
                submission_id: vec![target.clone()],
                visibility: vec![CommentVisibility { visibility_type: Some(1) }],
                ..Default::default()
            }),
        }, Some(course_id)
    ).await?;
    let count = result.comment.len();
    let entries = result.comment.into_iter()
        .filter_map(|entry| decode_private_feedback(entry, &target)).collect::<Vec<_>>();
    if count > 0 && entries.is_empty() {
        return Err("Classroom returned feedback without a matching private submission identity.".into());
    }
    let ids = entries.iter().filter_map(|(_, id, _)| *id).take(500).collect();
    let authors = load_user_names(session, ids).await.unwrap_or_default();
    Ok(entries.into_iter().map(|(id, author, body)| crate::desktop_data::Comment {
        id: id.to_string(),
        author_id: author,
        author: author.and_then(|id| authors.get(&id)).cloned().unwrap_or_else(|| "Class member".into()),
        body,
        created_label: None,
    }).collect())
}

fn decode_private_feedback(
    entry: classwork::Comment, target: &classwork::SubmissionId,
) -> Option<(i64, Option<i64>, String)> {
    let id = entry.comment_id?;
    if entry.status != Some(2) || entry.visibility?.visibility_type != Some(1)
        || id.submission_id.as_ref() != Some(target)
    {
        return None;
    }
    Some((id.id?, entry.creator_user_id.and_then(|author| author.id), entry.comment?))
}

// The generated mutation types refer to this sibling package by its proto name.
use classwork as classwork_evidence;

#[allow(dead_code)]
mod mutation {
    tonic::include_proto!("homeroom.mutation_evidence");
}

/// Send a single class-visible comment after the user explicitly clicks Post.
pub async fn post_class_comment(
    session: &LiveSession, course_id: i64, item_id: i64, body: String,
) -> Result<(), String> {
    check_class(course_id)?;
    if item_id <= 0 { return Err("Choose a published class item.".into()); }
    let (_, access) = load_stream_permission(session, course_id).await?;
    if !access.can_comment() {
        return Err("Classroom has disabled class comments for this account in this class.".into());
    }
    insert_comment(session, course_id, classwork::CommentId {
        stream_item_id: Some(classwork::StreamItemId {
            id: Some(item_id), course_id: Some(classwork::CourseId { id: Some(course_id) }),
        }),
        parent_type: Some(2),
        ..Default::default()
    }, 2, &body).await
}

pub async fn post_private_feedback(
    session: &LiveSession, course_id: i64, item_id: i64, body: String,
) -> Result<(), String> {
    check_class(course_id)?;
    let (_, mine) = load_submissions(session, course_id).await?.into_iter()
        .find(|(id, _)| id == &item_id.to_string())
        .ok_or("Classroom did not return your submission for this assignment.")?;
    let own_id = mine.remote_id.ok_or("The submission could not be identified.")?;
    insert_comment(session, course_id, classwork::CommentId {
        submission_id: Some(classwork::SubmissionId {
            id: Some(own_id),
            stream_item_id: Some(classwork::StreamItemId {
                id: Some(item_id), course_id: Some(classwork::CourseId { id: Some(course_id) }),
            }),
        }),
        parent_type: Some(3),
        ..Default::default()
    }, 1, &body).await
}

async fn insert_comment(
    session: &LiveSession, course_id: i64, target: classwork::CommentId, visibility: i32, body: &str,
) -> Result<(), String> {
    let body = body.trim();
    if body.is_empty() || body.len() > 10_000 {
        return Err("Enter a comment of up to 10,000 bytes.".into());
    }
    let request = mutation::WriteCommentRequest {
        request_header: Some(mutation::WriteRequestHeader { operation: Some(2) }),
        comment_mutation: vec![mutation::CommentMutation {
            comment_id: None,
            comment: Some(classwork::Comment {
                comment_id: Some(target.clone()),
                comment: Some(body.to_owned()),
                visibility: Some(classwork::CommentVisibility { visibility_type: Some(visibility) }),
                ..Default::default()
            }),
            comment_update_mask: None,
        }],
        comment_read_mask: Some(classwork::CommentReadMask {
            include_comment: Some(true),
            visibility_mask: Some(classwork::CommentVisibilityReadMask { include_visibility_type: Some(true) }),
            ..Default::default()
        }),
    };
    let response: mutation::WriteCommentResponse = session.write_rpc(
        "WriteComment", request, Some(course_id)
    ).await.map_err(|error| format!("{error} Refresh comments before posting again."))?;
    if response.comment.iter().any(|comment| {
        comment.comment_id.as_ref().is_some_and(|id| {
            id.id.is_some_and(|id| id > 0)
                && if visibility == 1 {
                    id.submission_id == target.submission_id
                        && comment.visibility.as_ref().and_then(|value| value.visibility_type) == Some(1)
                } else {
                    id.submission_id.is_none() && id.stream_item_id == target.stream_item_id
                        && comment.visibility.as_ref().and_then(|value| value.visibility_type) != Some(1)
                }
        })
    }) {
        Ok(())
    } else {
        Err("Google returned no confirmation. Refresh comments before posting again.".into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionAction {
    TurnIn,
    Unsubmit,
}

impl SubmissionAction {
    pub fn label(self) -> &'static str {
        match self { Self::TurnIn => "Turn in", Self::Unsubmit => "Unsubmit" }
    }

    pub fn for_submission(submission: &crate::desktop_data::Submission) -> Option<Self> {
        submission.remote_id?;
        match submission.remote_state? {
            4 | 5 => Some(Self::TurnIn),
            2 => Some(Self::Unsubmit),
            _ => None,
        }
    }

    fn state(self) -> i32 {
        match self { Self::TurnIn => 2, Self::Unsubmit => 5 }
    }
}

/// State transition for an existing submission belonging to the current user.
pub async fn change_submission(
    session: &LiveSession,
    course_id: i64,
    item_id: i64,
    action: SubmissionAction,
) -> Result<(), String> {
    check_class(course_id)?;
    let item_key = item_id.to_string();
    let (_, submission) = load_submissions(session, course_id).await?
        .into_iter().find(|(id, _)| id == &item_key)
        .ok_or("Classroom did not return an existing submission for this item.")?;
    if SubmissionAction::for_submission(&submission) != Some(action) {
        return Err("Submission status changed. Refresh your work before trying again.".into());
    }
    let submission_id = classwork::SubmissionId {
        id: submission.remote_id,
        stream_item_id: Some(classwork::StreamItemId {
            id: Some(item_id),
            course_id: Some(classwork::CourseId { id: Some(course_id) }),
        }),
    };
    let response: mutation::WriteSubmissionResponse = session.write_rpc(
        "WriteSubmission", mutation::WriteSubmissionRequest {
            request_header: Some(mutation::WriteRequestHeader { operation: Some(3) }),
            submission_mutation: vec![mutation::SubmissionMutation {
                submission_id: Some(submission_id.clone()),
                submission: Some(classwork::Submission {
                    submission_id: Some(submission_id.clone()),
                    current_state: Some(action.state()),
                    ..Default::default()
                }),
                submission_update_mask: Some(mutation::SubmissionUpdateMask {
                    update_current_state: Some(true),
                    ..Default::default()
                }),
                add_materials: vec![],
            }],
            submission_read_mask: Some(classwork::SubmissionReadMask {
                include_current_state: Some(true),
                ..Default::default()
            }),
        }, Some(course_id),
    ).await.map_err(|error| format!("{error} Refresh your work before trying again."))?;
    if response.submission.iter().any(|entry| {
        entry.submission_id.as_ref() == Some(&submission_id) && entry.current_state == Some(action.state())
    }) {
        Ok(())
    } else {
        Err("Classroom returned no confirmation. Refresh your work before trying again.".into())
    }
}

pub fn can_turn_in_question(item: &Coursework) -> bool {
    item.kind == CourseworkKind::Question && item.question.is_some()
        && item.submission.as_ref().is_some_and(|submission| {
            submission.question_answer.is_none()
                && SubmissionAction::for_submission(submission) == Some(SubmissionAction::TurnIn)
        })
}

#[derive(Clone, PartialEq, Eq)]
pub struct ShortAnswerRevision {
    pub submission_id: i64,
    pub answer: String,
}

pub fn editable_short_answer(item: &Coursework) -> Option<(i64, &str)> {
    if item.kind != CourseworkKind::Question || item.question != Some(QuestionDetails::ShortAnswer)
        || !item.allow_edit_after_turn_in
    {
        return None;
    }
    let submission = item.submission.as_ref()?;
    if submission.remote_state != Some(2) || submission.grade.is_some()
        || !matches!(submission.status, SubmissionStatus::TurnedIn | SubmissionStatus::TurnedInLate)
    {
        return None;
    }
    let QuestionAnswer::ShortAnswer(answer) = submission.question_answer.as_ref()? else { return None };
    Some((submission.remote_id?, answer.as_str()))
}

pub fn question_submission_is_current(item: &Coursework, revision: Option<&ShortAnswerRevision>) -> bool {
    match revision {
        Some(revision) => editable_short_answer(item) == Some((revision.submission_id, revision.answer.as_str())),
        None => can_turn_in_question(item),
    }
}

/// Turn in a first answer or update a permitted short answer for the current user.
pub async fn turn_in_question(
    session: &LiveSession, course_id: i64, item_id: i64, answer: QuestionAnswer,
    revision: Option<ShortAnswerRevision>,
) -> Result<(), String> {
    check_class(course_id)?;
    if item_id <= 0 {
        return Err("Open a published question first.".into());
    }
    let item_key = item_id.to_string();
    let (items, submissions) = tokio::try_join!(
        load_coursework(session, course_id),
        load_submissions(session, course_id),
    )?;
    let mut question = items.items.into_iter().find(|item| item.id == item_key)
        .ok_or("This question is no longer available.")?;
    question.submission = submissions.into_iter().find(|(id, _)| id == &item_key)
        .map(|(_, submission)| submission);
    if !question_submission_is_current(&question, revision.as_ref()) {
        return Err("The answer or its editing permissions changed in Classroom. Review your work before trying again.".into());
    }
    if !question.question.as_ref().is_some_and(|details| details.accepts(&answer)) {
        return Err("This answer no longer matches the published question.".into());
    }
    if let (Some(revision), QuestionAnswer::ShortAnswer(text)) = (&revision, &answer)
        && text == &revision.answer
    {
        return Err("Make a change to your answer before turning it in.".into());
    }
    let id = classwork::SubmissionId {
        id: question.submission.and_then(|submission| submission.remote_id),
        stream_item_id: Some(classwork::StreamItemId {
            id: Some(item_id),
            course_id: Some(classwork::CourseId { id: Some(course_id) }),
        }),
    };
    let response: mutation::WriteSubmissionResponse = session.write_rpc(
        "WriteSubmission", question_answer_request(id.clone(), answer.clone()), Some(course_id),
    ).await.map_err(|error| format!("{error} Check your work before trying again."))?;
    if response.submission.iter().any(|entry| {
        entry.submission_id.as_ref() == Some(&id)
            && entry.current_state == Some(SubmissionAction::TurnIn.state())
            && entry.question_submission.clone().and_then(own_question_answer).as_ref() == Some(&answer)
    }) {
        Ok(())
    } else {
        Err("Classroom did not confirm this answer. Check your work before trying again.".into())
    }
}

fn question_answer_request(id: classwork::SubmissionId, answer: QuestionAnswer) -> mutation::WriteSubmissionRequest {
    let question_submission = match answer {
        QuestionAnswer::ShortAnswer(text) => classwork::QuestionSubmission {
            short_answer: Some(text),
            ..Default::default()
        },
        QuestionAnswer::MultipleChoice(choice) => classwork::QuestionSubmission {
            multiple_choice_answer: Some(choice),
            question_answer_type: Some(2),
            ..Default::default()
        },
    };
    mutation::WriteSubmissionRequest {
        request_header: Some(mutation::WriteRequestHeader { operation: Some(3) }),
        submission_mutation: vec![mutation::SubmissionMutation {
            submission_id: Some(id.clone()),
            submission: Some(classwork::Submission {
                submission_id: Some(id),
                current_state: Some(SubmissionAction::TurnIn.state()),
                question_submission: Some(question_submission),
                ..Default::default()
            }),
            submission_update_mask: Some(mutation::SubmissionUpdateMask {
                update_current_state: Some(true),
                question_submission_mask: Some(mutation::QuestionSubmissionUpdateMask { update_answer: Some(true) }),
                ..Default::default()
            }),
            add_materials: vec![],
        }],
        submission_read_mask: Some(classwork::SubmissionReadMask {
            include_current_state: Some(true),
            question_submission_mask: Some(classwork::QuestionSubmissionReadMask { include_answer: Some(true) }),
            ..Default::default()
        }),
    }
}

pub async fn remove_submission_attachment(
    session: &LiveSession, course_id: i64, item_id: i64, material_id: i64,
) -> Result<(), String> {
    check_class(course_id)?;
    if material_id <= 0 {
        return Err("Classroom did not provide an ID for this attachment.".into());
    }
    let (_, mine) = load_submissions(session, course_id).await?.into_iter()
        .find(|(id, _)| id == &item_id.to_string())
        .ok_or("Classroom did not provide your existing submission.")?;
    if SubmissionAction::for_submission(&mine) != Some(SubmissionAction::TurnIn)
        || !mine.attachments.iter().any(|item| item.remote_id == Some(material_id))
    {
        return Err("The attachment is no longer part of an editable submission.".into());
    }
    let target = classwork::SubmissionId {
        id: mine.remote_id,
        stream_item_id: Some(classwork::StreamItemId {
            id: Some(item_id), course_id: Some(classwork::CourseId { id: Some(course_id) }),
        }),
    };
    let response: mutation::WriteSubmissionResponse = session.write_rpc(
        "WriteSubmission", mutation::WriteSubmissionRequest {
            request_header: Some(mutation::WriteRequestHeader { operation: Some(3) }),
            submission_mutation: vec![mutation::SubmissionMutation {
                submission_id: Some(target.clone()),
                submission: Some(classwork::Submission {
                    submission_id: Some(target.clone()),
                    assignment_submission: Some(classwork::AssignmentSubmission {
                        material: vec![classwork::Material { id: Some(material_id), ..Default::default() }],
                    }),
                    ..Default::default()
                }),
                submission_update_mask: Some(mutation::SubmissionUpdateMask {
                    assignment_submission_mask: Some(mutation::AssignmentSubmissionUpdateMask {
                        update_material: Some(3),
                    }),
                    ..Default::default()
                }),
                add_materials: vec![],
            }],
            submission_read_mask: Some(classwork::SubmissionReadMask {
                assignment_submission_mask: Some(classwork::AssignmentSubmissionReadMask {
                    material_mask: Some(classwork::MaterialReadMask { include_name: Some(true), ..Default::default() }),
                }),
                ..Default::default()
            }),
        }, Some(course_id),
    ).await.map_err(|error| format!("{error} Refresh your work before trying again."))?;
    let confirmed = response.submission.into_iter()
        .find(|item| item.submission_id.as_ref() == Some(&target))
        .and_then(|item| item.assignment_submission)
        .is_some_and(|work| {
            let received: BTreeSet<_> = work.material.iter().filter_map(|item| item.id).collect();
            !received.contains(&material_id)
                && mine.attachments.iter().filter_map(|item| item.remote_id)
                    .filter(|id| *id != material_id).all(|id| received.contains(&id))
        });
    if confirmed { Ok(()) } else {
        Err("Classroom did not confirm removal. Refresh your work before trying again.".into())
    }
}

pub async fn attach_submission_link(
    session: &LiveSession, course_id: i64, item_id: i64, url: String,
) -> Result<(), String> {
    let url = url.trim();
    if https_host(url).is_none() {
        return Err("Enter a valid HTTPS link.".into());
    }
    let (target, _) = editable_submission(session, course_id, item_id).await?;
    attach_material(session, course_id, target, link_material(url)).await
}

pub async fn attach_submission_drive_file(
    session: &LiveSession, course_id: i64, item_id: i64, file: DriveFileAttachment,
) -> Result<(), String> {
    let (target, mine) = editable_submission(session, course_id, item_id).await?;
    if mine.attachments.iter().any(|attachment| file.matches_attachment(attachment)) {
        return Ok(());
    }
    attach_material(session, course_id, target, file.0).await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveDocumentKind {
    Docs,
    Sheets,
    Slides,
    Drawings,
}

impl DriveDocumentKind {
    pub const ALL: [Self; 4] = [Self::Docs, Self::Sheets, Self::Slides, Self::Drawings];

    pub fn label(self) -> &'static str {
        match self {
            Self::Docs => "Docs",
            Self::Sheets => "Sheets",
            Self::Slides => "Slides",
            Self::Drawings => "Drawings",
        }
    }

    fn descriptor(self) -> (i32, &'static str, &'static str) {
        match self {
            Self::Docs => (1, "document", "application/vnd.google-apps.document"),
            Self::Sheets => (2, "spreadsheets", "application/vnd.google-apps.spreadsheet"),
            Self::Slides => (3, "presentation", "application/vnd.google-apps.presentation"),
            Self::Drawings => (6, "drawings", "application/vnd.google-apps.drawing"),
        }
    }
}

fn drive_document_request(course_id: i64, title: String, kind: DriveDocumentKind) -> classwork::DriveItemCreateRequest {
    classwork::DriveItemCreateRequest {
        course_id: Some(classwork::CourseId { id: Some(course_id) }),
        drive_item_type: Some(kind.descriptor().0),
        title: Some(title),
        material_read_mask: Some(classwork::MaterialReadMask {
            include_name: Some(true), include_mime_type: Some(true), include_open_url: Some(true),
            ..Default::default()
        }),
    }
}

fn created_drive_document(material: classwork::Material, kind: DriveDocumentKind) -> Option<classwork::Material> {
    let (_, editor, mime) = kind.descriptor();
    if material.reference_type.is_some_and(|value| value != 2)
        || material.mime_type.as_deref().is_some_and(|value| value != mime)
    {
        return None;
    }
    let link_id = material.open_url.as_deref().and_then(|url| {
        let host = https_host(url)?;
        let uri: tonic::codegen::http::Uri = url.parse().ok()?;
        let mut parts = uri.path().split('/').skip(1);
        match (host.as_str(), parts.next(), parts.next(), parts.next()) {
            ("docs.google.com", Some(category), Some("d"), Some(id))
                if category == editor && drive_files::valid_drive_id(id) => Some(id.to_owned()),
            ("drive.google.com", Some("file"), Some("d"), Some(id))
                if drive_files::valid_drive_id(id) => Some(id.to_owned()),
            _ => None,
        }
    });
    let id = match (material.reference, link_id) {
        (Some(reference), Some(link_id)) if reference != link_id => return None,
        (Some(reference), _) if drive_files::valid_drive_id(&reference) => reference,
        (None, Some(link_id)) => link_id,
        _ => return None,
    };
    Some(classwork::Material {
        name: material.name,
        open_url: Some(format!("https://docs.google.com/{editor}/d/{id}/edit")),
        reference: Some(id),
        reference_type: Some(2),
        mime_type: Some(mime.into()),
        ..Default::default()
    })
}

#[cfg(test)]
#[test]
fn create_drive_document_uses_the_recovered_request_tags_and_four_editor_types() {
    for (kind, tag) in DriveDocumentKind::ALL.into_iter().zip([1, 2, 3, 6]) {
        let request = drive_document_request(9, "Essay".into(), kind);
        assert_eq!(request.encode_to_vec(), [
            0x0a, 2, 0x08, 9, 0x10, tag, 0x22, 5, b'E', b's', b's', b'a', b'y',
            0x2a, 6, 0x10, 1, 0x20, 1, 0x30, 1,
        ]);
    }
}

#[cfg(test)]
#[test]
fn created_editor_material_binds_a_valid_drive_id_to_the_existing_submission_mutation() {
    let editor = "https://docs.google.com/document/d/fixture-doc/edit";
    let raw = classwork::Material { open_url: Some(editor.into()), ..Default::default() };
    let material = created_drive_document(raw.clone(), DriveDocumentKind::Docs).unwrap();
    let id = classwork::SubmissionId {
        id: Some(7),
        stream_item_id: Some(classwork::StreamItemId {
            id: Some(5), course_id: Some(classwork::CourseId { id: Some(9) }),
        }),
    };
    let request = attach_material_request(id.clone(), &material);
    let [entry] = request.submission_mutation.as_slice() else { panic!("Expected one attachment mutation") };
    assert_eq!(entry.submission_id.as_ref(), Some(&id));
    assert!(entry.submission.is_none() && entry.submission_update_mask.is_none());
    let decoded = classwork::Material::decode(entry.add_materials[0].as_slice()).unwrap();
    assert_eq!(decoded.reference.as_deref(), Some("fixture-doc"));
    assert_eq!(decoded.reference_type, Some(2));
    assert_eq!(decoded.open_url.as_deref(), Some(editor));
    assert_eq!(decoded.is_new_upload, None);
    assert!(created_drive_document(raw.clone(), DriveDocumentKind::Slides).is_none());
    assert!(created_drive_document(classwork::Material { reference: Some("wrong".into()), ..raw.clone() }, DriveDocumentKind::Docs).is_none());
    assert!(created_drive_document(classwork::Material {
        open_url: Some("https://docs.google.com.example.invalid/document/d/fixture-doc/edit".into()), ..raw
    }, DriveDocumentKind::Docs).is_none());
}

pub async fn create_submission_document(
    session: &LiveSession, course_id: i64, item_id: i64, title: String, kind: DriveDocumentKind,
) -> Result<String, String> {
    let (target, _) = editable_submission(session, course_id, item_id).await?;
    let title = title.trim().chars().take(200).collect::<String>();
    let response: classwork::DriveItemCreateResponse = session.write_rpc(
        "DriveItemCreate", drive_document_request(course_id, title, kind), Some(course_id),
    ).await.map_err(|error| format!("{error} Check Drive for the new file before trying again."))?;
    let material = response.material.and_then(|material| created_drive_document(material, kind))
        .ok_or("Classroom did not return a usable Drive file. Check Drive before trying again.")?;
    let url = material.open_url.clone().ok_or("The new Drive file did not include an editor link.")?;

    // Recheck ownership and editability between creation and attachment.
    let attachment = async {
        let (current, _) = editable_submission(session, course_id, item_id).await?;
        if current != target {
            return Err("Your submission changed.".to_owned());
        }
        attach_material(session, course_id, target, material).await
    }.await;
    attachment.map_err(|error| format!("The new file is in Google Drive. Classroom could not confirm attachment. {error}"))?;
    Ok(url)
}

fn link_material(url: &str) -> classwork::Material {
    classwork::Material {
        reference: Some(url.into()), open_url: Some(url.into()), reference_type: Some(4),
        ..Default::default()
    }
}

async fn editable_submission(
    session: &LiveSession, course_id: i64, item_id: i64,
) -> Result<(classwork::SubmissionId, crate::desktop_data::Submission), String> {
    check_class(course_id)?;
    if item_id <= 0 {
        return Err("Choose an assignment first.".into());
    }
    let key = item_id.to_string();
    let (_, mine) = load_submissions(session, course_id).await?
        .into_iter().find(|(id, _)| id == &key)
        .ok_or("Classroom did not provide an existing submission.")?;
    if SubmissionAction::for_submission(&mine) != Some(SubmissionAction::TurnIn) {
        return Err("Unsubmit your work before adding attachments.".into());
    }
    Ok((classwork::SubmissionId {
        id: mine.remote_id,
        stream_item_id: Some(classwork::StreamItemId {
            id: Some(item_id), course_id: Some(classwork::CourseId { id: Some(course_id) }),
        }),
    }, mine))
}

fn attach_material_request(target: classwork::SubmissionId, material: &classwork::Material) -> mutation::WriteSubmissionRequest {
    mutation::WriteSubmissionRequest {
        request_header: Some(mutation::WriteRequestHeader { operation: Some(3) }),
        submission_mutation: vec![mutation::SubmissionMutation {
            submission_id: Some(target),
            submission: None,
            submission_update_mask: None,
            add_materials: vec![material.encode_to_vec()],
        }],
        submission_read_mask: Some(classwork::SubmissionReadMask {
            assignment_submission_mask: Some(classwork::AssignmentSubmissionReadMask {
                material_mask: Some(classwork::MaterialReadMask {
                    include_open_url: Some(true), ..Default::default()
                }),
            }),
            ..Default::default()
        }),
    }
}

async fn attach_material(
    session: &LiveSession, course_id: i64, target: classwork::SubmissionId,
    material: classwork::Material,
) -> Result<(), String> {
    let response: mutation::WriteSubmissionResponse = session.write_rpc(
        "WriteSubmission", attach_material_request(target.clone(), &material), Some(course_id),
    ).await.map_err(|error| format!("{error} Refresh your work before adding the attachment again."))?;
    if response.submission.into_iter().any(|item| item.submission_id.as_ref() == Some(&target)
        && item.assignment_submission.is_some_and(|work| work.material.iter().any(|existing| {
            material_matches(&material, existing)
        }))) {
        Ok(())
    } else {
        Err("Classroom did not confirm the attachment. Refresh your work before trying again.".into())
    }
}

fn material_matches(sent: &classwork::Material, received: &classwork::Material) -> bool {
    if sent.reference.is_some() && sent.reference == received.reference
        || sent.open_url.is_some() && sent.open_url == received.open_url {
        return true;
    }
    if sent.reference_type != Some(2) {
        return false;
    }
    let (Some(id), Some(url)) = (sent.reference.as_deref(), received.open_url.as_deref()) else {
        return false;
    };
    let Ok(uri) = url.parse::<tonic::codegen::http::Uri>() else { return false };
    if uri.scheme_str() != Some("https") {
        return false;
    }
    if uri.host() == Some("docs.google.com") {
        let mut parts = uri.path().trim_start_matches('/').split('/');
        return matches!(
            (parts.next(), parts.next(), parts.next()),
            (Some("document" | "spreadsheets" | "presentation" | "forms" | "drawings"), Some("d"), Some(found))
                if found == id
        );
    }
    if uri.host() != Some("drive.google.com") {
        return false;
    }
    let mut parts = uri.path().trim_start_matches('/').split('/');
    matches!((parts.next(), parts.next(), parts.next()), (Some("file"), Some("d"), Some(found)) if found == id)
        || (uri.path() == "/open" && uri.query().is_some_and(|query| {
            form_urlencoded::parse(query.as_bytes()).any(|(key, value)| key == "id" && value == id)
        }))
}

#[derive(Deserialize)]
struct DriveUploadReply {
    id: Option<String>,
    name: Option<String>,
    mime_type: Option<String>,
}

fn drive_helper_path() -> Result<(PathBuf, PathBuf), String> {
    let runtime = Runtime::resolve()?;
    let script = runtime.sign_in.parent().ok_or("Drive upload runtime is missing.")?
        .join("drive_upload.py");
    if !script.is_file() {
        return Err("Drive upload runtime is missing.".into());
    }
    Ok((runtime.python, script))
}

pub async fn attach_submission_file(
    session: &LiveSession, course_id: i64, item_id: i64, path: PathBuf,
) -> Result<(), String> {
    let (target, _) = editable_submission(session, course_id, item_id).await?;
    let file = upload_drive_file(session, path).await?;
    attach_material(session, course_id, target, file.0).await.map_err(|error| {
        format!("Your file was uploaded to Google Drive, but attachment is unconfirmed. {error}")
    })
}

#[derive(Clone)]
pub struct DriveFileAttachment(classwork::Material);

impl DriveFileAttachment {
    pub fn name(&self) -> &str {
        self.0.name.as_deref().unwrap_or("Drive file")
    }

    pub fn id(&self) -> &str {
        self.0.reference.as_deref().unwrap_or_default()
    }

    pub fn matches_attachment(&self, attachment: &Attachment) -> bool {
        attachment.url.as_ref().is_some_and(|url| material_matches(&self.0, &classwork::Material {
            open_url: Some(url.clone()),
            ..Default::default()
        }))
    }

    pub fn existing(id: String, name: String, mime: String) -> Option<Self> {
        Self::from_drive(id, Some(name), Some(mime), false)
    }

    fn from_drive(id: String, name: Option<String>, mime: Option<String>, uploaded: bool) -> Option<Self> {
        if !drive_files::valid_drive_id(&id) {
            return None;
        }
        let open_url = format!("https://drive.google.com/file/d/{id}/view");
        Some(Self(classwork::Material {
            name,
            reference: Some(id),
            reference_type: Some(2),
            mime_type: mime,
            open_url: Some(open_url),
            is_new_upload: uploaded.then_some(true),
            ..Default::default()
        }))
    }
}

async fn upload_drive_file(session: &LiveSession, path: PathBuf) -> Result<DriveFileAttachment, String> {
    let bearer = session.bearer_for_request(None).await?;
    let payload = Zeroizing::new(serde_json::to_vec(&serde_json::json!({
        "path": path.to_str().ok_or("This file path is not valid UTF-8.")?,
        "bearer": bearer.as_str(),
    })).map_err(|_| "Could not prepare the upload.")?);
    let reply: DriveUploadReply = run_drive_helper(&[], payload, Duration::from_secs(600),
        "Upload did not finish. Check Google Drive before trying again.").await?;
    reply.id.and_then(|id| DriveFileAttachment::from_drive(id, reply.name, reply.mime_type, true))
        .ok_or_else(|| "Drive did not confirm the file ID.".into())
}

async fn run_drive_helper<R: serde::de::DeserializeOwned>(
    args: &[&str], payload: Zeroizing<Vec<u8>>, deadline: Duration, failure: &'static str,
) -> Result<R, String> {
    let (python, script) = drive_helper_path()?;
    let mut child = Command::new(python).args(["-I", "-B"]).arg(script).args(args)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
        .kill_on_drop(true).spawn().map_err(|_| "Could not open the Drive helper.")?;
    let mut input = child.stdin.take().ok_or("Drive helper pipe unavailable.")?;
    input.write_all(payload.as_slice()).await.map_err(|_| "Could not send Drive request details.")?;
    drop(input);
    drop(payload);
    let output = timeout(deadline, child.wait_with_output()).await
        .map_err(|_| failure)?.map_err(|_| failure)?;
    if !output.status.success() {
        return Err(serde_json::from_slice::<serde_json::Value>(&output.stdout).ok()
            .and_then(|reply| reply.get("error")?.as_str().map(str::to_owned))
            .unwrap_or_else(|| failure.into()));
    }
    serde_json::from_slice(&output.stdout).map_err(|_| "Drive returned an unreadable result.".into())
}

pub fn valid_class_code(code: &str) -> bool {
    let code = code.trim();
    (6..=8).contains(&code.len()) && code.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

pub async fn load_invitations(session: &LiveSession) -> Result<Vec<Course>, String> {
    let user_id = current_user_id(session).await?;
    invitations_for(session, user_id).await
}

async fn invitations_for(session: &LiveSession, user_id: i64) -> Result<Vec<Course>, String> {
    let request = wire::QueryCourseRequest {
        course_read_mask: Some(wire::CourseReadMask {
            include_title: Some(true),
            include_subtitle: Some(true),
            include_invited_student_user_id: Some(true),
            ..Default::default()
        }),
        filter_options: Some(wire::CourseFilterOptions {
            base_filter_options: Some(wire::FilterOptions {
                user_filter: vec![wire::UserFilter { use_current_user: Some(true) }],
            }),
            latest_enrollment_event: vec![3], // INVITED
            ..Default::default()
        }),
    };
    let response: wire::QueryCourseResponse = session.read_rpc("QueryCourse", request).await?;
    let returned = response.course.len();
    let mut seen = BTreeSet::new();
    let matched: Vec<_> = response.course.into_iter().filter(|course| {
        course.invited_student_user_id.iter().any(|bytes| {
            classwork::UserId::decode(bytes.as_slice()).is_ok_and(|id| id.id == Some(user_id))
        })
    }).filter_map(Course::from_wire).filter(|course| course.id > 0 && seen.insert(course.id)).collect();
    if returned != 0 && matched.is_empty() {
        return Err("Classroom returned no invitation that matches this student's identity.".into());
    }
    Ok(matched)
}

pub async fn join_class(session: &LiveSession, code: String) -> Result<i64, String> {
    if !valid_class_code(&code) {
        return Err("Enter the 6 to 8 character code from your teacher.".into());
    }
    let user_id = current_user_id(session).await?;
    let response: mutation::AddUserToCourseResponse = session.write_rpc(
        "AddUserToCourse", mutation::AddUserToCourseRequest {
            course_id: None,
            enrollment_code: Some(code.trim().to_owned()),
            user_id: Some(classwork::UserId { id: Some(user_id) }),
            course_role: Some(3),
            read_mask: Some(classwork::CourseUserReadMask { include_course_role: Some(true), ..Default::default() }),
        }, None,
    ).await.map_err(|error| format!("{error} Refresh Home before trying to join again."))?;
    response.course_user.and_then(|member| member.course_user_id)
        .filter(|id| id.user_id.as_ref().and_then(|user| user.id) == Some(user_id))
        .and_then(|id| id.course_id.and_then(|course| course.id))
        .filter(|id| *id > 0)
        .ok_or_else(|| "Google did not confirm enrollment. Refresh Home before trying again.".into())
}

pub async fn accept_invitation(session: &LiveSession, course_id: i64) -> Result<(), String> {
    check_class(course_id)?;
    let user_id = current_user_id(session).await?;
    if !invitations_for(session, user_id).await?.iter().any(|course| course.id == course_id) {
        return Err("This invitation is no longer pending. Refresh the invitation list.".into());
    }
    let response: mutation::AddUserToCourseResponse = session.write_rpc(
        "AddUserToCourse", mutation::AddUserToCourseRequest {
            course_id: Some(classwork::CourseId { id: Some(course_id) }),
            enrollment_code: None,
            user_id: Some(classwork::UserId { id: Some(user_id) }),
            course_role: Some(3),
            read_mask: Some(classwork::CourseUserReadMask { include_course_role: Some(true), ..Default::default() }),
        }, Some(course_id),
    ).await.map_err(|error| format!("{error} Refresh invitations before trying again."))?;
    let confirmed = response.course_user.and_then(|member| member.course_user_id)
        .is_some_and(|id| id.course_id.and_then(|course| course.id) == Some(course_id)
            && id.user_id.and_then(|user| user.id) == Some(user_id));
    if confirmed { Ok(()) } else {
        Err("Classroom did not confirm enrollment. Refresh Home and invitations before trying again.".into())
    }
}

fn decline_invitation_request(course_id: i64, user_id: i64) -> mutation::DeleteCourseInvitationRequest {
    mutation::DeleteCourseInvitationRequest {
        course_invitation_id: Some(mutation::CourseInvitationId {
            course_id: Some(classwork::CourseId { id: Some(course_id) }),
            id: Some(user_id),
        }),
        invited_role: Some(mutation::InvitedRole::Student.into()),
    }
}

pub async fn decline_invitation(session: &LiveSession, course_id: i64) -> Result<Vec<Course>, String> {
    check_class(course_id)?;
    let user_id = current_user_id(session).await?;
    if !invitations_for(session, user_id).await?.iter().any(|course| course.id == course_id) {
        return Err("This invitation is no longer pending.".into());
    }
    // Android discards the response body. Confirm by reading the pending list.
    let _: () = session.write_rpc(
        "DeleteCourseInvitation", decline_invitation_request(course_id, user_id), Some(course_id),
    ).await?;
    let remaining = invitations_for(session, user_id).await
        .map_err(|error| format!("Couldn’t confirm that the invitation was declined. {error}"))?;
    if remaining.iter().any(|course| course.id == course_id) {
        return Err("Classroom still shows this invitation as pending.".into());
    }
    Ok(remaining)
}

pub async fn leave_class(session: &LiveSession, course_id: i64) -> Result<(), String> {
    check_class(course_id)?;
    let user_id = current_user_id(session).await?;
    let response: wire::QueryCourseResponse = session.rpc(
        "QueryCourse", wire::QueryCourseRequest {
            course_read_mask: Some(wire::CourseReadMask {
                include_student_user_id: Some(true),
                include_teacher_user_id: Some(true),
                ..Default::default()
            }),
            filter_options: Some(wire::CourseFilterOptions {
                base_filter_options: None,
                course_id: vec![wire::CourseId { id: Some(course_id) }],
                ..Default::default()
            }),
        }, Some(course_id),
    ).await?;
    let me = |encoded: &Vec<u8>| {
        classwork::UserId::decode(encoded.as_slice()).is_ok_and(|user| user.id == Some(user_id))
    };
    let is_student = response.course.into_iter().find(|course| {
        course.course_id.as_ref().and_then(|id| id.id) == Some(course_id)
    }).is_some_and(|course| {
        course.student_user_id.iter().any(me) && !course.teacher_user_id.iter().any(me)
    });
    if !is_student {
        return Err("Google did not confirm that you are enrolled as a student in this class.".into());
    }
    let _: mutation::RemoveUserFromCourseResponse = session.write_rpc(
        "RemoveUserFromCourse", mutation::RemoveUserFromCourseRequest {
            course_id: Some(classwork::CourseId { id: Some(course_id) }),
            user_id: Some(classwork::UserId { id: Some(user_id) }),
            course_role: Some(3),
        }, Some(course_id),
    ).await.map_err(|error| format!("{error} Refresh Home before trying again."))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_teacher_names_only_use_the_course_owner_ids() {
        let names = BTreeMap::from([(1, " Alice ".into()), (2, "Bob".into()), (3, "Not an owner".into())]);
        assert_eq!(class_teacher_names(&[(7, vec![9, 1]), (8, vec![2]), (10, vec![9])], &names),
            BTreeMap::from([(7, "Alice".into()), (8, "Bob".into())]));
    }

    #[test]
    fn decline_invitation_encodes_android_course_id_and_invitee() {
        assert_eq!(
            decline_invitation_request(23, 156).encode_to_vec(),
            [0x0a, 7, 0x0a, 2, 0x08, 23, 0x10, 0x9c, 0x01, 0x10, 2],
        );
    }

    #[test]
    fn class_notifications_use_the_current_user_preference_and_no_student_mute_field() {
        for muted in [false, true] {
            let request = update_class_notifications_request(7, muted);
            let [entry] = request.course_mutation.as_slice() else { panic!("Expected one course") };
            let course = wire::Course::decode(entry.course.as_deref().unwrap()).unwrap();
            assert_eq!(course.course_id.unwrap().id, Some(7));
            assert!(course.muted_user_id.is_empty());
            assert_eq!(course.current_user_setting.unwrap().notifications_muted, Some(muted));
            let mask = mutation::CourseUpdateMask::decode(entry.course_update_mask.as_deref().unwrap()).unwrap();
            assert!(mask.course_setting_update_mask.is_none());
            assert_eq!(mask.current_user_setting_update_mask.unwrap().update_notifications_muted, Some(true));
        }
    }

    #[test]
    fn class_notifications_only_use_unique_active_classes_and_default_to_enabled() {
        let row = |course, state, notifications_muted| wire::Course {
            course_id: Some(wire::CourseId { id: Some(course) }), course_state: Some(state),
            current_user_setting: Some(wire::CourseUserSetting { notifications_muted, ..Default::default() }),
            muted_user_id: vec![vec![8, 7]], ..Default::default()
        };
        let response = wire::QueryCourseResponse { course: vec![
            row(1, 1, None), row(2, 1, Some(true)), row(3, 2, Some(true)),
            row(4, 1, Some(true)), row(4, 1, Some(false)), row(5, 1, Some(true)),
        ] };
        assert_eq!(own_class_notifications(response, &BTreeSet::from([1, 2, 3, 4])),
            BTreeMap::from([(1, false), (2, true)]));
    }

    #[test]
    fn short_answer_wire_shape_sets_answer_and_state_in_one_update() {
        let id = classwork::SubmissionId {
            id: Some(27),
            stream_item_id: Some(classwork::StreamItemId {
                id: Some(34), course_id: Some(classwork::CourseId { id: Some(56) }),
            }),
        };
        let request = question_answer_request(id.clone(), QuestionAnswer::ShortAnswer("A\nB".to_owned()));
        let decoded = mutation::WriteSubmissionRequest::decode(request.encode_to_vec().as_slice()).unwrap();
        assert_eq!(decoded.request_header.unwrap().operation, Some(3));
        assert_eq!(decoded.submission_mutation.len(), 1);
        let update = &decoded.submission_mutation[0];
        assert_eq!(update.submission_id.as_ref(), Some(&id));
        assert!(update.add_materials.is_empty());
        let submission = update.submission.as_ref().unwrap();
        assert_eq!(submission.submission_id.as_ref(), Some(&id));
        assert_eq!(submission.current_state, Some(2));
        assert_eq!(submission.question_submission.as_ref().unwrap().encode_to_vec(),
            [0x12, 3, b'A', b'\n', b'B']);
        assert_eq!(update.submission_update_mask.as_ref().unwrap().encode_to_vec(),
            [0x08, 1, 0x42, 2, 0x08, 1]);
    }

    #[test]
    fn multiple_choice_request_encodes_the_choice_field_and_shared_update_mask() {
        let id = classwork::SubmissionId { id: Some(42), stream_item_id: None };
        let request = question_answer_request(id, QuestionAnswer::MultipleChoice("Option A".into()));
        let decoded = mutation::WriteSubmissionRequest::decode(request.encode_to_vec().as_slice()).unwrap();
        let mutation = &decoded.submission_mutation[0];
        let answer = mutation.submission.as_ref().unwrap().question_submission.as_ref().unwrap();
        assert_eq!(answer.encode_to_vec(), [0x08, 2, 0x1a, 8, b'O', b'p', b't', b'i', b'o', b'n', b' ', b'A']);
        assert_eq!(mutation.submission_update_mask.as_ref().unwrap().encode_to_vec(), [0x08, 1, 0x42, 2, 0x08, 1]);
        assert_eq!(mutation.submission.as_ref().unwrap().current_state, Some(2));
    }

    #[test]
    fn editing_checks_permission_state_and_exact_original_answer() {
        let snapshot = Snapshot::demo();
        let mut question = snapshot.coursework_for(4).unwrap()[0].clone();
        question.submission = Some(crate::desktop_data::Submission {
            remote_id: Some(27), remote_state: Some(2), status: SubmissionStatus::TurnedIn,
            question_answer: Some(QuestionAnswer::ShortAnswer("Original response".into())),
            ..Default::default()
        });
        let revision = ShortAnswerRevision { submission_id: 27, answer: "Original response".into() };
        assert!(!question_submission_is_current(&question, Some(&revision)));
        question.allow_edit_after_turn_in = true;
        assert!(question_submission_is_current(&question, Some(&revision)));
        assert!(!question_submission_is_current(&question, None));
        assert!(!question_submission_is_current(&question, Some(&ShortAnswerRevision {
            submission_id: 28, answer: revision.answer.clone(),
        })));
        assert!(!question_submission_is_current(&question, Some(&ShortAnswerRevision {
            submission_id: 27, answer: "Other answer".into(),
        })));
        question.submission.as_mut().unwrap().status = SubmissionStatus::Returned;
        assert!(!question_submission_is_current(&question, Some(&revision)));
        question.submission.as_mut().unwrap().status = SubmissionStatus::TurnedInLate;
        assert!(question_submission_is_current(&question, Some(&revision)));
        question.submission.as_mut().unwrap().grade = Some(Grade { earned: 0.0, maximum: Some(5.0) });
        assert!(!question_submission_is_current(&question, Some(&revision)));
    }

    #[test]
    fn first_question_answer_requires_an_editable_existing_own_submission() {
        let snapshot = Snapshot::demo();
        let mut question = snapshot.coursework_for(4).unwrap()[0].clone();
        assert!(!can_turn_in_question(&question));
        question.submission = Some(crate::desktop_data::Submission {
            remote_id: Some(27), remote_state: Some(4), ..Default::default()
        });
        assert!(can_turn_in_question(&question));
        question.submission.as_mut().unwrap().question_answer = Some(QuestionAnswer::ShortAnswer("Existing answer".into()));
        assert!(!can_turn_in_question(&question));
        question.submission.as_mut().unwrap().question_answer = None;
        question.question = Some(QuestionDetails::MultipleChoice(vec!["A".into()]));
        assert!(can_turn_in_question(&question));
        question.kind = CourseworkKind::Assignment;
        assert!(!can_turn_in_question(&question));
    }

    #[test]
    fn question_choice_order_and_answer_type_survive_wire_decoding() {
        let source = classwork::QuestionInfo {
            question_type: Some(2),
            multiple_choice: Some(classwork::MultipleChoice { choice: vec!["B".into(), "A".into()] }),
        };
        let read = classwork::QuestionInfo::decode(source.encode_to_vec().as_slice()).unwrap();
        assert_eq!(question_details(read), Some(QuestionDetails::MultipleChoice(vec!["B".into(), "A".into()])));
        assert_eq!(question_details(classwork::QuestionInfo { question_type: Some(1), multiple_choice: None }),
            Some(QuestionDetails::ShortAnswer));
        assert!(question_details(classwork::QuestionInfo { question_type: Some(2), multiple_choice: None }).is_none());
        for (kind, expected) in [
            (1, Some(QuestionAnswer::ShortAnswer("Explanation".into()))),
            (2, Some(QuestionAnswer::MultipleChoice("A".into()))),
            (0, None),
        ] {
            let raw = classwork::QuestionSubmission { question_answer_type: Some(kind),
                short_answer: Some("Explanation".into()), multiple_choice_answer: Some("A".into()) };
            assert_eq!(own_question_answer(classwork::QuestionSubmission::decode(raw.encode_to_vec().as_slice()).unwrap()), expected);
        }
    }

    #[test]
    fn class_folders_match_requested_classes_and_signed_in_user() {
        use classwork::*;
        let build = |course, user, inner_user, url: &str| {
            let key = |user| CourseUserId {
                course_id: Some(CourseId { id: Some(course) }),
                user_id: Some(UserId { id: Some(user) }),
            };
            CourseUserLookupResult {
                course_user_id: Some(key(user)),
                course_user: Some(CourseUser {
                    course_user_id: Some(key(inner_user)),
                    course_role: Some(2),
                    course_folder: Some(Folder { open_url: Some(url.into()) }),
                    ..Default::default()
                }),
            }
        };
        let good = "https://drive.google.com/drive/folders/test";
        let response = CourseUserLookupResponse { results: vec![
            build(1, 15, 15, good),
            build(2, 14, 14, good),
            build(3, 15, 14, good),
            build(4, 15, 15, "https://drive.google.com.example.invalid/folders/test"),
            build(5, 15, 15, "https://drive.google.com@other.example.invalid/folders/test"),
            build(6, 15, 15, good),
        ] };
        let decoded = CourseUserLookupResponse::decode(response.encode_to_vec().as_slice()).unwrap();
        assert_eq!(own_class_details(decoded.clone(), &BTreeSet::from([1]), 15).roles, BTreeMap::from([(1, CourseRole::Teacher)]));
        assert_eq!(own_class_details(decoded, &BTreeSet::from([1, 2, 3, 4, 5]), 15).folders,
            BTreeMap::from([(1, good.to_owned())]));
    }

    #[test]
    fn meet_link_requires_the_student_visible_state_and_google_meet_host() {
        let link = "https://meet.google.com/lookup/demo";
        for state in [None, Some(0), Some(1), Some(2), Some(3), Some(4)] {
            let info = wire::VideoCallInfo { video_call_url_state: state,
                video_call_url: Some(link.into()), ..Default::default() };
            assert_eq!(student_meet_url(info).as_deref(), (state == Some(3)).then_some(link));
        }
        let unsafe_link = wire::VideoCallInfo { video_call_url_state: Some(3),
            video_call_url: Some("https://meet.google.com.demo.invalid/lookup/session".into()),
            ..Default::default() };
        assert!(student_meet_url(unsafe_link).is_none());
    }

    #[test]
    fn a_late_rejection_does_not_invalidate_a_renewed_token() {
        let rejected = Arc::new(Zeroizing::new("old-fixture".to_owned()));
        let fresh = Arc::new(Zeroizing::new("new-fixture".to_owned()));
        let mut token = ClassroomToken { bearer: fresh.clone(), expires_at: Some(1_100) };
        assert!(!token.expires_within(1_000, 30));
        assert!(token.expires_within(1_070, 30));
        assert!(!token.invalidate(&rejected));
        assert!(!token.expires_within(1_000, 0));
        assert!(token.invalidate(&fresh));
        assert!(token.expires_within(1_000, 0));
    }

    #[tokio::test]
    async fn a_rejected_read_retries_once_using_the_concurrently_renewed_token() {
        use std::sync::atomic::AtomicUsize;
        use tokio::{net::TcpListener, sync::oneshot};
        use tokio_stream::wrappers::TcpListenerStream;
        use tonic::{Request, Response, transport::{Endpoint, Server}};
        use wire::homeroom_data_service_server::{HomeroomDataService, HomeroomDataServiceServer};

        struct ReadServer {
            credentials: Arc<Mutex<SessionCredentials>>,
            calls: Arc<AtomicUsize>,
        }

        #[tonic::async_trait]
        impl HomeroomDataService for ReadServer {
            async fn query_course(&self, request: Request<wire::QueryCourseRequest>)
                -> Result<Response<wire::QueryCourseResponse>, Status>
            {
                assert_eq!(request.get_ref(), &course_details_request());
                let bearer = request.metadata().get("authorization").unwrap();
                if self.calls.fetch_add(1, Ordering::Relaxed) == 0 {
                    assert_eq!(bearer, "Bearer old-fixture");
                    // Simulate another in-flight read finishing renewal ahead of this rejection.
                    self.credentials.lock().await.token = ClassroomToken {
                        bearer: Arc::new(Zeroizing::new("new-fixture".into())), expires_at: None,
                    };
                    return Err(Status::unauthenticated("synthetic expiry"));
                }
                assert_eq!(bearer, "Bearer new-fixture");
                Ok(Response::new(wire::QueryCourseResponse::default()))
            }

            async fn query_stream_item(&self, _: Request<wire::StreamItemQueryRequest>)
                -> Result<Response<wire::StreamItemQueryResponse>, Status>
            {
                Err(Status::unimplemented("outside this fixture"))
            }

            async fn query_submission(&self, _: Request<wire::QuerySubmissionRequest>)
                -> Result<Response<wire::QuerySubmissionResponse>, Status>
            {
                Err(Status::unimplemented("outside this fixture"))
            }
        }

        let credentials = Arc::new(Mutex::new(SessionCredentials {
            token: ClassroomToken { bearer: Arc::new(Zeroizing::new("old-fixture".into())), expires_at: None },
            failure: None,
        }));
        let calls = Arc::new(AtomicUsize::new(0));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = Endpoint::from_shared(format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let (stop, stopped) = oneshot::channel();
        let service = ReadServer { credentials: credentials.clone(), calls: calls.clone() };
        let server = tokio::spawn(Server::builder().add_service(HomeroomDataServiceServer::new(service))
            .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async { let _ = stopped.await; }));
        let session = LiveSession {
            snapshot: Snapshot::live(vec![]),
            account: "local-fixture@example.org".into(),
            credentials,
            needs_sign_in: Arc::new(AtomicBool::new(false)),
            channel: endpoint.connect().await.unwrap(),
        };
        assert!(refresh(&session).await.is_ok());
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert!(!session.needs_sign_in());
        drop(session);
        stop.send(()).unwrap();
        server.await.unwrap().unwrap();
    }

    #[tokio::test]
    #[ignore = "requires the owner's saved Google Workspace session and sends live read-only requests"]
    async fn live_saved_session_renews_after_simulated_expiry() {
        let account = remembered_account().expect("A saved Google Workspace account is required");
        let session = restore_session(account).await.expect("Saved session should restore");
        let previous = {
            let mut credentials = session.credentials.lock().await;
            credentials.token.expires_at = Some(0);
            credentials.token.bearer.clone()
        };

        let (first, second) = tokio::join!(session.bearer_for_request(None), session.bearer_for_request(None));
        let first = first.expect("First read acquires a renewed token");
        let second = second.expect("Concurrent read uses the renewed token");
        assert!(!Arc::ptr_eq(&first, &previous));
        assert!(Arc::ptr_eq(&first, &second));
        assert!(!session.needs_sign_in());

        let (courses, user) = tokio::join!(refresh(&session), check_current_user(&session));
        let courses = courses.expect("Renewed token should read courses");
        assert!(!courses.courses.is_empty());
        user.expect("Renewed token should read the signed-in user's identity");
        println!("Renewed in memory and verified course and identity reads; courses={}", courses.courses.len());
    }

    #[test]
    fn feedback_requires_active_private_comment_on_the_exact_submission() {
        let target = classwork::SubmissionId {
            id: Some(100),
            stream_item_id: Some(classwork::StreamItemId {
                id: Some(200),
                course_id: Some(classwork::CourseId { id: Some(300) }),
            }),
        };
        let feedback = classwork::Comment {
            comment_id: Some(classwork::CommentId {
                id: Some(4),
                submission_id: Some(target.clone()),
                ..Default::default()
            }),
            status: Some(2),
            comment: Some("Synthetic feedback".into()),
            visibility: Some(classwork::CommentVisibility { visibility_type: Some(1) }),
            ..Default::default()
        };
        assert_eq!(decode_private_feedback(feedback.clone(), &target).unwrap().0, 4);
        let mut wrong_course = target.clone();
        wrong_course.stream_item_id.as_mut().unwrap().course_id.as_mut().unwrap().id = Some(301);
        assert!(decode_private_feedback(feedback.clone(), &wrong_course).is_none());
        let mut public = feedback.clone();
        public.visibility = Some(classwork::CommentVisibility { visibility_type: Some(2) });
        assert!(decode_private_feedback(public, &target).is_none());
        let mut trashed = feedback;
        trashed.status = Some(3);
        assert!(decode_private_feedback(trashed, &target).is_none());
    }

    #[test]
    fn classwork_read_limits_the_request_to_active_published_items_in_one_class() {
        let request = classwork_request(412);
        assert_eq!(request.filter.len(), 1);
        let filter = classwork::StreamFilter::decode(request.filter[0].as_slice()).unwrap();
        assert_eq!(filter.course_id[0].id, Some(412));
        assert_eq!(filter.allowed_status, [2]);
        assert_eq!(filter.publication_status, [2]);
        let mask = classwork::StreamReadMask::decode(request.read_mask.unwrap().as_slice()).unwrap();
        assert_eq!(mask.assignment_mask.unwrap().stream_item_info_mask.unwrap().include_title, Some(true));
    }

    #[test]
    fn classwork_decode_refuses_items_from_other_courses() {
        let item = classwork::StreamItem {
            assignment: Some(classwork::Assignment {
                stream_item_info: Some(classwork::StreamItemInfo {
                    stream_item_id: Some(classwork::StreamItemId {
                        id: Some(9),
                        course_id: Some(classwork::CourseId { id: Some(412) }),
                    }),
                    title: Some("Synthetic assignment".into()),
                    ..Default::default()
                }),
                task_info: None,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(coursework(411, item.clone()).is_none());
        let decoded = coursework(412, item).unwrap();
        assert_eq!(decoded.id, "9");
        assert_eq!(decoded.title, "Synthetic assignment");
    }

    #[test]
    fn sign_in_normalizes_and_bounds_account_input() {
        assert_eq!(normalize_account(" Fixture@EXAMPLE.ORG ").unwrap(), "fixture@example.org");
        assert!(normalize_account("fixture@-example.org").is_err());
        assert!(normalize_account("fixture@example.org\nsecond@example.org").is_err());
    }
}

mod artwork;

fn artwork_host_is_allowed(url: &str) -> bool {
    let Some(host) = https_host(url) else { return false };
    host == "gstatic.com" || host.ends_with(".gstatic.com")
        || host == "googleusercontent.com" || host.ends_with(".googleusercontent.com")
}

mod gradebook;
pub use gradebook::{gradebook_coverage, load_course_grade};

mod rubric;
pub use rubric::load_rubric;

pub use artwork::{artwork_updates, load_artworks};


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
