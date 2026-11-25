use super::*;

#[derive(Clone)]
pub(in crate::desktop) enum NotificationsMessage {
    LoadClassNotifications,
    ClassNotificationsLoaded(u64, Result<BTreeMap<i64, bool>, String>),
    SetClassNotifications(i64, bool),
    ClassNotificationsSaved(i64, Result<bool, String>),
    LoadEmailNotifications,
    EmailNotificationsLoaded(u64, Result<EmailNotificationSettings, String>),
    SetEmailNotification(EmailNotificationTarget, bool),
    EmailNotificationSaved(EmailNotificationTarget, Result<EmailNotificationSettings, String>),
}

impl App {
    pub(super) fn update_notifications(&mut self, message: NotificationsMessage) -> Task<Message> {
        match message {
            NotificationsMessage::LoadClassNotifications if self.can_read_pages() && !self.settings.loading_class_notifications
                && !self.has_pending_request() && self.screen == Screen::Settings =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                let courses = self.class_notification_courses().map(|course| course.id).collect();
                self.page_errors.remove(&PageRead::ClassNotifications);
                self.settings.loading_class_notifications = true;
                let generation = self.settings.notifications_generation;
                let classes = Task::perform(async move { desktop_auth::load_class_notifications(&session, courses).await },
                    move |result| Message::Notifications(NotificationsMessage::ClassNotificationsLoaded(generation, result)));
                self.queued_page_load = true;
                return Task::batch([classes, self.dequeue_page_load().map_or_else(Task::none, |request| self.update(request))]);
            }
            NotificationsMessage::ClassNotificationsLoaded(generation, result) if generation == self.settings.notifications_generation => {
                self.settings.loading_class_notifications = false;
                match result {
                    Ok(settings) => {
                        if std::env::var_os("CLASSROOM_READ_ONLY_DIAGNOSTICS").is_some() {
                            println!("Class notification settings ready; count={}", settings.len());
                        }
                        self.settings.class_notifications = Some(settings);
                    }
                    Err(error) => {
                        self.page_errors.insert(PageRead::ClassNotifications, error);
                    }
                }
            }
            NotificationsMessage::SetClassNotifications(course, enabled) if self.can_set_class_notifications(course, enabled) => {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.settings.saving_class_notifications = Some(course);
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::set_class_notifications(&session, course, !enabled).await },
                    move |result| Message::Notifications(NotificationsMessage::ClassNotificationsSaved(course, result)));
            }
            NotificationsMessage::ClassNotificationsSaved(course, result) if self.settings.saving_class_notifications == Some(course) => {
                self.settings.saving_class_notifications = None;
                match result {
                    Ok(muted) => {
                        if let Some(settings) = &mut self.settings.class_notifications {
                            settings.insert(course, muted);
                        }
                    }
                    Err(error) => {
                        self.settings.class_notifications = None;
                        self.error = Some(error);
                        self.queued_page_load |= self.screen == Screen::Settings;
                    }
                }
            }
            NotificationsMessage::LoadEmailNotifications if self.can_read_pages() && !self.settings.loading_email_notifications
                && !self.has_pending_request() && self.screen == Screen::Settings =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.page_errors.remove(&PageRead::EmailNotifications);
                self.settings.loading_email_notifications = true;
                let generation = self.settings.notifications_generation;
                return Task::perform(async move { desktop_auth::load_email_notifications(&session).await },
                    move |result| Message::Notifications(NotificationsMessage::EmailNotificationsLoaded(generation, result)));
            }
            NotificationsMessage::EmailNotificationsLoaded(generation, result) if generation == self.settings.notifications_generation => {
                self.settings.loading_email_notifications = false;
                match result {
                    Ok(settings) => self.settings.email_notifications = Some(settings),
                    Err(error) => { self.page_errors.insert(PageRead::EmailNotifications, error); }
                }
            }
            NotificationsMessage::SetEmailNotification(target, enabled) if self.can_set_email_notification(target, enabled) => {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.settings.saving_email_notification = Some(target);
                self.error = None;
                return Task::perform(async move { desktop_auth::set_email_notification(&session, target, enabled).await },
                    move |result| Message::Notifications(NotificationsMessage::EmailNotificationSaved(target, result)));
            }
            NotificationsMessage::EmailNotificationSaved(target, result) if self.settings.saving_email_notification == Some(target) => {
                self.settings.saving_email_notification = None;
                match result {
                    Ok(settings) => self.settings.email_notifications = Some(settings),
                    Err(error) => {
                        self.settings.email_notifications = None;
                        self.error = Some(error);
                        self.queued_page_load |= self.screen == Screen::Settings;
                    }
                }
            }
            _ => {}
        }
        self.finish_update()
    }
}
