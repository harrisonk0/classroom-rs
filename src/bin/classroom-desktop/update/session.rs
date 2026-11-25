use super::*;

impl App {
    pub(super) fn update_session(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Email(email) => self.email = email,
            Message::SignIn if self.can_sign_in() => {
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
                }, |result| Message::SignedIn(result.map(Arc::new)));
            }
            Message::SignedIn(result) => {
                self.busy = false;
                match result {
                    Ok(session) => {
                        if self.verify_live_course_count {
                            println!("Desktop live QueryCourse OK; class count={}", session.snapshot.courses.len());
                            let _ = io::stdout().flush();
                        }
                        self.submissions_checked.clear();
                        self.page_errors.clear();
                        if std::env::var_os("CLASSROOM_READ_ONLY_DIAGNOSTICS").is_some() {
                            println!("Native session ready; courses={}", session.snapshot.courses.len());
                            let _ = io::stdout().flush();
                        }
                        self.install_snapshot(session.snapshot.clone());
                        self.session = Some(session);
                        self.demo = false;
                        self.screen = Screen::Home;
                        self.search.clear();
                        self.reset_assignment_reads();
                        self.reset_artwork();
                        let folders = self.fetch_class_folders();
                        let teachers = self.fetch_class_teachers();
                        let artwork = self.fetch_artwork();
                        let invitations = self.update(Message::LoadInvitations);
                        return Task::batch([
                            artwork, folders, teachers, invitations, self.fetch_class_creation_permission(),
                            self.update(Message::LoadAssignments),
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
            Message::Reconnect if !self.has_pending_request() => {
                if let Some(session) = self.session.clone().filter(|session| session.needs_sign_in()) {
                    self.cancel_assignment_batch();
                    self.busy = true;
                    self.error = None;
                    return Task::perform(async move { session.reconnect().await }, Message::Reconnected);
                }
            }
            Message::Reconnected(result) => {
                self.busy = false;
                match result {
                    Ok(()) => {
                        self.reset_personal_copy();
                        self.submissions_checked.clear();
                        self.page_errors.clear();
                        self.reset_notifications();
                        self.reset_stream_access();
                        self.reset_assignment_reads();
                        self.queued_page_load = true;
                        self.invalidate_invitation_read();
                        let background = Task::batch([
                            self.fetch_class_folders(), self.fetch_class_teachers(), self.update(Message::LoadInvitations),
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
            Message::Refresh if !self.verify_live_course_count && !self.has_pending_request() => {
                if let Some(session) = self.session.clone() {
                    self.refreshing = true;
                    self.error = None;
                    return Task::perform(
                        async move { desktop_auth::refresh(&session).await.map(Arc::new) },
                        Message::Refreshed,
                    );
                }
            }
            Message::Refreshed(result) => {
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
                        self.rubrics.clear();
                        self.overall_grades.clear();
                        self.page_errors.retain(|read, _| !matches!(read, PageRead::Detail(DetailRead::Rubric(..))));
                        self.reset_assignment_reads();
                        self.retry_visible_reads_on_entry();
                        self.queued_page_load = true;
                        let background = Task::batch([
                            self.fetch_artwork(), self.fetch_class_folders(), self.fetch_class_teachers(), self.update(Message::LoadInvitations),
                            self.fetch_class_creation_permission(),
                        ]);
                        if self.screen == Screen::Home
                            && let Some(course) = self.opening_joined_course
                            && self.snapshot.as_ref().is_some_and(|data| data.courses.iter().any(|row| row.id == course))
                        {
                            self.opening_joined_course = None;
                            return Task::batch([background, self.update(Message::Navigate(Screen::Course(course)))]);
                        }
                        return match self.dequeue_page_load() {
                            Some(request) => Task::batch([background, self.update(request)]),
                            None => background,
                        };
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Message::Demo if !self.has_pending_request() => {
                self.install_snapshot(Snapshot::demo());
                self.reset_artwork();
                self.session = None;
                self.opening_drive_file = None;
                self.class_creation_generation = self.class_creation_generation.wrapping_add(1);
                self.class_creation_allowed = false;
                self.new_class = desktop_auth::NewClass::default();
                self.demo = true;
                self.screen = Screen::Home;
                self.search.clear();
                self.error = None;
            }
            Message::Disconnect if !self.has_pending_request() && self.session.is_some() => {
                self.discard_submission_capture();
                self.cancel_assignment_batch();
                let account = self.session.as_ref().unwrap().account.clone();
                self.busy = true;
                return Task::perform(desktop_auth::forget_session(account), Message::SessionForgotten);
            }
            Message::SessionForgotten(Err(error)) => {
                self.busy = false;
                self.queued_page_load = true;
                self.error = Some(error);
            }
            Message::SessionForgotten(Ok(())) | Message::Disconnect => {
                self.busy = false;
                self.reset_artwork();
                self.session = None;
                self.snapshot = None;
                self.reset_personal_copy();
                self.classroom_link_input = None;
                self.invalidate_classroom_link_check();
                self.show_hidden_classes = false;
                self.open_course_menu = None;
                self.move_course_menu = None;
                self.course_drag = None;
                self.confirm_hide_course = None;
                self.confirm_class_archive = None;
                self.reset_class_folders();
                self.teachers_generation = self.teachers_generation.wrapping_add(1);
                self.class_teachers.clear();
                self.reset_notifications();
                self.reset_stream_access();
                self.confirming_stream_delete = None;
                self.open_stream_composer = None;
                self.stream_drafts.clear();
                self.drive_picker = None;
                self.reset_assignment_reads();
                self.submissions_checked.clear();
                self.page_errors.clear();
                self.class_code.clear();
                self.invalidate_invitation_read();
                self.invitations = None;
                self.opening_joined_course = None;
                self.confirming_decline = None;
                self.confirming_unenroll = None;
                self.comment_drafts.clear();
                self.invalidate_people_email();
                self.comment_mentions_generation = self.comment_mentions_generation.wrapping_add(1);
                self.comment_mentions = None;
                self.short_answer_drafts.clear();
                self.short_answer_revisions.clear();
                self.choice_drafts.clear();
                self.confirming_question_answer = None;
                self.comment_unconfirmed.clear();
                self.private_feedback.clear();
                self.rubrics.clear();
                self.overall_grades.clear();
                self.rubric_open = false;
                self.open_criteria.clear();
                self.private_drafts.clear();
                self.confirming_comment_delete = None;
                self.private_unconfirmed.clear();
                self.pending_remove_material = None;
                self.pending_submission = None;
                self.item_id = None;
                self.attachment_link.clear();
                self.submission_file = None;
                self.demo = false;
                self.screen = Screen::Home;
                self.search.clear();
                self.email.clear();
                self.error = None;
                self.notice = None;
            }
            _ => {}
        }
        self.finish_update()
    }
}
