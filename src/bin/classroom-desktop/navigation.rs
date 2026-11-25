use super::*;
use iced::widget::column;

impl App {
    pub(super) fn view(&self) -> Element<'_, Message> {
        let sidebar: Element<'_, Message> = if self.sidebar {
            self.navigation()
        } else {
            space().width(0).into()
        };
        let content = match self.screen {
            Screen::Home => self.home(false),
            Screen::Join => self.join_class_view(),
            Screen::CreateClass => self.create_class_view(),
            Screen::Archived => self.home(true),
            Screen::Course(id) => self.course(id),
            Screen::Item(id) => self.item_detail(id),
            Screen::ToDo => self.todo(),
            Screen::Calendar => self.calendar(),
            Screen::Settings => self.settings(),
        };
        let banner_message = self.error.as_ref().or(self.notice.as_ref());
        let error_banner: Element<'_, Message> = if let Some(message) = banner_message {
            let is_error = self.error.is_some();
            let foreground = if is_error { color!(0x9b2722) } else { color!(0x137333) };
            let background = if is_error { color!(0xfce8e6) } else { color!(0xe6f4ea) };
            container(
                row![
                    icon(Icon::Info, foreground, 19),
                    text(message).size(14).style(move |theme| themed_text(theme, foreground)).width(Fill)
                ]
                .spacing(11)
                .align_y(Alignment::Center),
            )
            .style(move |theme| surface(appearance_color(theme, background), 9.0))
            .padding([11, 14])
            .width(Fill)
            .into()
        } else {
            space().height(0).into()
        };
        let mut main = column![
            container(error_banner).padding(if banner_message.is_some() {
                Padding { top: 12.0, right: 28.0, bottom: 0.0, left: 28.0 }
            } else {
                Padding::ZERO
            }),
        ];
        if let Some(value) = &self.classroom_link_input {
            main = main.push(container(self.classroom_link_form(value)).padding([12, 26]).width(Fill));
        }
        let main = main.push(content).width(Fill).height(Fill);
        container(column![
            self.header(),
            rule::horizontal(1).style(divider_style),
            row![sidebar, main].width(Fill).height(Fill),
        ])
        .width(Fill)
        .height(Fill)
        .style(|theme| surface(classroom_palette(theme).background, 0.0))
        .into()
    }

    fn classroom_link_form<'a>(&'a self, value: &'a str) -> Element<'a, Message> {
        let ready = !self.has_pending_request() && !self.checking_classroom_link && !value.trim().is_empty();
        let input = text_input("Paste a classroom.google.com link", value).padding(9).width(Fill)
            .on_input(|value| Message::Navigation(NavigationMessage::EditClassroomLink(value))).on_submit_maybe(ready.then_some(Message::Navigation(NavigationMessage::OpenClassroomLink)));
        container(column![
            text("Open Classroom link").size(17).font(medium()).style(move |theme| themed_text(theme, FG)),
            row![
                input,
                button("Cancel").style(quiet_style).padding([7, 9]).on_press(Message::Navigation(NavigationMessage::CloseClassroomLink)),
                button(if self.checking_classroom_link { "Checking account…" } else { "Open" })
                    .style(primary_style).padding([8, 12])
                    .on_press_maybe(ready.then_some(Message::Navigation(NavigationMessage::OpenClassroomLink))),
            ].spacing(8).align_y(Alignment::Center),
        ].spacing(8)).padding(12).style(|theme| outline_surface(theme, 8.0)).into()
    }

    pub(super) fn invalidate_classroom_link_check(&mut self) {
        self.classroom_link_generation = self.classroom_link_generation.wrapping_add(1);
        self.checking_classroom_link = false;
    }

    pub(super) fn open_classroom_link(&mut self, target: ClassroomLink) -> Task<Message> {
        let course = target.course();
        let Some(snapshot) = self.snapshot.as_ref().filter(|snapshot| snapshot.courses.iter().any(|entry| entry.id == course)) else {
            self.error = Some("That class is not in the current Classroom account or saved copy.".into());
            return Task::none();
        };
        let needs_reload = match target {
            ClassroomLink::Item { item, .. } => snapshot.coursework_for(course)
                .is_some_and(|items| !items.iter().any(|entry| entry.id.parse::<i64>().ok() == Some(item))),
            ClassroomLink::Topic { topic, .. } => snapshot.coursework_for(course)
                .is_some_and(|items| !items.iter().any(|entry| entry.topic_id.as_deref() == Some(&topic.to_string())))
                && snapshot.topics_for(course).is_none_or(|topics| !topics.iter().any(|entry| entry.id == topic.to_string())),
            _ => false,
        };
        if !matches!(target, ClassroomLink::YourWork { .. }) {
            self.classroom_link_input = None;
        }
        self.error = None;
        self.notice = None;
        match target {
            ClassroomLink::YourWork { .. } => {
                let Some(session) = self.session.clone().filter(|_| self.can_read_pages()) else {
                    self.error = Some("Connect to Classroom to verify this Your work link.".into());
                    return Task::none();
                };
                self.invalidate_classroom_link_check();
                self.checking_classroom_link = true;
                let generation = self.classroom_link_generation;
                Task::perform(async move { desktop_auth::own_work_link(&session, course).await },
                    move |result| Message::Navigation(NavigationMessage::ClassroomWorkLinkChecked(generation, target, result)))
            }
            ClassroomLink::Course { classwork, .. } => {
                let navigate = self.update(Message::Navigation(NavigationMessage::Navigate(Screen::Course(course))));
                Task::batch([navigate, self.update(Message::Navigation(NavigationMessage::SetTab(if classwork { Tab::Classwork } else { Tab::Stream })))])
            }
            ClassroomLink::Topic { topic, .. } => {
                let navigate = self.update(Message::Navigation(NavigationMessage::Navigate(Screen::Course(course))));
                let tab = self.update(Message::Navigation(NavigationMessage::SetTab(Tab::Classwork)));
                let topic = self.update(Message::Navigation(NavigationMessage::SelectTopic(Some(topic.to_string()))));
                let refresh = if needs_reload && self.can_read_pages() {
                    self.update(Message::Reads(ReadsMessage::LoadCoursework(course)))
                } else {
                    Task::none()
                };
                Task::batch([navigate, tab, topic, refresh])
            }
            ClassroomLink::Item { item, kind, .. } => {
                self.tab = if kind == ItemLinkKind::Announcement { Tab::Stream } else { Tab::Classwork };
                let open = self.update(Message::Navigation(NavigationMessage::OpenItem(course, item.to_string())));
                self.previous_screen = Some(Screen::Course(course));
                let refresh = if needs_reload && self.can_read_pages() {
                    self.update(Message::Reads(ReadsMessage::LoadCoursework(course)))
                } else {
                    Task::none()
                };
                Task::batch([open, refresh])
            }
        }
    }

    fn header(&self) -> Element<'_, Message> {
        let toggle = button(icon(Icon::Menu, MUTED, 21))
            .padding(12)
            .style(quiet_style)
            .on_press(Message::Navigation(NavigationMessage::ToggleNavigation));
        let brand = button(
            row![classroom_mark(), text("Classroom").size(22).style(move |theme| themed_text(theme, MUTED))]
                .spacing(9)
                .align_y(Alignment::Center),
        )
        .padding([7, 9])
        .style(quiet_style)
        .on_press(Message::Navigation(NavigationMessage::Navigate(Screen::Home)));
        let account: Element<'_, Message> = if let Some(session) = &self.session {
            let status: Element<'_, Message> = if session.needs_sign_in() {
                button(if self.busy { "Signing in…" } else { "Reconnect" })
                    .style(outline_style).padding([8, 12])
                    .on_press_maybe((!self.has_pending_request()).then_some(Message::Session(SessionMessage::Reconnect)))
                    .into()
            } else {
                text(if self.snapshot.as_ref().is_some_and(|data| data.source == SnapshotSource::Cache) {
                    "Viewing offline copy"
                } else {
                    "Connected"
                }).size(13).style(move |theme| themed_text(theme, color!(0x137333))).into()
            };
            let mut actions = row![
                status,
                button("Join class").style(quiet_style).padding([8, 12])
                    .on_press_maybe(self.can_change_membership().then_some(Message::Navigation(NavigationMessage::Navigate(Screen::Join)))),
            ];
            if self.can_create_class() {
                actions = actions.push(button("Create class").style(quiet_style).padding([8, 12])
                    .on_press_maybe((!self.has_pending_request()).then_some(Message::Navigation(NavigationMessage::Navigate(Screen::CreateClass)))));
            }
            actions.push(
                button("Sign out").style(quiet_style).padding([8, 12]).on_press_maybe(
                    (!self.has_pending_request()).then_some(Message::Session(SessionMessage::Disconnect))
                )
            ).push(avatar(first_initial(self.email.trim()), color!(0x1b6a54), 34.0, 15.0))
            .spacing(9)
            .align_y(Alignment::Center)
            .into()
        } else if self.demo {
            row![
                pill("Demo data", color!(0xfef7e0), color!(0x6d4e00)),
                button("Exit demo").style(quiet_style).padding([8, 12])
                    .on_press(Message::Session(SessionMessage::Disconnect)),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into()
        } else if self.snapshot.is_some() {
            row![
                pill("Offline copy", SOFT, MUTED),
                button("Close").style(quiet_style).padding([8, 12]).on_press(Message::Session(SessionMessage::Disconnect)),
            ].spacing(8).align_y(Alignment::Center).into()
        } else {
            pill("Signed out", SOFT, MUTED)
        };
        let links: Element<'_, Message> = if self.snapshot.is_some() && !self.verify_live_course_count {
            button("Open link…").style(quiet_style).padding([8, 10])
                .on_press(Message::Navigation(NavigationMessage::ShowClassroomLink)).into()
        } else {
            space().into()
        };
        container(
            row![toggle, brand, space().width(Fill), links, account]
                .padding([8, 18])
                .spacing(5)
                .align_y(Alignment::Center),
        )
        .width(Fill)
        .into()
    }

    fn navigation(&self) -> Element<'_, Message> {
        let mut course_entries = column![].spacing(3);
        if let Some(data) = &self.snapshot {
            if data.courses.iter().any(|course| home_course(course)) {
                course_entries = course_entries.push(
                    container(text("Enrolled").size(13).font(medium()).style(move |theme| themed_text(theme, MUTED)))
                        .padding(Padding {
                            top: 10.0, right: 12.0, bottom: 6.0, left: 17.0,
                        })
                );
            }
            for course in data.courses.iter().filter(|course| home_course(course)) {
                let destination = Screen::Course(course.id);
                course_entries = course_entries.push(
                    button(row![
                        avatar(first_initial(&course.title), course_color(course), 26.0, 12.0),
                        text(&course.title).size(14).style(move |theme| themed_text(theme,
                            if self.is_selected_course(course.id) { ACTIVE_FG } else { FG }
                        )).width(Fill),
                    ].spacing(11).align_y(Alignment::Center))
                    .on_press(Message::Navigation(NavigationMessage::Navigate(destination)))
                    .style(move |theme, status| nav_style(theme, self.is_selected_course(course.id), status))
                    .padding([9, 13])
                    .width(Fill)
                );
            }
        }
        let nav = column![
            self.nav_button(Icon::Home, "Home", Screen::Home),
            self.nav_button(Icon::Calendar, "Calendar", Screen::Calendar),
            self.nav_button(Icon::Check, "To-do", Screen::ToDo),
            self.nav_button(Icon::Book, "Archived classes", Screen::Archived),
            container(rule::horizontal(1).style(divider_style))
                .padding([9, 12]),
            course_entries,
            container(rule::horizontal(1).style(divider_style)).padding([9, 12]),
            self.nav_button(Icon::Settings, "Settings", Screen::Settings),
        ]
        .spacing(3)
        .padding([13, 10])
        .width(Fill);
        row![
            container(scrollable(nav)).width(238).height(Fill),
            rule::vertical(1).style(divider_style),
        ]
        .height(Fill)
        .into()
    }

    fn is_selected_course(&self, id: i64) -> bool {
        matches!(self.screen, Screen::Course(course) | Screen::Item(course) if course == id)
    }

    fn nav_button(&self, glyph: Icon, label: &'static str, destination: Screen) -> Element<'_, Message> {
        let archived_course = matches!(self.screen, Screen::Course(id) | Screen::Item(id)
            if self.snapshot.as_ref().is_some_and(|data| data.courses.iter()
                .any(|course| course.id == id && course.state == CourseState::Archived)));
        let active = self.screen == destination || destination == Screen::Archived && archived_course;
        let color = if active { ACTIVE_FG } else { MUTED };
        button(row![icon(glyph, color, 20), text(label).style(move |theme| themed_text(theme, color)).size(15)]
            .spacing(14).align_y(Alignment::Center))
            .style(move |theme, status| nav_style(theme, active, status))
            .width(Fill)
            .padding([11, 14])
            .on_press(Message::Navigation(NavigationMessage::Navigate(destination)))
            .into()
    }
}
