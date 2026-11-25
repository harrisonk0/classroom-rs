use super::*;

pub(super) fn schedule_capture(shot: Shot) -> Task<Message> {
    // The renderer captures the last draw. Keep the delay and capture in one
    // task so an intermediate app message cannot rebuild the editor's layout.
    Task::future(async { tokio::time::sleep(Duration::from_millis(425)).await })
        .then(move |()| window::latest().then(move |id| match id {
            Some(id) => window::screenshot(id)
                .map(move |image| Message::Screenshots(ScreenshotsMessage::ScreenshotReady(shot, Arc::new(image)))),
            None => iced::exit(),
        }))
}

pub(super) fn save_screenshot(
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

#[derive(Debug, Clone, Copy)]
pub(super) enum Shot {
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
