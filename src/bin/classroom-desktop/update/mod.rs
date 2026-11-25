mod navigation;
mod teaching;
mod courses;
mod session;
mod reads;
mod stream;
mod attachments;
mod comments;
mod notifications;
mod submissions;
mod deadlines;
mod screenshots;

use super::*;

impl App {
    pub(super) fn update(&mut self, message: Message) -> Task<Message> {
        if self.screenshot_directory.is_some()
            && !matches!(message, Message::ScreenshotReady(_, _))
        {
            return Task::none();
        }
        match message {
            Message::Offline(message) => self.update_offline(message),
            #[cfg(target_os = "macos")]
            Message::Capture(_) => self.update_navigation(message),
            #[cfg(target_os = "macos")]
            Message::ExportGrades(..) | Message::GradesExported(..) => self.update_teaching(message),
            Message::Navigate(..)
            | Message::ShowClassroomLink
            | Message::EditClassroomLink(..)
            | Message::OpenClassroomLink
            | Message::ClassroomWorkLinkChecked(..)
            | Message::CloseClassroomLink
            | Message::CopyClassroomLink(..)
            | Message::OpenYourWork(..)
            | Message::OpenGradebook(..)
            | Message::OpenGradebookCell(..)
            | Message::GradebookColumnPage(..)
            | Message::SetTab(..)
            | Message::ToggleNavigation
            | Message::Search(..)
            | Message::SortCourses
            | Message::ToggleRubric(..)
            | Message::ToggleRubricCriterion(..)
            | Message::SetRubricCriteria(..)
            | Message::OpenItem(..)
            | Message::CloseItem
            | Message::SelectTopic(..)
            | Message::ToggleClassworkItem(..)
            | Message::ToggleClassworkGroup(..)
            | Message::SetClassworkGroups(..) => self.update_navigation(message),
            Message::SetCoursePostingPolicy(..)
            | Message::SetCourseStreamDisplay(..)
            | Message::EditCourseTitle(..)
            | Message::EditCourseSection(..)
            | Message::EditCourseDescription(..)
            | Message::EditCourseRoom(..)
            | Message::SaveCourseStreamSettings
            | Message::CourseStreamSettingsSaved(..)
            | Message::ConfirmStudentMute(..)
            | Message::CancelStudentMute
            | Message::ChangeStudentMute(..)
            | Message::StudentMuteChanged(..)
            | Message::ToggleTopicManager
            | Message::OpenTopicEditor(..)
            | Message::EditTopicName(..)
            | Message::CloseTopicEditor
            | Message::SaveTopicEditor
            | Message::TeacherTopicChanged(..)
            | Message::MoveTeacherTopic(..)
            | Message::OpenItemTopic(..)
            | Message::SelectItemTopic(..)
            | Message::CloseItemTopic
            | Message::SaveItemTopic
            | Message::ItemTopicSaved(..)
            | Message::ReorderClasswork(..)
            | Message::ClassworkReordered(..)
            | Message::GradebookEditMode(..)
            | Message::EditGradebookCell(..)
            | Message::GradebookGradeInput(..)
            | Message::CancelGradebookEdit
            | Message::SaveGradebookEdit(..)
            | Message::GradebookEditSaved(..)
            | Message::ToggleTeacherDraft(..)
            | Message::OpenAssignmentEditor(..)
            | Message::EditAssignmentTitle(..)
            | Message::EditAssignmentInstructions(..)
            | Message::EditAssignmentDueDate(..)
            | Message::EditAssignmentDueTime(..)
            | Message::EditAssignmentPoints(..)
            | Message::CloseAssignmentEditor
            | Message::SaveAssignmentDraft
            | Message::AssignmentDraftSaved(..)
            | Message::OpenAssignmentPublication(..)
            | Message::EditAssignmentSchedule(..)
            | Message::CloseAssignmentPublication
            | Message::ConfirmAssignmentPublication
            | Message::AssignmentPublicationChanged(..)
            | Message::FilterStudentWork(..)
            | Message::SelectTeacherStudent(..)
            | Message::EditTeacherGrade(..)
            | Message::SaveTeacherGrade
            | Message::ConfirmReturnTeacherWork(..)
            | Message::ReturnTeacherWork
            | Message::TeacherGradeWritten(..)
            | Message::SelectBulkReturn(..)
            | Message::SelectVisibleReturns(..)
            | Message::ClearBulkReturns
            | Message::ConfirmBulkReturn(..)
            | Message::ReturnSelectedTeacherWork
            | Message::BulkTeacherWorkReturned(..)
            | Message::EditTeacherFeedback(..)
            | Message::SendTeacherFeedback(..)
            | Message::TeacherFeedbackSent(..) => self.update_teaching(message),
            Message::ToggleHiddenClasses
            | Message::ToggleCourseMenu(..)
            | Message::ChooseCourseMove(..)
            | Message::MoveCourse(..)
            | Message::CourseMoved(..)
            | Message::BeginCourseDrag(..)
            | Message::EnterCourseDropTarget(..)
            | Message::ExitCourseDropTarget(..)
            | Message::DropCourse
            | Message::CancelCourseDrag
            | Message::ConfirmHideCourse(..)
            | Message::ChangeCourseVisibility(..)
            | Message::CourseVisibilityChanged(..)
            | Message::ConfirmClassArchive(..)
            | Message::ChangeClassArchive(..)
            | Message::ClassArchiveChanged(..)
            | Message::EditClassCode(..)
            | Message::JoinClass
            | Message::LoadInvitations
            | Message::InvitationsLoaded(..)
            | Message::AcceptInvitation(..)
            | Message::ConfirmDeclineInvitation(..)
            | Message::CancelDeclineInvitation
            | Message::DeclineInvitation(..)
            | Message::InvitationDeclined(..)
            | Message::ConfirmUnenroll(..)
            | Message::CancelUnenroll
            | Message::Unenroll(..)
            | Message::MembershipFinished(..)
            | Message::ClassCreationPermission(..)
            | Message::EditNewClass(..)
            | Message::CreateClass => self.update_courses(message),
            Message::Email(..)
            | Message::SignIn
            | Message::SignedIn(..)
            | Message::Reconnect
            | Message::Reconnected(..)
            | Message::Refresh
            | Message::Refreshed(..)
            | Message::Demo
            | Message::Disconnect
            | Message::SessionForgotten(..) => self.update_session(message),
            Message::LoadCoursework(..)
            | Message::LoadStreamAccess(..)
            | Message::StreamAccessLoaded(..)
            | Message::LoadAssignments
            | Message::AssignmentLoaded(..)
            | Message::CourseworkLoaded(..)
            | Message::LoadDetail(..)
            | Message::DetailLoaded(..)
            | Message::ClassFoldersLoaded(..)
            | Message::ClassTeachersLoaded(..)
            | Message::RetryClassFolders
            | Message::AttachmentOpened
            | Message::AttachmentFailed
            | Message::ArtworkLoaded(..)
            | Message::StreamProfilesLoaded(..)
            | Message::StreamProfilePictureLoaded(..) => self.update_reads(message),
            Message::OpenStreamComposer(..)
            | Message::EditStreamDraft(..)
            | Message::FormatStreamDraft(..)
            | Message::OpenStreamLink(..)
            | Message::EditStreamLink(..)
            | Message::AddStreamLink(..)
            | Message::CancelStreamLink(..)
            | Message::RemoveStreamLink(..)
            | Message::ChooseStreamFile(..)
            | Message::StreamFileChosen(..)
            | Message::ClearStreamFile(..)
            | Message::UploadStreamFile(..)
            | Message::StreamFileUploaded(..)
            | Message::RemoveStreamFile(..)
            | Message::CloseStreamComposer
            | Message::SendStreamPost(..)
            | Message::StreamPostSent(..)
            | Message::ConfirmDeleteStreamPost(..)
            | Message::CancelDeleteStreamPost
            | Message::DeleteStreamPost(..)
            | Message::StreamPostDeleted(..) => self.update_stream(message),
            Message::OpenDrivePicker(..)
            | Message::OpenSubmissionDrivePicker(..)
            | Message::ToggleNewSubmissionDocument(..)
            | Message::CreateSubmissionDocument(..)
            | Message::SubmissionDocumentCreated(..)
            | Message::SubmissionDriveFileAttached(..)
            | Message::CloseDrivePicker
            | Message::EditDriveSearch(..)
            | Message::BrowseDrive(..)
            | Message::DriveScrolled(..)
            | Message::SearchDriveFiles(..)
            | Message::DriveFilesLoaded(..)
            | Message::AttachExistingDriveFile(..)
            | Message::OpenAttachment(..)
            | Message::OpenDriveFile(..)
            | Message::PreviewDriveFile(..)
            | Message::DrivePreviewReady(..)
            | Message::DrivePreviewClosed(..)
            | Message::SaveDriveFile(..)
            | Message::DriveFileSaved(..)
            | Message::DriveFileLinkLoaded(..) => self.update_attachments(message),
            Message::EditComment(..)
            | Message::OpenCommentMentions(..)
            | Message::EmailClassMember(..)
            | Message::ClassMemberEmailLoaded(..)
            | Message::ClassMemberEmailOpened(..)
            | Message::CommentMentionsLoaded(..)
            | Message::InsertCommentMention(..)
            | Message::CloseCommentMentions
            | Message::PostComment(..)
            | Message::EditPrivateComment(..)
            | Message::PostPrivateComment(..)
            | Message::ConfirmDeleteComment(..)
            | Message::CancelDeleteComment
            | Message::DeleteComment(..)
            | Message::CommentDeleted(..)
            | Message::PrivateCommentPosted(..)
            | Message::CommentPosted(..) => self.update_comments(message),
            Message::LoadClassNotifications
            | Message::ClassNotificationsLoaded(..)
            | Message::SetClassNotifications(..)
            | Message::ClassNotificationsSaved(..)
            | Message::LoadEmailNotifications
            | Message::EmailNotificationsLoaded(..)
            | Message::SetEmailNotification(..)
            | Message::EmailNotificationSaved(..) => self.update_notifications(message),
            Message::ConfirmSubmission(..)
            | Message::CancelSubmission
            | Message::EditAttachmentLink(..)
            | Message::AddAttachmentLink(..)
            | Message::ChooseSubmissionFile(..)
            | Message::SubmissionFileChosen(..)
            | Message::SubmissionFileDropped(..)
            | Message::SubmissionFileValidated(..)
            | Message::CancelSubmissionFile
            | Message::ConfirmPersonalCopy(..)
            | Message::CancelPersonalCopy
            | Message::PreparePersonalCopy(..)
            | Message::PersonalCopyPrepared(..)
            | Message::ConfirmRemoveMaterial(..)
            | Message::CancelRemoveMaterial
            | Message::RemoveMaterial(..)
            | Message::MaterialRemoved(..)
            | Message::UploadSubmissionFile(..)
            | Message::SubmissionFileUploaded(..)
            | Message::AttachmentLinkAdded(..)
            | Message::SendSubmission(..)
            | Message::SubmissionSent(..)
            | Message::BeginShortAnswerEdit(..)
            | Message::DiscardShortAnswerEdit(..)
            | Message::EditShortAnswer(..)
            | Message::SelectQuestionChoice(..)
            | Message::ConfirmQuestionAnswer(..)
            | Message::CancelQuestionAnswer
            | Message::SendQuestionAnswer(..)
            | Message::QuestionAnswerSent(..) => self.update_submissions(message),
            Message::SetTodoFilter(..)
            | Message::SetWorkFilter(..)
            | Message::FilterTodoClass(..)
            | Message::ToggleTodoGroup(..)
            | Message::CalendarShift(..)
            | Message::ClockTick(..)
            | Message::CalendarToday
            | Message::CalendarFilter(..) => self.update_deadlines(message),
            Message::ScreenshotReady(..) => self.update_screenshots(message),
        }
    }

    pub(super) fn finish_update(&mut self) -> Task<Message> {
        if !self.screen.uses_assignment_overview() {
            self.cancel_assignment_batch();
        }
        let profiles = self.fetch_stream_profiles();
        Task::batch([profiles, self.dequeue_page_load().map_or_else(Task::none, |request| self.update(request))])
    }
}
