use super::*;

#[derive(Clone)]
pub(in crate::desktop) enum CoursesMessage {
    ToggleHiddenClasses,
    ToggleCourseMenu(i64),
    ChooseCourseMove(i64),
    MoveCourse(i64, CoursePlacement),
    CourseMoved(i64, Result<Vec<Course>, String>),
    BeginCourseDrag(i64),
    EnterCourseDropTarget(i64),
    ExitCourseDropTarget(i64),
    DropCourse,
    CancelCourseDrag,
    ConfirmHideCourse(i64),
    ChangeCourseVisibility(i64, bool),
    CourseVisibilityChanged(i64, bool, Result<(), String>),
    ConfirmClassArchive(i64),
    ChangeClassArchive(i64, bool),
    ClassArchiveChanged(i64, bool, Result<CourseState, String>),
    EditClassCode(String),
    JoinClass,
    LoadInvitations,
    InvitationsLoaded(u64, Result<Vec<Course>, String>),
    AcceptInvitation(i64),
    ConfirmDeclineInvitation(i64),
    CancelDeclineInvitation,
    DeclineInvitation(i64),
    InvitationDeclined(Result<Vec<Course>, String>),
    ConfirmUnenroll(i64),
    CancelUnenroll,
    Unenroll(i64),
    MembershipFinished(Result<MembershipChange, String>),
    ClassCreationPermission(u64, Result<bool, String>),
    EditNewClass(NewClassField, String),
    CreateClass,
}

impl App {
    pub(super) fn update_courses(&mut self, message: CoursesMessage) -> Task<Message> {
        match message {
            CoursesMessage::ToggleHiddenClasses if self.screen == Screen::Home => {
                self.home.show_hidden_classes = !self.home.show_hidden_classes;
                self.home.open_course_menu = None;
                self.home.move_course_menu = None;
                self.home.confirm_hide_course = None;
            }
            CoursesMessage::ToggleCourseMenu(id) if matches!(self.screen, Screen::Home | Screen::Archived) => {
                self.home.open_course_menu = (self.home.open_course_menu != Some(id)).then_some(id);
                self.home.move_course_menu = None;
                self.home.confirm_hide_course = None;
                self.home.confirm_class_archive = None;
            }
            CoursesMessage::ChooseCourseMove(id) if self.home.open_course_menu == Some(id) && self.can_move_course(id) => {
                self.home.move_course_menu = Some(id);
                self.home.confirm_hide_course = None;
            }
            CoursesMessage::MoveCourse(id, placement) if self.can_move_course(id) && !self.has_pending_request() => {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.home.open_course_menu = None;
                self.home.move_course_menu = None;
                self.home.course_drag = None;
                self.home.moving_course = Some(id);
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::reorder_class(&session, id, placement).await },
                    move |result| Message::Courses(CoursesMessage::CourseMoved(id, result)));
            }
            CoursesMessage::CourseMoved(id, result) if self.home.moving_course == Some(id) => {
                self.home.moving_course = None;
                match result {
                    Ok(courses) => {
                        if let Some(snapshot) = &mut self.snapshot {
                            snapshot.courses = courses;
                        }
                        self.home.alphabetical = false;
                        self.notice = Some("Class order saved.".into());
                    }
                    Err(error) => {
                        let refresh = self.update(Message::Session(SessionMessage::Refresh));
                        self.error = Some(error);
                        return refresh;
                    }
                }
            }
            CoursesMessage::BeginCourseDrag(id) if self.can_move_course(id) && !self.home.alphabetical
                && self.home.search.trim().is_empty() && !self.has_pending_request() =>
            {
                self.home.open_course_menu = None;
                self.home.move_course_menu = None;
                self.home.confirm_hide_course = None;
                self.home.course_drag = Some(CourseDrag { source: id, target: None });
            }
            CoursesMessage::EnterCourseDropTarget(id) => {
                if let Some(drag) = &mut self.home.course_drag {
                    drag.target = (id != drag.source).then_some(id);
                }
            }
            CoursesMessage::ExitCourseDropTarget(id) => {
                if let Some(drag) = &mut self.home.course_drag
                    && drag.target == Some(id)
                {
                    drag.target = None;
                }
            }
            CoursesMessage::CancelCourseDrag => self.home.course_drag = None,
            CoursesMessage::DropCourse => {
                if let Some(CourseDrag { source, target: Some(target) }) = self.home.course_drag.take()
                    && let Some(placement) = self.drop_placement(source, target)
                {
                    return self.update(Message::Courses(CoursesMessage::MoveCourse(source, placement)));
                }
            }
            CoursesMessage::ConfirmHideCourse(id) if self.home.open_course_menu == Some(id)
                && self.can_change_course_visibility(id, true) =>
            {
                self.home.confirm_hide_course = Some(id);
            }
            CoursesMessage::ChangeCourseVisibility(id, hidden) if self.home.open_course_menu == Some(id)
                && (!hidden || self.home.confirm_hide_course == Some(id))
                && self.can_change_course_visibility(id, hidden) && !self.has_pending_request() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.home.open_course_menu = None;
                self.home.confirm_hide_course = None;
                self.home.changing_course_visibility = Some((id, hidden));
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::set_class_hidden(&session, id, hidden).await },
                    move |result| Message::Courses(CoursesMessage::CourseVisibilityChanged(id, hidden, result)));
            }
            CoursesMessage::CourseVisibilityChanged(id, hidden, result) if self.home.changing_course_visibility == Some((id, hidden)) => {
                self.home.changing_course_visibility = None;
                match result {
                    Ok(()) => {
                        if let Some(course) = self.snapshot.as_mut().and_then(|snapshot| snapshot.courses.iter_mut().find(|course| course.id == id)) {
                            course.hidden = Some(hidden);
                        }
                        if self.deadlines.todo_class == Some(id) {
                            self.deadlines.todo_class = None;
                        }
                        if self.deadlines.calendar_class == Some(id) {
                            self.deadlines.calendar_class = None;
                        }
                        self.cancel_assignment_batch();
                        self.assignments_completed.remove(&id);
                        if !hidden {
                            self.assignments_loaded = false;
                        }
                        self.queued_page_load |= self.screen.uses_assignment_overview();
                        self.notice = Some(if hidden { "Class hidden." } else { "Class unhidden." }.into());
                    }
                    Err(error) => {
                        let refresh = self.update(Message::Session(SessionMessage::Refresh));
                        self.error = Some(error);
                        return refresh;
                    }
                }
            }
            CoursesMessage::ConfirmClassArchive(id) if self.home.open_course_menu == Some(id)
                && self.can_change_class_archive(id, self.screen == Screen::Home) =>
            {
                self.home.confirm_class_archive = Some(id);
            }
            CoursesMessage::ChangeClassArchive(id, archive) if self.home.open_course_menu == Some(id)
                && self.home.confirm_class_archive == Some(id)
                && self.can_change_class_archive(id, archive) && !self.has_pending_request() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.home.open_course_menu = None;
                self.home.confirm_class_archive = None;
                self.home.changing_class_archive = Some((id, archive));
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::set_class_archived(&session, id, archive).await },
                    move |result| Message::Courses(CoursesMessage::ClassArchiveChanged(id, archive, result)));
            }
            CoursesMessage::ClassArchiveChanged(id, archive, result) if self.home.changing_class_archive == Some((id, archive)) => {
                self.home.changing_class_archive = None;
                match result {
                    Ok(state) => {
                        let session_snapshot = self.session.as_mut().map(|session| &mut Arc::make_mut(session).snapshot);
                        for snapshot in self.snapshot.iter_mut().chain(session_snapshot) {
                            if let Some(course) = snapshot.courses.iter_mut().find(|course| course.id == id) {
                                course.state = state;
                            }
                        }
                        for filter in [&mut self.deadlines.calendar_class, &mut self.deadlines.todo_class] {
                            if *filter == Some(id) {
                                *filter = None;
                            }
                        }
                        self.cancel_assignment_batch();
                        self.assignments_completed.remove(&id);
                        self.assignments_loaded = false;
                        self.queued_page_load |= self.screen.uses_assignment_overview();
                        self.notice = Some(if archive { "Class archived." } else { "Class restored." }.into());
                    }
                    Err(error) => {
                        let refresh = self.update(Message::Session(SessionMessage::Refresh));
                        self.error = Some(error);
                        return refresh;
                    }
                }
            }
            CoursesMessage::LoadInvitations
                if self.can_change_membership() && !self.loading_invitations && !self.changing_membership =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.loading_invitations = true;
                self.page_errors.remove(&PageRead::Invitations);
                let generation = self.invitation_generation;
                return Task::perform(async move { desktop_auth::load_invitations(&session).await },
                    move |result| Message::Courses(CoursesMessage::InvitationsLoaded(generation, result)));
            }
            CoursesMessage::InvitationsLoaded(generation, result) if generation == self.invitation_generation => {
                self.loading_invitations = false;
                self.queued_page_load |= self.screen == Screen::Home;
                match result {
                    Ok(rows) => {
                        if std::env::var_os("CLASSROOM_READ_ONLY_DIAGNOSTICS").is_some() {
                            println!("Class invitations ready; count={}", rows.len());
                            let _ = io::stdout().flush();
                        }
                        self.invitations = Some(rows);
                    }
                    Err(error) => {
                        self.page_errors.insert(PageRead::Invitations, error);
                    }
                }
            }
            CoursesMessage::AcceptInvitation(course) if self.can_respond_to_invitation(course) => {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.confirming_decline = None;
                self.changing_membership = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::accept_invitation(&session, course).await },
                    move |result| Message::Courses(CoursesMessage::MembershipFinished(result.map(|()| MembershipChange::Joined(course)))));
            }
            CoursesMessage::ConfirmDeclineInvitation(course) if self.can_respond_to_invitation(course) => {
                self.confirming_decline = Some(course);
            }
            CoursesMessage::CancelDeclineInvitation => self.confirming_decline = None,
            CoursesMessage::DeclineInvitation(course)
                if self.confirming_decline == Some(course) && self.can_respond_to_invitation(course) =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.confirming_decline = None;
                self.changing_membership = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::decline_invitation(&session, course).await },
                    |value| Message::Courses(CoursesMessage::InvitationDeclined(value)));
            }
            CoursesMessage::InvitationDeclined(result) => {
                self.changing_membership = false;
                match result {
                    Ok(remaining) => {
                        self.invalidate_invitation_read();
                        self.invitations = Some(remaining);
                        self.page_errors.remove(&PageRead::Invitations);
                        self.notice = Some("Class invitation declined.".into());
                    }
                    Err(error) => {
                        self.error = Some(error);
                        let invitations = self.update(Message::Courses(CoursesMessage::LoadInvitations));
                        return Task::batch([
                            invitations,
                            self.dequeue_page_load().map_or_else(Task::none, |read| self.update(read)),
                        ]);
                    }
                }
            }
            CoursesMessage::EditClassCode(code) if self.screen == Screen::Join && !self.changing_membership => {
                self.class_code = code;
            }
            CoursesMessage::ClassCreationPermission(generation, result) if generation == self.class_creation_generation => {
                self.class_creation_allowed = result.unwrap_or(false);
            }
            CoursesMessage::EditNewClass(field, value) if self.screen == Screen::CreateClass && !self.changing_membership => {
                let input = match field {
                    NewClassField::Name => &mut self.new_class.name,
                    NewClassField::Section => &mut self.new_class.section,
                    NewClassField::Subject => &mut self.new_class.subject,
                    NewClassField::Room => &mut self.new_class.room,
                };
                *input = value;
            }
            CoursesMessage::CreateClass if self.screen == Screen::CreateClass && self.can_create_class()
                && !self.has_pending_request() && desktop_auth::class_input_valid(&self.new_class) =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                let input = self.new_class.clone();
                self.error = None;
                self.notice = None;
                self.changing_membership = true;
                return Task::perform(async move { desktop_auth::create_class(&session, input).await },
                    |result| Message::Courses(CoursesMessage::MembershipFinished(result.map(MembershipChange::Created))));
            }
            CoursesMessage::JoinClass
                if self.screen == Screen::Join && self.can_change_membership()
                    && !self.has_pending_request() && desktop_auth::valid_class_code(&self.class_code) =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                let code = self.class_code.clone();
                self.error = None;
                self.notice = None;
                self.changing_membership = true;
                return Task::perform(async move { desktop_auth::join_class(&session, code).await },
                    |result| Message::Courses(CoursesMessage::MembershipFinished(result.map(MembershipChange::Joined))));
            }
            CoursesMessage::ConfirmUnenroll(course) if self.screen == Screen::Course(course) && self.can_change_membership() => {
                self.confirming_unenroll = Some(course);
            }
            CoursesMessage::CancelUnenroll => self.confirming_unenroll = None,
            CoursesMessage::Unenroll(course)
                if self.screen == Screen::Course(course) && self.confirming_unenroll == Some(course)
                    && self.can_change_membership() && !self.has_pending_request() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.confirming_unenroll = None;
                self.error = None;
                self.notice = None;
                self.changing_membership = true;
                return Task::perform(async move { desktop_auth::leave_class(&session, course).await },
                    move |result| Message::Courses(CoursesMessage::MembershipFinished(result.map(|()| MembershipChange::Left(course)))));
            }
            CoursesMessage::MembershipFinished(result) => {
                self.changing_membership = false;
                match result {
                    Ok(change) => {
                        self.invalidate_invitation_read();
                        self.class_code.clear();
                        self.screen = Screen::Home;
                        let message = match change {
                            MembershipChange::Created(id) => {
                                self.opening_joined_course = Some(id);
                                self.new_class = desktop_auth::NewClass::default();
                                "Class created."
                            }
                            MembershipChange::Joined(id) => {
                                self.opening_joined_course = Some(id);
                                if let Some(invitations) = &mut self.invitations {
                                    invitations.retain(|course| course.id != id);
                                }
                                "Class joined."
                            }
                            MembershipChange::Left(id) => {
                                if let Some(snapshot) = &mut self.snapshot {
                                    snapshot.courses.retain(|item| item.id != id);
                                    snapshot.coursework.remove(&id);
                                    snapshot.topics.remove(&id);
                                    snapshot.roster.remove(&id);
                                    self.student_work.submissions_checked.remove(&id);
                                }
                                "Unenrolled from class."
                            }
                        };
                        let refresh = self.update(Message::Session(SessionMessage::Refresh));
                        self.notice = Some(message.into());
                        return refresh;
                    }
                    Err(error) => {
                        let invitations = self.update(Message::Courses(CoursesMessage::LoadInvitations));
                        self.error = Some(error);
                        return invitations;
                    }
                }
            }
            _ => {}
        }
        self.finish_update()
    }
}
