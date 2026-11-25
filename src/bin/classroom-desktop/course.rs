use super::*;
use iced::widget::column;

impl App {
    pub(super) fn course(&self, id: i64) -> Element<'_, Message> {
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
            icon_action(Icon::Copy, "Copy class link",
                Some(Message::CopyClassroomLink(ClassroomLink::Course { course: id, classwork: self.tab == Tab::Classwork }))),
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
                    .style(move |theme, status| tab_style(theme, active, status))
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
}
