use super::*;

#[derive(Clone)]
pub(in crate::desktop) enum SubmissionsMessage {
    ConfirmSubmission(i64, i64, desktop_auth::SubmissionAction),
    CancelSubmission,
    EditAttachmentLink(String),
    AddAttachmentLink(i64, i64),
    ChooseSubmissionFile(i64, i64),
    SubmissionFileChosen(i64, i64, Result<Option<PathBuf>, String>),
    SubmissionFileDropped(PathBuf),
    SubmissionFileValidated(i64, i64, PathBuf, Result<(), String>),
    CancelSubmissionFile,
    ConfirmPersonalCopy(i64, i64, String),
    CancelPersonalCopy,
    PreparePersonalCopy(i64, i64, String),
    PersonalCopyPrepared(i64, i64, Result<desktop_auth::PersonalCopyPreparation, String>),
    ConfirmRemoveMaterial(i64, i64, i64),
    CancelRemoveMaterial,
    RemoveMaterial(i64, i64, i64),
    MaterialRemoved(i64, Result<(), String>),
    UploadSubmissionFile(i64, i64),
    SubmissionFileUploaded(i64, Result<(), String>, Option<(i64, PathBuf)>),
    AttachmentLinkAdded(i64, Result<(), String>),
    SendSubmission(i64, i64, desktop_auth::SubmissionAction),
    SubmissionSent(i64, Result<(), String>),
    BeginShortAnswerEdit(i64, i64),
    DiscardShortAnswerEdit(i64, i64),
    EditShortAnswer(i64, i64, text_editor::Action),
    SelectQuestionChoice(i64, i64, String),
    ConfirmQuestionAnswer(i64, i64),
    CancelQuestionAnswer,
    SendQuestionAnswer(i64, i64),
    QuestionAnswerSent(i64, i64, Result<(), String>),
}

impl App {
    pub(super) fn update_submissions(&mut self, message: SubmissionsMessage) -> Task<Message> {
        match message {
            SubmissionsMessage::EditAttachmentLink(url) if !self.has_pending_request() => {
                self.attachments.attachment_link = url;
            }
            SubmissionsMessage::AddAttachmentLink(course, item)
                if self.can_compose_comment(course, item) && !self.has_pending_request()
                    && trusted_attachment_host(self.attachments.attachment_link.trim()).is_some() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                let url = self.attachments.attachment_link.clone();
                self.student_work.sending_submission = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::attach_submission_link(&session, course, item, url).await
                }, move |result| Message::Submissions(SubmissionsMessage::AttachmentLinkAdded(course, result)));
            }
            SubmissionsMessage::AttachmentLinkAdded(course, result) => {
                self.student_work.sending_submission = false;
                match result {
                    Ok(()) => {
                        self.attachments.attachment_link.clear();
                        self.notice = Some("Link attached.".into());
                        return self.update(Message::Reads(ReadsMessage::LoadDetail(DetailRead::Submissions(course))));
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            SubmissionsMessage::ChooseSubmissionFile(course, item)
                if self.can_add_submission_drive_file(course, item) && !self.has_pending_request() =>
            {
                self.attachments.choosing_submission_file = true;
                return Task::perform(choose_attachment_file(),
                    move |result| Message::Submissions(SubmissionsMessage::SubmissionFileChosen(course, item, result)));
            }
            SubmissionsMessage::SubmissionFileChosen(course, item, result) => {
                self.attachments.choosing_submission_file = false;
                if self.can_add_submission_drive_file(course, item) {
                    match result {
                        Ok(Some(path)) => return self.validate_file_for_staging(course, item, path),
                        Ok(None) => {},
                        Err(error) => self.error = Some(error),
                    }
                }
            }
            SubmissionsMessage::SubmissionFileDropped(path) => {
                if let Screen::Item(course) = self.screen
                    && let Some(item) = self.item_id.as_deref().and_then(|id| id.parse::<i64>().ok())
                    && self.can_add_submission_drive_file(course, item)
                    && !self.has_pending_request()
                    && self.attachments.submission_file.is_none()
                    && self.attachments.drive_picker.is_none()
                    && self.student_work.pending_submission.is_none()
                    && self.attachments.pending_personal_copy.is_none()
                    && self.attachments.pending_remove_material.is_none()
                {
                    return self.validate_file_for_staging(course, item, path);
                }
            }
            SubmissionsMessage::SubmissionFileValidated(course, item, path, result) => {
                self.attachments.choosing_submission_file = false;
                if self.can_add_submission_drive_file(course, item) {
                    self.stage_submission_file(course, item, path, result);
                }
            }
            SubmissionsMessage::CancelSubmissionFile => self.attachments.submission_file = None,
            SubmissionsMessage::UploadSubmissionFile(course, item)
                if self.can_add_submission_drive_file(course, item) && !self.has_pending_request() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                let Some((selected_course, selected_item, path)) = self.attachments.submission_file.take() else {
                    return Task::none();
                };
                if (selected_course, selected_item) != (course, item) {
                    self.attachments.submission_file = Some((selected_course, selected_item, path));
                    return Task::none();
                }
                self.student_work.sending_submission = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    let checked_path = path.clone();
                    let validation = tokio::task::spawn_blocking(move || validate_submission_file(&checked_path)).await
                        .unwrap_or_else(|_| Err("Could not check the selected file.".into()));
                    if let Err(error) = validation {
                        return (Err(error), Some((item, path)));
                    }
                    (desktop_auth::attach_submission_file(&session, course, item, path).await, None)
                }, move |(result, file)| Message::Submissions(SubmissionsMessage::SubmissionFileUploaded(course, result, file)));
            }
            SubmissionsMessage::SubmissionFileUploaded(course, result, file) => {
                self.student_work.sending_submission = false;
                if let Some((item, path)) = file
                    && self.can_add_submission_drive_file(course, item)
                {
                    self.attachments.submission_file = Some((course, item, path));
                }
                match result {
                    Ok(()) => {
                        self.notice = Some("File uploaded and attached to your work.".into());
                        return self.update(Message::Reads(ReadsMessage::LoadDetail(DetailRead::Submissions(course))));
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            SubmissionsMessage::ConfirmPersonalCopy(course, item, reference)
                if !self.has_pending_request() && self.personal_copy_ready(course, item, &reference) =>
            {
                self.attachments.pending_personal_copy = Some((course, item, reference));
            }
            SubmissionsMessage::CancelPersonalCopy => self.attachments.pending_personal_copy = None,
            SubmissionsMessage::PreparePersonalCopy(course, item, reference)
                if !self.has_pending_request() && self.attachments.pending_personal_copy.as_ref() == Some(&(course, item, reference.clone()))
                    && self.personal_copy_ready(course, item, &reference) =>
            {
                let Some(session) = self.session.clone() else { return Task::none(); };
                self.attachments.pending_personal_copy = None;
                self.attachments.personal_copy_checks.remove(&(course, item));
                self.student_work.sending_submission = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::prepare_personal_copy(&session, course, item, reference).await
                }, move |result| Message::Submissions(SubmissionsMessage::PersonalCopyPrepared(course, item, result)));
            }
            SubmissionsMessage::PersonalCopyPrepared(course, item, result) => {
                self.student_work.sending_submission = false;
                self.attachments.personal_copy_checks.remove(&(course, item));
                self.student_work.submissions_checked.remove(&course);
                let refresh = self.update(Message::Reads(ReadsMessage::LoadDetail(DetailRead::Submissions(course))));
                match result {
                    Ok(desktop_auth::PersonalCopyPreparation::Available(_)) => self.notice = Some("Your work is ready.".into()),
                    Ok(desktop_auth::PersonalCopyPreparation::PreparedWithoutCopy) => self.notice = Some("Your work is ready, but Classroom did not return a personal copy. Ask your teacher about the attachment.".into()),
                    Ok(desktop_auth::PersonalCopyPreparation::ExistingSubmissionWithoutCopy) => self.notice = Some("Your submission already exists. Ask your teacher about the missing attachment.".into()),
                    Err(error) => self.error = Some(error),
                }
                return refresh;
            }
            SubmissionsMessage::ConfirmRemoveMaterial(course, item, material)
                if !self.has_pending_request() && self.can_compose_comment(course, item) =>
            {
                self.attachments.pending_remove_material = Some((course, item, material));
            }
            SubmissionsMessage::CancelRemoveMaterial => self.attachments.pending_remove_material = None,
            SubmissionsMessage::RemoveMaterial(course, item, material)
                if self.attachments.pending_remove_material == Some((course, item, material))
                    && !self.has_pending_request() && self.can_compose_comment(course, item) =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.attachments.pending_remove_material = None;
                self.student_work.sending_submission = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::remove_submission_attachment(&session, course, item, material).await
                }, move |result| Message::Submissions(SubmissionsMessage::MaterialRemoved(course, result)));
            }
            SubmissionsMessage::MaterialRemoved(course, result) => {
                self.student_work.sending_submission = false;
                match result {
                    Ok(()) => {
                        self.notice = Some("Attachment removed from your submission.".into());
                        return self.update(Message::Reads(ReadsMessage::LoadDetail(DetailRead::Submissions(course))));
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            SubmissionsMessage::ConfirmSubmission(course, item, action) if self.can_compose_comment(course, item) => {
                self.student_work.pending_submission = Some((course, item, action));
            }
            SubmissionsMessage::CancelSubmission => self.student_work.pending_submission = None,
            SubmissionsMessage::SendSubmission(course, item, action)
                if !self.has_pending_request() && self.can_compose_comment(course, item)
                    && self.student_work.pending_submission == Some((course, item, action)) =>
            {
                if let Some(session) = self.session.clone() {
                    self.student_work.pending_submission = None;
                    self.student_work.sending_submission = true;
                    self.error = None;
                    self.notice = None;
                    return Task::perform(async move {
                        desktop_auth::change_submission(&session, course, item, action).await
                    }, move |result| Message::Submissions(SubmissionsMessage::SubmissionSent(course, result)));
                }
            }
            SubmissionsMessage::SubmissionSent(course, result) => {
                self.student_work.sending_submission = false;
                match result {
                    Ok(()) => self.notice = Some("Submission status updated.".into()),
                    Err(error) => self.error = Some(error),
                }
                return self.update(Message::Reads(ReadsMessage::LoadDetail(DetailRead::Submissions(course))));
            }
            SubmissionsMessage::BeginShortAnswerEdit(course, item) if !self.has_pending_request() => {
                if let Some((submission_id, answer)) = self.current_question(course, item)
                    .and_then(desktop_auth::editable_short_answer)
                {
                    let editor = text_editor::Content::with_text(answer);
                    let revision = desktop_auth::ShortAnswerRevision { submission_id, answer: answer.to_owned() };
                    self.student_work.short_answer_drafts.insert((course, item), editor);
                    self.student_work.short_answer_revisions.insert((course, item), revision);
                }
            }
            SubmissionsMessage::DiscardShortAnswerEdit(course, item)
                if self.can_compose_comment(course, item) && !self.student_work.sending_submission =>
            {
                self.student_work.confirming_question_answer = None;
                self.student_work.short_answer_drafts.remove(&(course, item));
                self.student_work.short_answer_revisions.remove(&(course, item));
            }
            SubmissionsMessage::EditShortAnswer(course, item, action)
                if self.editable_question(course, item) == Some(&QuestionDetails::ShortAnswer) && !self.student_work.sending_submission
                    && self.student_work.confirming_question_answer.is_none() =>
            {
                self.student_work.short_answer_drafts.entry((course, item)).or_default().perform(action);
            }
            SubmissionsMessage::SelectQuestionChoice(course, item, choice)
                if !self.student_work.sending_submission && self.student_work.confirming_question_answer.is_none()
                    && self.editable_question(course, item).is_some_and(|details| {
                        details.accepts(&QuestionAnswer::MultipleChoice(choice.clone()))
                    }) =>
            {
                self.student_work.choice_drafts.insert((course, item), choice);
            }
            SubmissionsMessage::ConfirmQuestionAnswer(course, item)
                if !self.has_pending_request() && self.editable_question(course, item)
                    .and_then(|details| self.question_draft((course, item), details)).is_some() =>
            {
                self.student_work.confirming_question_answer = Some((course, item));
            }
            SubmissionsMessage::CancelQuestionAnswer => self.student_work.confirming_question_answer = None,
            SubmissionsMessage::SendQuestionAnswer(course, item)
                if self.student_work.confirming_question_answer == Some((course, item)) && !self.has_pending_request() =>
            {
                let Some(answer) = self.editable_question(course, item)
                    .and_then(|details| self.question_draft((course, item), details))
                else { return Task::none() };
                let Some(session) = self.session.clone() else { return Task::none() };
                let revision = self.student_work.short_answer_revisions.get(&(course, item)).cloned();
                self.student_work.confirming_question_answer = None;
                self.student_work.sending_submission = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::turn_in_question(&session, course, item, answer, revision).await
                }, move |result| Message::Submissions(SubmissionsMessage::QuestionAnswerSent(course, item, result)));
            }
            SubmissionsMessage::QuestionAnswerSent(course, item, result) => {
                self.student_work.sending_submission = false;
                let refresh = self.update(Message::Reads(ReadsMessage::LoadDetail(DetailRead::Submissions(course))));
                match result {
                    Ok(()) => {
                        self.student_work.short_answer_drafts.remove(&(course, item));
                        self.student_work.short_answer_revisions.remove(&(course, item));
                        self.student_work.choice_drafts.remove(&(course, item));
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
