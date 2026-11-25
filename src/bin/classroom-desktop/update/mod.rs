mod navigation;
pub(super) use navigation::NavigationMessage;
mod teaching;
pub(super) use teaching::TeachingMessage;
mod courses;
pub(super) use courses::CoursesMessage;
mod session;
pub(super) use session::SessionMessage;
mod reads;
pub(super) use reads::ReadsMessage;
mod stream;
pub(super) use stream::StreamMessage;
mod attachments;
pub(super) use attachments::AttachmentsMessage;
mod comments;
pub(super) use comments::CommentsMessage;
mod notifications;
pub(super) use notifications::NotificationsMessage;
mod submissions;
pub(super) use submissions::SubmissionsMessage;
mod deadlines;
pub(super) use deadlines::DeadlinesMessage;
mod screenshots;
pub(super) use screenshots::ScreenshotsMessage;

use super::*;

impl App {
    pub(super) fn update(&mut self, message: Message) -> Task<Message> {
        if self.screenshot_directory.is_some()
            && !matches!(message, Message::Screenshots(ScreenshotsMessage::ScreenshotReady(_, _)))
        {
            return Task::none();
        }
        match message {
            Message::Offline(message) => self.update_offline(message),
            Message::Navigation(message) => self.update_navigation(message),
            Message::Teaching(message) => self.update_teaching(message),
            Message::Courses(message) => self.update_courses(message),
            Message::Session(message) => self.update_session(message),
            Message::Reads(message) => self.update_reads(message),
            Message::Stream(message) => self.update_stream(message),
            Message::Attachments(message) => self.update_attachments(message),
            Message::Comments(message) => self.update_comments(message),
            Message::Submissions(message) => self.update_submissions(message),
            Message::Notifications(message) => self.update_notifications(message),
            Message::Deadlines(message) => self.update_deadlines(message),
            Message::Screenshots(message) => self.update_screenshots(message),
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
