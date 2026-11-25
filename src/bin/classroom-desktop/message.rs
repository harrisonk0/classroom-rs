use super::*;

#[derive(Clone)]
pub(super) enum Message {
    Offline(OfflineMessage),
    Navigation(NavigationMessage),
    Teaching(TeachingMessage),
    Courses(CoursesMessage),
    Session(SessionMessage),
    Reads(ReadsMessage),
    Stream(StreamMessage),
    Attachments(AttachmentsMessage),
    Comments(CommentsMessage),
    Submissions(SubmissionsMessage),
    Notifications(NotificationsMessage),
    Deadlines(DeadlinesMessage),
    Screenshots(ScreenshotsMessage),
}

impl fmt::Debug for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Offline(_) => "Offline",
            Self::Navigation(_) => "Navigation",
            Self::Teaching(_) => "Teaching",
            Self::Courses(_) => "Courses",
            Self::Session(_) => "Session",
            Self::Reads(_) => "Reads",
            Self::Stream(_) => "Stream",
            Self::Attachments(_) => "Attachments",
            Self::Comments(_) => "Comments",
            Self::Submissions(_) => "Submissions",
            Self::Notifications(_) => "Notifications",
            Self::Deadlines(_) => "Deadlines",
            Self::Screenshots(_) => "Screenshots",
        })
    }
}
