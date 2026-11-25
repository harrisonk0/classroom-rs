use super::*;

impl App {
    pub(super) fn update_submissions(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::EditAttachmentLink(url) if !self.has_pending_request() => {
                self.attachment_link = url;
            }
            Message::AddAttachmentLink(course, item)
                if self.can_compose_comment(course, item) && !self.has_pending_request()
                    && trusted_attachment_host(self.attachment_link.trim()).is_some() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                let url = self.attachment_link.clone();
                self.sending_submission = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::attach_submission_link(&session, course, item, url).await
                }, move |result| Message::AttachmentLinkAdded(course, result));
            }
            Message::AttachmentLinkAdded(course, result) => {
                self.sending_submission = false;
                match result {
                    Ok(()) => {
                        self.attachment_link.clear();
                        self.notice = Some("Link attached.".into());
                        return self.update(Message::LoadDetail(DetailRead::Submissions(course)));
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Message::ChooseSubmissionFile(course, item)
                if self.can_add_submission_drive_file(course, item) && !self.has_pending_request() =>
            {
                self.choosing_submission_file = true;
                return Task::perform(choose_attachment_file(),
                    move |result| Message::SubmissionFileChosen(course, item, result));
            }
            Message::SubmissionFileChosen(course, item, result) => {
                self.choosing_submission_file = false;
                if self.can_add_submission_drive_file(course, item) {
                    match result {
                        Ok(Some(path)) => self.stage_submission_file(course, item, path),
                        Ok(None) => {},
                        Err(error) => self.error = Some(error),
                    }
                }
            }
            Message::SubmissionFileDropped(path) => {
                if let Screen::Item(course) = self.screen
                    && let Some(item) = self.item_id.as_deref().and_then(|id| id.parse::<i64>().ok())
                    && self.can_add_submission_drive_file(course, item)
                    && !self.has_pending_request()
                    && self.submission_file.is_none()
                    && self.drive_picker.is_none()
                    && self.pending_submission.is_none()
                    && self.pending_personal_copy.is_none()
                    && self.pending_remove_material.is_none()
                {
                    self.stage_submission_file(course, item, path);
                }
            }
            Message::CancelSubmissionFile => self.submission_file = None,
            Message::UploadSubmissionFile(course, item)
                if self.can_add_submission_drive_file(course, item) && !self.has_pending_request() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                let Some((selected_course, selected_item, path)) = self.submission_file.take() else {
                    return Task::none();
                };
                if (selected_course, selected_item) != (course, item) {
                    self.submission_file = Some((selected_course, selected_item, path));
                    return Task::none();
                }
                if let Err(error) = validate_submission_file(&path) {
                    self.submission_file = Some((course, item, path));
                    self.error = Some(error);
                    return Task::none();
                }
                self.sending_submission = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::attach_submission_file(&session, course, item, path).await
                }, move |result| Message::SubmissionFileUploaded(course, result));
            }
            Message::SubmissionFileUploaded(course, result) => {
                self.sending_submission = false;
                match result {
                    Ok(()) => {
                        self.notice = Some("File uploaded and attached to your work.".into());
                        return self.update(Message::LoadDetail(DetailRead::Submissions(course)));
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Message::ConfirmPersonalCopy(course, item, reference)
                if !self.has_pending_request() && self.personal_copy_ready(course, item, &reference) =>
            {
                self.pending_personal_copy = Some((course, item, reference));
            }
            Message::CancelPersonalCopy => self.pending_personal_copy = None,
            Message::PreparePersonalCopy(course, item, reference)
                if !self.has_pending_request() && self.pending_personal_copy.as_ref() == Some(&(course, item, reference.clone()))
                    && self.personal_copy_ready(course, item, &reference) =>
            {
                let Some(session) = self.session.clone() else { return Task::none(); };
                self.pending_personal_copy = None;
                self.personal_copy_checks.remove(&(course, item));
                self.sending_submission = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::prepare_personal_copy(&session, course, item, reference).await
                }, move |result| Message::PersonalCopyPrepared(course, item, result));
            }
            Message::PersonalCopyPrepared(course, item, result) => {
                self.sending_submission = false;
                self.personal_copy_checks.remove(&(course, item));
                self.submissions_checked.remove(&course);
                let refresh = self.update(Message::LoadDetail(DetailRead::Submissions(course)));
                match result {
                    Ok(desktop_auth::PersonalCopyPreparation::Available(_)) => self.notice = Some("Your work is ready.".into()),
                    Ok(desktop_auth::PersonalCopyPreparation::PreparedWithoutCopy) => self.notice = Some("Your work is ready, but Classroom did not return a personal copy. Ask your teacher about the attachment.".into()),
                    Ok(desktop_auth::PersonalCopyPreparation::ExistingSubmissionWithoutCopy) => self.notice = Some("Your submission already exists. Ask your teacher about the missing attachment.".into()),
                    Err(error) => self.error = Some(error),
                }
                return refresh;
            }
            Message::ConfirmRemoveMaterial(course, item, material)
                if !self.has_pending_request() && self.can_compose_comment(course, item) =>
            {
                self.pending_remove_material = Some((course, item, material));
            }
            Message::CancelRemoveMaterial => self.pending_remove_material = None,
            Message::RemoveMaterial(course, item, material)
                if self.pending_remove_material == Some((course, item, material))
                    && !self.has_pending_request() && self.can_compose_comment(course, item) =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.pending_remove_material = None;
                self.sending_submission = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::remove_submission_attachment(&session, course, item, material).await
                }, move |result| Message::MaterialRemoved(course, result));
            }
            Message::MaterialRemoved(course, result) => {
                self.sending_submission = false;
                match result {
                    Ok(()) => {
                        self.notice = Some("Attachment removed from your submission.".into());
                        return self.update(Message::LoadDetail(DetailRead::Submissions(course)));
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Message::ConfirmSubmission(course, item, action) if self.can_compose_comment(course, item) => {
                self.pending_submission = Some((course, item, action));
            }
            Message::CancelSubmission => self.pending_submission = None,
            Message::SendSubmission(course, item, action)
                if !self.has_pending_request() && self.can_compose_comment(course, item)
                    && self.pending_submission == Some((course, item, action)) =>
            {
                if let Some(session) = self.session.clone() {
                    self.pending_submission = None;
                    self.sending_submission = true;
                    self.error = None;
                    self.notice = None;
                    return Task::perform(async move {
                        desktop_auth::change_submission(&session, course, item, action).await
                    }, move |result| Message::SubmissionSent(course, result));
                }
            }
            Message::SubmissionSent(course, result) => {
                self.sending_submission = false;
                match result {
                    Ok(()) => self.notice = Some("Submission status updated.".into()),
                    Err(error) => self.error = Some(error),
                }
                return self.update(Message::LoadDetail(DetailRead::Submissions(course)));
            }
            Message::BeginShortAnswerEdit(course, item) if !self.has_pending_request() => {
                if let Some((submission_id, answer)) = self.current_question(course, item)
                    .and_then(desktop_auth::editable_short_answer)
                {
                    let editor = text_editor::Content::with_text(answer);
                    let revision = desktop_auth::ShortAnswerRevision { submission_id, answer: answer.to_owned() };
                    self.short_answer_drafts.insert((course, item), editor);
                    self.short_answer_revisions.insert((course, item), revision);
                }
            }
            Message::DiscardShortAnswerEdit(course, item)
                if self.can_compose_comment(course, item) && !self.sending_submission =>
            {
                self.confirming_question_answer = None;
                self.short_answer_drafts.remove(&(course, item));
                self.short_answer_revisions.remove(&(course, item));
            }
            Message::EditShortAnswer(course, item, action)
                if self.editable_question(course, item) == Some(&QuestionDetails::ShortAnswer) && !self.sending_submission
                    && self.confirming_question_answer.is_none() =>
            {
                self.short_answer_drafts.entry((course, item)).or_default().perform(action);
            }
            Message::SelectQuestionChoice(course, item, choice)
                if !self.sending_submission && self.confirming_question_answer.is_none()
                    && self.editable_question(course, item).is_some_and(|details| {
                        details.accepts(&QuestionAnswer::MultipleChoice(choice.clone()))
                    }) =>
            {
                self.choice_drafts.insert((course, item), choice);
            }
            Message::ConfirmQuestionAnswer(course, item)
                if !self.has_pending_request() && self.editable_question(course, item)
                    .and_then(|details| self.question_draft((course, item), details)).is_some() =>
            {
                self.confirming_question_answer = Some((course, item));
            }
            Message::CancelQuestionAnswer => self.confirming_question_answer = None,
            Message::SendQuestionAnswer(course, item)
                if self.confirming_question_answer == Some((course, item)) && !self.has_pending_request() =>
            {
                let Some(answer) = self.editable_question(course, item)
                    .and_then(|details| self.question_draft((course, item), details))
                else { return Task::none() };
                let Some(session) = self.session.clone() else { return Task::none() };
                let revision = self.short_answer_revisions.get(&(course, item)).cloned();
                self.confirming_question_answer = None;
                self.sending_submission = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::turn_in_question(&session, course, item, answer, revision).await
                }, move |result| Message::QuestionAnswerSent(course, item, result));
            }
            Message::QuestionAnswerSent(course, item, result) => {
                self.sending_submission = false;
                let refresh = self.update(Message::LoadDetail(DetailRead::Submissions(course)));
                match result {
                    Ok(()) => {
                        self.short_answer_drafts.remove(&(course, item));
                        self.short_answer_revisions.remove(&(course, item));
                        self.choice_drafts.remove(&(course, item));
                        self.notice = Some("Answer turned in.".into());
                    }
                    Err(error) => self.error = Some(error),
                }
                return refresh;
            }
            _ => {}
        }
        self.finish_update()
    }
}
