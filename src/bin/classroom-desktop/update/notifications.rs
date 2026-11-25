use super::*;

impl App {
    pub(super) fn update_notifications(&mut self, message: Message) -> Task<Message> {
        match message {
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
            _ => {}
        }
        self.finish_update()
    }
}
