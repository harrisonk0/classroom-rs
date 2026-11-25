use super::*;
use iced::widget::column;

impl App {
    pub(super) fn reset_stream_access(&mut self) {
        self.stream_access_generation = self.stream_access_generation.wrapping_add(1);
        self.loading_stream_access.clear();
        self.stream_access.clear();
        self.stream_viewer_id = None;
        self.page_errors.retain(|read, _| !matches!(read, PageRead::StreamAccess(_)));
    }

    pub(super) fn stream_layout<'a>(&'a self, course: &'a Course) -> Element<'a, Message> {
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

    pub(super) fn on_live_stream(&self, course: i64) -> bool {
        self.screen == Screen::Course(course) && self.tab == Tab::Stream && self.can_read_visible_stream_access(course)
    }

    pub(super) fn can_edit_class_stream_settings(&self, course: i64) -> bool {
        self.can_read_pages() && self.screen == Screen::Course(course)
            && self.class_roles.get(&course) == Some(&CourseRole::Teacher) && self.can_grade_teacher_work(course)
    }

    pub(super) fn class_stream_settings_panel(&self, course: i64) -> Element<'_, Message> {
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

    pub(super) fn can_read_visible_stream_access(&self, course: i64) -> bool {
        (self.screen == Screen::Item(course) || self.screen == Screen::Course(course) && self.tab == Tab::Stream)
            && self.can_read_pages()
            && self.snapshot.as_ref().is_some_and(|snapshot| snapshot.courses.iter()
                .any(|entry| entry.id == course && entry.state == CourseState::Active))
    }

    pub(super) fn missing_stream_access(&self, course: i64) -> bool {
        self.can_read_visible_stream_access(course) && !self.stream_access.contains_key(&course)
            && !self.loading_stream_access.contains(&course)
            && !self.page_errors.contains_key(&PageRead::StreamAccess(course))
    }

    pub(super) fn can_post_stream(&self, course: i64) -> bool {
        self.on_live_stream(course) && self.stream_access.get(&course) == Some(&StreamAccess::PostAndComment)
    }

    pub(super) fn can_delete_stream_post(&self, course: i64, id: i64) -> bool {
        self.on_live_stream(course) && self.displayed_announcement(course, id).is_some_and(|item| {
            self.owns_announcement(item) || self.may_moderate_author(course, item.author_id)
        })
    }

    pub(super) fn displayed_announcement(&self, course: i64, id: i64) -> Option<&Coursework> {
        self.snapshot.as_ref()?.coursework_for(course)?.iter()
            .find(|item| item.kind == CourseworkKind::Announcement && item.id.parse() == Ok(id))
    }

    pub(super) fn owns_announcement(&self, item: &Coursework) -> bool {
        item.kind == CourseworkKind::Announcement
            && self.stream_viewer_id.is_some() && item.author_id == self.stream_viewer_id
    }

    fn stream_row<'a>(&'a self, item: &'a Coursework) -> Element<'a, Message> {
        let course = item.course_id;
        if item.kind != CourseworkKind::Announcement {
            return self.stream_card(item, coursework_row(item, None));
        }
        let mut card = column![].spacing(11).width(Fill);
        if !item.title.eq_ignore_ascii_case("announcement") {
            card = card.push(text(&item.title).font(medium()).size(16).color(FG).wrapping(text::Wrapping::WordOrGlyph));
        }
        if !item.description.trim().is_empty() {
            card = card.push(formatted_description(&item.description, &item.description_styles, &item.description_bullets));
        }
        for attachment in &item.attachments {
            card = card.push(self.stream_attachment_row(item, attachment));
        }
        let mut actions = row![
            button("Comments").padding([7, 10]).style(quiet_style)
                .on_press(Message::OpenItem(course, item.id.clone())),
        ].spacing(6).align_y(Alignment::Center);
        let fixture = self.screenshot_directory.is_some();
        let ready = fixture || !self.has_pending_request();
        if let Ok(id) = item.id.parse::<i64>()
            && (self.owns_announcement(item) || self.may_moderate_author(course, item.author_id))
            && (fixture || self.on_live_stream(course))
        {
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
            actions = actions.push(button(label).padding([8, 12]).style(quiet_style)
                .on_press_maybe(ready.then_some(Message::ConfirmDeleteStreamPost(course, id))));
        }
        self.stream_card(item, card.push(actions).into())
    }

    fn stream_card<'a>(&'a self, item: &'a Coursework, body: Element<'a, Message>) -> Element<'a, Message> {
        let mut card = column![].spacing(10).width(Fill);
        let posted_at = item.posted_at_millis.and_then(DateTime::from_timestamp_millis)
            .map(|date| date.with_timezone(&Local).format("%-d %b %Y at %H:%M").to_string());
        let mut timestamp = posted_at.map(|label| text(label).size(13).color(MUTED));
        if let Some(id) = item.author_id {
            let profile = self.stream_profiles.get(&id);
            let name = profile.and_then(|profile| profile.name.as_deref()).or_else(|| {
                self.snapshot.as_ref()?.roster_for(item.course_id)?.iter()
                    .find(|member| member.id.parse::<i64>().ok() == Some(id))
                    .map(|member| member.display_name.as_str())
            });
            if let Some(name) = name {
                let picture = profile.and_then(|profile| profile.photo_url.as_ref())
                    .and_then(|url| self.stream_profile_pictures.get(url));
                let face = picture.map_or_else(|| avatar(first_initial(name), BLUE, 36.0, 16.0), |picture|
                    image(picture.clone()).width(36).height(36).into());
                let mut heading = column![text(name).font(medium()).size(15).color(FG)].spacing(3);
                if let Some(time) = timestamp.take() {
                    heading = heading.push(time);
                }
                card = card.push(row![face, heading]
                    .spacing(10).align_y(Alignment::Center));
            }
        }
        if let Some(time) = timestamp {
            card = card.push(time);
        }
        container(card.push(body)).width(Fill).padding(16).style(|_| outline_surface(9.0)).into()
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

    pub(super) fn set_demo_stream_display(&mut self, display: StreamClassworkDisplay) {
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
            return self.stream_card(item, row);
        }
        let mut details = column![].spacing(8);
        if let Some(due) = &item.due_label {
            details = details.push(text(due).size(13).color(MUTED));
        }
        if !item.description.trim().is_empty() {
            details = details.push(formatted_description(&item.description, &item.description_styles, &item.description_bullets));
        }
        for attachment in &item.attachments {
            details = details.push(self.stream_attachment_row(item, attachment));
        }
        self.stream_card(item, column![row, details].spacing(4).into())
    }

    pub(super) fn stream_attachment_row<'a>(&'a self, item: &'a Coursework, attachment: &'a desktop_data::Attachment) -> Element<'a, Message> {
        let template = attachment.drive_sharing == Some(DriveSharing::IndividualCopy);
        let action = if template {
            Some(Message::OpenItem(item.course_id, item.id.clone()))
        } else {
            self.attachment_action(attachment)
        };
        let file = button(row![
            icon(attachment_icon(attachment.kind), BLUE, 16),
            text(&attachment.title).size(14).color(FG).width(Fill).wrapping(text::Wrapping::WordOrGlyph),
            icon(Icon::Arrow, MUTED, 14),
        ].spacing(8).align_y(Alignment::Center)).padding([7, 10]).width(Fill)
            .style(outline_style).on_press_maybe(action);
        let mut row = row![file].spacing(6).align_y(Alignment::Center);
        if !template && let Some(browser) = self.attachment_browser_button(attachment) {
            row = row.push(browser);
        }
        row.into()
    }

    pub(super) fn attachment_action(&self, attachment: &desktop_data::Attachment) -> Option<Message> {
        #[cfg(target_os = "macos")]
        if self.can_read_pages() && attachment.drive_reference.is_some()
            && attachment.kind != desktop_data::AttachmentKind::Form
        {
            return (!self.preparing_drive_preview).then(|| Message::PreviewDriveFile(attachment.clone()));
        }
        self.attachment_browser_action(attachment)
    }

    pub(super) fn attachment_browser_button<'a>(&'a self, attachment: &desktop_data::Attachment) -> Option<Element<'a, Message>> {
        if !self.can_read_pages() || attachment.drive_reference.is_none()
            || attachment.kind == desktop_data::AttachmentKind::Form
        {
            return None;
        }
        Some(icon_action(Icon::External, "Open in browser", self.attachment_browser_action(attachment)))
    }

    pub(super) fn save_attachment_button(&self, file: &desktop_data::Attachment) -> Option<Element<'static, Message>> {
        (cfg!(target_os = "macos") && self.can_read_pages()
            && file.kind == desktop_data::AttachmentKind::DriveFile && file.drive_reference.is_some())
            .then(|| icon_action(Icon::Download, "Save a copy…", (!self.has_pending_request())
                .then(|| Message::SaveDriveFile(file.clone()))))
    }

    fn attachment_browser_action(&self, attachment: &desktop_data::Attachment) -> Option<Message> {
        if self.verify_live_course_count {
            return None;
        }
        let url = attachment.url.as_deref().filter(|url| trusted_attachment_host(url).is_some());
        if let Some(id) = attachment.drive_reference.as_deref()
            && attachment.kind != desktop_data::AttachmentKind::Form
        {
            if let Some(url) = url.filter(|url| desktop_auth::drive_url_reference(url).as_deref() == Some(id)) {
                return Some(Message::OpenAttachment(url.to_owned()));
            }
            return (self.can_read_pages() && self.opening_drive_file.is_none())
                .then(|| Message::OpenDriveFile(id.to_owned()));
        }
        url.map(|url| Message::OpenAttachment(url.to_owned()))
    }
}
