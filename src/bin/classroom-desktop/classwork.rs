use super::*;
use iced::widget::column;

impl App {
    pub(super) fn apply_teacher_topics(&mut self, course: i64, topics: Vec<desktop_data::Topic>, removed: Option<&str>) {
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

    pub(super) fn can_manage_topics(&self, course: i64) -> bool {
        self.can_edit_assignment_drafts(course)
    }

    pub(super) fn apply_item_topic(&mut self, target: &desktop_auth::TopicItemTarget, topic: Option<String>) {
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

    pub(super) fn item_topic_editor_panel<'a>(&'a self, editor: &'a ItemTopicEditor) -> Element<'a, Message> {
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

    pub(super) fn teacher_topic_panel(&self, course: i64) -> Option<Element<'_, Message>> {
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

    pub(super) fn classwork(&self, course_id: i64) -> Element<'_, Message> {
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

    pub(super) fn teacher_draft_panel(&self, course: i64, announcements: bool) -> Option<Element<'_, Message>> {
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

    pub(super) fn can_edit_assignment_drafts(&self, course: i64) -> bool {
        self.can_read_pages() && self.screen == Screen::Course(course) && self.tab == Tab::Classwork
            && self.class_roles.get(&course) == Some(&CourseRole::Teacher)
            && self.snapshot.as_ref().is_some_and(|snapshot|
                snapshot.courses.iter().any(|entry| entry.id == course && entry.state == CourseState::Active))
    }

    pub(super) fn assignment_editor_panel<'a>(&'a self, editor: &'a AssignmentEditor) -> Element<'a, Message> {
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

    pub(super) fn assignment_publication_panel<'a>(&'a self, prompt: &'a AssignmentPublicationPrompt) -> Element<'a, Message> {
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

    pub(super) fn classwork_item_expanded(&self, item: &str) -> bool {
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
            .style(move |theme, status| nav_style(theme, active, status))
            .on_press(Message::SelectTopic(topic.map(str::to_owned)))
            .into()
    }
}

pub(super) fn topic_action_valid(action: &desktop_auth::TeacherTopicAction) -> bool {
    use desktop_auth::{TeacherTopicAction, normalized_topic_name};
    match action {
        TeacherTopicAction::Create(name) => normalized_topic_name(name).is_some(),
        TeacherTopicAction::Rename { topic, name } =>
            normalized_topic_name(name).is_some_and(|name| name != topic.title),
        TeacherTopicAction::Delete(topic) => !topic.id.is_empty(),
    }
}
