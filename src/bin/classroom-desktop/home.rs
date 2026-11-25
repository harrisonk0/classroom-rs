use super::*;
use iced::widget::column;

impl App {
    pub(super) fn home(&self, archived: bool) -> Element<'_, Message> {
        let Some(data) = &self.snapshot else { return self.sign_in_view() };
        let now = self.now;
        let query = self.search.trim().to_lowercase();
        let matching_state = |course: &&Course| {
            if archived { course.state == CourseState::Archived } else { home_course(course) }
        };
        let matches_search = |course: &&Course| {
            [&course.title, &course.subtitle, &course.description, &course.room]
                .iter().any(|item| item.to_lowercase().contains(&query))
        };
        let group_count = data.courses.iter().filter(matching_state).count();
        let mut visible: Vec<&Course> = data.courses.iter()
            .filter(matching_state)
            .filter(matches_search)
            .collect();
        if self.alphabetical {
            visible.sort_by_cached_key(|course| course.title.to_lowercase());
        }
        let course_count = if visible.len() == group_count {
            format!("{} {}", group_count, if group_count == 1 { "class" } else { "classes" })
        } else {
            format!("{} of {} classes", visible.len(), group_count)
        };
        let search = text_input("Search classes", &self.search).id("class-search")
            .on_input(Message::Search)
            .size(14)
            .padding([11, 13])
            .width(270)
            .style(search_style);
        let sort = button(if self.alphabetical { "A–Z" } else { "Classroom order" })
            .style(outline_style)
            .padding([10, 12])
            .on_press(Message::SortCourses);
        let refresh: Element<'_, Message> = if self.session.is_some() && !self.verify_live_course_count {
            button(row![
                icon(Icon::Refresh, BLUE, 17),
                text(if self.refreshing { "Refreshing…" } else { "Refresh" }).size(14),
            ].spacing(7).align_y(Alignment::Center))
            .padding([10, 12]).style(outline_style)
            .on_press_maybe((!self.has_pending_request()).then_some(Message::Refresh))
            .into()
        } else {
            space().width(0).into()
        };
        let toolbar = row![search, sort, refresh].spacing(9).align_y(Alignment::Center);
        let heading = row![
            column![text(if archived { "Archived classes" } else { "Classes" }).size(27).color(FG), text(course_count).size(14).color(MUTED)]
                .spacing(4).width(Fill),
            toolbar,
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        let classes: Element<'_, Message> = if visible.is_empty() {
            let (title, body) = if query.is_empty() && archived {
                ("No archived classes", "The current course list doesn't contain any classes marked as archived.")
            } else if query.is_empty() && data.courses.iter().any(hidden_course) {
                ("No visible classes", "Choose Show hidden classes below to see your hidden classes.")
            } else if query.is_empty() {
                ("No classes yet", "There are no courses to display for this account.")
            } else {
                ("No matching classes", "Try another name, section or room.")
            };
            empty_panel(Icon::Classroom, title, body)
        } else {
            row(visible.into_iter().map(|course| self.course_card(course, &now)))
                .spacing(24)
                .wrap()
                .vertical_spacing(24)
                .into()
        };
        let offline_controls: Element<'_, Message> = if data.source == SnapshotSource::Demo || self.verify_live_course_count {
            space().height(0).into()
        } else {
            let save: Element<'_, Message> = if data.source == SnapshotSource::Live {
                button("Save offline copy").style(quiet_style).padding([8, 11])
                    .on_press_maybe((!self.has_pending_request()).then_some(Message::Offline(OfflineMessage::Save))).into()
            } else {
                space().width(0).into()
            };
            row![
                save,
                button("View saved copy").style(quiet_style).padding([8, 11])
                    .on_press_maybe((!self.has_pending_request()).then_some(Message::Offline(OfflineMessage::Open))),
                button("Remove offline copy").style(quiet_style).padding([8, 11])
                    .on_press_maybe((!self.has_pending_request()).then_some(Message::Offline(OfflineMessage::Delete))),
            ].spacing(10).into()
        };
        let mut page = column![].spacing(25);
        if !archived && (self.can_read_pages() || self.screenshot_directory.is_some())
            && let Some(invitations) = self.invitation_cards()
        {
            page = page.push(invitations);
        }
        if !archived && !self.verify_live_course_count && !data.courses.is_empty() {
            page = page.push(self.due_soon(data, &now));
        }
        page = page.push(heading).push(classes);
        if !archived && data.courses.iter().any(hidden_course) {
            page = page.push(button(if self.show_hidden_classes { "Collapse hidden classes" } else { "Show hidden classes" })
                .style(outline_style).padding([9, 12]).on_press(Message::ToggleHiddenClasses));
            if self.show_hidden_classes {
                let mut hidden: Vec<_> = data.courses.iter().filter(|course| hidden_course(course))
                    .filter(matches_search).collect();
                if self.alphabetical {
                    hidden.sort_by_cached_key(|course| course.title.to_lowercase());
                }
                let cards: Element<'_, Message> = if hidden.is_empty() {
                    text("No matching hidden classes.").size(14).color(MUTED).into()
                } else {
                    row(hidden.into_iter().map(|course| self.course_card(course, &now)))
                        .spacing(24).wrap().vertical_spacing(24).into()
                };
                page = page.push(column![text("Hidden classes").size(21).font(medium()).color(FG), cards].spacing(12));
            }
        }
        scrollable(page.push(offline_controls).padding(26).width(Fill))
            .width(Fill).height(Fill)
            .into()
    }

    fn due_soon<'a>(&'a self, snapshot: &'a Snapshot, now: &DateTime<Local>) -> Element<'a, Message> {
        let title = if snapshot.source == SnapshotSource::Cache { "Upcoming deadlines" } else { "Due soon" };
        let mut panel = column![row![
            text(title).size(21).font(medium()).color(FG),
            space().width(Fill),
            button("View to-dos").padding([8, 12]).style(quiet_style)
                .on_press(Message::Navigate(Screen::ToDo)),
        ].align_y(Alignment::Center)].spacing(8);
        let items = snapshot.upcoming_work(now, None, 5);
        let status = self.page_read_status(PageRead::Assignments, self.assignments_loaded, "Checking upcoming work…");
        if let Some(status) = status {
            panel = panel.push(status);
        } else if items.is_empty() {
            panel = panel.push(text("No work due in the next seven days.").size(14).color(MUTED));
        }
        for (item, course) in items {
            let accent = course_color(course);
            let deadline = item.due_label.as_deref().unwrap_or("Due date available");
            panel = panel.push(button(row![
                icon(Icon::Check, accent, 19),
                column![
                    text(&item.title).size(15).font(medium()).color(FG).width(Fill)
                        .wrapping(text::Wrapping::WordOrGlyph),
                    text(&course.title).size(13).color(MUTED).width(Fill)
                        .wrapping(text::Wrapping::WordOrGlyph),
                ].spacing(2).width(Fill),
                text(deadline).size(13).color(MUTED),
                icon(Icon::Arrow, MUTED, 15),
            ].spacing(10).align_y(Alignment::Center))
                .padding([7, 9]).width(Fill).style(quiet_style)
                .on_press(Message::OpenItem(course.id, item.id.clone())));
        }
        container(panel).padding([12, 16]).width(Fill).style(|_| outline_surface(10.0)).into()
    }

    fn sign_in_view(&self) -> Element<'_, Message> {
        let form = column![
            container(classroom_mark()).padding(10).style(|_| surface(color!(0xe6f4ea), 12.0)),
            text("Your classes").color(FG).size(30).font(medium()),
            text("Sign in to browse your Google Classroom courses.").size(15).color(MUTED),
            text_input("Google account email", &self.email)
                .padding([12, 13]).size(15).style(search_style).on_input(Message::Email)
                .on_submit_maybe(self.can_sign_in().then_some(Message::SignIn)),
            row![
                button(if self.busy {
                        "Waiting for Google…"
                    } else if self.verification_submitted {
                        "Verification ended"
                    } else {
                        "Sign in with Google"
                    })
                    .padding([11, 17]).style(primary_style)
                    .on_press_maybe(self.can_sign_in().then_some(Message::SignIn)),
                button("Explore demo").padding([11, 13]).style(quiet_style)
                    .on_press_maybe((!self.busy).then_some(Message::Demo))
            ].spacing(8).align_y(Alignment::Center),
            text(if self.busy { "Complete sign-in in the separate Google window." }
                else { "Google opens in a separate sign-in window." })
                .size(13).color(MUTED),
        ]
        .spacing(15)
        .max_width(410);
        container(container(form).padding(30).style(|_| outline_surface(12.0)))
            .center_x(Fill).center_y(Fill).padding(20).into()
    }

    fn course_card<'a>(&'a self, course: &'a Course, now: &DateTime<Local>) -> Element<'a, Message> {
        let teacher = self.class_teachers.get(&course.id).map(String::as_str);
        let header = course_banner(course, self.artwork_for(course), 110.0, false, teacher);
        let mut preview = column![].spacing(7);
        if let Some(snapshot) = &self.snapshot {
            for (item, _) in snapshot.upcoming_work(now, Some(course.id), 2) {
                preview = preview.push(column![
                    text(item.due_label.as_deref().unwrap_or_default()).size(12).color(MUTED),
                    text(truncate(&item.title, 35)).size(14).color(FG),
                ].spacing(2));
            }
        }
        let body = container(preview).width(Fill).height(89).padding([11, 14]);
        let folder: Element<'_, Message> = match self.class_folder_url(course.id) {
            Some(url) => button(row![icon(Icon::Folder, MUTED, 17), text("Drive folder").size(13)]
                .spacing(6).align_y(Alignment::Center)).padding(9).style(quiet_style)
                .on_press(Message::OpenAttachment(url.to_owned())).into(),
            None => space().width(0).into(),
        };
        let footer = container(row![
            folder,
            space().width(Fill),
            button(row![icon(Icon::Check, MUTED, 17), text(if self.class_roles.get(&course.id) == Some(&CourseRole::Teacher) {
                "Open gradebook"
            } else { "Your work" }).size(13)]
                .spacing(6).align_y(Alignment::Center))
                .padding(9).style(quiet_style).on_press(if self.class_roles.get(&course.id) == Some(&CourseRole::Teacher) {
                    Message::OpenGradebook(course.id)
                } else { Message::OpenYourWork(course.id) }),
        ].align_y(Alignment::Center)).width(Fill).padding([2, 9]);
        let card_link = button(column![header, body]).padding(0).width(Fill)
            .style(quiet_style).on_press(Message::Navigate(Screen::Course(course.id)));
        let shown = course.hidden != Some(true);
        let fixture = self.screenshot_directory.is_some() && course.hidden.is_some() && self.screen == Screen::Home;
        let can_toggle = self.can_change_course_visibility(course.id, shown);
        let archive = self.screen == Screen::Home;
        let archive_fixture = self.screenshot_directory.is_some() && self.confirm_class_archive == Some(course.id);
        let can_archive = self.can_change_class_archive(course.id, archive) || archive_fixture;
        let can_move = self.can_move_course(course.id) || fixture && shown && course.state == CourseState::Active;
        let can_drag = can_move && !self.alphabetical && self.search.trim().is_empty() && !self.has_pending_request();
        let card_content: Element<'_, Message> = if can_toggle || can_move || can_archive || fixture {
            let menu = button(text("⋮").size(18).color(WHITE)).padding([2, 8])
                .style(|_, status| button::Style {
                    background: Some(Background::Color(Color { a: if status == button::Status::Hovered { 0.72 } else { 0.42 }, ..Color::BLACK })),
                    text_color: WHITE,
                    border: Border { radius: 15.0.into(), ..Default::default() },
                    ..Default::default()
                })
                .on_press(Message::ToggleCourseMenu(course.id));
            let mut actions = row![].spacing(5);
            if can_drag {
                let grip = container(icon(Icon::Grip, WHITE, 16)).padding(6)
                    .style(|_| surface(Color { a: 0.42, ..Color::BLACK }, 15.0));
                let handle = mouse_area(grip).on_press(Message::BeginCourseDrag(course.id))
                    .interaction(if self.course_drag.is_some() { mouse::Interaction::Grabbing } else { mouse::Interaction::Grab });
                actions = actions.push(tooltip(handle, "Drag to reorder", tooltip::Position::Bottom));
            }
            stack![card_link, container(actions.push(menu)).align_right(Fill).padding(8)].into()
        } else {
            card_link.into()
        };
        let mut card = column![card_content, rule::horizontal(1).style(divider_style), footer];
        if self.open_course_menu == Some(course.id) && (can_toggle || can_move || can_archive || fixture) {
            let ready = !self.has_pending_request() || fixture;
            let choosing_move = self.move_course_menu == Some(course.id) && can_move;
            let action = if shown {
                if self.confirm_hide_course == Some(course.id) {
                    Message::ChangeCourseVisibility(course.id, true)
                } else {
                    Message::ConfirmHideCourse(course.id)
                }
            } else {
                Message::ChangeCourseVisibility(course.id, false)
            };
            let confirming_hide = self.confirm_hide_course == Some(course.id);
            let confirming_archive = self.confirm_class_archive == Some(course.id);
            let mut menu = column![].spacing(6);
            if choosing_move {
                menu = menu.push(text("Move class").size(14).font(medium()).color(FG))
                    .push(pick_list(self.course_move_options(course.id), None::<CourseMoveOption>, move |choice| {
                        Message::MoveCourse(course.id, choice.placement)
                    }).placeholder("Choose a position").width(Fill));
            } else if confirming_hide {
                menu = menu.push(text("Hide this class from Home, To-do and Calendar?").size(14).color(FG))
                    .push(text("You’ll stay enrolled.").size(13).color(MUTED));
            } else if confirming_archive {
                menu = menu.push(text(if archive { "Archive this class?" } else { "Restore this class?" }).size(14).color(FG))
                    .push(text(if archive {
                        "The class moves to Archived classes. Members can still view its work."
                    } else {
                        "The class returns to the active class list."
                    }).size(13).color(MUTED));
            }
            let mut controls = row![
                button("Cancel").padding([6, 9]).style(quiet_style)
                    .on_press(Message::ToggleCourseMenu(course.id)),
            ].spacing(6);
            if !choosing_move && !confirming_archive && (can_toggle || fixture) {
                controls = controls.push(button(if shown { "Hide" } else { "Unhide" })
                    .padding([6, 9]).style(outline_style).on_press_maybe(ready.then_some(action)));
            }
            if !choosing_move && !confirming_hide && !confirming_archive && can_move {
                controls = controls.push(button("Move…").padding([6, 9]).style(outline_style)
                    .on_press_maybe(ready.then_some(Message::ChooseCourseMove(course.id))));
            }
            if !choosing_move && !confirming_hide && can_archive {
                let action = if confirming_archive { Message::ChangeClassArchive(course.id, archive) }
                    else { Message::ConfirmClassArchive(course.id) };
                controls = controls.push(button(if archive { "Archive" } else { "Restore" })
                    .padding([6, 9]).style(outline_style).on_press_maybe(ready.then_some(action)));
            }
            card = card.push(container(menu.push(controls)).width(Fill).padding([5, 10]));
        }
        let target = self.course_drag.as_ref().and_then(|drag| drag.target) == Some(course.id);
        let card = container(card).width(280).style(move |_| {
            let mut style = outline_surface(9.0);
            if target {
                style.border.color = BLUE;
                style.border.width = 2.0;
            }
            style
        }).clip(true);
        if self.course_drag.is_some() && can_move {
            mouse_area(card).on_enter(Message::EnterCourseDropTarget(course.id))
                .on_exit(Message::ExitCourseDropTarget(course.id)).interaction(mouse::Interaction::Grabbing).into()
        } else {
            card.into()
        }
    }

    fn visible_active_classes(&self) -> impl Iterator<Item = &Course> {
        self.snapshot.iter().flat_map(|snapshot| &snapshot.courses)
            .filter(|course| course.state == CourseState::Active && course.shows_on_home())
    }

    pub(super) fn can_move_course(&self, id: i64) -> bool {
        self.screen == Screen::Home && self.can_read_pages() && self.visible_active_classes().take(2).count() == 2
            && self.visible_active_classes().any(|course| course.id == id)
    }

    pub(super) fn course_move_options(&self, moved: i64) -> Vec<CourseMoveOption> {
        let mut options = vec![CourseMoveOption { label: "To beginning".into(), placement: CoursePlacement::Beginning }];
        options.extend(self.visible_active_classes().filter(|course| course.id != moved).map(|course| CourseMoveOption {
            label: if course.subtitle.trim().is_empty() { format!("After {}", course.title) }
                else { format!("After {} · {}", course.title, course.subtitle) },
            placement: CoursePlacement::After(course.id),
        }));
        options.push(CourseMoveOption { label: "To end".into(), placement: CoursePlacement::End });
        options
    }

    pub(super) fn drop_placement(&self, source: i64, target: i64) -> Option<CoursePlacement> {
        let ordered: Vec<_> = self.visible_active_classes().map(|course| course.id).collect();
        let source_index = ordered.iter().position(|&course| course == source)?;
        let target_index = ordered.iter().position(|&course| course == target)?;
        if source_index < target_index {
            Some(CoursePlacement::After(target))
        } else if source_index > target_index {
            Some(CoursePlacement::Before(target))
        } else {
            None
        }
    }

    pub(super) fn can_change_membership(&self) -> bool {
        self.session.is_some() && !self.verify_live_course_count
            && self.snapshot.as_ref().is_some_and(|snapshot| snapshot.source == SnapshotSource::Live)
    }

    pub(super) fn can_create_class(&self) -> bool {
        self.can_change_membership() && self.class_creation_allowed
    }

    pub(super) fn can_change_course_visibility(&self, id: i64, hidden: bool) -> bool {
        self.screen == Screen::Home && self.can_read_pages() && self.snapshot.as_ref()
            .is_some_and(|snapshot| snapshot.courses.iter().any(|course| {
                course.id == id && course.state == CourseState::Active && course.hidden == Some(!hidden)
                    && (hidden || self.show_hidden_classes)
            }))
    }

    pub(super) fn can_change_class_archive(&self, id: i64, archive: bool) -> bool {
        let (screen, state) = if archive { (Screen::Home, CourseState::Active) }
            else { (Screen::Archived, CourseState::Archived) };
        self.screen == screen && self.can_read_pages()
            && self.class_roles.get(&id) == Some(&CourseRole::Teacher)
            && self.snapshot.as_ref().is_some_and(|snapshot| snapshot.courses.iter()
                .any(|course| course.id == id && course.state == state))
    }

    pub(super) fn invalidate_invitation_read(&mut self) {
        self.invitation_generation = self.invitation_generation.wrapping_add(1);
        self.loading_invitations = false;
    }

    pub(super) fn pending_invitations(&self) -> impl Iterator<Item = &Course> {
        self.invitations.iter().flatten().filter(|invited| {
            self.snapshot.as_ref().is_none_or(|snapshot| !snapshot.courses.iter().any(|course| course.id == invited.id))
        })
    }

    pub(super) fn can_respond_to_invitation(&self, course_id: i64) -> bool {
        matches!(self.screen, Screen::Home | Screen::Join) && self.can_change_membership()
            && !self.has_pending_request() && !self.loading_invitations
            && self.pending_invitations().any(|course| course.id == course_id)
    }

    fn invitation_actions(&self, course: i64) -> Element<'_, Message> {
        let ready = self.can_respond_to_invitation(course) || self.screenshot_directory.is_some();
        if self.confirming_decline == Some(course) {
            return column![
                text("Decline this invitation?").size(14).color(FG),
                row![
                    button("Cancel").padding([7, 12]).style(quiet_style)
                        .on_press(Message::CancelDeclineInvitation),
                    button("Decline").padding([7, 12]).style(outline_style)
                        .on_press_maybe(ready.then_some(Message::DeclineInvitation(course))),
                ].spacing(6),
            ].spacing(6).into();
        }
        row![
            button("Decline").padding([7, 12]).style(quiet_style)
                .on_press_maybe(ready.then_some(Message::ConfirmDeclineInvitation(course))),
            button("Join").padding([7, 12]).style(primary_style)
                .on_press_maybe(ready.then_some(Message::AcceptInvitation(course))),
        ].spacing(6).into()
    }

    fn invitation_cards(&self) -> Option<Element<'_, Message>> {
        let mut invitations = self.pending_invitations().peekable();
        let error = self.can_read_pages() && self.page_errors.contains_key(&PageRead::Invitations);
        if invitations.peek().is_none() && !error {
            return None;
        }
        let mut section = column![text("Class invitations").size(21).font(medium()).color(FG)].spacing(10);
        if error && let Some(status) = self.page_read_status(PageRead::Invitations, false, "Checking invitations…") {
            section = section.push(status);
        }
        if invitations.peek().is_some() {
            section = section.push(row(invitations.map(|invited| {
                let card = column![
                    container(column![
                        text(&invited.title).size(18).font(medium()).color(WHITE)
                            .wrapping(text::Wrapping::WordOrGlyph),
                        text(&invited.subtitle).size(13).color(WHITE),
                    ].spacing(4)).padding(13).width(Fill)
                        .style(move |_| surface(course_color(invited), 0.0)),
                    container(self.invitation_actions(invited.id)).padding(10).align_right(Fill),
                ];
                container(card).width(280).style(|_| outline_surface(9.0)).clip(true).into()
            })).spacing(14).wrap().vertical_spacing(12));
        }
        Some(section.into())
    }

    pub(super) fn join_class_view(&self) -> Element<'_, Message> {
        let ready = self.can_change_membership() && !self.has_pending_request()
            && desktop_auth::valid_class_code(&self.class_code);
        let form = column![
            text("Join class").size(27).color(FG),
            text("Enter the class code from your teacher.").size(15).color(MUTED),
            text_input("Class code", &self.class_code).padding(11)
                .on_input(Message::EditClassCode)
                .on_submit_maybe(ready.then_some(Message::JoinClass)),
            row![
                button("Cancel").style(quiet_style).padding([9, 12])
                    .on_press(Message::Navigate(Screen::Home)),
                button(if self.changing_membership { "Joining…" } else { "Join" })
                    .style(primary_style).padding([9, 13]).on_press_maybe(ready.then_some(Message::JoinClass)),
            ].spacing(10),
        ].spacing(16).max_width(400);
        let mut invitations = column![text("Invitations").size(21).font(medium()).color(FG)].spacing(12);
        let mut rows = self.pending_invitations().peekable();
        if self.invitations.is_some() && rows.peek().is_none() && !self.loading_invitations {
            invitations = invitations.push(text("No pending class invitations.").size(14).color(MUTED));
        }
        for course in rows {
            invitations = invitations.push(row![
                column![text(&course.title).size(15), text(&course.subtitle).size(13).color(MUTED)].width(Fill),
                self.invitation_actions(course.id),
            ].spacing(12).align_y(Alignment::Center));
        }
        if let Some(status) = self.page_read_status(
            PageRead::Invitations, self.invitations.is_some(), "Loading invitations…",
        ) {
            invitations = invitations.push(status);
        }
        let panels = column![
            container(form).padding(25).style(|_| outline_surface(12.0)).width(Fill),
            container(invitations).padding(25).style(|_| outline_surface(12.0)).width(Fill),
        ].spacing(16).max_width(560);
        scrollable(container(panels).center_x(Fill).padding(24)).height(Fill).into()
    }

    pub(super) fn create_class_view(&self) -> Element<'_, Message> {
        let editing = !self.changing_membership;
        let ready = self.can_create_class() && !self.has_pending_request()
            && desktop_auth::class_input_valid(&self.new_class);
        let mut form = column![text("Create class").font(medium()).size(24).color(FG)].spacing(14);
        for (field, label, value) in [
            (NewClassField::Name, "Class name (required)", self.new_class.name.as_str()),
            (NewClassField::Section, "Section", self.new_class.section.as_str()),
            (NewClassField::Subject, "Subject", self.new_class.subject.as_str()),
            (NewClassField::Room, "Room", self.new_class.room.as_str()),
        ] {
            form = form.push(column![
                text(label).size(13).color(MUTED),
                text_input(label, value).padding(10)
                    .on_input_maybe(editing.then_some(move |value| Message::EditNewClass(field, value))),
            ].spacing(4));
        }
        let actions = row![
            button("Cancel").style(quiet_style).padding([9, 12])
                .on_press_maybe(editing.then_some(Message::Navigate(Screen::Home))),
            button(if self.changing_membership { "Creating…" } else { "Create" })
                .style(primary_style).padding([9, 13]).on_press_maybe(ready.then_some(Message::CreateClass)),
        ].spacing(10);
        let panel = container(form.push(actions)).padding(24).width(Fill).max_width(490)
            .style(|_| outline_surface(12.0));
        scrollable(container(panel).center_x(Fill).padding(24)).height(Fill).into()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct CourseMoveOption {
    pub(super) label: String,
    pub(super) placement: CoursePlacement,
}

impl fmt::Display for CourseMoveOption {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}

pub(super) struct CourseDrag {
    pub(super) source: i64,
    pub(super) target: Option<i64>,
}
