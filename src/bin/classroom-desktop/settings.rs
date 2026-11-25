use super::*;
use iced::widget::column;

impl App {
    pub(super) fn reset_notifications(&mut self) {
        self.notifications_generation = self.notifications_generation.wrapping_add(1);
        self.class_notifications = None;
        self.loading_class_notifications = false;
        self.page_errors.remove(&PageRead::ClassNotifications);
        self.email_notifications = None;
        self.loading_email_notifications = false;
        self.page_errors.remove(&PageRead::EmailNotifications);
    }

    pub(super) fn class_notification_courses(&self) -> impl Iterator<Item = &Course> {
        self.snapshot.iter().flat_map(|snapshot| &snapshot.courses)
            .filter(|course| course.state == CourseState::Active)
    }

    pub(super) fn can_set_class_notifications(&self, id: i64, enabled: bool) -> bool {
        self.screen == Screen::Settings && self.can_read_pages() && !self.has_pending_request()
            && !self.loading_class_notifications
            && self.email_notifications.as_ref().and_then(|settings| settings.enabled) != Some(false)
            && self.class_notification_courses().any(|course| course.id == id)
            && self.class_notifications.as_ref().and_then(|map| map.get(&id)) == Some(&enabled)
    }

    pub(super) fn can_set_email_notification(&self, target: EmailNotificationTarget, enabled: bool) -> bool {
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

    pub(super) fn settings(&self) -> Element<'_, Message> {
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
}

pub(super) fn notification_switch(
    enabled: bool, saving: bool, ready: bool, change: impl Fn(bool) -> Message + 'static,
) -> Element<'static, Message> {
    let status = if saving { "Saving…" } else if enabled { "On" } else { "Off" };
    toggler(enabled).label(status).size(19).text_size(14).width(112)
        .on_toggle_maybe(ready.then_some(change)).into()
}
