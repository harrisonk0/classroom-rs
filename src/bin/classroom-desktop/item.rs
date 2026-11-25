use super::*;
use iced::widget::column;

impl App {
    pub(super) fn item_detail(&self, course_id: i64) -> Element<'_, Message> {
        let Some(snapshot) = &self.snapshot else {
            return self.unavailable_screen("Classwork", Icon::Book, "This item is no longer loaded.");
        };
        let Some(item) = self.item_id.as_ref()
            .and_then(|key| snapshot.coursework_for(course_id)?.iter().find(|item| &item.id == key))
        else {
            let status = if snapshot.coursework_for(course_id).is_none() {
                self.page_read_status(PageRead::Coursework(course_id), false, "Loading classwork…")
            } else {
                None
            };
            return scrollable(column![
                button("‹ Back").style(quiet_style).on_press(Message::CloseItem),
                status.unwrap_or_else(|| empty_panel(
                    Icon::Book, "This item is unavailable", "It may have been removed since the last classwork refresh.",
                )),
            ].spacing(10).padding(26).width(Fill)).width(Fill).height(Fill).into();
        };
        let course_name = snapshot.courses.iter().find(|course| course.id == course_id)
            .map_or("Classwork", |course| course.title.as_str());
        let topic = item.topic_id.as_ref().and_then(|id| {
            snapshot.topics_for(course_id)?.iter().find(|topic| &topic.id == id)
        });
        let item_label = item.question.as_ref().map_or(item.kind.label(), QuestionDetails::label);
        let copy_link: Element<'_, Message> = match item_link_kind(item).zip(item.id.parse::<i64>().ok()) {
            Some((kind, id)) => icon_action(Icon::Copy, "Copy link",
                Some(Message::CopyClassroomLink(ClassroomLink::Item { course: course_id, item: id, kind }))),
            None => space().width(0).into(),
        };
        let open_in_classroom: Element<'_, Message> = if item.kind == CourseworkKind::Assignment {
            item.id.parse::<i64>().ok()
                .and_then(|id| ClassroomLink::Item {
                    course: course_id, item: id, kind: ItemLinkKind::Assignment,
                }.web_url())
                .filter(|_| !self.verify_live_course_count)
                .map_or_else(|| space().width(0).into(), |url| {
                    icon_action(Icon::External, "Open in Classroom", Some(Message::OpenAttachment(url)))
                })
        } else {
            space().width(0).into()
        };
        let mut body = column![
            row![
                button("‹ Back").style(quiet_style).padding([8, 10]).on_press(Message::CloseItem),
                text(course_name).size(14).color(MUTED).width(Fill),
                open_in_classroom,
                copy_link,
            ].spacing(8).align_y(Alignment::Center),
            text(topic.map_or_else(|| item_label.to_owned(), |topic| {
                format!("{item_label} · {}", topic.title)
            })).size(14).color(BLUE),
            text(&item.title).size(29).color(FG).font(medium()),
        ].spacing(9).width(Fill);
        if let Some(due) = &item.due_label {
            body = body.push(pill(due, ACTIVE_BG, ACTIVE_FG));
        }
        if !item.description.trim().is_empty() {
            body = body.push(container(formatted_description(&item.description, &item.description_styles, &item.description_bullets))
                .padding([10, 0]));
        }
        let own_answer = item.submission.as_ref().and_then(|submission| submission.question_answer.as_ref());
        if item.question_sharing == Some(QuestionSharing::Course) {
            let visibility = match &item.question {
                Some(QuestionDetails::ShortAnswer) => "Classmates can see your answer after they turn in theirs.",
                Some(QuestionDetails::MultipleChoice(_)) => "Your answer counts toward the class summary.",
                None => "",
            };
            if !visibility.is_empty() {
                body = body.push(text(visibility).size(14).color(MUTED));
            }
            if self.can_open_shared_question(course_id)
                && let Some((label, url)) = shared_question_link(item)
            {
                body = body.push(button(label).style(outline_style).padding([8, 12])
                    .on_press(Message::OpenAttachment(url)));
            }
        }
        let question_key = item.id.parse::<i64>().ok().map(|id| (course_id, id));
        let revision = question_key.and_then(|key| self.short_answer_revisions.get(&key));
        let editable = question_key.and_then(|(course, id)| self.editable_question(course, id))
            .or_else(|| {
                if self.screenshot_directory.is_some() && desktop_auth::question_submission_is_current(item, revision) {
                    item.question.as_ref()
                } else {
                    None
                }
            });
        let mut choice_shown = false;
        if let Some(QuestionDetails::MultipleChoice(choices)) = &item.question
            && !choices.is_empty()
        {
            let chosen = match own_answer {
                Some(QuestionAnswer::MultipleChoice(answer)) => matching_choice(choices, answer),
                _ => None,
            };
            let drafted = editable.and_then(|_| question_key)
                .and_then(|key| self.choice_drafts.get(&key)).and_then(|choice| matching_choice(choices, choice));
            let mut choices_panel = column![text("Answer choices").size(17).font(medium()).color(FG)].spacing(7);
            for (index, choice) in choices.iter().enumerate() {
                let selected = chosen == Some(index) || drafted == Some(index);
                choice_shown |= chosen == Some(index);
                let mut row = row![
                    text(if selected { "●" } else { "○" }).size(16).color(if selected { ACTIVE_FG } else { MUTED }),
                    text(choice).size(15).color(FG).width(Fill).wrapping(text::Wrapping::WordOrGlyph),
                ].spacing(8).align_y(Alignment::Center);
                if chosen == Some(index) {
                    row = row.push(pill("Your answer", ACTIVE_BG, ACTIVE_FG));
                } else if drafted == Some(index) {
                    row = row.push(pill("Selected", ACTIVE_BG, ACTIVE_FG));
                }
                let option: Element<'_, Message> = if let (Some((course, id)), Some(details)) = (question_key, editable)
                    && !self.sending_submission && self.confirming_question_answer.is_none()
                    && details.accepts(&QuestionAnswer::MultipleChoice(choice.clone()))
                {
                    button(row).width(Fill).padding([7, 4]).style(quiet_style)
                        .on_press(Message::SelectQuestionChoice(course, id, choice.clone())).into()
                } else {
                    container(row).width(Fill).padding([7, 4]).into()
                };
                choices_panel = choices_panel.push(option);
            }
            body = body.push(container(choices_panel).width(Fill).padding(15).style(|_| outline_surface(9.0)));
        }
        let teacher = self.class_roles.get(&course_id) == Some(&CourseRole::Teacher);
        if teacher && has_submission(item) && (self.can_read_pages() || self.screenshot_directory.is_some()) {
            body = body.push(self.teacher_student_work_panel(item));
        } else if has_submission(item) {
            let status = self.page_read_status(
                PageRead::Detail(DetailRead::Submissions(course_id)),
                self.submissions_checked.contains(&course_id), "Loading your work…",
            );
            let mut work = column![text("Your work").size(17).font(medium()).color(FG)].spacing(8);
            if let Some(submission) = &item.submission {
                work = work.push(text(submission.status.label()).size(14).color(MUTED));
                if let Some(date) = submission.last_turned_in_millis.and_then(DateTime::from_timestamp_millis) {
                    work = work.push(text(format!("Last turned in {}",
                        date.with_timezone(&Local).format("%-d %b %Y at %H:%M"))).size(14).color(MUTED));
                }
                if let Some(label) = grade_label(submission) {
                    work = work.push(text(label).size(15).color(FG));
                }
                if !choice_shown
                    && let Some(QuestionAnswer::ShortAnswer(answer) | QuestionAnswer::MultipleChoice(answer)) = own_answer
                {
                    work = work.push(text("Your answer").size(14).font(medium()).color(FG))
                        .push(text(answer).size(15).color(FG).width(Fill).wrapping(text::Wrapping::WordOrGlyph));
                }
                for attachment in &submission.attachments {
                    let label = if item.attachments.iter().any(|template| attachment.is_personal_copy_of(template)) {
                        format!("{} · Your copy", attachment.title)
                    } else {
                        attachment.title.clone()
                    };
                    let open = button(text(label)).padding([8, 10]).style(outline_style)
                        .on_press_maybe(self.attachment_action(attachment));
                    let mut line = row![open].spacing(7).align_y(Alignment::Center);
                    if let Some(browser) = self.attachment_browser_button(attachment) {
                        line = line.push(browser);
                    }
                    if let Some(save) = self.save_attachment_button(attachment) {
                        line = line.push(save);
                    }
                    if let (Some(material), Ok(item_id)) = (attachment.remote_id, item.id.parse::<i64>())
                        && self.can_compose_comment(course_id, item_id)
                        && desktop_auth::SubmissionAction::for_submission(submission)
                            == Some(desktop_auth::SubmissionAction::TurnIn)
                    {
                        if self.pending_remove_material == Some((course_id, item_id, material)) {
                            line = line
                                .push(button("Cancel").style(quiet_style).on_press(Message::CancelRemoveMaterial))
                                .push(button("Confirm remove").style(outline_style)
                                    .on_press_maybe((!self.has_pending_request())
                                        .then_some(Message::RemoveMaterial(course_id, item_id, material))));
                        } else {
                            line = line.push(button("Remove…").style(quiet_style)
                                .on_press_maybe((!self.has_pending_request())
                                    .then_some(Message::ConfirmRemoveMaterial(course_id, item_id, material))));
                        }
                    }
                    work = work.push(line);
                }
            } else if status.is_none() {
                work = work.push(text("No submission status available for this item.").size(14).color(MUTED));
            }
            if let Some(status) = status {
                work = work.push(status);
            }
            if let (Some(key @ (_, item_id)), Some(details)) = (question_key, editable) {
                let confirming = self.confirming_question_answer == Some(key);
                if *details == QuestionDetails::ShortAnswer {
                    let draft = self.short_answer_drafts.get(&key).unwrap_or(&self.empty_short_answer);
                    if revision.is_some() {
                        work = work.push(text("Edit your answer").size(15).font(medium()).color(FG));
                    }
                    let mut editor = text_editor(draft).placeholder("Type your answer")
                        .height(if revision.is_some() { 135 } else { 120 }).font(CLASSROOM_FONT)
                        .wrapping(text::Wrapping::WordOrGlyph).size(15).padding(9);
                    if !self.sending_submission && !confirming {
                        editor = editor.on_action(move |action| Message::EditShortAnswer(course_id, item_id, action));
                    }
                    work = work.push(editor);
                }
                let ready = !self.has_pending_request() && self.question_draft(key, details).is_some();
                if confirming {
                    work = work.push(text(if revision.is_some() { "Turn in this revised answer?" }
                        else { "Turn in this answer?" }).size(14))
                        .push(row![
                            button("Cancel").style(quiet_style).on_press(Message::CancelQuestionAnswer),
                            button("Turn in").style(primary_style).padding([8, 12])
                                .on_press_maybe(ready.then_some(Message::SendQuestionAnswer(course_id, item_id))),
                        ].spacing(8));
                } else {
                    let submit = button(if self.sending_submission { "Turning in…" } else { "Turn in…" })
                        .style(outline_style).padding([8, 12])
                        .on_press_maybe(ready.then_some(Message::ConfirmQuestionAnswer(course_id, item_id)));
                    if revision.is_some() {
                        work = work.push(row![
                            submit,
                            button("Discard edit").style(quiet_style).padding([8, 12])
                                .on_press_maybe((!self.sending_submission).then_some(Message::DiscardShortAnswerEdit(course_id, item_id))),
                        ].spacing(8));
                    } else {
                        work = work.push(submit);
                    }
                }
            } else if let Some((course, id)) = question_key
                && self.current_question(course, id).and_then(desktop_auth::editable_short_answer).is_some()
            {
                if revision.is_some() {
                    work = work.push(text("Your saved answer changed while you were editing.").size(14).color(MUTED));
                }
                work = work.push(button(if revision.is_some() { "Start a new edit" } else { "Edit answer" })
                    .style(outline_style).padding([8, 12])
                    .on_press_maybe((!self.has_pending_request()).then_some(Message::BeginShortAnswerEdit(course, id))));
            }
            if item.kind == CourseworkKind::Assignment
                && let Ok(item_id) = item.id.parse::<i64>()
                && (self.can_compose_comment(course_id, item_id)
                    || self.screenshot_directory.is_some()
                        && self.pending_submission.is_some_and(|(course, item, _)| (course, item) == (course_id, item_id)))
                && let Some(action) = item.submission.as_ref().and_then(desktop_auth::SubmissionAction::for_submission)
            {
                if action == desktop_auth::SubmissionAction::TurnIn {
                    let ready = !self.has_pending_request()
                        && trusted_attachment_host(self.attachment_link.trim()).is_some();
                    work = work.push(row![
                        text_input("Add an HTTPS link", &self.attachment_link).padding(9)
                            .on_input(Message::EditAttachmentLink).width(Fill),
                        button("Attach link").padding([8, 12]).style(outline_style)
                            .on_press_maybe(ready.then_some(Message::AddAttachmentLink(course_id, item_id))),
                    ].spacing(8).align_y(Alignment::Center));
                }
                if action == desktop_auth::SubmissionAction::TurnIn {
                    if let Some((course, item, path)) = &self.submission_file
                        && (*course, *item) == (course_id, item_id)
                    {
                        let name = path.file_name().map_or_else(|| "File".into(), |name| name.to_string_lossy());
                        work = work.push(row![
                            text(name).size(14).width(Fill),
                            button("Cancel").style(quiet_style).on_press(Message::CancelSubmissionFile),
                            button("Upload and attach").padding([8, 12]).style(primary_style)
                                .on_press_maybe((!self.has_pending_request())
                                    .then_some(Message::UploadSubmissionFile(course_id, item_id))),
                        ].spacing(8).align_y(Alignment::Center));
                    } else {
                        work = work.push(row![
                            button(if self.sending_submission { "Updating your work…" } else { "Choose file…" })
                                .padding([8, 12]).style(outline_style)
                                .on_press_maybe((!self.has_pending_request())
                                    .then_some(Message::ChooseSubmissionFile(course_id, item_id))),
                            text("or drop a file into this window").size(13).color(MUTED),
                        ].spacing(8).align_y(Alignment::Center));
                    }
                }
                let confirmation = (course_id, item_id, action);
                let mark_done = assignment_is_mark_done(action, item.submission.as_ref());
                let label = if mark_done { "Mark as done" } else { action.label() };
                if self.pending_submission == Some(confirmation) {
                    work = work.push(text(match action {
                        desktop_auth::SubmissionAction::TurnIn if mark_done => "Mark this assignment as done without attaching a file?",
                        desktop_auth::SubmissionAction::TurnIn => "Turn in the work currently attached to this assignment?",
                        desktop_auth::SubmissionAction::Unsubmit => "Unsubmit this work so you can make changes?",
                    }).size(14));
                    work = work.push(row![
                        button("Cancel").style(quiet_style).on_press(Message::CancelSubmission),
                        button(label).style(primary_style)
                            .on_press_maybe((!self.has_pending_request())
                                .then_some(Message::SendSubmission(course_id, item_id, action))),
                    ].spacing(8));
                } else {
                    work = work.push(button(label).style(outline_style)
                        .on_press_maybe((!self.has_pending_request())
                            .then_some(Message::ConfirmSubmission(course_id, item_id, action))));
                }
            }
            if let Ok(item_id) = item.id.parse::<i64>() {
                if item.kind == CourseworkKind::Assignment
                    && (self.can_read_pages() && self.class_roles.get(&course_id) == Some(&CourseRole::Student)
                        || self.screenshot_directory.is_some())
                    && let Some(url) = desktop_auth::originality_report_url(course_id, item_id, item.originality_enabled == Some(true))
                {
                    work = work.push(button("Open originality reports in Classroom").padding([8, 12])
                        .style(outline_style).on_press(Message::OpenAttachment(url)));
                }
                let picker = self.drive_picker.as_ref().filter(|picker| picker.course == course_id && picker.item == Some(item_id));
                let ready = self.can_add_submission_drive_file(course_id, item_id) && !self.has_pending_request();
                #[cfg(target_os = "macos")]
                if self.can_add_submission_drive_file(course_id, item_id) || self.submission_capture.is_some()
                    || (self.screenshot_directory.is_some() && self.new_submission_document_open)
                {
                    work = work.push(self.submission_capture_controls(ready || self.screenshot_directory.is_some()));
                }
                if let Some(picker) = picker
                    && (self.can_add_submission_drive_file(course_id, item_id) || self.screenshot_directory.is_some())
                {
                    work = work.push(self.drive_picker_panel(picker, ready || self.screenshot_directory.is_some(), |file| {
                        item.submission.as_ref().is_some_and(|submission| {
                            submission.attachments.iter().any(|attachment| file.matches_attachment(attachment))
                        })
                    }));
                } else if self.can_add_submission_drive_file(course_id, item_id) {
                    work = work.push(button("Attach from Google Drive").style(outline_style).padding([8, 12])
                        .on_press_maybe(ready.then_some(Message::OpenSubmissionDrivePicker(course_id, item_id))));
                }
                if self.can_add_submission_drive_file(course_id, item_id)
                    || self.screenshot_directory.is_some() && self.new_submission_document_open
                {
                    work = work.push(button(if self.new_submission_document_open { "Cancel" } else { "Create new" })
                        .style(outline_style).padding([8, 12])
                        .on_press_maybe(ready.then_some(Message::ToggleNewSubmissionDocument(course_id, item_id))));
                    if self.new_submission_document_open {
                        let kinds = DriveDocumentKind::ALL.into_iter().map(|kind| {
                            button(kind.label()).style(quiet_style).padding([7, 10])
                                .on_press_maybe((ready || self.screenshot_directory.is_some())
                                    .then_some(Message::CreateSubmissionDocument(course_id, item_id, kind))).into()
                        });
                        work = work.push(row(kinds).spacing(6).wrap().vertical_spacing(6));
                    }
                }
            }
            body = body.push(container(work).width(Fill).padding(16).style(|_| outline_surface(9.0)));
        }
        if snapshot.source == SnapshotSource::Live
            && item.submission.as_ref().is_some_and(|sub| sub.remote_id.is_some())
            && let Ok(item_id) = item.id.parse::<i64>()
        {
            let feedback = self.private_feedback.get(&(course_id, item_id));
            let mut panel = column![text("Private feedback").size(17).font(medium()).color(FG)].spacing(8);
            if let Some(status) = self.page_read_status(
                PageRead::Detail(DetailRead::PrivateFeedback(course_id, item_id)),
                feedback.is_some(), "Loading private feedback…",
            ) {
                panel = panel.push(status);
            }
            match feedback {
                Some(entries) if entries.is_empty() => {
                    panel = panel.push(text("No private feedback on this submission.").size(14).color(MUTED));
                }
                Some(entries) => {
                    for entry in entries {
                        panel = panel.push(self.comment_card(course_id, Some(item_id), CommentAudience::Private, entry));
                    }
                }
                None => {},
            }
            if self.can_compose_comment(course_id, item_id) {
                let key = (course_id, item_id);
                let draft = self.private_drafts.get(&key).unwrap_or(&self.empty_comment_draft);
                let draft_text = draft.text();
                let unconfirmed = self.private_unconfirmed.contains(&key);
                let allowed = !self.has_pending_request() && !draft_text.trim().is_empty() && !unconfirmed;
                if unconfirmed {
                    panel = panel.push(text("Check the updated comments before sending your draft again.").size(14).color(MUTED));
                }
                let editor = text_editor(draft).placeholder("Add a private comment for your teacher…")
                    .height(86).padding(10);
                let editor = if self.posting_comment { editor } else {
                    editor.on_action(move |action| Message::EditPrivateComment(course_id, item_id, action))
                };
                panel = panel.push(column![
                    editor,
                    row![space().width(Fill), button(if self.posting_comment { "Sending…" } else { "Send privately" })
                        .padding([9, 13]).style(primary_style)
                        .on_press_maybe(allowed.then_some(Message::PostPrivateComment(course_id, item_id)))],
                ].spacing(8));
            }
            body = body.push(container(panel).width(Fill).padding(16).style(|_| outline_surface(9.0)));
        }
        if !item.attachments.is_empty() {
            let mut attachments = column![text("Attachments").size(17).font(medium()).color(FG)].spacing(8);
            for attachment in &item.attachments {
                let template = attachment.drive_sharing == Some(DriveSharing::IndividualCopy);
                let file = if template {
                    matching_personal_copy(item, attachment)
                } else {
                    Some(attachment)
                };
                let action = file.and_then(|file| self.attachment_action(file));
                let opening = file.and_then(|file| file.drive_reference.as_ref())
                    .is_some_and(|id| self.opening_drive_file.as_ref() == Some(id));
                let host = file.and_then(|file| file.url.as_deref()).and_then(trusted_attachment_host);
                let description = match (attachment.drive_sharing, action.is_some(), opening) {
                    (_, _, true) => "Opening in Drive…".to_owned(),
                    (Some(DriveSharing::IndividualCopy), true, _) => "Opens your personal copy".to_owned(),
                    (Some(DriveSharing::IndividualCopy), false, _) if !self.submissions_checked.contains(&course_id)
                        && self.can_read_pages() && !self.page_errors.contains_key(&PageRead::Detail(DetailRead::Submissions(course_id))) =>
                        "Checking for your personal copy…".to_owned(),
                    (Some(DriveSharing::IndividualCopy), false, _) => "Personal copy unavailable".to_owned(),
                    (Some(sharing), _, _) => format!("{} · {}", attachment.kind.label(), sharing.label()),
                    (None, true, _) => host.map_or_else(|| attachment.kind.label().to_owned(), |host|
                        format!("{} · {host}", attachment.kind.label())),
                    (None, false, _) => format!("{} · no secure link", attachment.kind.label()),
                };
                let content = row![
                    icon(attachment_icon(attachment.kind), ACTIVE_FG, 20),
                    column![text(&attachment.title).size(15).color(FG), text(description).size(12).color(MUTED)]
                        .spacing(3).width(Fill),
                    icon(Icon::Arrow, MUTED, 16),
                ].spacing(12).align_y(Alignment::Center);
                let mut line = row![button(content).width(Fill).padding(13)
                    .style(outline_style).on_press_maybe(action)].spacing(4).align_y(Alignment::Center);
                if let Some(browser) = file.and_then(|file| self.attachment_browser_button(file)) {
                    line = line.push(browser);
                }
                if let Some(save) = file.and_then(|file| self.save_attachment_button(file)) {
                    line = line.push(save);
                }
                attachments = attachments.push(line);
                if let (Ok(item_id), Some(reference)) = (item.id.parse::<i64>(), attachment.drive_reference.as_ref())
                    && self.personal_copy_ready(course_id, item_id, reference)
                {
                    if self.pending_personal_copy.as_ref() == Some(&(course_id, item_id, reference.clone())) {
                        attachments = attachments.push(text("Prepare your work for this assignment?").size(14))
                            .push(row![
                                button("Cancel").style(quiet_style).padding([7, 10]).on_press(Message::CancelPersonalCopy),
                                button("Prepare").style(primary_style).padding([7, 10])
                                    .on_press_maybe((!self.has_pending_request()).then(||
                                        Message::PreparePersonalCopy(course_id, item_id, reference.clone()))),
                            ].spacing(8));
                    } else {
                        attachments = attachments.push(button("Prepare personal copy…").style(outline_style).padding([7, 10])
                            .on_press_maybe((!self.has_pending_request()).then(||
                                Message::ConfirmPersonalCopy(course_id, item_id, reference.clone()))));
                    }
                }

            }
            if let Ok(item_id) = item.id.parse::<i64>()
                && self.personal_copy_candidate(course_id, item_id).is_some()
                && let Some(status) = self.page_read_status(
                    PageRead::Detail(DetailRead::PersonalCopy(course_id, item_id)),
                    self.personal_copy_checks.contains_key(&(course_id, item_id)), "Checking personal-copy availability…",
                )
            {
                attachments = attachments.push(status);
            }
            body = body.push(attachments);
        }
        if item.kind == CourseworkKind::Assignment && let Ok(id) = item.id.parse::<i64>()
            && let Some(rubric) = self.assignment_rubric(course_id, id, item)
        {
            body = body.push(rubric);
        }
        let mut comments = column![text("Class comments").size(17).font(medium()).color(FG)].spacing(9);
        if let Ok(stream_item_id) = item.id.parse::<i64>() {
            if let Some(status) = self.page_read_status(
                PageRead::Detail(DetailRead::Comments(course_id, stream_item_id)),
                item.comments.is_some(), "Loading comments…",
            ) {
                comments = comments.push(status);
            }
            if self.can_compose_comment(course_id, stream_item_id)
                && let Some(status) = self.page_read_status(
                    PageRead::StreamAccess(course_id), self.stream_access.contains_key(&course_id), "Checking comment permissions…",
                )
            {
                comments = comments.push(status);
            }
            if self.can_compose_class_comment(course_id, stream_item_id) {
                let key = (course_id, stream_item_id);
                let draft = self.comment_drafts.get(&key).unwrap_or(&self.empty_comment_draft);
                let draft_text = draft.text();
                let unconfirmed = self.comment_unconfirmed.contains(&key);
                let allowed = !self.has_pending_request() && !draft_text.trim().is_empty() && !unconfirmed;
                if unconfirmed {
                    comments = comments.push(text("Check the updated comments before posting your draft again.").size(14).color(MUTED));
                }
                let editor = text_editor(draft).placeholder("Add a class comment…").height(86).padding(10);
                let editor = if self.posting_comment { editor } else {
                    editor.on_action(move |action| Message::EditComment(course_id, stream_item_id, action))
                };
                comments = comments.push(column![
                    editor,
                    row![
                        button("Mention").padding([9, 12]).style(outline_style)
                            .on_press_maybe((!self.has_pending_request())
                                .then_some(Message::OpenCommentMentions(course_id, stream_item_id))),
                        space().width(Fill),
                        button(if self.posting_comment { "Posting…" } else { "Post" })
                            .padding([9, 13]).style(primary_style)
                            .on_press_maybe(allowed.then_some(Message::PostComment(course_id, stream_item_id))),
                    ].spacing(8).align_y(Alignment::Center),
                ].spacing(8));
                if let Some(picker) = self.comment_mentions.as_ref()
                    .filter(|picker| picker.course == course_id && picker.item == stream_item_id)
                {
                    let mut list = column![
                        row![
                            text("Mention a class member").size(15).font(medium()).width(Fill),
                            button("Close").style(quiet_style).on_press(Message::CloseCommentMentions),
                        ].align_y(Alignment::Center)
                    ].spacing(6);
                    if let Some(error) = &picker.error {
                        list = list.push(text(error).size(13).color(color!(0x9b2722)));
                    } else if let Some(entries) = &picker.entries {
                        if entries.is_empty() {
                            list = list.push(text("No member email addresses are available.").size(13).color(MUTED));
                        }
                        for member in entries {
                            list = list.push(button(column![
                                text(&member.name).size(14),
                                text(&member.email).size(12).color(MUTED),
                            ].spacing(2)).width(Fill).padding(8).style(quiet_style)
                                .on_press(Message::InsertCommentMention(course_id, stream_item_id, member.id)));
                        }
                    } else {
                        list = list.push(text("Loading class members…").size(13).color(MUTED));
                    }
                    comments = comments.push(container(scrollable(list).height(200)).padding(10)
                        .style(|_| outline_surface(9.0)).width(Fill));
                }
            }
        }
        match item.comments.as_deref() {
            Some(entries) if !entries.is_empty() => {
                for comment in entries {
                    comments = comments.push(self.comment_card(course_id, item.id.parse().ok(), CommentAudience::Class, comment));
                }
            }
            Some(_) => comments = comments.push(text("No class comments returned.").size(14).color(MUTED)),
            None if !self.can_read_pages() => {
                comments = comments.push(text("Comments are unavailable in this copy.").size(14).color(MUTED));
            }
            None => {}
        }
        body = body.push(comments);
        scrollable(body.spacing(15).padding([19, 27])).width(Fill).height(Fill).into()
    }

    pub(super) fn comment_card<'a>(
        &'a self, course: i64, item: Option<i64>, audience: CommentAudience, comment: &'a Comment,
    ) -> Element<'a, Message> {
        let byline = match &comment.created_label {
            Some(label) => format!("{} · {label}", comment.author),
            None => comment.author.clone(),
        };
        let mut heading = row![text(byline).size(14).font(medium()).color(FG).width(Fill)].align_y(Alignment::Center);
        let mut confirmation = None;
        if let (Some(item), Ok(id)) = (item, comment.id.parse())
            && self.may_delete_visible_comment(course, audience, comment)
            && (self.can_compose_comment(course, item) || self.screenshot_directory.is_some())
        {
            let target = CommentTarget { course, item, id, audience };
            let ready = !self.has_pending_request();
            if self.confirming_comment_delete == Some(target) {
                confirmation = Some(column![
                    text("Delete this comment?").size(14).color(FG),
                    row![
                        button("Cancel").style(quiet_style).padding([5, 8]).on_press(Message::CancelDeleteComment),
                        button("Delete").style(outline_style).padding([5, 8])
                            .on_press_maybe(ready.then_some(Message::DeleteComment(target))),
                    ].spacing(6),
                ].spacing(6));
            } else {
                heading = heading.push(button(if self.deleting_comment == Some(target) { "Deleting…" } else { "Delete…" })
                    .style(quiet_style).padding([4, 7])
                    .on_press_maybe(ready.then_some(Message::ConfirmDeleteComment(target))));
            }
        }
        let mut body = column![heading, text(&comment.body).size(14).color(MUTED)].spacing(5);
        if let Some(confirmation) = confirmation {
            body = body.push(confirmation);
        }
        container(body).width(Fill).padding(12).style(|_| outline_surface(8.0)).into()
    }
}
