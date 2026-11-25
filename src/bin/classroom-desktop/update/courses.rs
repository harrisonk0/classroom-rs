use super::*;

impl App {
    pub(super) fn update_courses(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ToggleHiddenClasses if self.screen == Screen::Home => {
                self.show_hidden_classes = !self.show_hidden_classes;
                self.open_course_menu = None;
                self.move_course_menu = None;
                self.confirm_hide_course = None;
            }
            Message::ToggleCourseMenu(id) if matches!(self.screen, Screen::Home | Screen::Archived) => {
                self.open_course_menu = (self.open_course_menu != Some(id)).then_some(id);
                self.move_course_menu = None;
                self.confirm_hide_course = None;
                self.confirm_class_archive = None;
            }
            Message::ChooseCourseMove(id) if self.open_course_menu == Some(id) && self.can_move_course(id) => {
                self.move_course_menu = Some(id);
                self.confirm_hide_course = None;
            }
            Message::MoveCourse(id, placement) if self.can_move_course(id) && !self.has_pending_request() => {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.open_course_menu = None;
                self.move_course_menu = None;
                self.course_drag = None;
                self.moving_course = Some(id);
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::reorder_class(&session, id, placement).await },
                    move |result| Message::CourseMoved(id, result));
            }
            Message::CourseMoved(id, result) if self.moving_course == Some(id) => {
                self.moving_course = None;
                match result {
                    Ok(courses) => {
                        if let Some(snapshot) = &mut self.snapshot {
                            snapshot.courses = courses;
                        }
                        self.alphabetical = false;
                        self.notice = Some("Class order saved.".into());
                    }
                    Err(error) => {
                        let refresh = self.update(Message::Refresh);
                        self.error = Some(error);
                        return refresh;
                    }
                }
            }
            Message::BeginCourseDrag(id) if self.can_move_course(id) && !self.alphabetical
                && self.search.trim().is_empty() && !self.has_pending_request() =>
            {
                self.open_course_menu = None;
                self.move_course_menu = None;
                self.confirm_hide_course = None;
                self.course_drag = Some(CourseDrag { source: id, target: None });
            }
            Message::EnterCourseDropTarget(id) => {
                if let Some(drag) = &mut self.course_drag {
                    drag.target = (id != drag.source).then_some(id);
                }
            }
            Message::ExitCourseDropTarget(id) => {
                if let Some(drag) = &mut self.course_drag
                    && drag.target == Some(id)
                {
                    drag.target = None;
                }
            }
            Message::CancelCourseDrag => self.course_drag = None,
            Message::DropCourse => {
                if let Some(CourseDrag { source, target: Some(target) }) = self.course_drag.take()
                    && let Some(placement) = self.drop_placement(source, target)
                {
                    return self.update(Message::MoveCourse(source, placement));
                }
            }
            Message::ConfirmHideCourse(id) if self.open_course_menu == Some(id)
                && self.can_change_course_visibility(id, true) =>
            {
                self.confirm_hide_course = Some(id);
            }
            Message::ChangeCourseVisibility(id, hidden) if self.open_course_menu == Some(id)
                && (!hidden || self.confirm_hide_course == Some(id))
                && self.can_change_course_visibility(id, hidden) && !self.has_pending_request() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.open_course_menu = None;
                self.confirm_hide_course = None;
                self.changing_course_visibility = Some((id, hidden));
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::set_class_hidden(&session, id, hidden).await },
                    move |result| Message::CourseVisibilityChanged(id, hidden, result));
            }
            Message::CourseVisibilityChanged(id, hidden, result) if self.changing_course_visibility == Some((id, hidden)) => {
                self.changing_course_visibility = None;
                match result {
                    Ok(()) => {
                        if let Some(course) = self.snapshot.as_mut().and_then(|snapshot| snapshot.courses.iter_mut().find(|course| course.id == id)) {
                            course.hidden = Some(hidden);
                        }
                        if self.todo_class == Some(id) {
                            self.todo_class = None;
                        }
                        if self.calendar_class == Some(id) {
                            self.calendar_class = None;
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
                        let refresh = self.update(Message::Refresh);
                        self.error = Some(error);
                        return refresh;
                    }
                }
            }
            Message::ConfirmClassArchive(id) if self.open_course_menu == Some(id)
                && self.can_change_class_archive(id, self.screen == Screen::Home) =>
            {
                self.confirm_class_archive = Some(id);
            }
            Message::ChangeClassArchive(id, archive) if self.open_course_menu == Some(id)
                && self.confirm_class_archive == Some(id)
                && self.can_change_class_archive(id, archive) && !self.has_pending_request() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.open_course_menu = None;
                self.confirm_class_archive = None;
                self.changing_class_archive = Some((id, archive));
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::set_class_archived(&session, id, archive).await },
                    move |result| Message::ClassArchiveChanged(id, archive, result));
            }
            Message::ClassArchiveChanged(id, archive, result) if self.changing_class_archive == Some((id, archive)) => {
                self.changing_class_archive = None;
                match result {
                    Ok(state) => {
                        let session_snapshot = self.session.as_mut().map(|session| &mut Arc::make_mut(session).snapshot);
                        for snapshot in self.snapshot.iter_mut().chain(session_snapshot) {
                            if let Some(course) = snapshot.courses.iter_mut().find(|course| course.id == id) {
                                course.state = state;
                            }
                        }
                        for filter in [&mut self.calendar_class, &mut self.todo_class] {
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
                        let refresh = self.update(Message::Refresh);
                        self.error = Some(error);
                        return refresh;
                    }
                }
            }
            Message::LoadInvitations
                if self.can_change_membership() && !self.loading_invitations && !self.changing_membership =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.loading_invitations = true;
                self.page_errors.remove(&PageRead::Invitations);
                let generation = self.invitation_generation;
                return Task::perform(async move { desktop_auth::load_invitations(&session).await },
                    move |result| Message::InvitationsLoaded(generation, result));
            }
            Message::InvitationsLoaded(generation, result) if generation == self.invitation_generation => {
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
            Message::AcceptInvitation(course) if self.can_respond_to_invitation(course) => {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.confirming_decline = None;
                self.changing_membership = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::accept_invitation(&session, course).await },
                    move |result| Message::MembershipFinished(result.map(|()| MembershipChange::Joined(course))));
            }
            Message::ConfirmDeclineInvitation(course) if self.can_respond_to_invitation(course) => {
                self.confirming_decline = Some(course);
            }
            Message::CancelDeclineInvitation => self.confirming_decline = None,
            Message::DeclineInvitation(course)
                if self.confirming_decline == Some(course) && self.can_respond_to_invitation(course) =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.confirming_decline = None;
                self.changing_membership = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::decline_invitation(&session, course).await },
                    Message::InvitationDeclined);
            }
            Message::InvitationDeclined(result) => {
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
                        let invitations = self.update(Message::LoadInvitations);
                        return Task::batch([
                            invitations,
                            self.dequeue_page_load().map_or_else(Task::none, |read| self.update(read)),
                        ]);
                    }
                }
            }
            Message::EditClassCode(code) if self.screen == Screen::Join && !self.changing_membership => {
                self.class_code = code;
            }
            Message::ClassCreationPermission(generation, result) if generation == self.class_creation_generation => {
                self.class_creation_allowed = result.unwrap_or(false);
            }
            Message::EditNewClass(field, value) if self.screen == Screen::CreateClass && !self.changing_membership => {
                let input = match field {
                    NewClassField::Name => &mut self.new_class.name,
                    NewClassField::Section => &mut self.new_class.section,
                    NewClassField::Subject => &mut self.new_class.subject,
                    NewClassField::Room => &mut self.new_class.room,
                };
                *input = value;
            }
            Message::CreateClass if self.screen == Screen::CreateClass && self.can_create_class()
                && !self.has_pending_request() && desktop_auth::class_input_valid(&self.new_class) =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                let input = self.new_class.clone();
                self.error = None;
                self.notice = None;
                self.changing_membership = true;
                return Task::perform(async move { desktop_auth::create_class(&session, input).await },
                    |result| Message::MembershipFinished(result.map(MembershipChange::Created)));
            }
            Message::JoinClass
                if self.screen == Screen::Join && self.can_change_membership()
                    && !self.has_pending_request() && desktop_auth::valid_class_code(&self.class_code) =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                let code = self.class_code.clone();
                self.error = None;
                self.notice = None;
                self.changing_membership = true;
                return Task::perform(async move { desktop_auth::join_class(&session, code).await },
                    |result| Message::MembershipFinished(result.map(MembershipChange::Joined)));
            }
            Message::ConfirmUnenroll(course) if self.screen == Screen::Course(course) && self.can_change_membership() => {
                self.confirming_unenroll = Some(course);
            }
            Message::CancelUnenroll => self.confirming_unenroll = None,
            Message::Unenroll(course)
                if self.screen == Screen::Course(course) && self.confirming_unenroll == Some(course)
                    && self.can_change_membership() && !self.has_pending_request() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.confirming_unenroll = None;
                self.error = None;
                self.notice = None;
                self.changing_membership = true;
                return Task::perform(async move { desktop_auth::leave_class(&session, course).await },
                    move |result| Message::MembershipFinished(result.map(|()| MembershipChange::Left(course))));
            }
            Message::MembershipFinished(result) => {
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
                                    self.submissions_checked.remove(&id);
                                }
                                "Unenrolled from class."
                            }
                        };
                        let refresh = self.update(Message::Refresh);
                        self.notice = Some(message.into());
                        return refresh;
                    }
                    Err(error) => {
                        let invitations = self.update(Message::LoadInvitations);
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
