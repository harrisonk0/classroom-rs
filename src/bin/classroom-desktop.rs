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

#[cfg(target_os = "macos")]
#[path = "classroom-desktop/audio.rs"]
mod audio;

#[cfg(target_os = "macos")]
#[path = "classroom-desktop/camera.rs"]
mod camera;

#[cfg(target_os = "macos")]
#[path = "classroom-desktop/screen_recording.rs"]
mod screen_recording;

#[cfg(target_os = "macos")]
#[path = "classroom-desktop/grade_export.rs"]
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

fn main() -> iced::Result {
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
        .font(include_bytes!("../../assets/fonts/GoogleSansFlex.ttf").as_slice())
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TodoFilter {
    #[default]
    Assigned,
    Missing,
    Done,
}

impl TodoFilter {
    const ALL: [Self; 3] = [Self::Assigned, Self::Missing, Self::Done];

    fn label(self) -> &'static str {
        match self {
            Self::Assigned => "Assigned",
            Self::Missing => "Missing",
            Self::Done => "Done",
        }
    }

    fn from_status(status: Option<SubmissionStatus>) -> Option<Self> {
        match status? {
            SubmissionStatus::Assigned | SubmissionStatus::Reclaimed => Some(Self::Assigned),
            SubmissionStatus::Missing => Some(Self::Missing),
            SubmissionStatus::TurnedIn | SubmissionStatus::Returned | SubmissionStatus::TurnedInLate
                | SubmissionStatus::ReturnedLate | SubmissionStatus::Excused => Some(Self::Done),
            SubmissionStatus::Unknown => None,
        }
    }

    fn groups(self) -> &'static [TodoGroup] {
        use TodoGroup::*;
        match self {
            Self::Assigned => &[NoDueDate, ThisWeek, NextWeek, Later, LastWeek, Earlier, StatusUnavailable],
            Self::Missing => &[ThisWeek, LastWeek, Earlier, NoDueDate, NextWeek, Later],
            Self::Done => &[NoDueDate, DoneEarly, ThisWeek, LastWeek, Earlier, NextWeek, Later],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TodoGroup {
    NoDueDate,
    DoneEarly,
    ThisWeek,
    NextWeek,
    Later,
    LastWeek,
    Earlier,
    StatusUnavailable,
}

impl TodoGroup {
    fn label(self) -> &'static str {
        match self {
            Self::NoDueDate => "No due date",
            Self::DoneEarly => "Done early",
            Self::ThisWeek => "This week",
            Self::NextWeek => "Next week",
            Self::Later => "Later",
            Self::LastWeek => "Last week",
            Self::Earlier => "Earlier",
            Self::StatusUnavailable => "Status unavailable",
        }
    }

    fn for_deadline(
        due: Option<NaiveDate>, today: NaiveDate, completed: bool,
    ) -> Self {
        let Some(due) = due else { return Self::NoDueDate };
        if completed && due > today {
            return Self::DoneEarly;
        }
        let weeks = (monday_of(due) - monday_of(today)).num_weeks();
        match weeks {
            0 => Self::ThisWeek,
            1 => Self::NextWeek,
            -1 => Self::LastWeek,
            weeks if weeks > 1 => Self::Later,
            _ => Self::Earlier,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum WorkFilter {
    #[default]
    All,
    Assigned,
    ReturnedWithGrade,
    Missing,
}

impl WorkFilter {
    const ALL: [Self; 4] = [Self::All, Self::Assigned, Self::ReturnedWithGrade, Self::Missing];

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Assigned => "Assigned",
            Self::ReturnedWithGrade => "Returned with grade",
            Self::Missing => "Missing",
        }
    }

    fn includes(self, item: &Coursework) -> bool {
        let status = TodoFilter::from_status(item.submission.as_ref().map(|submission| submission.status));
        match self {
            Self::All => true,
            Self::Assigned => status == Some(TodoFilter::Assigned),
            Self::ReturnedWithGrade => item.submission.as_ref().is_some_and(|submission| submission.grade.is_some()),
            Self::Missing => status == Some(TodoFilter::Missing),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct ClassFilterOption {
    id: Option<i64>,
    label: String,
}

impl fmt::Display for ClassFilterOption {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DetailRead {
    Roster(i64),
    Submissions(i64),
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

#[derive(Clone)]
enum Message {
    #[cfg(target_os = "macos")]
    Capture(CaptureAction),
    Navigate(Screen),
    ShowClassroomLink,
    EditClassroomLink(String),
    OpenClassroomLink,
    CloseClassroomLink,
    CopyClassroomLink(ClassroomLink),
    OpenYourWork(i64),
    OpenGradebook(i64),
    OpenGradebookCell(i64, i64, i64),
    GradebookColumnPage(usize),
    SetCoursePostingPolicy(desktop_auth::StreamPostingPolicy),
    SetCourseStreamDisplay(StreamClassworkDisplay),
    EditCourseTitle(String),
    EditCourseSection(String),
    EditCourseDescription(text_editor::Action),
    EditCourseRoom(String),
    SaveCourseStreamSettings,
    CourseStreamSettingsSaved(i64, Result<desktop_auth::TeacherClassSettings, String>),
    ConfirmStudentMute(i64, i64, bool),
    CancelStudentMute,
    ChangeStudentMute(i64, i64, bool),
    StudentMuteChanged(i64, i64, Result<bool, String>),
    ToggleTopicManager,
    OpenTopicEditor(i64, desktop_auth::TeacherTopicAction),
    EditTopicName(String),
    CloseTopicEditor,
    SaveTopicEditor,
    TeacherTopicChanged(i64, Option<String>, Result<Vec<desktop_data::Topic>, String>),
    MoveTeacherTopic(i64, String, desktop_auth::TopicOrderDirection),
    OpenItemTopic(i64, String, bool),
    SelectItemTopic(TopicChoice),
    CloseItemTopic,
    SaveItemTopic,
    ItemTopicSaved(desktop_auth::TopicItemTarget, Result<Option<String>, String>),
    ReorderClasswork(i64, String, desktop_auth::ClassworkOrderDirection),
    ClassworkReordered(i64, Result<CourseContents, String>),
    GradebookEditMode(bool),
    EditGradebookCell(i64, i64, i64),
    GradebookGradeInput(String),
    CancelGradebookEdit,
    SaveGradebookEdit(bool),
    GradebookEditSaved(i64, i64, i64, i64, f64, bool, Result<(), String>),
    #[cfg(target_os = "macos")]
    ExportGrades(i64, Option<i64>),
    #[cfg(target_os = "macos")]
    GradesExported(Result<bool, String>),
    SetTab(Tab),
    ToggleNavigation,
    Search(String),
    SortCourses,
    ToggleHiddenClasses,
    ToggleCourseMenu(i64),
    ChooseCourseMove(i64),
    MoveCourse(i64, CoursePlacement),
    CourseMoved(i64, Result<Vec<Course>, String>),
    BeginCourseDrag(i64),
    EnterCourseDropTarget(i64),
    ExitCourseDropTarget(i64),
    DropCourse,
    CancelCourseDrag,
    ConfirmHideCourse(i64),
    ChangeCourseVisibility(i64, bool),
    CourseVisibilityChanged(i64, bool, Result<(), String>),
    ConfirmClassArchive(i64),
    ChangeClassArchive(i64, bool),
    ClassArchiveChanged(i64, bool, Result<CourseState, String>),
    Email(String),
    SignIn,
    SignedIn(Result<Arc<LiveSession>, String>),
    Reconnect,
    Reconnected(Result<(), String>),
    Refresh,
    Refreshed(Result<Snapshot, String>),
    LoadCoursework(i64),
    LoadStreamAccess(i64),
    StreamAccessLoaded(u64, i64, Result<(i64, StreamAccess), String>),
    OpenStreamComposer(i64),
    EditStreamDraft(i64, text_editor::Action),
    FormatStreamDraft(i64, StreamFormat),
    OpenStreamLink(i64),
    EditStreamLink(i64, String),
    AddStreamLink(i64),
    CancelStreamLink(i64),
    RemoveStreamLink(i64, usize),
    ChooseStreamFile(i64),
    StreamFileChosen(i64, Result<Option<PathBuf>, String>),
    ClearStreamFile(i64),
    UploadStreamFile(i64),
    StreamFileUploaded(i64, Result<DriveFileAttachment, String>),
    RemoveStreamFile(i64, usize),
    OpenDrivePicker(i64),
    OpenSubmissionDrivePicker(i64, i64),
    ToggleNewSubmissionDocument(i64, i64),
    CreateSubmissionDocument(i64, i64, DriveDocumentKind),
    SubmissionDocumentCreated(i64, i64, Result<String, String>),
    SubmissionDriveFileAttached(i64, Result<(), String>),
    CloseDrivePicker,
    EditDriveSearch(i64, String),
    BrowseDrive(i64, DriveNavigation),
    DriveScrolled(i64, u64, bool),
    SearchDriveFiles(i64, bool),
    DriveFilesLoaded(i64, u64, bool, Result<DriveFilePage, String>),
    AttachExistingDriveFile(i64, String),
    CloseStreamComposer,
    SendStreamPost(i64),
    StreamPostSent(i64, Result<PublishedAnnouncement, String>),
    ConfirmDeleteStreamPost(i64, i64),
    CancelDeleteStreamPost,
    DeleteStreamPost(i64, i64),
    StreamPostDeleted(i64, i64, Result<(), String>),
    LoadAssignments,
    AssignmentLoaded(u64, desktop_auth::AssignmentRefresh),
    CourseworkLoaded(i64, Result<CourseContents, String>),
    LoadDetail(DetailRead),
    DetailLoaded(DetailRead, Result<DetailData, String>),
    ToggleRubric(i64, i64),
    ToggleRubricCriterion(i64, i64, usize),
    SetRubricCriteria(i64, i64, bool),
    OpenItem(i64, String),
    CloseItem,
    EditComment(i64, i64, String),
    OpenCommentMentions(i64, i64),
    CommentMentionsLoaded(u64, i64, i64, Result<Vec<desktop_auth::ClassMention>, String>),
    InsertCommentMention(i64, i64, i64),
    CloseCommentMentions,
    PostComment(i64, i64),
    EditPrivateComment(i64, i64, String),
    PostPrivateComment(i64, i64),
    ConfirmDeleteComment(CommentTarget),
    CancelDeleteComment,
    DeleteComment(CommentTarget),
    CommentDeleted(CommentTarget, Result<(), String>),
    PrivateCommentPosted(i64, i64, Result<(), String>),
    ConfirmSubmission(i64, i64, desktop_auth::SubmissionAction),
    CancelSubmission,
    EditAttachmentLink(String),
    AddAttachmentLink(i64, i64),
    ChooseSubmissionFile(i64, i64),
    SubmissionFileChosen(i64, i64, Result<Option<PathBuf>, String>),
    CancelSubmissionFile,
    ConfirmRemoveMaterial(i64, i64, i64),
    CancelRemoveMaterial,
    RemoveMaterial(i64, i64, i64),
    MaterialRemoved(i64, Result<(), String>),
    UploadSubmissionFile(i64, i64),
    SubmissionFileUploaded(i64, Result<(), String>),
    AttachmentLinkAdded(i64, Result<(), String>),
    EditClassCode(String),
    JoinClass,
    LoadInvitations,
    LoadClassNotifications,
    ClassNotificationsLoaded(u64, Result<BTreeMap<i64, bool>, String>),
    SetClassNotifications(i64, bool),
    ClassNotificationsSaved(i64, Result<bool, String>),
    LoadEmailNotifications,
    EmailNotificationsLoaded(u64, Result<EmailNotificationSettings, String>),
    SetEmailNotification(EmailNotificationTarget, bool),
    EmailNotificationSaved(EmailNotificationTarget, Result<EmailNotificationSettings, String>),
    InvitationsLoaded(u64, Result<Vec<Course>, String>),
    AcceptInvitation(i64),
    ConfirmDeclineInvitation(i64),
    CancelDeclineInvitation,
    DeclineInvitation(i64),
    InvitationDeclined(Result<Vec<Course>, String>),
    ConfirmUnenroll(i64),
    CancelUnenroll,
    Unenroll(i64),
    MembershipFinished(Result<MembershipChange, String>),
    ClassCreationPermission(u64, Result<bool, String>),
    EditNewClass(NewClassField, String),
    CreateClass,
    SendSubmission(i64, i64, desktop_auth::SubmissionAction),
    SubmissionSent(i64, Result<(), String>),
    BeginShortAnswerEdit(i64, i64),
    DiscardShortAnswerEdit(i64, i64),
    EditShortAnswer(i64, i64, text_editor::Action),
    SelectQuestionChoice(i64, i64, String),
    ConfirmQuestionAnswer(i64, i64),
    CancelQuestionAnswer,
    SendQuestionAnswer(i64, i64),
    QuestionAnswerSent(i64, i64, Result<(), String>),
    CommentPosted(i64, i64, Result<(), String>),
    SelectTopic(Option<String>),
    ToggleClassworkItem(i64, String),
    ToggleClassworkGroup(i64, ClassworkGroupId),
    SetClassworkGroups(i64, bool),
    SetTodoFilter(TodoFilter),
    SetWorkFilter(WorkFilter),
    FilterTodoClass(ClassFilterOption),
    ToggleTodoGroup(TodoGroup),
    CalendarShift(i32),
    CalendarToday,
    CalendarFilter(ClassFilterOption),
    OpenAttachment(String),
    OpenDriveFile(String),
    DriveFileLinkLoaded(String, Result<String, String>),
    ClassFoldersLoaded(u64, Result<desktop_auth::OwnClassDetails, String>),
    ToggleTeacherDraft(i64, String),
    OpenAssignmentEditor(i64, Option<String>),
    EditAssignmentTitle(String),
    EditAssignmentInstructions(text_editor::Action),
    EditAssignmentDueDate(String),
    EditAssignmentDueTime(String),
    EditAssignmentPoints(String),
    CloseAssignmentEditor,
    SaveAssignmentDraft,
    AssignmentDraftSaved(i64, Result<desktop_auth::TeacherDraft, String>),
    OpenAssignmentPublication(i64, String, bool),
    EditAssignmentSchedule(String),
    CloseAssignmentPublication,
    ConfirmAssignmentPublication,
    AssignmentPublicationChanged(i64, desktop_auth::AssignmentPublication, Result<(), String>),
    FilterStudentWork(StudentWorkFilter),
    SelectTeacherStudent(i64),
    EditTeacherGrade(String),
    SaveTeacherGrade,
    ConfirmReturnTeacherWork(bool),
    ReturnTeacherWork,
    TeacherGradeWritten(i64, i64, desktop_auth::TeacherGradingAction, Result<(), String>),
    SelectBulkReturn(i64, bool),
    SelectVisibleReturns(bool),
    ClearBulkReturns,
    ConfirmBulkReturn(bool),
    ReturnSelectedTeacherWork,
    BulkTeacherWorkReturned(i64, i64, usize, Result<usize, String>),
    EditTeacherFeedback(desktop_auth::TeacherFeedbackKey, String),
    SendTeacherFeedback(desktop_auth::TeacherFeedbackKey),
    TeacherFeedbackSent(desktop_auth::TeacherFeedbackKey, Result<(), String>),
    ClassTeachersLoaded(u64, Result<BTreeMap<i64, String>, String>),
    RetryClassFolders,
    AttachmentOpened,
    AttachmentFailed,
    ArtworkLoaded(u64, String, image::Handle),
    SaveOffline,
    OpenOffline,
    DeleteOffline,
    ScreenshotReady(Shot, window::Screenshot),
    Demo,
    Disconnect,
    SessionForgotten(Result<(), String>),
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

impl fmt::Debug for Message {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            #[cfg(target_os = "macos")]
            Self::Capture(_) => "Capture",
            Self::Navigate(_) => "Navigate",
            Self::ShowClassroomLink => "ShowClassroomLink",
            Self::EditClassroomLink(_) => "EditClassroomLink",
            Self::OpenClassroomLink => "OpenClassroomLink",
            Self::CloseClassroomLink => "CloseClassroomLink",
            Self::CopyClassroomLink(_) => "CopyClassroomLink",
            Self::OpenYourWork(_) => "OpenYourWork",
            Self::OpenGradebook(_) => "OpenGradebook",
            Self::OpenGradebookCell(..) => "OpenGradebookCell",
            Self::GradebookColumnPage(_) => "GradebookColumnPage",
            Self::SetCoursePostingPolicy(_) => "SetCoursePostingPolicy",
            Self::SetCourseStreamDisplay(_) => "SetCourseStreamDisplay",
            Self::EditCourseTitle(_) => "EditCourseTitle",
            Self::EditCourseSection(_) => "EditCourseSection",
            Self::EditCourseDescription(_) => "EditCourseDescription",
            Self::EditCourseRoom(_) => "EditCourseRoom",
            Self::SaveCourseStreamSettings => "SaveCourseStreamSettings",
            Self::CourseStreamSettingsSaved(..) => "CourseStreamSettingsSaved",
            Self::ConfirmStudentMute(..) => "ConfirmStudentMute",
            Self::CancelStudentMute => "CancelStudentMute",
            Self::ChangeStudentMute(..) => "ChangeStudentMute",
            Self::StudentMuteChanged(..) => "StudentMuteChanged",
            Self::ToggleTopicManager => "ToggleTopicManager",
            Self::OpenTopicEditor(..) => "OpenTopicEditor",
            Self::EditTopicName(_) => "EditTopicName",
            Self::CloseTopicEditor => "CloseTopicEditor",
            Self::SaveTopicEditor => "SaveTopicEditor",
            Self::TeacherTopicChanged(..) => "TeacherTopicChanged",
            Self::MoveTeacherTopic(..) => "MoveTeacherTopic",
            Self::OpenItemTopic(..) => "OpenItemTopic",
            Self::SelectItemTopic(_) => "SelectItemTopic",
            Self::CloseItemTopic => "CloseItemTopic",
            Self::SaveItemTopic => "SaveItemTopic",
            Self::ItemTopicSaved(..) => "ItemTopicSaved",
            Self::ReorderClasswork(..) => "ReorderClasswork",
            Self::ClassworkReordered(..) => "ClassworkReordered",
            Self::GradebookEditMode(_) => "GradebookEditMode",
            Self::EditGradebookCell(..) => "EditGradebookCell",
            Self::GradebookGradeInput(_) => "GradebookGradeInput",
            Self::CancelGradebookEdit => "CancelGradebookEdit",
            Self::SaveGradebookEdit(_) => "SaveGradebookEdit",
            Self::GradebookEditSaved(..) => "GradebookEditSaved",
            #[cfg(target_os = "macos")]
            Self::ExportGrades(..) => "ExportGrades",
            #[cfg(target_os = "macos")]
            Self::GradesExported(_) => "GradesExported",
            Self::SetTab(_) => "SetTab",
            Self::ToggleNavigation => "ToggleNavigation",
            Self::Search(_) => "Search",
            Self::SortCourses => "SortCourses",
            Self::ToggleHiddenClasses => "ToggleHiddenClasses",
            Self::ToggleCourseMenu(_) => "ToggleCourseMenu",
            Self::ChooseCourseMove(_) => "ChooseCourseMove",
            Self::MoveCourse(..) => "MoveCourse",
            Self::CourseMoved(..) => "CourseMoved",
            Self::BeginCourseDrag(_) => "BeginCourseDrag",
            Self::EnterCourseDropTarget(_) => "EnterCourseDropTarget",
            Self::ExitCourseDropTarget(_) => "ExitCourseDropTarget",
            Self::DropCourse => "DropCourse",
            Self::CancelCourseDrag => "CancelCourseDrag",
            Self::ConfirmHideCourse(_) => "ConfirmHideCourse",
            Self::ChangeCourseVisibility(..) => "ChangeCourseVisibility",
            Self::CourseVisibilityChanged(..) => "CourseVisibilityChanged",
            Self::ConfirmClassArchive(_) => "ConfirmClassArchive",
            Self::ChangeClassArchive(..) => "ChangeClassArchive",
            Self::ClassArchiveChanged(..) => "ClassArchiveChanged",
            Self::Email(_) => "Email",
            Self::SignIn => "SignIn",
            Self::SignedIn(_) => "SignedIn",
            Self::Reconnect => "Reconnect",
            Self::Reconnected(_) => "Reconnected",
            Self::Refresh => "Refresh",
            Self::Refreshed(_) => "Refreshed",
            Self::LoadCoursework(_) => "LoadCoursework",
            Self::LoadStreamAccess(_) => "LoadStreamAccess",
            Self::StreamAccessLoaded(..) => "StreamAccessLoaded",
            Self::OpenStreamComposer(_) => "OpenStreamComposer",
            Self::EditStreamDraft(..) => "EditStreamDraft",
            Self::FormatStreamDraft(..) => "FormatStreamDraft",
            Self::OpenStreamLink(_) => "OpenStreamLink",
            Self::EditStreamLink(..) => "EditStreamLink",
            Self::AddStreamLink(_) => "AddStreamLink",
            Self::CancelStreamLink(_) => "CancelStreamLink",
            Self::RemoveStreamLink(..) => "RemoveStreamLink",
            Self::ChooseStreamFile(_) => "ChooseStreamFile",
            Self::StreamFileChosen(..) => "StreamFileChosen",
            Self::ClearStreamFile(_) => "ClearStreamFile",
            Self::UploadStreamFile(_) => "UploadStreamFile",
            Self::StreamFileUploaded(..) => "StreamFileUploaded",
            Self::RemoveStreamFile(..) => "RemoveStreamFile",
            Self::OpenDrivePicker(_) => "OpenDrivePicker",
            Self::OpenSubmissionDrivePicker(..) => "OpenSubmissionDrivePicker",
            Self::ToggleNewSubmissionDocument(..) => "ToggleNewSubmissionDocument",
            Self::CreateSubmissionDocument(..) => "CreateSubmissionDocument",
            Self::SubmissionDocumentCreated(..) => "SubmissionDocumentCreated",
            Self::SubmissionDriveFileAttached(..) => "SubmissionDriveFileAttached",
            Self::CloseDrivePicker => "CloseDrivePicker",
            Self::EditDriveSearch(..) => "EditDriveSearch",
            Self::BrowseDrive(..) => "BrowseDrive",
            Self::DriveScrolled(..) => "DriveScrolled",
            Self::SearchDriveFiles(..) => "SearchDriveFiles",
            Self::DriveFilesLoaded(..) => "DriveFilesLoaded",
            Self::AttachExistingDriveFile(..) => "AttachExistingDriveFile",
            Self::CloseStreamComposer => "CloseStreamComposer",
            Self::SendStreamPost(_) => "SendStreamPost",
            Self::StreamPostSent(..) => "StreamPostSent",
            Self::ConfirmDeleteStreamPost(..) => "ConfirmDeleteStreamPost",
            Self::CancelDeleteStreamPost => "CancelDeleteStreamPost",
            Self::DeleteStreamPost(..) => "DeleteStreamPost",
            Self::StreamPostDeleted(..) => "StreamPostDeleted",
            Self::LoadAssignments => "LoadAssignments",
            Self::AssignmentLoaded(_, _) => "AssignmentLoaded",
            Self::CourseworkLoaded(_, _) => "CourseworkLoaded",
            Self::LoadDetail(_) => "LoadDetail",
            Self::DetailLoaded(_, _) => "DetailLoaded",
            Self::ToggleRubric(..) => "ToggleRubric",
            Self::ToggleRubricCriterion(..) => "ToggleRubricCriterion",
            Self::SetRubricCriteria(..) => "SetRubricCriteria",
            Self::OpenItem(_, _) => "OpenItem",
            Self::CloseItem => "CloseItem",
            Self::EditComment(..) => "EditComment",
            Self::OpenCommentMentions(..) => "OpenCommentMentions",
            Self::CommentMentionsLoaded(..) => "CommentMentionsLoaded",
            Self::InsertCommentMention(..) => "InsertCommentMention",
            Self::CloseCommentMentions => "CloseCommentMentions",
            Self::PostComment(..) => "PostComment",
            Self::EditPrivateComment(..) => "EditPrivateComment",
            Self::PostPrivateComment(..) => "PostPrivateComment",
            Self::ConfirmDeleteComment(_) => "ConfirmDeleteComment",
            Self::CancelDeleteComment => "CancelDeleteComment",
            Self::DeleteComment(_) => "DeleteComment",
            Self::CommentDeleted(..) => "CommentDeleted",
            Self::PrivateCommentPosted(..) => "PrivateCommentPosted",
            Self::ConfirmSubmission(..) => "ConfirmSubmission",
            Self::CancelSubmission => "CancelSubmission",
            Self::EditAttachmentLink(_) => "EditAttachmentLink",
            Self::AddAttachmentLink(..) => "AddAttachmentLink",
            Self::ChooseSubmissionFile(..) => "ChooseSubmissionFile",
            Self::SubmissionFileChosen(..) => "SubmissionFileChosen",
            Self::CancelSubmissionFile => "CancelSubmissionFile",
            Self::ConfirmRemoveMaterial(..) => "ConfirmRemoveMaterial",
            Self::CancelRemoveMaterial => "CancelRemoveMaterial",
            Self::RemoveMaterial(..) => "RemoveMaterial",
            Self::MaterialRemoved(..) => "MaterialRemoved",
            Self::UploadSubmissionFile(..) => "UploadSubmissionFile",
            Self::SubmissionFileUploaded(..) => "SubmissionFileUploaded",
            Self::AttachmentLinkAdded(..) => "AttachmentLinkAdded",
            Self::EditClassCode(_) => "EditClassCode",
            Self::JoinClass => "JoinClass",
            Self::LoadInvitations => "LoadInvitations",
            Self::LoadClassNotifications => "LoadClassNotifications",
            Self::ClassNotificationsLoaded(..) => "ClassNotificationsLoaded",
            Self::SetClassNotifications(..) => "SetClassNotifications",
            Self::ClassNotificationsSaved(..) => "ClassNotificationsSaved",
            Self::LoadEmailNotifications => "LoadEmailNotifications",
            Self::EmailNotificationsLoaded(..) => "EmailNotificationsLoaded",
            Self::SetEmailNotification(..) => "SetEmailNotification",
            Self::EmailNotificationSaved(..) => "EmailNotificationSaved",
            Self::InvitationsLoaded(..) => "InvitationsLoaded",
            Self::AcceptInvitation(_) => "AcceptInvitation",
            Self::ConfirmDeclineInvitation(_) => "ConfirmDeclineInvitation",
            Self::CancelDeclineInvitation => "CancelDeclineInvitation",
            Self::DeclineInvitation(_) => "DeclineInvitation",
            Self::InvitationDeclined(_) => "InvitationDeclined",
            Self::ConfirmUnenroll(_) => "ConfirmUnenroll",
            Self::CancelUnenroll => "CancelUnenroll",
            Self::Unenroll(_) => "Unenroll",
            Self::MembershipFinished(..) => "MembershipFinished",
            Self::ClassCreationPermission(..) => "ClassCreationPermission",
            Self::EditNewClass(..) => "EditNewClass",
            Self::CreateClass => "CreateClass",
            Self::SendSubmission(..) => "SendSubmission",
            Self::SubmissionSent(..) => "SubmissionSent",
            Self::BeginShortAnswerEdit(..) => "BeginShortAnswerEdit",
            Self::DiscardShortAnswerEdit(..) => "DiscardShortAnswerEdit",
            Self::EditShortAnswer(..) => "EditShortAnswer",
            Self::SelectQuestionChoice(..) => "SelectQuestionChoice",
            Self::ConfirmQuestionAnswer(..) => "ConfirmQuestionAnswer",
            Self::CancelQuestionAnswer => "CancelQuestionAnswer",
            Self::SendQuestionAnswer(..) => "SendQuestionAnswer",
            Self::QuestionAnswerSent(..) => "QuestionAnswerSent",
            Self::CommentPosted(..) => "CommentPosted",
            Self::SelectTopic(_) => "SelectTopic",
            Self::ToggleClassworkItem(_, _) => "ToggleClassworkItem",
            Self::ToggleClassworkGroup(_, _) => "ToggleClassworkGroup",
            Self::SetClassworkGroups(_, _) => "SetClassworkGroups",
            Self::SetTodoFilter(_) => "SetTodoFilter",
            Self::SetWorkFilter(_) => "SetWorkFilter",
            Self::FilterTodoClass(_) => "FilterTodoClass",
            Self::ToggleTodoGroup(_) => "ToggleTodoGroup",
            Self::CalendarShift(_) => "CalendarShift",
            Self::CalendarToday => "CalendarToday",
            Self::CalendarFilter(_) => "CalendarFilter",
            Self::OpenAttachment(_) => "OpenAttachment",
            Self::OpenDriveFile(_) => "OpenDriveFile",
            Self::DriveFileLinkLoaded(..) => "DriveFileLinkLoaded",
            Self::ClassFoldersLoaded(_, _) => "ClassFoldersLoaded",
            Self::ToggleTeacherDraft(..) => "ToggleTeacherDraft",
            Self::OpenAssignmentEditor(..) => "OpenAssignmentEditor",
            Self::EditAssignmentTitle(_) => "EditAssignmentTitle",
            Self::EditAssignmentInstructions(_) => "EditAssignmentInstructions",
            Self::EditAssignmentDueDate(_) => "EditAssignmentDueDate",
            Self::EditAssignmentDueTime(_) => "EditAssignmentDueTime",
            Self::EditAssignmentPoints(_) => "EditAssignmentPoints",
            Self::CloseAssignmentEditor => "CloseAssignmentEditor",
            Self::SaveAssignmentDraft => "SaveAssignmentDraft",
            Self::AssignmentDraftSaved(..) => "AssignmentDraftSaved",
            Self::OpenAssignmentPublication(..) => "OpenAssignmentPublication",
            Self::EditAssignmentSchedule(_) => "EditAssignmentSchedule",
            Self::CloseAssignmentPublication => "CloseAssignmentPublication",
            Self::ConfirmAssignmentPublication => "ConfirmAssignmentPublication",
            Self::AssignmentPublicationChanged(..) => "AssignmentPublicationChanged",
            Self::FilterStudentWork(_) => "FilterStudentWork",
            Self::SelectTeacherStudent(_) => "SelectTeacherStudent",
            Self::EditTeacherGrade(_) => "EditTeacherGrade",
            Self::SaveTeacherGrade => "SaveTeacherGrade",
            Self::ConfirmReturnTeacherWork(_) => "ConfirmReturnTeacherWork",
            Self::ReturnTeacherWork => "ReturnTeacherWork",
            Self::TeacherGradeWritten(..) => "TeacherGradeWritten",
            Self::SelectBulkReturn(..) => "SelectBulkReturn",
            Self::SelectVisibleReturns(_) => "SelectVisibleReturns",
            Self::ClearBulkReturns => "ClearBulkReturns",
            Self::ConfirmBulkReturn(_) => "ConfirmBulkReturn",
            Self::ReturnSelectedTeacherWork => "ReturnSelectedTeacherWork",
            Self::BulkTeacherWorkReturned(..) => "BulkTeacherWorkReturned",
            Self::EditTeacherFeedback(..) => "EditTeacherFeedback",
            Self::SendTeacherFeedback(_) => "SendTeacherFeedback",
            Self::TeacherFeedbackSent(..) => "TeacherFeedbackSent",
            Self::ClassTeachersLoaded(..) => "ClassTeachersLoaded",
            Self::RetryClassFolders => "RetryClassFolders",
            Self::AttachmentOpened => "AttachmentOpened",
            Self::AttachmentFailed => "AttachmentFailed",
            Self::ArtworkLoaded(_, _, _) => "ArtworkLoaded",
            Self::SaveOffline => "SaveOffline",
            Self::OpenOffline => "OpenOffline",
            Self::DeleteOffline => "DeleteOffline",
            Self::ScreenshotReady(_, _) => "ScreenshotReady",
            Self::Demo => "Demo",
            Self::Disconnect => "Disconnect",
            Self::SessionForgotten(_) => "SessionForgotten",
        };
        formatter.write_str(name)
    }
}

#[derive(Debug, Clone, Copy)]
enum Shot {
    Dashboard,
    Invitations,
    DeclineInvitation,
    Classwork,
    ClassworkTopic,
    YourWork,
    ToDo,
    Question,
    ShortAnswer,
    SubmittedShortAnswer,
    ShortAnswerEdit,
    MultipleChoiceDraft,
    Rubric,
    Calendar,
    ClassCalendar,
    CompactCalendar,
    CompactClasswork,
    Settings,
    StreamComposer,
    DrivePicker,
    SubmissionDrivePicker,
    StreamDelete,
    CommentDelete,
    CompactStream,
    HiddenClasses,
    MoveCourse,
    ArchiveClass,
    ClassroomLink,
    CondensedStream,
    ExpandedStream,
    HiddenStream,
    MarkDone,
    NewSubmissionDocument,
    #[cfg(target_os = "macos")]
    CameraFraming,
    #[cfg(target_os = "macos")]
    PersonalCopy,
    #[cfg(target_os = "macos")]
    TeacherDrafts,
    #[cfg(target_os = "macos")]
    AssignmentEditor,
    #[cfg(target_os = "macos")]
    AssignmentSchedule,
    #[cfg(target_os = "macos")]
    TeacherStudentWork,
    #[cfg(target_os = "macos")]
    TeacherGradebook,
    #[cfg(target_os = "macos")]
    TeacherGradebookEdit,
    #[cfg(target_os = "macos")]
    BulkReturn,
    #[cfg(target_os = "macos")]
    TeacherFeedback,
    #[cfg(target_os = "macos")]
    TeacherModerationComment,
    #[cfg(target_os = "macos")]
    TeacherModerationPost,
    #[cfg(target_os = "macos")]
    ClassStreamSettings,
    #[cfg(target_os = "macos")]
    StudentMuting,
    #[cfg(target_os = "macos")]
    TopicManagement,
    #[cfg(target_os = "macos")]
    ItemTopic,
}

#[derive(Clone, PartialEq, Eq)]
struct CourseMoveOption {
    label: String,
    placement: CoursePlacement,
}

impl fmt::Display for CourseMoveOption {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}

struct CourseDrag {
    source: i64,
    target: Option<i64>,
}

struct AssignmentEditor {
    course: i64,
    original: Option<desktop_auth::TeacherDraft>,
    title: String,
    instructions: text_editor::Content,
    settings: AssignmentFieldInputs,
}

struct AssignmentFieldInputs {
    due_date: String,
    due_time: String,
    points: String,
}

impl AssignmentFieldInputs {
    fn from_item(item: Option<&Coursework>) -> Self {
        let points = match item {
            None => "100".into(),
            Some(item) => item.maximum_points.map_or_else(String::new, |points| points.to_string()),
        };
        let due_date = item.and_then(|item| item.due_date(&Local))
            .map_or_else(String::new, |date| date.format("%Y-%m-%d").to_string());
        let due_time = item.filter(|item| item.due_has_time == Some(true))
            .and_then(|item| DateTime::from_timestamp_millis(item.due_at_millis?))
            .map_or_else(String::new, |due| due.with_timezone(&Local).format("%H:%M").to_string());
        Self { due_date, due_time, points }
    }

    fn changes(&self, initial: &Self) -> Result<desktop_auth::AssignmentFieldChanges, String> {
        let due = (self.due_date.trim() != initial.due_date || self.due_time.trim() != initial.due_time)
            .then(|| desktop_auth::parse_assignment_due(&self.due_date, &self.due_time)).transpose()?;
        let points = (self.points.trim() != initial.points)
            .then(|| desktop_auth::parse_assignment_points(&self.points)).transpose()?;
        Ok(desktop_auth::AssignmentFieldChanges { due, points })
    }
}

impl AssignmentEditor {
    fn new(course: i64, original: Option<desktop_auth::TeacherDraft>) -> Self {
        let item = original.as_ref().map(|draft| &draft.item);
        let title = item.map_or_else(String::new, |item| item.title.clone());
        let initial = item.map_or("", |item| item.description.as_str());
        let instructions = text_editor::Content::with_text(initial);
        let settings = AssignmentFieldInputs::from_item(item);
        Self { course, original, title, instructions, settings }
    }

    fn field_changes(&self) -> Result<desktop_auth::AssignmentFieldChanges, String> {
        self.settings.changes(&AssignmentFieldInputs::from_item(self.original.as_ref().map(|draft| &draft.item)))
    }

    fn instructions_editable(&self) -> bool {
        self.original.as_ref().is_none_or(|draft| draft.item.description_styles.is_empty()
            && draft.item.description_bullets.is_empty())
    }

    fn instructions_text(&self) -> String {
        if self.instructions_editable() {
            self.instructions.text()
        } else {
            self.original.as_ref().map_or_else(String::new, |draft| draft.item.description.clone())
        }
    }
}

struct AssignmentPublicationPrompt {
    draft: desktop_auth::TeacherDraft,
    schedule: Option<String>,
}

struct CommentMentions {
    course: i64,
    item: i64,
    entries: Option<Vec<desktop_auth::ClassMention>>,
    error: Option<String>,
}

#[derive(Clone, Copy)]
enum NewClassField {
    Name,
    Section,
    Subject,
    Room,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum StudentWorkFilter {
    #[default]
    All,
    Assigned,
    TurnedIn,
    Graded,
    Returned,
}

impl StudentWorkFilter {
    const ALL: [Self; 5] = [Self::All, Self::Assigned, Self::TurnedIn, Self::Graded, Self::Returned];

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Assigned => "Assigned",
            Self::TurnedIn => "Turned in",
            Self::Graded => "Graded",
            Self::Returned => "Returned",
        }
    }

    fn includes(self, row: &desktop_auth::TeacherSubmission) -> bool {
        use SubmissionStatus::*;
        match self {
            Self::All => true,
            Self::Assigned => matches!(row.status, Assigned | Missing | Reclaimed),
            Self::TurnedIn => matches!(row.status, TurnedIn | TurnedInLate),
            Self::Graded => matches!(row.status, Returned | ReturnedLate) && row.released_grade.is_some(),
            Self::Returned => matches!(row.status, Returned | ReturnedLate) && row.released_grade.is_none(),
        }
    }
}

struct TeacherWork {
    course: i64,
    item: i64,
    students: Vec<desktop_auth::TeacherSubmission>,
    filter: StudentWorkFilter,
    selected: Option<i64>,
    grade: String,
    confirming_return: bool,
    bulk_selected: BTreeSet<i64>,
    bulk_confirming: bool,
}

#[derive(Default)]
struct TeacherFeedbackThread {
    comments: Option<Vec<Comment>>,
    draft: String,
    unconfirmed: bool,
}

struct ClassStreamSettingsEditor {
    course: i64,
    original: desktop_auth::TeacherClassSettings,
    draft: desktop_auth::TeacherClassSettings,
    description: text_editor::Content,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TopicChoice {
    id: Option<String>,
    label: String,
}

impl fmt::Display for TopicChoice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}

struct ItemTopicEditor {
    target: desktop_auth::TopicItemTarget,
    title: String,
    selected: Option<String>,
}

impl ClassStreamSettingsEditor {
    fn new(course: i64, original: desktop_auth::TeacherClassSettings) -> Self {
        let description = text_editor::Content::with_text(&original.details.description);
        Self { course, draft: original.clone(), original, description }
    }

    fn changes(&self) -> desktop_auth::TeacherClassSettings {
        let mut draft = self.draft.clone();
        draft.details.description = self.description.text();
        draft
    }
}

struct GradebookEdit {
    course: i64,
    item: i64,
    expected: desktop_auth::TeacherSubmission,
    input: String,
}

impl GradebookEdit {
    fn grade(&self, maximum: Option<f64>) -> Option<f64> {
        let grade = self.input.trim().parse().ok()?;
        desktop_auth::TeacherGradingAction::SaveDraft(grade).allowed(&self.expected, maximum).then_some(grade)
    }
}

const GRADEBOOK_FIELD: &str = "gradebook-score";

impl TeacherWork {
    fn returnable(&self) -> impl Iterator<Item = &desktop_auth::TeacherSubmission> {
        self.students.iter().filter(|row| desktop_auth::TeacherGradingAction::Return.allowed(row, None))
    }

    fn selected_returns(&self) -> impl Iterator<Item = &desktop_auth::TeacherSubmission> {
        self.returnable().filter(|row| self.bulk_selected.contains(&row.student_id))
    }

    fn selected_row(&self) -> Option<&desktop_auth::TeacherSubmission> {
        self.students.iter().find(|row| self.selected == Some(row.student_id))
    }

    fn select(&mut self, selected: Option<i64>) {
        self.selected = selected;
        self.grade = self.selected_row().and_then(|row| row.draft_grade.or(row.released_grade))
            .map_or_else(String::new, |grade| grade.to_string());
        self.confirming_return = false;
    }
}

impl AssignmentPublicationPrompt {
    fn action(&self) -> Option<desktop_auth::AssignmentPublication> {
        let action = match &self.schedule {
            None => desktop_auth::AssignmentPublication::Publish,
            Some(value) => desktop_auth::AssignmentPublication::Schedule(local_schedule_millis(&Local, value)?),
        };
        action.allowed_for(self.draft.publication).then_some(action)
    }
}

fn local_schedule_millis(zone: &impl TimeZone, value: &str) -> Option<i64> {
    let date = NaiveDateTime::parse_from_str(value.trim(), "%Y-%m-%d %H:%M").ok()?;
    zone.from_local_datetime(&date).single().map(|date| date.timestamp_millis())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CommentTarget {
    course: i64,
    item: i64,
    id: i64,
    audience: CommentAudience,
}


#[derive(Default)]
struct StreamDraft {
    body: text_editor::Content,
    links: Vec<String>,
    link_input: Option<String>,
    selected_file: Option<PathBuf>,
    files: Vec<DriveFileAttachment>,
    styles: Vec<u8>,
    typing_style: Option<u8>,
    bullets: BTreeSet<usize>,
}

#[derive(Clone, Copy)]
enum StreamFormat {
    Bold = 1,
    Italic = 2,
    Underline = 4,
    BulletList = 8,
}

impl StreamDraft {
    fn position(&self, position: text_editor::Position) -> usize {
        let lines_before: usize = self.body.lines().take(position.line).map(|line| {
            line.text.len() + if line.ending == text_editor::LineEnding::None {
                1
            } else {
                line.ending.as_str().len()
            }
        }).sum();
        lines_before + self.body.line(position.line).map_or(0, |line| position.column.min(line.text.len()))
    }

    fn format(&mut self, format: StreamFormat) {
        let cursor = self.body.cursor();
        let position = self.position(cursor.position);
        if matches!(format, StreamFormat::BulletList) {
            let text = self.body.text();
            let selection = cursor.selection.map_or(position, |selection| self.position(selection));
            let start = paragraph_start(&text, position.min(selection));
            let end = position.max(selection);
            let positions: Vec<_> = paragraph_starts(&text)
                .filter(|&line| line == start || (start < line && line < end)).collect();
            let remove = positions.iter().all(|line| self.bullets.contains(line));
            for line in positions {
                if remove {
                    self.bullets.remove(&line);
                } else {
                    self.bullets.insert(line);
                }
            }
            return;
        }
        let bit = format as u8;
        if let Some(selection) = cursor.selection {
            let selection = self.position(selection);
            let text = self.body.text();
            let start = position.min(selection);
            let end = position.max(selection).min(text.len());
            if start < end && text.is_char_boundary(start) && text.is_char_boundary(end) {
                self.styles.resize(text.len(), 0);
                let range = &mut self.styles[start..end];
                let enabled = range.iter().all(|style| style & bit != 0);
                for style in range {
                    *style = if enabled { *style & !bit } else { *style | bit };
                }
                self.typing_style = None;
                return;
            }
        }
        let previous = self.styles.get(position.saturating_sub(1)).copied().unwrap_or(0);
        self.typing_style = Some(self.typing_style.unwrap_or(previous) ^ bit);
    }

    fn edit(&mut self, action: text_editor::Action) {
        if !action.is_edit() {
            if !matches!(action, text_editor::Action::Scroll { .. }) {
                self.typing_style = None;
            }
            self.body.perform(action);
            return;
        }
        let old = self.body.text();
        let cursor = self.body.cursor();
        let position = self.position(cursor.position);
        let selection = cursor.selection.map_or(position, |selection| self.position(selection));
        let start = position.min(selection);
        let end = position.max(selection);
        let paragraph = paragraph_start(&old, start);
        if matches!(action, text_editor::Action::Edit(text_editor::Edit::Enter))
            && start == end && old[paragraph..].split('\n').next().is_none_or(|text| text.trim().is_empty())
            && self.bullets.remove(&paragraph)
        {
            return;
        }
        self.body.perform(action);
        let new = self.body.text();
        if old == new {
            return;
        }
        let new_position = self.position(self.body.cursor().position);
        let limit = start.min(new_position);
        let mut prefix = 0;
        for (a, b) in old.chars().zip(new.chars()) {
            if a != b || prefix + a.len_utf8() > limit {
                break;
            }
            prefix += a.len_utf8();
        }
        let old_tail = &old[prefix..];
        let new_tail = &new[prefix..];
        let suffix_limit = old.len().saturating_sub(end).min(new.len().saturating_sub(new_position));
        let mut suffix = 0;
        for (a, b) in old_tail.chars().rev().zip(new_tail.chars().rev()) {
            if a != b || suffix + a.len_utf8() > suffix_limit {
                break;
            }
            suffix += a.len_utf8();
        }
        self.styles.resize(old.len(), 0);
        let inherited = self.typing_style.unwrap_or_else(|| {
            self.styles.get(prefix.saturating_sub(1)).copied().unwrap_or(0)
        });
        self.styles.splice(prefix..old.len() - suffix, std::iter::repeat_n(inherited, new.len() - prefix - suffix));
        self.update_bullets(&old, &new, prefix, suffix);
    }

    fn update_bullets(&mut self, old: &str, new: &str, prefix: usize, suffix: usize) {
        if self.bullets.is_empty() {
            return;
        }
        let old_end = old.len() - suffix;
        let new_end = new.len() - suffix;
        let inherited = self.bullets.contains(&paragraph_start(old, prefix));
        self.bullets = paragraph_starts(new).filter(|&start| {
            if start <= prefix {
                self.bullets.contains(&start)
            } else if start <= new_end && (start < new_end || inherited) {
                inherited
            } else {
                self.bullets.contains(&(old_end + start - new_end))
            }
        }).collect();
    }

    fn description_styles(&self) -> Vec<DescriptionStyle> {
        let mut previous = None;
        self.styles.iter().copied().enumerate().filter_map(|(start, bits)| {
            if previous.replace(bits) == Some(bits) {
                return None;
            }
            Some(DescriptionStyle { start, bold: bits & 1 != 0, italic: bits & 2 != 0, underline: bits & 4 != 0 })
        }).collect()
    }

    fn pending_link(&self) -> Option<&str> {
        let url = self.link_input.as_deref()?.trim();
        (trusted_attachment_host(url).is_some() && !self.links.iter().any(|link| link == url)).then_some(url)
    }

    fn can_send(&self, body: &str) -> bool {
        valid_stream_draft(body) && self.selected_file.is_none()
            && self.link_input.as_ref().is_none_or(|input| input.trim().is_empty())
    }
}

#[derive(Default)]
struct DrivePicker {
    course: i64,
    item: Option<i64>,
    query: String,
    submitted_query: String,
    file_page: Option<DriveFilePage>,
    error: Option<(bool, String)>,
    listed_tokens: BTreeSet<String>,
    loading: bool,
    section: DriveSection,
    folder_path: Vec<DriveFolder>,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum DriveSection {
    #[default]
    Recent,
    MyDrive,
    Shared,
}

impl DriveSection {
    fn label(self) -> &'static str {
        match self {
            Self::Recent => "Recent",
            Self::MyDrive => "My Drive",
            Self::Shared => "Shared with me",
        }
    }
}

#[derive(Clone)]
enum DriveNavigation {
    Section(DriveSection),
    OpenFolder(String),
    Back,
}

impl DrivePicker {
    fn accept_page(&mut self, mut page: DriveFilePage, append: bool) {
        if append && let Some(previous) = &mut self.file_page {
            self.listed_tokens.extend(previous.next_page.take());
            if page.next_page.as_ref().is_some_and(|token| self.listed_tokens.contains(token)) {
                page.next_page = None;
                page.incomplete = true;
            }
            let known: BTreeSet<_> = previous.files.iter().map(|file| file.id().to_owned()).collect();
            previous.files.extend(page.files.drain(..).filter(|file| !known.contains(file.id())));
            let known: BTreeSet<_> = previous.folders.iter().map(|folder| folder.id.clone()).collect();
            previous.folders.extend(page.folders.drain(..).filter(|folder| !known.contains(&folder.id)));
            previous.next_page = page.next_page;
            previous.incomplete |= page.incomplete;
        } else {
            self.file_page = Some(page);
        }
    }

    fn can_auto_page(&self) -> bool {
        !self.loading && self.error.is_none() && self.query.trim() == self.submitted_query
            && self.file_page.as_ref().is_some_and(|page| page.next_page.is_some())
    }

    fn needs_more_for_viewport(&self) -> bool {
        self.can_auto_page() && self.file_page.as_ref()
            .is_some_and(|page| page.files.len() + page.folders.len() <= 4)
    }

    fn location(&self) -> DriveLocation {
        if let Some(folder) = self.folder_path.last() {
            return DriveLocation::Folder(folder.id.clone());
        }
        match self.section {
            DriveSection::Recent => DriveLocation::Recent,
            DriveSection::MyDrive => DriveLocation::Folder("root".into()),
            DriveSection::Shared => DriveLocation::Shared,
        }
    }

    fn browse(&mut self, navigation: DriveNavigation) -> bool {
        match navigation {
            DriveNavigation::Section(section) => {
                self.section = section;
                self.folder_path.clear();
            }
            DriveNavigation::OpenFolder(id) => {
                let Some(folder) = self.file_page.as_ref()
                    .and_then(|page| page.folders.iter().find(|folder| folder.id == id))
                else { return false };
                self.folder_path.push(folder.clone());
            }
            DriveNavigation::Back => {
                if self.folder_path.pop().is_none() {
                    return false;
                }
            }
        }
        self.query.clear();
        true
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
    comment_drafts: BTreeMap<(i64, i64), String>,
    comment_mentions_generation: u64,
    comment_mentions: Option<CommentMentions>,
    comment_unconfirmed: BTreeSet<(i64, i64)>,
    posting_comment: bool,
    class_code: String,
    class_creation_generation: u64,
    class_creation_allowed: bool,
    opening_drive_file: Option<String>,
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
    private_drafts: BTreeMap<(i64, i64), String>,
    confirming_comment_delete: Option<CommentTarget>,
    deleting_comment: Option<CommentTarget>,
    private_unconfirmed: BTreeSet<(i64, i64)>,
    pending_remove_material: Option<(i64, i64, i64)>,
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
            comment_mentions_generation: 0,
            comment_mentions: None,
            comment_unconfirmed: BTreeSet::new(),
            posting_comment: false,
            class_code: String::new(),
            class_creation_generation: 0,
            class_creation_allowed: false,
            opening_drive_file: None,
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
        let subscriptions = Subscription::batch([drag, edit_keys]);
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

    fn update(&mut self, message: Message) -> Task<Message> {
        if self.screenshot_directory.is_some()
            && !matches!(message, Message::ScreenshotReady(_, _))
        {
            return Task::none();
        }
        match message {
            #[cfg(target_os = "macos")]
            Message::Capture(action) => return self.update_capture(action),
            Message::ShowClassroomLink if self.snapshot.is_some() && !self.verify_live_course_count => {
                self.classroom_link_input.get_or_insert_with(String::new);
            }
            Message::EditClassroomLink(value) if value.len() <= 4096 && self.classroom_link_input.is_some() => {
                self.classroom_link_input = Some(value);
            }
            Message::CloseClassroomLink => self.classroom_link_input = None,
            Message::OpenClassroomLink if !self.has_pending_request() && !self.verify_live_course_count => {
                if let Some(value) = &self.classroom_link_input {
                    if let Some(target) = ClassroomLink::parse(value) {
                        return self.open_classroom_link(target);
                    }
                    self.error = Some("Paste a Classroom class, assignment or question link.".into());
                }
            }
            Message::CopyClassroomLink(target) => {
                if self.snapshot.as_ref().is_some_and(|snapshot| snapshot.courses.iter().any(|course| course.id == target.course()))
                    && let Some(url) = target.web_url()
                {
                    self.error = None;
                    self.notice = Some("Classroom link copied.".into());
                    return iced::clipboard::write(url);
                }
            }
            Message::Navigate(screen) => {
                if self.screen != screen {
                    self.comment_mentions_generation = self.comment_mentions_generation.wrapping_add(1);
                    self.comment_mentions = None;
                    if screen == Screen::CreateClass {
                        if !self.can_create_class() || self.has_pending_request() {
                            return Task::none();
                        }
                        self.new_class = desktop_auth::NewClass::default();
                    }
                    self.open_teacher_draft = None;
                    self.assignment_editor = None;
                    self.assignment_publication = None;
                    self.teacher_work = None;
                    self.teacher_feedback.clear();
                    self.gradebook_page = 0;
                    self.gradebook_student = None;
                    self.class_stream_settings = None;
                    self.pending_student_mute = None;
                    self.topic_manager = false;
                    self.topic_editor = None;
                    self.item_topic_editor = None;
                    self.gradebook_edit_mode = false;
                    self.gradebook_edit = None;
                    self.discard_submission_capture();
                    self.confirming_comment_delete = None;
                    self.drive_picker = None;
                    self.opening_joined_course = None;
                    self.confirming_decline = None;
                    self.open_course_menu = None;
                    self.move_course_menu = None;
                    self.course_drag = None;
                    self.confirm_hide_course = None;
                    self.confirm_class_archive = None;
                    self.confirming_stream_delete = None;
                    self.screen = screen;
                    self.confirming_unenroll = None;
                    self.confirming_question_answer = None;
                    if screen != Screen::Join {
                        self.class_code.clear();
                    }
                    self.tab = Tab::Stream;
                    self.selected_topic = None;
                    self.classwork_toggles.clear();
                    self.collapsed_classwork_groups.clear();
                    self.item_id = None;
                    if screen == Screen::Settings && !self.loading_class_notifications && !self.loading_email_notifications
                        && self.saving_class_notifications.is_none() && self.saving_email_notification.is_none()
                    {
                        self.reset_notifications();
                    }
                    self.retry_visible_reads_on_entry();
                }
                self.queued_page_load = true;
            }
            Message::OpenYourWork(course_id) => {
                self.screen = Screen::Course(course_id);
                self.tab = Tab::YourWork;
                self.overall_grades.remove(&course_id);
                self.work_filter = WorkFilter::All;
                self.selected_topic = None;
                self.classwork_toggles.clear();
                self.collapsed_classwork_groups.clear();
                self.item_id = None;
                self.retry_visible_reads_on_entry();
                self.queued_page_load = true;
            }
            Message::OpenGradebook(course) if self.class_roles.get(&course) == Some(&CourseRole::Teacher) => {
                if self.screen != Screen::Course(course) {
                    let navigate = self.update(Message::Navigate(Screen::Course(course)));
                    return Task::batch([navigate, self.update(Message::SetTab(Tab::Grades))]);
                }
                return self.update(Message::SetTab(Tab::Grades));
            }
            Message::OpenGradebookCell(course, item, student) if self.screen == Screen::Course(course)
                && self.tab == Tab::Grades && self.class_roles.get(&course) == Some(&CourseRole::Teacher)
                && self.gradebooks.get(&course).is_some_and(|grid| grid.cells.contains_key(&(student, item))) =>
            {
                let open = self.update(Message::OpenItem(course, item.to_string()));
                self.gradebook_student = Some((course, item, student));
                return open;
            }
            Message::GradebookColumnPage(page) if self.tab == Tab::Grades && self.gradebook_edit.is_none() => self.gradebook_page = page,
            Message::SetCoursePostingPolicy(policy) if !self.saving_stream_settings && self.tab == Tab::ClassSettings => {
                if let Some(editor) = self.class_stream_settings.as_mut().filter(|editor| editor.original.stream.posting.is_some()) {
                    editor.draft.stream.posting = Some(policy);
                }
            }
            Message::SetCourseStreamDisplay(display) if !self.saving_stream_settings && self.tab == Tab::ClassSettings => {
                if let Some(editor) = self.class_stream_settings.as_mut().filter(|editor| editor.original.stream.classwork.is_some()) {
                    editor.draft.stream.classwork = Some(display);
                }
            }
            Message::EditCourseTitle(value) if !self.saving_stream_settings && self.tab == Tab::ClassSettings => {
                if let Some(editor) = &mut self.class_stream_settings {
                    editor.draft.details.title = value;
                }
            }
            Message::EditCourseSection(value) if !self.saving_stream_settings && self.tab == Tab::ClassSettings => {
                if let Some(editor) = &mut self.class_stream_settings {
                    editor.draft.details.subtitle = value;
                }
            }
            Message::EditCourseRoom(value) if !self.saving_stream_settings && self.tab == Tab::ClassSettings => {
                if let Some(editor) = &mut self.class_stream_settings {
                    editor.draft.details.room = value;
                }
            }
            Message::EditCourseDescription(action) if !self.saving_stream_settings && self.tab == Tab::ClassSettings => {
                if let Some(editor) = &mut self.class_stream_settings {
                    editor.description.perform(action);
                }
            }
            Message::SaveCourseStreamSettings if !self.has_pending_request() && self.tab == Tab::ClassSettings => {
                let (Some(editor), Some(session)) = (self.class_stream_settings.as_ref(), self.session.clone()) else {
                    return Task::none();
                };
                let (course, original, draft) = (editor.course, editor.original.clone(), editor.changes());
                if !self.can_edit_class_stream_settings(course) || draft == original || !draft.details.valid() {
                    return Task::none();
                }
                self.saving_stream_settings = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::save_teacher_class_settings(&session, course, original, draft).await
                }, move |result| Message::CourseStreamSettingsSaved(course, result));
            }
            Message::CourseStreamSettingsSaved(course, result) => {
                self.saving_stream_settings = false;
                match result {
                    Ok(settings) => {
                        if let Some(entry) = self.snapshot.as_mut().and_then(|snapshot| snapshot.courses.iter_mut().find(|entry| entry.id == course)) {
                            entry.title = settings.details.title.clone();
                            entry.subtitle = settings.details.subtitle.clone();
                            entry.description = settings.details.description.clone();
                            entry.room = settings.details.room.clone();
                            entry.stream_classwork_display = settings.stream.classwork;
                        }
                        self.stream_access.remove(&course);
                        if self.screen == Screen::Course(course) && self.tab == Tab::ClassSettings {
                            self.class_stream_settings = Some(ClassStreamSettingsEditor::new(course, settings));
                        }
                        self.notice = Some("Class settings saved.".into());
                    }
                    Err(error) => {
                        if self.screen == Screen::Course(course) && self.tab == Tab::ClassSettings {
                            self.class_stream_settings = None;
                            self.queued_page_load = true;
                        }
                        self.error = Some(error);
                    }
                }
            }
            Message::ConfirmStudentMute(course, student, current)
                if self.can_change_student_mute(course, student, current) && !self.has_pending_request() =>
            {
                self.pending_student_mute = Some((course, student, current));
            }
            Message::CancelStudentMute => self.pending_student_mute = None,
            Message::ChangeStudentMute(course, student, current)
                if self.pending_student_mute == Some((course, student, current)) && !self.has_pending_request()
                    && self.can_change_student_mute(course, student, current) =>
            {
                let Some(session) = self.session.clone() else { return Task::none(); };
                self.pending_student_mute = None;
                self.changing_student_mute = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::set_student_muted(&session, course, student, current, !current).await
                }, move |result| Message::StudentMuteChanged(course, student, result));
            }
            Message::StudentMuteChanged(course, student, result) => {
                self.changing_student_mute = false;
                match result {
                    Ok(muted) => {
                        if let Some(states) = self.student_muting.get_mut(&course) {
                            states.insert(student, muted);
                        }
                        self.notice = Some(if muted { "Student muted." } else { "Student unmuted." }.into());
                    }
                    Err(error) => {
                        self.student_muting.remove(&course);
                        self.error = Some(error);
                        self.queued_page_load |= self.screen == Screen::Course(course) && self.tab == Tab::People;
                    }
                }
            }
            Message::ToggleTopicManager if !self.has_pending_request() && self.tab == Tab::Classwork => {
                self.topic_manager = !self.topic_manager;
            }
            Message::OpenTopicEditor(course, action) if self.can_manage_topics(course) && !self.has_pending_request() => {
                self.topic_editor = Some((course, action));
                self.error = None;
            }
            Message::EditTopicName(value) if !self.saving_topic && value.len() <= 1_600 => {
                if let Some((_, desktop_auth::TeacherTopicAction::Create(name)
                    | desktop_auth::TeacherTopicAction::Rename { name, .. })) = &mut self.topic_editor
                {
                    *name = value;
                }
            }
            Message::CloseTopicEditor if !self.saving_topic => self.topic_editor = None,
            Message::SaveTopicEditor if !self.has_pending_request() => {
                let (Some((course, action)), Some(session)) = (self.topic_editor.clone(), self.session.clone())
                    else { return Task::none(); };
                if !self.can_manage_topics(course) || !topic_action_valid(&action) { return Task::none(); }
                let removed = match &action {
                    desktop_auth::TeacherTopicAction::Delete(topic) => Some(topic.id.clone()),
                    _ => None,
                };
                self.saving_topic = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::mutate_teacher_topic(&session, course, action).await },
                    move |result| Message::TeacherTopicChanged(course, removed.clone(), result));
            }
            Message::TeacherTopicChanged(course, removed, result) => {
                self.saving_topic = false;
                match result {
                    Ok(topics) => {
                        self.apply_teacher_topics(course, topics, removed.as_deref());
                        if self.topic_editor.as_ref().is_some_and(|(id, _)| *id == course) {
                            self.topic_editor = None;
                        }
                        self.notice = Some(if removed.is_some() { "Topic deleted." } else { "Topic saved." }.into());
                    }
                    Err(error) => {
                        self.error = Some(error);
                        self.teacher_topics.remove(&course);
                        self.queued_page_load |= self.screen == Screen::Course(course) && self.tab == Tab::Classwork;
                    }
                }
            }
            Message::MoveTeacherTopic(course, id, direction) if self.can_manage_topics(course)
                && !self.has_pending_request() && self.topic_editor.is_none()
                && self.teacher_topics.get(&course).is_some_and(|topics| topics.iter().any(|topic| topic.id == id)) =>
            {
                let Some(session) = self.session.clone() else { return Task::none(); };
                self.saving_topic = true;
                self.error = None;
                return Task::perform(async move { desktop_auth::reorder_topic(&session, course, &id, direction).await },
                    move |result| Message::TeacherTopicChanged(course, None, result));
            }
            Message::OpenItemTopic(course, id, draft) if self.can_manage_topics(course)
                && !self.has_pending_request() && self.teacher_topics.contains_key(&course) =>
            {
                let item = if draft {
                    self.teacher_drafts.get(&course).and_then(|rows| rows.iter().find(|row| row.item.id == id)).map(|row| &row.item)
                } else {
                    self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(course))
                        .and_then(|items| items.iter().find(|item| item.id == id))
                };
                if let (Some(item), Ok(id)) = (item, id.parse::<i64>())
                    && item.kind != CourseworkKind::Announcement
                {
                    self.item_topic_editor = Some(ItemTopicEditor {
                        target: desktop_auth::TopicItemTarget { course, item: id, kind: item.kind,
                            draft, original: item.topic_id.clone() },
                        title: item.title.clone(), selected: item.topic_id.clone(),
                    });
                    self.error = None;
                }
            }
            Message::SelectItemTopic(topic) if !self.saving_topic => {
                if let Some(editor) = &mut self.item_topic_editor
                    && topic.id.as_ref().is_none_or(|id| self.teacher_topics.get(&editor.target.course)
                        .is_some_and(|topics| topics.iter().any(|topic| &topic.id == id)))
                {
                    editor.selected = topic.id;
                }
            }
            Message::CloseItemTopic if !self.saving_topic => self.item_topic_editor = None,
            Message::SaveItemTopic if !self.has_pending_request() => {
                let (Some(editor), Some(session)) = (&self.item_topic_editor, self.session.clone()) else { return Task::none(); };
                if !self.can_manage_topics(editor.target.course) || editor.selected == editor.target.original {
                    return Task::none();
                }
                let target = editor.target.clone();
                let result_target = target.clone();
                let selected = editor.selected.clone();
                self.saving_topic = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::change_classwork_topic(&session, target, selected).await },
                    move |result| Message::ItemTopicSaved(result_target.clone(), result));
            }
            Message::ItemTopicSaved(target, result) => {
                self.saving_topic = false;
                match result {
                    Ok(topic) => {
                        self.apply_item_topic(&target, topic);
                        if self.item_topic_editor.as_ref().is_some_and(|editor| editor.target == target) {
                            self.item_topic_editor = None;
                        }
                        self.notice = Some("Classwork topic updated.".into());
                        if !target.draft {
                            return self.update(Message::LoadCoursework(target.course));
                        }
                    }
                    Err(error) => {
                        self.error = Some(error);
                        if self.item_topic_editor.as_ref().is_some_and(|editor| editor.target == target) {
                            self.item_topic_editor = None;
                        }
                        let request = if target.draft { Message::LoadDetail(DetailRead::TeacherDrafts(target.course)) }
                            else { Message::LoadCoursework(target.course) };
                        return self.update(request);
                    }
                }
            }
            Message::ReorderClasswork(course, id, direction) if self.can_manage_topics(course)
                && !self.has_pending_request() && self.item_topic_editor.is_none() =>
            {
                let (Some(snapshot), Some(session)) = (&self.snapshot, self.session.clone()) else { return Task::none(); };
                let Some(section) = snapshot.classwork_sections(course, None).into_iter().find(|section|
                    section.confirmed_order && section.items.iter().any(|item| item.id == id))
                    else { return Task::none(); };
                let ClassworkGroupId::Topic(topic) = section.id else { return Task::none(); };
                let order = section.items.into_iter().map(|item| item.id.clone()).collect::<Vec<_>>();
                self.saving_topic = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::reorder_classwork(&session, course, &id, &topic, &order, direction).await
                }, move |result| Message::ClassworkReordered(course, result));
            }
            Message::ClassworkReordered(course, result) => {
                self.saving_topic = false;
                match result {
                    Ok(contents) => {
                        self.notice = Some("Classwork order updated.".into());
                        return self.update(Message::CourseworkLoaded(course, Ok(contents)));
                    }
                    Err(error) => {
                        let reload = self.update(Message::LoadCoursework(course));
                        self.error = Some(error);
                        return reload;
                    }
                }
            }
            Message::GradebookEditMode(enabled) if !self.has_pending_request() => {
                if let Screen::Course(course) = self.screen
                    && self.can_edit_gradebook(course)
                {
                    self.gradebook_edit_mode = enabled;
                    self.gradebook_edit = None;
                }
            }
            Message::EditGradebookCell(course, item, student) if self.gradebook_edit_mode
                && self.gradebook_edit.is_none() && !self.has_pending_request() && self.can_edit_gradebook(course) =>
            {
                return self.begin_gradebook_edit(course, item, student);
            }
            Message::GradebookGradeInput(input) if !self.saving_teacher_grade && input.len() <= 20 => {
                if let Some(edit) = &mut self.gradebook_edit {
                    edit.input = input;
                }
            }
            Message::CancelGradebookEdit if !self.saving_teacher_grade => self.gradebook_edit = None,
            Message::SaveGradebookEdit(advance) if !self.has_pending_request() => {
                let Some(edit) = &self.gradebook_edit else { return Task::none(); };
                if !self.can_edit_gradebook(edit.course) { return Task::none(); }
                let (course, item) = (edit.course, edit.item);
                let expected = edit.expected.clone();
                let (student, submission) = (expected.student_id, expected.submission_id);
                if advance && grade_input_matches_saved(&edit.input, &expected) {
                    self.gradebook_edit = None;
                    if let Some(next) = self.gradebooks.get(&course)
                        .and_then(|grid| next_editable_gradebook_student(grid, item, student))
                    {
                        return self.begin_gradebook_edit(course, item, next);
                    }
                    return Task::none();
                }
                let (Some(grade), Some(session)) = (edit.grade(self.gradebook_maximum(course, item)), self.session.clone())
                    else { return Task::none(); };
                self.saving_teacher_grade = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::write_teacher_grade(&session, course, item, expected,
                        desktop_auth::TeacherGradingAction::SaveDraft(grade)).await
                }, move |result| Message::GradebookEditSaved(course, item, student, submission, grade, advance, result));
            }
            Message::GradebookEditSaved(course, item, student, submission, grade, advance, result) => {
                self.saving_teacher_grade = false;
                let grid = self.gradebooks.get_mut(&course);
                match result {
                    Ok(()) => {
                        if let Some(cell) = grid.and_then(|grid| grid.cells.get_mut(&(student, item)))
                            && cell.submission_id == submission
                        {
                            cell.draft_grade = Some(grade);
                        }
                        if self.gradebook_edit.as_ref().is_some_and(|edit| edit.course == course && edit.item == item
                            && edit.expected.student_id == student && edit.expected.submission_id == submission)
                        {
                            self.gradebook_edit = None;
                            self.notice = Some("Draft grade saved.".into());
                            if advance && let Some(next) = self.gradebooks.get(&course)
                                .and_then(|grid| next_editable_gradebook_student(grid, item, student))
                            {
                                return self.begin_gradebook_edit(course, item, next);
                            }
                        }
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            #[cfg(target_os = "macos")]
            Message::ExportGrades(course, item) if self.can_read_pages() && !self.has_pending_request()
                && self.class_roles.get(&course) == Some(&CourseRole::Teacher) =>
            {
                return self.export_grades(course, item);
            }
            #[cfg(target_os = "macos")]
            Message::GradesExported(result) => {
                self.exporting_grades = false;
                match result {
                    Ok(true) => self.notice = Some("Grade CSV saved.".into()),
                    Ok(false) => {}
                    Err(error) => self.error = Some(error),
                }
            }
            Message::SetTab(tab) => {
                if tab == Tab::Grades && !matches!(self.screen, Screen::Course(course)
                    if self.class_roles.get(&course) == Some(&CourseRole::Teacher))
                {
                    return Task::none();
                }
                if tab == Tab::ClassSettings && !matches!(self.screen, Screen::Course(course)
                    if self.can_edit_class_stream_settings(course))
                {
                    return Task::none();
                }
                if self.tab != tab {
                    self.drive_picker = None;
                    self.pending_student_mute = None;
                    self.topic_editor = None;
                    self.item_topic_editor = None;
                    self.topic_manager = false;
                    self.class_stream_settings = None;
                    self.gradebook_edit_mode = false;
                    self.gradebook_edit = None;
                    self.tab = tab;
                    self.confirming_stream_delete = None;
                    self.retry_visible_reads_on_entry();
                }
                self.queued_page_load = true;
            },
            Message::ToggleNavigation => self.sidebar = !self.sidebar,
            Message::Search(search) => self.search = search,
            Message::SortCourses => {
                self.alphabetical = !self.alphabetical;
                self.course_drag = None;
            }
            Message::ToggleHiddenClasses if self.screen == Screen::Home => {
                self.show_hidden_classes = !self.show_hidden_classes;
                self.open_course_menu = None;
                self.move_course_menu = None;
                self.confirm_hide_course = None;
            }
            Message::ToggleCourseMenu(id) if matches!(self.screen, Screen::Home | Screen::Archived) => {
                self.open_course_menu = (self.open_course_menu != Some(id)).then_some(id);
                self.move_course_menu = None;
                self.confirm_hide_course = None;
                self.confirm_class_archive = None;
            }
            Message::ChooseCourseMove(id) if self.open_course_menu == Some(id) && self.can_move_course(id) => {
                self.move_course_menu = Some(id);
                self.confirm_hide_course = None;
            }
            Message::MoveCourse(id, placement) if self.can_move_course(id) && !self.has_pending_request() => {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.open_course_menu = None;
                self.move_course_menu = None;
                self.course_drag = None;
                self.moving_course = Some(id);
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::reorder_class(&session, id, placement).await },
                    move |result| Message::CourseMoved(id, result));
            }
            Message::CourseMoved(id, result) if self.moving_course == Some(id) => {
                self.moving_course = None;
                match result {
                    Ok(courses) => {
                        if let Some(snapshot) = &mut self.snapshot {
                            snapshot.courses = courses;
                        }
                        self.alphabetical = false;
                        self.notice = Some("Class order saved.".into());
                    }
                    Err(error) => {
                        let refresh = self.update(Message::Refresh);
                        self.error = Some(error);
                        return refresh;
                    }
                }
            }
            Message::BeginCourseDrag(id) if self.can_move_course(id) && !self.alphabetical
                && self.search.trim().is_empty() && !self.has_pending_request() =>
            {
                self.open_course_menu = None;
                self.move_course_menu = None;
                self.confirm_hide_course = None;
                self.course_drag = Some(CourseDrag { source: id, target: None });
            }
            Message::EnterCourseDropTarget(id) => {
                if let Some(drag) = &mut self.course_drag {
                    drag.target = (id != drag.source).then_some(id);
                }
            }
            Message::ExitCourseDropTarget(id) => {
                if let Some(drag) = &mut self.course_drag
                    && drag.target == Some(id)
                {
                    drag.target = None;
                }
            }
            Message::CancelCourseDrag => self.course_drag = None,
            Message::DropCourse => {
                if let Some(CourseDrag { source, target: Some(target) }) = self.course_drag.take()
                    && let Some(placement) = self.drop_placement(source, target)
                {
                    return self.update(Message::MoveCourse(source, placement));
                }
            }
            Message::ConfirmHideCourse(id) if self.open_course_menu == Some(id)
                && self.can_change_course_visibility(id, true) =>
            {
                self.confirm_hide_course = Some(id);
            }
            Message::ChangeCourseVisibility(id, hidden) if self.open_course_menu == Some(id)
                && (!hidden || self.confirm_hide_course == Some(id))
                && self.can_change_course_visibility(id, hidden) && !self.has_pending_request() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.open_course_menu = None;
                self.confirm_hide_course = None;
                self.changing_course_visibility = Some((id, hidden));
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::set_class_hidden(&session, id, hidden).await },
                    move |result| Message::CourseVisibilityChanged(id, hidden, result));
            }
            Message::CourseVisibilityChanged(id, hidden, result) if self.changing_course_visibility == Some((id, hidden)) => {
                self.changing_course_visibility = None;
                match result {
                    Ok(()) => {
                        if let Some(course) = self.snapshot.as_mut().and_then(|snapshot| snapshot.courses.iter_mut().find(|course| course.id == id)) {
                            course.hidden = Some(hidden);
                        }
                        if self.todo_class == Some(id) {
                            self.todo_class = None;
                        }
                        if self.calendar_class == Some(id) {
                            self.calendar_class = None;
                        }
                        self.cancel_assignment_batch();
                        self.assignments_completed.remove(&id);
                        if !hidden {
                            self.assignments_loaded = false;
                        }
                        self.queued_page_load |= self.screen.uses_assignment_overview();
                        self.notice = Some(if hidden { "Class hidden." } else { "Class unhidden." }.into());
                    }
                    Err(error) => {
                        let refresh = self.update(Message::Refresh);
                        self.error = Some(error);
                        return refresh;
                    }
                }
            }
            Message::ConfirmClassArchive(id) if self.open_course_menu == Some(id)
                && self.can_change_class_archive(id, self.screen == Screen::Home) =>
            {
                self.confirm_class_archive = Some(id);
            }
            Message::ChangeClassArchive(id, archive) if self.open_course_menu == Some(id)
                && self.confirm_class_archive == Some(id)
                && self.can_change_class_archive(id, archive) && !self.has_pending_request() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.open_course_menu = None;
                self.confirm_class_archive = None;
                self.changing_class_archive = Some((id, archive));
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::set_class_archived(&session, id, archive).await },
                    move |result| Message::ClassArchiveChanged(id, archive, result));
            }
            Message::ClassArchiveChanged(id, archive, result) if self.changing_class_archive == Some((id, archive)) => {
                self.changing_class_archive = None;
                match result {
                    Ok(state) => {
                        let session_snapshot = self.session.as_mut().map(|session| &mut Arc::make_mut(session).snapshot);
                        for snapshot in self.snapshot.iter_mut().chain(session_snapshot) {
                            if let Some(course) = snapshot.courses.iter_mut().find(|course| course.id == id) {
                                course.state = state;
                            }
                        }
                        for filter in [&mut self.calendar_class, &mut self.todo_class] {
                            if *filter == Some(id) {
                                *filter = None;
                            }
                        }
                        self.cancel_assignment_batch();
                        self.assignments_completed.remove(&id);
                        self.assignments_loaded = false;
                        self.queued_page_load |= self.screen.uses_assignment_overview();
                        self.notice = Some(if archive { "Class archived." } else { "Class restored." }.into());
                    }
                    Err(error) => {
                        let refresh = self.update(Message::Refresh);
                        self.error = Some(error);
                        return refresh;
                    }
                }
            }
            Message::Email(email) => self.email = email,
            Message::SignIn if self.can_sign_in() => {
                self.verification_submitted = self.verify_live_course_count;
                self.busy = true;
                self.error = None;
                self.notice = None;
                let email = self.email.trim().to_owned();
                let verify_only = self.verify_live_course_count;
                return Task::perform(async move {
                    if verify_only {
                        desktop_auth::sign_in_basic(email).await
                    } else {
                        desktop_auth::sign_in(email).await
                    }
                }, |result| Message::SignedIn(result.map(Arc::new)));
            }
            Message::SignedIn(result) => {
                self.busy = false;
                match result {
                    Ok(session) => {
                        if self.verify_live_course_count {
                            println!("Desktop live QueryCourse OK; class count={}", session.snapshot.courses.len());
                            let _ = io::stdout().flush();
                        }
                        self.submissions_checked.clear();
                        self.page_errors.clear();
                        if std::env::var_os("CLASSROOM_READ_ONLY_DIAGNOSTICS").is_some() {
                            println!("Native session ready; courses={}", session.snapshot.courses.len());
                            let _ = io::stdout().flush();
                        }
                        self.install_snapshot(session.snapshot.clone());
                        self.session = Some(session);
                        self.demo = false;
                        self.screen = Screen::Home;
                        self.search.clear();
                        self.reset_assignment_reads();
                        self.reset_artwork();
                        let folders = self.fetch_class_folders();
                        let teachers = self.fetch_class_teachers();
                        let artwork = self.fetch_artwork();
                        let invitations = self.update(Message::LoadInvitations);
                        return Task::batch([
                            artwork, folders, teachers, invitations, self.fetch_class_creation_permission(),
                            self.update(Message::LoadAssignments),
                        ]);
                    }
                    Err(error) => {
                        if self.verify_live_course_count {
                            println!("Desktop live QueryCourse did not complete");
                            let _ = io::stdout().flush();
                        }
                        self.error = Some(error);
                    }
                }
            }
            Message::Reconnect if !self.has_pending_request() => {
                if let Some(session) = self.session.clone().filter(|session| session.needs_sign_in()) {
                    self.cancel_assignment_batch();
                    self.busy = true;
                    self.error = None;
                    return Task::perform(async move { session.reconnect().await }, Message::Reconnected);
                }
            }
            Message::Reconnected(result) => {
                self.busy = false;
                match result {
                    Ok(()) => {
                        self.page_errors.clear();
                        self.reset_notifications();
                        self.reset_stream_access();
                        self.reset_assignment_reads();
                        self.queued_page_load = true;
                        self.invalidate_invitation_read();
                        let background = Task::batch([
                            self.fetch_class_folders(), self.fetch_class_teachers(), self.update(Message::LoadInvitations),
                            self.fetch_class_creation_permission(),
                        ]);
                        return match self.dequeue_page_load() {
                            Some(request) => Task::batch([background, self.update(request)]),
                            None => background,
                        };
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Message::Refresh if !self.verify_live_course_count && !self.has_pending_request() => {
                if let Some(session) = self.session.clone() {
                    self.refreshing = true;
                    self.error = None;
                    return Task::perform(
                        async move { desktop_auth::refresh(&session).await },
                        Message::Refreshed,
                    );
                }
            }
            Message::Refreshed(result) => {
                self.refreshing = false;
                match result {
                    Ok(mut snapshot) => {
                        if let Some(previous) = &self.snapshot {
                            for course in &snapshot.courses {
                                if let Some(items) = previous.coursework_for(course.id) {
                                    snapshot.coursework.insert(course.id, items.to_vec());
                                }
                                if let Some(topics) = previous.topics_for(course.id) {
                                    snapshot.topics.insert(course.id, topics.to_vec());
                                }
                                if let Some(roster) = previous.roster_for(course.id) {
                                    snapshot.roster.insert(course.id, roster.to_vec());
                                }
                            }
                        }
                        self.install_snapshot(snapshot);
                        self.rubrics.clear();
                        self.overall_grades.clear();
                        self.page_errors.retain(|read, _| !matches!(read, PageRead::Detail(DetailRead::Rubric(..))));
                        self.reset_assignment_reads();
                        self.retry_visible_reads_on_entry();
                        self.queued_page_load = true;
                        let background = Task::batch([
                            self.fetch_artwork(), self.fetch_class_folders(), self.fetch_class_teachers(), self.update(Message::LoadInvitations),
                            self.fetch_class_creation_permission(),
                        ]);
                        if self.screen == Screen::Home
                            && let Some(course) = self.opening_joined_course
                            && self.snapshot.as_ref().is_some_and(|data| data.courses.iter().any(|row| row.id == course))
                        {
                            self.opening_joined_course = None;
                            return Task::batch([background, self.update(Message::Navigate(Screen::Course(course)))]);
                        }
                        return match self.dequeue_page_load() {
                            Some(request) => Task::batch([background, self.update(request)]),
                            None => background,
                        };
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Message::LoadCoursework(id) if !self.verify_live_course_count && !self.has_pending_request() => {
                if let Some(session) = self.session.clone()
                    && self.snapshot.as_ref().is_some_and(|snapshot| {
                        snapshot.courses.iter().any(|course| course.id == id)
                    })
                {
                    self.loading_coursework = Some(id);
                    self.page_errors.remove(&PageRead::Coursework(id));
                    self.error = None;
                    self.notice = None;
                    let coursework = Task::perform(
                        async move { desktop_auth::load_coursework(&session, id).await },
                        move |result| Message::CourseworkLoaded(id, result),
                    );
                    let access = self.missing_stream_access(id).then_some(Message::LoadStreamAccess(id));
                    return Task::batch([coursework, access.map_or_else(Task::none, |message| self.update(message))]);
                }
            }
            Message::LoadStreamAccess(id) if self.can_read_visible_stream_access(id) && !self.loading_stream_access.contains(&id) => {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.page_errors.remove(&PageRead::StreamAccess(id));
                self.loading_stream_access.insert(id);
                let generation = self.stream_access_generation;
                let access = Task::perform(async move { desktop_auth::load_stream_permission(&session, id).await },
                    move |result| Message::StreamAccessLoaded(generation, id, result));
                if self.screen == Screen::Item(id) {
                    self.queued_page_load = true;
                    return Task::batch([access, self.dequeue_page_load().map_or_else(Task::none, |message| self.update(message))]);
                }
                return access;
            }
            Message::StreamAccessLoaded(generation, id, result) if generation == self.stream_access_generation => {
                self.loading_stream_access.remove(&id);
                match result {
                    Ok((user, access)) => {
                        self.stream_viewer_id = Some(user);
                        self.stream_access.insert(id, access);
                    }
                    Err(error) => { self.page_errors.insert(PageRead::StreamAccess(id), error); }
                }
            }
            Message::OpenStreamComposer(id) if self.can_post_stream(id) => {
                self.stream_drafts.entry(id).or_default();
                self.open_stream_composer = Some(id);
            }
            Message::EditStreamDraft(id, action) if self.open_stream_composer == Some(id)
                && self.can_post_stream(id) && self.sending_stream_post != Some(id) =>
            {
                if let Some(draft) = self.stream_drafts.get_mut(&id) {
                    draft.edit(action);
                }
            }
            Message::FormatStreamDraft(id, format) if self.can_edit_stream_attachments(id) => {
                if let Some(draft) = self.stream_drafts.get_mut(&id) {
                    draft.format(format);
                }
            }
            Message::OpenStreamLink(id) if self.can_edit_stream_attachments(id) => {
                if let Some(draft) = self.stream_drafts.get_mut(&id) {
                    draft.link_input.get_or_insert_with(String::new);
                }
            }
            Message::EditStreamLink(id, value) if self.can_edit_stream_attachments(id) && value.len() <= 8192 => {
                if let Some(input) = self.stream_drafts.get_mut(&id).and_then(|draft| draft.link_input.as_mut()) {
                    *input = value;
                }
            }
            Message::AddStreamLink(id) if self.can_edit_stream_attachments(id) => {
                if let Some(draft) = self.stream_drafts.get_mut(&id)
                    && let Some(url) = draft.pending_link().map(str::to_owned)
                {
                    draft.links.push(url);
                    draft.link_input = None;
                }
            }
            Message::CancelStreamLink(id) if self.can_edit_stream_attachments(id) => {
                if let Some(draft) = self.stream_drafts.get_mut(&id) {
                    draft.link_input = None;
                }
            }
            Message::RemoveStreamLink(id, index) if self.can_edit_stream_attachments(id) => {
                if let Some(draft) = self.stream_drafts.get_mut(&id)
                    && index < draft.links.len()
                {
                    draft.links.remove(index);
                }
            }
            Message::ChooseStreamFile(id) if self.can_edit_stream_attachments(id) && !self.has_pending_request() => {
                self.choosing_stream_file = Some(id);
                return Task::perform(choose_attachment_file(), move |result| Message::StreamFileChosen(id, result));
            }
            Message::StreamFileChosen(id, result) if self.choosing_stream_file == Some(id) => {
                self.choosing_stream_file = None;
                match result {
                    Ok(Some(path)) => {
                        if let Some(draft) = self.stream_drafts.get_mut(&id) {
                            draft.selected_file = Some(path);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => self.error = Some(error),
                }
            }
            Message::ClearStreamFile(id) if self.can_edit_stream_attachments(id) => {
                if let Some(draft) = self.stream_drafts.get_mut(&id) {
                    draft.selected_file = None;
                }
            }
            Message::UploadStreamFile(id) if self.can_edit_stream_attachments(id) && !self.has_pending_request() => {
                if let (Some(session), Some(path)) = (
                    self.session.clone(),
                    self.stream_drafts.get(&id).and_then(|draft| draft.selected_file.clone()),
                ) {
                    self.uploading_stream_file = Some(id);
                    self.error = None;
                    self.notice = None;
                    return Task::perform(async move { desktop_auth::upload_stream_file(&session, id, path).await },
                        move |result| Message::StreamFileUploaded(id, result));
                }
            }
            Message::StreamFileUploaded(id, result) if self.uploading_stream_file == Some(id) => {
                self.uploading_stream_file = None;
                match result {
                    Ok(file) => {
                        if let Some(draft) = self.stream_drafts.get_mut(&id) {
                            draft.selected_file = None;
                            draft.files.push(file);
                        }
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Message::RemoveStreamFile(id, index) if self.can_edit_stream_attachments(id) => {
                if let Some(draft) = self.stream_drafts.get_mut(&id)
                    && index < draft.files.len()
                {
                    draft.files.remove(index);
                }
            }
            Message::OpenDrivePicker(id) if self.can_edit_stream_attachments(id) => {
                self.drive_picker = Some(DrivePicker { course: id, ..Default::default() });
                return self.update(Message::SearchDriveFiles(id, false));
            }
            Message::OpenSubmissionDrivePicker(course, item) if self.can_add_submission_drive_file(course, item)
                && !self.has_pending_request() =>
            {
                self.new_submission_document_open = false;
                self.drive_picker = Some(DrivePicker { course, item: Some(item), ..Default::default() });
                return self.update(Message::SearchDriveFiles(course, false));
            }
            Message::ToggleNewSubmissionDocument(course, item)
                if self.can_add_submission_drive_file(course, item) && !self.has_pending_request() =>
            {
                self.new_submission_document_open = !self.new_submission_document_open;
                self.drive_picker = None;
            }
            Message::CreateSubmissionDocument(course, item, kind)
                if self.new_submission_document_open && !self.has_pending_request()
                    && self.can_add_submission_drive_file(course, item) =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                let key = item.to_string();
                let Some(work) = self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(course))
                    .and_then(|items| items.iter().find(|entry| entry.id == key)) else { return Task::none() };
                let title = work.title.clone();
                self.new_submission_document_open = false;
                self.sending_submission = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::create_submission_document(&session, course, item, title, kind).await
                }, move |result| Message::SubmissionDocumentCreated(course, item, result));
            }
            Message::SubmissionDocumentCreated(course, item, result) => {
                self.sending_submission = false;
                let open = match result {
                    Ok(url) => {
                        self.notice = Some("New file attached to your work.".into());
                        if self.screen == Screen::Item(course)
                            && self.item_id.as_deref().and_then(|id| id.parse::<i64>().ok()) == Some(item)
                        {
                            self.update(Message::OpenAttachment(url))
                        } else {
                            Task::none()
                        }
                    }
                    Err(error) => {
                        self.error = Some(error);
                        Task::none()
                    }
                };
                return Task::batch([open, self.update(Message::LoadDetail(DetailRead::Submissions(course)))]);
            }
            Message::CloseDrivePicker => self.drive_picker = None,
            Message::EditDriveSearch(id, query) if query.len() <= 1024 => {
                if let Some(picker) = self.drive_picker.as_mut().filter(|picker| picker.course == id) {
                    picker.query = query;
                }
            }
            Message::BrowseDrive(id, navigation) if self.can_use_drive_picker(id) => {
                if let Some(picker) = self.drive_picker.as_mut().filter(|picker| picker.course == id)
                    && picker.browse(navigation)
                {
                    return self.update(Message::SearchDriveFiles(id, false));
                }
            }
            Message::DriveScrolled(id, generation, true) if generation == self.drive_picker_generation
                && self.drive_picker.as_ref().is_some_and(|picker| picker.course == id && picker.can_auto_page()) =>
            {
                return self.update(Message::SearchDriveFiles(id, true));
            }
            Message::SearchDriveFiles(id, append) if self.can_use_drive_picker(id) => {
                let Some(session) = self.session.clone() else { return Task::none() };
                if let Some(picker) = self.drive_picker.as_mut().filter(|picker| picker.course == id) {
                    let query = picker.query.trim().to_owned();
                    let page_token = if append {
                        if picker.loading || query != picker.submitted_query {
                            return Task::none();
                        }
                        let Some(token) = picker.file_page.as_ref().and_then(|page| page.next_page.clone()) else {
                            return Task::none();
                        };
                        Some(token)
                    } else {
                        picker.file_page = None;
                        picker.listed_tokens.clear();
                        None
                    };
                    self.drive_picker_generation = self.drive_picker_generation.wrapping_add(1);
                    let generation = self.drive_picker_generation;
                    picker.loading = true;
                    picker.error = None;
                    picker.submitted_query = query.clone();
                    let location = picker.location();
                    return Task::perform(async move { desktop_auth::list_drive_files(&session, location, query, page_token).await },
                        move |result| Message::DriveFilesLoaded(id, generation, append, result));
                }
            }
            Message::DriveFilesLoaded(id, generation, append, result) if generation == self.drive_picker_generation => {
                if let Some(picker) = self.drive_picker.as_mut().filter(|picker| picker.course == id) {
                    picker.loading = false;
                    match result {
                        Ok(page) => picker.accept_page(page, append),
                        Err(error) => picker.error = Some((append, error)),
                    }
                    if picker.needs_more_for_viewport() {
                        return self.update(Message::SearchDriveFiles(id, true));
                    }
                }
            }
            Message::AttachExistingDriveFile(id, file_id) if self.can_use_drive_picker(id) && !self.has_pending_request() => {
                if let Some(picker) = self.drive_picker.as_ref().filter(|picker| picker.course == id)
                    && let Some(file) = picker.file_page.as_ref()
                    .and_then(|page| page.files.iter().find(|file| file.id() == file_id))
                {
                    if let Some(item) = picker.item {
                        let Some(session) = self.session.clone() else { return Task::none() };
                        let file = file.clone();
                        self.sending_submission = true;
                        self.error = None;
                        self.notice = None;
                        return Task::perform(async move { desktop_auth::attach_submission_drive_file(&session, id, item, file).await },
                            move |result| Message::SubmissionDriveFileAttached(id, result));
                    }
                    if let Some(draft) = self.stream_drafts.get_mut(&id)
                        && !draft.files.iter().any(|existing| existing.id() == file_id)
                    {
                        draft.files.push(file.clone());
                    }
                }
            }
            Message::SubmissionDriveFileAttached(course, result) => {
                self.sending_submission = false;
                match result {
                    Ok(()) => {
                        self.notice = Some("Drive file attached to your work.".into());
                        return self.update(Message::LoadDetail(DetailRead::Submissions(course)));
                    }
                    Err(error) => {
                        self.error = Some(error);
                        let refresh = self.update(Message::LoadDetail(DetailRead::Submissions(course)));
                        return refresh;
                    }
                }
            }
            Message::CloseStreamComposer => {
                self.open_stream_composer = None;
                self.drive_picker = None;
            }
            Message::SendStreamPost(id) if self.open_stream_composer == Some(id)
                && self.can_post_stream(id) && !self.has_pending_request()
                && !self.page_errors.contains_key(&PageRead::Coursework(id)) =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                if let Some((body, styles, bullets, links, files)) = self.stream_drafts.get(&id).and_then(|draft| {
                    let body = draft.body.text();
                    draft.can_send(&body).then(|| (body, draft.description_styles(), draft.bullets.iter().copied().collect(),
                        draft.links.clone(), draft.files.clone()))
                }) {
                    self.sending_stream_post = Some(id);
                    self.error = None;
                    self.notice = None;
                    return Task::perform(async move { desktop_auth::post_stream_announcement(&session, id, body, styles, bullets, links, files).await },
                        move |result| Message::StreamPostSent(id, result));
                }
            }
            Message::StreamPostSent(id, result) if self.sending_stream_post == Some(id) => {
                self.sending_stream_post = None;
                match result {
                    Ok(PublishedAnnouncement { item, unconfirmed_attachments, formatting_confirmed }) => {
                        self.stream_drafts.remove(&id);
                        self.drive_picker = None;
                        if self.open_stream_composer == Some(id) {
                            self.open_stream_composer = None;
                        }
                        if let Some(snapshot) = &mut self.snapshot {
                            let items = snapshot.coursework.entry(id).or_default();
                            items.retain(|existing| existing.id != item.id);
                            items.insert(0, item);
                        }
                        if unconfirmed_attachments == 0 {
                            if formatting_confirmed {
                                self.notice = Some("Posted to Stream.".into());
                            } else {
                                self.error = Some("Posted to Stream. Classroom didn’t confirm the formatting.".into());
                            }
                        } else {
                            let noun = if unconfirmed_attachments == 1 { "attachment" } else { "attachments" };
                            let formatting = if formatting_confirmed { "" } else { " or the formatting" };
                            self.error = Some(format!("Posted to Stream. Classroom didn’t confirm {unconfirmed_attachments} {noun}{formatting}."));
                        }
                    }
                    Err(error) => {
                        self.stream_access.remove(&id);
                        if self.on_live_stream(id) {
                            let task = self.update(Message::LoadCoursework(id));
                            self.error = Some(error);
                            return task;
                        }
                        self.error = Some(error);
                    }
                }
            }
            Message::ConfirmDeleteStreamPost(course, id) if self.can_delete_stream_post(course, id) => {
                self.confirming_stream_delete = Some((course, id));
            }
            Message::CancelDeleteStreamPost => self.confirming_stream_delete = None,
            Message::DeleteStreamPost(course, id) if self.confirming_stream_delete == Some((course, id))
                && self.can_delete_stream_post(course, id) && !self.has_pending_request() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                let Some(item) = self.displayed_announcement(course, id) else { return Task::none(); };
                let moderation_author = if self.owns_announcement(item) { None } else { item.author_id };
                self.confirming_stream_delete = None;
                self.deleting_stream_post = Some((course, id));
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    if let Some(author) = moderation_author {
                        desktop_auth::moderate_stream_post(&session, course, id, author).await
                    } else {
                        desktop_auth::delete_own_stream_post(&session, course, id).await
                    }
                },
                    move |result| Message::StreamPostDeleted(course, id, result));
            }
            Message::StreamPostDeleted(course, id, result) if self.deleting_stream_post == Some((course, id)) => {
                self.deleting_stream_post = None;
                match result {
                    Ok(()) => {
                        if let Some(items) = self.snapshot.as_mut().and_then(|snapshot| snapshot.coursework.get_mut(&course)) {
                            let item_id = id.to_string();
                            items.retain(|item| item.id != item_id || item.kind != CourseworkKind::Announcement);
                        }
                        self.notice = Some("Post deleted.".into());
                    }
                    Err(error) => {
                        if self.on_live_stream(course) {
                            let refresh = self.update(Message::LoadCoursework(course));
                            self.error = Some(error);
                            return refresh;
                        }
                        self.error = Some(error);
                    }
                }
            }
            Message::LoadAssignments if self.can_read_pages() && !self.has_pending_request()
                && self.assignment_batch.is_none() && self.screen.uses_assignment_overview() =>
            {
                self.page_errors.remove(&PageRead::Assignments);
                let remaining = self.assignment_courses();
                self.assignments_loaded = remaining.is_empty();
                if !remaining.is_empty()
                    && let Some(session) = self.session.clone()
                {
                    self.assignment_generation = self.assignment_generation.wrapping_add(1);
                    let generation = self.assignment_generation;
                    let updates = desktop_auth::assignment_updates(session, remaining.iter().copied().collect());
                    let (task, handle) = Task::run(updates, move |result| {
                        Message::AssignmentLoaded(generation, result)
                    }).abortable();
                    self.assignment_batch = Some(AssignmentBatch {
                        handle,
                        remaining,
                        failed_classwork: 0,
                        failed_status: 0,
                    });
                    return task;
                }
            }
            Message::AssignmentLoaded(generation, result) if generation == self.assignment_generation => {
                self.apply_assignment_update(result);
            }
            Message::CourseworkLoaded(id, result) => {
                self.loading_coursework = None;
                if std::env::var_os("CLASSROOM_READ_ONLY_DIAGNOSTICS").is_some() {
                    println!("Classwork read completed; success={}", result.is_ok());
                }
                if self.screen == Screen::Item(id)
                    || (self.screen == Screen::Course(id) && self.tab != Tab::People)
                {
                    self.queued_page_load = true;
                }
                match result {
                    Ok(contents) => {
                        self.gradebooks.remove(&id);
                        if let Some(snapshot) = &mut self.snapshot {
                            if self.screen == Screen::Course(id)
                                && let Some(topic) = self.selected_topic.as_ref()
                                && contents.topics.as_ref().is_some_and(|topics| !topics.iter().any(|entry| &entry.id == topic))
                                && !contents.items.iter().any(|item| item.topic_id.as_ref() == Some(topic))
                            {
                                self.selected_topic = None;
                                self.classwork_toggles.clear();
                            }
                            self.submissions_checked.remove(&id);
                            if self.assignments_completed.remove(&id) {
                                self.assignments_loaded = false;
                            }
                            snapshot.coursework.insert(id, contents.items);
                            if let Some(topics) = contents.topics {
                                snapshot.topics.insert(id, topics);
                            }
                        }
                    }
                    Err(error) => {
                        self.page_errors.insert(PageRead::Coursework(id), error);
                    }
                }
            }
            Message::LoadDetail(request) if !self.verify_live_course_count && !self.has_pending_request() => {
                if let Some(session) = self.session.clone() {
                    let requested_ids = match request {
                        DetailRead::TeacherGradebook(course) => self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(course))
                            .unwrap_or_default().iter().filter(|item| has_submission(item))
                            .filter_map(|item| item.id.parse::<i64>().ok()).collect(),
                        DetailRead::StudentMuting(course) => self.snapshot.as_ref().and_then(|snapshot| snapshot.roster_for(course))
                            .unwrap_or_default().iter().filter(|member| member.role == CourseRole::Student)
                            .filter_map(|member| member.id.parse::<i64>().ok()).collect(),
                        _ => Vec::new(),
                    };
                    self.loading_detail = Some(request);
                    self.page_errors.remove(&PageRead::Detail(request));
                    return Task::perform(async move {
                        match request {
                            DetailRead::Roster(course) => desktop_auth::load_roster(&session, course).await
                                .map(DetailData::Roster),
                            DetailRead::Submissions(course) => desktop_auth::load_submissions(&session, course).await
                                .map(DetailData::Submissions),
                            DetailRead::Comments(course, item) => desktop_auth::load_comments(&session, course, item).await
                                .map(DetailData::Comments),
                            DetailRead::PrivateFeedback(course, item) =>
                                desktop_auth::load_private_feedback(&session, course, item).await.map(DetailData::PrivateFeedback),
                            DetailRead::Rubric(course, item) => desktop_auth::load_rubric(&session, course, item).await
                                .map(DetailData::Rubric),
                            DetailRead::OverallGrade(course) => desktop_auth::load_course_grade(&session, course).await
                                .map(DetailData::OverallGrade),
                            DetailRead::TeacherDrafts(course) => desktop_auth::load_teacher_drafts(&session, course).await
                                .map(DetailData::TeacherDrafts),
                            DetailRead::TeacherSubmissions(course, item) => desktop_auth::load_teacher_submissions(&session, course, item).await
                                .map(DetailData::TeacherSubmissions),
                            DetailRead::TeacherFeedback(key) => desktop_auth::load_teacher_private_feedback(&session, key).await
                                .map(DetailData::TeacherFeedback),
                            DetailRead::TeacherGradebook(course) => desktop_auth::load_teacher_gradebook(&session, course, requested_ids).await
                                .map(DetailData::TeacherGradebook),
                            DetailRead::ClassStreamSettings(course) => desktop_auth::load_teacher_class_settings(&session, course).await
                                .map(DetailData::ClassStreamSettings),
                            DetailRead::StudentMuting(course) => desktop_auth::load_student_muting(&session, course, requested_ids).await
                                .map(DetailData::StudentMuting),
                            DetailRead::TeacherTopics(course) => desktop_auth::load_teacher_topics(&session, course).await
                                .map(DetailData::TeacherTopics),
                        }
                    }, move |result| Message::DetailLoaded(request, result));
                }
            }
            Message::DetailLoaded(request, result) => {
                self.loading_detail = None;
                if std::env::var_os("CLASSROOM_READ_ONLY_DIAGNOSTICS").is_some() {
                    let section = match request {
                        DetailRead::Roster(_) => "People",
                        DetailRead::Submissions(_) => "Your work",
                        DetailRead::Comments(..) => "Class comments",
                        DetailRead::PrivateFeedback(..) => "Private feedback",
                        DetailRead::Rubric(..) => "Rubric",
                        DetailRead::OverallGrade(_) => "Overall grade",
                        DetailRead::TeacherDrafts(_) => "Teacher drafts",
                        DetailRead::TeacherSubmissions(..) => "Teacher submissions",
                        DetailRead::TeacherFeedback(_) => "Teacher private feedback",
                        DetailRead::TeacherGradebook(_) => "Teacher gradebook",
                        DetailRead::ClassStreamSettings(_) => "Class Stream settings",
                        DetailRead::StudentMuting(_) => "Student mute states",
                        DetailRead::TeacherTopics(_) => "Teacher Classwork topics",
                    };
                    println!("{section} read completed; success={}", result.is_ok());
                }
                if matches!(self.screen, Screen::Item(_)) || self.tab == Tab::YourWork {
                    self.queued_page_load = true;
                }
                match (request, result) {
                    (_, Err(error)) => {
                        self.page_errors.insert(PageRead::Detail(request), error);
                    }
                    (DetailRead::Roster(course), Ok(DetailData::Roster(members))) => {
                        if let Some(snapshot) = &mut self.snapshot {
                            snapshot.roster.insert(course, members);
                        }
                        self.student_muting.remove(&course);
                        self.queued_page_load |= self.screen == Screen::Course(course) && self.tab == Tab::People;
                    }
                    (DetailRead::Submissions(course), Ok(DetailData::Submissions(submissions))) => {
                        if let Some(snapshot) = &mut self.snapshot {
                            snapshot.set_submissions(course, submissions);
                            self.submissions_checked.insert(course);
                        }
                    }
                    (DetailRead::PrivateFeedback(course, item), Ok(DetailData::PrivateFeedback(comments))) => {
                        self.private_feedback.insert((course, item), comments);
                        self.private_unconfirmed.remove(&(course, item));
                    }
                    (DetailRead::OverallGrade(course), Ok(DetailData::OverallGrade(grade))) => {
                        self.overall_grades.insert(course, grade);
                    }
                    (DetailRead::TeacherDrafts(course), Ok(DetailData::TeacherDrafts(drafts)))
                        if self.class_roles.get(&course) == Some(&CourseRole::Teacher) =>
                    {
                        self.teacher_drafts.insert(course, drafts);
                        self.queued_page_load = true;
                    }
                    (DetailRead::TeacherSubmissions(course, item), Ok(DetailData::TeacherSubmissions(students)))
                        if self.can_read_teacher_work(course, item) =>
                    {
                        let previous = self.teacher_work.take().filter(|work| work.course == course && work.item == item);
                        let requested = self.gradebook_student.take()
                            .and_then(|(c, i, student)| (c == course && i == item).then_some(student));
                        let (filter, selected) = previous.as_ref().map_or((StudentWorkFilter::All, requested), |work| (work.filter, work.selected.or(requested)));
                        let mut work = TeacherWork { course, item, students, filter, selected: None,
                            grade: String::new(), confirming_return: false, bulk_selected: BTreeSet::new(), bulk_confirming: false };
                        work.select(selected.filter(|id| work.students.iter().any(|row| row.student_id == *id && filter.includes(row))));
                        self.teacher_work = Some(work);
                    }
                    (DetailRead::TeacherFeedback(key), Ok(DetailData::TeacherFeedback(comments)))
                        if self.selected_teacher_feedback_key() == Some(key) =>
                    {
                        let thread = self.teacher_feedback.entry(key).or_default();
                        thread.comments = Some(comments);
                        thread.unconfirmed = false;
                    }
                    (DetailRead::TeacherGradebook(course), Ok(DetailData::TeacherGradebook(gradebook)))
                        if self.class_roles.get(&course) == Some(&CourseRole::Teacher) =>
                    {
                        self.gradebooks.insert(course, gradebook);
                    }
                    (DetailRead::ClassStreamSettings(course), Ok(DetailData::ClassStreamSettings(settings)))
                        if self.tab == Tab::ClassSettings && self.can_edit_class_stream_settings(course) =>
                    {
                        self.class_stream_settings = Some(ClassStreamSettingsEditor::new(course, settings));
                    }
                    (DetailRead::StudentMuting(course), Ok(DetailData::StudentMuting(states)))
                        if self.class_roles.get(&course) == Some(&CourseRole::Teacher) =>
                    {
                        self.student_muting.insert(course, states);
                    }
                    (DetailRead::TeacherTopics(course), Ok(DetailData::TeacherTopics(topics)))
                        if self.class_roles.get(&course) == Some(&CourseRole::Teacher) =>
                    {
                        self.apply_teacher_topics(course, topics, None);
                        self.queued_page_load |= self.screen == Screen::Course(course) && self.tab == Tab::Classwork;
                    }
                    (DetailRead::Rubric(course, item), Ok(DetailData::Rubric(rubric))) => {
                        self.rubrics.insert((course, item), rubric);
                    }
                    (DetailRead::Comments(course, item), Ok(DetailData::Comments(comments))) => {
                        self.comment_unconfirmed.remove(&(course, item));
                        let item_id = item.to_string();
                        if let Some(item) = self.snapshot.as_mut()
                            .and_then(|snapshot| snapshot.coursework.get_mut(&course))
                            .and_then(|items| items.iter_mut().find(|item| item.id == item_id))
                        {
                            item.comments = Some(comments);
                        }
                    }
                    _ => {}
                }
            }
            Message::EditPrivateComment(course, item, draft)
                if self.can_compose_comment(course, item) && draft.len() <= 10_000 =>
            {
                self.private_drafts.insert((course, item), draft);
            }
            Message::ConfirmDeleteComment(target) if self.may_delete_comment(target) => {
                self.confirming_comment_delete = Some(target);
            }
            Message::CancelDeleteComment => self.confirming_comment_delete = None,
            Message::DeleteComment(target) if self.confirming_comment_delete == Some(target)
                && self.may_delete_comment(target) && !self.has_pending_request() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                let Some(comment) = self.displayed_comment(target) else { return Task::none(); };
                let moderation_author = if self.owns_comment(comment) { None } else { comment.author_id };
                self.confirming_comment_delete = None;
                self.deleting_comment = Some(target);
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    if let Some(author) = moderation_author {
                        desktop_auth::moderate_class_comment(&session, target.course, target.item, target.id, author).await
                    } else {
                        desktop_auth::delete_own_comment(&session, target.course, target.item, target.id, target.audience).await
                    }
                }, move |result| Message::CommentDeleted(target, result));
            }
            Message::CommentDeleted(target, result) if self.deleting_comment == Some(target) => {
                self.deleting_comment = None;
                let id = target.id.to_string();
                match result {
                    Ok(()) => {
                        let comments = if target.audience == CommentAudience::Private {
                            self.private_feedback.get_mut(&(target.course, target.item))
                        } else {
                            self.snapshot.as_mut().and_then(|snapshot| snapshot.coursework.get_mut(&target.course))
                                .and_then(|items| items.iter_mut().find(|item| item.id.parse::<i64>().ok() == Some(target.item)))
                                .and_then(|item| item.comments.as_mut())
                        };
                        if let Some(comments) = comments {
                            comments.retain(|comment| comment.id != id);
                        }
                        self.notice = Some("Comment deleted.".into());
                    }
                    Err(error) => {
                        self.error = Some(error);
                        if self.screen == Screen::Item(target.course)
                            && self.item_id.as_ref().and_then(|id| id.parse().ok()) == Some(target.item)
                        {
                            let read = match target.audience {
                                CommentAudience::Class => DetailRead::Comments(target.course, target.item),
                                CommentAudience::Private => DetailRead::PrivateFeedback(target.course, target.item),
                            };
                            return self.update(Message::LoadDetail(read));
                        }
                    }
                }
            }
            Message::PostPrivateComment(course, item)
                if !self.has_pending_request() && self.can_compose_comment(course, item)
                    && !self.private_unconfirmed.contains(&(course, item)) =>
            {
                if let (Some(session), Some(draft)) = (self.session.clone(),
                    self.private_drafts.get(&(course, item)).filter(|draft| !draft.trim().is_empty()).cloned())
                {
                    self.posting_comment = true;
                    self.error = None;
                    self.notice = None;
                    return Task::perform(async move {
                        desktop_auth::post_private_feedback(&session, course, item, draft).await
                    }, move |result| Message::PrivateCommentPosted(course, item, result));
                }
            }
            Message::PrivateCommentPosted(course, item, result) => {
                self.posting_comment = false;
                match result {
                    Ok(()) => {
                        self.private_drafts.remove(&(course, item));
                        self.notice = Some("Private comment sent.".into());
                        return self.update(Message::LoadDetail(DetailRead::PrivateFeedback(course, item)));
                    }
                    Err(error) => {
                        self.private_unconfirmed.insert((course, item));
                        self.error = Some(error);
                        self.queued_page_load = true;
                    }
                }
            }
            Message::EditComment(course, item, draft) if self.can_compose_class_comment(course, item) => {
                if draft.len() <= 10_000 {
                    self.comment_drafts.insert((course, item), draft);
                }
            }
            Message::OpenCommentMentions(course, item) if self.can_compose_class_comment(course, item)
                && !self.has_pending_request() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.comment_mentions_generation = self.comment_mentions_generation.wrapping_add(1);
                let generation = self.comment_mentions_generation;
                self.comment_mentions = Some(CommentMentions { course, item, entries: None, error: None });
                return Task::perform(async move { desktop_auth::load_class_mentions(&session, course).await },
                    move |result| Message::CommentMentionsLoaded(generation, course, item, result));
            }
            Message::CommentMentionsLoaded(generation, course, item, result)
                if generation == self.comment_mentions_generation && self.can_compose_class_comment(course, item) =>
            {
                if let Some(picker) = self.comment_mentions.as_mut()
                    .filter(|picker| picker.course == course && picker.item == item)
                {
                    match result {
                        Ok(entries) => picker.entries = Some(entries),
                        Err(error) => picker.error = Some(error),
                    }
                }
            }
            Message::InsertCommentMention(course, item, member) if self.can_compose_class_comment(course, item)
                && !self.has_pending_request() =>
            {
                let Some(email) = self.comment_mentions.as_ref()
                    .filter(|picker| picker.course == course && picker.item == item)
                    .and_then(|picker| picker.entries.as_ref())
                    .and_then(|entries| entries.iter().find(|entry| entry.id == member))
                    .map(|entry| entry.email.clone())
                    else { return Task::none(); };
                let draft = self.comment_drafts.entry((course, item)).or_default();
                append_comment_mention(draft, &email);
                self.comment_mentions = None;
            }
            Message::CloseCommentMentions => {
                self.comment_mentions_generation = self.comment_mentions_generation.wrapping_add(1);
                self.comment_mentions = None;
            }
            Message::PostComment(course, item)
                if !self.has_pending_request() && self.can_compose_class_comment(course, item)
                    && !self.comment_unconfirmed.contains(&(course, item)) =>
            {
                if let (Some(session), Some(draft)) = (self.session.clone(),
                    self.comment_drafts.get(&(course, item)).filter(|draft| !draft.trim().is_empty()).cloned())
                {
                    self.posting_comment = true;
                    self.error = None;
                    self.notice = None;
                    return Task::perform(async move {
                        desktop_auth::post_class_comment(&session, course, item, draft).await
                    }, move |result| Message::CommentPosted(course, item, result));
                }
            }
            Message::CommentPosted(course, item, result) => {
                self.posting_comment = false;
                match result {
                    Ok(()) => {
                        self.comment_drafts.remove(&(course, item));
                        self.notice = Some("Class comment posted.".into());
                        return self.update(Message::LoadDetail(DetailRead::Comments(course, item)));
                    }
                    Err(error) => {
                        self.stream_access.remove(&course);
                        self.page_errors.remove(&PageRead::StreamAccess(course));
                        self.comment_unconfirmed.insert((course, item));
                        self.error = Some(error);
                        self.queued_page_load = true;
                    }
                }
            }
            Message::LoadClassNotifications if self.can_read_pages() && !self.loading_class_notifications
                && !self.has_pending_request() && self.screen == Screen::Settings =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                let courses = self.class_notification_courses().map(|course| course.id).collect();
                self.page_errors.remove(&PageRead::ClassNotifications);
                self.loading_class_notifications = true;
                let generation = self.notifications_generation;
                let classes = Task::perform(async move { desktop_auth::load_class_notifications(&session, courses).await },
                    move |result| Message::ClassNotificationsLoaded(generation, result));
                self.queued_page_load = true;
                return Task::batch([classes, self.dequeue_page_load().map_or_else(Task::none, |request| self.update(request))]);
            }
            Message::ClassNotificationsLoaded(generation, result) if generation == self.notifications_generation => {
                self.loading_class_notifications = false;
                match result {
                    Ok(settings) => {
                        if std::env::var_os("CLASSROOM_READ_ONLY_DIAGNOSTICS").is_some() {
                            println!("Class notification settings ready; count={}", settings.len());
                        }
                        self.class_notifications = Some(settings);
                    }
                    Err(error) => {
                        self.page_errors.insert(PageRead::ClassNotifications, error);
                    }
                }
            }
            Message::SetClassNotifications(course, enabled) if self.can_set_class_notifications(course, enabled) => {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.saving_class_notifications = Some(course);
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::set_class_notifications(&session, course, !enabled).await },
                    move |result| Message::ClassNotificationsSaved(course, result));
            }
            Message::ClassNotificationsSaved(course, result) if self.saving_class_notifications == Some(course) => {
                self.saving_class_notifications = None;
                match result {
                    Ok(muted) => {
                        if let Some(settings) = &mut self.class_notifications {
                            settings.insert(course, muted);
                        }
                    }
                    Err(error) => {
                        self.class_notifications = None;
                        self.error = Some(error);
                        self.queued_page_load |= self.screen == Screen::Settings;
                    }
                }
            }
            Message::LoadEmailNotifications if self.can_read_pages() && !self.loading_email_notifications
                && !self.has_pending_request() && self.screen == Screen::Settings =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.page_errors.remove(&PageRead::EmailNotifications);
                self.loading_email_notifications = true;
                let generation = self.notifications_generation;
                return Task::perform(async move { desktop_auth::load_email_notifications(&session).await },
                    move |result| Message::EmailNotificationsLoaded(generation, result));
            }
            Message::EmailNotificationsLoaded(generation, result) if generation == self.notifications_generation => {
                self.loading_email_notifications = false;
                match result {
                    Ok(settings) => self.email_notifications = Some(settings),
                    Err(error) => { self.page_errors.insert(PageRead::EmailNotifications, error); }
                }
            }
            Message::SetEmailNotification(target, enabled) if self.can_set_email_notification(target, enabled) => {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.saving_email_notification = Some(target);
                self.error = None;
                return Task::perform(async move { desktop_auth::set_email_notification(&session, target, enabled).await },
                    move |result| Message::EmailNotificationSaved(target, result));
            }
            Message::EmailNotificationSaved(target, result) if self.saving_email_notification == Some(target) => {
                self.saving_email_notification = None;
                match result {
                    Ok(settings) => self.email_notifications = Some(settings),
                    Err(error) => {
                        self.email_notifications = None;
                        self.error = Some(error);
                        self.queued_page_load |= self.screen == Screen::Settings;
                    }
                }
            }
            Message::LoadInvitations
                if self.can_change_membership() && !self.loading_invitations && !self.changing_membership =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.loading_invitations = true;
                self.page_errors.remove(&PageRead::Invitations);
                let generation = self.invitation_generation;
                return Task::perform(async move { desktop_auth::load_invitations(&session).await },
                    move |result| Message::InvitationsLoaded(generation, result));
            }
            Message::InvitationsLoaded(generation, result) if generation == self.invitation_generation => {
                self.loading_invitations = false;
                self.queued_page_load |= self.screen == Screen::Home;
                match result {
                    Ok(rows) => {
                        if std::env::var_os("CLASSROOM_READ_ONLY_DIAGNOSTICS").is_some() {
                            println!("Class invitations ready; count={}", rows.len());
                            let _ = io::stdout().flush();
                        }
                        self.invitations = Some(rows);
                    }
                    Err(error) => {
                        self.page_errors.insert(PageRead::Invitations, error);
                    }
                }
            }
            Message::AcceptInvitation(course) if self.can_respond_to_invitation(course) => {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.confirming_decline = None;
                self.changing_membership = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::accept_invitation(&session, course).await },
                    move |result| Message::MembershipFinished(result.map(|()| MembershipChange::Joined(course))));
            }
            Message::ConfirmDeclineInvitation(course) if self.can_respond_to_invitation(course) => {
                self.confirming_decline = Some(course);
            }
            Message::CancelDeclineInvitation => self.confirming_decline = None,
            Message::DeclineInvitation(course)
                if self.confirming_decline == Some(course) && self.can_respond_to_invitation(course) =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.confirming_decline = None;
                self.changing_membership = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::decline_invitation(&session, course).await },
                    Message::InvitationDeclined);
            }
            Message::InvitationDeclined(result) => {
                self.changing_membership = false;
                match result {
                    Ok(remaining) => {
                        self.invalidate_invitation_read();
                        self.invitations = Some(remaining);
                        self.page_errors.remove(&PageRead::Invitations);
                        self.notice = Some("Class invitation declined.".into());
                    }
                    Err(error) => {
                        self.error = Some(error);
                        let invitations = self.update(Message::LoadInvitations);
                        return Task::batch([
                            invitations,
                            self.dequeue_page_load().map_or_else(Task::none, |read| self.update(read)),
                        ]);
                    }
                }
            }
            Message::EditClassCode(code) if self.screen == Screen::Join && !self.changing_membership => {
                self.class_code = code;
            }
            Message::ClassCreationPermission(generation, result) if generation == self.class_creation_generation => {
                self.class_creation_allowed = result.unwrap_or(false);
            }
            Message::EditNewClass(field, value) if self.screen == Screen::CreateClass && !self.changing_membership => {
                let input = match field {
                    NewClassField::Name => &mut self.new_class.name,
                    NewClassField::Section => &mut self.new_class.section,
                    NewClassField::Subject => &mut self.new_class.subject,
                    NewClassField::Room => &mut self.new_class.room,
                };
                *input = value;
            }
            Message::CreateClass if self.screen == Screen::CreateClass && self.can_create_class()
                && !self.has_pending_request() && desktop_auth::class_input_valid(&self.new_class) =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                let input = self.new_class.clone();
                self.error = None;
                self.notice = None;
                self.changing_membership = true;
                return Task::perform(async move { desktop_auth::create_class(&session, input).await },
                    |result| Message::MembershipFinished(result.map(MembershipChange::Created)));
            }
            Message::JoinClass
                if self.screen == Screen::Join && self.can_change_membership()
                    && !self.has_pending_request() && desktop_auth::valid_class_code(&self.class_code) =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                let code = self.class_code.clone();
                self.error = None;
                self.notice = None;
                self.changing_membership = true;
                return Task::perform(async move { desktop_auth::join_class(&session, code).await },
                    |result| Message::MembershipFinished(result.map(MembershipChange::Joined)));
            }
            Message::ConfirmUnenroll(course) if self.screen == Screen::Course(course) && self.can_change_membership() => {
                self.confirming_unenroll = Some(course);
            }
            Message::CancelUnenroll => self.confirming_unenroll = None,
            Message::Unenroll(course)
                if self.screen == Screen::Course(course) && self.confirming_unenroll == Some(course)
                    && self.can_change_membership() && !self.has_pending_request() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.confirming_unenroll = None;
                self.error = None;
                self.notice = None;
                self.changing_membership = true;
                return Task::perform(async move { desktop_auth::leave_class(&session, course).await },
                    move |result| Message::MembershipFinished(result.map(|()| MembershipChange::Left(course))));
            }
            Message::MembershipFinished(result) => {
                self.changing_membership = false;
                match result {
                    Ok(change) => {
                        self.invalidate_invitation_read();
                        self.class_code.clear();
                        self.screen = Screen::Home;
                        let message = match change {
                            MembershipChange::Created(id) => {
                                self.opening_joined_course = Some(id);
                                self.new_class = desktop_auth::NewClass::default();
                                "Class created."
                            }
                            MembershipChange::Joined(id) => {
                                self.opening_joined_course = Some(id);
                                if let Some(invitations) = &mut self.invitations {
                                    invitations.retain(|course| course.id != id);
                                }
                                "Class joined."
                            }
                            MembershipChange::Left(id) => {
                                if let Some(snapshot) = &mut self.snapshot {
                                    snapshot.courses.retain(|item| item.id != id);
                                    snapshot.coursework.remove(&id);
                                    snapshot.topics.remove(&id);
                                    snapshot.roster.remove(&id);
                                    self.submissions_checked.remove(&id);
                                }
                                "Unenrolled from class."
                            }
                        };
                        let refresh = self.update(Message::Refresh);
                        self.notice = Some(message.into());
                        return refresh;
                    }
                    Err(error) => {
                        let invitations = self.update(Message::LoadInvitations);
                        self.error = Some(error);
                        return invitations;
                    }
                }
            }
            Message::EditAttachmentLink(url) if !self.has_pending_request() => {
                self.attachment_link = url;
            }
            Message::AddAttachmentLink(course, item)
                if self.can_compose_comment(course, item) && !self.has_pending_request()
                    && trusted_attachment_host(self.attachment_link.trim()).is_some() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                let url = self.attachment_link.clone();
                self.sending_submission = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::attach_submission_link(&session, course, item, url).await
                }, move |result| Message::AttachmentLinkAdded(course, result));
            }
            Message::AttachmentLinkAdded(course, result) => {
                self.sending_submission = false;
                match result {
                    Ok(()) => {
                        self.attachment_link.clear();
                        self.notice = Some("Link attached.".into());
                        return self.update(Message::LoadDetail(DetailRead::Submissions(course)));
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Message::ChooseSubmissionFile(course, item)
                if self.can_compose_comment(course, item) && !self.has_pending_request() =>
            {
                self.choosing_submission_file = true;
                return Task::perform(choose_attachment_file(),
                    move |result| Message::SubmissionFileChosen(course, item, result));
            }
            Message::SubmissionFileChosen(course, item, result) => {
                self.choosing_submission_file = false;
                if self.can_compose_comment(course, item) {
                    match result {
                        Ok(Some(path)) => self.submission_file = Some((course, item, path)),
                        Ok(None) => {},
                        Err(error) => self.error = Some(error),
                    }
                }
            }
            Message::CancelSubmissionFile => self.submission_file = None,
            Message::UploadSubmissionFile(course, item)
                if self.can_compose_comment(course, item) && !self.has_pending_request() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                let Some((selected_course, selected_item, path)) = self.submission_file.take() else {
                    return Task::none();
                };
                if (selected_course, selected_item) != (course, item) {
                    return Task::none();
                }
                self.sending_submission = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::attach_submission_file(&session, course, item, path).await
                }, move |result| Message::SubmissionFileUploaded(course, result));
            }
            Message::SubmissionFileUploaded(course, result) => {
                self.sending_submission = false;
                match result {
                    Ok(()) => {
                        self.notice = Some("File uploaded and attached to your work.".into());
                        return self.update(Message::LoadDetail(DetailRead::Submissions(course)));
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Message::ConfirmRemoveMaterial(course, item, material)
                if !self.has_pending_request() && self.can_compose_comment(course, item) =>
            {
                self.pending_remove_material = Some((course, item, material));
            }
            Message::CancelRemoveMaterial => self.pending_remove_material = None,
            Message::RemoveMaterial(course, item, material)
                if self.pending_remove_material == Some((course, item, material))
                    && !self.has_pending_request() && self.can_compose_comment(course, item) =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.pending_remove_material = None;
                self.sending_submission = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::remove_submission_attachment(&session, course, item, material).await
                }, move |result| Message::MaterialRemoved(course, result));
            }
            Message::MaterialRemoved(course, result) => {
                self.sending_submission = false;
                match result {
                    Ok(()) => {
                        self.notice = Some("Attachment removed from your submission.".into());
                        return self.update(Message::LoadDetail(DetailRead::Submissions(course)));
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Message::ConfirmSubmission(course, item, action) if self.can_compose_comment(course, item) => {
                self.pending_submission = Some((course, item, action));
            }
            Message::CancelSubmission => self.pending_submission = None,
            Message::SendSubmission(course, item, action)
                if !self.has_pending_request() && self.can_compose_comment(course, item)
                    && self.pending_submission == Some((course, item, action)) =>
            {
                if let Some(session) = self.session.clone() {
                    self.pending_submission = None;
                    self.sending_submission = true;
                    self.error = None;
                    self.notice = None;
                    return Task::perform(async move {
                        desktop_auth::change_submission(&session, course, item, action).await
                    }, move |result| Message::SubmissionSent(course, result));
                }
            }
            Message::SubmissionSent(course, result) => {
                self.sending_submission = false;
                match result {
                    Ok(()) => self.notice = Some("Submission status updated.".into()),
                    Err(error) => self.error = Some(error),
                }
                return self.update(Message::LoadDetail(DetailRead::Submissions(course)));
            }
            Message::BeginShortAnswerEdit(course, item) if !self.has_pending_request() => {
                if let Some((submission_id, answer)) = self.current_question(course, item)
                    .and_then(desktop_auth::editable_short_answer)
                {
                    let editor = text_editor::Content::with_text(answer);
                    let revision = desktop_auth::ShortAnswerRevision { submission_id, answer: answer.to_owned() };
                    self.short_answer_drafts.insert((course, item), editor);
                    self.short_answer_revisions.insert((course, item), revision);
                }
            }
            Message::DiscardShortAnswerEdit(course, item)
                if self.can_compose_comment(course, item) && !self.sending_submission =>
            {
                self.confirming_question_answer = None;
                self.short_answer_drafts.remove(&(course, item));
                self.short_answer_revisions.remove(&(course, item));
            }
            Message::EditShortAnswer(course, item, action)
                if self.editable_question(course, item) == Some(&QuestionDetails::ShortAnswer) && !self.sending_submission
                    && self.confirming_question_answer.is_none() =>
            {
                self.short_answer_drafts.entry((course, item)).or_default().perform(action);
            }
            Message::SelectQuestionChoice(course, item, choice)
                if !self.sending_submission && self.confirming_question_answer.is_none()
                    && self.editable_question(course, item).is_some_and(|details| {
                        details.accepts(&QuestionAnswer::MultipleChoice(choice.clone()))
                    }) =>
            {
                self.choice_drafts.insert((course, item), choice);
            }
            Message::ConfirmQuestionAnswer(course, item)
                if !self.has_pending_request() && self.editable_question(course, item)
                    .and_then(|details| self.question_draft((course, item), details)).is_some() =>
            {
                self.confirming_question_answer = Some((course, item));
            }
            Message::CancelQuestionAnswer => self.confirming_question_answer = None,
            Message::SendQuestionAnswer(course, item)
                if self.confirming_question_answer == Some((course, item)) && !self.has_pending_request() =>
            {
                let Some(answer) = self.editable_question(course, item)
                    .and_then(|details| self.question_draft((course, item), details))
                else { return Task::none() };
                let Some(session) = self.session.clone() else { return Task::none() };
                let revision = self.short_answer_revisions.get(&(course, item)).cloned();
                self.confirming_question_answer = None;
                self.sending_submission = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::turn_in_question(&session, course, item, answer, revision).await
                }, move |result| Message::QuestionAnswerSent(course, item, result));
            }
            Message::QuestionAnswerSent(course, item, result) => {
                self.sending_submission = false;
                let refresh = self.update(Message::LoadDetail(DetailRead::Submissions(course)));
                match result {
                    Ok(()) => {
                        self.short_answer_drafts.remove(&(course, item));
                        self.short_answer_revisions.remove(&(course, item));
                        self.choice_drafts.remove(&(course, item));
                        self.notice = Some("Answer turned in.".into());
                    }
                    Err(error) => self.error = Some(error),
                }
                return refresh;
            }
            Message::ToggleRubric(course, item) if self.visible_rubric(course, item).is_some() => {
                self.rubric_open = !self.rubric_open;
            }
            Message::ToggleRubricCriterion(course, item, index)
                if self.rubric_open && self.visible_rubric(course, item).is_some_and(|rubric| index < rubric.criteria.len()) =>
            {
                if !self.open_criteria.remove(&index) {
                    self.open_criteria.insert(index);
                }
            }
            Message::SetRubricCriteria(course, item, expanded) if self.rubric_open => {
                if let Some(rubric) = self.visible_rubric(course, item) {
                    self.open_criteria = if expanded { (0..rubric.criteria.len()).collect() } else { BTreeSet::new() };
                }
            }
            Message::OpenItem(course, item) => {
                self.comment_mentions_generation = self.comment_mentions_generation.wrapping_add(1);
                self.comment_mentions = None;
                self.teacher_work = None;
                self.teacher_feedback.clear();
                self.gradebook_student = None;
                self.gradebook_edit = None;
                self.discard_submission_capture();
                self.confirming_comment_delete = None;
                self.drive_picker = None;
                self.new_submission_document_open = false;
                self.confirming_stream_delete = None;
                self.confirming_question_answer = None;
                self.rubric_open = false;
                self.open_criteria.clear();
                self.attachment_link.clear();
                self.submission_file = None;
                self.pending_remove_material = None;
                self.pending_submission = None;
                self.item_id = Some(item);
                self.previous_screen = Some(self.screen);
                self.screen = Screen::Item(course);
                self.retry_visible_reads_on_entry();
                self.queued_page_load = true;
            }
            Message::CloseItem => {
                self.comment_mentions_generation = self.comment_mentions_generation.wrapping_add(1);
                self.comment_mentions = None;
                self.teacher_work = None;
                self.teacher_feedback.clear();
                self.gradebook_student = None;
                self.discard_submission_capture();
                self.confirming_comment_delete = None;
                self.drive_picker = None;
                self.confirming_question_answer = None;
                self.attachment_link.clear();
                self.submission_file = None;
                self.pending_remove_material = None;
                self.pending_submission = None;
                self.screen = self.previous_screen.take().unwrap_or(Screen::Home);
                self.item_id = None;
                self.retry_visible_reads_on_entry();
                self.queued_page_load = true;
            }
            Message::SelectTopic(topic) if self.selected_topic != topic => {
                if let Some(topic) = &topic {
                    self.collapsed_classwork_groups.remove(&ClassworkGroupId::Topic(topic.clone()));
                }
                self.selected_topic = topic;
                self.classwork_toggles.clear();
            }
            Message::ToggleClassworkItem(course, item)
                if self.screen == Screen::Course(course) && self.tab == Tab::Classwork =>
            {
                if !self.classwork_toggles.remove(&item) {
                    self.classwork_toggles.insert(item);
                }
            }
            Message::ToggleClassworkGroup(course, group)
                if self.screen == Screen::Course(course) && self.tab == Tab::Classwork =>
            {
                if !self.collapsed_classwork_groups.remove(&group) {
                    self.collapsed_classwork_groups.insert(group);
                }
            }
            Message::SetClassworkGroups(course, collapse)
                if self.screen == Screen::Course(course) && self.tab == Tab::Classwork =>
            {
                self.collapsed_classwork_groups.clear();
                if collapse && let Some(snapshot) = &self.snapshot {
                    self.collapsed_classwork_groups.extend(snapshot.classwork_sections(course, None)
                        .into_iter().map(|section| section.id));
                }
            }
            Message::SetTodoFilter(filter) => self.todo_filter = filter,
            Message::SetWorkFilter(filter) => self.work_filter = filter,
            Message::FilterTodoClass(class) => self.todo_class = class.id,
            Message::ToggleTodoGroup(group) => {
                let key = (self.todo_filter, group);
                if !self.collapsed_todo_groups.remove(&key) {
                    self.collapsed_todo_groups.insert(key);
                }
            }
            Message::CalendarShift(direction) => {
                if let Some(monday) = self.calendar_monday.checked_add_signed(TimeDelta::weeks(direction.into()))
                    && monday.checked_add_days(Days::new(6)).is_some()
                {
                    self.calendar_monday = monday;
                }
            }
            Message::CalendarToday => self.calendar_monday = monday_of(Local::now().date_naive()),
            Message::CalendarFilter(class) => self.calendar_class = class.id,
            Message::OpenAttachment(url) if !self.verify_live_course_count => {
                if trusted_attachment_host(&url).is_none() {
                    self.error = Some("This attachment does not have a supported secure link.".into());
                } else {
                    return Task::perform(async move {
                        tokio::process::Command::new("/usr/bin/open")
                            .arg(url)
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .status()
                            .await
                            .is_ok_and(|status| status.success())
                    }, |opened| {
                        if opened { Message::AttachmentOpened } else { Message::AttachmentFailed }
                    });
                }
            }
            Message::OpenDriveFile(id) if self.can_read_pages() && self.opening_drive_file.is_none() => {
                let Some(session) = self.session.clone() else { return Task::none(); };
                self.error = None;
                self.opening_drive_file = Some(id.clone());
                return Task::perform(async move {
                    let result = desktop_auth::lookup_drive_file_link(&session, &id).await;
                    (id, result)
                }, |(id, result)| Message::DriveFileLinkLoaded(id, result));
            }
            Message::DriveFileLinkLoaded(id, result) if self.opening_drive_file.as_ref() == Some(&id) => {
                self.opening_drive_file = None;
                match result {
                    Ok(url) => return self.update(Message::OpenAttachment(url)),
                    Err(error) => self.error = Some(error),
                }
            }
            Message::ClassFoldersLoaded(generation, result) if generation == self.class_folders_generation => {
                match result {
                    Ok(details) => {
                        if std::env::var_os("CLASSROOM_READ_ONLY_DIAGNOSTICS").is_some() {
                            println!("Class Drive folders ready; count={}", details.folders.len());
                            let _ = io::stdout().flush();
                        }
                        self.class_folders = details.folders;
                        self.class_roles = details.roles;
                        self.queued_page_load |= matches!(self.screen, Screen::Course(_) | Screen::Item(_));
                    }
                    Err(_) => self.class_folders_failed = true,
                }
            }
            Message::RetryClassFolders if self.class_folders_failed => return self.fetch_class_folders(),
            Message::ToggleTeacherDraft(course, item) if self.screen == Screen::Course(course)
                && self.class_roles.get(&course) == Some(&CourseRole::Teacher)
                && self.teacher_drafts.get(&course).is_some_and(|drafts| drafts.iter().any(|draft| draft.item.id == item)) =>
            {
                let key = (course, item);
                self.open_teacher_draft = (self.open_teacher_draft.as_ref() != Some(&key)).then_some(key);
            }
            Message::OpenAssignmentEditor(course, item) if self.can_edit_assignment_drafts(course)
                && !self.has_pending_request() && self.assignment_editor.is_none() && self.assignment_publication.is_none() =>
            {
                let original = if let Some(id) = item {
                    let Some(draft) = self.teacher_drafts.get(&course).and_then(|drafts| drafts.iter().find(|draft|
                        draft.item.id == id && draft.item.kind == CourseworkKind::Assignment
                        && draft.publication == desktop_auth::DraftPublication::Draft)) else { return Task::none(); };
                    Some(draft.clone())
                } else {
                    None
                };
                self.assignment_editor = Some(AssignmentEditor::new(course, original));
                self.error = None;
            }
            Message::EditAssignmentTitle(title) if !self.saving_assignment_draft && title.chars().count() <= 3_000 => {
                if let Some(editor) = &mut self.assignment_editor {
                    editor.title = title;
                }
            }
            Message::EditAssignmentInstructions(action) if !self.saving_assignment_draft => {
                if let Some(editor) = self.assignment_editor.as_mut().filter(|editor| editor.instructions_editable()) {
                    editor.instructions.perform(action);
                }
            }
            Message::EditAssignmentDueDate(value) if !self.saving_assignment_draft && value.len() <= 16 => {
                if let Some(editor) = &mut self.assignment_editor {
                    editor.settings.due_date = value;
                }
            }
            Message::EditAssignmentDueTime(value) if !self.saving_assignment_draft && value.len() <= 12 => {
                if let Some(editor) = &mut self.assignment_editor {
                    editor.settings.due_time = value;
                }
            }
            Message::EditAssignmentPoints(value) if !self.saving_assignment_draft && value.len() <= 32 => {
                if let Some(editor) = &mut self.assignment_editor {
                    editor.settings.points = value;
                }
            }
            Message::CloseAssignmentEditor if !self.saving_assignment_draft => self.assignment_editor = None,
            Message::SaveAssignmentDraft if !self.has_pending_request() => {
                let Some(editor) = self.assignment_editor.as_ref().filter(|editor| self.can_edit_assignment_drafts(editor.course))
                    else { return Task::none() };
                let (course, original, title, instructions) =
                    (editor.course, editor.original.clone(), editor.title.clone(), editor.instructions_text());
                if !desktop_auth::assignment_draft_text_valid(&title, &instructions) { return Task::none(); }
                let fields = match editor.field_changes() {
                    Ok(fields) => fields,
                    Err(error) => {
                        self.error = Some(error);
                        return Task::none();
                    }
                };
                let Some(session) = self.session.clone() else { return Task::none() };
                self.saving_assignment_draft = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::save_assignment_draft_with_fields(&session, course, original, title, instructions, fields).await
                }, move |result| Message::AssignmentDraftSaved(course, result));
            }
            Message::AssignmentDraftSaved(course, result) => {
                self.saving_assignment_draft = false;
                match result {
                    Ok(draft) => {
                        self.assignment_editor = None;
                        self.open_teacher_draft = Some((course, draft.item.id));
                        self.teacher_drafts.remove(&course);
                        self.notice = Some("Assignment draft saved.".into());
                        self.queued_page_load |= self.screen == Screen::Course(course);
                    }
                    Err(error) => {
                        self.error = Some(error);
                        self.teacher_drafts.remove(&course);
                        self.queued_page_load |= self.screen == Screen::Course(course);
                    }
                }
            }
            Message::OpenAssignmentPublication(course, id, scheduling) if self.can_edit_assignment_drafts(course)
                && !self.has_pending_request() && self.assignment_editor.is_none() && self.assignment_publication.is_none() =>
            {
                use desktop_auth::DraftPublication;
                let Some(draft) = self.teacher_drafts.get(&course).and_then(|drafts| drafts.iter().find(|draft|
                    draft.item.id == id && draft.item.kind == CourseworkKind::Assignment))
                    else { return Task::none(); };
                let schedule = match (scheduling, draft.publication) {
                    (false, DraftPublication::Draft) => None,
                    (true, DraftPublication::Draft | DraftPublication::Scheduled(None)) => Some(String::new()),
                    (true, DraftPublication::Scheduled(Some(time))) => Some(DateTime::from_timestamp_millis(time)
                        .map_or_else(String::new, |date| date.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string())),
                    _ => return Task::none(),
                };
                self.assignment_publication = Some(AssignmentPublicationPrompt { draft: draft.clone(), schedule });
                self.error = None;
            }
            Message::EditAssignmentSchedule(value) if !self.saving_assignment_draft && value.len() <= 32 => {
                if let Some(schedule) = self.assignment_publication.as_mut().and_then(|prompt| prompt.schedule.as_mut()) {
                    *schedule = value;
                }
            }
            Message::CloseAssignmentPublication if !self.saving_assignment_draft => self.assignment_publication = None,
            Message::FilterStudentWork(filter) => {
                if let Some(work) = &mut self.teacher_work {
                    work.filter = filter;
                    work.select(None);
                }
            }
            Message::SelectTeacherStudent(student) => {
                if let Some(work) = &mut self.teacher_work
                    && work.students.iter().any(|row| row.student_id == student && work.filter.includes(row))
                {
                    work.select((work.selected != Some(student)).then_some(student));
                    self.queued_page_load = true;
                }
            }
            Message::EditTeacherGrade(value) if !self.saving_teacher_grade && value.len() <= 20 => {
                if let Some(work) = &mut self.teacher_work {
                    work.grade = value;
                    work.confirming_return = false;
                }
            }
            Message::ConfirmReturnTeacherWork(confirm) if !self.saving_teacher_grade => {
                if let Some(work) = &mut self.teacher_work {
                    work.confirming_return = confirm;
                }
            }
            Message::SaveTeacherGrade | Message::ReturnTeacherWork if !self.has_pending_request() => {
                let Some(work) = &self.teacher_work else { return Task::none(); };
                let (course, item) = (work.course, work.item);
                if !self.can_read_teacher_work(course, item) || !self.can_grade_teacher_work(course) {
                    return Task::none();
                }
                let (Some(row), Some(session)) = (work.selected_row().cloned(), self.session.clone()) else { return Task::none(); };
                let action = if matches!(message, Message::SaveTeacherGrade) {
                    let Ok(value) = work.grade.trim().parse::<f64>() else { return Task::none(); };
                    desktop_auth::TeacherGradingAction::SaveDraft(value)
                } else if work.confirming_return && grade_input_matches_saved(&work.grade, &row) {
                    desktop_auth::TeacherGradingAction::Return
                } else {
                    return Task::none();
                };
                let maximum = self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(course))
                    .and_then(|items| items.iter().find(|entry| entry.id.parse::<i64>().ok() == Some(item)))
                    .and_then(|entry| entry.maximum_points);
                if !action.allowed(&row, maximum) { return Task::none(); }
                self.saving_teacher_grade = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::write_teacher_grade(&session, course, item, row, action).await
                }, move |result| Message::TeacherGradeWritten(course, item, action, result));
            }
            Message::TeacherGradeWritten(course, item, action, result) => {
                self.saving_teacher_grade = false;
                self.gradebooks.remove(&course);
                if self.can_read_teacher_work(course, item) {
                    if let Some(work) = &mut self.teacher_work {
                        work.confirming_return = false;
                    }
                    match result {
                        Ok(()) => {
                            self.notice = Some(match action {
                                desktop_auth::TeacherGradingAction::SaveDraft(_) => "Draft grade saved.",
                                desktop_auth::TeacherGradingAction::Return => "Work returned.",
                            }.into());
                        }
                        Err(error) => self.error = Some(error),
                    }
                    return self.update(Message::LoadDetail(DetailRead::TeacherSubmissions(course, item)));
                }
            }
            Message::SelectBulkReturn(student, selected) if !self.has_pending_request() => {
                if let Some(work) = &mut self.teacher_work
                    && work.returnable().any(|row| row.student_id == student)
                {
                    if selected { work.bulk_selected.insert(student); } else { work.bulk_selected.remove(&student); }
                    work.bulk_confirming = false;
                }
            }
            Message::SelectVisibleReturns(selected) if !self.has_pending_request() => {
                if let Some(work) = &mut self.teacher_work {
                    let visible: BTreeSet<_> = work.returnable().filter(|row| work.filter.includes(row))
                        .map(|row| row.student_id).collect();
                    if selected {
                        work.bulk_selected.extend(visible);
                    } else {
                        work.bulk_selected.retain(|id| !visible.contains(id));
                    }
                    work.bulk_confirming = false;
                }
            }
            Message::ClearBulkReturns if !self.has_pending_request() => {
                if let Some(work) = &mut self.teacher_work {
                    work.bulk_selected.clear();
                    work.bulk_confirming = false;
                }
            }
            Message::ConfirmBulkReturn(confirm) if !self.has_pending_request() => {
                if let Some(work) = &mut self.teacher_work {
                    let has_selected = work.selected_returns().next().is_some();
                    work.bulk_confirming = confirm && has_selected;
                    work.confirming_return = false;
                }
            }
            Message::ReturnSelectedTeacherWork if !self.has_pending_request() => {
                let Some(work) = &self.teacher_work else { return Task::none(); };
                let (course, item) = (work.course, work.item);
                if !work.bulk_confirming || !self.can_read_teacher_work(course, item) || !self.can_grade_teacher_work(course) {
                    return Task::none();
                }
                let selected: Vec<_> = work.selected_returns().cloned().collect();
                let count = selected.len();
                if count == 0 || count != work.bulk_selected.len() { return Task::none(); }
                let Some(session) = self.session.clone() else { return Task::none(); };
                self.saving_teacher_grade = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::return_teacher_submissions(&session, course, item, selected).await
                }, move |result| Message::BulkTeacherWorkReturned(course, item, count, result));
            }
            Message::BulkTeacherWorkReturned(course, item, requested, result) => {
                self.saving_teacher_grade = false;
                self.gradebooks.remove(&course);
                if self.can_read_teacher_work(course, item) {
                    if let Some(work) = &mut self.teacher_work {
                        work.bulk_selected.clear();
                        work.bulk_confirming = false;
                    }
                    match result {
                        Ok(confirmed) if confirmed == requested => self.notice = Some(format!("{confirmed} submissions returned.")),
                        Ok(confirmed) => self.error = Some(format!("Classroom confirmed {confirmed} of {requested} returns. Review the updated submissions before retrying.")),
                        Err(error) => self.error = Some(error),
                    }
                    return self.update(Message::LoadDetail(DetailRead::TeacherSubmissions(course, item)));
                }
            }
            Message::EditTeacherFeedback(key, text) if text.len() <= 10_000
                && self.selected_teacher_feedback_key() == Some(key) && !self.posting_comment =>
            {
                self.teacher_feedback.entry(key).or_default().draft = text;
            }
            Message::SendTeacherFeedback(key) if !self.has_pending_request()
                && self.selected_teacher_feedback_key() == Some(key) && self.can_grade_teacher_work(key.course) =>
            {
                let (Some(session), Some(thread)) = (self.session.clone(), self.teacher_feedback.get(&key)) else {
                    return Task::none();
                };
                if thread.unconfirmed || thread.comments.is_none() || thread.draft.trim().is_empty() {
                    return Task::none();
                }
                let draft = thread.draft.clone();
                self.posting_comment = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::post_teacher_private_feedback(&session, key, draft).await },
                    move |result| Message::TeacherFeedbackSent(key, result));
            }
            Message::TeacherFeedbackSent(key, result) => {
                self.posting_comment = false;
                if let Some(thread) = self.teacher_feedback.get_mut(&key) {
                    thread.comments = None;
                    self.page_errors.remove(&PageRead::Detail(DetailRead::TeacherFeedback(key)));
                    match result {
                        Ok(()) => {
                            thread.draft.clear();
                            self.notice = Some("Private feedback sent.".into());
                        }
                        Err(error) => {
                            thread.unconfirmed = true;
                            self.error = Some(error);
                        }
                    }
                }
                self.queued_page_load |= self.selected_teacher_feedback_key().is_some();
            }
            Message::ConfirmAssignmentPublication if !self.has_pending_request() => {
                let Some(prompt) = &self.assignment_publication else { return Task::none(); };
                let course = prompt.draft.item.course_id;
                if !self.can_edit_assignment_drafts(course) { return Task::none(); }
                let (Some(session), Some(action)) = (self.session.clone(), prompt.action()) else { return Task::none(); };
                let draft = prompt.draft.clone();
                self.saving_assignment_draft = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::change_assignment_publication(&session, course, draft, action).await
                }, move |result| Message::AssignmentPublicationChanged(course, action, result));
            }
            Message::AssignmentPublicationChanged(course, action, result) => {
                self.saving_assignment_draft = false;
                self.teacher_drafts.remove(&course);
                self.queued_page_load |= self.screen == Screen::Course(course);
                let published = action == desktop_auth::AssignmentPublication::Publish;
                match result {
                    Ok(()) => {
                        self.assignment_publication = None;
                        self.notice = Some(if published { "Assignment published." } else { "Assignment scheduled." }.into());
                    }
                    Err(error) => self.error = Some(error),
                }
                if published {
                    return self.update(Message::LoadCoursework(course));
                }
            }
            Message::ClassTeachersLoaded(generation, Ok(teachers)) if generation == self.teachers_generation => {
                self.class_teachers = teachers;
            }
            Message::AttachmentOpened => {}
            Message::AttachmentFailed => self.error = Some("The attachment couldn't be opened in your browser.".into()),
            Message::ArtworkLoaded(generation, url, image) if generation == self.artwork_generation => {
                self.artwork.insert(url, image);
            }
            Message::SaveOffline if !self.has_pending_request() => self.save_offline(),
            Message::OpenOffline if !self.has_pending_request() => self.open_offline(),
            Message::DeleteOffline if !self.has_pending_request() => self.delete_offline(),
            Message::ScreenshotReady(shot, image) => {
                let Some(directory) = &self.screenshot_directory else { return Task::none() };
                if let Err(error) = save_screenshot(directory, shot, &image) {
                    eprintln!("Could not save screenshot: {error}");
                    return iced::exit();
                }
                match shot {
                    Shot::Dashboard => {
                        self.invitations = Some(vec![Course {
                            id: 700, title: "Art & Design".into(), subtitle: "Year 12 · Group A".into(),
                            ..Default::default()
                        }]);
                        return schedule_capture(Shot::Invitations);
                    }
                    Shot::Invitations => {
                        self.confirming_decline = Some(700);
                        return schedule_capture(Shot::DeclineInvitation);
                    }
                    Shot::DeclineInvitation => {
                        self.confirming_decline = None;
                        self.invitations = None;
                        if let Some(course) = self.snapshot.as_ref().and_then(|data| data.courses.first()) {
                            self.screen = Screen::Course(course.id);
                            self.tab = Tab::Classwork;
                            return schedule_capture(Shot::Classwork);
                        }
                    }
                    Shot::Classwork => {
                        self.selected_topic = Some("demo-poetry-unit".into());
                        return schedule_capture(Shot::ClassworkTopic);
                    }
                    Shot::ClassworkTopic => {
                        self.selected_topic = None;
                        self.screen = Screen::Course(2);
                        self.tab = Tab::YourWork;
                        self.overall_grades.insert(2, Some(CourseGrade {
                            percent: 87.6644, calculation: GradeCalculation::TotalPoints,
                        }));
                        return schedule_capture(Shot::YourWork);
                    }
                    Shot::YourWork => {
                        self.overall_grades.remove(&2);
                        self.screen = Screen::ToDo;
                        return schedule_capture(Shot::ToDo);
                    }
                    Shot::ToDo => {
                        if let Some(question) = self.snapshot.as_mut()
                            .and_then(|data| data.coursework.get_mut(&4))
                            .and_then(|items| items.first_mut())
                        {
                            question.question = Some(QuestionDetails::MultipleChoice(vec![
                                "Source A".into(), "Source B".into(), "Both sources".into(),
                            ]));
                            question.question_sharing = Some(QuestionSharing::Course);
                            question.submission = Some(Submission {
                                status: SubmissionStatus::TurnedIn,
                                question_answer: Some(QuestionAnswer::MultipleChoice("Both sources".into())),
                                last_turned_in_millis: Some(1_784_600_123_000),
                                ..Default::default()
                            });
                            self.screen = Screen::Item(4);
                            self.item_id = Some(question.id.clone());
                            return schedule_capture(Shot::Question);
                        }
                        self.screen = Screen::Calendar;
                        return schedule_capture(Shot::Calendar);
                    }
                    Shot::Question => {
                        if let Some(question) = self.snapshot.as_mut()
                            .and_then(|data| data.coursework.get_mut(&4))
                            .and_then(|items| items.first_mut())
                        {
                            question.id = "42".into();
                            question.question = Some(QuestionDetails::ShortAnswer);
                            question.question_sharing = None;
                            question.submission = Some(Submission {
                                status: SubmissionStatus::Assigned,
                                remote_id: Some(67),
                                remote_state: Some(4),
                                ..Default::default()
                            });
                            self.item_id = Some(question.id.clone());
                            self.short_answer_drafts.insert((4, 42), text_editor::Content::with_text(
                                "Source A describes the event from a witness’s perspective. Source B explains how it was interpreted later.",
                            ));
                            return schedule_capture(Shot::ShortAnswer);
                        }
                        self.screen = Screen::Calendar;
                        return schedule_capture(Shot::Calendar);
                    }
                    Shot::ShortAnswer => {
                        if let Some(question) = self.snapshot.as_mut()
                            .and_then(|data| data.coursework.get_mut(&4))
                            .and_then(|items| items.first_mut())
                        {
                            let original = "Source A is a witness’s account.";
                            question.allow_edit_after_turn_in = true;
                            question.submission = Some(Submission {
                                status: SubmissionStatus::TurnedIn,
                                remote_id: Some(67),
                                remote_state: Some(2),
                                question_answer: Some(QuestionAnswer::ShortAnswer(original.into())),
                                ..Default::default()
                            });
                            self.short_answer_drafts.remove(&(4, 42));
                            return schedule_capture(Shot::SubmittedShortAnswer);
                        }
                        self.screen = Screen::Calendar;
                        return schedule_capture(Shot::Calendar);
                    }
                    Shot::SubmittedShortAnswer => {
                        self.short_answer_drafts.insert((4, 42), text_editor::Content::with_text(
                            "Source A is a witness’s account. Source B describes how the event was interpreted later.",
                        ));
                        self.short_answer_revisions.insert((4, 42), desktop_auth::ShortAnswerRevision {
                            submission_id: 67, answer: "Source A is a witness’s account.".into(),
                        });
                        return schedule_capture(Shot::ShortAnswerEdit);
                    }
                    Shot::ShortAnswerEdit => {
                        if let Some(question) = self.snapshot.as_mut()
                            .and_then(|data| data.coursework.get_mut(&4))
                            .and_then(|items| items.first_mut())
                        {
                            question.question = Some(QuestionDetails::MultipleChoice(vec![
                                "Source A".into(), "Source B".into(), "Both sources".into(),
                            ]));
                            question.allow_edit_after_turn_in = false;
                            question.submission = Some(Submission {
                                status: SubmissionStatus::Assigned,
                                remote_id: Some(67),
                                remote_state: Some(4),
                                ..Default::default()
                            });
                            self.short_answer_drafts.remove(&(4, 42));
                            self.short_answer_revisions.remove(&(4, 42));
                            self.choice_drafts.insert((4, 42), "Both sources".into());
                            self.confirming_question_answer = Some((4, 42));
                            return schedule_capture(Shot::MultipleChoiceDraft);
                        }
                        self.screen = Screen::Calendar;
                        return schedule_capture(Shot::Calendar);
                    }
                    Shot::MultipleChoiceDraft => {
                        self.confirming_question_answer = None;
                        if let Some(item) = self.snapshot.as_mut().and_then(|data| data.coursework.get_mut(&1))
                            .and_then(|items| items.iter_mut().find(|item| item.kind == CourseworkKind::Assignment))
                        {
                            item.id = "101".into();
                            item.submission = Some(Submission {
                                status: SubmissionStatus::Returned,
                                grade: Some(desktop_data::Grade { earned: 8.0, maximum: Some(10.0) }),
                                rubric_scores: BTreeMap::from([("analysis".into(), desktop_data::CriterionScore {
                                    rating_id: Some("clear".into()), points: Some(8.0),
                                })]),
                                ..Default::default()
                            });
                            self.screen = Screen::Item(1);
                            self.item_id = Some(item.id.clone());
                            self.rubrics.insert((1, 101), Some(Rubric {
                                title: Some("Poetry comparison".into()),
                                criteria: vec![desktop_data::RubricCriterion {
                                    id: Some("analysis".into()),
                                    title: "Analysis".into(),
                                    description: "Explain how each poem develops the theme.".into(),
                                    ratings: vec![
                                        desktop_data::RubricRating {
                                            id: Some("detailed".into()), title: "Detailed".into(),
                                            description: "Compares both poems using specific textual evidence.".into(), points: Some(10.0),
                                        },
                                        desktop_data::RubricRating {
                                            id: Some("clear".into()), title: "Clear".into(),
                                            description: "Explains the theme using clear examples.".into(), points: Some(8.0),
                                        },
                                    ],
                                }],
                            }));
                            self.rubric_open = true;
                            self.open_criteria.insert(0);
                            return schedule_capture(Shot::Rubric);
                        }
                        self.screen = Screen::Calendar;
                        return schedule_capture(Shot::Calendar);
                    }
                    Shot::Rubric => {
                        self.screen = Screen::Calendar;
                        return schedule_capture(Shot::Calendar);
                    }
                    Shot::Calendar => {
                        self.calendar_class = Some(1);
                        if let Some(course) = self.snapshot.as_mut()
                            .and_then(|snapshot| snapshot.courses.iter_mut().find(|course| course.id == 1))
                        {
                            course.calendar_url = Some("https://calendar.google.com/calendar/u/0?cid=fixture".into());
                        }
                        return schedule_capture(Shot::ClassCalendar);
                    }
                    Shot::ClassCalendar => {
                        self.calendar_class = None;
                        return window::latest().then(|id| match id {
                            Some(id) => window::resize(id, iced::Size::new(810.0, 565.0))
                                .chain(schedule_capture(Shot::CompactCalendar)),
                            None => iced::exit(),
                        });
                    }
                    Shot::CompactCalendar => {
                        self.screen = Screen::Course(1);
                        self.tab = Tab::Classwork;
                        self.selected_topic = Some("demo-poetry-unit".into());
                        return schedule_capture(Shot::CompactClasswork);
                    }
                    Shot::CompactClasswork => {
                        self.screen = Screen::Settings;
                        self.class_notifications = Some(self.class_notification_courses()
                            .enumerate().map(|(index, course)| (course.id, index == 1)).collect());
                        self.email_notifications = Some(EmailNotificationSettings {
                            enabled: Some(true),
                            categories: vec![desktop_auth::EmailNotificationCategory {
                                label: Some("Comments".into()),
                                groups: vec![
                                    desktop_auth::EmailNotificationGroup { tag: 1, label: "Comments on your posts".into(), enabled: Some(true) },
                                    desktop_auth::EmailNotificationGroup { tag: 2, label: "Comments that mention you".into(), enabled: Some(false) },
                                ],
                            }],
                        });
                        return window::latest().then(|id| match id {
                            Some(id) => window::resize(id, iced::Size::new(1200.0, 810.0))
                                .chain(schedule_capture(Shot::Settings)),
                            None => Task::none(),
                        });
                    }
                    Shot::Settings => {
                        self.screen = Screen::Course(1);
                        self.tab = Tab::Stream;
                        self.stream_access.insert(1, StreamAccess::PostAndComment);
                        self.open_stream_composer = Some(1);
                        let sample = "Read the first chapter before Friday.\nBring one question.\nReview your class notes.";
                        let mut styles = vec![0; sample.len()];
                        styles[..22].fill(1);
                        styles[29..35].fill(2 | 4);
                        self.stream_drafts.insert(1, StreamDraft {
                            body: text_editor::Content::with_text(sample),
                            styles,
                            bullets: paragraph_starts(sample).skip(1).collect(),
                            links: vec!["https://www.bl.uk/learning".into()],
                            ..Default::default()
                        });
                        return schedule_capture(Shot::StreamComposer);
                    }
                    Shot::StreamComposer => {
                        if let Some(draft) = self.stream_drafts.get_mut(&1) {
                            draft.selected_file = None;
                        }
                        self.drive_picker = Some(DrivePicker {
                            course: 1,
                            file_page: Some(DriveFilePage {
                                folders: vec![DriveFolder { id: "fixture_folder".into(), name: "English Literature".into() }],
                                files: [
                                    ("fixture_reading", "Course reading notes", "application/vnd.google-apps.document"),
                                    ("fixture_study", "Poetry study guide.pdf", "application/pdf"),
                                    ("fixture_examples", "Worked examples", "application/vnd.google-apps.presentation"),
                                ].into_iter().filter_map(|(id, name, mime)| {
                                    DriveFileAttachment::existing(id.into(), name.into(), mime.into())
                                }).collect(),
                                next_page: None,
                                incomplete: false,
                            }),
                            section: DriveSection::MyDrive,
                            ..Default::default()
                        });
                        return schedule_capture(Shot::DrivePicker);
                    }
                    Shot::DrivePicker => {
                        if let Some(item) = self.snapshot.as_mut().and_then(|snapshot| snapshot.coursework.get_mut(&1))
                            .and_then(|items| items.iter_mut().find(|item| item.kind == CourseworkKind::Assignment))
                        {
                            item.id = "42".into();
                            item.submission = Some(Submission {
                                remote_id: Some(123),
                                remote_state: Some(4),
                                status: SubmissionStatus::Assigned,
                                ..Default::default()
                            });
                            self.screen = Screen::Item(1);
                            self.item_id = Some("42".into());
                            if let Some(picker) = &mut self.drive_picker {
                                picker.item = Some(42);
                            }
                        }
                        return schedule_capture(Shot::SubmissionDrivePicker);
                    }
                    Shot::SubmissionDrivePicker => {
                        self.screen = Screen::Course(1);
                        self.item_id = None;
                        self.drive_picker = None;
                        self.open_stream_composer = None;
                        if let Some(post) = self.snapshot.as_mut().and_then(|snapshot| snapshot.coursework.get_mut(&1))
                            .and_then(|items| items.iter_mut().find(|item| item.kind == CourseworkKind::Announcement))
                        {
                            post.id = "91".into();
                            post.author_id = Some(1);
                            self.stream_viewer_id = Some(1);
                            self.confirming_stream_delete = Some((1, 91));
                        }
                        return schedule_capture(Shot::StreamDelete);
                    }
                    Shot::StreamDelete => {
                        self.confirming_stream_delete = None;
                        if let Some(comment) = self.snapshot.as_mut()
                            .and_then(|snapshot| snapshot.coursework.get_mut(&1))
                            .and_then(|items| items.iter_mut().find(|item| item.id == "91"))
                            .and_then(|item| item.comments.as_mut()).and_then(|comments| comments.first_mut())
                        {
                            comment.id = "92".into();
                            comment.author_id = Some(1);
                            self.stream_viewer_id = Some(1);
                            self.screen = Screen::Item(1);
                            self.item_id = Some("91".into());
                            self.confirming_comment_delete = Some(CommentTarget { course: 1, item: 91, id: 92, audience: CommentAudience::Class });
                        }
                        return schedule_capture(Shot::CommentDelete);
                    }
                    Shot::CommentDelete => {
                        self.confirming_comment_delete = None;
                        self.screen = Screen::Course(1);
                        self.item_id = None;
                        return window::latest().then(|id| match id {
                            Some(id) => window::resize(id, iced::Size::new(810.0, 565.0))
                                .chain(schedule_capture(Shot::CompactStream)),
                            None => Task::none(),
                        });
                    }
                    Shot::CompactStream => {
                        self.screen = Screen::Home;
                        self.show_hidden_classes = true;
                        if let Some(snapshot) = &mut self.snapshot {
                            snapshot.courses.iter_mut().for_each(|course| {
                                course.hidden = Some(course.id == 5);
                            });
                            snapshot.courses.retain(|course| course.id <= 3 || course.id == 5);
                        }
                        self.open_course_menu = Some(5);
                        return window::latest().then(|id| match id {
                            Some(id) => window::resize(id, iced::Size::new(1200.0, 1020.0))
                                .chain(schedule_capture(Shot::HiddenClasses)),
                            None => Task::none(),
                        });
                    }
                    Shot::HiddenClasses => {
                        self.open_course_menu = Some(1);
                        self.move_course_menu = Some(1);
                        return window::latest().then(|id| match id {
                            Some(id) => window::resize(id, iced::Size::new(1200.0, 810.0))
                                .chain(schedule_capture(Shot::MoveCourse)),
                            None => Task::none(),
                        });
                    }
                    Shot::MoveCourse => {
                        self.move_course_menu = None;
                        self.class_roles.insert(1, CourseRole::Teacher);
                        self.confirm_class_archive = Some(1);
                        return schedule_capture(Shot::ArchiveClass);
                    }
                    Shot::ArchiveClass => {
                        self.screen = Screen::Home;
                        self.open_course_menu = None;
                        self.move_course_menu = None;
                        self.confirm_class_archive = None;
                        self.class_roles.remove(&1);
                        self.classroom_link_input = ClassroomLink::Item { course: 1, item: 42, kind: ItemLinkKind::Assignment }.web_url();
                        return schedule_capture(Shot::ClassroomLink);
                    }
                    Shot::ClassroomLink => {
                        self.classroom_link_input = None;
                        self.screen = Screen::Course(1);
                        self.tab = Tab::Stream;
                        self.set_demo_stream_display(StreamClassworkDisplay::Condensed);
                        return schedule_capture(Shot::CondensedStream);
                    }
                    Shot::CondensedStream => {
                        self.set_demo_stream_display(StreamClassworkDisplay::Expanded);
                        if let Some(item) = self.snapshot.as_mut().and_then(|snapshot| snapshot.coursework.get_mut(&1))
                            .and_then(|items| items.iter_mut().find(|item| item.kind == CourseworkKind::Assignment))
                        {
                            item.attachments = vec![desktop_data::Attachment {
                                title: "Poetry reading notes".into(),
                                kind: desktop_data::AttachmentKind::Link,
                                url: Some("https://example.org/reading".into()),
                                remote_id: None,
                                drive_sharing: None,
                                drive_reference: None,
                                drive_parent_reference: None,
                            }];
                        }
                        return schedule_capture(Shot::ExpandedStream);
                    }
                    Shot::ExpandedStream => {
                        self.set_demo_stream_display(StreamClassworkDisplay::Hidden);
                        return schedule_capture(Shot::HiddenStream);
                    }
                    Shot::HiddenStream => {
                        if let Some(item) = self.snapshot.as_mut().and_then(|snapshot| snapshot.coursework.get_mut(&1))
                            .and_then(|items| items.iter_mut().find(|item| item.kind == CourseworkKind::Assignment))
                        {
                            item.id = "42".into();
                            item.submission = Some(Submission {
                                remote_id: Some(123), remote_state: Some(4), status: SubmissionStatus::Assigned,
                                ..Default::default()
                            });
                            self.screen = Screen::Item(1);
                            self.item_id = Some("42".into());
                            self.pending_submission = Some((1, 42, desktop_auth::SubmissionAction::TurnIn));
                        }
                        return schedule_capture(Shot::MarkDone);
                    }
                    Shot::MarkDone => {
                        self.pending_submission = None;
                        self.new_submission_document_open = true;
                        return schedule_capture(Shot::NewSubmissionDocument);
                    }
                    Shot::NewSubmissionDocument => {
                        #[cfg(target_os = "macos")]
                        {
                            let colours = [[224, 226, 230, 255], [215, 187, 101, 255], [111, 184, 191, 255],
                                [115, 179, 138, 255], [172, 137, 181, 255], [202, 119, 117, 255], [104, 142, 192, 255]];
                            let (width, height) = (420, 240);
                            let pixels: Vec<_> = (0..height).flat_map(|_| (0..width)
                                .flat_map(|x| colours[x * colours.len() / width])).collect();
                            self.new_submission_document_open = false;
                            self.submission_capture = Some(CaptureState::RecordingCamera {
                                control: camera::Control::screenshot_fixture(), phase: CameraPhase::Framing,
                                frame: Some(image::Handle::from_rgba(width as u32, height as u32, pixels)),
                            });
                            return schedule_capture(Shot::CameraFraming);
                        }
                    }
                    #[cfg(target_os = "macos")]
                    Shot::CameraFraming => {
                        self.discard_submission_capture();
                        if let Some(item) = self.snapshot.as_mut().and_then(|snapshot| snapshot.coursework.get_mut(&1))
                            .and_then(|items| items.iter_mut().find(|item| item.kind == CourseworkKind::Assignment))
                            && let Some(submission) = &mut item.submission
                        {
                            let template = desktop_data::Attachment {
                                remote_id: None,
                                title: "Poetry comparison worksheet".into(),
                                url: Some("https://docs.google.com/document/d/demo-worksheet/edit".into()),
                                kind: desktop_data::AttachmentKind::DriveFile,
                                drive_sharing: Some(DriveSharing::IndividualCopy),
                                drive_reference: Some("demo-worksheet".into()),
                                drive_parent_reference: None,
                            };
                            submission.attachments.push(desktop_data::Attachment {
                                title: "Sam — Poetry comparison worksheet".into(),
                                url: Some("https://docs.google.com/document/d/demo-personal-worksheet/edit".into()),
                                drive_sharing: None,
                                drive_reference: Some("demo-personal-worksheet".into()),
                                drive_parent_reference: template.drive_reference.clone(),
                                ..template.clone()
                            });
                            item.attachments.push(template);
                            item.originality_enabled = Some(true);
                        }
                        return schedule_capture(Shot::PersonalCopy);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::PersonalCopy => {
                        self.screen = Screen::Course(1);
                        self.tab = Tab::Classwork;
                        self.class_roles.insert(1, CourseRole::Teacher);
                        if let Some(item) = self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(1))
                            .and_then(|items| items.iter().find(|item| item.kind == CourseworkKind::Assignment))
                        {
                            let mut draft = item.clone();
                            draft.title = "Poetry workshop".into();
                            draft.id = "draft-poetry".into();
                            let mut scheduled = draft.clone();
                            scheduled.title = "Comparative essay".into();
                            scheduled.id = "scheduled-essay".into();
                            self.teacher_drafts.insert(1, vec![
                                desktop_auth::TeacherDraft { item: draft, publication: desktop_auth::DraftPublication::Draft },
                                desktop_auth::TeacherDraft { item: scheduled, publication: desktop_auth::DraftPublication::Scheduled(Some(1_800_000_000_000)) },
                            ]);
                        }
                        return schedule_capture(Shot::TeacherDrafts);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::TeacherDrafts => {
                        let mut editor = AssignmentEditor::new(1, None);
                        editor.title = "Poetry comparison".into();
                        editor.instructions = text_editor::Content::with_text("Compare how two poets present a shared theme.");
                        editor.settings.due_date = "2027-01-26".into();
                        editor.settings.due_time = "16:00".into();
                        self.assignment_editor = Some(editor);
                        return schedule_capture(Shot::AssignmentEditor);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::AssignmentEditor => {
                        self.assignment_editor = None;
                        self.assignment_publication = self.teacher_drafts.get(&1).and_then(|drafts| drafts.iter()
                            .find(|draft| draft.publication == desktop_auth::DraftPublication::Draft))
                            .map(|draft| AssignmentPublicationPrompt { draft: draft.clone(), schedule: Some("2027-01-19 09:00".into()) });
                        return schedule_capture(Shot::AssignmentSchedule);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::AssignmentSchedule => {
                        self.assignment_publication = None;
                        self.screen = Screen::Item(1);
                        self.item_id = Some("42".into());
                        let row = |student_id, name: &str, status, released_grade, draft_grade| desktop_auth::TeacherSubmission {
                            student_id, submission_id: student_id + 100,
                            remote_state: Some(match status { SubmissionStatus::Returned => 3, SubmissionStatus::TurnedIn => 2, _ => 4 }),
                            name: Some(name.into()), status, released_grade, draft_grade, attachments: Vec::new(), answer: None,
                        };
                        self.teacher_work = Some(TeacherWork {
                            course: 1, item: 42, filter: StudentWorkFilter::All, selected: Some(12),
                            grade: "16".into(), confirming_return: true,
                            bulk_selected: BTreeSet::new(), bulk_confirming: false,
                            students: vec![
                                row(11, "Alex Bell", SubmissionStatus::Returned, Some(18.0), None),
                                row(12, "Jamie Lane", SubmissionStatus::TurnedIn, None, Some(16.0)),
                                row(13, "Morgan Vale", SubmissionStatus::Assigned, None, None),
                                row(14, "Sam Ellis", SubmissionStatus::Missing, None, None),
                            ],
                        });
                        if let Some(item) = self.snapshot.as_mut().and_then(|snapshot| snapshot.coursework.get_mut(&1))
                            .and_then(|items| items.iter_mut().find(|item| item.id == "42"))
                        {
                            item.maximum_points = Some(20.0);
                        }
                        return schedule_capture(Shot::TeacherStudentWork);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::TeacherStudentWork => {
                        self.screen = Screen::Course(1);
                        self.tab = Tab::Grades;
                        if let Some(items) = self.snapshot.as_mut().and_then(|snapshot| snapshot.coursework.get_mut(&1))
                            && let Some(template) = items.iter().find(|item| item.id == "42").cloned()
                        {
                            for (id, title) in [("43", "Poetry workshop"), ("44", "Comparative essay")] {
                                let mut item = template.clone();
                                item.id = id.into();
                                item.title = title.into();
                                items.push(item);
                            }
                        }
                        if let Some(work) = self.teacher_work.take() {
                            let students = work.students.iter().map(|row| CourseMember {
                                id: row.student_id.to_string(), display_name: row.name.clone().unwrap_or_default(), role: CourseRole::Student,
                            }).collect();
                            let cells = work.students.into_iter().flat_map(|row| {
                                let mark = desktop_auth::TeacherGradebookCell {
                                    submission_id: row.submission_id, remote_state: row.remote_state,
                                    status: row.status, released_grade: row.released_grade, draft_grade: row.draft_grade,
                                };
                                [((row.student_id, 42), mark),
                                    ((row.student_id, 43), desktop_auth::TeacherGradebookCell { status: SubmissionStatus::Assigned,
                                        submission_id: row.submission_id + 100, remote_state: Some(4), released_grade: None, draft_grade: None }),
                                    ((row.student_id, 44), desktop_auth::TeacherGradebookCell { status: SubmissionStatus::Returned,
                                        submission_id: row.submission_id + 200, remote_state: Some(3), released_grade: Some(20.0), draft_grade: None })]
                            }).collect();
                            self.gradebooks.insert(1, desktop_auth::TeacherGradebook { students, cells });
                        }
                        return schedule_capture(Shot::TeacherGradebook);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::TeacherGradebook => {
                        self.gradebook_edit_mode = true;
                        self.gradebook_edit = self.gradebooks.get(&1).and_then(|grid| grid.cells.get(&(12, 42)))
                            .map(|cell| GradebookEdit { course: 1, item: 42, expected: cell.grading_row(12), input: "17.5".into() });
                        return schedule_capture(Shot::TeacherGradebookEdit);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::TeacherGradebookEdit => {
                        self.gradebook_edit = None;
                        self.screen = Screen::Item(1);
                        self.item_id = Some("42".into());
                        let students = self.gradebooks.get(&1).map(|grid| grid.students.iter().filter_map(|student| {
                            let id = student.id.parse().ok()?;
                            let mut row = grid.cells.get(&(id, 42))?.grading_row(id);
                            row.name = Some(student.display_name.clone());
                            if id == 14 {
                                row.status = SubmissionStatus::TurnedIn;
                                row.remote_state = Some(2);
                            }
                            Some(row)
                        }).collect()).unwrap_or_default();
                        self.teacher_work = Some(TeacherWork { course: 1, item: 42, students, filter: StudentWorkFilter::All,
                            selected: None, grade: String::new(), confirming_return: false,
                            bulk_selected: BTreeSet::from([12, 14]), bulk_confirming: true });
                        return schedule_capture(Shot::BulkReturn);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::BulkReturn => {
                        if let Some(work) = &mut self.teacher_work {
                            work.bulk_selected.clear();
                            work.bulk_confirming = false;
                            work.select(Some(12));
                        }
                        if let Some(key) = self.selected_teacher_feedback_key() {
                            self.teacher_feedback.insert(key, TeacherFeedbackThread {
                                comments: Some(vec![Comment {
                                    id: "57".into(), author_id: None, author: "Jamie Lane".into(),
                                    body: "I added the quotations from both poems.".into(), created_label: None,
                                }]),
                                draft: "Thanks. Please explain how your second quotation supports your comparison.".into(),
                                unconfirmed: false,
                            });
                        }
                        return schedule_capture(Shot::TeacherFeedback);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::TeacherFeedback => {
                        self.teacher_work = None;
                        self.teacher_feedback.clear();
                        if let Some(post) = self.snapshot.as_mut().and_then(|snapshot| snapshot.coursework.get_mut(&1))
                            .and_then(|items| items.iter_mut().find(|item| item.kind == CourseworkKind::Announcement))
                        {
                            post.id = "77".into();
                            post.author_id = Some(900);
                            post.comments = Some(vec![Comment {
                                id: "81".into(), author_id: Some(901), author: "Sam Ellis".into(),
                                body: "Is the workshop still happening on Friday?".into(), created_label: None,
                            }]);
                            self.item_id = Some("77".into());
                            self.confirming_comment_delete = Some(CommentTarget { course: 1, item: 77, id: 81, audience: CommentAudience::Class });
                        }
                        return schedule_capture(Shot::TeacherModerationComment);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::TeacherModerationComment => {
                        self.screen = Screen::Course(1);
                        self.tab = Tab::Stream;
                        self.confirming_comment_delete = None;
                        self.confirming_stream_delete = Some((1, 77));
                        return schedule_capture(Shot::TeacherModerationPost);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::TeacherModerationPost => {
                        self.tab = Tab::ClassSettings;
                        self.confirming_stream_delete = None;
                        let original = desktop_auth::TeacherClassSettings {
                            details: desktop_auth::ClassDetails { title: "English Literature".into(),
                                subtitle: "Year 12 · Group A".into(), description: "Read and discuss poetry, prose, and drama.".into(),
                                room: "B14".into() },
                            stream: desktop_auth::CourseStreamSettings { posting: Some(desktop_auth::StreamPostingPolicy::PostAndComment),
                                classwork: Some(StreamClassworkDisplay::Condensed) },
                        };
                        let mut editor = ClassStreamSettingsEditor::new(1, original);
                        editor.draft.details.room = "B16".into();
                        editor.draft.stream.posting = Some(desktop_auth::StreamPostingPolicy::CommentOnly);
                        editor.draft.stream.classwork = Some(StreamClassworkDisplay::Expanded);
                        self.class_stream_settings = Some(editor);
                        return schedule_capture(Shot::ClassStreamSettings);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::ClassStreamSettings => {
                        self.tab = Tab::People;
                        if let Some(member) = self.snapshot.as_mut().and_then(|snapshot| snapshot.roster.get_mut(&1))
                            .and_then(|members| members.iter_mut().find(|member| member.role == CourseRole::Student))
                        {
                            member.id = "81".into();
                            self.student_muting.insert(1, BTreeMap::from([(81, false)]));
                            self.pending_student_mute = Some((1, 81, false));
                        }
                        return schedule_capture(Shot::StudentMuting);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::StudentMuting => {
                        self.tab = Tab::Classwork;
                        self.topic_manager = true;
                        let topic = desktop_data::Topic { id: "demo-poetry-unit".into(), title: "Poetry".into() };
                        self.teacher_topics.insert(1, vec![topic.clone(), desktop_data::Topic {
                            id: "demo-reading".into(), title: "Independent reading".into(),
                        }]);
                        self.topic_editor = Some((1, desktop_auth::TeacherTopicAction::Delete(topic)));
                        return schedule_capture(Shot::TopicManagement);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::TopicManagement => {
                        self.topic_manager = false;
                        self.topic_editor = None;
                        if let Some(item) = self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(1))
                            .and_then(|items| items.iter().find(|item| item.kind == CourseworkKind::Assignment))
                        {
                            self.item_topic_editor = Some(ItemTopicEditor {
                                target: desktop_auth::TopicItemTarget { course: 1, item: item.id.parse().unwrap_or(42),
                                    kind: item.kind, draft: false, original: item.topic_id.clone() },
                                title: item.title.clone(), selected: Some("demo-reading".into()),
                            });
                        }
                        return schedule_capture(Shot::ItemTopic);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::ItemTopic => {}
                }
                return iced::exit();
            }
            Message::Demo if !self.has_pending_request() => {
                self.install_snapshot(Snapshot::demo());
                self.reset_artwork();
                self.session = None;
                self.opening_drive_file = None;
                self.class_creation_generation = self.class_creation_generation.wrapping_add(1);
                self.class_creation_allowed = false;
                self.new_class = desktop_auth::NewClass::default();
                self.demo = true;
                self.screen = Screen::Home;
                self.search.clear();
                self.error = None;
            }
            Message::Disconnect if !self.has_pending_request() && self.session.is_some() => {
                self.discard_submission_capture();
                self.cancel_assignment_batch();
                let account = self.session.as_ref().unwrap().account.clone();
                self.busy = true;
                return Task::perform(desktop_auth::forget_session(account), Message::SessionForgotten);
            }
            Message::SessionForgotten(Err(error)) => {
                self.busy = false;
                self.queued_page_load = true;
                self.error = Some(error);
            }
            Message::SessionForgotten(Ok(())) | Message::Disconnect => {
                self.busy = false;
                self.reset_artwork();
                self.session = None;
                self.snapshot = None;
                self.classroom_link_input = None;
                self.show_hidden_classes = false;
                self.open_course_menu = None;
                self.move_course_menu = None;
                self.course_drag = None;
                self.confirm_hide_course = None;
                self.confirm_class_archive = None;
                self.reset_class_folders();
                self.teachers_generation = self.teachers_generation.wrapping_add(1);
                self.class_teachers.clear();
                self.reset_notifications();
                self.reset_stream_access();
                self.confirming_stream_delete = None;
                self.open_stream_composer = None;
                self.stream_drafts.clear();
                self.drive_picker = None;
                self.reset_assignment_reads();
                self.submissions_checked.clear();
                self.page_errors.clear();
                self.class_code.clear();
                self.invalidate_invitation_read();
                self.invitations = None;
                self.opening_joined_course = None;
                self.confirming_decline = None;
                self.confirming_unenroll = None;
                self.comment_drafts.clear();
                self.comment_mentions_generation = self.comment_mentions_generation.wrapping_add(1);
                self.comment_mentions = None;
                self.short_answer_drafts.clear();
                self.short_answer_revisions.clear();
                self.choice_drafts.clear();
                self.confirming_question_answer = None;
                self.comment_unconfirmed.clear();
                self.private_feedback.clear();
                self.rubrics.clear();
                self.overall_grades.clear();
                self.rubric_open = false;
                self.open_criteria.clear();
                self.private_drafts.clear();
                self.confirming_comment_delete = None;
                self.private_unconfirmed.clear();
                self.pending_remove_material = None;
                self.pending_submission = None;
                self.item_id = None;
                self.attachment_link.clear();
                self.submission_file = None;
                self.demo = false;
                self.screen = Screen::Home;
                self.search.clear();
                self.email.clear();
                self.error = None;
                self.notice = None;
            }
            _ => {}
        }
        if !self.screen.uses_assignment_overview() {
            self.cancel_assignment_batch();
        }
        self.dequeue_page_load().map_or_else(Task::none, |request| self.update(request))
    }

    fn assignment_courses(&self) -> BTreeSet<i64> {
        self.snapshot.iter().flat_map(|snapshot| &snapshot.courses)
            .filter(|course| home_course(course) && !self.assignments_completed.contains(&course.id))
            .map(|course| course.id).collect()
    }

    fn apply_assignment_update(&mut self, result: desktop_auth::AssignmentRefresh) {
        let (Some(batch), Some(snapshot)) = (&mut self.assignment_batch, &mut self.snapshot) else {
            return;
        };
        let id = result.course_id;
        if !batch.remaining.remove(&id) {
            return;
        }

        if let Some(mut contents) = result.contents {
            if let Some(previous) = snapshot.coursework_for(id) {
                for item in &mut contents.items {
                    if let Some(old) = previous.iter().find(|old| old.id == item.id) {
                        item.comments = old.comments.clone();
                        if result.submissions.is_none() {
                            item.submission = old.submission.clone();
                        }
                    }
                }
            }
            let needs_status = contents.items.iter().any(has_submission);
            if let Some(topics) = contents.topics {
                snapshot.topics.insert(id, topics);
            }
            snapshot.coursework.insert(id, contents.items);
            if let Some(submissions) = result.submissions {
                snapshot.set_submissions(id, submissions);
                self.submissions_checked.insert(id);
                self.assignments_completed.insert(id);
            } else if needs_status {
                batch.failed_status += 1;
            } else {
                self.assignments_completed.insert(id);
            }
        } else {
            batch.failed_classwork += 1;
        }

        if batch.remaining.is_empty() {
            if let Some(error) = batch.error() {
                self.page_errors.insert(PageRead::Assignments, error);
            }
            self.assignments_loaded = true;
            self.assignment_batch = None;
            if self.screen == Screen::Home && std::env::var_os("CLASSROOM_READ_ONLY_DIAGNOSTICS").is_some() {
                println!("Home overview ready; classes={} due_soon={}",
                    self.assignments_completed.len(), snapshot.upcoming_work(&Local::now(), None, 5).len());
                let _ = io::stdout().flush();
            }
        }
    }

    fn dequeue_page_load(&mut self) -> Option<Message> {
        if self.queued_page_load && !self.has_pending_request() {
            self.queued_page_load = false;
            return self.visible_page_request();
        }
        None
    }

    fn can_compose_comment(&self, course: i64, item: i64) -> bool {
        if self.session.is_none() || self.verify_live_course_count
            || self.screen != Screen::Item(course)
            || self.item_id.as_deref().and_then(|id| id.parse::<i64>().ok()) != Some(item)
        {
            return false;
        }
        self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.source == SnapshotSource::Live && snapshot.courses.iter()
                .any(|entry| entry.id == course && entry.state == CourseState::Active)
        })
    }

    fn can_compose_class_comment(&self, course: i64, item: i64) -> bool {
        self.can_compose_comment(course, item)
            && self.stream_access.get(&course).is_some_and(|access| access.can_comment())
    }

    fn may_delete_comment(&self, target: CommentTarget) -> bool {
        self.can_compose_comment(target.course, target.item) && self.displayed_comment(target)
            .is_some_and(|comment| self.may_delete_visible_comment(target.course, target.audience, comment))
    }

    fn displayed_comment(&self, target: CommentTarget) -> Option<&Comment> {
        let comments = match target.audience {
            CommentAudience::Private => self.private_feedback.get(&(target.course, target.item)),
            CommentAudience::Class => self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(target.course))
                .and_then(|items| items.iter().find(|item| item.id.parse::<i64>().ok() == Some(target.item)))
                .and_then(|item| item.comments.as_ref()),
        }?;
        comments.iter().find(|comment| comment.id.parse::<i64>().ok() == Some(target.id))
    }

    fn may_delete_visible_comment(&self, course: i64, audience: CommentAudience, comment: &Comment) -> bool {
        self.owns_comment(comment) || (audience == CommentAudience::Class && self.may_moderate_author(course, comment.author_id))
    }

    fn may_moderate_author(&self, course: i64, author: Option<i64>) -> bool {
        self.class_roles.get(&course) == Some(&CourseRole::Teacher) && author.is_some_and(|id| id > 0)
    }

    fn owns_comment(&self, comment: &Comment) -> bool {
        self.stream_viewer_id.is_some() && self.stream_viewer_id == comment.author_id
    }

    fn current_question(&self, course: i64, item: i64) -> Option<&Coursework> {
        if !self.can_compose_comment(course, item)
            || self.page_errors.contains_key(&PageRead::Detail(DetailRead::Submissions(course)))
        {
            return None;
        }
        self.snapshot.as_ref()?.coursework_for(course)?
            .iter().find(|work| work.id == item.to_string())
    }

    fn editable_question(&self, course: i64, item: i64) -> Option<&QuestionDetails> {
        let work = self.current_question(course, item)?;
        let revision = self.short_answer_revisions.get(&(course, item));
        if desktop_auth::question_submission_is_current(work, revision) {
            work.question.as_ref()
        } else {
            None
        }
    }

    fn question_draft(&self, key: (i64, i64), details: &QuestionDetails) -> Option<QuestionAnswer> {
        let draft = match details {
            QuestionDetails::ShortAnswer => {
                let text = self.short_answer_drafts.get(&key)?.text();
                if self.short_answer_revisions.get(&key).is_some_and(|revision| revision.answer == text) {
                    return None;
                }
                QuestionAnswer::ShortAnswer(text)
            }
            QuestionDetails::MultipleChoice(_) => QuestionAnswer::MultipleChoice(self.choice_drafts.get(&key)?.clone()),
        };
        details.accepts(&draft).then_some(draft)
    }

    fn retry_visible_reads_on_entry(&mut self) {
        let item = self.item_id.as_ref().and_then(|id| id.parse::<i64>().ok());
        self.page_errors.retain(|read, _| {
            let visible = match (*read, self.screen) {
                (PageRead::Assignments, screen) => screen.uses_assignment_overview(),
                (PageRead::Invitations, Screen::Home | Screen::Join) => true,
                (PageRead::ClassNotifications | PageRead::EmailNotifications, Screen::Settings) => true,
                (PageRead::Coursework(course), Screen::Course(id)) => id == course && self.tab != Tab::People,
                (PageRead::StreamAccess(course), Screen::Course(id)) => id == course && self.tab == Tab::Stream,
                (PageRead::StreamAccess(course), Screen::Item(id)) => id == course,
                (PageRead::Coursework(course), Screen::Item(id)) => id == course,
                (PageRead::Detail(DetailRead::Roster(course)), Screen::Course(id)) => id == course && self.tab == Tab::People,
                (PageRead::Detail(DetailRead::StudentMuting(course)), Screen::Course(id)) => id == course && self.tab == Tab::People,
                (PageRead::Detail(DetailRead::TeacherTopics(course)), Screen::Course(id)) => id == course && self.tab == Tab::Classwork,
                (PageRead::Detail(DetailRead::Submissions(course)), Screen::Course(id)) =>
                    id == course && self.tab != Tab::People,
                (PageRead::Detail(DetailRead::OverallGrade(course)), Screen::Course(id)) =>
                    id == course && self.tab == Tab::YourWork,
                (PageRead::Detail(DetailRead::TeacherDrafts(course)), Screen::Course(id)) =>
                    id == course && matches!(self.tab, Tab::Classwork | Tab::Stream),
                (PageRead::Detail(DetailRead::TeacherGradebook(course)), Screen::Course(id)) =>
                    id == course && self.tab == Tab::Grades,
                (PageRead::Detail(DetailRead::ClassStreamSettings(course)), Screen::Course(id)) =>
                    id == course && self.tab == Tab::ClassSettings,
                (PageRead::Detail(DetailRead::Submissions(course)), Screen::Item(id)) => id == course,
                (PageRead::Detail(DetailRead::Comments(course, key) | DetailRead::PrivateFeedback(course, key)
                    | DetailRead::Rubric(course, key) | DetailRead::TeacherSubmissions(course, key)), Screen::Item(id)) =>
                    id == course && item == Some(key),
                (PageRead::Detail(DetailRead::TeacherFeedback(key)), Screen::Item(id)) =>
                    id == key.course && item == Some(key.item),
                _ => false,
            };
            !visible
        });
        if self.screen.uses_assignment_overview() && !self.assignment_courses().is_empty() {
            self.assignments_loaded = false;
        }
    }

    fn visible_page_request(&self) -> Option<Message> {
        if !self.can_read_pages() {
            return None;
        }
        self.next_page_read(self.snapshot.as_ref()?).map(PageRead::message)
    }

    fn next_page_read(&self, snapshot: &Snapshot) -> Option<PageRead> {
        let read = match self.screen {
            Screen::Settings if self.class_notifications.is_none() && !self.loading_class_notifications
                && !self.page_errors.contains_key(&PageRead::ClassNotifications) =>
                PageRead::ClassNotifications,
            Screen::Settings if self.email_notifications.is_none() && !self.loading_email_notifications =>
                PageRead::EmailNotifications,
            Screen::Home | Screen::Join if self.invitations.is_none() && !self.loading_invitations
                && !self.page_errors.contains_key(&PageRead::Invitations) => PageRead::Invitations,
            Screen::Home | Screen::Calendar | Screen::ToDo if !self.assignments_loaded && self.assignment_batch.is_none() => {
                PageRead::Assignments
            }
            Screen::Course(course) if self.tab == Tab::People && snapshot.roster_for(course).is_none() => {
                PageRead::Detail(DetailRead::Roster(course))
            }
            Screen::Course(course) if self.tab == Tab::People && self.class_roles.get(&course) == Some(&CourseRole::Teacher)
                && snapshot.courses.iter().any(|entry| entry.id == course && entry.state == CourseState::Active)
                && !self.student_muting.contains_key(&course) =>
            {
                PageRead::Detail(DetailRead::StudentMuting(course))
            }
            Screen::Course(course) if self.tab == Tab::ClassSettings => {
                if self.class_roles.get(&course) != Some(&CourseRole::Teacher)
                    || self.class_stream_settings.as_ref().is_some_and(|editor| editor.course == course)
                {
                    return None;
                }
                PageRead::Detail(DetailRead::ClassStreamSettings(course))
            }
            Screen::Course(course) if self.tab != Tab::People => {
                let coursework = PageRead::Coursework(course);
                let submissions = PageRead::Detail(DetailRead::Submissions(course));
                let overall = PageRead::Detail(DetailRead::OverallGrade(course));
                let items = snapshot.coursework_for(course);
                let teacher = self.class_roles.get(&course) == Some(&CourseRole::Teacher);
                [
                    (items.is_none(), coursework),
                    (teacher && self.tab == Tab::Classwork && !self.teacher_topics.contains_key(&course)
                        && snapshot.courses.iter().any(|entry| entry.id == course && entry.state == CourseState::Active),
                        PageRead::Detail(DetailRead::TeacherTopics(course))),
                    (self.tab == Tab::Stream && snapshot.courses.iter().any(|entry| entry.id == course && entry.state == CourseState::Active)
                        && !self.stream_access.contains_key(&course) && !self.loading_stream_access.contains(&course), PageRead::StreamAccess(course)),
                    (teacher && matches!(self.tab, Tab::Classwork | Tab::Stream) && !self.teacher_drafts.contains_key(&course),
                        PageRead::Detail(DetailRead::TeacherDrafts(course))),
                    (teacher && self.tab == Tab::Grades && !self.gradebooks.contains_key(&course),
                        PageRead::Detail(DetailRead::TeacherGradebook(course))),
                    (!teacher && matches!(self.tab, Tab::Classwork | Tab::YourWork) && !self.submissions_checked.contains(&course)
                        && items.is_some_and(|items| items.iter().any(has_submission)), submissions),
                    (self.tab == Tab::Stream && !self.submissions_checked.contains(&course)
                        && !snapshot.upcoming_work(&Local::now(), Some(course), 1).is_empty(), submissions),
                    (self.tab == Tab::YourWork && !self.overall_grades.contains_key(&course), overall),
                ].into_iter().find_map(|(missing, read)| {
                    (missing && !self.page_errors.contains_key(&read)).then_some(read)
                })?
            }
            Screen::Item(course) if snapshot.coursework_for(course).is_none() => PageRead::Coursework(course),
            Screen::Item(course) => {
                let id = self.item_id.as_ref()?;
                let item = snapshot.coursework_for(course)?.iter().find(|item| &item.id == id)?;
                let item_id = id.parse().ok()?;
                let comments = PageRead::Detail(DetailRead::Comments(course, item_id));
                let submissions = PageRead::Detail(DetailRead::Submissions(course));
                let feedback = PageRead::Detail(DetailRead::PrivateFeedback(course, item_id));
                let rubric = PageRead::Detail(DetailRead::Rubric(course, item_id));
                let teacher = self.class_roles.get(&course) == Some(&CourseRole::Teacher);
                let teacher_work = PageRead::Detail(DetailRead::TeacherSubmissions(course, item_id));
                if let Some(key) = self.selected_teacher_feedback_key() {
                    let thread = PageRead::Detail(DetailRead::TeacherFeedback(key));
                    if self.teacher_feedback.get(&key).is_none_or(|data| data.comments.is_none())
                        && !self.page_errors.contains_key(&thread)
                    {
                        return Some(thread);
                    }
                }
                [
                    (teacher && has_submission(item) && !self.teacher_work.as_ref()
                        .is_some_and(|work| work.course == course && work.item == item_id), teacher_work),
                    (snapshot.courses.iter().any(|entry| entry.id == course && entry.state == CourseState::Active)
                        && !self.stream_access.contains_key(&course) && !self.loading_stream_access.contains(&course), PageRead::StreamAccess(course)),
                    (item.comments.is_none() || self.comment_unconfirmed.contains(&(course, item_id)), comments),
                    (!teacher && has_submission(item) && !self.submissions_checked.contains(&course), submissions),
                    (item.kind == CourseworkKind::Assignment && !self.rubrics.contains_key(&(course, item_id)), rubric),
                    (!teacher && snapshot.source == SnapshotSource::Live
                        && item.submission.as_ref().is_some_and(|sub| sub.remote_id.is_some())
                        && (!self.private_feedback.contains_key(&(course, item_id))
                            || self.private_unconfirmed.contains(&(course, item_id))), feedback),
                ].into_iter().find_map(|(missing, read)| {
                    (missing && !self.page_errors.contains_key(&read)).then_some(read)
                })?
            }
            _ => return None,
        };
        (!self.page_errors.contains_key(&read)).then_some(read)
    }

    fn reset_class_folders(&mut self) {
        self.class_folders_generation = self.class_folders_generation.wrapping_add(1);
        self.class_folders.clear();
        self.class_roles.clear();
        self.teacher_drafts.clear();
        self.open_teacher_draft = None;
        self.assignment_editor = None;
        self.assignment_publication = None;
        self.teacher_work = None;
        self.teacher_feedback.clear();
        self.gradebooks.clear();
        self.class_stream_settings = None;
        self.student_muting.clear();
        self.pending_student_mute = None;
        self.teacher_topics.clear();
        self.topic_editor = None;
        self.item_topic_editor = None;
        self.topic_manager = false;
        self.gradebook_student = None;
        self.gradebook_edit_mode = false;
        self.gradebook_edit = None;
        self.page_errors.retain(|read, _| !matches!(read, PageRead::Detail(DetailRead::TeacherDrafts(_) | DetailRead::TeacherGradebook(_))));
        self.class_folders_failed = false;
    }

    fn fetch_class_folders(&mut self) -> Task<Message> {
        self.reset_class_folders();
        if !self.can_read_pages() {
            return Task::none();
        }
        let (Some(snapshot), Some(session)) = (&self.snapshot, self.session.clone()) else {
            return Task::none();
        };
        let courses = snapshot.courses.iter()
            .filter(|course| course.state != CourseState::Other)
            .map(|course| course.id).collect();
        let generation = self.class_folders_generation;
        Task::perform(async move { desktop_auth::load_own_class_details(&session, courses).await },
            move |result| Message::ClassFoldersLoaded(generation, result))
    }

    fn fetch_class_creation_permission(&mut self) -> Task<Message> {
        self.class_creation_generation = self.class_creation_generation.wrapping_add(1);
        self.class_creation_allowed = false;
        let generation = self.class_creation_generation;
        if let Some(session) = self.session.clone().filter(|_| self.can_read_pages()) {
            Task::perform(async move { desktop_auth::can_create_classes(&session).await },
                move |result| Message::ClassCreationPermission(generation, result))
        } else {
            Task::none()
        }
    }

    fn fetch_class_teachers(&mut self) -> Task<Message> {
        if !self.can_read_pages() {
            return Task::none();
        }
        let (Some(snapshot), Some(session)) = (&self.snapshot, self.session.clone()) else {
            return Task::none();
        };
        let courses: Vec<_> = snapshot.courses.iter()
            .filter(|course| !course.owner_ids.is_empty() && course.state != CourseState::Other)
            .map(|course| (course.id, course.owner_ids.clone())).collect();
        self.teachers_generation = self.teachers_generation.wrapping_add(1);
        let generation = self.teachers_generation;
        Task::perform(async move { desktop_auth::load_class_teacher_names(&session, &courses).await },
            move |result| Message::ClassTeachersLoaded(generation, result))
    }

    fn class_folder_url(&self, id: i64) -> Option<&str> {
        let snapshot = self.snapshot.as_ref()?;
        if snapshot.source != SnapshotSource::Live && self.screenshot_directory.is_none() {
            return None;
        }
        self.class_folders.get(&id).map(String::as_str)
    }

    fn reset_artwork(&mut self) {
        self.artwork.clear();
        self.artwork_generation = self.artwork_generation.wrapping_add(1);
    }

    fn fetch_artwork(&self) -> Task<Message> {
        if self.verify_live_course_count || self.screenshot_directory.is_some() || self.session.is_none() {
            return Task::none();
        }
        let Some(snapshot) = &self.snapshot else { return Task::none() };
        let ordered = snapshot.courses.iter().filter(|course| home_course(course))
            .chain(snapshot.courses.iter().filter(|course| !home_course(course)));
        let urls: Vec<_> = ordered
            .filter_map(|course| course.theme.as_ref()?.artwork_url.as_ref())
            .filter(|url| !self.artwork.contains_key(url.as_str()))
            .cloned()
            .collect();
        if urls.is_empty() {
            return Task::none();
        }
        let generation = self.artwork_generation;
        Task::run(desktop_auth::artwork_updates(urls), move |(url, image)| {
            Message::ArtworkLoaded(generation, url, image)
        })
    }

    fn artwork_for<'a>(&'a self, course: &Course) -> Option<&'a image::Handle> {
        self.artwork.get(course.theme.as_ref()?.artwork_url.as_ref()?)
    }

    fn save_offline(&mut self) {
        let (Some(session), Some(snapshot)) = (&self.session, &self.snapshot) else { return };
        let result = cache_path().ok_or_else(|| "Offline storage isn't available.".to_string())
            .and_then(|path| desktop_data::save_cache(path, &session.account, snapshot)
                .map_err(|_| "The offline copy couldn't be saved.".into()));
        self.finish_cache_action(result, "Offline copy saved on this Mac.");
    }

    fn open_offline(&mut self) {
        let Some(session) = &self.session else { return };
        let result = cache_path().ok_or_else(|| "Offline storage isn't available.".to_string())
            .and_then(|path| desktop_data::load_cache(path, &session.account)
                .map_err(|_| "The offline copy couldn't be opened.".into()));
        match result {
            Ok(Some(snapshot)) => {
                self.install_snapshot(snapshot);
                self.error = None;
                self.notice = None;
                self.screen = Screen::Home;
                self.demo = false;
            }
            Ok(None) => self.error = Some("There is no saved copy for that email.".into()),
            Err(error) => self.error = Some(error),
        }
    }

    fn delete_offline(&mut self) {
        let Some(session) = &self.session else { return };
        let result = cache_path().ok_or_else(|| "Offline storage isn't available.".to_string())
            .and_then(|path| desktop_data::clear_cache(path, &session.account)
                .map_err(|_| "The offline copy couldn't be removed.".into()));
        if result.is_ok() && self.snapshot.as_ref().is_some_and(|data| data.source == SnapshotSource::Cache) {
            self.install_snapshot(session.snapshot.clone());
        }
        self.finish_cache_action(result, "Offline copy removed from this Mac.");
    }

    fn finish_cache_action(&mut self, result: Result<(), String>, success: &str) {
        self.error = result.as_ref().err().cloned();
        self.notice = result.is_ok().then(|| success.to_owned());
    }

    fn install_snapshot(&mut self, snapshot: Snapshot) {
        self.teachers_generation = self.teachers_generation.wrapping_add(1);
        self.class_teachers.clear();
        self.reset_notifications();
        self.reset_stream_access();
        if self.snapshot.as_ref().is_some_and(|old| old.source != snapshot.source) {
            self.reset_assignment_reads();
            self.submissions_checked.clear();
        }
        if let Screen::Course(id) | Screen::Item(id) = self.screen
            && !snapshot.courses.iter().any(|course| course.id == id)
        {
            self.discard_submission_capture();
            self.screen = Screen::Home;
            self.tab = Tab::Stream;
        }
        for filter in [&mut self.calendar_class, &mut self.todo_class] {
            if filter.is_some_and(|id| !snapshot.courses.iter()
                .any(|course| course.id == id && course.state.shows_on_home()))
            {
                *filter = None;
            }
        }
        self.snapshot = Some(snapshot);
    }

    fn view(&self) -> Element<'_, Message> {
        let sidebar: Element<'_, Message> = if self.sidebar {
            self.navigation()
        } else {
            space().width(0).into()
        };
        let content = match self.screen {
            Screen::Home => self.home(false),
            Screen::Join => self.join_class_view(),
            Screen::CreateClass => self.create_class_view(),
            Screen::Archived => self.home(true),
            Screen::Course(id) => self.course(id),
            Screen::Item(id) => self.item_detail(id),
            Screen::ToDo => self.todo(),
            Screen::Calendar => self.calendar(),
            Screen::Settings => self.settings(),
        };
        let banner_message = self.error.as_ref().or(self.notice.as_ref());
        let error_banner: Element<'_, Message> = if let Some(message) = banner_message {
            let is_error = self.error.is_some();
            let foreground = if is_error { color!(0x9b2722) } else { color!(0x137333) };
            let background = if is_error { color!(0xfce8e6) } else { color!(0xe6f4ea) };
            container(
                row![
                    icon(Icon::Info, foreground, 19),
                    text(message).size(14).color(foreground).width(Fill)
                ]
                .spacing(11)
                .align_y(Alignment::Center),
            )
            .style(move |_| surface(background, 9.0))
            .padding([11, 14])
            .width(Fill)
            .into()
        } else {
            space().height(0).into()
        };
        let mut main = column![
            container(error_banner).padding(if banner_message.is_some() {
                Padding { top: 12.0, right: 28.0, bottom: 0.0, left: 28.0 }
            } else {
                Padding::ZERO
            }),
        ];
        if let Some(value) = &self.classroom_link_input {
            main = main.push(container(self.classroom_link_form(value)).padding([12, 26]).width(Fill));
        }
        let main = main.push(content).width(Fill).height(Fill);
        container(column![
            self.header(),
            rule::horizontal(1).style(divider_style),
            row![sidebar, main].width(Fill).height(Fill),
        ])
        .width(Fill)
        .height(Fill)
        .style(|_| surface(WHITE, 0.0))
        .into()
    }

    fn classroom_link_form<'a>(&'a self, value: &'a str) -> Element<'a, Message> {
        let ready = !self.has_pending_request() && !value.trim().is_empty();
        let input = text_input("Paste a classroom.google.com link", value).padding(9).width(Fill)
            .on_input(Message::EditClassroomLink).on_submit_maybe(ready.then_some(Message::OpenClassroomLink));
        container(column![
            text("Open Classroom link").size(17).font(medium()).color(FG),
            row![
                input,
                button("Cancel").style(quiet_style).padding([7, 9]).on_press(Message::CloseClassroomLink),
                button("Open").style(primary_style).padding([8, 12])
                    .on_press_maybe(ready.then_some(Message::OpenClassroomLink)),
            ].spacing(8).align_y(Alignment::Center),
        ].spacing(8)).padding(12).style(|_| outline_surface(8.0)).into()
    }

    fn open_classroom_link(&mut self, target: ClassroomLink) -> Task<Message> {
        let course = target.course();
        let Some(snapshot) = self.snapshot.as_ref().filter(|snapshot| snapshot.courses.iter().any(|entry| entry.id == course)) else {
            self.error = Some("That class is not in the current Classroom account or saved copy.".into());
            return Task::none();
        };
        let needs_reload = match target {
            ClassroomLink::Item { item, .. } => snapshot.coursework_for(course)
                .is_some_and(|items| !items.iter().any(|entry| entry.id.parse::<i64>().ok() == Some(item))),
            ClassroomLink::Topic { topic, .. } => snapshot.coursework_for(course)
                .is_some_and(|items| !items.iter().any(|entry| entry.topic_id.as_deref() == Some(&topic.to_string())))
                && snapshot.topics_for(course).is_none_or(|topics| !topics.iter().any(|entry| entry.id == topic.to_string())),
            _ => false,
        };
        self.classroom_link_input = None;
        self.error = None;
        self.notice = None;
        match target {
            ClassroomLink::Course { classwork, .. } => {
                let navigate = self.update(Message::Navigate(Screen::Course(course)));
                Task::batch([navigate, self.update(Message::SetTab(if classwork { Tab::Classwork } else { Tab::Stream }))])
            }
            ClassroomLink::Topic { topic, .. } => {
                let navigate = self.update(Message::Navigate(Screen::Course(course)));
                let tab = self.update(Message::SetTab(Tab::Classwork));
                let topic = self.update(Message::SelectTopic(Some(topic.to_string())));
                let refresh = if needs_reload && self.can_read_pages() {
                    self.update(Message::LoadCoursework(course))
                } else {
                    Task::none()
                };
                Task::batch([navigate, tab, topic, refresh])
            }
            ClassroomLink::Item { item, kind, .. } => {
                self.tab = if kind == ItemLinkKind::Announcement { Tab::Stream } else { Tab::Classwork };
                let open = self.update(Message::OpenItem(course, item.to_string()));
                self.previous_screen = Some(Screen::Course(course));
                let refresh = if needs_reload && self.can_read_pages() {
                    self.update(Message::LoadCoursework(course))
                } else {
                    Task::none()
                };
                Task::batch([open, refresh])
            }
        }
    }

    fn header(&self) -> Element<'_, Message> {
        let toggle = button(icon(Icon::Menu, MUTED, 21))
            .padding(12)
            .style(quiet_style)
            .on_press(Message::ToggleNavigation);
        let brand = button(
            row![classroom_mark(), text("Classroom").size(22).color(MUTED)]
                .spacing(9)
                .align_y(Alignment::Center),
        )
        .padding([7, 9])
        .style(quiet_style)
        .on_press(Message::Navigate(Screen::Home));
        let account: Element<'_, Message> = if let Some(session) = &self.session {
            let status: Element<'_, Message> = if session.needs_sign_in() {
                button(if self.busy { "Signing in…" } else { "Reconnect" })
                    .style(outline_style).padding([8, 12])
                    .on_press_maybe((!self.has_pending_request()).then_some(Message::Reconnect))
                    .into()
            } else {
                text(if self.snapshot.as_ref().is_some_and(|data| data.source == SnapshotSource::Cache) {
                    "Viewing offline copy"
                } else {
                    "Connected"
                }).size(13).color(color!(0x137333)).into()
            };
            let mut actions = row![
                status,
                button("Join class").style(quiet_style).padding([8, 12])
                    .on_press_maybe(self.can_change_membership().then_some(Message::Navigate(Screen::Join))),
            ];
            if self.can_create_class() {
                actions = actions.push(button("Create class").style(quiet_style).padding([8, 12])
                    .on_press_maybe((!self.has_pending_request()).then_some(Message::Navigate(Screen::CreateClass))));
            }
            actions.push(
                button("Sign out").style(quiet_style).padding([8, 12]).on_press_maybe(
                    (!self.has_pending_request()).then_some(Message::Disconnect)
                )
            ).push(avatar(first_initial(self.email.trim()), color!(0x1b6a54), 34.0, 15.0))
            .spacing(9)
            .align_y(Alignment::Center)
            .into()
        } else if self.demo {
            row![
                pill("Demo data", color!(0xfef7e0), color!(0x6d4e00)),
                button("Exit demo").style(quiet_style).padding([8, 12])
                    .on_press(Message::Disconnect),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into()
        } else if self.snapshot.is_some() {
            row![
                pill("Offline copy", SOFT, MUTED),
                button("Close").style(quiet_style).padding([8, 12]).on_press(Message::Disconnect),
            ].spacing(8).align_y(Alignment::Center).into()
        } else {
            pill("Signed out", SOFT, MUTED)
        };
        let links: Element<'_, Message> = if self.snapshot.is_some() && !self.verify_live_course_count {
            button("Open link…").style(quiet_style).padding([8, 10])
                .on_press(Message::ShowClassroomLink).into()
        } else {
            space().into()
        };
        container(
            row![toggle, brand, space().width(Fill), links, account]
                .padding([8, 18])
                .spacing(5)
                .align_y(Alignment::Center),
        )
        .width(Fill)
        .into()
    }

    fn navigation(&self) -> Element<'_, Message> {
        let mut course_entries = column![].spacing(3);
        if let Some(data) = &self.snapshot {
            if data.courses.iter().any(|course| home_course(course)) {
                course_entries = course_entries.push(
                    container(text("Enrolled").size(13).font(medium()).color(MUTED))
                        .padding(Padding {
                            top: 10.0, right: 12.0, bottom: 6.0, left: 17.0,
                        })
                );
            }
            for course in data.courses.iter().filter(|course| home_course(course)) {
                let destination = Screen::Course(course.id);
                course_entries = course_entries.push(
                    button(row![
                        avatar(first_initial(&course.title), course_color(course), 26.0, 12.0),
                        text(&course.title).size(14).color(
                            if self.is_selected_course(course.id) { ACTIVE_FG } else { FG }
                        ).width(Fill),
                    ].spacing(11).align_y(Alignment::Center))
                    .on_press(Message::Navigate(destination))
                    .style(move |_, status| nav_style(self.is_selected_course(course.id), status))
                    .padding([9, 13])
                    .width(Fill)
                );
            }
        }
        let nav = column![
            self.nav_button(Icon::Home, "Home", Screen::Home),
            self.nav_button(Icon::Calendar, "Calendar", Screen::Calendar),
            self.nav_button(Icon::Check, "To-do", Screen::ToDo),
            self.nav_button(Icon::Book, "Archived classes", Screen::Archived),
            container(rule::horizontal(1).style(divider_style))
                .padding([9, 12]),
            course_entries,
            container(rule::horizontal(1).style(divider_style)).padding([9, 12]),
            self.nav_button(Icon::Settings, "Settings", Screen::Settings),
        ]
        .spacing(3)
        .padding([13, 10])
        .width(Fill);
        row![
            container(scrollable(nav)).width(238).height(Fill),
            rule::vertical(1).style(divider_style),
        ]
        .height(Fill)
        .into()
    }

    fn is_selected_course(&self, id: i64) -> bool {
        matches!(self.screen, Screen::Course(course) | Screen::Item(course) if course == id)
    }

    fn nav_button(&self, glyph: Icon, label: &'static str, destination: Screen) -> Element<'_, Message> {
        let archived_course = matches!(self.screen, Screen::Course(id) | Screen::Item(id)
            if self.snapshot.as_ref().is_some_and(|data| data.courses.iter()
                .any(|course| course.id == id && course.state == CourseState::Archived)));
        let active = self.screen == destination || destination == Screen::Archived && archived_course;
        let color = if active { ACTIVE_FG } else { MUTED };
        button(row![icon(glyph, color, 20), text(label).color(color).size(15)]
            .spacing(14).align_y(Alignment::Center))
            .style(move |_, status| nav_style(active, status))
            .width(Fill)
            .padding([11, 14])
            .on_press(Message::Navigate(destination))
            .into()
    }

    fn home(&self, archived: bool) -> Element<'_, Message> {
        let Some(data) = &self.snapshot else { return self.sign_in_view() };
        let now = Local::now();
        let query = self.search.trim().to_lowercase();
        let matching_state = |course: &&Course| {
            if archived { course.state == CourseState::Archived } else { home_course(course) }
        };
        let matches_search = |course: &&Course| {
            [&course.title, &course.subtitle, &course.description, &course.room]
                .iter().any(|item| item.to_lowercase().contains(&query))
        };
        let group_count = data.courses.iter().filter(matching_state).count();
        let mut visible: Vec<&Course> = data.courses.iter()
            .filter(matching_state)
            .filter(matches_search)
            .collect();
        if self.alphabetical {
            visible.sort_by_cached_key(|course| course.title.to_lowercase());
        }
        let course_count = if visible.len() == group_count {
            format!("{} {}", group_count, if group_count == 1 { "class" } else { "classes" })
        } else {
            format!("{} of {} classes", visible.len(), group_count)
        };
        let search = text_input("Search classes", &self.search)
            .on_input(Message::Search)
            .size(14)
            .padding([11, 13])
            .width(270)
            .style(search_style);
        let sort = button(if self.alphabetical { "A–Z" } else { "Classroom order" })
            .style(outline_style)
            .padding([10, 12])
            .on_press(Message::SortCourses);
        let refresh: Element<'_, Message> = if self.session.is_some() && !self.verify_live_course_count {
            button(row![
                icon(Icon::Refresh, BLUE, 17),
                text(if self.refreshing { "Refreshing…" } else { "Refresh" }).size(14),
            ].spacing(7).align_y(Alignment::Center))
            .padding([10, 12]).style(outline_style)
            .on_press_maybe((!self.has_pending_request()).then_some(Message::Refresh))
            .into()
        } else {
            space().width(0).into()
        };
        let toolbar = row![search, sort, refresh].spacing(9).align_y(Alignment::Center);
        let heading = row![
            column![text(if archived { "Archived classes" } else { "Classes" }).size(27).color(FG), text(course_count).size(14).color(MUTED)]
                .spacing(4).width(Fill),
            toolbar,
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        let classes: Element<'_, Message> = if visible.is_empty() {
            let (title, body) = if query.is_empty() && archived {
                ("No archived classes", "The current course list doesn't contain any classes marked as archived.")
            } else if query.is_empty() && data.courses.iter().any(hidden_course) {
                ("No visible classes", "Choose Show hidden classes below to see your hidden classes.")
            } else if query.is_empty() {
                ("No classes yet", "There are no courses to display for this account.")
            } else {
                ("No matching classes", "Try another name, section or room.")
            };
            empty_panel(Icon::Classroom, title, body)
        } else {
            row(visible.into_iter().map(|course| self.course_card(course, &now)))
                .spacing(24)
                .wrap()
                .vertical_spacing(24)
                .into()
        };
        let offline_controls: Element<'_, Message> = if data.source == SnapshotSource::Demo || self.verify_live_course_count {
            space().height(0).into()
        } else {
            let save: Element<'_, Message> = if data.source == SnapshotSource::Live {
                button("Save offline copy").style(quiet_style).padding([8, 11])
                    .on_press_maybe((!self.has_pending_request()).then_some(Message::SaveOffline)).into()
            } else {
                space().width(0).into()
            };
            row![
                save,
                button("View saved copy").style(quiet_style).padding([8, 11])
                    .on_press_maybe((!self.has_pending_request()).then_some(Message::OpenOffline)),
                button("Remove offline copy").style(quiet_style).padding([8, 11])
                    .on_press_maybe((!self.has_pending_request()).then_some(Message::DeleteOffline)),
            ].spacing(10).into()
        };
        let mut page = column![].spacing(25);
        if !archived && (self.can_read_pages() || self.screenshot_directory.is_some())
            && let Some(invitations) = self.invitation_cards()
        {
            page = page.push(invitations);
        }
        if !archived && !self.verify_live_course_count && !data.courses.is_empty() {
            page = page.push(self.due_soon(data, &now));
        }
        page = page.push(heading).push(classes);
        if !archived && data.courses.iter().any(hidden_course) {
            page = page.push(button(if self.show_hidden_classes { "Collapse hidden classes" } else { "Show hidden classes" })
                .style(outline_style).padding([9, 12]).on_press(Message::ToggleHiddenClasses));
            if self.show_hidden_classes {
                let mut hidden: Vec<_> = data.courses.iter().filter(|course| hidden_course(course))
                    .filter(matches_search).collect();
                if self.alphabetical {
                    hidden.sort_by_cached_key(|course| course.title.to_lowercase());
                }
                let cards: Element<'_, Message> = if hidden.is_empty() {
                    text("No matching hidden classes.").size(14).color(MUTED).into()
                } else {
                    row(hidden.into_iter().map(|course| self.course_card(course, &now)))
                        .spacing(24).wrap().vertical_spacing(24).into()
                };
                page = page.push(column![text("Hidden classes").size(21).font(medium()).color(FG), cards].spacing(12));
            }
        }
        scrollable(page.push(offline_controls).padding(26).width(Fill))
            .width(Fill).height(Fill)
            .into()
    }

    fn due_soon<'a>(&'a self, snapshot: &'a Snapshot, now: &DateTime<Local>) -> Element<'a, Message> {
        let title = if snapshot.source == SnapshotSource::Cache { "Upcoming deadlines" } else { "Due soon" };
        let mut panel = column![row![
            text(title).size(21).font(medium()).color(FG),
            space().width(Fill),
            button("View to-dos").padding([8, 12]).style(quiet_style)
                .on_press(Message::Navigate(Screen::ToDo)),
        ].align_y(Alignment::Center)].spacing(8);
        let items = snapshot.upcoming_work(now, None, 5);
        let status = self.page_read_status(PageRead::Assignments, self.assignments_loaded, "Checking upcoming work…");
        if let Some(status) = status {
            panel = panel.push(status);
        } else if items.is_empty() {
            panel = panel.push(text("No work due in the next seven days.").size(14).color(MUTED));
        }
        for (item, course) in items {
            let accent = course_color(course);
            let deadline = item.due_label.as_deref().unwrap_or("Due date available");
            panel = panel.push(button(row![
                icon(Icon::Check, accent, 19),
                column![
                    text(&item.title).size(15).font(medium()).color(FG).width(Fill)
                        .wrapping(text::Wrapping::WordOrGlyph),
                    text(&course.title).size(13).color(MUTED).width(Fill)
                        .wrapping(text::Wrapping::WordOrGlyph),
                ].spacing(2).width(Fill),
                text(deadline).size(13).color(MUTED),
                icon(Icon::Arrow, MUTED, 15),
            ].spacing(10).align_y(Alignment::Center))
                .padding([7, 9]).width(Fill).style(quiet_style)
                .on_press(Message::OpenItem(course.id, item.id.clone())));
        }
        container(panel).padding([12, 16]).width(Fill).style(|_| outline_surface(10.0)).into()
    }

    fn sign_in_view(&self) -> Element<'_, Message> {
        let form = column![
            container(classroom_mark()).padding(10).style(|_| surface(color!(0xe6f4ea), 12.0)),
            text("Your classes").color(FG).size(30).font(medium()),
            text("Sign in to browse your Google Classroom courses.").size(15).color(MUTED),
            text_input("Google account email", &self.email)
                .padding([12, 13]).size(15).style(search_style).on_input(Message::Email)
                .on_submit_maybe(self.can_sign_in().then_some(Message::SignIn)),
            row![
                button(if self.busy {
                        "Waiting for Google…"
                    } else if self.verification_submitted {
                        "Verification ended"
                    } else {
                        "Sign in with Google"
                    })
                    .padding([11, 17]).style(primary_style)
                    .on_press_maybe(self.can_sign_in().then_some(Message::SignIn)),
                button("Explore demo").padding([11, 13]).style(quiet_style)
                    .on_press_maybe((!self.busy).then_some(Message::Demo))
            ].spacing(8).align_y(Alignment::Center),
            text(if self.busy { "Complete sign-in in the separate Google window." }
                else { "Google opens in a separate sign-in window." })
                .size(13).color(MUTED),
        ]
        .spacing(15)
        .max_width(410);
        container(container(form).padding(30).style(|_| outline_surface(12.0)))
            .center_x(Fill).center_y(Fill).padding(20).into()
    }

    fn course_card<'a>(&'a self, course: &'a Course, now: &DateTime<Local>) -> Element<'a, Message> {
        let teacher = self.class_teachers.get(&course.id).map(String::as_str);
        let header = course_banner(course, self.artwork_for(course), 110.0, false, teacher);
        let mut preview = column![].spacing(7);
        if let Some(snapshot) = &self.snapshot {
            for (item, _) in snapshot.upcoming_work(now, Some(course.id), 2) {
                preview = preview.push(column![
                    text(item.due_label.as_deref().unwrap_or_default()).size(12).color(MUTED),
                    text(truncate(&item.title, 35)).size(14).color(FG),
                ].spacing(2));
            }
        }
        let body = container(preview).width(Fill).height(89).padding([11, 14]);
        let folder: Element<'_, Message> = match self.class_folder_url(course.id) {
            Some(url) => button(row![icon(Icon::Folder, MUTED, 17), text("Drive folder").size(13)]
                .spacing(6).align_y(Alignment::Center)).padding(9).style(quiet_style)
                .on_press(Message::OpenAttachment(url.to_owned())).into(),
            None => space().width(0).into(),
        };
        let footer = container(row![
            folder,
            space().width(Fill),
            button(row![icon(Icon::Check, MUTED, 17), text(if self.class_roles.get(&course.id) == Some(&CourseRole::Teacher) {
                "Open gradebook"
            } else { "Your work" }).size(13)]
                .spacing(6).align_y(Alignment::Center))
                .padding(9).style(quiet_style).on_press(if self.class_roles.get(&course.id) == Some(&CourseRole::Teacher) {
                    Message::OpenGradebook(course.id)
                } else { Message::OpenYourWork(course.id) }),
        ].align_y(Alignment::Center)).width(Fill).padding([2, 9]);
        let card_link = button(column![header, body]).padding(0).width(Fill)
            .style(quiet_style).on_press(Message::Navigate(Screen::Course(course.id)));
        let shown = course.hidden != Some(true);
        let fixture = self.screenshot_directory.is_some() && course.hidden.is_some() && self.screen == Screen::Home;
        let can_toggle = self.can_change_course_visibility(course.id, shown);
        let archive = self.screen == Screen::Home;
        let archive_fixture = self.screenshot_directory.is_some() && self.confirm_class_archive == Some(course.id);
        let can_archive = self.can_change_class_archive(course.id, archive) || archive_fixture;
        let can_move = self.can_move_course(course.id) || fixture && shown && course.state == CourseState::Active;
        let can_drag = can_move && !self.alphabetical && self.search.trim().is_empty() && !self.has_pending_request();
        let card_content: Element<'_, Message> = if can_toggle || can_move || can_archive || fixture {
            let menu = button(text("⋮").size(18).color(WHITE)).padding([2, 8])
                .style(|_, status| button::Style {
                    background: Some(Background::Color(Color { a: if status == button::Status::Hovered { 0.72 } else { 0.42 }, ..Color::BLACK })),
                    text_color: WHITE,
                    border: Border { radius: 15.0.into(), ..Default::default() },
                    ..Default::default()
                })
                .on_press(Message::ToggleCourseMenu(course.id));
            let mut actions = row![].spacing(5);
            if can_drag {
                let grip = container(icon(Icon::Grip, WHITE, 16)).padding(6)
                    .style(|_| surface(Color { a: 0.42, ..Color::BLACK }, 15.0));
                let handle = mouse_area(grip).on_press(Message::BeginCourseDrag(course.id))
                    .interaction(if self.course_drag.is_some() { mouse::Interaction::Grabbing } else { mouse::Interaction::Grab });
                actions = actions.push(tooltip(handle, "Drag to reorder", tooltip::Position::Bottom));
            }
            stack![card_link, container(actions.push(menu)).align_right(Fill).padding(8)].into()
        } else {
            card_link.into()
        };
        let mut card = column![card_content, rule::horizontal(1).style(divider_style), footer];
        if self.open_course_menu == Some(course.id) && (can_toggle || can_move || can_archive || fixture) {
            let ready = !self.has_pending_request() || fixture;
            let choosing_move = self.move_course_menu == Some(course.id) && can_move;
            let action = if shown {
                if self.confirm_hide_course == Some(course.id) {
                    Message::ChangeCourseVisibility(course.id, true)
                } else {
                    Message::ConfirmHideCourse(course.id)
                }
            } else {
                Message::ChangeCourseVisibility(course.id, false)
            };
            let confirming_hide = self.confirm_hide_course == Some(course.id);
            let confirming_archive = self.confirm_class_archive == Some(course.id);
            let mut menu = column![].spacing(6);
            if choosing_move {
                menu = menu.push(text("Move class").size(14).font(medium()).color(FG))
                    .push(pick_list(self.course_move_options(course.id), None::<CourseMoveOption>, move |choice| {
                        Message::MoveCourse(course.id, choice.placement)
                    }).placeholder("Choose a position").width(Fill));
            } else if confirming_hide {
                menu = menu.push(text("Hide this class from Home, To-do and Calendar?").size(14).color(FG))
                    .push(text("You’ll stay enrolled.").size(13).color(MUTED));
            } else if confirming_archive {
                menu = menu.push(text(if archive { "Archive this class?" } else { "Restore this class?" }).size(14).color(FG))
                    .push(text(if archive {
                        "The class moves to Archived classes. Members can still view its work."
                    } else {
                        "The class returns to the active class list."
                    }).size(13).color(MUTED));
            }
            let mut controls = row![
                button("Cancel").padding([6, 9]).style(quiet_style)
                    .on_press(Message::ToggleCourseMenu(course.id)),
            ].spacing(6);
            if !choosing_move && !confirming_archive && (can_toggle || fixture) {
                controls = controls.push(button(if shown { "Hide" } else { "Unhide" })
                    .padding([6, 9]).style(outline_style).on_press_maybe(ready.then_some(action)));
            }
            if !choosing_move && !confirming_hide && !confirming_archive && can_move {
                controls = controls.push(button("Move…").padding([6, 9]).style(outline_style)
                    .on_press_maybe(ready.then_some(Message::ChooseCourseMove(course.id))));
            }
            if !choosing_move && !confirming_hide && can_archive {
                let action = if confirming_archive { Message::ChangeClassArchive(course.id, archive) }
                    else { Message::ConfirmClassArchive(course.id) };
                controls = controls.push(button(if archive { "Archive" } else { "Restore" })
                    .padding([6, 9]).style(outline_style).on_press_maybe(ready.then_some(action)));
            }
            card = card.push(container(menu.push(controls)).width(Fill).padding([5, 10]));
        }
        let target = self.course_drag.as_ref().and_then(|drag| drag.target) == Some(course.id);
        let card = container(card).width(280).style(move |_| {
            let mut style = outline_surface(9.0);
            if target {
                style.border.color = BLUE;
                style.border.width = 2.0;
            }
            style
        }).clip(true);
        if self.course_drag.is_some() && can_move {
            mouse_area(card).on_enter(Message::EnterCourseDropTarget(course.id))
                .on_exit(Message::ExitCourseDropTarget(course.id)).interaction(mouse::Interaction::Grabbing).into()
        } else {
            card.into()
        }
    }

    fn visible_active_classes(&self) -> impl Iterator<Item = &Course> {
        self.snapshot.iter().flat_map(|snapshot| &snapshot.courses)
            .filter(|course| course.state == CourseState::Active && course.shows_on_home())
    }

    fn can_move_course(&self, id: i64) -> bool {
        self.screen == Screen::Home && self.can_read_pages() && self.visible_active_classes().take(2).count() == 2
            && self.visible_active_classes().any(|course| course.id == id)
    }

    fn course_move_options(&self, moved: i64) -> Vec<CourseMoveOption> {
        let mut options = vec![CourseMoveOption { label: "To beginning".into(), placement: CoursePlacement::Beginning }];
        options.extend(self.visible_active_classes().filter(|course| course.id != moved).map(|course| CourseMoveOption {
            label: if course.subtitle.trim().is_empty() { format!("After {}", course.title) }
                else { format!("After {} · {}", course.title, course.subtitle) },
            placement: CoursePlacement::After(course.id),
        }));
        options.push(CourseMoveOption { label: "To end".into(), placement: CoursePlacement::End });
        options
    }

    fn drop_placement(&self, source: i64, target: i64) -> Option<CoursePlacement> {
        let ordered: Vec<_> = self.visible_active_classes().map(|course| course.id).collect();
        let source_index = ordered.iter().position(|&course| course == source)?;
        let target_index = ordered.iter().position(|&course| course == target)?;
        if source_index < target_index {
            Some(CoursePlacement::After(target))
        } else if source_index > target_index {
            Some(CoursePlacement::Before(target))
        } else {
            None
        }
    }

    fn can_change_membership(&self) -> bool {
        self.session.is_some() && !self.verify_live_course_count
            && self.snapshot.as_ref().is_some_and(|snapshot| snapshot.source == SnapshotSource::Live)
    }

    fn can_create_class(&self) -> bool {
        self.can_change_membership() && self.class_creation_allowed
    }

    fn can_change_course_visibility(&self, id: i64, hidden: bool) -> bool {
        self.screen == Screen::Home && self.can_read_pages() && self.snapshot.as_ref()
            .is_some_and(|snapshot| snapshot.courses.iter().any(|course| {
                course.id == id && course.state == CourseState::Active && course.hidden == Some(!hidden)
                    && (hidden || self.show_hidden_classes)
            }))
    }

    fn can_change_class_archive(&self, id: i64, archive: bool) -> bool {
        let (screen, state) = if archive { (Screen::Home, CourseState::Active) }
            else { (Screen::Archived, CourseState::Archived) };
        self.screen == screen && self.can_read_pages()
            && self.class_roles.get(&id) == Some(&CourseRole::Teacher)
            && self.snapshot.as_ref().is_some_and(|snapshot| snapshot.courses.iter()
                .any(|course| course.id == id && course.state == state))
    }

    fn invalidate_invitation_read(&mut self) {
        self.invitation_generation = self.invitation_generation.wrapping_add(1);
        self.loading_invitations = false;
    }

    fn pending_invitations(&self) -> impl Iterator<Item = &Course> {
        self.invitations.iter().flatten().filter(|invited| {
            self.snapshot.as_ref().is_none_or(|snapshot| !snapshot.courses.iter().any(|course| course.id == invited.id))
        })
    }

    fn can_respond_to_invitation(&self, course_id: i64) -> bool {
        matches!(self.screen, Screen::Home | Screen::Join) && self.can_change_membership()
            && !self.has_pending_request() && !self.loading_invitations
            && self.pending_invitations().any(|course| course.id == course_id)
    }

    fn invitation_actions(&self, course: i64) -> Element<'_, Message> {
        let ready = self.can_respond_to_invitation(course) || self.screenshot_directory.is_some();
        if self.confirming_decline == Some(course) {
            return column![
                text("Decline this invitation?").size(14).color(FG),
                row![
                    button("Cancel").padding([7, 12]).style(quiet_style)
                        .on_press(Message::CancelDeclineInvitation),
                    button("Decline").padding([7, 12]).style(outline_style)
                        .on_press_maybe(ready.then_some(Message::DeclineInvitation(course))),
                ].spacing(6),
            ].spacing(6).into();
        }
        row![
            button("Decline").padding([7, 12]).style(quiet_style)
                .on_press_maybe(ready.then_some(Message::ConfirmDeclineInvitation(course))),
            button("Join").padding([7, 12]).style(primary_style)
                .on_press_maybe(ready.then_some(Message::AcceptInvitation(course))),
        ].spacing(6).into()
    }

    fn invitation_cards(&self) -> Option<Element<'_, Message>> {
        let mut invitations = self.pending_invitations().peekable();
        let error = self.can_read_pages() && self.page_errors.contains_key(&PageRead::Invitations);
        if invitations.peek().is_none() && !error {
            return None;
        }
        let mut section = column![text("Class invitations").size(21).font(medium()).color(FG)].spacing(10);
        if error && let Some(status) = self.page_read_status(PageRead::Invitations, false, "Checking invitations…") {
            section = section.push(status);
        }
        if invitations.peek().is_some() {
            section = section.push(row(invitations.map(|invited| {
                let card = column![
                    container(column![
                        text(&invited.title).size(18).font(medium()).color(WHITE)
                            .wrapping(text::Wrapping::WordOrGlyph),
                        text(&invited.subtitle).size(13).color(WHITE),
                    ].spacing(4)).padding(13).width(Fill)
                        .style(move |_| surface(course_color(invited), 0.0)),
                    container(self.invitation_actions(invited.id)).padding(10).align_right(Fill),
                ];
                container(card).width(280).style(|_| outline_surface(9.0)).clip(true).into()
            })).spacing(14).wrap().vertical_spacing(12));
        }
        Some(section.into())
    }

    fn join_class_view(&self) -> Element<'_, Message> {
        let ready = self.can_change_membership() && !self.has_pending_request()
            && desktop_auth::valid_class_code(&self.class_code);
        let form = column![
            text("Join class").size(27).color(FG),
            text("Enter the class code from your teacher.").size(15).color(MUTED),
            text_input("Class code", &self.class_code).padding(11)
                .on_input(Message::EditClassCode)
                .on_submit_maybe(ready.then_some(Message::JoinClass)),
            row![
                button("Cancel").style(quiet_style).padding([9, 12])
                    .on_press(Message::Navigate(Screen::Home)),
                button(if self.changing_membership { "Joining…" } else { "Join" })
                    .style(primary_style).padding([9, 13]).on_press_maybe(ready.then_some(Message::JoinClass)),
            ].spacing(10),
        ].spacing(16).max_width(400);
        let mut invitations = column![text("Invitations").size(21).font(medium()).color(FG)].spacing(12);
        let mut rows = self.pending_invitations().peekable();
        if self.invitations.is_some() && rows.peek().is_none() && !self.loading_invitations {
            invitations = invitations.push(text("No pending class invitations.").size(14).color(MUTED));
        }
        for course in rows {
            invitations = invitations.push(row![
                column![text(&course.title).size(15), text(&course.subtitle).size(13).color(MUTED)].width(Fill),
                self.invitation_actions(course.id),
            ].spacing(12).align_y(Alignment::Center));
        }
        if let Some(status) = self.page_read_status(
            PageRead::Invitations, self.invitations.is_some(), "Loading invitations…",
        ) {
            invitations = invitations.push(status);
        }
        let panels = column![
            container(form).padding(25).style(|_| outline_surface(12.0)).width(Fill),
            container(invitations).padding(25).style(|_| outline_surface(12.0)).width(Fill),
        ].spacing(16).max_width(560);
        scrollable(container(panels).center_x(Fill).padding(24)).height(Fill).into()
    }

    fn create_class_view(&self) -> Element<'_, Message> {
        let editing = !self.changing_membership;
        let ready = self.can_create_class() && !self.has_pending_request()
            && desktop_auth::class_input_valid(&self.new_class);
        let mut form = column![text("Create class").font(medium()).size(24).color(FG)].spacing(14);
        for (field, label, value) in [
            (NewClassField::Name, "Class name (required)", self.new_class.name.as_str()),
            (NewClassField::Section, "Section", self.new_class.section.as_str()),
            (NewClassField::Subject, "Subject", self.new_class.subject.as_str()),
            (NewClassField::Room, "Room", self.new_class.room.as_str()),
        ] {
            form = form.push(column![
                text(label).size(13).color(MUTED),
                text_input(label, value).padding(10)
                    .on_input_maybe(editing.then_some(move |value| Message::EditNewClass(field, value))),
            ].spacing(4));
        }
        let actions = row![
            button("Cancel").style(quiet_style).padding([9, 12])
                .on_press_maybe(editing.then_some(Message::Navigate(Screen::Home))),
            button(if self.changing_membership { "Creating…" } else { "Create" })
                .style(primary_style).padding([9, 13]).on_press_maybe(ready.then_some(Message::CreateClass)),
        ].spacing(10);
        let panel = container(form.push(actions)).padding(24).width(Fill).max_width(490)
            .style(|_| outline_surface(12.0));
        scrollable(container(panel).center_x(Fill).padding(24)).height(Fill).into()
    }

    fn course(&self, id: i64) -> Element<'_, Message> {
        let Some(course) = self.snapshot.as_ref().and_then(|data| data.courses.iter().find(|course| course.id == id)) else {
            return self.unavailable_screen("Class", Icon::Classroom, "This class is no longer in the course list.");
        };
        let title = &course.title;
        let mut breadcrumb = row![
            button(if course.state == CourseState::Archived { "Archived classes" } else { "Classes" })
                .padding([8, 10]).style(quiet_style).on_press(Message::Navigate(
                    if course.state == CourseState::Archived { Screen::Archived } else { Screen::Home }
                )),
            icon(Icon::Arrow, MUTED, 16),
            text(title).size(14).color(MUTED).width(Fill),
            button("Copy class link").padding([6, 8]).style(quiet_style)
                .on_press(Message::CopyClassroomLink(ClassroomLink::Course { course: id, classwork: self.tab == Tab::Classwork })),
        ].spacing(4).align_y(Alignment::Center);
        if self.can_edit_class_stream_settings(id)
            || self.screenshot_directory.is_some() && self.class_roles.get(&id) == Some(&CourseRole::Teacher)
        {
            breadcrumb = breadcrumb.push(button("Class settings").padding([6, 8]).style(quiet_style)
                .on_press(Message::SetTab(Tab::ClassSettings)));
        }
        let tabs = row(Tab::COURSE_TABS.into_iter()
            .filter(|tab| *tab != Tab::Grades || self.class_roles.get(&id) == Some(&CourseRole::Teacher)).map(|tab| {
            let active = self.tab == tab || tab == Tab::Classwork && self.tab == Tab::YourWork;
            container(column![
                button(text(tab.name()).size(15).font(medium())).padding([12, 18])
                    .style(move |_, status| tab_style(active, status))
                    .on_press(Message::SetTab(tab)),
                container(space().height(3)).width(Fill)
                    .style(move |_| surface(if active { BLUE } else { Color::TRANSPARENT }, 2.0)),
            ]).width(Length::Shrink).into()
        })).spacing(8);
        let mut body = column![breadcrumb, tabs].spacing(18);
        if self.tab == Tab::Stream {
            body = body.push(course_banner(course, self.artwork_for(course), 192.0, true, None));
        }
        let meet: Option<Element<'_, Message>> = course.meet_url.as_deref().map(|url| {
            button(row![icon(Icon::Video, BLUE, 17), text("Meet")].spacing(6).align_y(Alignment::Center))
                .padding([8, 12]).style(outline_style)
                .on_press(Message::OpenAttachment(url.to_owned())).into()
        });
        let detail: Element<'_, Message> = match self.tab {
            Tab::Stream => self.stream_layout(course),
            Tab::Classwork => {
                let mut actions = row![].spacing(8);
                if self.class_roles.get(&id) == Some(&CourseRole::Teacher) {
                    if self.can_edit_assignment_drafts(id) || self.screenshot_directory.is_some() {
                        actions = actions.push(button("Create assignment").padding([8, 12]).style(primary_style)
                            .on_press_maybe((!self.has_pending_request() && self.assignment_editor.is_none() && self.assignment_publication.is_none())
                                .then_some(Message::OpenAssignmentEditor(id, None))));
                    }
                } else {
                    actions = actions.push(button("View your work").padding([8, 12]).style(outline_style)
                        .on_press(Message::OpenYourWork(id)));
                }
                if let Some(url) = self.class_folder_url(id) {
                    actions = actions.push(button(row![icon(Icon::Folder, BLUE, 17), text("Class Drive folder")]
                        .spacing(6).align_y(Alignment::Center)).padding([8, 12]).style(outline_style)
                        .on_press(Message::OpenAttachment(url.to_owned())));
                }
                if let Some(calendar) = self.class_calendar_button(id) {
                    actions = actions.push(calendar);
                }
                if let Some(meet) = meet {
                    actions = actions.push(meet);
                }
                let mut work = column![actions.wrap().vertical_spacing(8)].spacing(12);
                if let Some(editor) = self.assignment_editor.as_ref().filter(|editor| editor.course == id) {
                    work = work.push(self.assignment_editor_panel(editor));
                }
                if let Some(prompt) = self.assignment_publication.as_ref().filter(|prompt| prompt.draft.item.course_id == id) {
                    work = work.push(self.assignment_publication_panel(prompt));
                }
                if let Some(topics) = self.teacher_topic_panel(id) {
                    work = work.push(topics);
                }
                if let Some(editor) = self.item_topic_editor.as_ref().filter(|editor| editor.target.course == id) {
                    work = work.push(self.item_topic_editor_panel(editor));
                }
                if self.class_folders_failed && self.can_read_pages() {
                    work = work.push(row![
                        text("Couldn’t load class Drive folders.").size(14).color(MUTED),
                        button("Retry").padding([7, 10]).style(quiet_style).on_press(Message::RetryClassFolders),
                    ].spacing(8).align_y(Alignment::Center));
                }
                work.push(self.classwork(id)).into()
            }
            Tab::People => self.people(id),
            Tab::YourWork => self.your_work(id),
            Tab::Grades => self.teacher_gradebook_panel(id),
            Tab::ClassSettings => self.class_stream_settings_panel(id),
        };
        let membership: Element<'_, Message> = if self.tab == Tab::YourWork || self.class_roles.get(&id) == Some(&CourseRole::Teacher)
            || !self.can_change_membership() || !course.state.shows_on_home()
        {
            space().height(0).into()
        } else if self.confirming_unenroll == Some(id) {
            column![
                text("Unenroll from this class? You’ll lose access to its classwork.").size(15).color(FG),
                row![
                    button("Cancel").style(quiet_style).on_press(Message::CancelUnenroll),
                    button("Unenroll").style(outline_style)
                        .on_press_maybe((!self.has_pending_request()).then_some(Message::Unenroll(id))),
                ].spacing(10),
            ].spacing(9).into()
        } else {
            button("Unenroll…").style(quiet_style).padding([7, 9])
                .on_press_maybe((!self.has_pending_request()).then_some(Message::ConfirmUnenroll(id)))
                .into()
        };
        scrollable(body.push(detail).push(membership).padding([19, 27]).width(Fill))
            .width(Fill).height(Fill).into()
    }

    fn reset_stream_access(&mut self) {
        self.stream_access_generation = self.stream_access_generation.wrapping_add(1);
        self.loading_stream_access.clear();
        self.stream_access.clear();
        self.stream_viewer_id = None;
        self.page_errors.retain(|read, _| !matches!(read, PageRead::StreamAccess(_)));
    }

    fn stream_layout<'a>(&'a self, course: &'a Course) -> Element<'a, Message> {
        responsive(move |size| {
            let mut feed = column![].spacing(13).width(Fill);
            if let Some(composer) = self.stream_composer(course.id) {
                feed = feed.push(composer);
            }
            feed = feed.push(self.stream_items(course));
            let sidebar = self.stream_sidebar(course);
            if size.width >= 675.0 {
                row![container(sidebar).width(190), feed].spacing(18).into()
            } else {
                column![feed, sidebar].spacing(13).into()
            }
        }).width(Fill).height(Length::Shrink).into()
    }

    fn stream_sidebar<'a>(&'a self, course: &'a Course) -> Element<'a, Message> {
        let mut sidebar = column![].spacing(13).width(Fill);
        if let Some(url) = course.meet_url.as_deref() {
            sidebar = sidebar.push(button(row![icon(Icon::Video, BLUE, 17), text("Meet")].spacing(8))
                .padding([8, 12]).width(Fill).style(outline_style)
                .on_press(Message::OpenAttachment(url.to_owned())));
        }
        if home_course(course) {
            let snapshot = self.snapshot.as_ref();
            let work = snapshot.map_or_else(Vec::new, |data| data.upcoming_work(&Local::now(), Some(course.id), 5));
            let coursework_loaded = snapshot.is_some_and(|data| data.coursework_for(course.id).is_some());
            let mut upcoming = column![text("Upcoming").size(16).font(medium()).color(FG)].spacing(7).width(Fill);
            if coursework_loaded && work.is_empty() {
                upcoming = upcoming.push(text("No work due soon").size(14).color(MUTED));
            } else if coursework_loaded
                && let Some(status) = self.page_read_status(
                    PageRead::Detail(DetailRead::Submissions(course.id)),
                    self.submissions_checked.contains(&course.id), "Checking upcoming work…",
                )
            {
                upcoming = upcoming.push(status);
            } else if !coursework_loaded && self.can_read_pages() {
                upcoming = upcoming.push(text("Checking upcoming work…").size(14).color(MUTED));
            } else {
                for (item, _) in work {
                    upcoming = upcoming.push(button(column![
                        text(&item.title).size(14).color(FG).wrapping(text::Wrapping::WordOrGlyph),
                        text(item.due_label.as_deref().unwrap_or_default()).size(12).color(MUTED),
                    ].spacing(3)).padding([5, 3]).width(Fill).style(quiet_style)
                        .on_press(Message::OpenItem(course.id, item.id.clone())));
                }
            }
            upcoming = upcoming.push(container(button("View all").padding([5, 7]).style(quiet_style)
                .on_press(Message::OpenYourWork(course.id))).align_right(Fill));
            sidebar = sidebar.push(container(upcoming).width(Fill).padding(13).style(|_| outline_surface(9.0)));
        }
        if !course.description.trim().is_empty() || !course.room.trim().is_empty() {
            let mut details = column![text("About this class").size(16).font(medium()).color(FG)].spacing(7);
            if !course.description.trim().is_empty() {
                details = details.push(text(&course.description).size(14).color(MUTED).wrapping(text::Wrapping::WordOrGlyph));
            }
            if !course.room.trim().is_empty() {
                details = details.push(text(format!("Room {}", course.room)).size(13).color(MUTED));
            }
            sidebar = sidebar.push(container(details).padding(13).width(Fill).style(|_| outline_surface(9.0)));
        }
        sidebar.into()
    }

    fn on_live_stream(&self, course: i64) -> bool {
        self.screen == Screen::Course(course) && self.tab == Tab::Stream && self.can_read_visible_stream_access(course)
    }

    fn can_edit_class_stream_settings(&self, course: i64) -> bool {
        self.can_read_pages() && self.screen == Screen::Course(course)
            && self.class_roles.get(&course) == Some(&CourseRole::Teacher) && self.can_grade_teacher_work(course)
    }

    fn class_stream_settings_panel(&self, course: i64) -> Element<'_, Message> {
        let mut panel = column![
            text("Class settings").size(24).font(medium()).color(FG),
        ].spacing(14).width(Fill).max_width(640);
        let editor = self.class_stream_settings.as_ref().filter(|editor| editor.course == course);
        if let Some(status) = self.page_read_status(
            PageRead::Detail(DetailRead::ClassStreamSettings(course)), editor.is_some(), "Loading class settings…",
        ) {
            panel = panel.push(status);
        }
        if let Some(editor) = editor {
            let ready = !self.has_pending_request();
            let details = &editor.draft.details;
            let editable = !self.saving_stream_settings;
            let description = text_editor(&editor.description).placeholder("Description").height(82).padding(8);
            let description = if editable { description.on_action(Message::EditCourseDescription) } else { description };
            let mut general = column![
                text("Class details").size(17).font(medium()),
                text("Class name").size(13).color(MUTED),
                text_input("Class name", &details.title).padding(8).on_input_maybe(editable.then_some(Message::EditCourseTitle)),
                row![
                    column![text("Section").size(13).color(MUTED), text_input("Section", &details.subtitle).padding(8)
                        .on_input_maybe(editable.then_some(Message::EditCourseSection))].spacing(4).width(Fill),
                    column![text("Room").size(13).color(MUTED), text_input("Room", &details.room).padding(8)
                        .on_input_maybe(editable.then_some(Message::EditCourseRoom))].spacing(4).width(Fill),
                ].spacing(8),
                text("Description").size(13).color(MUTED),
                description,
            ].spacing(7);
            let desired = editor.changes();
            if !desired.details.valid() {
                general = general.push(text("Enter a class name and check the field lengths.").size(13).color(MUTED));
            }
            let mut posting = column![text("Student posting permissions").size(17).font(medium())].spacing(7);
            for policy in desktop_auth::StreamPostingPolicy::ALL {
                let selected = editor.draft.stream.posting == Some(policy);
                posting = posting.push(button(policy.label()).width(Fill).padding([8, 10])
                    .style(move |_, status| nav_style(selected, status))
                    .on_press_maybe((ready && editor.original.stream.posting.is_some()).then_some(Message::SetCoursePostingPolicy(policy))));
            }
            if editor.original.stream.posting.is_none() {
                posting = posting.push(text("Classroom did not send a posting policy.").size(13).color(MUTED));
            }
            let mut display = column![text("Classwork on the Stream").size(17).font(medium())].spacing(7);
            for (option, label) in [
                (StreamClassworkDisplay::Condensed, "Show condensed notifications"),
                (StreamClassworkDisplay::Expanded, "Show attachments and details"),
                (StreamClassworkDisplay::Hidden, "Hide Classwork notifications"),
            ] {
                let selected = editor.draft.stream.classwork == Some(option);
                display = display.push(button(label).width(Fill).padding([8, 10])
                    .style(move |_, status| nav_style(selected, status))
                    .on_press_maybe((ready && editor.original.stream.classwork.is_some()).then_some(Message::SetCourseStreamDisplay(option))));
            }
            if editor.original.stream.classwork.is_none() {
                display = display.push(text("Classroom did not send a Classwork display setting.").size(13).color(MUTED));
            }
            panel = panel.push(container(general).padding(14).width(Fill).style(|_| outline_surface(9.0)))
                .push(container(posting).padding(14).width(Fill).style(|_| outline_surface(9.0)))
                .push(container(display).padding(14).width(Fill).style(|_| outline_surface(9.0)))
                .push(row![
                    button(if self.saving_stream_settings { "Saving…" } else { "Save" }).padding([8, 13]).style(primary_style)
                        .on_press_maybe((ready && desired.details.valid() && editor.original != desired).then_some(Message::SaveCourseStreamSettings)),
                    button("Back to Stream").style(quiet_style).padding([8, 13]).on_press(Message::SetTab(Tab::Stream)),
                ].spacing(8));
        }
        panel.into()
    }

    fn can_read_visible_stream_access(&self, course: i64) -> bool {
        (self.screen == Screen::Item(course) || self.screen == Screen::Course(course) && self.tab == Tab::Stream)
            && self.can_read_pages()
            && self.snapshot.as_ref().is_some_and(|snapshot| snapshot.courses.iter()
                .any(|entry| entry.id == course && entry.state == CourseState::Active))
    }

    fn missing_stream_access(&self, course: i64) -> bool {
        self.can_read_visible_stream_access(course) && !self.stream_access.contains_key(&course)
            && !self.loading_stream_access.contains(&course)
            && !self.page_errors.contains_key(&PageRead::StreamAccess(course))
    }

    fn can_post_stream(&self, course: i64) -> bool {
        self.on_live_stream(course) && self.stream_access.get(&course) == Some(&StreamAccess::PostAndComment)
    }

    fn can_delete_stream_post(&self, course: i64, id: i64) -> bool {
        self.on_live_stream(course) && self.displayed_announcement(course, id).is_some_and(|item| {
            self.owns_announcement(item) || self.may_moderate_author(course, item.author_id)
        })
    }

    fn displayed_announcement(&self, course: i64, id: i64) -> Option<&Coursework> {
        self.snapshot.as_ref()?.coursework_for(course)?.iter()
            .find(|item| item.kind == CourseworkKind::Announcement && item.id.parse() == Ok(id))
    }

    fn owns_announcement(&self, item: &Coursework) -> bool {
        item.kind == CourseworkKind::Announcement
            && self.stream_viewer_id.is_some() && item.author_id == self.stream_viewer_id
    }

    fn stream_row<'a>(&'a self, item: &'a Coursework) -> Element<'a, Message> {
        let course = item.course_id;
        let base = coursework_row(item, None);
        let Ok(id) = item.id.parse::<i64>() else { return base };
        let fixture = self.screenshot_directory.is_some();
        if item.kind != CourseworkKind::Announcement
            || (!self.owns_announcement(item) && !self.may_moderate_author(course, item.author_id))
            || (!fixture && !self.on_live_stream(course))
        {
            return base;
        }
        let ready = fixture || !self.has_pending_request();
        if self.confirming_stream_delete == Some((course, id)) {
            let confirmation = column![
                text("Delete this post?").font(medium()).size(16).color(FG),
                text("This also deletes its comments. You can’t undo this.").size(14).color(MUTED),
                row![
                    button("Cancel").padding([8, 12]).style(quiet_style).on_press(Message::CancelDeleteStreamPost),
                    button("Delete").padding([8, 12]).style(outline_style)
                        .on_press_maybe(ready.then_some(Message::DeleteStreamPost(course, id))),
                ].spacing(8),
            ].spacing(8);
            return container(confirmation).width(Fill).padding(14)
                .style(|_| outline_surface(9.0)).into();
        }
        let label = if self.deleting_stream_post == Some((course, id)) { "Deleting…" } else { "Delete…" };
        row![
            base,
            button(label).padding([8, 12]).style(quiet_style)
                .on_press_maybe(ready.then_some(Message::ConfirmDeleteStreamPost(course, id))),
        ].spacing(6).align_y(Alignment::Center).into()
    }

    fn stream_composer(&self, course: i64) -> Option<Element<'_, Message>> {
        if self.on_live_stream(course)
            && let Some(status) = self.page_read_status(
                PageRead::StreamAccess(course), self.stream_access.contains_key(&course), "Checking Stream permissions…",
            )
        {
            return Some(status);
        }
        let fixture = self.screenshot_directory.is_some();
        if !self.can_post_stream(course) && !fixture {
            return None;
        }
        if self.open_stream_composer != Some(course) {
            return Some(button(
                row![icon(Icon::Feed, BLUE, 20), text("Announce something to your class").size(15).color(FG)]
                    .spacing(12).align_y(Alignment::Center),
            ).padding([13, 16]).width(Fill).style(outline_style)
                .on_press(Message::OpenStreamComposer(course)).into());
        }
        let draft = self.stream_drafts.get(&course)?;
        let sending = self.sending_stream_post == Some(course);
        let uploading = self.uploading_stream_file == Some(course);
        let choosing = self.choosing_stream_file == Some(course);
        let ready_to_attach = fixture || !self.has_pending_request();
        let body = draft.body.text();
        let ready = (fixture || !self.has_pending_request())
            && !self.page_errors.contains_key(&PageRead::Coursework(course))
            && draft.can_send(&body);
        let mut actions = column![
            text("Announce something to your class").size(15).font(medium()).color(FG),
            row([
                (StreamFormat::Bold, "B", "Bold (⌘B)"),
                (StreamFormat::Italic, "I", "Italic (⌘I)"),
                (StreamFormat::Underline, "U", "Underline (⌘U)"),
                (StreamFormat::BulletList, "• List", "Bulleted list"),
            ].into_iter().map(|(format, label, help)| {
                let glyph = text(label).size(14).font(Font {
                    weight: if matches!(format, StreamFormat::Bold) { Weight::Bold } else { Weight::Normal },
                    style: if matches!(format, StreamFormat::Italic) { iced::font::Style::Italic } else { iced::font::Style::Normal },
                    ..FORMATTED_FONT
                });
                tooltip(button(glyph).padding([4, 10]).style(outline_style)
                    .on_press_maybe((!sending).then_some(Message::FormatStreamDraft(course, format))),
                    help, tooltip::Position::Top).into()
            })).spacing(5),
            text_editor(&draft.body).placeholder("Share something with your class…").height(112).padding(10)
                .key_binding(move |event| stream_editor_binding(course, event))
                .on_action(move |action| Message::EditStreamDraft(course, action)),
        ].spacing(9);
        let styles = draft.description_styles();
        let bullets: Vec<_> = draft.bullets.iter().copied().collect();
        if styles.iter().any(|style| !style.is_plain()) || !bullets.is_empty() {
            actions = actions.push(container(column![
                text("Preview").color(MUTED).size(12),
                formatted_description(&body, &styles, &bullets),
            ].spacing(4)).width(Fill).padding(9).style(|_| surface(SOFT, 7.0)));
        }
        if body.trim().len() > 10_000 {
            actions = actions.push(text("The message is too long.").size(14).color(color!(0x9b2722)));
        }
        for (index, url) in draft.links.iter().enumerate() {
            actions = actions.push(container(row![
                text(truncate(url, 56)).color(FG).size(14).width(Fill),
                button("Remove").style(quiet_style).padding([5, 8])
                    .on_press_maybe((!sending).then_some(Message::RemoveStreamLink(course, index))),
            ].spacing(8).align_y(Alignment::Center)).padding([4, 9]).width(Fill)
                .style(|_| outline_surface(7.0)));
        }
        for (index, file) in draft.files.iter().enumerate() {
            actions = actions.push(container(row![
                icon(Icon::Folder, MUTED, 16),
                text(truncate(file.name(), 50)).size(14).color(FG).width(Fill),
                button("Remove").style(quiet_style).padding([5, 8])
                    .on_press_maybe(ready_to_attach.then_some(Message::RemoveStreamFile(course, index))),
            ].spacing(8).align_y(Alignment::Center)).padding([4, 9]).width(Fill)
                .style(|_| outline_surface(7.0)));
        }
        if !draft.files.is_empty() {
            actions = actions.push(text("Removing an attachment leaves the file in Google Drive.").size(12).color(MUTED));
        }
        if let Some(path) = &draft.selected_file {
            let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("Selected file");
            actions = actions.push(container(row![
                text(truncate(name, 50)).size(14).color(FG).width(Fill),
                button("Remove").style(quiet_style).padding([5, 7])
                    .on_press_maybe(ready_to_attach.then_some(Message::ClearStreamFile(course))),
                button(if uploading { "Uploading…" } else { "Upload to Drive" }).style(outline_style).padding([6, 9])
                    .on_press_maybe(ready_to_attach.then_some(Message::UploadStreamFile(course))),
            ].spacing(6).align_y(Alignment::Center)).padding([4, 9]).width(Fill)
                .style(|_| outline_surface(7.0)));
        }
        if let Some(input) = &draft.link_input {
            let can_add = !sending && draft.pending_link().is_some();
            let link_input = text_input("https://", input).width(Fill).padding(8)
                .on_input(move |value| Message::EditStreamLink(course, value))
                .on_submit_maybe(can_add.then_some(Message::AddStreamLink(course)));
            actions = actions.push(row![
                link_input,
                button("Cancel").style(quiet_style).padding([6, 8])
                    .on_press_maybe((!sending).then_some(Message::CancelStreamLink(course))),
                button("Attach").style(outline_style).padding([6, 9])
                    .on_press_maybe(can_add.then_some(Message::AddStreamLink(course))),
            ].spacing(6).align_y(Alignment::Center));
        }
        if let Some(picker) = self.drive_picker.as_ref().filter(|picker| picker.course == course && picker.item.is_none()) {
            actions = actions.push(self.drive_picker_panel(picker, ready_to_attach,
                |file| draft.files.iter().any(|existing| existing.id() == file.id())));
        }
        actions = actions.push(container(row![
            button("Add link").style(outline_style).padding([8, 10])
                .on_press_maybe((!sending && draft.link_input.is_none()).then_some(Message::OpenStreamLink(course))),
            button(if choosing { "Choosing…" } else { "Choose file…" }).style(outline_style).padding([8, 10])
                .on_press_maybe((ready_to_attach && draft.selected_file.is_none()).then_some(Message::ChooseStreamFile(course))),
            button("Google Drive").style(outline_style).padding([8, 10])
                .on_press_maybe((ready_to_attach && self.drive_picker.is_none()).then_some(Message::OpenDrivePicker(course))),
            space().width(Fill),
            button("Close").style(quiet_style).padding([8, 12])
                .on_press_maybe((!sending).then_some(Message::CloseStreamComposer)),
            button(if sending { "Posting…" } else { "Post" }).style(primary_style).padding([8, 13])
                .on_press_maybe(ready.then_some(Message::SendStreamPost(course))),
        ].spacing(8).wrap().vertical_spacing(6)).align_right(Fill));
        Some(container(actions).width(Fill).padding(14).style(|_| outline_surface(9.0)).into())
    }

    fn can_edit_stream_attachments(&self, course: i64) -> bool {
        self.open_stream_composer == Some(course) && self.can_post_stream(course)
            && self.sending_stream_post != Some(course) && self.uploading_stream_file != Some(course)
    }

    fn can_add_submission_drive_file(&self, course: i64, item: i64) -> bool {
        self.can_compose_comment(course, item) && self.submissions_checked.contains(&course)
            && !self.page_errors.contains_key(&PageRead::Detail(DetailRead::Submissions(course)))
            && self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(course))
                .and_then(|items| items.iter().find(|entry| entry.id.parse::<i64>().ok() == Some(item)))
                .is_some_and(|entry| entry.kind == CourseworkKind::Assignment
                    && entry.submission.as_ref().and_then(desktop_auth::SubmissionAction::for_submission)
                        == Some(desktop_auth::SubmissionAction::TurnIn))
    }

    fn discard_submission_capture(&mut self) {
        #[cfg(target_os = "macos")]
        {
            self.capture_generation = self.capture_generation.wrapping_add(1);
            self.submission_capture = None;
        }
    }

    #[cfg(target_os = "macos")]
    fn capture_target(&self) -> Option<(i64, i64)> {
        let Screen::Item(course) = self.screen else { return None };
        let item = self.item_id.as_deref()?.parse().ok()?;
        self.can_add_submission_drive_file(course, item).then_some((course, item))
    }

    #[cfg(target_os = "macos")]
    fn update_capture(&mut self, action: CaptureAction) -> Task<Message> {
        match action {
            CaptureAction::StartScreen if self.submission_capture.is_none() && self.capture_target().is_some()
                && !self.has_pending_request() =>
            {
                let generation = self.capture_generation;
                let (task, handle) = Task::perform(screen_recording::capture(),
                    move |result| Message::Capture(CaptureAction::MovieCaptured(generation, "Screen recording", result))).abortable();
                self.submission_capture = Some(CaptureState::RecordingScreen { _handle: handle.abort_on_drop() });
                self.error = None;
                self.notice = None;
                return task;
            }
            CaptureAction::StartCamera if self.submission_capture.is_none() && self.capture_target().is_some()
                && !self.has_pending_request() =>
            {
                let generation = self.capture_generation;
                self.submission_capture = Some(CaptureState::AskingCamera);
                self.error = None;
                self.notice = None;
                let permission = camera::request_permission();
                return Task::perform(async move { permission.await.unwrap_or(false) },
                    move |allowed| Message::Capture(CaptureAction::CameraPermission(generation, allowed)));
            }
            CaptureAction::CameraPermission(generation, allowed) if generation == self.capture_generation
                && matches!(self.submission_capture, Some(CaptureState::AskingCamera)) =>
            {
                if !allowed || self.capture_target().is_none() {
                    self.submission_capture = None;
                    if !allowed {
                        self.error = Some("Camera access is off. Allow Classroom for Mac in System Settings under Privacy & Security → Camera.".into());
                    }
                    return Task::none();
                }
                self.submission_capture = None;
                match camera::start() {
                    Ok((control, result)) => {
                        self.submission_capture = Some(CaptureState::RecordingCamera { control, phase: CameraPhase::Framing, frame: None });
                        return Task::perform(async move { result.await.unwrap_or_else(|_| Err("The camera worker stopped unexpectedly.".into())) },
                            move |result| Message::Capture(CaptureAction::MovieCaptured(generation, "Camera recording", result)));
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            CaptureAction::CameraFrame => {
                if let Some(CaptureState::RecordingCamera { control, frame, .. }) = &mut self.submission_capture
                    && let Some((width, height, pixels)) = control.take_frame()
                {
                    *frame = Some(image::Handle::from_rgba(width, height, pixels));
                }
            }
            CaptureAction::StartCameraRecording if self.capture_target().is_some() => {
                if let Some(CaptureState::RecordingCamera { phase, frame: Some(_), .. }) = &mut self.submission_capture
                    && *phase == CameraPhase::Framing
                {
                    *phase = CameraPhase::AskingMicrophone;
                    self.error = None;
                    let generation = self.capture_generation;
                    let permission = audio::request_permission();
                    return Task::perform(async move { permission.await.unwrap_or(false) },
                        move |allowed| Message::Capture(CaptureAction::CameraMicrophonePermission(generation, allowed)));
                }
            }
            CaptureAction::CameraMicrophonePermission(generation, allowed) if generation == self.capture_generation => {
                if let Some(CaptureState::RecordingCamera { control, phase, .. }) = &mut self.submission_capture
                    && *phase == CameraPhase::AskingMicrophone
                {
                    if allowed {
                        *phase = CameraPhase::Recording;
                        control.begin();
                    } else {
                        *phase = CameraPhase::Framing;
                        self.error = Some("Microphone access is off. Allow Classroom for Mac in System Settings under Privacy & Security → Microphone.".into());
                    }
                }
            }
            CaptureAction::StopCamera => {
                if let Some(CaptureState::RecordingCamera { control, phase, .. }) = &mut self.submission_capture
                    && *phase == CameraPhase::Recording
                {
                    *phase = CameraPhase::Finishing;
                    control.stop();
                }
            }
            CaptureAction::MovieCaptured(generation, label, result) if generation == self.capture_generation
                && matches!(self.submission_capture, Some(CaptureState::RecordingScreen { .. } | CaptureState::RecordingCamera { .. })) =>
            {
                self.submission_capture = None;
                match result {
                    Ok(Some(draft)) => self.submission_capture = Some(CaptureState::MovieDraft { label, draft }),
                    Ok(None) => {}
                    Err(error) => self.error = Some(error),
                }
            }
            CaptureAction::Start if self.submission_capture.is_none() && self.capture_target().is_some()
                && !self.has_pending_request() =>
            {
                let permission = audio::request_permission();
                let generation = self.capture_generation;
                self.submission_capture = Some(CaptureState::AskingPermission);
                self.error = None;
                self.notice = None;
                return Task::perform(async move { permission.await.unwrap_or(false) },
                    move |allowed| Message::Capture(CaptureAction::Permission(generation, allowed)));
            }
            CaptureAction::Permission(generation, allowed) if generation == self.capture_generation
                && matches!(self.submission_capture, Some(CaptureState::AskingPermission)) =>
            {
                self.submission_capture = None;
                if self.capture_target().is_none() { return Task::none(); }
                if !allowed {
                    self.error = Some("Microphone access is off. Allow Classroom for Mac in System Settings under Privacy & Security → Microphone.".into());
                    return Task::none();
                }
                match audio::Recording::start() {
                    Ok(recording) => {
                        let (limit, handle) = Task::perform(tokio::time::sleep(audio::MAX_DURATION),
                            move |()| Message::Capture(CaptureAction::TimeLimit(generation))).abortable();
                        self.submission_capture = Some(CaptureState::Recording(recording, handle.abort_on_drop()));
                        return limit;
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            CaptureAction::TimeLimit(generation) if generation == self.capture_generation => {
                return self.update_capture(CaptureAction::Stop);
            }
            CaptureAction::Stop if matches!(self.submission_capture, Some(CaptureState::Recording(..))) => {
                if let Some(CaptureState::Recording(recording, _limit)) = self.submission_capture.take() {
                    match recording.finish() {
                        Ok(draft) => self.submission_capture = Some(CaptureState::Draft(draft)),
                        Err(error) => self.error = Some(error),
                    }
                }
            }
            CaptureAction::Preview if !self.sending_submission => {
                if let Some(CaptureState::MovieDraft { draft, .. }) = &self.submission_capture {
                    let generation = self.capture_generation;
                    return Task::perform(draft.clone().preview(),
                        move |result| Message::Capture(CaptureAction::ScreenPreviewFinished(generation, result)));
                }
                if let Some(CaptureState::Draft(draft)) = &self.submission_capture
                    && let Err(error) = draft.play_preview()
                {
                    self.error = Some(error);
                }
            }
            CaptureAction::ScreenPreviewFinished(generation, Err(error)) if generation == self.capture_generation => {
                self.error = Some(error);
            }
            CaptureAction::StopPreview => {
                if let Some(CaptureState::Draft(draft)) = &self.submission_capture {
                    draft.stop_preview();
                }
            }
            CaptureAction::Discard if !self.sending_submission => self.discard_submission_capture(),
            CaptureAction::Attach if !self.has_pending_request() => {
                if let (Some((course, item)), Some((keep_alive, path)), Some(session)) =
                    (self.capture_target(), self.submission_capture.as_ref().and_then(CaptureState::recording_file),
                        self.session.clone())
                {
                    let generation = self.capture_generation;
                    self.sending_submission = true;
                    self.error = None;
                    self.notice = None;
                    return Task::perform(async move {
                        let result = desktop_auth::attach_submission_file(&session, course, item, path).await;
                        drop(keep_alive);
                        result
                    }, move |result| Message::Capture(CaptureAction::Attached(course, generation, result)));
                }
            }
            CaptureAction::Attached(course, generation, result) => {
                self.sending_submission = false;
                match result {
                    Ok(()) => {
                        if self.capture_generation == generation {
                            self.discard_submission_capture();
                        }
                        self.notice = Some("Recording attached to your work.".into());
                    }
                    Err(error) => self.error = Some(error),
                }
                return self.update(Message::LoadDetail(DetailRead::Submissions(course)));
            }
            _ => {}
        }
        Task::none()
    }

    #[cfg(target_os = "macos")]
    fn submission_capture_controls(&self, ready: bool) -> Element<'_, Message> {
        let capture_button = |label: &'static str, action, enabled: bool| {
            button(label).style(outline_style).padding([7, 10])
                .on_press_maybe(enabled.then_some(Message::Capture(action)))
        };
        let discard = capture_button("Discard", CaptureAction::Discard, !self.sending_submission);
        match &self.submission_capture {
            None => row![
                capture_button("Record audio", CaptureAction::Start, ready),
                capture_button("Record video", CaptureAction::StartCamera, ready),
                capture_button("Record screen", CaptureAction::StartScreen, ready),
            ].spacing(8).wrap().vertical_spacing(8).into(),
            Some(CaptureState::AskingPermission) => row![
                text("Waiting for microphone access…").size(14).color(MUTED), discard,
            ].spacing(8).align_y(Alignment::Center).into(),
            Some(CaptureState::AskingCamera) => row![
                text("Waiting for camera access…").size(14).color(MUTED), discard,
            ].spacing(8).align_y(Alignment::Center).wrap().vertical_spacing(8).into(),
            Some(CaptureState::Recording(..)) => row![
                text("Recording audio · 5 min limit").size(14).color(FG),
                capture_button("Stop recording", CaptureAction::Stop, true), discard,
            ].spacing(8).align_y(Alignment::Center).wrap().vertical_spacing(8).into(),
            Some(CaptureState::RecordingScreen { .. }) => row![
                text("Select an area in the macOS toolbar. Use the menu bar to stop.").size(14).color(FG),
                discard,
            ].spacing(8).align_y(Alignment::Center).wrap().vertical_spacing(8).into(),
            Some(CaptureState::RecordingCamera { phase, frame, .. }) => {
                let preview: Element<'_, Message> = match frame {
                    Some(frame) => image(frame.clone()).width(Fill).height(220).content_fit(ContentFit::Contain).into(),
                    None => container(text("Opening camera…").size(14)).center_x(Fill).center_y(220).into(),
                };
                let (status, action) = match phase {
                    CameraPhase::Framing => ("Camera preview", capture_button("Start recording", CaptureAction::StartCameraRecording, frame.is_some())),
                    CameraPhase::AskingMicrophone => ("Waiting for microphone access…", capture_button("Start recording", CaptureAction::StartCameraRecording, false)),
                    CameraPhase::Recording => ("Recording video and audio · 5 min limit", capture_button("Stop recording", CaptureAction::StopCamera, true)),
                    CameraPhase::Finishing => ("Saving camera recording…", capture_button("Stop recording", CaptureAction::StopCamera, false)),
                };
                column![
                    preview,
                    row![text(status).size(14), action, discard].spacing(8).align_y(Alignment::Center).wrap().vertical_spacing(8),
                ].spacing(6).into()
            }
            Some(CaptureState::Draft(draft)) => {
                let seconds = draft.duration.as_secs();
                column![
                    text(format!("Audio recording · {}:{:02}", seconds / 60, seconds % 60)).size(14),
                    row![
                        capture_button("Play preview", CaptureAction::Preview, !self.sending_submission),
                        capture_button("Stop preview", CaptureAction::StopPreview, draft.is_playing()),
                        capture_button("Attach recording", CaptureAction::Attach, ready),
                        discard,
                    ].spacing(8).wrap().vertical_spacing(8),
                ].spacing(6).into()
            }
            Some(CaptureState::MovieDraft { label, .. }) => column![
                text(*label).size(14),
                row![
                    capture_button("Preview in QuickTime", CaptureAction::Preview, !self.sending_submission),
                    capture_button("Attach recording", CaptureAction::Attach, ready),
                    discard,
                ].spacing(8).wrap().vertical_spacing(8),
            ].spacing(6).into(),
        }
    }

    fn can_use_drive_picker(&self, course: i64) -> bool {
        self.drive_picker.as_ref().is_some_and(|picker| {
            picker.course == course && match picker.item {
                Some(item) => self.can_add_submission_drive_file(course, item),
                None => self.can_edit_stream_attachments(course),
            }
        })
    }

    fn drive_picker_panel<'a>(
        &'a self, picker: &'a DrivePicker, ready: bool, already_attached: impl Fn(&DriveFileAttachment) -> bool,
    ) -> Element<'a, Message> {
        let course = picker.course;
        let placeholder = if !picker.folder_path.is_empty() || picker.section == DriveSection::MyDrive {
            "Search this folder"
        } else {
            "Search Drive by name"
        };
        let search = text_input(placeholder, &picker.query).width(Fill).padding(7)
            .on_input(move |query| Message::EditDriveSearch(course, query))
            .on_submit_maybe(ready.then_some(Message::SearchDriveFiles(course, false)));
        let mut panel = column![
            row![
                text("Google Drive").size(15).font(medium()).color(FG).width(Fill),
                button("Close").style(quiet_style).padding([4, 7]).on_press(Message::CloseDrivePicker),
            ].align_y(Alignment::Center),
            row([DriveSection::Recent, DriveSection::MyDrive, DriveSection::Shared].into_iter().map(|section| {
                let selected = picker.section == section;
                button(section.label()).padding([4, 7]).style(move |_, status| nav_style(selected, status))
                    .on_press_maybe((ready && (!selected || !picker.folder_path.is_empty()))
                        .then_some(Message::BrowseDrive(course, DriveNavigation::Section(section)))).into()
            })).spacing(5).wrap().vertical_spacing(5),
            row![
                search,
                button("Search").style(outline_style).padding([6, 8])
                    .on_press_maybe(ready.then_some(Message::SearchDriveFiles(course, false))),
            ].spacing(6).align_y(Alignment::Center),
        ].spacing(7);
        if let Some(folder) = picker.folder_path.last() {
            panel = panel.push(row![
                button("‹ Back").style(quiet_style).padding([4, 6])
                    .on_press_maybe(ready.then_some(Message::BrowseDrive(course, DriveNavigation::Back))),
                text(truncate(&folder.name, 55)).size(14).font(medium()).color(FG).width(Fill),
            ].spacing(6).align_y(Alignment::Center));
        }
        if let Some((append, error)) = &picker.error {
            panel = panel.push(row![
                text(error).size(13).color(color!(0x9b2722)).width(Fill),
                button("Retry").style(quiet_style).padding([4, 7])
                    .on_press_maybe((ready && !picker.loading && picker.query.trim() == picker.submitted_query)
                        .then_some(Message::SearchDriveFiles(course, *append))),
            ].spacing(7).align_y(Alignment::Center));
        }
        if let Some(page) = &picker.file_page {
            let mut rows = column![].spacing(4);
            for folder in &page.folders {
                rows = rows.push(button(row![
                    icon(Icon::Folder, BLUE, 16),
                    text(&folder.name).size(14).color(FG).width(Fill).wrapping(text::Wrapping::WordOrGlyph),
                    icon(Icon::Arrow, MUTED, 14),
                ].spacing(7).align_y(Alignment::Center)).padding([5, 8]).width(Fill).style(quiet_style)
                    .on_press_maybe(ready.then(|| Message::BrowseDrive(course, DriveNavigation::OpenFolder(folder.id.clone())))));
            }
            for file in &page.files {
                let attached = already_attached(file);
                rows = rows.push(row![
                    text(file.name()).size(14).color(FG).width(Fill).wrapping(text::Wrapping::WordOrGlyph),
                    button(if attached { "Added" } else { "Attach" }).style(quiet_style).padding([5, 8])
                        .on_press_maybe((ready && !attached).then(|| Message::AttachExistingDriveFile(course, file.id().into()))),
                ].spacing(7).align_y(Alignment::Center));
            }
            if page.files.is_empty() && page.folders.is_empty() && !picker.loading && picker.error.is_none() {
                rows = rows.push(text("No matching files or folders.").size(13).color(MUTED));
            }
            let mut list = scrollable(container(rows).padding(Padding { right: 12.0, ..Padding::ZERO })).height(125);
            if ready && picker.can_auto_page() {
                let generation = self.drive_picker_generation;
                list = list.on_scroll(move |viewport| {
                    let remaining = viewport.content_bounds().height - viewport.bounds().height - viewport.absolute_offset().y;
                    Message::DriveScrolled(course, generation, remaining <= 60.0)
                });
            }
            panel = panel.push(list);
            if page.incomplete {
                panel = panel.push(text("Drive reported incomplete search results.").size(13).color(MUTED));
            }
        }
        if picker.loading {
            panel = panel.push(text("Checking Drive…").size(13).color(MUTED));
        }
        container(panel).padding(10).width(Fill).style(|_| outline_surface(8.0)).into()
    }

    fn set_demo_stream_display(&mut self, display: StreamClassworkDisplay) {
        if let Some(course) = self.snapshot.as_mut().and_then(|snapshot| snapshot.courses.iter_mut().find(|course| course.id == 1)) {
            course.stream_classwork_display = Some(display);
        }
    }

    fn stream_items(&self, course: &Course) -> Element<'_, Message> {
        let course_id = course.id;
        let display = course.stream_classwork_display;
        let mut section = column![].spacing(13);
        let items = self.snapshot.as_ref().and_then(|data| data.coursework_for(course_id));
        if let Some(status) = self.page_read_status(
            PageRead::Coursework(course_id), items.is_some(), "Loading classwork…",
        ) {
            section = section.push(status);
        }
        if let Some(drafts) = self.teacher_draft_panel(course_id, true) {
            section = section.push(drafts);
        }
        match items {
            Some(items) => {
                let mut visible = items.iter().filter(|item| display.is_none_or(|display| display.shows(item.kind))).peekable();
                if visible.peek().is_none() {
                    let title = if display == Some(StreamClassworkDisplay::Hidden) { "No announcements yet" } else { "Nothing posted yet" };
                    section.push(empty_panel(Icon::Feed, title, "Class posts will appear here."))
                } else {
                    section.extend(visible.map(|item| {
                        if item.kind == CourseworkKind::Announcement || display.is_none() {
                            self.stream_row(item)
                        } else {
                            self.stream_classwork_notice(item, display == Some(StreamClassworkDisplay::Expanded))
                        }
                    }))
                }
            }
            None if self.can_read_pages() => section,
            None => section.push(empty_panel(Icon::Feed, "Stream unavailable", "This offline copy has no class posts.")),
        }.width(Fill).into()
    }

    fn stream_classwork_notice<'a>(&'a self, item: &'a Coursework, expanded: bool) -> Element<'a, Message> {
        let row = coursework_row_with_preview(item, None, false);
        if !expanded || (item.description.trim().is_empty() && item.attachments.is_empty() && item.due_label.is_none()) {
            return row;
        }
        let mut details = column![].spacing(8);
        if let Some(due) = &item.due_label {
            details = details.push(text(due).size(13).color(MUTED));
        }
        if !item.description.trim().is_empty() {
            details = details.push(formatted_description(&item.description, &item.description_styles, &item.description_bullets));
        }
        for attachment in &item.attachments {
            let action = if attachment.drive_sharing == Some(DriveSharing::IndividualCopy) {
                Some(Message::OpenItem(item.course_id, item.id.clone()))
            } else {
                self.attachment_action(attachment)
            };
            details = details.push(button(row![
                icon(Icon::Book, BLUE, 16),
                text(&attachment.title).size(14).color(FG).width(Fill),
                icon(Icon::Arrow, MUTED, 14),
            ].spacing(8).align_y(Alignment::Center)).padding([7, 10]).width(Fill)
                .style(outline_style).on_press_maybe(action));
        }
        container(column![row, container(details).padding([3, 14]).width(Fill)].spacing(4))
            .width(Fill).padding(Padding { bottom: 11.0, ..Padding::ZERO }).style(|_| outline_surface(9.0)).into()
    }

    fn attachment_action(&self, attachment: &desktop_data::Attachment) -> Option<Message> {
        if self.verify_live_course_count {
            return None;
        }
        let url = attachment.url.as_deref().filter(|url| trusted_attachment_host(url).is_some());
        if let Some(id) = attachment.drive_reference.as_deref()
            && attachment.kind == desktop_data::AttachmentKind::DriveFile
        {
            if let Some(url) = url.filter(|url| desktop_auth::drive_url_reference(url).as_deref() == Some(id)) {
                return Some(Message::OpenAttachment(url.to_owned()));
            }
            return (self.can_read_pages() && self.opening_drive_file.is_none())
                .then(|| Message::OpenDriveFile(id.to_owned()));
        }
        url.map(|url| Message::OpenAttachment(url.to_owned()))
    }

    fn apply_teacher_topics(&mut self, course: i64, topics: Vec<desktop_data::Topic>, removed: Option<&str>) {
        if let Some(removed) = removed
            && let Some(drafts) = self.teacher_drafts.get_mut(&course)
        {
            for draft in drafts.iter_mut().filter(|draft| draft.item.topic_id.as_deref() == Some(removed)) {
                draft.item.topic_id = None;
            }
        }
        if let Some(snapshot) = &mut self.snapshot
            && let Some(items) = snapshot.coursework.get_mut(&course)
        {
            if let Some(removed) = removed {
                for item in items.iter_mut().filter(|item| item.topic_id.as_deref() == Some(removed)) {
                    item.topic_id = None;
                }
                if self.screen == Screen::Course(course) && self.selected_topic.as_deref() == Some(removed) {
                    self.selected_topic = None;
                }
            }
            let published_ids: BTreeSet<_> = items.iter().filter_map(|item| item.topic_id.as_deref()).collect();
            snapshot.topics.insert(course, topics.iter().filter(|topic| published_ids.contains(topic.id.as_str())).cloned().collect());
        }
        self.teacher_topics.insert(course, topics);
    }

    fn can_manage_topics(&self, course: i64) -> bool {
        self.can_edit_assignment_drafts(course)
    }

    fn apply_item_topic(&mut self, target: &desktop_auth::TopicItemTarget, topic: Option<String>) {
        let matching = |item: &Coursework| item.kind == target.kind && item.id.parse::<i64>().ok() == Some(target.item);
        if target.draft {
            if let Some(row) = self.teacher_drafts.get_mut(&target.course)
                .and_then(|drafts| drafts.iter_mut().find(|row| matching(&row.item)))
            {
                row.item.topic_id = topic;
            }
        } else if let Some(item) = self.snapshot.as_mut().and_then(|snapshot| snapshot.coursework.get_mut(&target.course))
            .and_then(|items| items.iter_mut().find(|item| matching(item)))
        {
            item.topic_id = topic;
            item.classwork_sort_key = None;
            if let Some(topics) = self.teacher_topics.get(&target.course).cloned() {
                self.apply_teacher_topics(target.course, topics, None);
            }
        }
    }

    fn item_topic_button<'a>(&'a self, item: &'a Coursework, draft: bool) -> Option<Element<'a, Message>> {
        if item.kind == CourseworkKind::Announcement || item.id.parse::<i64>().is_err()
            || !self.teacher_topics.contains_key(&item.course_id)
            || !(self.can_manage_topics(item.course_id)
                || self.screenshot_directory.is_some() && self.class_roles.get(&item.course_id) == Some(&CourseRole::Teacher))
        {
            return None;
        }
        Some(button("Change topic…").style(outline_style).padding([7, 10])
            .on_press_maybe((!self.has_pending_request() && self.item_topic_editor.is_none())
                .then(|| Message::OpenItemTopic(item.course_id, item.id.clone(), draft))).into())
    }

    fn item_topic_editor_panel<'a>(&'a self, editor: &'a ItemTopicEditor) -> Element<'a, Message> {
        let choices: Vec<_> = std::iter::once(TopicChoice { id: None, label: "No topic".into() })
            .chain(self.teacher_topics.get(&editor.target.course).into_iter().flatten()
                .map(|topic| TopicChoice { id: Some(topic.id.clone()), label: topic.title.clone() })).collect();
        let selected = choices.iter().find(|choice| choice.id == editor.selected).cloned();
        let ready = !self.has_pending_request() && selected.is_some() && editor.selected != editor.target.original;
        container(column![
            text("Change topic").size(17).font(medium()),
            text(&editor.title).size(14),
            pick_list(choices, selected, Message::SelectItemTopic).placeholder("Choose a topic").padding(8),
            row![
                button(if self.saving_topic { "Saving…" } else { "Save" }).style(primary_style).padding([7, 10])
                    .on_press_maybe(ready.then_some(Message::SaveItemTopic)),
                button("Cancel").style(quiet_style).padding([7, 10])
                    .on_press_maybe((!self.saving_topic).then_some(Message::CloseItemTopic)),
            ].spacing(8),
        ].spacing(8)).padding(12).width(Fill).style(|_| outline_surface(9.0)).into()
    }

    fn teacher_topic_panel(&self, course: i64) -> Option<Element<'_, Message>> {
        use desktop_auth::TeacherTopicAction;
        if !self.can_manage_topics(course)
            && !(self.screenshot_directory.is_some() && self.class_roles.get(&course) == Some(&CourseRole::Teacher))
        {
            return None;
        }
        let topics = self.teacher_topics.get(&course);
        if let Some(status) = self.page_read_status(
            PageRead::Detail(DetailRead::TeacherTopics(course)), topics.is_some(), "Loading topics…",
        ) {
            return Some(status);
        }
        let topics = topics?;
        let ready = !self.has_pending_request();
        let actions = row![
            button("Create topic").style(outline_style).padding([7, 10])
                .on_press_maybe((ready && self.topic_editor.is_none()).then(|| Message::OpenTopicEditor(course, TeacherTopicAction::Create(String::new())))),
            button(if self.topic_manager { "Hide topic manager" } else { "Manage topics" }).style(quiet_style).padding([7, 10])
                .on_press_maybe(ready.then_some(Message::ToggleTopicManager)),
        ].spacing(8).wrap().vertical_spacing(8);
        let mut panel = column![actions].spacing(8);
        if let Some((id, action)) = self.topic_editor.as_ref().filter(|(id, _)| *id == course) {
            let mut editor = column![].spacing(8);
            let label = match action {
                TeacherTopicAction::Create(name) | TeacherTopicAction::Rename { name, .. } => {
                    editor = editor.push(text_input("Topic name", name).padding(8).width(Fill)
                        .on_input_maybe((!self.saving_topic).then_some(Message::EditTopicName)));
                    if matches!(action, TeacherTopicAction::Create(_)) { "Add topic" } else { "Rename topic" }
                }
                TeacherTopicAction::Delete(topic) => {
                    editor = editor.push(text(format!("Delete “{}”?", topic.title)).size(15).font(medium()))
                        .push(text("Posts in this topic move to No topic.").size(13).color(MUTED));
                    "Delete topic"
                }
            };
            editor = editor.push(row![
                button(label).style(primary_style).padding([7, 10])
                    .on_press_maybe((ready && *id == course && topic_action_valid(action)).then_some(Message::SaveTopicEditor)),
                button("Cancel").style(quiet_style).padding([7, 10])
                    .on_press_maybe((!self.saving_topic).then_some(Message::CloseTopicEditor)),
            ].spacing(8));
            panel = panel.push(container(editor).padding(12).width(Fill).style(|_| outline_surface(9.0)));
        }
        if self.topic_manager {
            let mut list = column![text("Topics").size(16).font(medium())].spacing(6);
            if topics.is_empty() {
                list = list.push(text("No topics yet.").size(14).color(MUTED));
            }
            for (index, topic) in topics.iter().enumerate() {
                let available = ready && self.topic_editor.is_none();
                list = list.push(row![
                    text(&topic.title).size(14).width(Fill).wrapping(text::Wrapping::WordOrGlyph),
                    button("Move up").style(quiet_style).padding([5, 7])
                        .on_press_maybe((available && index > 0).then(|| Message::MoveTeacherTopic(course,
                            topic.id.clone(), desktop_auth::TopicOrderDirection::Up))),
                    button("Move down").style(quiet_style).padding([5, 7])
                        .on_press_maybe((available && index + 1 < topics.len()).then(|| Message::MoveTeacherTopic(course,
                            topic.id.clone(), desktop_auth::TopicOrderDirection::Down))),
                    button("Rename…").style(quiet_style).padding([5, 7])
                        .on_press_maybe(available.then(|| Message::OpenTopicEditor(course,
                            TeacherTopicAction::Rename { topic: topic.clone(), name: topic.title.clone() }))),
                    button("Delete…").style(quiet_style).padding([5, 7])
                        .on_press_maybe(available.then(|| Message::OpenTopicEditor(course, TeacherTopicAction::Delete(topic.clone())))),
                ].spacing(5).align_y(Alignment::Center));
            }
            panel = panel.push(container(list).padding(12).width(Fill).style(|_| outline_surface(9.0)));
        }
        Some(panel.into())
    }

    fn classwork(&self, course_id: i64) -> Element<'_, Message> {
        let Some(snapshot) = &self.snapshot else { return space().into() };
        let topics: Vec<_> = if let Some(topics) = self.teacher_topics.get(&course_id)
            && self.class_roles.get(&course_id) == Some(&CourseRole::Teacher)
            && (self.can_read_pages() || self.screenshot_directory.is_some())
        {
            topics.iter().map(|topic| (topic.id.clone(), topic.title.as_str())).collect()
        } else {
            snapshot.classwork_sections(course_id, None).into_iter()
            .filter_map(|section| match section.id {
                ClassworkGroupId::Topic(id) => Some((id, section.title)),
                _ => None,
            }).collect()
        };
        if topics.is_empty() {
            return self.classwork_items(course_id);
        }

        // Use the same topic controls in the class sidebar and the narrow layout.
        responsive(move |size| {
            if size.width >= 650.0 {
                let mut menu = column![self.topic_button("All topics", None, true)].spacing(4).width(155);
                for (id, title) in &topics {
                    menu = menu.push(self.topic_button(title, Some(id), true));
                }
                row![menu, self.classwork_items(course_id)].spacing(18).into()
            } else {
                let choices: Vec<_> = std::iter::once(TopicChoice { id: None, label: "All topics".into() })
                    .chain(topics.iter().map(|(id, title)| TopicChoice {
                        id: Some(id.clone()), label: (*title).to_owned(),
                    })).collect();
                let selected = choices.iter().find(|choice| choice.id == self.selected_topic).cloned();
                column![
                    pick_list(choices, selected, |choice| Message::SelectTopic(choice.id))
                        .placeholder("All topics").padding(8).width(Length::Shrink),
                    self.classwork_items(course_id),
                ].spacing(12).into()
            }
        }).height(Length::Shrink).into()
    }

    fn classwork_items(&self, course_id: i64) -> Element<'_, Message> {
        // Render the content independently of the topic control layout.
        let Some(snapshot) = &self.snapshot else { return space().into() };
        let loaded = snapshot.coursework_for(course_id).is_some();
        let status = self.page_read_status(PageRead::Coursework(course_id), loaded, "Loading classwork…");
        let pending = status.is_some();
        let sections = snapshot.classwork_sections(course_id, self.selected_topic.as_deref());
        let mut work = column![].spacing(12).width(Fill);
        if let Some(status) = status {
            work = work.push(status);
        }
        if let Some(drafts) = self.teacher_draft_panel(course_id, false) {
            work = work.push(drafts);
        }
        if self.class_roles.get(&course_id) != Some(&CourseRole::Teacher)
            && snapshot.coursework_for(course_id).is_some_and(|items| items.iter().any(has_submission))
            && let Some(status) = self.page_read_status(
                PageRead::Detail(DetailRead::Submissions(course_id)),
                self.submissions_checked.contains(&course_id), "Loading your work…",
            )
        {
            work = work.push(status);
        }
        if sections.is_empty() && !pending {
            work = work.push(if loaded {
                empty_panel(Icon::Book, "Nothing posted here yet", "Assignments, questions and materials will appear here.")
            } else {
                empty_panel(Icon::Book, "Classwork unavailable", "This offline copy has no classwork.")
            });
        }
        if self.selected_topic.is_none() && !sections.is_empty() {
            let all_closed = sections.iter().all(|section| self.collapsed_classwork_groups.contains(&section.id));
            work = work.push(button(if all_closed { "Expand all" } else { "Collapse all" })
                .style(quiet_style).padding([7, 10]).on_press(Message::SetClassworkGroups(course_id, !all_closed)));
        }
        for section in sections {
            let closed = self.collapsed_classwork_groups.contains(&section.id);
            let reorder = self.can_manage_topics(course_id) && section.confirmed_order
                && matches!(section.id, ClassworkGroupId::Topic(_));
            work = work.push(button(row![
                text(section.title).size(19).font(medium()).color(FG).width(Fill)
                    .wrapping(text::Wrapping::WordOrGlyph),
                text(if closed { "⌄" } else { "⌃" }).size(17).color(MUTED),
            ].spacing(8).align_y(Alignment::Center))
                .width(Fill).style(quiet_style).padding([8, 9])
                .on_press(Message::ToggleClassworkGroup(course_id, section.id)));
            if !closed {
                let count = section.items.len();
                work = work.extend(section.items.into_iter().enumerate().map(|(index, item)| {
                    self.classwork_item(item, reorder && index > 0, reorder && index + 1 < count)
                }));
            }
        }
        work.into()
    }

    fn teacher_draft_panel(&self, course: i64, announcements: bool) -> Option<Element<'_, Message>> {
        use desktop_auth::DraftPublication;
        if self.class_roles.get(&course) != Some(&CourseRole::Teacher)
            || (!self.can_read_pages() && self.screenshot_directory.is_none())
        {
            return None;
        }
        if let Some(status) = self.page_read_status(
            PageRead::Detail(DetailRead::TeacherDrafts(course)),
            self.teacher_drafts.contains_key(&course), "Loading drafts and scheduled posts…",
        ) {
            return Some(status);
        }
        let mut drafts: Vec<_> = self.teacher_drafts.get(&course)?.iter()
            .filter(|draft| (draft.item.kind == CourseworkKind::Announcement) == announcements).collect();
        if drafts.is_empty() {
            return None;
        }
        drafts.sort_by_key(|draft| match draft.publication {
            DraftPublication::Scheduled(time) | DraftPublication::Retrying(time) | DraftPublication::Failed(time) =>
                (0, time.unwrap_or(i64::MAX)),
            DraftPublication::Draft => (1, 0),
        });
        let mut panel = column![text("Drafts and scheduled posts").size(18).font(medium()).color(FG)].spacing(8);
        for draft in drafts {
            let item = &draft.item;
            let (status, time) = match draft.publication {
                DraftPublication::Draft => ("Draft", None),
                DraftPublication::Scheduled(time) => ("Scheduled", time),
                DraftPublication::Retrying(time) => ("Retrying", time),
                DraftPublication::Failed(time) => ("Scheduling failed", time),
            };
            let display = time.and_then(DateTime::from_timestamp_millis)
                .map_or_else(|| status.to_owned(), |date| format!("{status} · {}", date.with_timezone(&Local).format("%-d %b %Y, %H:%M")));
            let open = self.open_teacher_draft.as_ref().is_some_and(|(id, draft)| *id == course && draft == &item.id);
            let heading = row![
                icon(coursework_icon(item.kind), BLUE, 18),
                text(&item.title).size(15).font(medium()).width(Fill),
                text(display).size(13).color(MUTED),
                text(if open { "⌃" } else { "⌄" }).size(16).color(MUTED),
            ].spacing(8).align_y(Alignment::Center);
            let main = button(heading).width(Fill).style(quiet_style).padding([8, 10])
                .on_press(Message::ToggleTeacherDraft(course, item.id.clone()));
            let mut line = row![main].spacing(6).align_y(Alignment::Center);
            if (self.can_edit_assignment_drafts(course) || self.screenshot_directory.is_some() && self.tab == Tab::Classwork)
                && item.kind == CourseworkKind::Assignment && draft.publication == DraftPublication::Draft
            {
                line = line.push(button("Edit").padding([7, 10]).style(outline_style)
                    .on_press_maybe((!self.has_pending_request() && self.assignment_editor.is_none() && self.assignment_publication.is_none())
                        .then(|| Message::OpenAssignmentEditor(course, Some(item.id.clone())))));
            }
            panel = panel.push(line);
            if open && !item.description.is_empty() {
                panel = panel.push(container(formatted_description(&item.description, &item.description_styles, &item.description_bullets))
                    .padding([2, 10]).width(Fill));
            }
            if open && !announcements
                && let Some(control) = self.item_topic_button(item, true)
            {
                panel = panel.push(control);
            }
            if open && item.kind == CourseworkKind::Assignment && self.can_edit_assignment_drafts(course)
                && matches!(draft.publication, DraftPublication::Draft | DraftPublication::Scheduled(_))
            {
                let ready = !self.has_pending_request() && self.assignment_editor.is_none() && self.assignment_publication.is_none();
                let mut actions = row![].spacing(6);
                if draft.publication == DraftPublication::Draft {
                    actions = actions.push(button("Publish now…").style(outline_style).padding([7, 10])
                        .on_press_maybe(ready.then(|| Message::OpenAssignmentPublication(course, item.id.clone(), false))));
                }
                let label = if draft.publication == DraftPublication::Draft { "Schedule…" } else { "Reschedule…" };
                actions = actions.push(button(label).style(outline_style).padding([7, 10])
                    .on_press_maybe(ready.then(|| Message::OpenAssignmentPublication(course, item.id.clone(), true))));
                panel = panel.push(actions);
            }
        }
        Some(container(panel).padding(12).width(Fill).style(|_| outline_surface(9.0)).into())
    }

    fn can_edit_assignment_drafts(&self, course: i64) -> bool {
        self.can_read_pages() && self.screen == Screen::Course(course) && self.tab == Tab::Classwork
            && self.class_roles.get(&course) == Some(&CourseRole::Teacher)
            && self.snapshot.as_ref().is_some_and(|snapshot|
                snapshot.courses.iter().any(|entry| entry.id == course && entry.state == CourseState::Active))
    }

    fn assignment_editor_panel<'a>(&'a self, editor: &'a AssignmentEditor) -> Element<'a, Message> {
        let instructions = editor.instructions_text();
        let fields = editor.field_changes();
        let fields_changed = fields.as_ref().is_ok_and(|changes| changes.due.is_some() || changes.points.is_some());
        let ready = !self.has_pending_request()
            && fields.is_ok()
            && desktop_auth::assignment_draft_text_valid(&editor.title, &instructions)
            && editor.original.as_ref().is_none_or(|draft|
                draft.item.title != editor.title.trim() || draft.item.description != instructions || fields_changed);
        let editing = !self.saving_assignment_draft;
        let instructions_field = if editor.instructions_editable() {
            let field = text_editor(&editor.instructions).placeholder("Instructions (optional)").height(100).padding(9);
            if self.saving_assignment_draft { field } else { field.on_action(Message::EditAssignmentInstructions) }
        } else {
            text_editor(&editor.instructions).height(100).padding(9)
        };
        let mut form = column![
            text(if editor.original.is_some() { "Edit assignment draft" } else { "New assignment draft" }).size(19).font(medium()),
            text_input("Title", &editor.title).padding(9)
                .on_input_maybe((!self.saving_assignment_draft).then_some(Message::EditAssignmentTitle)),
            instructions_field,
        ].spacing(9).width(Fill);
        if !editor.instructions_editable() {
            form = form.push(text("These instructions use formatting and are read-only here.").size(13).color(MUTED));
        }
        form = form.push(row![
            column![
                text("Due date").size(13).color(MUTED),
                text_input("YYYY-MM-DD", &editor.settings.due_date).padding(8)
                    .on_input_maybe(editing.then_some(Message::EditAssignmentDueDate)),
            ].spacing(4).width(Fill),
            column![
                text("Due time").size(13).color(MUTED),
                text_input("HH:MM", &editor.settings.due_time).padding(8)
                    .on_input_maybe(editing.then_some(Message::EditAssignmentDueTime)),
            ].spacing(4).width(Fill),
            column![
                text("Points").size(13).color(MUTED),
                text_input("Ungraded", &editor.settings.points).padding(8)
                    .on_input_maybe(editing.then_some(Message::EditAssignmentPoints)),
            ].spacing(4).width(Fill),
        ].spacing(9));
        if let Err(error) = fields {
            form = form.push(text(error).size(13).color(color!(0x9b2722)));
        }
        form = form.push(row![
            button(if self.saving_assignment_draft { "Saving…" } else { "Save draft" }).padding([8, 12]).style(primary_style)
                .on_press_maybe(ready.then_some(Message::SaveAssignmentDraft)),
            button("Cancel").padding([8, 12]).style(quiet_style)
                .on_press_maybe((!self.saving_assignment_draft).then_some(Message::CloseAssignmentEditor)),
        ].spacing(8));
        container(form).padding(14).width(Fill).style(|_| outline_surface(9.0)).into()
    }

    fn assignment_publication_panel<'a>(&'a self, prompt: &'a AssignmentPublicationPrompt) -> Element<'a, Message> {
        let action = prompt.action();
        let scheduled = prompt.schedule.is_some();
        let mut panel = column![
            text(if scheduled { "Schedule assignment" } else { "Publish assignment" }).size(19).font(medium()),
            text(&prompt.draft.item.title).size(15).color(FG),
        ].spacing(8).width(Fill);
        if let Some(value) = &prompt.schedule {
            panel = panel.push(text("Enter the date and time on this Mac in YYYY-MM-DD HH:MM format.").size(13).color(MUTED))
                .push(text_input("YYYY-MM-DD HH:MM", value).padding(8).width(245)
                    .on_input_maybe((!self.saving_assignment_draft).then_some(Message::EditAssignmentSchedule)));
            if let Some(desktop_auth::AssignmentPublication::Schedule(time)) = action
                && let Some(time) = DateTime::from_timestamp_millis(time)
            {
                panel = panel.push(text(format!("Posts {}", time.with_timezone(&Local).format("%-d %b %Y at %H:%M %Z")))
                    .size(14).color(FG));
            } else if !value.trim().is_empty() {
                panel = panel.push(text("Choose a future local time that differs from the current schedule.").size(13).color(MUTED));
            }
        } else {
            panel = panel.push(text("Students will be able to see this assignment immediately.").size(14).color(MUTED));
        }
        let label = if self.saving_assignment_draft { "Saving…" } else if scheduled { "Schedule" } else { "Publish now" };
        panel = panel.push(row![
            button(label).padding([8, 12]).style(primary_style)
                .on_press_maybe((action.is_some() && !self.has_pending_request()).then_some(Message::ConfirmAssignmentPublication)),
            button("Cancel").padding([8, 12]).style(quiet_style)
                .on_press_maybe((!self.saving_assignment_draft).then_some(Message::CloseAssignmentPublication)),
        ].spacing(8));
        container(panel).padding(14).width(Fill).style(|_| outline_surface(9.0)).into()
    }

    fn classwork_item_expanded(&self, item: &str) -> bool {
        self.selected_topic.is_some() ^ self.classwork_toggles.contains(item)
    }

    fn classwork_item<'a>(&'a self, item: &'a Coursework, move_up: bool, move_down: bool) -> Element<'a, Message> {
        let expanded = self.classwork_item_expanded(&item.id);
        let deadline: Element<'_, Message> = item.due_label.as_ref()
            .map_or_else(|| space().width(0).into(), |due| text(due).size(13).color(MUTED).into());
        let header = button(row![
            container(icon(coursework_icon(item.kind), ACTIVE_FG, 19)).center_x(36).center_y(36)
                .style(|_| surface(ACTIVE_BG, 18.0)),
            text(&item.title).font(medium()).size(15).width(Fill).wrapping(text::Wrapping::WordOrGlyph),
            deadline,
            text(if expanded { "⌃" } else { "⌄" }).size(17).color(MUTED),
        ].spacing(10).align_y(Alignment::Center)).width(Fill).padding([9, 12])
            .style(quiet_style).on_press(Message::ToggleClassworkItem(item.course_id, item.id.clone()));
        let mut card = column![header];
        if expanded {
            let mut summary = column![].spacing(9);
            if !item.description.trim().is_empty() {
                summary = summary.push(text(truncate(&item.description, 460)).size(14).color(FG)
                    .width(Fill).wrapping(text::Wrapping::WordOrGlyph));
            }
            if let Some(submission) = &item.submission {
                let label = grade_label(submission).unwrap_or_else(|| submission.status.label().into());
                summary = summary.push(text(label).size(13).color(ACTIVE_FG));
            }
            if !item.attachments.is_empty() {
                let count = item.attachments.len();
                let label = if count == 1 { "1 attachment".to_owned() } else { format!("{count} attachments") };
                summary = summary.push(text(label).size(13).color(MUTED));
            }
            let action = match item.kind {
                CourseworkKind::Assignment => "View instructions",
                CourseworkKind::Question => "View question",
                CourseworkKind::Material => "View material",
                CourseworkKind::Announcement => "View post",
            };
            summary = summary.push(button(action).style(outline_style).padding([7, 10])
                .on_press(Message::OpenItem(item.course_id, item.id.clone())));
            if let Some(control) = self.item_topic_button(item, false) {
                summary = summary.push(control);
            }
            if move_up || move_down {
                let ready = !self.has_pending_request() && self.item_topic_editor.is_none();
                let move_button = |label, allowed, direction| button(label).style(quiet_style).padding([7, 10])
                    .on_press_maybe((ready && allowed)
                        .then(|| Message::ReorderClasswork(item.course_id, item.id.clone(), direction)));
                use desktop_auth::ClassworkOrderDirection::{Up, Down};
                summary = summary.push(row![
                    move_button("Move up", move_up, Up), move_button("Move down", move_down, Down),
                ].spacing(7));
            }
            card = card.push(rule::horizontal(1).style(divider_style))
                .push(container(summary).padding([10, 13]));
        }
        container(card).style(|_| outline_surface(9.0)).width(Fill).into()
    }

    fn topic_button<'a>(&self, title: &'a str, topic: Option<&str>, sidebar: bool) -> Element<'a, Message> {
        let active = self.selected_topic.as_deref() == topic;
        button(text(title).size(14).wrapping(text::Wrapping::WordOrGlyph)).padding([8, 12])
            .width(if sidebar { Fill } else { Length::Shrink })
            .style(move |_, status| nav_style(active, status))
            .on_press(Message::SelectTopic(topic.map(str::to_owned)))
            .into()
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

    fn reset_notifications(&mut self) {
        self.notifications_generation = self.notifications_generation.wrapping_add(1);
        self.class_notifications = None;
        self.loading_class_notifications = false;
        self.page_errors.remove(&PageRead::ClassNotifications);
        self.email_notifications = None;
        self.loading_email_notifications = false;
        self.page_errors.remove(&PageRead::EmailNotifications);
    }

    fn class_notification_courses(&self) -> impl Iterator<Item = &Course> {
        self.snapshot.iter().flat_map(|snapshot| &snapshot.courses)
            .filter(|course| course.state == CourseState::Active)
    }

    fn can_set_class_notifications(&self, id: i64, enabled: bool) -> bool {
        self.screen == Screen::Settings && self.can_read_pages() && !self.has_pending_request()
            && !self.loading_class_notifications
            && self.email_notifications.as_ref().and_then(|settings| settings.enabled) != Some(false)
            && self.class_notification_courses().any(|course| course.id == id)
            && self.class_notifications.as_ref().and_then(|map| map.get(&id)) == Some(&enabled)
    }

    fn can_set_email_notification(&self, target: EmailNotificationTarget, enabled: bool) -> bool {
        self.screen == Screen::Settings && self.can_read_pages() && !self.has_pending_request()
            && !self.loading_email_notifications
            && self.email_notifications.as_ref().is_some_and(|settings| {
                (target == EmailNotificationTarget::AllEmail || settings.enabled == Some(true))
                    && settings.value(target) == Some(!enabled)
            })
    }

    fn email_notification_row(
        &self, label: &str, target: EmailNotificationTarget, enabled: Option<bool>,
    ) -> Element<'static, Message> {
        let control = match enabled {
            Some(enabled) => {
                let ready = self.can_set_email_notification(target, !enabled) || self.screenshot_directory.is_some();
                let saving = self.saving_email_notification == Some(target);
                notification_switch(enabled, saving, ready, move |value| Message::SetEmailNotification(target, value))
            }
            None => text("Unavailable").size(14).color(MUTED).into(),
        };
        row![text(label.to_owned()).size(15).color(FG).width(Fill), control]
            .spacing(12).align_y(Alignment::Center).into()
    }

    fn email_notification_panel(&self) -> Element<'_, Message> {
        let mut panel = column![text("Email notifications").size(20).font(medium()).color(FG)]
            .spacing(10).width(Fill);
        if let Some(status) = self.page_read_status(
            PageRead::EmailNotifications, self.email_notifications.is_some(), "Loading email notifications…",
        ) {
            panel = panel.push(status);
        }
        if let Some(settings) = &self.email_notifications {
            panel = panel.push(self.email_notification_row(
                "Allow email notifications", EmailNotificationTarget::AllEmail, settings.enabled,
            ));
            if settings.enabled == Some(true) {
                for category in &settings.categories {
                    panel = panel.push(rule::horizontal(1).style(divider_style));
                    if let Some(label) = &category.label {
                        panel = panel.push(text(label).size(14).font(medium()).color(MUTED));
                    }
                    for group in &category.groups {
                        panel = panel.push(self.email_notification_row(
                            &group.label, EmailNotificationTarget::Group(group.tag), group.enabled,
                        ));
                    }
                }
            }
        } else if !self.can_read_pages() {
            panel = panel.push(text("Sign in to manage email notifications.").size(14).color(MUTED));
        }
        container(panel).padding(22).width(Fill).style(|_| outline_surface(12.0)).into()
    }

    fn settings(&self) -> Element<'_, Message> {
        let mut section = column![
            text("Class notifications").size(20).font(medium()).color(FG),
            text("Choose which classes can send you notifications.").size(14).color(MUTED),
        ].spacing(10).width(Fill);
        if let Some(status) = self.page_read_status(
            PageRead::ClassNotifications, self.class_notifications.is_some(), "Loading class notifications…",
        ) {
            section = section.push(status);
        }
        if let Some(settings) = &self.class_notifications {
            if self.email_notifications.as_ref().and_then(|settings| settings.enabled) == Some(false) {
                section = section.push(text("Turn on email notifications to change these settings.").size(14).color(MUTED));
            }
            let mut courses = self.class_notification_courses().peekable();
            if courses.peek().is_none() {
                section = section.push(text("No active classes.").size(14).color(MUTED));
            }
            for course in courses {
                let id = course.id;
                let control: Element<'_, Message> = match settings.get(&id) {
                    Some(muted) => {
                        let enabled = !muted;
                        let ready = self.can_set_class_notifications(id, !enabled) || self.screenshot_directory.is_some();
                        notification_switch(enabled, self.saving_class_notifications == Some(id), ready,
                            move |value| Message::SetClassNotifications(id, value))
                    }
                    None => text("Unavailable").size(14).color(MUTED).into(),
                };
                section = section.push(rule::horizontal(1).style(divider_style)).push(row![
                    avatar(first_initial(&course.title), course_color(course), 28.0, 12.0),
                    column![
                        text(&course.title).size(15).color(FG),
                        text(&course.subtitle).size(13).color(MUTED),
                    ].spacing(3).width(Fill),
                    control,
                ].spacing(12).align_y(Alignment::Center));
            }
        } else if !self.can_read_pages() {
            section = section.push(text("Sign in to manage class notifications.").size(14).color(MUTED));
        }
        let body = column![
            text("Settings").size(27).color(FG),
            self.email_notification_panel(),
            container(section).padding(22).width(Fill).style(|_| outline_surface(12.0)),
        ].spacing(18).max_width(780);
        scrollable(container(body).center_x(Fill).padding(25)).height(Fill).into()
    }

    fn can_change_student_mute(&self, course: i64, student: i64, current: bool) -> bool {
        self.tab == Tab::People && self.can_edit_class_stream_settings(course)
            && self.student_muting.get(&course).and_then(|states| states.get(&student)) == Some(&current)
            && self.snapshot.as_ref().and_then(|snapshot| snapshot.roster_for(course)).is_some_and(|members|
                members.iter().any(|member| member.role == CourseRole::Student && member.id.parse::<i64>().ok() == Some(student)))
    }

    fn people(&self, id: i64) -> Element<'_, Message> {
        let roster = self.snapshot.as_ref().and_then(|snapshot| snapshot.roster_for(id));
        let mut body = column![].spacing(14);
        if let Some(status) = self.page_read_status(
            PageRead::Detail(DetailRead::Roster(id)), roster.is_some(), "Loading people…",
        ) {
            body = body.push(status);
        }
        let teacher = self.class_roles.get(&id) == Some(&CourseRole::Teacher);
        let moderator = teacher && (self.can_edit_class_stream_settings(id) || self.screenshot_directory.is_some());
        if moderator && roster.is_some()
            && let Some(status) = self.page_read_status(
                PageRead::Detail(DetailRead::StudentMuting(id)), self.student_muting.contains_key(&id), "Loading student Stream permissions…",
            )
        {
            body = body.push(status);
        }
        if let Some(members) = roster.filter(|members| !members.is_empty()) {
            for (role, label) in [
                (CourseRole::Teacher, "Teachers"),
                (CourseRole::Student, if teacher { "Students" } else { "Classmates" }),
                (CourseRole::Unknown, "Other members"),
            ] {
                let mut matching = members.iter().filter(|member| member.role == role).peekable();
                if matching.peek().is_none() {
                    continue;
                }
                body = body.push(text(label).font(medium()).size(18).color(FG));
                let mut group = column![].spacing(0);
                for member in matching {
                    let mut line = row![
                        avatar(first_initial(&member.display_name), BLUE, 36.0, 15.0),
                        text(&member.display_name).size(15).color(FG),
                        space().width(Fill),
                        text(member.role.label()).size(13).color(MUTED),
                    ].spacing(12).align_y(Alignment::Center).padding(12);
                    if moderator && member.role == CourseRole::Student
                        && let Ok(student) = member.id.parse::<i64>()
                        && let Some(&muted) = self.student_muting.get(&id).and_then(|rows| rows.get(&student))
                    {
                        if muted {
                            line = line.push(text("Muted").size(13).color(MUTED));
                        }
                        if self.pending_student_mute == Some((id, student, muted)) {
                            group = group.push(line).push(column![
                                text(if muted { "Restore this student’s Stream access?" } else { "Mute this student in the Stream?" })
                                    .size(14).color(FG),
                                text("The class posting or commenting setting still applies. Private feedback stays available.")
                                    .size(13).color(MUTED),
                                row![
                                    button(if muted { "Unmute" } else { "Mute" }).padding([7, 10]).style(primary_style)
                                        .on_press_maybe((!self.has_pending_request()).then_some(Message::ChangeStudentMute(id, student, muted))),
                                    button("Cancel").padding([7, 10]).style(quiet_style).on_press(Message::CancelStudentMute),
                                ].spacing(8),
                            ].spacing(8).padding(Padding { top: 0.0, left: 12.0, bottom: 12.0, right: 12.0 }));
                            continue;
                        }
                        line = line.push(button(if muted { "Unmute…" } else { "Mute…" }).padding([7, 10]).style(outline_style)
                            .on_press_maybe((!self.has_pending_request()).then_some(Message::ConfirmStudentMute(id, student, muted))));
                    }
                    group = group.push(line);
                }
                body = body.push(container(group).style(|_| outline_surface(9.0)).width(Fill));
            }
        } else if roster.is_some() {
            body = body.push(empty_panel(Icon::People, "No people returned", "No roster entries were available to this account."));
        } else if !self.can_read_pages() {
            body = body.push(empty_panel(
                Icon::People, "Roster unavailable", "This copy has no roster data.",
            ));
        }
        body.width(Fill).into()
    }

    fn your_work(&self, id: i64) -> Element<'_, Message> {
        let items = self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(id));
        let filters = row(WorkFilter::ALL.into_iter().map(|filter| {
            let active = self.work_filter == filter;
            button(filter.label()).padding([8, 12]).style(move |_, status| nav_style(active, status))
                .on_press(Message::SetWorkFilter(filter)).into()
        })).spacing(6).wrap().vertical_spacing(6);
        let mut body = column![
            button("‹ Classwork").padding([7, 9]).style(quiet_style).on_press(Message::SetTab(Tab::Classwork)),
            text("Your work").font(medium()).size(21).color(FG),
            filters,
        ].spacing(12);
        if self.snapshot.as_ref().is_some_and(|data| data.source == SnapshotSource::Cache) {
            body = body.push(text("Submission status and grades are available when connected.").size(14).color(MUTED));
        }
        if let Some(Some(grade)) = self.overall_grades.get(&id) {
            let method = match grade.calculation {
                GradeCalculation::TotalPoints => "Total points",
                GradeCalculation::WeightedCategories => "Weighted categories",
            };
            body = body.push(container(column![
                text(format!("Overall grade · {:.2}%", grade.percent)).size(16).font(medium()).color(FG),
                text(method).size(14).color(MUTED),
            ].spacing(5)).padding(12).width(Fill).style(|_| outline_surface(8.0)));
        } else if let Some(status) = self.page_read_status(
            PageRead::Detail(DetailRead::OverallGrade(id)),
            self.overall_grades.contains_key(&id), "Checking for an overall grade…",
        ) {
            body = body.push(status);
        }
        let status = match items {
            None => self.page_read_status(PageRead::Coursework(id), false, "Loading classwork…"),
            Some(items) if items.iter().any(has_submission) => self.page_read_status(
                PageRead::Detail(DetailRead::Submissions(id)),
                self.submissions_checked.contains(&id),
                "Loading your work…",
            ),
            _ => None,
        };
        let pending = status.is_some();
        if let Some(status) = status {
            body = body.push(status);
        }
        let mut visible = false;
        for item in items.unwrap_or_default().iter()
            .filter(|item| has_submission(item) && self.work_filter.includes(item))
        {
            visible = true;
            body = body.push(coursework_row(item, None));
        }
        if !visible && !pending {
            body = body.push(empty_panel(
                Icon::Check, "No matching work", "There are no assignments or questions that match this filter.",
            ));
        }
        body.width(Fill).into()
    }

    fn visible_rubric(&self, course: i64, item: i64) -> Option<&Rubric> {
        if self.screen != Screen::Item(course) || self.item_id.as_deref().and_then(|id| id.parse().ok()) != Some(item) {
            return None;
        }
        self.rubrics.get(&(course, item))?.as_ref()
    }

    fn assignment_rubric<'a>(&'a self, course: i64, item_id: i64, item: &'a Coursework) -> Option<Element<'a, Message>> {
        let key = (course, item_id);
        if let Some(status) = self.page_read_status(
            PageRead::Detail(DetailRead::Rubric(course, item_id)), self.rubrics.contains_key(&key), "Checking for a rubric…",
        ) {
            return Some(status);
        }
        let rubric = self.visible_rubric(course, item_id)?;
        let mut panel = column![button(row![
            text("Rubric").size(19).font(medium()).color(FG),
            space().width(Fill),
            text(if self.rubric_open { "⌃" } else { "⌄" }).size(17).color(MUTED),
        ].spacing(8).align_y(Alignment::Center)).style(quiet_style).width(Fill)
            .padding([8, 10]).on_press(Message::ToggleRubric(course, item_id))].spacing(9);
        if self.rubric_open {
            if let Some(title) = &rubric.title {
                panel = panel.push(text(title).size(16).color(FG));
            }
            if !rubric.criteria.is_empty() {
                let all_open = self.open_criteria.len() == rubric.criteria.len();
                panel = panel.push(button(if all_open { "Collapse criteria" } else { "Expand criteria" })
                    .style(quiet_style).padding([7, 10])
                    .on_press(Message::SetRubricCriteria(course, item_id, !all_open)));
            }
            for (index, criterion) in rubric.criteria.iter().enumerate() {
                let score = item.submission.as_ref().and_then(|submission| {
                    submission.rubric_scores.get(criterion.id.as_ref()?)
                });
                let open = self.open_criteria.contains(&index);
                let mut heading = row![text(&criterion.title).font(medium()).size(15).width(Fill)
                    .wrapping(text::Wrapping::WordOrGlyph)].spacing(8).align_y(Alignment::Center);
                if let Some(points) = score.and_then(|score| score.points) {
                    heading = heading.push(pill(format!("{points} points"), ACTIVE_BG, ACTIVE_FG));
                }
                if !open && let Some(rating_id) = score.and_then(|score| score.rating_id.as_ref())
                    && let Some(rating) = criterion.ratings.iter().find(|rating| rating.id.as_ref() == Some(rating_id))
                    && !rating.title.is_empty()
                {
                    heading = heading.push(text(&rating.title).size(13).color(ACTIVE_FG));
                }
                heading = heading.push(text(if open { "⌃" } else { "⌄" }).size(17).color(MUTED));
                let mut section = column![button(heading).width(Fill).padding([8, 10]).style(quiet_style)
                    .on_press(Message::ToggleRubricCriterion(course, item_id, index))].spacing(6);
                if open {
                    if !criterion.description.is_empty() {
                        section = section.push(text(&criterion.description).size(14).color(FG)
                            .width(Fill).wrapping(text::Wrapping::WordOrGlyph));
                    }
                    for (rank, rating) in criterion.ratings.iter().enumerate() {
                        let selected = rating.id.as_ref().is_some_and(|id| {
                            score.and_then(|score| score.rating_id.as_ref()) == Some(id)
                        });
                        let title = if rating.title.is_empty() { format!("Level {}", rank + 1) } else { rating.title.clone() };
                        let mut level_heading = row![text(title).font(medium()).size(14).width(Fill)]
                            .spacing(8).align_y(Alignment::Center);
                        if let Some(points) = rating.points {
                            level_heading = level_heading.push(text(format!("{points} points")).size(13).color(MUTED));
                        }
                        if selected {
                            level_heading = level_heading.push(pill("Selected level", ACTIVE_BG, ACTIVE_FG));
                        }
                        let mut level = column![level_heading].spacing(6);
                        if !rating.description.is_empty() {
                            level = level.push(text(&rating.description).size(14).width(Fill)
                                .wrapping(text::Wrapping::WordOrGlyph));
                        }
                        section = section.push(container(level).padding(8).width(Fill)
                            .style(move |_| surface(if selected { ACTIVE_BG } else { SOFT }, 7.0)));
                    }
                }
                panel = panel.push(container(section).width(Fill).padding(8).style(|_| outline_surface(8.0)));
            }
        }
        Some(container(panel).width(Fill).padding(9).style(|_| outline_surface(9.0)).into())
    }

    fn can_edit_gradebook(&self, course: i64) -> bool {
        self.can_read_pages() && self.screen == Screen::Course(course) && self.tab == Tab::Grades
            && self.class_roles.get(&course) == Some(&CourseRole::Teacher) && self.can_grade_teacher_work(course)
    }

    fn gradebook_maximum(&self, course: i64, item: i64) -> Option<f64> {
        self.snapshot.as_ref()?.coursework_for(course)?.iter()
            .find(|entry| has_submission(entry) && entry.id.parse::<i64>().ok() == Some(item))?.maximum_points
    }

    fn begin_gradebook_edit(&mut self, course: i64, item: i64, student: i64) -> Task<Message> {
        if !self.can_edit_gradebook(course) || self.gradebook_maximum(course, item).is_none() {
            return Task::none();
        }
        let Some(cell) = self.gradebooks.get(&course).and_then(|grid| grid.cells.get(&(student, item))).filter(|cell| cell.can_edit())
            else { return Task::none(); };
        self.gradebook_edit = Some(GradebookEdit {
            course, item, expected: cell.grading_row(student),
            input: cell.draft_grade.or(cell.released_grade).map_or_else(String::new, |grade| grade.to_string()),
        });
        iced::widget::operation::focus(GRADEBOOK_FIELD)
    }

    fn teacher_gradebook_panel(&self, course: i64) -> Element<'_, Message> {
        const COLUMN_LIMIT: usize = 7;
        const CELL_WIDTH: f32 = 142.0;
        const NAME_WIDTH: f32 = 164.0;
        if !self.can_read_pages() && self.screenshot_directory.is_none() {
            return text("Sign in to view grades.").size(14).color(MUTED).into();
        }
        if self.class_roles.get(&course) != Some(&CourseRole::Teacher) {
            return text("Checking gradebook access…").size(14).color(MUTED).into();
        }
        let items = self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(course));
        if items.is_none()
            && let Some(status) = self.page_read_status(PageRead::Coursework(course), false, "Loading classwork…")
        {
            return status;
        }
        let grid = self.gradebooks.get(&course);
        if let Some(status) = self.page_read_status(
            PageRead::Detail(DetailRead::TeacherGradebook(course)), grid.is_some(), "Loading gradebook…",
        ) {
            return status;
        }
        let Some(grid) = grid else { return space().into(); };
        let columns: Vec<_> = items.unwrap_or_default().iter().filter(|item| has_submission(item))
            .filter_map(|item| Some((item.id.parse::<i64>().ok()?, item))).collect();
        if columns.is_empty() || grid.students.is_empty() {
            return empty_panel(Icon::Check, "No grades to show", "Published work and enrolled students will appear here.");
        }
        let total_columns = columns.len();
        let pages = total_columns.div_ceil(COLUMN_LIMIT);
        let page = self.gradebook_page.min(pages - 1);
        let start = page * COLUMN_LIMIT;
        let stop = (start + COLUMN_LIMIT).min(columns.len());
        let columns = &columns[start..stop];
        let header_cell = |label: String, width| container(text(label).size(14).font(medium())
            .wrapping(text::Wrapping::WordOrGlyph)).padding(8).width(width);
        let mut heading = row![header_cell("Student".into(), NAME_WIDTH)].spacing(2);
        for (_, item) in columns {
            let mut header = column![
                text(&item.title).size(14).font(medium()).wrapping(text::Wrapping::WordOrGlyph),
            ].spacing(4);
            if let Some(points) = item.maximum_points {
                header = header.push(text(format!("{points} points")).size(12).color(MUTED));
            }
            heading = heading.push(container(header).padding(8).width(CELL_WIDTH));
        }
        let mut table = column![container(heading).style(|_| surface(SOFT, 5.0))].spacing(1);
        for student in &grid.students {
            let Ok(id) = student.id.parse::<i64>() else { continue; };
            let mut line = row![container(text(&student.display_name).size(14).wrapping(text::Wrapping::WordOrGlyph))
                .padding(8).width(NAME_WIDTH)].spacing(2).align_y(Alignment::Center);
            for (item, work) in columns {
                if let Some(cell) = grid.cells.get(&(id, *item)) {
                    let draft = cell.draft_grade.filter(|grade| Some(*grade) != cell.released_grade);
                    let label = if let Some(grade) = draft {
                        format!("{grade} · draft")
                    } else if let Some(grade) = cell.released_grade {
                        grade.to_string()
                    } else {
                        cell.status.label().to_owned()
                    };
                    let color = if cell.status == SubmissionStatus::Missing { color!(0xa42620) }
                        else if draft.is_some() { color!(0x137333) } else { FG };
                    if let Some(edit) = self.gradebook_edit.as_ref().filter(|edit| {
                        (edit.course, edit.item, edit.expected.student_id) == (course, *item, id)
                    }) {
                        let ready = !self.has_pending_request() && edit.grade(work.maximum_points).is_some();
                        let submit = ready || (!self.has_pending_request() && grade_input_matches_saved(&edit.input, &edit.expected));
                        let field = text_input("Points", &edit.input).id(GRADEBOOK_FIELD).padding(6).width(Fill)
                            .on_input_maybe((!self.saving_teacher_grade).then_some(Message::GradebookGradeInput))
                            .on_submit_maybe(submit.then_some(Message::SaveGradebookEdit(true)));
                        line = line.push(row![
                            field,
                            button("Save").padding([6, 7]).style(outline_style)
                                .on_press_maybe(ready.then_some(Message::SaveGradebookEdit(false))),
                        ].spacing(4).align_y(Alignment::Center).width(CELL_WIDTH));
                    } else {
                        let action = if self.gradebook_edit_mode && cell.can_edit() && work.maximum_points.is_some() {
                            Message::EditGradebookCell(course, *item, id)
                        } else {
                            Message::OpenGradebookCell(course, *item, id)
                        };
                        line = line.push(button(text(label).size(13).color(color)).style(quiet_style).padding(8)
                            .width(CELL_WIDTH).on_press_maybe(self.gradebook_edit.is_none().then_some(action)));
                    }
                } else {
                    line = line.push(container(text("—").size(13).color(MUTED)).padding(8).width(CELL_WIDTH));
                }
            }
            table = table.push(rule::horizontal(1).style(divider_style)).push(line);
        }
        let mut actions = row![].spacing(8);
        #[cfg(target_os = "macos")]
        {
            actions = actions.push(button(if self.exporting_grades { "Exporting…" } else { "Download grades (.csv)" })
                .style(outline_style).padding([7, 10])
                .on_press_maybe((!self.has_pending_request()).then_some(Message::ExportGrades(course, None))));
        }
        if self.can_edit_gradebook(course) || self.screenshot_directory.is_some() {
            actions = actions.push(button(if self.gradebook_edit_mode { "Done editing" } else { "Edit grades" })
                .style(outline_style).padding([7, 10])
                .on_press_maybe((!self.has_pending_request() && self.gradebook_edit.is_none())
                    .then_some(Message::GradebookEditMode(!self.gradebook_edit_mode))));
            if self.gradebook_edit.is_some() {
                actions = actions.push(button("Cancel cell edit").style(quiet_style).padding([7, 10])
                    .on_press_maybe((!self.saving_teacher_grade).then_some(Message::CancelGradebookEdit)));
            }
        }
        let mut page_view = column![text("Grades").size(23).font(medium()).color(FG), actions.wrap().vertical_spacing(6)].spacing(10);
        if pages > 1 {
            page_view = page_view.push(row![
                button("‹").style(outline_style).padding([6, 10])
                    .on_press_maybe((page > 0 && self.gradebook_edit.is_none()).then_some(Message::GradebookColumnPage(page.saturating_sub(1)))),
                text(format!("Classwork {}–{stop} of {total_columns}", start + 1)).size(13),
                button("›").style(outline_style).padding([6, 10])
                    .on_press_maybe((page + 1 < pages && self.gradebook_edit.is_none()).then_some(Message::GradebookColumnPage(page + 1))),
            ].spacing(8).align_y(Alignment::Center));
        }
        page_view.push(scrollable(table)
            .direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::default())).width(Fill)).into()
    }

    fn can_read_teacher_work(&self, course: i64, item: i64) -> bool {
        self.can_read_pages() && self.screen == Screen::Item(course)
            && self.item_id.as_deref().and_then(|value| value.parse().ok()) == Some(item)
            && self.class_roles.get(&course) == Some(&CourseRole::Teacher)
            && self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(course))
                .is_some_and(|items| items.iter().any(|work| work.id.parse::<i64>().ok() == Some(item) && has_submission(work)))
    }

    fn selected_teacher_feedback_key(&self) -> Option<desktop_auth::TeacherFeedbackKey> {
        let work = self.teacher_work.as_ref()?;
        if self.screen != Screen::Item(work.course) || self.item_id.as_deref()?.parse::<i64>().ok()? != work.item
            || self.class_roles.get(&work.course) != Some(&CourseRole::Teacher)
        {
            return None;
        }
        let row = work.selected_row()?;
        (row.student_id > 0 && row.submission_id > 0).then_some(desktop_auth::TeacherFeedbackKey {
            course: work.course, item: work.item, student: row.student_id, submission: row.submission_id,
        })
    }

    #[cfg(target_os = "macos")]
    fn export_grades(&mut self, course: i64, item: Option<i64>) -> Task<Message> {
        let (Some(session), Some(snapshot)) = (self.session.clone(), self.snapshot.as_ref()) else { return Task::none(); };
        let Some(items) = snapshot.coursework_for(course) else { return Task::none(); };
        let (csv, suggested) = if let Some(item) = item {
            let Some(work) = self.teacher_work.as_ref().filter(|work| work.course == course && work.item == item
                && self.can_read_teacher_work(course, item)) else { return Task::none(); };
            let Some(assignment) = items.iter().find(|entry| entry.id.parse::<i64>().ok() == Some(item)) else { return Task::none(); };
            let grid = desktop_auth::TeacherGradebook {
                students: work.students.iter().map(|row| CourseMember {
                    id: row.student_id.to_string(), display_name: row.name.clone().unwrap_or_else(|| "Student name unavailable".into()),
                    role: CourseRole::Student,
                }).collect(),
                cells: work.students.iter().map(|row| ((row.student_id, item), desktop_auth::TeacherGradebookCell {
                    submission_id: row.submission_id, remote_state: row.remote_state,
                    status: row.status, released_grade: row.released_grade, draft_grade: None,
                })).collect(),
            };
            (grade_export::gradebook_csv(&grid, &[(item, assignment)]), grade_export::suggested_name(&assignment.title))
        } else {
            if self.screen != Screen::Course(course) || self.tab != Tab::Grades { return Task::none(); }
            let Some(grid) = self.gradebooks.get(&course) else { return Task::none(); };
            let Some(entry) = snapshot.courses.iter().find(|entry| entry.id == course) else { return Task::none(); };
            let columns: Vec<_> = items.iter().filter(|item| has_submission(item))
                .filter_map(|item| Some((item.id.parse::<i64>().ok()?, item))).collect();
            (grade_export::gradebook_csv(grid, &columns), grade_export::suggested_name(&entry.title))
        };
        self.exporting_grades = true;
        self.error = None;
        return Task::perform(async move {
            let details = desktop_auth::load_own_class_details(&session, vec![course]).await?;
            if details.roles.get(&course) != Some(&CourseRole::Teacher) {
                return Err("Classroom did not confirm teacher access for this export.".into());
            }
            let Some(path) = grade_export::choose_destination(suggested).await? else { return Ok(false); };
            tokio::task::spawn_blocking(move || grade_export::save_csv(&path, &csv)).await
                .map_err(|_| "The local CSV save did not finish.")??;
            Ok(true)
        }, Message::GradesExported);
    }

    fn can_grade_teacher_work(&self, course: i64) -> bool {
        self.snapshot.as_ref().is_some_and(|snapshot| snapshot.courses.iter()
            .any(|entry| entry.id == course && entry.state == CourseState::Active))
    }

    fn teacher_student_work_panel<'a>(&'a self, item: &'a Coursework) -> Element<'a, Message> {
        let mut panel = column![text(if item.kind == CourseworkKind::Question { "Student answers" } else { "Student work" })
            .size(18).font(medium()).color(FG)].spacing(10).width(Fill);
        if let Ok(id) = item.id.parse::<i64>() {
            let work = self.teacher_work.as_ref().filter(|work| work.course == item.course_id && work.item == id);
            if let Some(status) = self.page_read_status(
                PageRead::Detail(DetailRead::TeacherSubmissions(item.course_id, id)), work.is_some(), "Loading student submissions…",
            ) {
                panel = panel.push(status);
            }
            if let Some(work) = work {
                #[cfg(target_os = "macos")]
                {
                    panel = panel.push(button("Download these grades (.csv)").padding([7, 10]).style(outline_style)
                        .on_press_maybe((!self.has_pending_request()).then_some(Message::ExportGrades(item.course_id, Some(id)))));
                }
                let filters = StudentWorkFilter::ALL.into_iter().map(|filter| {
                    let count = work.students.iter().filter(|student| filter.includes(student)).count();
                    let selected = work.filter == filter;
                    button(text(format!("{} {count}", filter.label())).size(13))
                        .padding([7, 10]).style(move |_, status| nav_style(selected, status))
                        .on_press(Message::FilterStudentWork(filter)).into()
                });
                panel = panel.push(row(filters).spacing(6).wrap().vertical_spacing(6));
                let can_return = self.can_grade_teacher_work(item.course_id) || self.screenshot_directory.is_some();
                if can_return && work.returnable().next().is_some() {
                    panel = panel.push(self.bulk_return_controls(work));
                }
                let mut visible = work.students.iter().filter(|student| work.filter.includes(student)).peekable();
                if visible.peek().is_none() {
                    panel = panel.push(text("No student work in this view.").size(14).color(MUTED));
                }
                for student in visible {
                    let selected = work.selected == Some(student.student_id);
                    let name = student.name.as_deref().unwrap_or("Student name unavailable");
                    let student_row = button(row![
                        avatar(first_initial(name), BLUE, 28.0, 12.0),
                        text(name).size(15).width(Fill),
                        text(student.status.label()).size(13).color(MUTED),
                        text(if selected { "⌃" } else { "⌄" }).size(16).color(MUTED),
                    ].spacing(8).align_y(Alignment::Center)).width(Fill).padding([7, 10])
                    .style(move |_, status| nav_style(selected, status)).on_press(Message::SelectTeacherStudent(student.student_id));
                    if can_return {
                        let selection: Element<'_, Message> = if desktop_auth::TeacherGradingAction::Return.allowed(student, None) {
                            checkbox(work.bulk_selected.contains(&student.student_id))
                                .on_toggle_maybe((!self.has_pending_request()).then(|| {
                                    let id = student.student_id;
                                    move |checked| Message::SelectBulkReturn(id, checked)
                                })).into()
                        } else {
                            space().width(18).into()
                        };
                        panel = panel.push(row![selection, student_row].spacing(5).align_y(Alignment::Center));
                    } else {
                        panel = panel.push(student_row);
                    }
                    if selected {
                        let mut detail = column![].spacing(6);
                        let pending_grade = student.draft_grade.filter(|grade| Some(*grade) != student.released_grade);
                        for (label, grade) in [("Released grade", student.released_grade), ("Draft grade", pending_grade)] {
                            if let Some(grade) = grade {
                                let mark = item.maximum_points.map_or_else(|| grade.to_string(), |out_of| format!("{grade} / {out_of}"));
                                detail = detail.push(text(format!("{label} · {mark}")).size(14).color(FG));
                            }
                        }
                        if let Some(QuestionAnswer::ShortAnswer(answer) | QuestionAnswer::MultipleChoice(answer)) = &student.answer {
                            detail = detail.push(text(answer).size(15).color(FG).width(Fill)
                                .wrapping(text::Wrapping::WordOrGlyph));
                        }
                        for attachment in &student.attachments {
                            detail = detail.push(button(attachment.title.as_str()).style(outline_style).padding([7, 10])
                                .on_press_maybe(attachment.url.as_ref().filter(|url| trusted_attachment_host(url).is_some())
                                    .map(|url| Message::OpenAttachment(url.clone()))));
                        }
                        if student.released_grade.is_none() && student.draft_grade.is_none() && student.answer.is_none()
                            && student.attachments.is_empty()
                        {
                            detail = detail.push(text("No grade, answer, or attachment was returned.").size(13).color(MUTED));
                        }
                        if can_return && !work.bulk_confirming {
                            detail = detail.push(self.teacher_grading_controls(work, student, item.maximum_points));
                        }
                        if !work.bulk_confirming
                            && let Some(key) = self.selected_teacher_feedback_key()
                        {
                            detail = detail.push(self.teacher_feedback_panel(key, name, can_return));
                        }
                        panel = panel.push(container(detail).padding([4, 12]).width(Fill));
                    }
                }
            }
        }
        container(panel).padding(14).width(Fill).style(|_| outline_surface(9.0)).into()
    }

    fn bulk_return_controls<'a>(&'a self, work: &'a TeacherWork) -> Element<'a, Message> {
        let selected: Vec<_> = work.selected_returns().collect();
        let ready = !self.has_pending_request();
        let visible: Vec<_> = work.returnable().filter(|row| work.filter.includes(row)).collect();
        let all_selected = !visible.is_empty() && visible.iter().all(|row| work.bulk_selected.contains(&row.student_id));
        let mut actions = row![
            button(if all_selected { "Deselect visible" } else { "Select all visible" }).padding([7, 10]).style(quiet_style)
                .on_press_maybe((ready && !visible.is_empty()).then_some(Message::SelectVisibleReturns(!all_selected))),
        ].spacing(6).align_y(Alignment::Center);
        if !selected.is_empty() {
            actions = actions.push(text(format!("{} selected", selected.len())).size(13).color(MUTED));
            if !work.bulk_confirming {
                actions = actions.push(button("Return selected…").padding([7, 10]).style(outline_style)
                    .on_press_maybe(ready.then_some(Message::ConfirmBulkReturn(true))));
            }
            actions = actions.push(button("Clear selection").padding([7, 10]).style(quiet_style)
                .on_press_maybe(ready.then_some(Message::ClearBulkReturns)));
        }
        let mut panel = column![actions.wrap().vertical_spacing(6)].spacing(8);
        if work.bulk_confirming && !selected.is_empty() {
            let graded = selected.iter().filter(|row| row.draft_grade.or(row.released_grade).is_some()).count();
            panel = panel.push(text(format!("Return {} submissions? {graded} include a saved grade.", selected.len())).size(14))
                .push(row![
                    button("Return work").padding([7, 10]).style(primary_style)
                        .on_press_maybe(ready.then_some(Message::ReturnSelectedTeacherWork)),
                    button("Cancel").padding([7, 10]).style(quiet_style)
                        .on_press_maybe(ready.then_some(Message::ConfirmBulkReturn(false))),
                ].spacing(6));
        }
        panel.into()
    }

    fn teacher_feedback_panel<'a>(
        &'a self, key: desktop_auth::TeacherFeedbackKey, student: &'a str, compose: bool,
    ) -> Element<'a, Message> {
        let thread = self.teacher_feedback.get(&key);
        let comments = thread.and_then(|thread| thread.comments.as_ref());
        let mut panel = column![text(format!("Private feedback to {student}")).size(15).font(medium())].spacing(7);
        if let Some(status) = self.page_read_status(
            PageRead::Detail(DetailRead::TeacherFeedback(key)), comments.is_some(), "Loading private feedback…",
        ) {
            panel = panel.push(status);
        }
        if let Some(comments) = comments {
            if comments.is_empty() {
                panel = panel.push(text("No private comments on this submission.").size(13).color(MUTED));
            }
            for comment in comments {
                panel = panel.push(self.comment_card(key.course, None, CommentAudience::Private, comment));
            }
        }
        if compose {
            let draft = thread.map_or("", |thread| thread.draft.as_str());
            let unconfirmed = thread.is_some_and(|thread| thread.unconfirmed);
            if unconfirmed {
                panel = panel.push(text("Review the updated thread before sending this draft again.").size(13).color(MUTED));
            }
            let ready = !self.has_pending_request() && comments.is_some() && !unconfirmed && !draft.trim().is_empty();
            panel = panel.push(row![
                text_input("Add a private comment…", draft).padding(8).width(Fill)
                    .on_input_maybe((!self.posting_comment).then(|| move |value| Message::EditTeacherFeedback(key, value))),
                button("Send privately").style(primary_style).padding([8, 10])
                    .on_press_maybe(ready.then_some(Message::SendTeacherFeedback(key))),
            ].spacing(6).wrap().vertical_spacing(6));
        }
        container(panel).padding(10).width(Fill).style(|_| outline_surface(8.0)).into()
    }

    fn teacher_grading_controls<'a>(&'a self, work: &'a TeacherWork, student: &'a desktop_auth::TeacherSubmission,
        maximum: Option<f64>) -> Element<'a, Message> {
        use desktop_auth::TeacherGradingAction::{Return, SaveDraft};
        let ready = !self.has_pending_request();
        let mut controls = column![].spacing(8);
        if maximum.is_some() {
            let save = work.grade.trim().parse::<f64>().ok().is_some_and(|grade| SaveDraft(grade).allowed(student, maximum));
            controls = controls.push(row![
                text_input("Points", &work.grade).padding(7).width(100)
                    .on_input_maybe(ready.then_some(Message::EditTeacherGrade)),
                button("Save draft grade").padding([7, 10]).style(outline_style)
                    .on_press_maybe((ready && save).then_some(Message::SaveTeacherGrade)),
            ].spacing(7).align_y(Alignment::Center));
        }
        if Return.allowed(student, maximum) {
            let ready = ready && grade_input_matches_saved(&work.grade, student);
            if work.confirming_return {
                let message = student.draft_grade.or(student.released_grade).map_or_else(
                    || "Return this work without a grade?".to_owned(),
                    |grade| format!("Return this work with {grade} points?"),
                );
                controls = controls.push(text(message).size(14)).push(row![
                    button("Return work").padding([7, 10]).style(primary_style)
                        .on_press_maybe(ready.then_some(Message::ReturnTeacherWork)),
                    button("Cancel").padding([7, 10]).style(quiet_style)
                        .on_press_maybe((!self.saving_teacher_grade).then_some(Message::ConfirmReturnTeacherWork(false))),
                ].spacing(6));
            } else {
                controls = controls.push(button("Return…").padding([7, 10]).style(outline_style)
                    .on_press_maybe(ready.then_some(Message::ConfirmReturnTeacherWork(true))));
            }
        }
        controls.into()
    }

    fn item_detail(&self, course_id: i64) -> Element<'_, Message> {
        let Some(snapshot) = &self.snapshot else {
            return self.unavailable_screen("Classwork", Icon::Book, "This item is no longer loaded.");
        };
        let Some(item) = self.item_id.as_ref()
            .and_then(|key| snapshot.coursework_for(course_id)?.iter().find(|item| &item.id == key))
        else {
            let status = if snapshot.coursework_for(course_id).is_none() {
                self.page_read_status(PageRead::Coursework(course_id), false, "Loading classwork…")
            } else {
                None
            };
            return scrollable(column![
                button("‹ Back").style(quiet_style).on_press(Message::CloseItem),
                status.unwrap_or_else(|| empty_panel(
                    Icon::Book, "This item is unavailable", "It may have been removed since the last classwork refresh.",
                )),
            ].spacing(10).padding(26).width(Fill)).width(Fill).height(Fill).into();
        };
        let course_name = snapshot.courses.iter().find(|course| course.id == course_id)
            .map_or("Classwork", |course| course.title.as_str());
        let topic = item.topic_id.as_ref().and_then(|id| {
            snapshot.topics_for(course_id)?.iter().find(|topic| &topic.id == id)
        });
        let item_label = item.question.as_ref().map_or(item.kind.label(), QuestionDetails::label);
        let copy_link: Element<'_, Message> = match item_link_kind(item).zip(item.id.parse::<i64>().ok()) {
            Some((kind, id)) => button("Copy link").padding([6, 8]).style(quiet_style)
                .on_press(Message::CopyClassroomLink(ClassroomLink::Item { course: course_id, item: id, kind })).into(),
            None => space().width(0).into(),
        };
        let mut body = column![
            row![
                button("‹ Back").style(quiet_style).padding([8, 10]).on_press(Message::CloseItem),
                text(course_name).size(14).color(MUTED).width(Fill),
                copy_link,
            ].spacing(8).align_y(Alignment::Center),
            text(topic.map_or_else(|| item_label.to_owned(), |topic| {
                format!("{item_label} · {}", topic.title)
            })).size(14).color(BLUE),
            text(&item.title).size(29).color(FG).font(medium()),
        ].spacing(9).width(Fill);
        if let Some(due) = &item.due_label {
            body = body.push(pill(due, ACTIVE_BG, ACTIVE_FG));
        }
        if !item.description.trim().is_empty() {
            body = body.push(container(formatted_description(&item.description, &item.description_styles, &item.description_bullets))
                .padding([10, 0]));
        }
        let own_answer = item.submission.as_ref().and_then(|submission| submission.question_answer.as_ref());
        if item.question_sharing == Some(QuestionSharing::Course) {
            let visibility = match &item.question {
                Some(QuestionDetails::ShortAnswer) => "Classmates can see your answer after they turn in theirs.",
                Some(QuestionDetails::MultipleChoice(_)) => "Your answer counts toward the class summary.",
                None => "",
            };
            if !visibility.is_empty() {
                body = body.push(text(visibility).size(14).color(MUTED));
            }
        }
        let question_key = item.id.parse::<i64>().ok().map(|id| (course_id, id));
        let revision = question_key.and_then(|key| self.short_answer_revisions.get(&key));
        let editable = question_key.and_then(|(course, id)| self.editable_question(course, id))
            .or_else(|| {
                if self.screenshot_directory.is_some() && desktop_auth::question_submission_is_current(item, revision) {
                    item.question.as_ref()
                } else {
                    None
                }
            });
        let mut choice_shown = false;
        if let Some(QuestionDetails::MultipleChoice(choices)) = &item.question
            && !choices.is_empty()
        {
            let chosen = match own_answer {
                Some(QuestionAnswer::MultipleChoice(answer)) => matching_choice(choices, answer),
                _ => None,
            };
            let drafted = editable.and_then(|_| question_key)
                .and_then(|key| self.choice_drafts.get(&key)).and_then(|choice| matching_choice(choices, choice));
            let mut choices_panel = column![text("Answer choices").size(17).font(medium()).color(FG)].spacing(7);
            for (index, choice) in choices.iter().enumerate() {
                let selected = chosen == Some(index) || drafted == Some(index);
                choice_shown |= chosen == Some(index);
                let mut row = row![
                    text(if selected { "●" } else { "○" }).size(16).color(if selected { ACTIVE_FG } else { MUTED }),
                    text(choice).size(15).color(FG).width(Fill).wrapping(text::Wrapping::WordOrGlyph),
                ].spacing(8).align_y(Alignment::Center);
                if chosen == Some(index) {
                    row = row.push(pill("Your answer", ACTIVE_BG, ACTIVE_FG));
                } else if drafted == Some(index) {
                    row = row.push(pill("Selected", ACTIVE_BG, ACTIVE_FG));
                }
                let option: Element<'_, Message> = if let (Some((course, id)), Some(details)) = (question_key, editable)
                    && !self.sending_submission && self.confirming_question_answer.is_none()
                    && details.accepts(&QuestionAnswer::MultipleChoice(choice.clone()))
                {
                    button(row).width(Fill).padding([7, 4]).style(quiet_style)
                        .on_press(Message::SelectQuestionChoice(course, id, choice.clone())).into()
                } else {
                    container(row).width(Fill).padding([7, 4]).into()
                };
                choices_panel = choices_panel.push(option);
            }
            body = body.push(container(choices_panel).width(Fill).padding(15).style(|_| outline_surface(9.0)));
        }
        let teacher = self.class_roles.get(&course_id) == Some(&CourseRole::Teacher);
        if teacher && has_submission(item) && (self.can_read_pages() || self.screenshot_directory.is_some()) {
            body = body.push(self.teacher_student_work_panel(item));
        } else if has_submission(item) {
            let status = self.page_read_status(
                PageRead::Detail(DetailRead::Submissions(course_id)),
                self.submissions_checked.contains(&course_id), "Loading your work…",
            );
            let mut work = column![text("Your work").size(17).font(medium()).color(FG)].spacing(8);
            if let Some(submission) = &item.submission {
                work = work.push(text(submission.status.label()).size(14).color(MUTED));
                if let Some(date) = submission.last_turned_in_millis.and_then(DateTime::from_timestamp_millis) {
                    work = work.push(text(format!("Last turned in {}",
                        date.with_timezone(&Local).format("%-d %b %Y at %H:%M"))).size(14).color(MUTED));
                }
                if let Some(label) = grade_label(submission) {
                    work = work.push(text(label).size(15).color(FG));
                }
                if !choice_shown
                    && let Some(QuestionAnswer::ShortAnswer(answer) | QuestionAnswer::MultipleChoice(answer)) = own_answer
                {
                    work = work.push(text("Your answer").size(14).font(medium()).color(FG))
                        .push(text(answer).size(15).color(FG).width(Fill).wrapping(text::Wrapping::WordOrGlyph));
                }
                for attachment in &submission.attachments {
                    let label = if item.attachments.iter().any(|template| attachment.is_personal_copy_of(template)) {
                        format!("{} · Your copy", attachment.title)
                    } else {
                        attachment.title.clone()
                    };
                    let open = button(text(label)).padding([8, 10]).style(outline_style)
                        .on_press_maybe(self.attachment_action(attachment));
                    let mut line = row![open].spacing(7).align_y(Alignment::Center);
                    if let (Some(material), Ok(item_id)) = (attachment.remote_id, item.id.parse::<i64>())
                        && self.can_compose_comment(course_id, item_id)
                        && desktop_auth::SubmissionAction::for_submission(submission)
                            == Some(desktop_auth::SubmissionAction::TurnIn)
                    {
                        if self.pending_remove_material == Some((course_id, item_id, material)) {
                            line = line
                                .push(button("Cancel").style(quiet_style).on_press(Message::CancelRemoveMaterial))
                                .push(button("Confirm remove").style(outline_style)
                                    .on_press_maybe((!self.has_pending_request())
                                        .then_some(Message::RemoveMaterial(course_id, item_id, material))));
                        } else {
                            line = line.push(button("Remove…").style(quiet_style)
                                .on_press_maybe((!self.has_pending_request())
                                    .then_some(Message::ConfirmRemoveMaterial(course_id, item_id, material))));
                        }
                    }
                    work = work.push(line);
                }
            } else if status.is_none() {
                work = work.push(text("No submission status available for this item.").size(14).color(MUTED));
            }
            if let Some(status) = status {
                work = work.push(status);
            }
            if let (Some(key @ (_, item_id)), Some(details)) = (question_key, editable) {
                let confirming = self.confirming_question_answer == Some(key);
                if *details == QuestionDetails::ShortAnswer {
                    let draft = self.short_answer_drafts.get(&key).unwrap_or(&self.empty_short_answer);
                    if revision.is_some() {
                        work = work.push(text("Edit your answer").size(15).font(medium()).color(FG));
                    }
                    let mut editor = text_editor(draft).placeholder("Type your answer")
                        .height(if revision.is_some() { 135 } else { 120 }).font(CLASSROOM_FONT)
                        .wrapping(text::Wrapping::WordOrGlyph).size(15).padding(9);
                    if !self.sending_submission && !confirming {
                        editor = editor.on_action(move |action| Message::EditShortAnswer(course_id, item_id, action));
                    }
                    work = work.push(editor);
                }
                let ready = !self.has_pending_request() && self.question_draft(key, details).is_some();
                if confirming {
                    work = work.push(text(if revision.is_some() { "Turn in this revised answer?" }
                        else { "Turn in this answer?" }).size(14))
                        .push(row![
                            button("Cancel").style(quiet_style).on_press(Message::CancelQuestionAnswer),
                            button("Turn in").style(primary_style).padding([8, 12])
                                .on_press_maybe(ready.then_some(Message::SendQuestionAnswer(course_id, item_id))),
                        ].spacing(8));
                } else {
                    let submit = button(if self.sending_submission { "Turning in…" } else { "Turn in…" })
                        .style(outline_style).padding([8, 12])
                        .on_press_maybe(ready.then_some(Message::ConfirmQuestionAnswer(course_id, item_id)));
                    if revision.is_some() {
                        work = work.push(row![
                            submit,
                            button("Discard edit").style(quiet_style).padding([8, 12])
                                .on_press_maybe((!self.sending_submission).then_some(Message::DiscardShortAnswerEdit(course_id, item_id))),
                        ].spacing(8));
                    } else {
                        work = work.push(submit);
                    }
                }
            } else if let Some((course, id)) = question_key
                && self.current_question(course, id).and_then(desktop_auth::editable_short_answer).is_some()
            {
                if revision.is_some() {
                    work = work.push(text("Your saved answer changed while you were editing.").size(14).color(MUTED));
                }
                work = work.push(button(if revision.is_some() { "Start a new edit" } else { "Edit answer" })
                    .style(outline_style).padding([8, 12])
                    .on_press_maybe((!self.has_pending_request()).then_some(Message::BeginShortAnswerEdit(course, id))));
            }
            if item.kind == CourseworkKind::Assignment
                && let Ok(item_id) = item.id.parse::<i64>()
                && (self.can_compose_comment(course_id, item_id)
                    || self.screenshot_directory.is_some()
                        && self.pending_submission.is_some_and(|(course, item, _)| (course, item) == (course_id, item_id)))
                && let Some(action) = item.submission.as_ref().and_then(desktop_auth::SubmissionAction::for_submission)
            {
                if action == desktop_auth::SubmissionAction::TurnIn {
                    let ready = !self.has_pending_request()
                        && trusted_attachment_host(self.attachment_link.trim()).is_some();
                    work = work.push(row![
                        text_input("Add an HTTPS link", &self.attachment_link).padding(9)
                            .on_input(Message::EditAttachmentLink).width(Fill),
                        button("Attach link").padding([8, 12]).style(outline_style)
                            .on_press_maybe(ready.then_some(Message::AddAttachmentLink(course_id, item_id))),
                    ].spacing(8).align_y(Alignment::Center));
                }
                if action == desktop_auth::SubmissionAction::TurnIn {
                    if let Some((course, item, path)) = &self.submission_file
                        && (*course, *item) == (course_id, item_id)
                    {
                        let name = path.file_name().map_or_else(|| "File".into(), |name| name.to_string_lossy());
                        work = work.push(row![
                            text(name).size(14).width(Fill),
                            button("Cancel").style(quiet_style).on_press(Message::CancelSubmissionFile),
                            button("Upload and attach").padding([8, 12]).style(primary_style)
                                .on_press_maybe((!self.has_pending_request())
                                    .then_some(Message::UploadSubmissionFile(course_id, item_id))),
                        ].spacing(8).align_y(Alignment::Center));
                    } else {
                        work = work.push(button(if self.sending_submission { "Updating your work…" } else { "Choose file…" })
                            .padding([8, 12]).style(outline_style)
                            .on_press_maybe((!self.has_pending_request())
                                .then_some(Message::ChooseSubmissionFile(course_id, item_id))));
                    }
                }
                let confirmation = (course_id, item_id, action);
                let mark_done = assignment_is_mark_done(action, item.submission.as_ref());
                let label = if mark_done { "Mark as done" } else { action.label() };
                if self.pending_submission == Some(confirmation) {
                    work = work.push(text(match action {
                        desktop_auth::SubmissionAction::TurnIn if mark_done => "Mark this assignment as done without attaching a file?",
                        desktop_auth::SubmissionAction::TurnIn => "Turn in the work currently attached to this assignment?",
                        desktop_auth::SubmissionAction::Unsubmit => "Unsubmit this work so you can make changes?",
                    }).size(14));
                    work = work.push(row![
                        button("Cancel").style(quiet_style).on_press(Message::CancelSubmission),
                        button(label).style(primary_style)
                            .on_press_maybe((!self.has_pending_request())
                                .then_some(Message::SendSubmission(course_id, item_id, action))),
                    ].spacing(8));
                } else {
                    work = work.push(button(label).style(outline_style)
                        .on_press_maybe((!self.has_pending_request())
                            .then_some(Message::ConfirmSubmission(course_id, item_id, action))));
                }
            }
            if let Ok(item_id) = item.id.parse::<i64>() {
                if item.kind == CourseworkKind::Assignment
                    && (self.can_read_pages() && self.class_roles.get(&course_id) == Some(&CourseRole::Student)
                        || self.screenshot_directory.is_some())
                    && let Some(url) = desktop_auth::originality_report_url(course_id, item_id, item.originality_enabled == Some(true))
                {
                    work = work.push(button("Open originality reports in Classroom").padding([8, 12])
                        .style(outline_style).on_press(Message::OpenAttachment(url)));
                }
                let picker = self.drive_picker.as_ref().filter(|picker| picker.course == course_id && picker.item == Some(item_id));
                let ready = self.can_add_submission_drive_file(course_id, item_id) && !self.has_pending_request();
                #[cfg(target_os = "macos")]
                if self.can_add_submission_drive_file(course_id, item_id) || self.submission_capture.is_some()
                    || (self.screenshot_directory.is_some() && self.new_submission_document_open)
                {
                    work = work.push(self.submission_capture_controls(ready || self.screenshot_directory.is_some()));
                }
                if let Some(picker) = picker
                    && (self.can_add_submission_drive_file(course_id, item_id) || self.screenshot_directory.is_some())
                {
                    work = work.push(self.drive_picker_panel(picker, ready || self.screenshot_directory.is_some(), |file| {
                        item.submission.as_ref().is_some_and(|submission| {
                            submission.attachments.iter().any(|attachment| file.matches_attachment(attachment))
                        })
                    }));
                } else if self.can_add_submission_drive_file(course_id, item_id) {
                    work = work.push(button("Attach from Google Drive").style(outline_style).padding([8, 12])
                        .on_press_maybe(ready.then_some(Message::OpenSubmissionDrivePicker(course_id, item_id))));
                }
                if self.can_add_submission_drive_file(course_id, item_id)
                    || self.screenshot_directory.is_some() && self.new_submission_document_open
                {
                    work = work.push(button(if self.new_submission_document_open { "Cancel" } else { "Create new" })
                        .style(outline_style).padding([8, 12])
                        .on_press_maybe(ready.then_some(Message::ToggleNewSubmissionDocument(course_id, item_id))));
                    if self.new_submission_document_open {
                        let kinds = DriveDocumentKind::ALL.into_iter().map(|kind| {
                            button(kind.label()).style(quiet_style).padding([7, 10])
                                .on_press_maybe((ready || self.screenshot_directory.is_some())
                                    .then_some(Message::CreateSubmissionDocument(course_id, item_id, kind))).into()
                        });
                        work = work.push(row(kinds).spacing(6).wrap().vertical_spacing(6));
                    }
                }
            }
            body = body.push(container(work).width(Fill).padding(16).style(|_| outline_surface(9.0)));
        }
        if snapshot.source == SnapshotSource::Live
            && item.submission.as_ref().is_some_and(|sub| sub.remote_id.is_some())
            && let Ok(item_id) = item.id.parse::<i64>()
        {
            let feedback = self.private_feedback.get(&(course_id, item_id));
            let mut panel = column![text("Private feedback").size(17).font(medium()).color(FG)].spacing(8);
            if let Some(status) = self.page_read_status(
                PageRead::Detail(DetailRead::PrivateFeedback(course_id, item_id)),
                feedback.is_some(), "Loading private feedback…",
            ) {
                panel = panel.push(status);
            }
            match feedback {
                Some(entries) if entries.is_empty() => {
                    panel = panel.push(text("No private feedback on this submission.").size(14).color(MUTED));
                }
                Some(entries) => {
                    for entry in entries {
                        panel = panel.push(self.comment_card(course_id, Some(item_id), CommentAudience::Private, entry));
                    }
                }
                None => {},
            }
            if self.can_compose_comment(course_id, item_id) {
                let key = (course_id, item_id);
                let draft = self.private_drafts.get(&key).map_or("", String::as_str);
                let unconfirmed = self.private_unconfirmed.contains(&key);
                let allowed = !self.has_pending_request() && !draft.trim().is_empty() && !unconfirmed;
                if unconfirmed {
                    panel = panel.push(text("Check the updated comments before sending your draft again.").size(14).color(MUTED));
                }
                panel = panel.push(row![
                    text_input("Add a private comment for your teacher…", draft).padding(10).width(Fill)
                        .on_input(move |value| Message::EditPrivateComment(course_id, item_id, value)),
                    button("Send privately").padding([9, 13]).style(primary_style)
                        .on_press_maybe(allowed.then_some(Message::PostPrivateComment(course_id, item_id))),
                ].spacing(8).align_y(Alignment::Center));
            }
            body = body.push(container(panel).width(Fill).padding(16).style(|_| outline_surface(9.0)));
        }
        if !item.attachments.is_empty() {
            let mut attachments = column![text("Attachments").size(17).font(medium()).color(FG)].spacing(8);
            for attachment in &item.attachments {
                let template = attachment.drive_sharing == Some(DriveSharing::IndividualCopy);
                let file = if template {
                    matching_personal_copy(item, attachment)
                } else {
                    Some(attachment)
                };
                let action = file.and_then(|file| self.attachment_action(file));
                let opening = file.and_then(|file| file.drive_reference.as_ref())
                    .is_some_and(|id| self.opening_drive_file.as_ref() == Some(id));
                let host = file.and_then(|file| file.url.as_deref()).and_then(trusted_attachment_host);
                let description = match (attachment.drive_sharing, action.is_some(), opening) {
                    (_, _, true) => "Opening in Drive…".to_owned(),
                    (Some(DriveSharing::IndividualCopy), true, _) => "Opens your personal copy".to_owned(),
                    (Some(DriveSharing::IndividualCopy), false, _) if !self.submissions_checked.contains(&course_id)
                        && self.can_read_pages() && !self.page_errors.contains_key(&PageRead::Detail(DetailRead::Submissions(course_id))) =>
                        "Checking for your personal copy…".to_owned(),
                    (Some(DriveSharing::IndividualCopy), false, _) => "Personal copy unavailable".to_owned(),
                    (Some(sharing), _, _) => format!("{} · {}", attachment.kind.label(), sharing.label()),
                    (None, true, _) => host.map_or_else(|| attachment.kind.label().to_owned(), |host|
                        format!("{} · {host}", attachment.kind.label())),
                    (None, false, _) => format!("{} · no secure link", attachment.kind.label()),
                };
                let content = row![
                    icon(Icon::Book, ACTIVE_FG, 20),
                    column![text(&attachment.title).size(15).color(FG), text(description).size(12).color(MUTED)]
                        .spacing(3).width(Fill),
                    icon(Icon::Arrow, MUTED, 16),
                ].spacing(12).align_y(Alignment::Center);
                attachments = attachments.push(button(content).width(Fill).padding(13)
                    .style(outline_style).on_press_maybe(action));
            }
            body = body.push(attachments);
        }
        if item.kind == CourseworkKind::Assignment && let Ok(id) = item.id.parse::<i64>()
            && let Some(rubric) = self.assignment_rubric(course_id, id, item)
        {
            body = body.push(rubric);
        }
        let mut comments = column![text("Class comments").size(17).font(medium()).color(FG)].spacing(9);
        if let Ok(stream_item_id) = item.id.parse::<i64>() {
            if let Some(status) = self.page_read_status(
                PageRead::Detail(DetailRead::Comments(course_id, stream_item_id)),
                item.comments.is_some(), "Loading comments…",
            ) {
                comments = comments.push(status);
            }
            if self.can_compose_comment(course_id, stream_item_id)
                && let Some(status) = self.page_read_status(
                    PageRead::StreamAccess(course_id), self.stream_access.contains_key(&course_id), "Checking comment permissions…",
                )
            {
                comments = comments.push(status);
            }
            if self.can_compose_class_comment(course_id, stream_item_id) {
                let key = (course_id, stream_item_id);
                let draft = self.comment_drafts.get(&key).map_or("", String::as_str);
                let unconfirmed = self.comment_unconfirmed.contains(&key);
                let allowed = !self.has_pending_request() && !draft.trim().is_empty() && !unconfirmed;
                if unconfirmed {
                    comments = comments.push(text("Check the updated comments before posting your draft again.").size(14).color(MUTED));
                }
                comments = comments.push(row![
                    text_input("Add a class comment…", draft).padding(10).width(Fill)
                        .on_input(move |value| Message::EditComment(course_id, stream_item_id, value)),
                    button("@").padding([9, 12]).style(outline_style)
                        .on_press_maybe((!self.has_pending_request())
                            .then_some(Message::OpenCommentMentions(course_id, stream_item_id))),
                    button(if self.posting_comment { "Posting…" } else { "Post" })
                        .padding([9, 13]).style(primary_style)
                        .on_press_maybe(allowed.then_some(Message::PostComment(course_id, stream_item_id))),
                ].spacing(8).align_y(Alignment::Center));
                if let Some(picker) = self.comment_mentions.as_ref()
                    .filter(|picker| picker.course == course_id && picker.item == stream_item_id)
                {
                    let mut list = column![
                        row![
                            text("Mention a class member").size(15).font(medium()).width(Fill),
                            button("Close").style(quiet_style).on_press(Message::CloseCommentMentions),
                        ].align_y(Alignment::Center)
                    ].spacing(6);
                    if let Some(error) = &picker.error {
                        list = list.push(text(error).size(13).color(color!(0x9b2722)));
                    } else if let Some(entries) = &picker.entries {
                        if entries.is_empty() {
                            list = list.push(text("No member email addresses are available.").size(13).color(MUTED));
                        }
                        for member in entries {
                            list = list.push(button(column![
                                text(&member.name).size(14),
                                text(&member.email).size(12).color(MUTED),
                            ].spacing(2)).width(Fill).padding(8).style(quiet_style)
                                .on_press(Message::InsertCommentMention(course_id, stream_item_id, member.id)));
                        }
                    } else {
                        list = list.push(text("Loading class members…").size(13).color(MUTED));
                    }
                    comments = comments.push(container(scrollable(list).height(200)).padding(10)
                        .style(|_| outline_surface(9.0)).width(Fill));
                }
            }
        }
        match item.comments.as_deref() {
            Some(entries) if !entries.is_empty() => {
                for comment in entries {
                    comments = comments.push(self.comment_card(course_id, item.id.parse().ok(), CommentAudience::Class, comment));
                }
            }
            Some(_) => comments = comments.push(text("No class comments returned.").size(14).color(MUTED)),
            None if !self.can_read_pages() => {
                comments = comments.push(text("Comments are unavailable in this copy.").size(14).color(MUTED));
            }
            None => {}
        }
        body = body.push(comments);
        scrollable(body.spacing(15).padding([19, 27])).width(Fill).height(Fill).into()
    }

    fn comment_card<'a>(
        &'a self, course: i64, item: Option<i64>, audience: CommentAudience, comment: &'a Comment,
    ) -> Element<'a, Message> {
        let byline = match &comment.created_label {
            Some(label) => format!("{} · {label}", comment.author),
            None => comment.author.clone(),
        };
        let mut heading = row![text(byline).size(14).font(medium()).color(FG).width(Fill)].align_y(Alignment::Center);
        let mut confirmation = None;
        if let (Some(item), Ok(id)) = (item, comment.id.parse())
            && self.may_delete_visible_comment(course, audience, comment)
            && (self.can_compose_comment(course, item) || self.screenshot_directory.is_some())
        {
            let target = CommentTarget { course, item, id, audience };
            let ready = !self.has_pending_request();
            if self.confirming_comment_delete == Some(target) {
                confirmation = Some(column![
                    text("Delete this comment?").size(14).color(FG),
                    row![
                        button("Cancel").style(quiet_style).padding([5, 8]).on_press(Message::CancelDeleteComment),
                        button("Delete").style(outline_style).padding([5, 8])
                            .on_press_maybe(ready.then_some(Message::DeleteComment(target))),
                    ].spacing(6),
                ].spacing(6));
            } else {
                heading = heading.push(button(if self.deleting_comment == Some(target) { "Deleting…" } else { "Delete…" })
                    .style(quiet_style).padding([4, 7])
                    .on_press_maybe(ready.then_some(Message::ConfirmDeleteComment(target))));
            }
        }
        let mut body = column![heading, text(&comment.body).size(14).color(MUTED)].spacing(5);
        if let Some(confirmation) = confirmation {
            body = body.push(confirmation);
        }
        container(body).width(Fill).padding(12).style(|_| outline_surface(8.0)).into()
    }

    fn active_course_options(&self) -> Vec<ClassFilterOption> {
        let mut classes: Vec<_> = self.snapshot.iter().flat_map(|data| &data.courses)
            .filter(|course| home_course(course))
            .map(|course| ClassFilterOption { id: Some(course.id), label: course.title.clone() })
            .collect();
        classes.sort_by_cached_key(|course| course.label.to_lowercase());
        classes.insert(0, ClassFilterOption { id: None, label: "All classes".into() });
        classes
    }

    fn class_filter_picker(
        &self, class_id: Option<i64>, on_select: fn(ClassFilterOption) -> Message,
    ) -> Element<'_, Message> {
        let classes = self.active_course_options();
        let selected = classes.iter().find(|class| class.id == class_id).cloned();
        pick_list(classes, selected, on_select)
            .width(190).text_size(14).padding(8)
            .style(|_, status| {
                let active = matches!(status, pick_list::Status::Active);
                pick_list::Style {
                    text_color: FG,
                    placeholder_color: MUTED,
                    handle_color: ACTIVE_FG,
                    background: (if active { WHITE } else { ACTIVE_BG }).into(),
                    border: border(if active { BORDER } else { ACTIVE_FG }, 1.0, 8.0),
                }
            })
            .into()
    }

    fn calendar_items(&self) -> BTreeMap<NaiveDate, Vec<(&Coursework, &Course)>> {
        let mut by_day: BTreeMap<_, Vec<_>> = BTreeMap::new();
        if let Some(data) = &self.snapshot {
            for course in data.courses.iter().filter(|course| {
                home_course(course) && self.calendar_class.is_none_or(|id| id == course.id)
            }) {
                for item in data.coursework_for(course.id).unwrap_or_default() {
                    if let Some(date) = item.due_date(&Local)
                        && monday_of(date) == self.calendar_monday
                    {
                        by_day.entry(date).or_default().push((item, course));
                    }
                }
            }
        }
        for items in by_day.values_mut() {
            items.sort_by_key(|(item, _)| item.due_at_millis);
        }
        by_day
    }

    fn calendar(&self) -> Element<'_, Message> {
        let today = Local::now().date_naive();
        let monday = self.calendar_monday;
        let sunday = monday + Days::new(6);
        let span = if monday.year() != sunday.year() {
            format!("{} – {}", monday.format("%-d %b %Y"), sunday.format("%-d %b %Y"))
        } else if monday.month() != sunday.month() {
            format!("{} – {}", monday.format("%-d %b"), sunday.format("%-d %b %Y"))
        } else {
            format!("{}–{}", monday.day(), sunday.format("%-d %b %Y"))
        };
        let mut toolbar = row![
            button("‹").padding([7, 12]).style(outline_style).on_press(Message::CalendarShift(-1)),
            button("›").padding([7, 12]).style(outline_style).on_press(Message::CalendarShift(1)),
            button("Today").padding([7, 12]).style(outline_style).on_press(Message::CalendarToday),
            text(span).size(18).font(medium()).color(FG),
            self.class_filter_picker(self.calendar_class, Message::CalendarFilter),
        ].spacing(9).align_y(Alignment::Center);
        if let Some(calendar) = self.calendar_class.and_then(|course| self.class_calendar_button(course)) {
            toolbar = toolbar.push(calendar);
        }
        let toolbar = toolbar.wrap().vertical_spacing(9);

        let items = self.calendar_items();
        let mut body = column![text("Calendar").size(27).color(FG), toolbar].spacing(14);
        let status = self.page_read_status(PageRead::Assignments, self.assignments_loaded, "Loading due dates…");
        if let Some(status) = status {
            body = body.push(status);
        } else if items.is_empty() {
            body = body.push(text("No work due this week.").size(14).color(MUTED));
        }

        let week = responsive(move |size| {
            if size.width >= 670.0 {
                return Self::calendar_grid(monday, today, &items);
            }
            let mut agenda = column![].spacing(12);
            for (date, entries) in &items {
                let mut day = column![
                    text(date.format("%A, %-d %B").to_string()).size(15).font(medium()).color(FG)
                ].spacing(7);
                for (item, course) in entries {
                    day = day.push(coursework_row(item, Some(&course.title)));
                }
                agenda = agenda.push(day);
            }
            agenda.into()
        }).height(Length::Shrink);
        scrollable(body.push(week).padding(26).width(Fill))
            .width(Fill).height(Fill).into()
    }

    fn class_calendar_button(&self, course: i64) -> Option<Element<'_, Message>> {
        let snapshot = self.snapshot.as_ref()?;
        if self.verify_live_course_count || (snapshot.source != SnapshotSource::Live && self.screenshot_directory.is_none()) {
            return None;
        }
        let url = snapshot.courses.iter().find(|entry| entry.id == course)?.calendar_url.as_ref()?;
        Some(button(row![icon(Icon::Calendar, BLUE, 17), text("Google Calendar")].spacing(6).align_y(Alignment::Center))
            .padding([8, 12]).style(outline_style).on_press(Message::OpenAttachment(url.clone())).into())
    }

    fn calendar_grid<'a>(
        monday: NaiveDate,
        today: NaiveDate,
        items: &BTreeMap<NaiveDate, Vec<(&'a Coursework, &'a Course)>>,
    ) -> Element<'a, Message> {
        let mut days = row![].spacing(5);
        for offset in 0..7 {
            let date = monday + Days::new(offset);
            let is_today = today == date;
            let heading = column![
                text(date.format("%a").to_string()).size(12).color(MUTED),
                text(date.day().to_string()).size(18).font(medium())
                    .color(if is_today { ACTIVE_FG } else { FG }),
            ].spacing(1).align_x(Alignment::Center);
            let mut day = column![
                container(heading).width(Fill).center_x(Fill).padding([8, 2])
                    .style(move |_| surface(if is_today { ACTIVE_BG } else { SOFT }, 6.0)),
            ].spacing(5).width(Fill);
            for (item, course) in items.get(&date).into_iter().flatten() {
                let accent = course_color(course);
                let entry = row![
                    container(space().width(3).height(28)).style(move |_| surface(accent, 2.0)),
                    column![
                        text(truncate(&item.title, 36)).size(13).color(FG).width(Fill)
                            .wrapping(text::Wrapping::WordOrGlyph),
                        text(truncate(&course.title, 22)).size(11).color(MUTED).width(Fill)
                            .wrapping(text::Wrapping::WordOrGlyph),
                    ].spacing(3).width(Fill),
                ].spacing(4).align_y(Alignment::Center);
                let open = button(entry).padding([6, 4]).width(Fill).style(outline_style)
                    .on_press(Message::OpenItem(item.course_id, item.id.clone()));
                let details = column![
                    text(&item.title).size(14).font(medium()).color(FG),
                    text(&course.title).size(13).color(MUTED),
                    text(item.due_label.as_deref().unwrap_or_default()).size(13).color(MUTED),
                ].spacing(3);
                day = day.push(tooltip(open, details, tooltip::Position::Bottom)
                    .style(|_| outline_surface(8.0)));
            }
            days = days.push(container(day).width(Length::FillPortion(1)).padding(4)
                .style(|_| outline_surface(7.0)));
        }

        days.into()
    }

    fn todo_items(&self, today: NaiveDate) -> BTreeMap<TodoGroup, Vec<(&Coursework, &Course)>> {
        let mut groups: BTreeMap<_, Vec<_>> = BTreeMap::new();
        if let Some(data) = &self.snapshot {
            for course in data.courses.iter().filter(|course| {
                home_course(course) && self.todo_class.is_none_or(|id| id == course.id)
            }) {
                for item in data.coursework_for(course.id).unwrap_or_default().iter().filter(|item| has_submission(item)) {
                    let status = item.submission.as_ref().map(|submission| submission.status);
                    let group = match TodoFilter::from_status(status) {
                        Some(filter) if filter == self.todo_filter => {
                            let completed = filter == TodoFilter::Done && status != Some(SubmissionStatus::Excused);
                            TodoGroup::for_deadline(item.due_date(&Local), today, completed)
                        }
                        None if self.todo_filter == TodoFilter::Assigned => TodoGroup::StatusUnavailable,
                        _ => continue,
                    };
                    groups.entry(group).or_default().push((item, course));
                }
            }
        }
        for items in groups.values_mut() {
            items.sort_by_key(|(item, _)| item.due_at_millis);
            if self.todo_filter != TodoFilter::Assigned {
                items.reverse();
            }
        }
        groups
    }

    fn todo(&self) -> Element<'_, Message> {
        let filters = row(TodoFilter::ALL.into_iter().map(|filter| {
            let active = self.todo_filter == filter;
            button(filter.label()).padding([8, 12]).style(move |_, status| nav_style(active, status))
                .on_press(Message::SetTodoFilter(filter)).into()
        })).spacing(6);
        let toolbar = row![filters, self.class_filter_picker(self.todo_class, Message::FilterTodoClass)]
            .spacing(14).align_y(Alignment::Center).wrap().vertical_spacing(8);
        let mut body = column![text("To-do").size(27).color(FG), toolbar].spacing(14);
        let status = self.page_read_status(PageRead::Assignments, self.assignments_loaded, "Loading assignments…");
        let pending = status.is_some();
        if let Some(status) = status {
            body = body.push(status);
        }
        let groups = self.todo_items(Local::now().date_naive());
        if groups.is_empty() && !pending {
            body = body.push(empty_panel(
                Icon::Check, "No matching work", "There are no assignments or questions in this view.",
            ));
        }
        for group in self.todo_filter.groups() {
            let Some(entries) = groups.get(group) else { continue };
            let closed = self.collapsed_todo_groups.contains(&(self.todo_filter, *group));
            let heading = button(row![
                text(group.label()).size(16).font(medium()).color(FG),
                space().width(Fill),
                text(entries.len().to_string()).size(14).color(MUTED),
                text(if closed { "⌄" } else { "⌃" }).size(16).color(MUTED),
            ].spacing(12).align_y(Alignment::Center))
                .style(outline_style).width(Fill).padding([10, 13]).on_press(Message::ToggleTodoGroup(*group));
            body = body.push(heading);
            if !closed {
                for (item, course) in entries {
                    body = body.push(coursework_row(item, Some(&course.title)));
                }
            }
        }
        scrollable(body.padding(26).width(Fill)).width(Fill).height(Fill).into()
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

fn empty_panel(
    glyph: Icon, title: &'static str, body: &'static str,
) -> Element<'static, Message> {
    let panel = column![
        container(icon(glyph, ACTIVE_FG, 25))
            .center_x(52).center_y(52).style(|_| surface(ACTIVE_BG, 26.0)),
        text(title).color(FG).font(medium()).size(17),
        text(body).color(MUTED).size(14),
    ]
    .spacing(10).align_x(Alignment::Center);
    container(panel).width(Fill).center_x(Fill).padding([32, 15])
        .style(|_| outline_surface(10.0)).into()
}

fn medium() -> Font {
    Font { weight: Weight::Medium, ..CLASSROOM_FONT }
}

fn stream_editor_binding(course: i64, event: text_editor::KeyPress) -> Option<text_editor::Binding<Message>> {
    if matches!(event.status, text_editor::Status::Focused { .. }) && event.modifiers.command() {
        let format = match event.key.to_latin(event.physical_key) {
            Some('b') => Some(StreamFormat::Bold),
            Some('i') => Some(StreamFormat::Italic),
            Some('u') => Some(StreamFormat::Underline),
            _ => None,
        };
        if let Some(format) = format {
            return Some(text_editor::Binding::Custom(Message::FormatStreamDraft(course, format)));
        }
    }
    text_editor::Binding::from_key_press(event)
}

fn paragraph_start(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset.min(text.len())].iter().rposition(|&byte| byte == b'\n').map_or(0, |index| index + 1)
}

fn paragraph_starts(text: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(text.match_indices('\n').map(|(index, _)| index + 1))
}

fn formatted_description(body: &str, styles: &[DescriptionStyle], bullets: &[usize]) -> Element<'static, Message> {
    if bullets.is_empty() {
        return formatted_inline(body, styles);
    }
    let mut paragraphs = column![].spacing(4);
    for start in paragraph_starts(body) {
        let line = body[start..].split('\n').next().unwrap_or_default().trim_end_matches('\r');
        let end = start + line.len();
        let mut inline = Vec::new();
        if let Some(previous) = styles.iter().filter(|style| style.start <= start).max_by_key(|style| style.start) {
            inline.push(DescriptionStyle { start: 0, ..*previous });
        }
        inline.extend(styles.iter().filter(|style| (start + 1..end).contains(&style.start))
            .map(|style| DescriptionStyle { start: style.start - start, ..*style }));
        let content = formatted_inline(line, &inline);
        if bullets.contains(&start) {
            paragraphs = paragraphs.push(row![text("•").size(15).color(FG).width(15), content].spacing(3));
        } else if line.is_empty() {
            paragraphs = paragraphs.push(space().height(15));
        } else {
            paragraphs = paragraphs.push(content);
        }
    }
    paragraphs.width(Fill).into()
}

fn formatted_inline(body: &str, styles: &[DescriptionStyle]) -> Element<'static, Message> {
    if styles.iter().all(|style| style.is_plain()) {
        return text(body.to_owned()).size(15).color(FG).wrapping(text::Wrapping::WordOrGlyph).into();
    }
    let mut runs = BTreeMap::from([(0, DescriptionStyle::default())]);
    runs.extend(styles.iter().copied().filter(|style| style.start < body.len() && body.is_char_boundary(style.start))
        .map(|style| (style.start, style)));
    let mut runs = runs.into_iter().peekable();
    let mut spans: Vec<text::Span<'static, (), Font>> = Vec::new();
    while let Some((start, style)) = runs.next() {
        let end = runs.peek().map_or(body.len(), |(next, _)| *next);
        spans.push(span(body[start..end].to_owned()).font(Font {
            weight: if style.bold { Weight::Bold } else { Weight::Normal },
            style: if style.italic { iced::font::Style::Italic } else { iced::font::Style::Normal },
            ..FORMATTED_FONT
        }).underline(style.underline));
    }
    rich_text(spans).size(15).width(Fill).wrapping(text::Wrapping::WordOrGlyph).into()
}

fn truncate(label: &str, max: usize) -> String {
    let mut chars = label.chars();
    let mut text: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        text.push('…');
    }
    text
}

async fn choose_attachment_file() -> Result<Option<PathBuf>, String> {
    let output = tokio::process::Command::new("/usr/bin/osascript")
        .args(["-e", "POSIX path of (choose file with prompt \"Choose a file to attach\")"])
        .output().await.map_err(|_| "Could not open the file picker.")?;
    if !output.status.success() {
        if String::from_utf8_lossy(&output.stderr).contains("(-128)") {
            return Ok(None);
        }
        return Err("Could not choose a file.".into());
    }
    let path = String::from_utf8(output.stdout).map_err(|_| "The picker returned an unreadable path.")?;
    let path = path.strip_suffix('\n').unwrap_or(&path);
    Ok(Some(PathBuf::from(path)))
}

fn first_initial(name: &str) -> String {
    name.chars()
        .find(|letter| letter.is_alphanumeric())
        .map(|letter| letter.to_uppercase().collect())
        .unwrap_or_else(|| "C".to_owned())
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

fn hidden_course(course: &Course) -> bool {
    course.state.shows_on_home() && course.hidden == Some(true)
}

fn course_color(course: &Course) -> Color {
    if let Some([r, g, b]) = course.theme.as_ref().and_then(|theme| theme.color_rgb) {
        Color::from_rgb8(r, g, b)
    } else {
        CARD_COLORS[course.id.rem_euclid(CARD_COLORS.len() as i64) as usize]
    }
}

fn course_banner<'a>(
    course: &'a Course, artwork: Option<&image::Handle>,
    height: f32, hero: bool, teacher: Option<&'a str>,
) -> Element<'a, Message> {
    let accent = course_color(course);
    let radius = if hero { iced::border::radius(11.0) } else { iced::border::radius(11.0).bottom(0.0) };
    let mut labels = column![
        text(if hero { course.title.clone() } else { truncate(&course.title, 24) })
            .size(if hero { 34 } else { 22 }).font(medium()).color(WHITE),
        text(if hero { course.subtitle.clone() } else { truncate(&course.subtitle, 32) })
            .size(if hero { 16 } else { 14 }).color(WHITE.scale_alpha(0.94)),
    ].spacing(4);
    if let Some(teacher) = teacher {
        labels = labels.push(text(truncate(teacher, 32)).size(13).color(WHITE.scale_alpha(0.94)));
    }
    let labels = container(labels).width(Fill).height(height)
        .align_y(if hero { Alignment::End } else { Alignment::Start })
        .padding(if hero { 24 } else { 15 });

    if let Some(handle) = artwork {
        let backdrop = container(
            image(handle.clone()).width(Fill).height(height).content_fit(ContentFit::Cover).border_radius(radius),
        ).width(Fill).height(height).style(move |_| surface(accent, radius)).clip(true);
        let overlay = labels.style(move |_| {
            let gradient = iced::gradient::Linear::new(iced::Radians(std::f32::consts::FRAC_PI_2))
                .add_stop(0.0, Color::BLACK.scale_alpha(0.68))
                .add_stop(0.76, Color::BLACK.scale_alpha(0.36))
                .add_stop(1.0, Color::BLACK.scale_alpha(0.24));
            container::Style {
                background: Some(Background::Gradient(gradient.into())),
                ..surface(Color::TRANSPARENT, radius)
            }
        });
        container(stack![backdrop, overlay].width(Fill).height(height))
            .width(Fill).height(height).clip(true).into()
    } else {
        labels.style(move |_| {
            let luminance = accent.r * 0.2126 + accent.g * 0.7152 + accent.b * 0.0722;
            let background = if luminance > 0.56 {
                Color::from_rgb(accent.r * 0.70, accent.g * 0.70, accent.b * 0.70)
            } else { accent };
            surface(background, radius)
        }).into()
    }
}

fn avatar(label: String, background: Color, diameter: f32, font_size: f32) -> Element<'static, Message> {
    container(text(label).color(WHITE).font(medium()).size(font_size))
        .center_x(diameter)
        .center_y(diameter)
        .style(move |_| surface(background, diameter / 2.0))
        .into()
}

fn pill<'a>(label: impl Into<String>, background: Color, foreground: Color) -> Element<'a, Message> {
    container(text(label.into()).size(13).color(foreground))
        .padding([5, 10])
        .style(move |_| surface(background, 13.0))
        .into()
}

fn border(color: Color, width: f32, radius: impl Into<iced::border::Radius>) -> Border {
    Border { color, width, radius: radius.into() }
}

fn surface(background: Color, radius: impl Into<iced::border::Radius>) -> container::Style {
    container::Style {
        background: Some(Background::Color(background)),
        border: border(Color::TRANSPARENT, 0.0, radius),
        text_color: Some(FG),
        ..Default::default()
    }
}

fn outline_surface(radius: f32) -> container::Style {
    container::Style {
        border: border(BORDER, 1.0, radius),
        ..surface(WHITE, radius)
    }
}

fn divider_style(theme: &Theme) -> rule::Style {
    rule::Style { color: BORDER, ..rule::default(theme) }
}

fn quiet_style(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: match status {
            button::Status::Hovered | button::Status::Pressed => Some(SOFT.into()),
            _ => None,
        },
        text_color: if status == button::Status::Disabled { MUTED } else { FG },
        border: border(Color::TRANSPARENT, 0.0, 7.0),
        ..Default::default()
    }
}

fn nav_style(active: bool, status: button::Status) -> button::Style {
    let mut style = quiet_style(&Theme::Light, status);
    style.border.radius = 23.0.into();
    if active {
        style.background = Some(ACTIVE_BG.into());
        style.text_color = ACTIVE_FG;
    }
    style
}

fn outline_style(_: &Theme, status: button::Status) -> button::Style {
    let mut style = quiet_style(&Theme::Light, status);
    style.text_color = if status == button::Status::Disabled { MUTED } else { ACTIVE_FG };
    style.border = border(BORDER, 1.0, 7.0);
    style
}

fn tab_style(active: bool, status: button::Status) -> button::Style {
    let mut style = quiet_style(&Theme::Light, status);
    style.text_color = if active { ACTIVE_FG } else { MUTED };
    style
}

fn primary_style(_: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => color!(0x1765cc),
        button::Status::Disabled => BORDER,
        _ => BLUE,
    };
    button::Style {
        background: Some(background.into()),
        text_color: if status == button::Status::Disabled { MUTED } else { WHITE },
        border: border(Color::TRANSPARENT, 0.0, 7.0),
        ..Default::default()
    }
}

fn card_style(_: &Theme, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(WHITE.into()),
        text_color: FG,
        border: border(if hovered { color!(0xa1b9d9) } else { BORDER }, 1.0, 12.0),
        shadow: if hovered {
            Shadow { color: Color::BLACK.scale_alpha(0.10), offset: Vector::new(0.0, 3.0), blur_radius: 10.0 }
        } else {
            Shadow::default()
        },
        ..Default::default()
    }
}

fn search_style(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let mut style = text_input::default(theme, status);
    let active = matches!(status, text_input::Status::Focused { .. });
    style.background = if active { WHITE.into() } else { SOFT.into() };
    style.border = border(if active { BLUE } else { Color::TRANSPARENT }, 1.0, 8.0);
    style.placeholder = MUTED;
    style.value = FG;
    style
}

#[derive(Clone, Copy)]
enum Icon {
    Menu, Home, Calendar, Check, Classroom, Refresh, Arrow, Book, Feed, People, Info, Folder, Video, Settings, Grip,
}

impl Icon {
    fn path(self) -> &'static str {
        match self {
            Self::Grip => "M9 5h.01m6 0h.01M9 12h.01m6 0h.01M9 19h.01m6 0h.01",
            Self::Menu => "M3 6h18M3 12h18M3 18h18",
            Self::Home => "M3 11l9-8 9 8M5 10v10h5v-6h4v6h5V10",
            Self::Calendar => "M7 3v4m10-4v4M4 10h16M5 5h14a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1Z",
            Self::Check => "M5 4h14a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1Zm3 8 3 3 5-6",
            Self::Classroom => "M4 4h16a1 1 0 0 1 1 1v12H3V5a1 1 0 0 1 1-1Zm3 16h10m-8-7 3-2 3 2",
            Self::Refresh => "M20 11a8 8 0 1 0-2.4 6M20 4v7h-7",
            Self::Arrow => "m9 6 6 6-6 6",
            Self::Book => "M5 3h14v18H6a2 2 0 0 1 0-4h13M5 3v16m4-11h6m-6 4h5",
            Self::Feed => "M4 4h16v16H4zm4 5h8m-8 4h8m-8 4h4",
            Self::People => "M16 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2m16 0v-2a4 4 0 0 0-2-3.46M13.5 7.5a3.5 3.5 0 1 1-7 0 3.5 3.5 0 0 1 7 0Zm4 3.4a3.5 3.5 0 0 0 0-6.8",
            Self::Info => "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20Zm0-11v5m0-8h.01",
            Self::Folder => "M3 6h7l2 2h9v11H3V6Z",
            Self::Video => "M3 6h12v12H3zM15 10l6-3v10l-6-3",
            Self::Settings => "M10 3h4l1 3 3 1 3 2-1 3 1 3-3 2-3 1-1 3h-4l-1-3-3-1-3-2 1-3-1-3 3-2 3-1zM15 12a3 3 0 1 0-6 0 3 3 0 0 0 6 0Z",
        }
    }
}

fn icon(glyph: Icon, color: Color, size: impl Into<Pixels>) -> Element<'static, Message> {
    let size = size.into();
    let path = glyph.path();
    let markup = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="{path}" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/></svg>"#);
    svg(svg::Handle::from_memory(markup.into_bytes()))
        .width(size).height(size)
        .style(move |_, _| svg::Style { color: Some(color) })
        .into()
}

fn classroom_mark() -> Element<'static, Message> {
    let artwork = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 28">
      <rect x="1" y="2" width="30" height="24" rx="3.5" fill="#197A50"/>
      <rect x="3" y="4" width="26" height="20" rx="2" fill="#229664"/>
      <circle cx="16" cy="11" r="3.2" fill="#fff"/>
      <circle cx="8.8" cy="12.2" r="2.4" fill="#C9E9D7"/>
      <circle cx="23.2" cy="12.2" r="2.4" fill="#C9E9D7"/>
      <path d="M4.8 20a4 4 0 0 1 7.1-2.5l-.6 2.5zm16 0-.6-2.5a4 4 0 0 1 7.1 2.5z" fill="#C9E9D7"/>
      <path d="M10.5 20v-1a5.5 5.5 0 1 1 11 0v1z" fill="#fff"/>
    </svg>"##;
    svg(svg::Handle::from_memory(artwork.as_slice()))
        .width(Pixels(31.0)).height(Pixels(27.0)).into()
}

fn coursework_icon(kind: CourseworkKind) -> Icon {
    match kind {
        CourseworkKind::Announcement => Icon::Feed,
        CourseworkKind::Assignment => Icon::Check,
        CourseworkKind::Question => Icon::Info,
        CourseworkKind::Material => Icon::Book,
    }
}

fn coursework_row<'a>(item: &'a Coursework, course_name: Option<&'a str>) -> Element<'a, Message> {
    coursework_row_with_preview(item, course_name, true)
}

fn coursework_row_with_preview<'a>(item: &'a Coursework, course_name: Option<&'a str>, preview: bool) -> Element<'a, Message> {
    let glyph = coursework_icon(item.kind);
    let subtitle = match course_name {
        Some(course) => format!("{course} · {}", item.kind.label()),
        None => item.kind.label().into(),
    };
    let mut body = column![
        text(&item.title).font(medium()).size(16).color(FG),
        text(subtitle).size(13).color(MUTED)
    ].spacing(4);
    if preview && !item.description.trim().is_empty() {
        body = body.push(text(truncate(&item.description, 105)).size(14).color(MUTED));
    }
    if preview && let Some(submission) = &item.submission {
        let label = grade_label(submission).unwrap_or_else(|| submission.status.label().into());
        body = body.push(text(label).size(13).color(ACTIVE_FG));
    }
    let due: Element<'_, Message> = match item.due_label.as_ref().filter(|_| preview) {
        Some(label) => pill(label, SOFT, MUTED),
        None => space().width(0).into(),
    };
    button(row![
        container(icon(glyph, ACTIVE_FG, 20)).center_x(42).center_y(42)
            .style(|_| surface(ACTIVE_BG, 21.0)),
        body.width(Fill),
        due,
        icon(Icon::Arrow, MUTED, 18),
    ].spacing(13).align_y(Alignment::Center))
        .padding(16).width(Fill).style(card_style)
        .on_press(Message::OpenItem(item.course_id, item.id.clone()))
        .into()
}

fn valid_stream_draft(body: &str) -> bool {
    let body = body.trim();
    !body.is_empty() && body.len() <= 10_000
}

fn notification_switch(
    enabled: bool, saving: bool, ready: bool, change: impl Fn(bool) -> Message + 'static,
) -> Element<'static, Message> {
    let status = if saving { "Saving…" } else if enabled { "On" } else { "Off" };
    toggler(enabled).label(status).size(19).text_size(14).width(112)
        .on_toggle_maybe(ready.then_some(change)).into()
}

fn grade_input_matches_saved(input: &str, row: &desktop_auth::TeacherSubmission) -> bool {
    let input = input.trim();
    match row.draft_grade.or(row.released_grade) {
        Some(saved) => input.parse::<f64>().ok() == Some(saved),
        None => input.is_empty(),
    }
}

fn topic_action_valid(action: &desktop_auth::TeacherTopicAction) -> bool {
    use desktop_auth::{TeacherTopicAction, normalized_topic_name};
    match action {
        TeacherTopicAction::Create(name) => normalized_topic_name(name).is_some(),
        TeacherTopicAction::Rename { topic, name } =>
            normalized_topic_name(name).is_some_and(|name| name != topic.title),
        TeacherTopicAction::Delete(topic) => !topic.id.is_empty(),
    }
}

fn next_editable_gradebook_student(grid: &desktop_auth::TeacherGradebook, item: i64, student: i64) -> Option<i64> {
    let mut ids = grid.students.iter().filter_map(|student| student.id.parse::<i64>().ok());
    ids.find(|id| *id == student)?;
    ids.find(|id| grid.cells.get(&(*id, item)).is_some_and(|cell| cell.can_edit()))
}

fn grade_label(submission: &Submission) -> Option<String> {
    let grade = submission.grade.as_ref()?;
    Some(match grade.maximum {
        Some(maximum) => format!("Released grade: {} / {maximum}", grade.earned),
        None => format!("Released grade: {}", grade.earned),
    })
}

fn trusted_attachment_host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    if url.chars().any(|c| c.is_whitespace() || c.is_control() || c == '\\') {
        return None;
    }
    let host = rest.split(['/', '?', '#']).next()?;
    let mut labels = host.rsplit('.');
    let tld = labels.next()?;
    if tld.len() < 2 || !tld.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let labels: Vec<_> = labels.collect();
    (!labels.is_empty()
        && labels.iter().all(|label| valid_host_label(label))
        && !matches!(tld.to_ascii_lowercase().as_str(), "localhost" | "local" | "internal"))
        .then_some(host)
}

fn matching_personal_copy<'a>(item: &'a Coursework, template: &desktop_data::Attachment) -> Option<&'a desktop_data::Attachment> {
    let mut matches = item.submission.as_ref()?.attachments.iter().filter(|file| file.is_personal_copy_of(template));
    let copy = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    (copy.kind == desktop_data::AttachmentKind::DriveFile).then_some(copy)
}

fn append_comment_mention(draft: &mut String, email: &str) {
    let prefix = if draft.is_empty() || draft.ends_with(char::is_whitespace) { "" } else { " " };
    if draft.len() + prefix.len() + email.len() + 2 <= 10_000 {
        draft.push_str(prefix);
        draft.push('+');
        draft.push_str(email);
        draft.push(' ');
    }
}

fn valid_host_label(label: &str) -> bool {
    !label.is_empty() && !label.starts_with('-') && !label.ends_with('-')
        && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn cache_path() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    Some(home.join("Library/Application Support/Classroom for Mac/offline.sqlite3"))
}

fn schedule_capture(shot: Shot) -> Task<Message> {
    // The renderer captures the last draw. Keep the delay and capture in one
    // task so an intermediate app message cannot rebuild the editor's layout.
    Task::future(async { tokio::time::sleep(Duration::from_millis(425)).await })
        .then(move |()| window::latest().then(move |id| match id {
            Some(id) => window::screenshot(id)
                .map(move |image| Message::ScreenshotReady(shot, image)),
            None => iced::exit(),
        }))
}

fn save_screenshot(
    directory: &Path, shot: Shot, image: &window::Screenshot,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(directory)?;
    let name = match shot {
        Shot::Dashboard => "dashboard.png",
        Shot::Invitations => "invitations.png",
        Shot::DeclineInvitation => "decline-invitation.png",
        Shot::Classwork => "classwork.png",
        Shot::ClassworkTopic => "classwork-topic.png",
        Shot::YourWork => "your-work.png",
        Shot::ToDo => "to-do.png",
        Shot::Question => "question.png",
        Shot::ShortAnswer => "short-answer.png",
        Shot::SubmittedShortAnswer => "short-answer-submitted.png",
        Shot::ShortAnswerEdit => "short-answer-edit.png",
        Shot::MultipleChoiceDraft => "multiple-choice-draft.png",
        Shot::Rubric => "rubric.png",
        Shot::Calendar => "calendar.png",
        Shot::ClassCalendar => "class-calendar.png",
        Shot::CompactCalendar => "calendar-compact.png",
        Shot::CompactClasswork => "classwork-compact.png",
        Shot::Settings => "settings.png",
        Shot::StreamComposer => "stream-composer.png",
        Shot::DrivePicker => "drive-picker.png",
        Shot::SubmissionDrivePicker => "submission-drive-picker.png",
        Shot::StreamDelete => "stream-delete.png",
        Shot::CommentDelete => "comment-delete.png",
        Shot::CompactStream => "stream-compact.png",
        Shot::HiddenClasses => "hidden-classes.png",
        Shot::MoveCourse => "move-course.png",
        Shot::ArchiveClass => "archive-class.png",
        Shot::ClassroomLink => "classroom-link.png",
        Shot::CondensedStream => "stream-condensed.png",
        Shot::ExpandedStream => "stream-expanded.png",
        Shot::HiddenStream => "stream-hidden-classwork.png",
        Shot::MarkDone => "mark-as-done.png",
        Shot::NewSubmissionDocument => "new-submission-document.png",
        #[cfg(target_os = "macos")]
        Shot::CameraFraming => "camera-viewfinder.png",
        #[cfg(target_os = "macos")]
        Shot::PersonalCopy => "personal-drive-copy.png",
        #[cfg(target_os = "macos")]
        Shot::TeacherDrafts => "teacher-drafts.png",
        #[cfg(target_os = "macos")]
        Shot::AssignmentEditor => "assignment-draft-editor.png",
        #[cfg(target_os = "macos")]
        Shot::AssignmentSchedule => "assignment-schedule.png",
        #[cfg(target_os = "macos")]
        Shot::TeacherStudentWork => "teacher-student-work.png",
        #[cfg(target_os = "macos")]
        Shot::TeacherGradebook => "teacher-gradebook.png",
        #[cfg(target_os = "macos")]
        Shot::TeacherGradebookEdit => "teacher-gradebook-edit.png",
        #[cfg(target_os = "macos")]
        Shot::BulkReturn => "teacher-bulk-return.png",
        #[cfg(target_os = "macos")]
        Shot::TeacherFeedback => "teacher-private-feedback.png",
        #[cfg(target_os = "macos")]
        Shot::TeacherModerationComment => "teacher-moderation-comment.png",
        #[cfg(target_os = "macos")]
        Shot::TeacherModerationPost => "teacher-moderation-post.png",
        #[cfg(target_os = "macos")]
        Shot::ClassStreamSettings => "teacher-stream-settings.png",
        #[cfg(target_os = "macos")]
        Shot::StudentMuting => "teacher-student-muting.png",
        #[cfg(target_os = "macos")]
        Shot::TopicManagement => "teacher-topic-management.png",
        #[cfg(target_os = "macos")]
        Shot::ItemTopic => "teacher-classwork-topic.png",
    };
    let path = directory.join(name);
    let output = fs::File::create(&path)?;
    let mut png = png::Encoder::new(output, image.size.width, image.size.height);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()?.write_image_data(image.as_ref())?;
    println!("Saved {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmed_topic_deletion_preserves_posts_and_keeps_empty_topics_out_of_the_published_cache() {
        let (mut app, snapshot) = fixture();
        let original = snapshot.coursework_for(1).unwrap().to_vec();
        let id = original.iter().find_map(|item| item.topic_id.clone()).unwrap();
        app.install_snapshot(snapshot);
        app.screen = Screen::Course(1);
        app.selected_topic = Some(id.clone());
        let empty = desktop_data::Topic { id: "empty".into(), title: "Independent reading".into() };
        app.apply_teacher_topics(1, vec![empty.clone()], Some(&id));
        let snapshot = app.snapshot.as_ref().unwrap();
        assert_eq!(snapshot.coursework_for(1).unwrap().len(), original.len());
        for (item, old) in snapshot.coursework_for(1).unwrap().iter().zip(original) {
            assert_eq!(item.id, old.id);
            assert_eq!(item.title, old.title);
            assert_ne!(item.topic_id.as_deref(), Some(id.as_str()));
        }
        assert!(snapshot.topics_for(1).unwrap().is_empty());
        assert_eq!(app.teacher_topics[&1], [empty]);
        assert!(app.selected_topic.is_none());
    }

    #[test]
    fn class_settings_queues_the_read_only_with_a_teacher_role() {
        let (mut app, snapshot) = fixture();
        app.screen = Screen::Course(1);
        app.tab = Tab::ClassSettings;
        app.class_roles.insert(1, CourseRole::Student);
        assert_eq!(app.next_page_read(&snapshot), None);
        app.class_roles.insert(1, CourseRole::Teacher);
        assert_eq!(app.next_page_read(&snapshot), Some(PageRead::Detail(DetailRead::ClassStreamSettings(1))));
    }

    #[test]
    fn only_a_teacher_of_the_class_can_moderate_another_members_public_comment() {
        let (mut app, _) = fixture();
        let comment = Comment { id: "17".into(), author_id: Some(10), author: "Class member".into(),
            body: "A test comment".into(), created_label: None };
        app.stream_viewer_id = Some(12);
        app.class_roles.insert(1, CourseRole::Student);
        app.class_roles.insert(2, CourseRole::Teacher);
        assert!(!app.may_delete_visible_comment(1, CommentAudience::Class, &comment));
        assert!(app.may_delete_visible_comment(2, CommentAudience::Class, &comment));
        assert!(!app.may_delete_visible_comment(2, CommentAudience::Private, &comment));
        app.stream_viewer_id = Some(10);
        assert!(app.may_delete_visible_comment(1, CommentAudience::Class, &comment));
    }

    #[test]
    fn switching_student_keeps_private_drafts_on_their_original_submissions() {
        let (mut app, _) = fixture();
        let row = |student_id| desktop_auth::TeacherSubmission {
            student_id, submission_id: 100 + student_id, remote_state: Some(2), name: None,
            status: SubmissionStatus::TurnedIn, released_grade: None, draft_grade: None, attachments: Vec::new(), answer: None,
        };
        app.screen = Screen::Item(1);
        app.item_id = Some("42".into());
        app.class_roles.insert(1, CourseRole::Teacher);
        app.teacher_work = Some(TeacherWork {
            course: 1, item: 42, filter: StudentWorkFilter::All, students: vec![row(7), row(9)], selected: Some(7),
            grade: String::new(), confirming_return: false, bulk_selected: BTreeSet::new(), bulk_confirming: false,
        });
        let first = app.selected_teacher_feedback_key().unwrap();
        let _ = app.update(Message::EditTeacherFeedback(first, "First thread".into()));
        let _ = app.update(Message::SelectTeacherStudent(9));
        let second = app.selected_teacher_feedback_key().unwrap();
        let _ = app.update(Message::EditTeacherFeedback(first, "Stale edit".into()));
        let _ = app.update(Message::EditTeacherFeedback(second, "Second thread".into()));
        assert_eq!(app.teacher_feedback[&first].draft, "First thread");
        assert_eq!(app.teacher_feedback[&second].draft, "Second thread");
        let _ = app.update(Message::CloseItem);
        assert!(app.teacher_feedback.is_empty());
    }

    #[test]
    fn bulk_return_selection_respects_filter_and_skips_unreturnable_rows() {
        let (mut app, _) = fixture();
        let row = |student_id, status, remote_state| desktop_auth::TeacherSubmission {
            student_id, submission_id: 100 + student_id, remote_state: Some(remote_state),
            name: None, status, released_grade: None, draft_grade: None, attachments: Vec::new(), answer: None,
        };
        app.teacher_work = Some(TeacherWork {
            course: 1, item: 42, filter: StudentWorkFilter::TurnedIn, selected: None, grade: String::new(),
            confirming_return: false, bulk_confirming: false, bulk_selected: BTreeSet::new(),
            students: vec![row(1, SubmissionStatus::TurnedIn, 2), row(2, SubmissionStatus::Assigned, 4)],
        });
        let _ = app.update(Message::SelectVisibleReturns(true));
        let _ = app.update(Message::SelectBulkReturn(2, true));
        assert_eq!(app.teacher_work.as_ref().unwrap().bulk_selected, BTreeSet::from([1]));
        let _ = app.update(Message::FilterStudentWork(StudentWorkFilter::Assigned));
        let _ = app.update(Message::ConfirmBulkReturn(true));
        assert!(app.teacher_work.as_ref().unwrap().bulk_confirming);
        let _ = app.update(Message::ClearBulkReturns);
        assert!(app.teacher_work.as_ref().unwrap().bulk_selected.is_empty());
    }

    #[test]
    fn gradebook_advances_in_roster_order_past_unavailable_cells() {
        let student = |id: i64| CourseMember { id: id.to_string(), display_name: id.to_string(), role: CourseRole::Student };
        let cell = |submission_id, remote_state| desktop_auth::TeacherGradebookCell {
            submission_id, remote_state, status: SubmissionStatus::Assigned, released_grade: None, draft_grade: None,
        };
        let grid = desktop_auth::TeacherGradebook {
            students: vec![student(5), student(9), student(4), student(3)],
            cells: BTreeMap::from([
                ((5, 8), cell(100, Some(4))), ((9, 8), cell(101, Some(6))),
                ((4, 10), cell(102, Some(4))), ((3, 8), cell(103, Some(2))),
            ]),
        };
        assert_eq!(next_editable_gradebook_student(&grid, 8, 5), Some(3));
        assert_eq!(next_editable_gradebook_student(&grid, 8, 3), None);
    }

    #[test]
    fn published_item_queues_class_submissions_only_for_a_confirmed_teacher() {
        let (mut app, mut snapshot) = fixture();
        let item = snapshot.coursework.get_mut(&1).unwrap().iter_mut()
            .find(|row| row.kind == CourseworkKind::Assignment).unwrap();
        item.id = "42".into();
        app.screen = Screen::Item(1);
        app.item_id = Some(item.id.clone());
        app.class_roles.insert(1, CourseRole::Student);
        assert_ne!(app.next_page_read(&snapshot), Some(PageRead::Detail(DetailRead::TeacherSubmissions(1, 42))));
        app.class_roles.insert(1, CourseRole::Teacher);
        assert_eq!(app.next_page_read(&snapshot), Some(PageRead::Detail(DetailRead::TeacherSubmissions(1, 42))));
    }

    #[test]
    fn schedule_input_interprets_the_time_in_the_displayed_local_timezone() {
        let zone = chrono::FixedOffset::east_opt(3600).unwrap();
        let expected = DateTime::parse_from_rfc3339("2027-12-21T12:15:00Z").unwrap().timestamp_millis();
        assert_eq!(local_schedule_millis(&zone, "2027-12-21 13:15"), Some(expected));
        assert_eq!(local_schedule_millis(&zone, "2027-02-29 13:15"), None);
        assert_eq!(local_schedule_millis(&zone, "tomorrow"), None);
    }

    #[test]
    fn classwork_queues_unpublished_reads_only_for_a_confirmed_teacher() {
        let (mut app, snapshot) = fixture();
        app.screen = Screen::Course(1);
        app.tab = Tab::Classwork;
        for role in [None, Some(CourseRole::Student), Some(CourseRole::Teacher)] {
            if let Some(role) = role {
                app.class_roles.insert(1, role);
            }
            let read = app.next_page_read(&snapshot);
            assert_eq!(read, Some(PageRead::Detail(if role == Some(CourseRole::Teacher) {
                DetailRead::TeacherTopics(1)
            } else {
                DetailRead::Submissions(1)
            })));
        }
        app.teacher_topics.insert(1, Vec::new());
        assert_eq!(app.next_page_read(&snapshot), Some(PageRead::Detail(DetailRead::TeacherDrafts(1))));
    }

    #[test]
    fn a_personal_template_only_opens_an_exact_current_submission_copy() {
        let (mut app, snapshot) = fixture();
        app.verify_live_course_count = false;
        let mut item = snapshot.coursework_for(1).unwrap().iter()
            .find(|item| item.kind == CourseworkKind::Assignment).unwrap().clone();
        let template = desktop_data::Attachment {
            title: "Worksheet".into(), remote_id: None, kind: desktop_data::AttachmentKind::DriveFile,
            url: Some("https://docs.google.com/document/d/source/edit".into()),
            drive_sharing: Some(DriveSharing::IndividualCopy), drive_reference: Some("source".into()),
            drive_parent_reference: None,
        };
        let mine = desktop_data::Attachment {
            url: Some("https://docs.google.com/document/d/mine/edit".into()),
            drive_reference: Some("mine".into()), drive_parent_reference: Some("source".into()),
            drive_sharing: None, ..template.clone()
        };
        assert_eq!(matching_personal_copy(&item, &template), None);
        item.submission = Some(Submission { attachments: vec![mine.clone()], ..Default::default() });
        assert!(matches!(
            matching_personal_copy(&item, &template).and_then(|copy| app.attachment_action(copy)),
            Some(Message::OpenAttachment(url)) if Some(&url) == mine.url.as_ref()
        ));
        item.submission.as_mut().unwrap().attachments[0].url = template.url.clone();
        let copy = matching_personal_copy(&item, &template).unwrap();
        assert_eq!(copy.drive_reference.as_deref(), Some("mine"));
        assert!(app.attachment_action(copy).is_none());
        item.submission.as_mut().unwrap().attachments.push(mine);
        assert_eq!(matching_personal_copy(&item, &template), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn leaving_an_assignment_ignores_its_pending_microphone_permission() {
        let (mut app, snapshot) = fixture();
        app.install_snapshot(snapshot);
        app.screen = Screen::Item(1);
        app.item_id = Some("42".into());
        app.submission_capture = Some(CaptureState::AskingPermission);
        let generation = app.capture_generation;
        let _ = app.update(Message::Navigate(Screen::Home));
        let _ = app.update(Message::Capture(CaptureAction::Permission(generation, true)));
        assert_eq!(app.screen, Screen::Home);
        assert!(app.submission_capture.is_none());
        assert_ne!(app.capture_generation, generation);
    }

    fn fixture() -> (App, Snapshot) {
        let (app, _) = App::boot(None, true);
        let mut snapshot = Snapshot::demo();
        snapshot.source = SnapshotSource::Live;
        (app, snapshot)
    }

    #[test]
    fn comment_mention_preserves_draft_spacing_and_length_limit() {
        let mut draft = "Thanks".to_owned();
        append_comment_mention(&mut draft, "student@example.org");
        assert_eq!(draft, "Thanks +student@example.org ");
        append_comment_mention(&mut draft, "teacher@example.org");
        assert_eq!(draft, "Thanks +student@example.org +teacher@example.org ");
        let mut full = "x".repeat(9_999);
        append_comment_mention(&mut full, "student@example.org");
        assert_eq!(full.len(), 9_999);
    }

    #[test]
    fn calendar_filters_by_week_and_active_class() {
        let (mut app, mut snapshot) = fixture();
        let today = Local::now().date_naive();
        app.calendar_monday = monday_of(today);
        let original_week = app.calendar_monday;
        let mut archived_item = snapshot.coursework_for(1).unwrap()[1].clone();
        archived_item.course_id = 6;
        snapshot.coursework.get_mut(&6).unwrap().push(archived_item);
        app.install_snapshot(snapshot);

        assert_eq!(app.calendar_items().values().flatten().count(), 4);
        assert!(app.active_course_options().iter().all(|class| class.id != Some(6)));
        let class = app.active_course_options().into_iter().find(|class| class.id == Some(1)).unwrap();
        let _ = app.update(Message::CalendarFilter(class));
        let events: Vec<_> = app.calendar_items().into_values().flatten().collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0.title, "Poetry comparison");

        let _ = app.update(Message::CalendarShift(1));
        assert_eq!(app.calendar_monday, original_week + Days::new(7));
        assert!(app.calendar_items().is_empty());
        let _ = app.update(Message::CalendarShift(-1));
        assert_eq!(app.calendar_monday, original_week);
        let _ = app.update(Message::CalendarShift(-1));
        let _ = app.update(Message::CalendarToday);
        assert_eq!(app.calendar_monday, original_week);

        let _ = app.update(Message::Navigate(Screen::Calendar));
        let (id, item) = app.calendar_items().into_values().flatten().next()
            .map(|(item, course)| (course.id, item.id.clone())).unwrap();
        let _ = app.update(Message::OpenItem(id, item));
        assert_eq!(app.screen, Screen::Item(1));
        let _ = app.update(Message::CloseItem);
        assert_eq!(app.screen, Screen::Calendar);
        assert_eq!(app.calendar_class, Some(1));
    }

    #[test]
    fn calendar_preserves_date_only_deadlines_and_converts_timed_deadlines() {
        let (_, snapshot) = fixture();
        let mut item = snapshot.coursework_for(1).unwrap()[1].clone();
        let midnight = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap();
        item.due_at_millis = Some(midnight.timestamp_millis());
        let timezone = chrono::FixedOffset::west_opt(8 * 60 * 60).unwrap();
        let new_year = midnight.date_naive();

        item.due_has_time = Some(false);
        assert_eq!(item.due_date(&timezone), Some(new_year));
        item.due_has_time = Some(true);
        assert_eq!(item.due_date(&timezone), new_year.pred_opt());
        assert_eq!(monday_of(new_year), NaiveDate::from_ymd_opt(2025, 12, 29).unwrap());

        let mut cached = serde_json::to_value(&item).unwrap();
        cached.as_object_mut().unwrap().remove("due_has_time");
        for (label, expected) in [("Due 1 Jan", Some(new_year)), ("Due 31 Dec, 4:00 PM", new_year.pred_opt())] {
            cached["due_label"] = label.into();
            let legacy = serde_json::from_value::<Coursework>(cached.clone()).unwrap();
            assert_eq!(legacy.due_date(&timezone), expected);
        }
    }

    #[test]
    fn folder_load_discards_the_previous_session_result_and_hides_links_offline() {
        let (mut app, mut snapshot) = fixture();
        app.install_snapshot(snapshot.clone());
        let old_generation = app.class_folders_generation;
        let url = "https://drive.google.com/drive/folders/test";
        app.reset_class_folders();
        let details = || desktop_auth::OwnClassDetails { folders: BTreeMap::from([(1, url.into())]), ..Default::default() };
        let _ = app.update(Message::ClassFoldersLoaded(old_generation, Ok(details())));
        assert!(app.class_folder_url(1).is_none());
        let _ = app.update(Message::ClassFoldersLoaded(app.class_folders_generation, Ok(details())));
        assert_eq!(app.class_folder_url(1), Some(url));
        snapshot.source = SnapshotSource::Cache;
        app.install_snapshot(snapshot);
        assert!(app.class_folder_url(1).is_none());
    }

    #[test]
    fn edited_short_answer_requires_a_change_and_survives_a_failed_request() {
        let (mut app, _) = fixture();
        let key = (4, 42);
        let original = "Original".to_owned();
        let revision = desktop_auth::ShortAnswerRevision {
            submission_id: 67, answer: original.clone(),
        };
        app.short_answer_drafts.insert(key, text_editor::Content::with_text(&original));
        app.short_answer_revisions.insert(key, revision);
        assert!(app.question_draft(key, &QuestionDetails::ShortAnswer).is_none());
        app.short_answer_drafts.insert(key, text_editor::Content::with_text("Revised"));
        assert_eq!(app.question_draft(key, &QuestionDetails::ShortAnswer),
            Some(QuestionAnswer::ShortAnswer("Revised".into())));
        let _ = app.update(Message::QuestionAnswerSent(key.0, key.1, Err("Retry after checking.".into())));
        assert_eq!(app.question_draft(key, &QuestionDetails::ShortAnswer),
            Some(QuestionAnswer::ShortAnswer("Revised".into())));
        assert_eq!(app.error.as_deref(), Some("Retry after checking."));
    }

    #[test]
    fn a_student_choice_is_highlighted_only_when_the_answer_matches_one_option() {
        let choices = vec!["Alpha".into(), "Beta".into(), "Alpha".into()];
        assert_eq!(matching_choice(&choices, "Beta"), Some(1));
        assert_eq!(matching_choice(&choices, "Alpha"), None);
        assert_eq!(matching_choice(&choices, "1"), None);
        let question = QuestionDetails::MultipleChoice(choices);
        assert!(question.accepts(&QuestionAnswer::MultipleChoice("Beta".into())));
        assert!(!question.accepts(&QuestionAnswer::ShortAnswer("Beta".into())));
        assert!(!question.accepts(&QuestionAnswer::MultipleChoice("Alpha".into())));
    }

    #[test]
    fn classwork_uses_returned_sort_keys_and_retains_fallback_order() {
        let (_, mut snapshot) = fixture();
        let items = snapshot.coursework.get_mut(&1).unwrap();
        items.retain(|item| item.kind != CourseworkKind::Announcement);
        assert!(items.len() >= 2);
        for (index, item) in items.iter_mut().enumerate() {
            item.topic_id = Some("demo-poetry-unit".into());
            item.classwork_sort_key = Some(vec![200 - index as u8]);
        }
        let response: Vec<_> = items.iter().map(|item| item.id.clone()).collect();
        let display_order = |snapshot: &Snapshot| snapshot.classwork_sections(1, Some("demo-poetry-unit"))
            .into_iter().flat_map(|section| section.items).map(|item| item.id.clone()).collect::<Vec<_>>();
        assert_eq!(display_order(&snapshot), response.iter().rev().cloned().collect::<Vec<_>>());

        let items = snapshot.coursework.get_mut(&1).unwrap();
        items[0].classwork_sort_key = items[1].classwork_sort_key.clone();
        assert_eq!(display_order(&snapshot), response);
        snapshot.coursework.get_mut(&1).unwrap()[0].classwork_sort_key = None;
        assert_eq!(display_order(&snapshot), response);
    }

    #[test]
    fn classwork_groups_only_published_work_and_keeps_untagged_items_first() {
        let (_, mut snapshot) = fixture();
        let topics = snapshot.topics.get_mut(&1).unwrap();
        topics.insert(0, desktop_data::Topic { id: "topic-first".into(), title: "First topic".into() });
        topics.push(desktop_data::Topic { id: "empty".into(), title: "Empty topic".into() });
        let items = snapshot.coursework.get_mut(&1).unwrap();
        let template = items[1].clone();
        for (id, topic) in [("untagged", None), ("first", Some("topic-first")), ("unresolved", Some("missing"))] {
            let mut item = template.clone();
            item.id = id.into();
            item.topic_id = topic.map(str::to_owned);
            items.push(item);
        }

        let groups = snapshot.classwork_sections(1, None);
        assert_eq!(groups.iter().map(|group| group.title).collect::<Vec<_>>(),
            ["No topic", "First topic", "Poetry", "Topic unavailable"]);
        assert_eq!(groups[0].items[0].id, "untagged");
        assert_eq!(groups[2].items.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(),
            ["demo-poetry", "demo-poetry-reading"]);
        assert_eq!(groups.iter().map(|group| group.items.len()).sum::<usize>(), 5);
        assert_eq!(snapshot.classwork_sections(1, Some("topic-first"))[0].items[0].id, "first");
        assert!(snapshot.classwork_sections(1, Some("empty")).is_empty());
    }

    #[test]
    fn classwork_item_and_topic_preview_state_survives_opening_instructions() {
        let (mut app, snapshot) = fixture();
        app.install_snapshot(snapshot);
        let _ = app.update(Message::Navigate(Screen::Course(1)));
        let _ = app.update(Message::SetTab(Tab::Classwork));
        let item = "demo-poetry";
        let topic = "demo-poetry-unit";
        let key = ClassworkGroupId::Topic(topic.into());
        assert!(!app.classwork_item_expanded(item));

        let _ = app.update(Message::SetClassworkGroups(1, true));
        assert!(app.collapsed_classwork_groups.contains(&key));
        let _ = app.update(Message::ToggleClassworkGroup(1, key.clone()));
        assert!(!app.collapsed_classwork_groups.contains(&key));
        let _ = app.update(Message::ToggleClassworkItem(1, item.into()));
        assert!(app.classwork_item_expanded(item));
        let _ = app.update(Message::OpenItem(1, item.into()));
        assert_eq!(app.screen, Screen::Item(1));
        let _ = app.update(Message::CloseItem);
        assert_eq!((app.screen, app.tab), (Screen::Course(1), Tab::Classwork));
        assert!(app.classwork_item_expanded(item));

        let _ = app.update(Message::SelectTopic(Some(topic.into())));
        assert!(app.classwork_item_expanded(item));
        let _ = app.update(Message::ToggleClassworkItem(1, item.into()));
        assert!(!app.classwork_item_expanded(item));
        let _ = app.update(Message::SelectTopic(None));
        assert!(!app.classwork_item_expanded(item));
        let _ = app.update(Message::SetClassworkGroups(1, false));
        assert!(app.collapsed_classwork_groups.is_empty());
        let _ = app.update(Message::Navigate(Screen::Course(2)));
        assert!(app.classwork_toggles.is_empty());
    }

    #[test]
    fn home_due_soon_uses_local_days_and_open_work_across_active_classes() {
        let (mut app, mut snapshot) = fixture();
        let template = snapshot.coursework_for(1).unwrap()[1].clone();
        snapshot.coursework.clear();
        let now = DateTime::parse_from_rfc3339("2026-12-31T15:00:00-08:00").unwrap();
        let mut add = |course, id: &str, deadline: &str, timed, status| {
            let mut item = template.clone();
            item.course_id = course;
            item.id = id.into();
            item.due_at_millis = DateTime::parse_from_rfc3339(deadline).ok().map(|date| date.timestamp_millis());
            item.due_has_time = Some(timed);
            item.submission = Some(Submission { status, ..Default::default() });
            snapshot.coursework.entry(course).or_default().push(item);
        };
        use SubmissionStatus::*;
        add(2, "today-date", "2026-12-31T00:00:00Z", false, Assigned);
        add(1, "today-time", "2027-01-01T00:00:00Z", true, Assigned);
        add(1, "past-time", "2026-12-31T22:00:00Z", true, Assigned);
        add(1, "past-date", "2026-12-30T00:00:00Z", false, Assigned);
        add(1, "done", "2027-01-01T00:00:00Z", false, TurnedIn);
        add(1, "missing", "2027-01-01T00:00:00Z", false, Missing);
        add(1, "undated", "", false, Assigned);
        add(2, "last-day", "2027-01-06T00:00:00Z", false, Reclaimed);
        add(2, "outside-week", "2027-01-07T00:00:00Z", false, Assigned);
        add(6, "archived", "2026-12-31T00:00:00Z", false, Assigned);

        let upcoming = |selected, limit| snapshot.upcoming_work(&now, selected, limit)
            .into_iter().map(|(item, _)| item.id.as_str()).collect::<Vec<_>>();
        assert_eq!(upcoming(None, 5), ["today-time", "today-date", "last-day"]);
        assert_eq!(upcoming(None, 2), ["today-time", "today-date"]);
        assert_eq!(upcoming(Some(2), 5), ["today-date", "last-day"]);

        app.install_snapshot(snapshot);
        let _ = app.update(Message::OpenItem(1, "today-time".into()));
        assert_eq!(app.screen, Screen::Item(1));
        let _ = app.update(Message::CloseItem);
        assert_eq!(app.screen, Screen::Home);
    }

    #[test]
    fn todo_groups_deadlines_across_local_week_boundaries() {
        let today = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let deadline = |year, month, day| NaiveDate::from_ymd_opt(year, month, day);
        for (due, group) in [
            (None, TodoGroup::NoDueDate),
            (deadline(2025, 12, 28), TodoGroup::LastWeek),
            (deadline(2025, 12, 29), TodoGroup::ThisWeek),
            (deadline(2026, 1, 4), TodoGroup::ThisWeek),
            (deadline(2026, 1, 5), TodoGroup::NextWeek),
            (deadline(2026, 1, 12), TodoGroup::Later),
            (deadline(2025, 12, 21), TodoGroup::Earlier),
        ] {
            assert_eq!(TodoGroup::for_deadline(due, today, false), group);
        }
        assert_eq!(TodoGroup::for_deadline(deadline(2026, 1, 2), today, true), TodoGroup::DoneEarly);
    }

    #[test]
    fn todo_routes_unknown_work_class_filters_and_group_collapsing() {
        let (mut app, mut snapshot) = fixture();
        let today = Local::now().date_naive();
        let mut archived_item = snapshot.coursework_for(1).unwrap()[1].clone();
        archived_item.course_id = 6;
        snapshot.coursework.get_mut(&6).unwrap().push(archived_item);
        app.install_snapshot(snapshot);

        let groups = app.todo_items(today);
        assert_eq!(groups[&TodoGroup::ThisWeek][0].0.id, "demo-poetry");
        assert!(groups[&TodoGroup::StatusUnavailable].iter().any(|(item, _)| item.id == "demo-sort"));
        assert!(groups.values().flatten().all(|(_, course)| course.id != 6));

        let filter = app.active_course_options().into_iter().find(|class| class.id == Some(5)).unwrap();
        let _ = app.update(Message::FilterTodoClass(filter));
        assert!(app.todo_items(today).is_empty());
        let _ = app.update(Message::SetTodoFilter(TodoFilter::Missing));
        let groups = app.todo_items(today);
        assert_eq!(groups[&TodoGroup::ThisWeek][0].0.id, "demo-waves");
        let _ = app.update(Message::ToggleTodoGroup(TodoGroup::ThisWeek));
        assert!(app.collapsed_todo_groups.contains(&(TodoFilter::Missing, TodoGroup::ThisWeek)));
        assert!(!app.collapsed_todo_groups.contains(&(TodoFilter::Assigned, TodoGroup::ThisWeek)));
        let _ = app.update(Message::ToggleTodoGroup(TodoGroup::ThisWeek));
        assert!(app.collapsed_todo_groups.is_empty());

        let all_classes = app.active_course_options().remove(0);
        let _ = app.update(Message::FilterTodoClass(all_classes));
        let _ = app.update(Message::SetTodoFilter(TodoFilter::Done));
        assert_eq!(app.todo_items(today).values().flatten().count(), 1);
    }

    #[test]
    fn your_work_routes_to_the_class_and_filters_only_released_grades() {
        let (mut app, snapshot) = fixture();
        app.install_snapshot(snapshot);
        let _ = app.update(Message::OpenYourWork(2));
        assert_eq!((app.screen, app.tab), (Screen::Course(2), Tab::YourWork));
        let math = &app.snapshot.as_ref().unwrap().coursework_for(2).unwrap()[0];
        let missing = &app.snapshot.as_ref().unwrap().coursework_for(5).unwrap()[0];
        assert!(WorkFilter::ReturnedWithGrade.includes(math));
        assert!(!WorkFilter::Assigned.includes(math));
        assert!(WorkFilter::Missing.includes(missing));
        assert!(!WorkFilter::ReturnedWithGrade.includes(missing));

        let _ = app.update(Message::SetWorkFilter(WorkFilter::Missing));
        let _ = app.update(Message::OpenYourWork(1));
        assert_eq!(app.work_filter, WorkFilter::All);
    }

    #[test]
    fn home_invitations_are_nonblocking_and_reject_stale_results() {
        let (mut app, snapshot) = fixture();
        let duplicate = snapshot.courses[0].clone();
        app.install_snapshot(snapshot);
        app.loading_invitations = true;
        assert!(!app.has_pending_request());
        let old = app.invitation_generation;
        app.invalidate_invitation_read();
        let invited = Course { id: 799, title: "Art fixture".into(), ..Default::default() };
        let _ = app.update(Message::InvitationsLoaded(old, Ok(vec![invited.clone()])));
        assert!(app.invitations.is_none());
        let _ = app.update(Message::InvitationsLoaded(app.invitation_generation, Ok(vec![duplicate, invited])));
        assert_eq!(app.pending_invitations().map(|course| course.id).collect::<Vec<_>>(), [799]);
        assert!(!app.can_respond_to_invitation(799)); // The fixture has no live session.
    }

    #[test]
    fn confirmed_join_waits_for_the_joined_class_in_the_refreshed_list() {
        let (mut app, mut snapshot) = fixture();
        app.install_snapshot(snapshot.clone());
        app.screen = Screen::Join;
        let invited = Course { id: 799, title: "Art fixture".into(), ..Default::default() };
        app.invitations = Some(vec![invited.clone()]);
        let _ = app.update(Message::MembershipFinished(Ok(MembershipChange::Joined(799))));
        assert_eq!(app.screen, Screen::Home);
        assert!(app.pending_invitations().next().is_none());
        assert_eq!(app.opening_joined_course, Some(799));
        let _ = app.update(Message::Refreshed(Ok(snapshot.clone())));
        assert_eq!(app.screen, Screen::Home);
        snapshot.courses.push(invited);
        let _ = app.update(Message::Refreshed(Ok(snapshot)));
        assert_eq!((app.screen, app.tab), (Screen::Course(799), Tab::Stream));
        assert!(app.opening_joined_course.is_none());
    }

    #[test]
    fn declining_an_invitation_keeps_the_current_page_and_other_classes() {
        let (mut app, snapshot) = fixture();
        let enrolled: Vec<_> = snapshot.courses.iter().map(|course| course.id).collect();
        app.install_snapshot(snapshot);
        app.screen = Screen::Join;
        let first = Course { id: 701, title: "Art fixture".into(), ..Default::default() };
        let second = Course { id: 702, title: "Maths fixture".into(), ..Default::default() };
        app.invitations = Some(vec![first.clone(), second.clone()]);
        let previous_generation = app.invitation_generation;
        let _ = app.update(Message::InvitationDeclined(Ok(vec![second])));
        let _ = app.update(Message::InvitationsLoaded(previous_generation, Ok(vec![first])));
        assert_eq!(app.screen, Screen::Join);
        assert_eq!(app.pending_invitations().map(|course| course.id).collect::<Vec<_>>(), [702]);
        assert_eq!(app.snapshot.unwrap().courses.iter().map(|course| course.id).collect::<Vec<_>>(), enrolled);
    }

    #[test]
    fn opening_pages_selects_the_required_reads() {
        let (mut app, mut snapshot) = fixture();
        snapshot.coursework.remove(&1);
        snapshot.roster.remove(&1);
        for (screen, tab, expected) in [
            (Screen::Home, Tab::Stream, PageRead::Assignments),
            (Screen::Calendar, Tab::Stream, PageRead::Assignments),
            (Screen::ToDo, Tab::Stream, PageRead::Assignments),
            (Screen::Join, Tab::Stream, PageRead::Invitations),
            (Screen::Settings, Tab::Stream, PageRead::ClassNotifications),
            (Screen::Course(1), Tab::Stream, PageRead::Coursework(1)),
            (Screen::Course(1), Tab::Classwork, PageRead::Coursework(1)),
            (Screen::Course(1), Tab::YourWork, PageRead::Coursework(1)),
            (Screen::Course(1), Tab::People, PageRead::Detail(DetailRead::Roster(1))),
        ] {
            app.screen = screen;
            app.tab = tab;
            app.invitations = (screen == Screen::Home).then(Vec::new);
            assert_eq!(app.next_page_read(&snapshot), Some(expected), "{screen:?} {tab:?}");
        }

        app.tab = Tab::YourWork;
        let snapshot = Snapshot::demo();
        assert_eq!(app.next_page_read(&snapshot), Some(PageRead::Detail(DetailRead::Submissions(1))));
        app.submissions_checked.insert(1);
        assert_eq!(app.next_page_read(&snapshot), Some(PageRead::Detail(DetailRead::OverallGrade(1))));
        let _ = app.update(Message::DetailLoaded(DetailRead::OverallGrade(1), Ok(DetailData::OverallGrade(None))));
        assert_eq!(app.next_page_read(&snapshot), None);
    }

    #[test]
    fn opening_classwork_fetches_submission_status_after_coursework() {
        let (mut app, mut snapshot) = fixture();
        let items = snapshot.coursework.remove(&1).unwrap();
        app.install_snapshot(snapshot);
        let _ = app.update(Message::Navigate(Screen::Course(1)));
        let _ = app.update(Message::SetTab(Tab::Classwork));
        assert_eq!(app.next_page_read(app.snapshot.as_ref().unwrap()), Some(PageRead::Coursework(1)));

        let _ = app.update(Message::CourseworkLoaded(1, Ok(CourseContents { items, topics: None })));
        let submissions = DetailRead::Submissions(1);
        assert_eq!(app.next_page_read(app.snapshot.as_ref().unwrap()), Some(PageRead::Detail(submissions)));
        let _ = app.update(Message::DetailLoaded(submissions, Ok(DetailData::Submissions(vec![]))));
        assert_eq!(app.next_page_read(app.snapshot.as_ref().unwrap()), None);
    }

    #[test]
    fn drive_auto_pages_short_results_and_stops_at_a_repeated_cursor() {
        let mut picker = DrivePicker::default();
        let page = || DriveFilePage { files: vec![], folders: vec![], next_page: Some("cursor".into()), incomplete: false };
        picker.accept_page(page(), false);
        assert!(picker.needs_more_for_viewport());
        picker.error = Some((true, "Try again".into()));
        assert!(!picker.can_auto_page());
        picker.error = None;
        picker.query = "unsent query".into();
        assert!(!picker.can_auto_page());
        picker.query.clear();
        picker.accept_page(page(), true);
        assert!(!picker.can_auto_page());
        assert!(picker.file_page.unwrap().incomplete);
    }

    #[test]
    fn returning_to_a_page_retries_only_its_failed_reads() {
        let (mut app, mut snapshot) = fixture();
        snapshot.roster.remove(&1);
        app.install_snapshot(snapshot.clone());
        let roster = PageRead::Detail(DetailRead::Roster(1));
        let other_roster = PageRead::Detail(DetailRead::Roster(2));
        app.page_errors.insert(roster, "Timed out".into());
        app.page_errors.insert(other_roster, "Timed out".into());

        let _ = app.update(Message::Navigate(Screen::Course(1)));
        assert!(app.page_errors.contains_key(&roster));
        let _ = app.update(Message::SetTab(Tab::People));
        assert_eq!(app.next_page_read(&snapshot), Some(roster));
        assert!(app.page_errors.contains_key(&other_roster));
        let _ = app.update(Message::DetailLoaded(DetailRead::Roster(1), Err("Timed out again".into())));
        assert_eq!(app.next_page_read(&snapshot), None);
    }

    #[test]
    fn stream_checks_permissions_automatically_and_keeps_an_unconfirmed_draft() {
        let (mut app, snapshot) = fixture();
        app.install_snapshot(snapshot.clone());
        let _ = app.update(Message::Navigate(Screen::Course(1)));
        assert_eq!(app.next_page_read(&snapshot), Some(PageRead::StreamAccess(1)));
        let generation = app.stream_access_generation;
        let _ = app.update(Message::StreamAccessLoaded(generation, 1, Ok((9, StreamAccess::ReadOnly))));
        let _ = app.update(Message::OpenStreamComposer(1));
        assert!(app.open_stream_composer.is_none());
        app.stream_drafts.insert(1, StreamDraft {
            body: text_editor::Content::with_text("Unconfirmed draft"),
            links: vec!["https://example.org/lesson".into()],
            ..Default::default()
        });
        app.sending_stream_post = Some(1);
        let _ = app.update(Message::StreamPostSent(1, Err("Unconfirmed".into())));
        assert_eq!(app.stream_drafts[&1].body.text().trim(), "Unconfirmed draft");
        assert_eq!(app.stream_drafts[&1].links, ["https://example.org/lesson"]);
        assert!(!app.stream_access.contains_key(&1));
        assert!(!valid_stream_draft(" \n "));
    }

    #[test]
    fn pending_local_file_cannot_be_silently_left_out_of_a_post() {
        let (mut app, snapshot) = fixture();
        app.install_snapshot(snapshot);
        app.stream_drafts.insert(1, StreamDraft {
            body: text_editor::Content::with_text("Draft with file"),
            ..Default::default()
        });
        app.choosing_stream_file = Some(1);
        let path = PathBuf::from("Reading notes.pdf");
        let _ = app.update(Message::StreamFileChosen(1, Ok(Some(path.clone()))));
        assert!(!app.stream_drafts[&1].can_send("Draft with file"));
        app.uploading_stream_file = Some(1);
        let _ = app.update(Message::StreamFileUploaded(1, Err("Drive did not confirm the upload.".into())));
        assert_eq!(app.stream_drafts[&1].selected_file.as_ref(), Some(&path));
        assert_eq!(app.stream_drafts[&1].body.text().trim(), "Draft with file");
        assert!(!app.stream_drafts[&1].can_send("Draft with file"));
    }

    #[test]
    fn stream_formatting_survives_insertions_and_handles_non_ascii_selection() {
        let mut draft = StreamDraft { body: text_editor::Content::with_text("a📝b"), ..Default::default() };
        draft.body.move_to(text_editor::Cursor {
            position: text_editor::Position { line: 0, column: 5 },
            selection: Some(text_editor::Position { line: 0, column: 1 }),
        });
        draft.format(StreamFormat::Underline);
        assert_eq!(draft.description_styles().iter().filter(|style| style.underline).map(|style| style.start).collect::<Vec<_>>(), [1]);
        draft.edit(text_editor::Action::Move(text_editor::Motion::DocumentStart));
        draft.edit(text_editor::Action::Move(text_editor::Motion::DocumentStart));
        assert!(draft.body.cursor().selection.is_none());
        assert_eq!(draft.body.cursor().position.column, 0);
        draft.edit(text_editor::Action::Edit(text_editor::Edit::Insert('x')));
        assert_eq!(draft.body.text(), "xa📝b");
        assert_eq!(draft.description_styles().iter().filter(|style| style.underline).map(|style| style.start).collect::<Vec<_>>(), [2]);
    }

    #[test]
    fn stream_bullets_follow_paragraph_edits_and_enter_on_empty_bullet_ends_the_list() {
        let mut draft = StreamDraft { body: text_editor::Content::with_text("Read 📝"), ..Default::default() };
        draft.edit(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
        draft.format(StreamFormat::BulletList);
        assert_eq!(draft.bullets, BTreeSet::from([0]));
        draft.edit(text_editor::Action::Edit(text_editor::Edit::Enter));
        assert_eq!(draft.body.text(), "Read 📝\n");
        assert_eq!(draft.bullets, BTreeSet::from([0, "Read 📝\n".len()]));
        draft.edit(text_editor::Action::Edit(text_editor::Edit::Enter));
        assert_eq!(draft.body.text(), "Read 📝\n");
        assert_eq!(draft.bullets, BTreeSet::from([0]));
    }

    #[test]
    fn drive_folder_navigation_uses_only_returned_folders_and_resets_search() {
        let mut picker = DrivePicker { query: "old search".into(), ..Default::default() };
        assert!(picker.browse(DriveNavigation::Section(DriveSection::MyDrive)));
        assert!(matches!(picker.location(), DriveLocation::Folder(id) if id == "root"));
        assert!(picker.query.is_empty());
        picker.accept_page(DriveFilePage {
            folders: vec![DriveFolder { id: "folder_fixture".into(), name: "Notes".into() }],
            files: vec![], next_page: Some("page_cursor".into()), incomplete: false,
        }, false);
        assert!(!picker.browse(DriveNavigation::OpenFolder("unknown".into())));
        assert!(picker.browse(DriveNavigation::OpenFolder("folder_fixture".into())));
        assert!(matches!(picker.location(), DriveLocation::Folder(id) if id == "folder_fixture"));
        assert!(picker.browse(DriveNavigation::Back));
        assert!(matches!(picker.location(), DriveLocation::Folder(id) if id == "root"));
        assert!(picker.browse(DriveNavigation::Section(DriveSection::Shared)));
        assert!(matches!(picker.location(), DriveLocation::Shared));
    }

    #[test]
    fn stream_delete_only_removes_the_confirmed_announcement() {
        let (mut app, mut snapshot) = fixture();
        let items = snapshot.coursework.get_mut(&1).unwrap();
        let announcement = items.iter_mut().find(|item| item.kind == CourseworkKind::Announcement).unwrap();
        announcement.id = "91".into();
        announcement.author_id = Some(9);
        app.stream_viewer_id = Some(10);
        assert!(!app.owns_announcement(announcement));
        app.stream_viewer_id = Some(9);
        assert!(app.owns_announcement(announcement));
        let before = items.len();
        app.install_snapshot(snapshot);
        app.deleting_stream_post = Some((1, 91));
        let _ = app.update(Message::StreamPostDeleted(1, 92, Ok(())));
        let _ = app.update(Message::StreamPostDeleted(1, 91, Err("Unconfirmed".into())));
        assert_eq!(app.snapshot.as_ref().unwrap().coursework_for(1).unwrap().len(), before);
        app.deleting_stream_post = Some((1, 91));
        let _ = app.update(Message::StreamPostDeleted(1, 91, Ok(())));
        let items = app.snapshot.as_ref().unwrap().coursework_for(1).unwrap();
        assert_eq!(items.len(), before - 1);
        assert!(items.iter().all(|item| item.id != "91"));
    }

    #[test]
    fn deleting_a_private_comment_preserves_the_class_thread_with_the_same_id() {
        let (mut app, mut snapshot) = fixture();
        let fixture = Comment {
            id: "19".into(), author_id: Some(12), author: "Student".into(),
            body: "Sample comment".into(), created_label: None,
        };
        let item = &mut snapshot.coursework.get_mut(&1).unwrap()[0];
        item.id = "42".into();
        item.comments = Some(vec![fixture.clone()]);
        app.install_snapshot(snapshot);
        app.stream_viewer_id = Some(11);
        assert!(!app.owns_comment(&fixture));
        app.stream_viewer_id = Some(12);
        assert!(app.owns_comment(&fixture));
        app.private_feedback.insert((1, 42), vec![fixture]);
        let target = CommentTarget { course: 1, item: 42, id: 19, audience: CommentAudience::Private };
        app.deleting_comment = Some(target);
        let _ = app.update(Message::CommentDeleted(target, Ok(())));
        assert!(app.private_feedback[&(1, 42)].is_empty());
        assert_eq!(app.snapshot.as_ref().unwrap().coursework_for(1).unwrap()[0].comments.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn dragging_a_class_uses_its_direction_and_skips_hidden_targets() {
        let (mut app, mut snapshot) = fixture();
        snapshot.courses.iter_mut().find(|course| course.id == 2).unwrap().hidden = Some(true);
        app.install_snapshot(snapshot);
        assert_eq!(app.drop_placement(1, 3), Some(CoursePlacement::After(3)));
        assert_eq!(app.drop_placement(3, 1), Some(CoursePlacement::Before(1)));
        assert_eq!(app.drop_placement(1, 2), None);
        assert_eq!(app.drop_placement(1, 1), None);
        assert!(app.course_move_options(1).iter().all(|option| option.placement != CoursePlacement::After(2)));
    }

    #[test]
    fn settings_use_confirmed_values_and_ignore_stale_notification_reads() {
        let (mut app, snapshot) = fixture();
        app.install_snapshot(snapshot);
        let _ = app.update(Message::Navigate(Screen::Settings));
        let generation = app.notifications_generation;
        let _ = app.update(Message::ClassNotificationsLoaded(generation, Ok(BTreeMap::from([(1, false)]))));
        let _ = app.update(Message::SetClassNotifications(1, false));
        assert!(app.saving_class_notifications.is_none());
        assert_eq!(app.class_notifications.as_ref().unwrap().get(&1), Some(&false));

        app.saving_class_notifications = Some(1);
        let _ = app.update(Message::ClassNotificationsSaved(1, Ok(true)));
        assert_eq!(app.class_notifications.as_ref().unwrap().get(&1), Some(&true));
        let _ = app.update(Message::Navigate(Screen::Home));
        let _ = app.update(Message::Navigate(Screen::Settings));
        let _ = app.update(Message::ClassNotificationsLoaded(generation, Ok(BTreeMap::from([(1, false)]))));
        assert!(app.class_notifications.is_none());
    }

    #[test]
    fn assignment_editor_preserves_existing_fields_and_can_clear_them_independently() {
        let (_, snapshot) = fixture();
        let mut item = snapshot.coursework_for(1).unwrap().iter()
            .find(|item| item.kind == CourseworkKind::Assignment).unwrap().clone();
        let date = NaiveDate::from_ymd_opt(2027, 1, 26).unwrap();
        item.due_at_millis = Some(date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis());
        item.due_has_time = Some(false);
        item.maximum_points = Some(42.5);
        let draft = desktop_auth::TeacherDraft { item, publication: desktop_auth::DraftPublication::Draft };
        let mut editor = AssignmentEditor::new(1, Some(draft));
        assert_eq!(editor.settings.due_date, "2027-01-26");
        assert_eq!(editor.settings.due_time, "");
        assert_eq!(editor.settings.points, "42.5");
        editor.title.push_str(" revised");
        let unchanged = editor.field_changes().unwrap();
        assert!(unchanged.due.is_none() && unchanged.points.is_none());

        editor.settings.due_date.clear();
        let cleared_date = editor.field_changes().unwrap();
        assert_eq!(cleared_date.due, Some(None));
        assert!(cleared_date.points.is_none());
        editor.settings.points = "0".into();
        assert_eq!(editor.field_changes().unwrap().points, Some(None));
        editor.settings.due_time = "16:00".into();
        assert!(editor.field_changes().is_err());
    }

    #[test]
    fn new_assignment_prefills_default_points_and_only_sends_explicit_changes() {
        let mut editor = AssignmentEditor::new(1, None);
        assert_eq!(editor.settings.points, "100");
        assert!(editor.field_changes().unwrap().points.is_none());
        editor.settings.points = "25".into();
        assert_eq!(editor.field_changes().unwrap().points, Some(Some(25.0)));
        editor.settings.points = "-1".into();
        assert!(editor.field_changes().is_err());
    }

    #[test]
    fn archive_and_restore_update_overviews_after_a_matching_confirmation() {
        let (mut app, snapshot) = fixture();
        let count = snapshot.courses.len();
        app.install_snapshot(snapshot);
        app.todo_class = Some(1);
        app.calendar_class = Some(1);
        app.changing_class_archive = Some((1, true));
        let _ = app.update(Message::ClassArchiveChanged(1, false, Ok(CourseState::Active)));
        assert_eq!(app.changing_class_archive, Some((1, true)));
        let _ = app.update(Message::ClassArchiveChanged(1, true, Ok(CourseState::Archived)));
        let data = app.snapshot.as_ref().unwrap();
        assert_eq!(data.courses.len(), count);
        assert_eq!(data.courses.iter().find(|course| course.id == 1).unwrap().state, CourseState::Archived);
        assert!(data.upcoming_work(&Local::now(), Some(1), 5).is_empty());
        assert!(!app.assignment_courses().contains(&1));
        assert!(app.active_course_options().iter().all(|option| option.id != Some(1)));
        assert_eq!((app.todo_class, app.calendar_class), (None, None));
        app.changing_class_archive = Some((1, false));
        let _ = app.update(Message::ClassArchiveChanged(1, false, Ok(CourseState::Active)));
        assert!(app.assignment_courses().contains(&1));
        assert!(app.active_course_options().iter().any(|option| option.id == Some(1)));
    }

    #[test]
    fn hidden_class_keeps_enrollment_and_leaves_overviews_until_unhidden() {
        let (mut app, snapshot) = fixture();
        let count = snapshot.courses.len();
        app.install_snapshot(snapshot);
        app.calendar_class = Some(1);
        app.todo_class = Some(1);
        app.changing_course_visibility = Some((1, true));
        let _ = app.update(Message::CourseVisibilityChanged(2, true, Ok(())));
        let _ = app.update(Message::CourseVisibilityChanged(1, true, Ok(())));
        let data = app.snapshot.as_ref().unwrap();
        let course = data.courses.iter().find(|course| course.id == 1).unwrap();
        assert!(hidden_course(course));
        assert_eq!(course.state, CourseState::Active);
        assert_eq!(data.courses.len(), count);
        assert!(data.upcoming_work(&Local::now(), Some(1), 5).is_empty());
        assert!(!app.assignment_courses().contains(&1));
        assert!(app.active_course_options().iter().all(|option| option.id != Some(1)));
        assert_eq!((app.calendar_class, app.todo_class), (None, None));

        app.changing_course_visibility = Some((1, false));
        let _ = app.update(Message::CourseVisibilityChanged(1, false, Ok(())));
        assert!(app.assignment_courses().contains(&1));
        assert!(app.active_course_options().iter().any(|option| option.id == Some(1)));
    }

    #[test]
    fn opening_an_unloaded_item_fetches_classwork_then_comments() {
        let (mut app, mut snapshot) = fixture();
        let mut items = snapshot.coursework.remove(&1).unwrap();
        items[0].id = "42".into();
        items[0].comments = None;
        app.install_snapshot(snapshot.clone());

        let _ = app.update(Message::OpenItem(1, "42".into()));
        assert_eq!(app.next_page_read(&snapshot), Some(PageRead::Coursework(1)));
        let _ = app.update(Message::CourseworkLoaded(1, Ok(CourseContents { items, topics: None })));
        assert_eq!(app.next_page_read(app.snapshot.as_ref().unwrap()), Some(PageRead::StreamAccess(1)));
        app.loading_stream_access.insert(1);
        assert_eq!(app.next_page_read(app.snapshot.as_ref().unwrap()),
            Some(PageRead::Detail(DetailRead::Comments(1, 42))));
    }

    #[test]
    fn classroom_link_opens_the_item_and_back_returns_to_its_class() {
        let (mut app, mut snapshot) = fixture();
        app.verify_live_course_count = false;
        snapshot.coursework.get_mut(&1).unwrap()[0].id = "42".into();
        app.install_snapshot(snapshot);
        app.screen = Screen::Course(2);
        app.classroom_link_input = Some("https://classroom.google.com/c/MQ/a/NDI/details".into());
        let _ = app.update(Message::OpenClassroomLink);
        assert_eq!(app.screen, Screen::Item(1));
        assert_eq!(app.item_id.as_deref(), Some("42"));
        let _ = app.update(Message::CloseItem);
        assert_eq!((app.screen, app.tab), (Screen::Course(1), Tab::Classwork));
        app.classroom_link_input = Some("https://classroom.google.com/c/999999999".into());
        let _ = app.update(Message::OpenClassroomLink);
        assert_eq!(app.screen, Screen::Course(1));
        assert!(app.error.is_some());
    }

    #[test]
    fn topic_link_opens_classwork_and_keeps_selection_when_items_load() {
        let (mut app, mut snapshot) = fixture();
        app.verify_live_course_count = false;
        let mut items = snapshot.coursework.remove(&1).unwrap();
        items.iter_mut().find(|item| item.kind == CourseworkKind::Assignment).unwrap()
            .topic_id = Some("456".into());
        app.install_snapshot(snapshot);
        app.screen = Screen::Course(2);
        app.classroom_link_input = Some("https://classroom.google.com/u/0/w/MQ/tc/NDU2".into());
        let _ = app.update(Message::OpenClassroomLink);
        assert_eq!((app.screen, app.tab), (Screen::Course(1), Tab::Classwork));
        assert_eq!(app.selected_topic.as_deref(), Some("456"));
        let _ = app.update(Message::CourseworkLoaded(1, Ok(CourseContents { items, topics: None })));
        assert_eq!(app.selected_topic.as_deref(), Some("456"));
        assert_eq!(app.snapshot.as_ref().unwrap().classwork_sections(1, Some("456")).len(), 1);
    }

    #[test]
    fn material_and_announcement_links_open_matching_items() {
        let (mut app, mut snapshot) = fixture();
        app.verify_live_course_count = false;
        let mut item = snapshot.coursework_for(1).unwrap()[0].clone();
        item.id = "42".into();
        item.kind = CourseworkKind::Material;
        snapshot.coursework.insert(1, vec![item.clone()]);
        app.install_snapshot(snapshot);
        app.classroom_link_input = Some("https://classroom.google.com/c/MQ/m/NDI/details".into());
        let _ = app.update(Message::OpenClassroomLink);
        assert_eq!((app.screen, app.tab), (Screen::Item(1), Tab::Classwork));
        assert_eq!(item_link_kind(&item), Some(ItemLinkKind::Material));
        let _ = app.update(Message::CloseItem);

        item.kind = CourseworkKind::Announcement;
        app.snapshot.as_mut().unwrap().coursework.insert(1, vec![item.clone()]);
        app.classroom_link_input = Some("https://classroom.google.com/c/MQ/p/NDI/details".into());
        let _ = app.update(Message::OpenClassroomLink);
        assert_eq!((app.screen, app.tab), (Screen::Item(1), Tab::Stream));
        assert_eq!(item_link_kind(&item), Some(ItemLinkKind::Announcement));
    }

    #[test]
    fn class_calendar_shortcuts_require_a_live_course_and_never_survive_an_offline_copy() {
        let (mut app, mut snapshot) = fixture();
        app.verify_live_course_count = false;
        snapshot.courses[0].calendar_url = Some("https://calendar.google.com/calendar/u/0?cid=fixture".into());
        app.install_snapshot(snapshot.clone());
        assert!(app.class_calendar_button(1).is_some());
        assert!(app.class_calendar_button(2).is_none());
        snapshot.source = SnapshotSource::Cache;
        app.install_snapshot(snapshot);
        assert!(app.class_calendar_button(1).is_none());
    }

    #[test]
    fn hiding_classwork_on_stream_does_not_remove_it_from_classwork() {
        let (_, mut snapshot) = fixture();
        snapshot.courses[0].stream_classwork_display = Some(StreamClassworkDisplay::Hidden);
        let display = snapshot.courses[0].stream_classwork_display.unwrap();
        let kinds: Vec<_> = snapshot.coursework_for(1).unwrap().iter().map(|item| item.kind).collect();
        assert_eq!(kinds.iter().copied().filter(|&kind| display.shows(kind)).collect::<Vec<_>>(),
            [CourseworkKind::Announcement]);
        assert_eq!(snapshot.classwork_sections(1, None).iter().map(|section| section.items.len()).sum::<usize>(), 2);
    }

    #[test]
    fn mark_done_label_applies_only_to_an_editable_submission_without_attachments() {
        use desktop_auth::SubmissionAction::{TurnIn, Unsubmit};
        let mut submission = Submission::default();
        assert!(assignment_is_mark_done(TurnIn, Some(&submission)));
        assert!(!assignment_is_mark_done(Unsubmit, Some(&submission)));
        assert!(!assignment_is_mark_done(TurnIn, None));
        submission.attachments.push(desktop_data::Attachment {
            title: "Sample".into(), kind: desktop_data::AttachmentKind::Link,
            url: Some("https://example.org".into()), remote_id: Some(1),
            drive_sharing: None, drive_reference: None, drive_parent_reference: None,
        });
        assert!(!assignment_is_mark_done(TurnIn, Some(&submission)));
    }

    #[test]
    fn failed_submission_update_is_shown_as_an_error() {
        let (mut app, _) = fixture();
        app.sending_submission = true;
        let _ = app.update(Message::SubmissionSent(1, Err("Classroom did not update the submission.".into())));
        assert!(!app.sending_submission);
        assert_eq!(app.error.as_deref(), Some("Classroom did not update the submission."));
        assert!(app.notice.is_none());
    }

    #[test]
    fn uncertain_comment_result_triggers_one_automatic_read_with_cached_comments() {
        let (mut app, mut snapshot) = fixture();
        let item = &mut snapshot.coursework.get_mut(&1).unwrap()[0];
        item.id = "42".into();
        item.comments = Some(vec![]);
        app.install_snapshot(snapshot.clone());
        let _ = app.update(Message::OpenItem(1, "42".into()));
        let _ = app.update(Message::CommentPosted(1, 42, Err("Connection ended before result".into())));
        assert_eq!(app.next_page_read(&snapshot), Some(PageRead::StreamAccess(1)));
        app.loading_stream_access.insert(1);
        assert_eq!(app.next_page_read(&snapshot), Some(PageRead::Detail(DetailRead::Comments(1, 42))));
        let _ = app.update(Message::DetailLoaded(DetailRead::Comments(1, 42), Err("Read timed out".into())));
        assert_ne!(app.next_page_read(&snapshot), Some(PageRead::Detail(DetailRead::Comments(1, 42))));
        assert!(app.comment_unconfirmed.contains(&(1, 42)));
    }

    #[test]
    fn item_keeps_loading_after_a_section_fails_without_retrying_in_a_loop() {
        let (mut app, mut snapshot) = fixture();
        let assignment = snapshot.coursework.get_mut(&1).unwrap()
            .iter_mut().find(|item| item.kind == CourseworkKind::Assignment).unwrap();
        assignment.id = "42".into();
        assignment.submission.as_mut().unwrap().remote_id = Some(43);

        app.screen = Screen::Item(1);
        app.item_id = Some("42".into());
        app.page_errors.insert(PageRead::StreamAccess(1), "Permission request failed.".into());
        let comments = PageRead::Detail(DetailRead::Comments(1, 42));
        let submissions = PageRead::Detail(DetailRead::Submissions(1));
        let feedback = PageRead::Detail(DetailRead::PrivateFeedback(1, 42));
        assert_eq!(app.next_page_read(&snapshot), Some(comments));
        app.page_errors.insert(comments, "Comments request failed.".into());
        assert_eq!(app.next_page_read(&snapshot), Some(submissions));
        app.submissions_checked.insert(1);
        assert_eq!(app.next_page_read(&snapshot), Some(PageRead::Detail(DetailRead::Rubric(1, 42))));
        app.rubrics.insert((1, 42), None);
        assert_eq!(app.next_page_read(&snapshot), Some(feedback));
        app.private_feedback.insert((1, 42), vec![]);
        assert_eq!(app.next_page_read(&snapshot), None);
    }

    #[test]
    fn assignment_batch_keeps_completed_classes_when_navigation_cancels_pending_reads() {
        let (mut app, mut snapshot) = fixture();
        snapshot.coursework.remove(&1);
        snapshot.coursework.remove(&2);
        app.install_snapshot(snapshot);
        app.screen = Screen::Home;
        let (_task, handle) = Task::<Message>::none().abortable();
        app.assignment_generation = 1;
        app.assignment_batch = Some(AssignmentBatch {
            handle: handle.clone(),
            remaining: BTreeSet::from([1, 2]),
            failed_classwork: 0,
            failed_status: 0,
        });
        let (_, data) = fixture();
        let ready = desktop_auth::AssignmentRefresh {
            course_id: 1,
            contents: Some(CourseContents { items: data.coursework_for(1).unwrap().to_vec(), topics: None }),
            submissions: Some(vec![]),
        };
        let _ = app.update(Message::AssignmentLoaded(1, ready));
        assert!(app.snapshot.as_ref().unwrap().coursework_for(1).is_some());
        assert!(app.snapshot.as_ref().unwrap().coursework_for(2).is_none());
        assert!(!app.assignments_loaded);
        assert!(app.assignment_batch.is_some());

        let _ = app.update(Message::Navigate(Screen::Calendar));
        let _ = app.update(Message::Navigate(Screen::ToDo));
        assert!(!handle.is_aborted());
        let _ = app.update(Message::Navigate(Screen::Course(2)));
        assert!(handle.is_aborted());
        assert!(app.assignment_batch.is_none());
        assert!(!app.queued_page_load);

        let (_task, handle) = Task::<Message>::none().abortable();
        app.screen = Screen::Calendar;
        app.assignment_batch = Some(AssignmentBatch {
            handle,
            remaining: BTreeSet::from([2]),
            failed_classwork: 0,
            failed_status: 0,
        });
        let late = desktop_auth::AssignmentRefresh {
            course_id: 2,
            contents: Some(CourseContents { items: vec![], topics: None }),
            submissions: None,
        };
        let _ = app.update(Message::AssignmentLoaded(1, late));
        assert!(app.snapshot.as_ref().unwrap().coursework_for(2).is_none());
        assert!(!app.assignment_courses().contains(&1));
        assert!(app.assignment_courses().contains(&2));

        let _ = app.update(Message::AssignmentLoaded(app.assignment_generation, desktop_auth::AssignmentRefresh {
            course_id: 2, contents: None, submissions: None,
        }));
        assert!(app.assignments_loaded);
        assert!(app.assignment_batch.is_none());
        assert!(app.page_errors.contains_key(&PageRead::Assignments));
        assert!(app.assignment_courses().contains(&2));
    }

    #[test]
    fn page_navigation_remains_queued_during_an_existing_request() {
        let (mut app, _) = fixture();
        app.loading_coursework = Some(2);
        let _ = app.update(Message::Navigate(Screen::Course(1)));
        let _ = app.update(Message::SetTab(Tab::People));
        assert!(app.queued_page_load);
        assert_eq!((app.screen, app.tab), (Screen::Course(1), Tab::People));

        let _ = app.update(Message::CourseworkLoaded(2, Ok(CourseContents { items: vec![], topics: None })));
        assert!(!app.queued_page_load);
    }
}
