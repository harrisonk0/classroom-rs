//! Native application state, startup, and shared message types.

mod attachments;
mod classwork;
mod comment_policy;
mod comments;
mod course;
mod course_data;
mod editors;
mod grading;
mod home;
mod item;
mod loading;
mod message;
mod navigation;
mod people;
mod rich_text;
mod screenshots;
mod settings;
mod stream;
mod stream_draft;
mod student_work;
mod ui;
mod update;

use attachments::{
    choose_attachment_file, DriveNavigation, DrivePicker, DriveSection, eligible_personal_copy_template,
    matching_personal_copy, trusted_attachment_host, validate_submission_file,
};
use classwork::topic_action_valid;
use comments::{append_comment_mention, edit_comment_draft};
use deadlines::{ClassFilterOption, TodoFilter, TodoGroup, WorkFilter};
use editors::{
    AssignmentEditor, AssignmentPublicationPrompt,
    ClassStreamSettingsEditor, CommentMentions, GradebookEdit, ItemTopicEditor, NewClassField,
    StudentWorkFilter, TeacherFeedbackThread, TeacherWork, TopicChoice,
};
use grading::{grade_input_matches_saved, grade_label, next_editable_gradebook_student};
use home::CourseDrag;
use message::Message;
use people::class_member_email_url;
use rich_text::{
    formatted_description, paragraph_start, paragraph_starts,
    stream_editor_binding,
};
use screenshots::{Shot, save_screenshot, schedule_capture};
use stream_draft::{CommentTarget, StreamDraft, StreamFormat};
use ui::{
    avatar, border, course_banner, course_color, coursework_row,
    coursework_row_with_preview, divider_style, empty_panel, first_initial, medium, nav_style,
    outline_style, outline_surface, pill, primary_style, quiet_style, search_style, surface,
    tab_style, truncate,
};

use chrono::{DateTime, Datelike, Days, Local, NaiveDate, NaiveDateTime, TimeDelta, TimeZone};
use classroom_rust::{
    classroom_links::{ClassroomLink, ItemLinkKind},
    desktop_auth::{self, CommentAudience, CourseContents, CoursePlacement, DriveDocumentKind, DriveFileAttachment, DriveFilePage, DriveFolder, DriveLocation, EmailNotificationSettings,
        EmailNotificationTarget, LiveSession, PublishedAnnouncement, StreamAccess},
    desktop_data::{
        self, ClassworkGroupId, Comment, Course, CourseMember, CourseRole, CourseState, Coursework,
        CourseGrade, CourseworkKind, DescriptionStyle, DriveSharing, GradeCalculation, QuestionAnswer, QuestionDetails, QuestionSharing, Rubric,
        Snapshot, SnapshotSource, StreamClassworkDisplay, Submission, SubmissionStatus, matching_choice, monday_of,
    },
};
use iced::{
    Alignment, Background, Border, Color, Element, Fill, Font, Length, Padding, Pixels,
    ContentFit, Event, Shadow, Subscription, Task, Theme, Vector, color, mouse,
    font::Weight,
    widget::{button, checkbox, column, container, image, mouse_area, pick_list, responsive, rich_text, row, rule, scrollable, space, span, stack, svg, text, text_editor, text_input, toggler, tooltip},
};
use std::{
    collections::{BTreeMap, BTreeSet}, fmt, fs, io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use iced::window;

mod deadlines;
#[cfg(test)]
use deadlines::calendar_agenda_heading;
#[cfg(test)]
use editors::local_schedule_millis;
mod icons;
use icons::{attachment_icon, classroom_mark, coursework_icon, icon, icon_action, Icon};

#[cfg(target_os = "macos")]
mod audio;

#[cfg(target_os = "macos")]
mod camera;

#[cfg(target_os = "macos")]
mod screen_recording;

#[cfg(target_os = "macos")]
mod grade_export;

const FG: Color = color!(0x202124);
const MUTED: Color = color!(0x5f6368);
const BORDER: Color = color!(0xdadce0);
const SOFT: Color = color!(0xf1f3f4);
const BLUE: Color = color!(0x1a73e8);
const ACTIVE_BG: Color = color!(0xe8f0fe);
const ACTIVE_FG: Color = color!(0x174ea6);
const WHITE: Color = Color::WHITE;
const CLASSROOM_FONT: Font = Font::with_name("Google Sans Flex");
const FORMATTED_FONT: Font = Font::with_name("Helvetica Neue");
const CARD_COLORS: [Color; 8] = [
    color!(0x18635d),
    color!(0x1768ad),
    color!(0x824b98),
    color!(0x97701e),
    color!(0x6958b3),
    color!(0x3e7690),
    color!(0x994767),
    color!(0x41785f),
];

pub(super) fn run() -> iced::Result {
    let mut arguments = std::env::args_os().skip(1);
    let first = arguments.next();
    let verify_live_course_count = first.as_deref().is_some_and(|flag| flag == "--verify-live-course-count");
    let screenshot_directory = match (first, arguments.next(), arguments.next()) {
        (Some(flag), Some(path), None) if flag == "--screenshots" => Some(PathBuf::from(path)),
        _ => None,
    };
    iced::application(
        move || App::boot(screenshot_directory.clone(), verify_live_course_count), App::update, App::view,
    )
        .title("Classroom for Mac")
        .theme(Theme::Light)
        .font(include_bytes!("../../../assets/fonts/GoogleSansFlex.ttf").as_slice())
        .default_font(CLASSROOM_FONT)
        .subscription(App::subscription)
        .window(window::Settings {
            size: iced::Size::new(1200.0, 810.0),
            min_size: Some(iced::Size::new(810.0, 565.0)),
            ..Default::default()
        })
        .centered()
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Home,
    Join,
    CreateClass,
    Archived,
    Calendar,
    ToDo,
    Settings,
    Course(i64),
    Item(i64),
}

impl Screen {
    fn uses_assignment_overview(self) -> bool {
        matches!(self, Self::Home | Self::ToDo | Self::Calendar)
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Stream,
    Classwork,
    People,
    YourWork,
    Grades,
    ClassSettings,
}

impl Tab {
    const COURSE_TABS: [Self; 4] = [Self::Stream, Self::Classwork, Self::People, Self::Grades];

    fn name(self) -> &'static str {
        match self {
            Self::Stream => "Stream",
            Self::Classwork => "Classwork",
            Self::People => "People",
            Self::YourWork => "Your work",
            Self::Grades => "Grades",
            Self::ClassSettings => "Class settings",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DetailRead {
    Roster(i64),
    Submissions(i64),
    PersonalCopy(i64, i64),
    Comments(i64, i64),
    PrivateFeedback(i64, i64),
    Rubric(i64, i64),
    OverallGrade(i64),
    TeacherDrafts(i64),
    TeacherSubmissions(i64, i64),
    TeacherFeedback(desktop_auth::TeacherFeedbackKey),
    TeacherGradebook(i64),
    ClassStreamSettings(i64),
    StudentMuting(i64),
    TeacherTopics(i64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PageRead {
    Coursework(i64),
    Assignments,
    Detail(DetailRead),
    Invitations,
    ClassNotifications,
    EmailNotifications,
    StreamAccess(i64),
}

impl PageRead {
    fn message(self) -> Message {
        match self {
            Self::Coursework(id) => Message::LoadCoursework(id),
            Self::Assignments => Message::LoadAssignments,
            Self::Detail(read) => Message::LoadDetail(read),
            Self::Invitations => Message::LoadInvitations,
            Self::ClassNotifications => Message::LoadClassNotifications,
            Self::EmailNotifications => Message::LoadEmailNotifications,
            Self::StreamAccess(id) => Message::LoadStreamAccess(id),
        }
    }
}

#[derive(Clone)]
enum DetailData {
    Roster(Vec<CourseMember>),
    Submissions(Vec<(String, Submission)>),
    PersonalCopy(String, bool),
    Comments(Vec<Comment>),
    PrivateFeedback(Vec<Comment>),
    Rubric(Option<Rubric>),
    OverallGrade(Option<CourseGrade>),
    TeacherDrafts(Vec<desktop_auth::TeacherDraft>),
    TeacherSubmissions(Vec<desktop_auth::TeacherSubmission>),
    TeacherFeedback(Vec<Comment>),
    TeacherGradebook(desktop_auth::TeacherGradebook),
    ClassStreamSettings(desktop_auth::TeacherClassSettings),
    StudentMuting(BTreeMap<i64, bool>),
    TeacherTopics(Vec<desktop_data::Topic>),
}

#[derive(Clone)]
enum MembershipChange {
    Created(i64),
    Joined(i64),
    Left(i64),
}

#[cfg(target_os = "macos")]
#[derive(Clone)]
enum CaptureAction {
    Start,
    StartScreen,
    StartCamera,
    StartCameraRecording,
    CameraFrame,
    CameraPermission(u64, bool),
    CameraMicrophonePermission(u64, bool),
    StopCamera,
    MovieCaptured(u64, &'static str, Result<Option<screen_recording::Draft>, String>),
    Permission(u64, bool),
    Stop,
    TimeLimit(u64),
    Preview,
    StopPreview,
    ScreenPreviewFinished(u64, Result<(), String>),
    Discard,
    Attach,
    Attached(i64, u64, Result<(), String>),
}

#[cfg(target_os = "macos")]
enum CaptureState {
    AskingPermission,
    AskingCamera,
    Recording(audio::Recording, iced::task::Handle),
    Draft(audio::Draft),
    RecordingScreen { _handle: iced::task::Handle },
    RecordingCamera { control: camera::Control, phase: CameraPhase, frame: Option<image::Handle> },
    MovieDraft { label: &'static str, draft: screen_recording::Draft },
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, PartialEq, Eq)]
enum CameraPhase {
    Framing,
    AskingMicrophone,
    Recording,
    Finishing,
}

#[cfg(target_os = "macos")]
impl CaptureState {
    fn recording_file(&self) -> Option<(Arc<tempfile::TempDir>, PathBuf)> {
        match self {
            Self::Draft(draft) => {
                draft.stop_preview();
                Some(draft.file())
            }
            Self::MovieDraft { draft, .. } => Some(draft.file()),
            _ => None,
        }
    }
}

struct AssignmentBatch {
    handle: iced::task::Handle,
    remaining: BTreeSet<i64>,
    failed_classwork: usize,
    failed_status: usize,
}

impl AssignmentBatch {
    fn error(&self) -> Option<String> {
        match (self.failed_classwork, self.failed_status) {
            (0, 0) => None,
            (0, count) => Some(format!("Couldn’t load submission status for {count} classes.")),
            (count, 0) => Some(format!("Couldn’t load classwork for {count} classes.")),
            (classes, statuses) => Some(format!(
                "Couldn’t load classwork for {classes} classes or submission status for {statuses} other classes."
            )),
        }
    }
}

struct App {
    screen: Screen,
    classroom_link_input: Option<String>,
    classroom_link_generation: u64,
    checking_classroom_link: bool,
    tab: Tab,
    sidebar: bool,
    search: String,
    alphabetical: bool,
    show_hidden_classes: bool,
    open_course_menu: Option<i64>,
    move_course_menu: Option<i64>,
    moving_course: Option<i64>,
    course_drag: Option<CourseDrag>,
    confirm_hide_course: Option<i64>,
    confirm_class_archive: Option<i64>,
    changing_class_archive: Option<(i64, bool)>,
    changing_course_visibility: Option<(i64, bool)>,
    email: String,
    demo: bool,
    busy: bool,
    refreshing: bool,
    loading_coursework: Option<i64>,
    loading_stream_access: BTreeSet<i64>,
    stream_access: BTreeMap<i64, StreamAccess>,
    stream_viewer_id: Option<i64>,
    stream_access_generation: u64,
    stream_drafts: BTreeMap<i64, StreamDraft>,
    open_stream_composer: Option<i64>,
    sending_stream_post: Option<i64>,
    choosing_stream_file: Option<i64>,
    uploading_stream_file: Option<i64>,
    drive_picker: Option<DrivePicker>,
    drive_picker_generation: u64,
    confirming_stream_delete: Option<(i64, i64)>,
    deleting_stream_post: Option<(i64, i64)>,
    assignment_batch: Option<AssignmentBatch>,
    assignment_generation: u64,
    assignments_completed: BTreeSet<i64>,
    assignments_loaded: bool,
    queued_page_load: bool,
    loading_detail: Option<DetailRead>,
    rubrics: BTreeMap<(i64, i64), Option<Rubric>>,
    overall_grades: BTreeMap<i64, Option<CourseGrade>>,
    rubric_open: bool,
    open_criteria: BTreeSet<usize>,
    page_errors: BTreeMap<PageRead, String>,
    comment_drafts: BTreeMap<(i64, i64), text_editor::Content>,
    empty_comment_draft: text_editor::Content,
    people_email_generation: u64,
    people_email_pending: Option<(i64, i64)>,
    comment_mentions_generation: u64,
    comment_mentions: Option<CommentMentions>,
    comment_unconfirmed: BTreeSet<(i64, i64)>,
    posting_comment: bool,
    class_code: String,
    class_creation_generation: u64,
    class_creation_allowed: bool,
    opening_drive_file: Option<String>,
    preparing_drive_preview: bool,
    saving_drive_file: bool,
    new_class: desktop_auth::NewClass,
    invitations: Option<Vec<Course>>,
    loading_invitations: bool,
    invitation_generation: u64,
    class_notifications: Option<BTreeMap<i64, bool>>,
    loading_class_notifications: bool,
    notifications_generation: u64,
    saving_class_notifications: Option<i64>,
    email_notifications: Option<EmailNotificationSettings>,
    loading_email_notifications: bool,
    saving_email_notification: Option<EmailNotificationTarget>,
    opening_joined_course: Option<i64>,
    confirming_decline: Option<i64>,
    confirming_unenroll: Option<i64>,
    changing_membership: bool,
    pending_submission: Option<(i64, i64, desktop_auth::SubmissionAction)>,
    new_submission_document_open: bool,
    short_answer_drafts: BTreeMap<(i64, i64), text_editor::Content>,
    short_answer_revisions: BTreeMap<(i64, i64), desktop_auth::ShortAnswerRevision>,
    empty_short_answer: text_editor::Content,
    choice_drafts: BTreeMap<(i64, i64), String>,
    confirming_question_answer: Option<(i64, i64)>,
    sending_submission: bool,
    attachment_link: String,
    submission_file: Option<(i64, i64, PathBuf)>,
    #[cfg(target_os = "macos")]
    submission_capture: Option<CaptureState>,
    #[cfg(target_os = "macos")]
    capture_generation: u64,
    choosing_submission_file: bool,
    private_feedback: BTreeMap<(i64, i64), Vec<Comment>>,
    private_drafts: BTreeMap<(i64, i64), text_editor::Content>,
    confirming_comment_delete: Option<CommentTarget>,
    deleting_comment: Option<CommentTarget>,
    private_unconfirmed: BTreeSet<(i64, i64)>,
    pending_remove_material: Option<(i64, i64, i64)>,
    personal_copy_checks: BTreeMap<(i64, i64), (String, bool)>,
    pending_personal_copy: Option<(i64, i64, String)>,
    personal_copy_generation: u64,
    personal_copy_loading_generation: Option<u64>,
    submissions_checked: BTreeSet<i64>,
    selected_topic: Option<String>,
    classwork_toggles: BTreeSet<String>,
    collapsed_classwork_groups: BTreeSet<ClassworkGroupId>,
    todo_filter: TodoFilter,
    work_filter: WorkFilter,
    todo_class: Option<i64>,
    collapsed_todo_groups: BTreeSet<(TodoFilter, TodoGroup)>,
    calendar_monday: NaiveDate,
    calendar_class: Option<i64>,
    item_id: Option<String>,
    previous_screen: Option<Screen>,
    artwork: BTreeMap<String, image::Handle>,
    artwork_generation: u64,
    stream_profiles_requested: BTreeSet<i64>,
    stream_profiles: BTreeMap<i64, desktop_auth::UserProfile>,
    stream_profile_pictures: BTreeMap<String, image::Handle>,
    class_folders: BTreeMap<i64, String>,
    class_roles: BTreeMap<i64, CourseRole>,
    teacher_drafts: BTreeMap<i64, Vec<desktop_auth::TeacherDraft>>,
    open_teacher_draft: Option<(i64, String)>,
    assignment_editor: Option<AssignmentEditor>,
    assignment_publication: Option<AssignmentPublicationPrompt>,
    teacher_work: Option<TeacherWork>,
    teacher_feedback: BTreeMap<desktop_auth::TeacherFeedbackKey, TeacherFeedbackThread>,
    gradebooks: BTreeMap<i64, desktop_auth::TeacherGradebook>,
    gradebook_page: usize,
    gradebook_student: Option<(i64, i64, i64)>,
    class_stream_settings: Option<ClassStreamSettingsEditor>,
    student_muting: BTreeMap<i64, BTreeMap<i64, bool>>,
    pending_student_mute: Option<(i64, i64, bool)>,
    changing_student_mute: bool,
    teacher_topics: BTreeMap<i64, Vec<desktop_data::Topic>>,
    topic_manager: bool,
    topic_editor: Option<(i64, desktop_auth::TeacherTopicAction)>,
    item_topic_editor: Option<ItemTopicEditor>,
    saving_topic: bool,
    saving_stream_settings: bool,
    gradebook_edit_mode: bool,
    gradebook_edit: Option<GradebookEdit>,
    exporting_grades: bool,
    saving_teacher_grade: bool,
    saving_assignment_draft: bool,
    class_folders_generation: u64,
    class_teachers: BTreeMap<i64, String>,
    teachers_generation: u64,
    class_folders_failed: bool,
    notice: Option<String>,
    error: Option<String>,
    screenshot_directory: Option<PathBuf>,
    verify_live_course_count: bool,
    verification_submitted: bool,
    session: Option<Arc<LiveSession>>,
    snapshot: Option<Snapshot>,
}

impl App {
    fn boot(screenshot_directory: Option<PathBuf>, verify_live_course_count: bool) -> (Self, Task<Message>) {
        let synthetic = screenshot_directory.is_some();
        let mut app = Self {
            screen: Screen::Home,
            classroom_link_input: None,
            classroom_link_generation: 0,
            checking_classroom_link: false,
            tab: Tab::Stream,
            sidebar: true,
            search: String::new(),
            alphabetical: false,
            show_hidden_classes: false,
            open_course_menu: None,
            move_course_menu: None,
            moving_course: None,
            course_drag: None,
            confirm_hide_course: None,
            confirm_class_archive: None,
            changing_class_archive: None,
            changing_course_visibility: None,
            email: String::new(),
            demo: synthetic,
            busy: false,
            refreshing: false,
            loading_coursework: None,
            loading_stream_access: BTreeSet::new(),
            stream_access: BTreeMap::new(),
            stream_viewer_id: None,
            stream_access_generation: 0,
            stream_drafts: BTreeMap::new(),
            open_stream_composer: None,
            sending_stream_post: None,
            choosing_stream_file: None,
            uploading_stream_file: None,
            drive_picker: None,
            drive_picker_generation: 0,
            confirming_stream_delete: None,
            deleting_stream_post: None,
            assignment_batch: None,
            assignment_generation: 0,
            assignments_completed: BTreeSet::new(),
            assignments_loaded: false,
            queued_page_load: false,
            loading_detail: None,
            rubrics: BTreeMap::new(),
            overall_grades: BTreeMap::new(),
            rubric_open: false,
            open_criteria: BTreeSet::new(),
            page_errors: BTreeMap::new(),
            comment_drafts: BTreeMap::new(),
            empty_comment_draft: text_editor::Content::new(),
            people_email_generation: 0,
            people_email_pending: None,
            comment_mentions_generation: 0,
            comment_mentions: None,
            comment_unconfirmed: BTreeSet::new(),
            posting_comment: false,
            class_code: String::new(),
            class_creation_generation: 0,
            class_creation_allowed: false,
            opening_drive_file: None,
            preparing_drive_preview: false,
            saving_drive_file: false,
            new_class: desktop_auth::NewClass::default(),
            invitations: None,
            loading_invitations: false,
            invitation_generation: 0,
            class_notifications: None,
            loading_class_notifications: false,
            notifications_generation: 0,
            saving_class_notifications: None,
            email_notifications: None,
            loading_email_notifications: false,
            saving_email_notification: None,
            opening_joined_course: None,
            confirming_decline: None,
            confirming_unenroll: None,
            changing_membership: false,
            pending_submission: None,
            new_submission_document_open: false,
            short_answer_drafts: BTreeMap::new(),
            short_answer_revisions: BTreeMap::new(),
            empty_short_answer: text_editor::Content::new(),
            choice_drafts: BTreeMap::new(),
            confirming_question_answer: None,
            sending_submission: false,
            attachment_link: String::new(),
            submission_file: None,
            #[cfg(target_os = "macos")]
            submission_capture: None,
            #[cfg(target_os = "macos")]
            capture_generation: 0,
            choosing_submission_file: false,
            private_feedback: BTreeMap::new(),
            private_drafts: BTreeMap::new(),
            confirming_comment_delete: None,
            deleting_comment: None,
            private_unconfirmed: BTreeSet::new(),
            pending_remove_material: None,
            personal_copy_checks: BTreeMap::new(),
            pending_personal_copy: None,
            personal_copy_generation: 0,
            personal_copy_loading_generation: None,
            submissions_checked: BTreeSet::new(),
            selected_topic: None,
            classwork_toggles: BTreeSet::new(),
            collapsed_classwork_groups: BTreeSet::new(),
            todo_filter: TodoFilter::default(),
            work_filter: WorkFilter::default(),
            todo_class: None,
            collapsed_todo_groups: BTreeSet::new(),
            calendar_monday: monday_of(Local::now().date_naive()),
            calendar_class: None,
            item_id: None,
            previous_screen: None,
            artwork: BTreeMap::new(),
            artwork_generation: 0,
            stream_profiles_requested: BTreeSet::new(),
            stream_profiles: BTreeMap::new(),
            stream_profile_pictures: BTreeMap::new(),
            class_folders: if synthetic {
                BTreeMap::from([(1, "https://drive.google.com/drive/folders/demo".into())])
            } else {
                BTreeMap::new()
            },
            class_folders_generation: 0,
            class_roles: BTreeMap::new(),
            teacher_drafts: BTreeMap::new(),
            open_teacher_draft: None,
            assignment_editor: None,
            assignment_publication: None,
            teacher_work: None,
            teacher_feedback: BTreeMap::new(),
            gradebooks: BTreeMap::new(),
            gradebook_page: 0,
            gradebook_student: None,
            class_stream_settings: None,
            student_muting: BTreeMap::new(),
            pending_student_mute: None,
            changing_student_mute: false,
            teacher_topics: BTreeMap::new(),
            topic_manager: false,
            topic_editor: None,
            item_topic_editor: None,
            saving_topic: false,
            saving_stream_settings: false,
            gradebook_edit_mode: false,
            gradebook_edit: None,
            exporting_grades: false,
            saving_teacher_grade: false,
            saving_assignment_draft: false,
            class_teachers: if synthetic {
                BTreeMap::from([(1, "Morgan Lane".into()), (2, "Alex Smith".into()), (3, "Jamie Clarke".into())])
            } else { BTreeMap::new() },
            teachers_generation: 0,
            class_folders_failed: false,
            notice: None,
            error: None,
            screenshot_directory,
            verify_live_course_count,
            verification_submitted: false,
            session: None,
            snapshot: synthetic.then(Snapshot::demo),
        };
        if synthetic
            && let Some(course) = app.snapshot.as_mut().and_then(|data| data.courses.first_mut())
        {
            course.meet_url = Some("https://meet.google.com/lookup/demo".into());
        }
        if synthetic && std::env::var_os("CLASSROOM_CLIP_FIXTURE").is_some()
            && let Some(course) = app.snapshot.as_mut().and_then(|data| data.courses.first_mut())
        {
            let key = "fixture:tall-banner".to_owned();
            if let Some(theme) = &mut course.theme {
                theme.artwork_url = Some(key.clone());
                app.artwork.insert(key, image::Handle::from_rgba(48, 768, [255, 0, 255, 255].repeat(48 * 768)));
            }
        }
        let task = if synthetic {
            schedule_capture(Shot::Dashboard)
        } else if !verify_live_course_count {
            if let Some(account) = desktop_auth::remembered_account() {
                app.email = account.clone();
                app.busy = true;
                Task::perform(desktop_auth::restore_session(account), |result| Message::SignedIn(result.map(Arc::new)))
            } else {
                Task::none()
            }
        } else {
            Task::none()
        };
        (app, task)
    }

    fn has_pending_request(&self) -> bool {
        #[cfg(target_os = "macos")]
        if matches!(self.submission_capture, Some(CaptureState::AskingPermission | CaptureState::Recording(..)
            | CaptureState::AskingCamera | CaptureState::RecordingCamera { .. } | CaptureState::RecordingScreen { .. })) {
            return true;
        }
        self.busy || self.refreshing || self.changing_membership || self.posting_comment || self.choosing_submission_file
            || self.saving_drive_file || self.preparing_drive_preview
            || self.saving_assignment_draft
            || self.saving_teacher_grade
            || self.saving_stream_settings
            || self.changing_student_mute
            || self.saving_topic
            || self.exporting_grades
            || self.sending_submission || self.loading_coursework.is_some() || self.loading_detail.is_some()
            || self.saving_class_notifications.is_some() || self.saving_email_notification.is_some()
            || self.sending_stream_post.is_some() || self.deleting_stream_post.is_some()
            || self.changing_course_visibility.is_some() || self.moving_course.is_some()
            || self.changing_class_archive.is_some()
            || self.choosing_stream_file.is_some() || self.uploading_stream_file.is_some()
            || self.deleting_comment.is_some()
    }

    fn subscription(&self) -> Subscription<Message> {
        let drag = if self.course_drag.is_some() {
            iced::event::listen_with(|event, _, _| match event {
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => Some(Message::DropCourse),
            Event::Window(window::Event::Unfocused) | Event::Mouse(mouse::Event::CursorLeft) =>
                Some(Message::CancelCourseDrag),
            _ => None,
            })
        } else {
            Subscription::none()
        };
        let edit_keys = if self.gradebook_edit.is_some() {
            iced::event::listen_with(|event, _, _| match event {
                Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape), ..
                }) => Some(Message::CancelGradebookEdit),
                _ => None,
            })
        } else {
            Subscription::none()
        };
        let file_drop = iced::event::listen_with(|event, _, _| match event {
            Event::Window(window::Event::FileDropped(path)) => Some(Message::SubmissionFileDropped(path)),
            _ => None,
        });
        let subscriptions = Subscription::batch([drag, edit_keys, file_drop]);
        #[cfg(target_os = "macos")]
        if matches!(self.submission_capture, Some(CaptureState::RecordingCamera { .. })) {
            let frames = iced::time::every(Duration::from_millis(100))
                .map(|_| Message::Capture(CaptureAction::CameraFrame));
            return Subscription::batch([subscriptions, frames]);
        }
        subscriptions
    }

    fn cancel_assignment_batch(&mut self) {
        if let Some(batch) = self.assignment_batch.take() {
            batch.handle.abort();
            self.assignment_generation = self.assignment_generation.wrapping_add(1);
        }
    }

    fn reset_assignment_reads(&mut self) {
        self.cancel_assignment_batch();
        self.assignments_completed.clear();
        self.assignments_loaded = false;
        self.page_errors.remove(&PageRead::Assignments);
    }

    fn can_sign_in(&self) -> bool {
        !self.has_pending_request() && !self.verification_submitted && !self.email.trim().is_empty()
    }

    fn can_open_shared_question(&self, course: i64) -> bool {
        self.screenshot_directory.is_some()
            || shared_question_read_is_current(
                self.can_read_pages(),
                self.class_roles.get(&course).copied(),
                self.submissions_checked.contains(&course),
                self.page_errors.contains_key(&PageRead::Detail(DetailRead::Submissions(course))),
                self.loading_detail == Some(DetailRead::Submissions(course)),
            )
    }

    fn can_read_pages(&self) -> bool {
        self.session.is_some() && !self.verify_live_course_count && !self.demo
            && self.snapshot.as_ref().is_some_and(|snapshot| snapshot.source == SnapshotSource::Live)
    }

    fn page_read_status(
        &self, read: PageRead, available: bool, pending: &'static str,
    ) -> Option<Element<'_, Message>> {
        if !self.can_read_pages() {
            return None;
        }
        if let Some(error) = self.page_errors.get(&read) {
            return Some(row![
                text(error).size(14).color(color!(0x9b2722)).width(Fill),
                button("Retry").padding([8, 12]).style(outline_style)
                    .on_press_maybe((!self.has_pending_request()).then_some(read.message())),
            ].spacing(12).align_y(Alignment::Center).into());
        }
        let loading = match read {
            PageRead::Coursework(id) => self.loading_coursework == Some(id),
            PageRead::Assignments => self.assignment_batch.is_some(),
            PageRead::Detail(detail) => self.loading_detail == Some(detail),
            PageRead::Invitations => self.loading_invitations,
            PageRead::ClassNotifications => self.loading_class_notifications,
            PageRead::EmailNotifications => self.loading_email_notifications,
            PageRead::StreamAccess(course) => self.loading_stream_access.contains(&course),
        };
        (loading || !available).then(|| text(pending).size(14).color(MUTED).into())
    }

    fn unavailable_screen(
        &self, title: &'static str, glyph: Icon, body: &'static str,
    ) -> Element<'_, Message> {
        scrollable(column![
            text(title).size(27).color(FG),
            empty_panel(glyph, "No live data available", body),
        ].spacing(22).padding(26).width(Fill)).width(Fill).height(Fill).into()
    }
}

fn has_submission(item: &Coursework) -> bool {
    matches!(item.kind, CourseworkKind::Assignment | CourseworkKind::Question)
}

fn home_course(course: &Course) -> bool {
    course.shows_on_home()
}

fn assignment_is_mark_done(action: desktop_auth::SubmissionAction, work: Option<&Submission>) -> bool {
    action == desktop_auth::SubmissionAction::TurnIn && work.is_some_and(|work| work.attachments.is_empty())
}

fn item_link_kind(item: &Coursework) -> Option<ItemLinkKind> {
    match (&item.kind, &item.question) {
        (CourseworkKind::Assignment, _) => Some(ItemLinkKind::Assignment),
        (CourseworkKind::Question, Some(QuestionDetails::ShortAnswer)) => Some(ItemLinkKind::ShortAnswer),
        (CourseworkKind::Question, Some(QuestionDetails::MultipleChoice(_))) => Some(ItemLinkKind::MultipleChoice),
        (CourseworkKind::Material, _) => Some(ItemLinkKind::Material),
        (CourseworkKind::Announcement, _) => Some(ItemLinkKind::Announcement),
        _ => None,
    }
}

fn shared_question_link(item: &Coursework) -> Option<(&'static str, String)> {
    if item.kind != CourseworkKind::Question || item.question_sharing != Some(QuestionSharing::Course) {
        return None;
    }
    let submission = item.submission.as_ref()?;
    if !matches!(submission.status,
        SubmissionStatus::TurnedIn | SubmissionStatus::TurnedInLate
        | SubmissionStatus::Returned | SubmissionStatus::ReturnedLate)
    {
        return None;
    }
    let (label, kind) = match (&item.question, &submission.question_answer) {
        (Some(QuestionDetails::ShortAnswer), Some(QuestionAnswer::ShortAnswer(answer))) if !answer.trim().is_empty() =>
            ("See classmate answers in Classroom", ItemLinkKind::ShortAnswer),
        (Some(QuestionDetails::MultipleChoice(choices)), Some(QuestionAnswer::MultipleChoice(answer)))
            if matching_choice(choices, answer).is_some() =>
            ("See class summary in Classroom", ItemLinkKind::MultipleChoice),
        _ => return None,
    };
    let url = ClassroomLink::Item {
        course: item.course_id, item: item.id.parse().ok()?, kind,
    }.web_url()?;
    Some((label, url))
}

fn shared_question_read_is_current(
    connected: bool, role: Option<CourseRole>, checked: bool, failed: bool, loading: bool,
) -> bool {
    connected && role == Some(CourseRole::Student) && checked && !failed && !loading
}

fn hidden_course(course: &Course) -> bool {
    course.state.shows_on_home() && course.hidden == Some(true)
}

fn valid_stream_draft(body: &str) -> bool {
    let body = body.trim();
    !body.is_empty() && body.len() <= 10_000
}

fn valid_host_label(label: &str) -> bool {
    !label.is_empty() && !label.starts_with('-') && !label.ends_with('-')
        && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn cache_path() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    Some(home.join("Library/Application Support/Classroom for Mac/offline.sqlite3"))
}

#[cfg(test)]
mod tests;
