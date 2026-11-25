use super::*;
use iced::widget::column;

impl App {
    pub(super) fn can_change_student_mute(&self, course: i64, student: i64, current: bool) -> bool {
        self.tab == Tab::People && self.can_edit_class_stream_settings(course)
            && self.people.student_muting.get(&course).and_then(|states| states.get(&student)) == Some(&current)
            && self.snapshot.as_ref().and_then(|snapshot| snapshot.roster_for(course)).is_some_and(|members|
                members.iter().any(|member| member.role == CourseRole::Student && member.id.parse::<i64>().ok() == Some(student)))
    }

    pub(super) fn invalidate_people_email(&mut self) {
        self.people.people_email_generation = self.people.people_email_generation.wrapping_add(1);
        self.people.people_email_pending = None;
    }

    pub(super) fn can_email_class_member(&self, course: i64, member: i64) -> bool {
        self.can_read_pages() && self.screen == Screen::Course(course) && self.tab == Tab::People
            && self.snapshot.as_ref().and_then(|snapshot| snapshot.roster_for(course))
                .is_some_and(|members| members.iter().any(|entry| entry.id.parse::<i64>().ok() == Some(member)))
    }

    pub(super) fn people(&self, id: i64) -> Element<'_, Message> {
        let roster = self.snapshot.as_ref().and_then(|snapshot| snapshot.roster_for(id));
        let mut body = column![].spacing(14);
        if let Some(status) = self.page_read_status(
            PageRead::Detail(DetailRead::Roster(id)), roster.is_some(), "Loading people…",
        ) {
            body = body.push(status);
        }
        let teacher = self.class_roles.get(&id) == Some(&CourseRole::Teacher);
        let moderator = teacher && (self.can_edit_class_stream_settings(id) || self.screenshot_directory.is_some());
        if moderator && roster.is_some()
            && let Some(status) = self.page_read_status(
                PageRead::Detail(DetailRead::StudentMuting(id)), self.people.student_muting.contains_key(&id), "Loading student Stream permissions…",
            )
        {
            body = body.push(status);
        }
        if let Some(members) = roster.filter(|members| !members.is_empty()) {
            for (role, label) in [
                (CourseRole::Teacher, "Teachers"),
                (CourseRole::Student, if teacher { "Students" } else { "Classmates" }),
                (CourseRole::Unknown, "Other members"),
            ] {
                let mut matching = members.iter().filter(|member| member.role == role).peekable();
                if matching.peek().is_none() {
                    continue;
                }
                body = body.push(text(label).font(medium()).size(18).style(move |theme| themed_text(theme, FG)));
                let mut group = column![].spacing(0);
                for member in matching {
                    let mut line = row![
                        avatar(first_initial(&member.display_name), BLUE, 36.0, 15.0),
                        text(&member.display_name).size(15).style(move |theme| themed_text(theme, FG)),
                        space().width(Fill),
                        text(member.role.label()).size(13).style(move |theme| themed_text(theme, MUTED)),
                    ].spacing(12).align_y(Alignment::Center).padding(12);
                    if let Ok(person) = member.id.parse::<i64>()
                        && self.can_email_class_member(id, person)
                    {
                        let pending = self.people.people_email_pending == Some((id, person));
                        line = line.push(button(if pending { "Opening…" } else { "Email" })
                            .padding([7, 10]).style(quiet_style)
                            .on_press_maybe((self.people.people_email_pending.is_none() && !self.has_pending_request())
                                .then_some(Message::Comments(CommentsMessage::EmailClassMember(id, person)))));
                    }
                    if moderator && member.role == CourseRole::Student
                        && let Ok(student) = member.id.parse::<i64>()
                        && let Some(&muted) = self.people.student_muting.get(&id).and_then(|rows| rows.get(&student))
                    {
                        if muted {
                            line = line.push(text("Muted").size(13).style(move |theme| themed_text(theme, MUTED)));
                        }
                        if self.people.pending_student_mute == Some((id, student, muted)) {
                            group = group.push(line).push(column![
                                text(if muted { "Restore this student’s Stream access?" } else { "Mute this student in the Stream?" })
                                    .size(14).style(move |theme| themed_text(theme, FG)),
                                text("The class posting or commenting setting still applies. Private feedback stays available.")
                                    .size(13).style(move |theme| themed_text(theme, MUTED)),
                                row![
                                    button(if muted { "Unmute" } else { "Mute" }).padding([7, 10]).style(primary_style)
                                        .on_press_maybe((!self.has_pending_request()).then_some(Message::Teaching(TeachingMessage::ChangeStudentMute(id, student, muted)))),
                                    button("Cancel").padding([7, 10]).style(quiet_style).on_press(Message::Teaching(TeachingMessage::CancelStudentMute)),
                                ].spacing(8),
                            ].spacing(8).padding(Padding { top: 0.0, left: 12.0, bottom: 12.0, right: 12.0 }));
                            continue;
                        }
                        line = line.push(button(if muted { "Unmute…" } else { "Mute…" }).padding([7, 10]).style(outline_style)
                            .on_press_maybe((!self.has_pending_request()).then_some(Message::Teaching(TeachingMessage::ConfirmStudentMute(id, student, muted)))));
                    }
                    group = group.push(line);
                }
                body = body.push(container(group).style(|theme| outline_surface(theme, 9.0)).width(Fill));
            }
        } else if roster.is_some() {
            body = body.push(empty_panel(Icon::People, "No people returned", "No roster entries were available to this account."));
        } else if !self.can_read_pages() {
            body = body.push(empty_panel(
                Icon::People, "Roster unavailable", "This copy has no roster data.",
            ));
        }
        body.width(Fill).into()
    }
}

pub(super) fn class_member_email_url(members: &[desktop_auth::ClassMention], member: i64) -> Option<String> {
    let mut matching = members.iter().filter(|entry| entry.id == member);
    let email = &matching.next()?.email;
    if matching.next().is_some() { return None; }
    let (local, domain) = email.split_once('@')?;
    if local.is_empty() || domain.is_empty()
        || !domain.bytes().all(|byte| byte.is_ascii_alphanumeric() || b".-".contains(&byte))
    {
        return None;
    }
    let encoded: String = form_urlencoded::byte_serialize(local.as_bytes()).collect();
    Some(format!("mailto:{encoded}@{domain}"))
}

#[derive(Default)]
pub(super) struct PeopleState {
    pub(super) people_email_generation: u64,
    pub(super) people_email_pending: Option<(i64, i64)>,
    pub(super) student_muting: BTreeMap<i64, BTreeMap<i64, bool>>,
    pub(super) pending_student_mute: Option<(i64, i64, bool)>,
    pub(super) changing_student_mute: bool,
}
