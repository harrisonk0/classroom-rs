use super::*;

#[derive(Clone)]
pub(in crate::desktop) enum SessionMessage {
    Email(String),
    SignIn,
    SignedIn(Result<Arc<LiveSession>, String>),
    Reconnect,
    Reconnected(Result<(), String>),
    Refresh,
    Refreshed(Result<Arc<Snapshot>, String>),
    Demo,
    Disconnect,
    SessionForgotten(Result<(), String>),
}

impl App {
    pub(super) fn update_session(&mut self, message: SessionMessage) -> Task<Message> {
        match message {
            SessionMessage::Email(email) => self.email = email,
            SessionMessage::SignIn if self.can_sign_in() => {
                self.verification_submitted = self.verify_live_course_count;
                self.busy = true;
                self.error = None;
                self.notice = None;
                let email = self.email.trim().to_owned();
                let verify_only = self.verify_live_course_count;
                return Task::perform(async move {
                    if verify_only {
                        desktop_auth::sign_in_basic(email).await
                    } else {
                        desktop_auth::sign_in(email).await
                    }
                }, |result| Message::Session(SessionMessage::SignedIn(result.map(Arc::new))));
            }
            SessionMessage::SignedIn(result) => {
                self.busy = false;
                match result {
                    Ok(session) => {
                        if self.verify_live_course_count {
                            println!("Desktop live QueryCourse OK; class count={}", session.snapshot.courses.len());
                            let _ = io::stdout().flush();
                        }
                        self.student_work.submissions_checked.clear();
                        self.page_errors.clear();
                        if std::env::var_os("CLASSROOM_READ_ONLY_DIAGNOSTICS").is_some() {
                            println!("Native session ready; courses={}", session.snapshot.courses.len());
                            let _ = io::stdout().flush();
                        }
                        self.install_snapshot(session.snapshot.clone());
                        self.session = Some(session);
                        self.demo = false;
                        self.screen = Screen::Home;
                        self.home.search.clear();
                        self.reset_assignment_reads();
                        self.reset_artwork();
                        let folders = self.fetch_class_folders();
                        let teachers = self.fetch_class_teachers();
                        let artwork = self.fetch_artwork();
                        let invitations = self.update(Message::Courses(CoursesMessage::LoadInvitations));
                        return Task::batch([
                            artwork, folders, teachers, invitations, self.fetch_class_creation_permission(),
                            self.update(Message::Reads(ReadsMessage::LoadAssignments)),
                        ]);
                    }
                    Err(error) => {
                        if self.verify_live_course_count {
                            println!("Desktop live QueryCourse did not complete");
                            let _ = io::stdout().flush();
                        }
                        self.error = Some(error);
                    }
                }
            }
            SessionMessage::Reconnect if !self.has_pending_request() => {
                if let Some(session) = self.session.clone().filter(|session| session.needs_sign_in()) {
                    self.cancel_assignment_batch();
                    self.busy = true;
                    self.error = None;
                    return Task::perform(async move { session.reconnect().await }, |value| Message::Session(SessionMessage::Reconnected(value)));
                }
            }
            SessionMessage::Reconnected(result) => {
                self.busy = false;
                match result {
                    Ok(()) => {
                        self.reset_personal_copy();
                        self.student_work.submissions_checked.clear();
                        self.page_errors.clear();
                        self.reset_notifications();
                        self.reset_stream_access();
                        self.reset_assignment_reads();
                        self.queued_page_load = true;
                        self.invalidate_invitation_read();
                        let background = Task::batch([
                            self.fetch_class_folders(), self.fetch_class_teachers(), self.update(Message::Courses(CoursesMessage::LoadInvitations)),
                            self.fetch_class_creation_permission(),
                        ]);
                        return match self.dequeue_page_load() {
                            Some(request) => Task::batch([background, self.update(request)]),
                            None => background,
                        };
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            SessionMessage::Refresh if !self.verify_live_course_count && !self.has_pending_request() => {
                if let Some(session) = self.session.clone() {
                    self.refreshing = true;
                    self.error = None;
                    return Task::perform(
                        async move { desktop_auth::refresh(&session).await.map(Arc::new) },
                        |value| Message::Session(SessionMessage::Refreshed(value)),
                    );
                }
            }
            SessionMessage::Refreshed(result) => {
                self.refreshing = false;
                match result {
                    Ok(snapshot) => {
                        let mut snapshot = Arc::unwrap_or_clone(snapshot);
                        if let Some(previous) = &self.snapshot {
                            for course in &snapshot.courses {
                                if let Some(items) = previous.coursework_for(course.id) {
                                    snapshot.coursework.insert(course.id, items.to_vec());
                                }
                                if let Some(topics) = previous.topics_for(course.id) {
                                    snapshot.topics.insert(course.id, topics.to_vec());
                                }
                                if let Some(roster) = previous.roster_for(course.id) {
                                    snapshot.roster.insert(course.id, roster.to_vec());
                                }
                            }
                        }
                        self.install_snapshot(snapshot);
                        self.student_work.rubrics.clear();
                        self.student_work.overall_grades.clear();
                        self.page_errors.retain(|read, _| !matches!(read, PageRead::Detail(DetailRead::Rubric(..))));
                        self.reset_assignment_reads();
                        self.retry_visible_reads_on_entry();
                        self.queued_page_load = true;
                        let background = Task::batch([
                            self.fetch_artwork(), self.fetch_class_folders(), self.fetch_class_teachers(), self.update(Message::Courses(CoursesMessage::LoadInvitations)),
                            self.fetch_class_creation_permission(),
                        ]);
                        if self.screen == Screen::Home
                            && let Some(course) = self.opening_joined_course
                            && self.snapshot.as_ref().is_some_and(|data| data.courses.iter().any(|row| row.id == course))
                        {
                            self.opening_joined_course = None;
                            return Task::batch([background, self.update(Message::Navigation(NavigationMessage::Navigate(Screen::Course(course))))]);
                        }
                        return match self.dequeue_page_load() {
                            Some(request) => Task::batch([background, self.update(request)]),
                            None => background,
                        };
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            SessionMessage::Demo if !self.has_pending_request() => {
                self.install_snapshot(Snapshot::demo());
                self.reset_artwork();
                self.session = None;
                self.attachments.opening_drive_file = None;
                self.class_creation_generation = self.class_creation_generation.wrapping_add(1);
                self.class_creation_allowed = false;
                self.new_class = desktop_auth::NewClass::default();
                self.demo = true;
                self.screen = Screen::Home;
                self.home.search.clear();
                self.error = None;
            }
            SessionMessage::Disconnect if !self.has_pending_request() && self.session.is_some() => {
                self.discard_submission_capture();
                self.cancel_assignment_batch();
                let account = self.session.as_ref().unwrap().account.clone();
                self.busy = true;
                return Task::perform(desktop_auth::forget_session(account), |value| Message::Session(SessionMessage::SessionForgotten(value)));
            }
            SessionMessage::SessionForgotten(Err(error)) => {
                self.busy = false;
                self.queued_page_load = true;
                self.error = Some(error);
            }
            SessionMessage::SessionForgotten(Ok(())) | SessionMessage::Disconnect => {
                self.busy = false;
                self.reset_artwork();
                self.session = None;
                self.snapshot = None;
                self.reset_personal_copy();
                self.classroom_link_input = None;
                self.invalidate_classroom_link_check();
                self.home.show_hidden_classes = false;
                self.home.open_course_menu = None;
                self.home.move_course_menu = None;
                self.home.course_drag = None;
                self.home.confirm_hide_course = None;
                self.home.confirm_class_archive = None;
                self.reset_class_folders();
                self.teachers_generation = self.teachers_generation.wrapping_add(1);
                self.class_teachers.clear();
                self.reset_notifications();
                self.reset_stream_access();
                self.stream.confirming_stream_delete = None;
                self.stream.open_stream_composer = None;
                self.stream.stream_drafts.clear();
                self.attachments.drive_picker = None;
                self.reset_assignment_reads();
                self.student_work.submissions_checked.clear();
                self.page_errors.clear();
                self.class_code.clear();
                self.invalidate_invitation_read();
                self.invitations = None;
                self.opening_joined_course = None;
                self.confirming_decline = None;
                self.confirming_unenroll = None;
                self.comments.comment_drafts.clear();
                self.invalidate_people_email();
                self.comments.comment_mentions_generation = self.comments.comment_mentions_generation.wrapping_add(1);
                self.comments.comment_mentions = None;
                self.student_work.short_answer_drafts.clear();
                self.student_work.short_answer_revisions.clear();
                self.student_work.choice_drafts.clear();
                self.student_work.confirming_question_answer = None;
                self.comments.comment_unconfirmed.clear();
                self.comments.private_feedback.clear();
                self.student_work.rubrics.clear();
                self.student_work.overall_grades.clear();
                self.student_work.rubric_open = false;
                self.student_work.open_criteria.clear();
                self.comments.private_drafts.clear();
                self.comments.confirming_comment_delete = None;
                self.comments.private_unconfirmed.clear();
                self.attachments.pending_remove_material = None;
                self.student_work.pending_submission = None;
                self.item_id = None;
                self.attachments.attachment_link.clear();
                self.attachments.submission_file = None;
                self.demo = false;
                self.screen = Screen::Home;
                self.home.search.clear();
                self.email.clear();
                self.error = None;
                self.notice = None;
            }
            _ => {}
        }
        self.finish_update()
    }
}
