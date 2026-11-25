use super::*;

#[derive(Clone)]
pub(in crate::desktop) enum AttachmentsMessage {
    OpenDrivePicker(i64),
    OpenSubmissionDrivePicker(i64, i64),
    ToggleNewSubmissionDocument(i64, i64),
    CreateSubmissionDocument(i64, i64, DriveDocumentKind),
    SubmissionDocumentCreated(i64, i64, Result<String, String>),
    SubmissionDriveFileAttached(i64, Result<(), String>),
    CloseDrivePicker,
    EditDriveSearch(i64, String),
    BrowseDrive(i64, DriveNavigation),
    DriveScrolled(i64, u64, bool),
    SearchDriveFiles(i64, bool),
    DriveFilesLoaded(i64, u64, bool, Result<DriveFilePage, String>),
    AttachExistingDriveFile(i64, String),
    OpenAttachment(String),
    OpenDriveFile(String),
    PreviewDriveFile(desktop_data::Attachment),
    DrivePreviewReady(Result<(Arc<tempfile::TempDir>, PathBuf), String>),
    DrivePreviewClosed(Result<(), String>),
    SaveDriveFile(desktop_data::Attachment),
    DriveFileSaved(Result<bool, String>),
    DriveFileLinkLoaded(String, Result<String, String>),
}

impl App {
    pub(super) fn update_attachments(&mut self, message: AttachmentsMessage) -> Task<Message> {
        match message {
            AttachmentsMessage::OpenDrivePicker(id) if self.can_edit_stream_attachments(id) => {
                self.attachments.drive_picker = Some(DrivePicker { course: id, ..Default::default() });
                return self.update(Message::Attachments(AttachmentsMessage::SearchDriveFiles(id, false)));
            }
            AttachmentsMessage::OpenSubmissionDrivePicker(course, item) if self.can_add_submission_drive_file(course, item)
                && !self.has_pending_request() =>
            {
                self.attachments.new_submission_document_open = false;
                self.attachments.drive_picker = Some(DrivePicker { course, item: Some(item), ..Default::default() });
                return self.update(Message::Attachments(AttachmentsMessage::SearchDriveFiles(course, false)));
            }
            AttachmentsMessage::ToggleNewSubmissionDocument(course, item)
                if self.can_add_submission_drive_file(course, item) && !self.has_pending_request() =>
            {
                self.attachments.new_submission_document_open = !self.attachments.new_submission_document_open;
                self.attachments.drive_picker = None;
            }
            AttachmentsMessage::CreateSubmissionDocument(course, item, kind)
                if self.attachments.new_submission_document_open && !self.has_pending_request()
                    && self.can_add_submission_drive_file(course, item) =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                let key = item.to_string();
                let Some(work) = self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(course))
                    .and_then(|items| items.iter().find(|entry| entry.id == key)) else { return Task::none() };
                let title = work.title.clone();
                self.attachments.new_submission_document_open = false;
                self.student_work.sending_submission = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::create_submission_document(&session, course, item, title, kind).await
                }, move |result| Message::Attachments(AttachmentsMessage::SubmissionDocumentCreated(course, item, result)));
            }
            AttachmentsMessage::SubmissionDocumentCreated(course, item, result) => {
                self.student_work.sending_submission = false;
                let open = match result {
                    Ok(url) => {
                        self.notice = Some("New file attached to your work.".into());
                        if self.screen == Screen::Item(course)
                            && self.item_id.as_deref().and_then(|id| id.parse::<i64>().ok()) == Some(item)
                        {
                            self.update(Message::Attachments(AttachmentsMessage::OpenAttachment(url)))
                        } else {
                            Task::none()
                        }
                    }
                    Err(error) => {
                        self.error = Some(error);
                        Task::none()
                    }
                };
                return Task::batch([open, self.update(Message::Reads(ReadsMessage::LoadDetail(DetailRead::Submissions(course))))]);
            }
            AttachmentsMessage::CloseDrivePicker => self.attachments.drive_picker = None,
            AttachmentsMessage::EditDriveSearch(id, query) if query.len() <= 1024 => {
                if let Some(picker) = self.attachments.drive_picker.as_mut().filter(|picker| picker.course == id) {
                    picker.query = query;
                }
            }
            AttachmentsMessage::BrowseDrive(id, navigation) if self.can_use_drive_picker(id) => {
                if let Some(picker) = self.attachments.drive_picker.as_mut().filter(|picker| picker.course == id)
                    && picker.browse(navigation)
                {
                    return self.update(Message::Attachments(AttachmentsMessage::SearchDriveFiles(id, false)));
                }
            }
            AttachmentsMessage::DriveScrolled(id, generation, true) if generation == self.attachments.drive_picker_generation
                && self.attachments.drive_picker.as_ref().is_some_and(|picker| picker.course == id && picker.can_auto_page()) =>
            {
                return self.update(Message::Attachments(AttachmentsMessage::SearchDriveFiles(id, true)));
            }
            AttachmentsMessage::SearchDriveFiles(id, append) if self.can_use_drive_picker(id) => {
                let Some(session) = self.session.clone() else { return Task::none() };
                if let Some(picker) = self.attachments.drive_picker.as_mut().filter(|picker| picker.course == id) {
                    let query = picker.query.trim().to_owned();
                    let page_token = if append {
                        if picker.loading || query != picker.submitted_query {
                            return Task::none();
                        }
                        let Some(token) = picker.file_page.as_ref().and_then(|page| page.next_page.clone()) else {
                            return Task::none();
                        };
                        Some(token)
                    } else {
                        picker.file_page = None;
                        picker.listed_tokens.clear();
                        None
                    };
                    self.attachments.drive_picker_generation = self.attachments.drive_picker_generation.wrapping_add(1);
                    let generation = self.attachments.drive_picker_generation;
                    picker.loading = true;
                    picker.error = None;
                    picker.submitted_query = query.clone();
                    let location = picker.location();
                    return Task::perform(async move { desktop_auth::list_drive_files(&session, location, query, page_token).await },
                        move |result| Message::Attachments(AttachmentsMessage::DriveFilesLoaded(id, generation, append, result)));
                }
            }
            AttachmentsMessage::DriveFilesLoaded(id, generation, append, result) if generation == self.attachments.drive_picker_generation => {
                if let Some(picker) = self.attachments.drive_picker.as_mut().filter(|picker| picker.course == id) {
                    picker.loading = false;
                    match result {
                        Ok(page) => picker.accept_page(page, append),
                        Err(error) => picker.error = Some((append, error)),
                    }
                    if picker.needs_more_for_viewport() {
                        return self.update(Message::Attachments(AttachmentsMessage::SearchDriveFiles(id, true)));
                    }
                }
            }
            AttachmentsMessage::AttachExistingDriveFile(id, file_id) if self.can_use_drive_picker(id) && !self.has_pending_request() => {
                if let Some(picker) = self.attachments.drive_picker.as_ref().filter(|picker| picker.course == id)
                    && let Some(file) = picker.file_page.as_ref()
                    .and_then(|page| page.files.iter().find(|file| file.id() == file_id))
                {
                    if let Some(item) = picker.item {
                        let Some(session) = self.session.clone() else { return Task::none() };
                        let file = file.clone();
                        self.student_work.sending_submission = true;
                        self.error = None;
                        self.notice = None;
                        return Task::perform(async move { desktop_auth::attach_submission_drive_file(&session, id, item, file).await },
                            move |result| Message::Attachments(AttachmentsMessage::SubmissionDriveFileAttached(id, result)));
                    }
                    if let Some(draft) = self.stream.stream_drafts.get_mut(&id)
                        && !draft.files.iter().any(|existing| existing.id() == file_id)
                    {
                        draft.files.push(file.clone());
                    }
                }
            }
            AttachmentsMessage::SubmissionDriveFileAttached(course, result) => {
                self.student_work.sending_submission = false;
                match result {
                    Ok(()) => {
                        self.notice = Some("Drive file attached to your work.".into());
                        return self.update(Message::Reads(ReadsMessage::LoadDetail(DetailRead::Submissions(course))));
                    }
                    Err(error) => {
                        self.error = Some(error);
                        let refresh = self.update(Message::Reads(ReadsMessage::LoadDetail(DetailRead::Submissions(course))));
                        return refresh;
                    }
                }
            }
            AttachmentsMessage::OpenAttachment(url) if !self.verify_live_course_count => {
                if trusted_attachment_host(&url).is_none() {
                    self.error = Some("This attachment does not have a supported secure link.".into());
                } else {
                    return Task::perform(async move {
                        tokio::process::Command::new("/usr/bin/open")
                            .arg(url)
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .status()
                            .await
                            .is_ok_and(|status| status.success())
                    }, |opened| {
                        if opened { Message::Reads(ReadsMessage::AttachmentOpened) } else { Message::Reads(ReadsMessage::AttachmentFailed) }
                    });
                }
            }
            AttachmentsMessage::OpenDriveFile(id) if self.can_read_pages() && self.attachments.opening_drive_file.is_none() => {
                let Some(session) = self.session.clone() else { return Task::none(); };
                self.error = None;
                self.attachments.opening_drive_file = Some(id.clone());
                return Task::perform(async move {
                    let result = desktop_auth::lookup_drive_file_link(&session, &id).await;
                    (id, result)
                }, |(id, result)| Message::Attachments(AttachmentsMessage::DriveFileLinkLoaded(id, result)));
            }
            #[cfg(target_os = "macos")]
            AttachmentsMessage::PreviewDriveFile(attachment) if self.can_read_pages() && !self.attachments.preparing_drive_preview => {
                let (Some(session), Some(id)) = (self.session.clone(), attachment.drive_reference) else {
                    return Task::none();
                };
                self.attachments.preparing_drive_preview = true;
                self.error = None;
                self.notice = Some("Preparing Quick Look preview…".into());
                return Task::perform(async move {
                    let directory = Arc::new(tempfile::Builder::new().prefix("classroom-preview-").tempdir()
                        .map_err(|_| "Could not create a temporary preview folder.")?);
                    let path = desktop_auth::preview_drive_file(&session, &id, attachment.url.as_deref(), directory.path()).await?;
                    Ok((directory, path))
                }, |value| Message::Attachments(AttachmentsMessage::DrivePreviewReady(value)));
            }
            #[cfg(target_os = "macos")]
            AttachmentsMessage::DrivePreviewReady(result) => {
                self.attachments.preparing_drive_preview = false;
                self.notice = None;
                match result {
                    Ok((directory, path)) => {
                        return Task::perform(async move {
                            let status = tokio::process::Command::new("/usr/bin/qlmanage")
                                .arg("-p").arg(path)
                                .stdout(std::process::Stdio::null())
                                .stderr(std::process::Stdio::null())
                                .kill_on_drop(true)
                                .status().await;
                            drop(directory);
                            if status.is_ok_and(|status| status.success()) {
                                Ok(())
                            } else {
                                Err("Quick Look couldn’t preview this file.".into())
                            }
                        }, |value| Message::Attachments(AttachmentsMessage::DrivePreviewClosed(value)));
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            AttachmentsMessage::DrivePreviewClosed(Err(error)) => self.error = Some(error),
            AttachmentsMessage::DrivePreviewClosed(Ok(())) => {}
            #[cfg(target_os = "macos")]
            AttachmentsMessage::SaveDriveFile(attachment) if self.can_read_pages() && !self.has_pending_request() => {
                let (Some(session), Some(id)) = (self.session.clone(), attachment.drive_reference.clone()) else {
                    return Task::none();
                };
                self.attachments.saving_drive_file = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    let suggested: String = attachment.title.chars().map(|ch| match ch {
                        '/' | ':' | '\\' => '-',
                        ch if ch.is_control() => ' ',
                        ch => ch,
                    }).take(160).collect();
                    let suggested = suggested.trim_matches(|ch: char| ch.is_whitespace() || ch == '.');
                    let suggested = if suggested.is_empty() { "Classroom file" } else { suggested };
                    let Some(path) = grade_export::choose_file_destination(suggested.into(), "Save a copy").await? else {
                        return Ok(false);
                    };
                    desktop_auth::save_drive_file(&session, &id, attachment.url.as_deref(), &path).await?;
                    Ok(true)
                }, |value| Message::Attachments(AttachmentsMessage::DriveFileSaved(value)));
            }
            AttachmentsMessage::DriveFileSaved(result) => {
                self.attachments.saving_drive_file = false;
                match result {
                    Ok(true) => self.notice = Some("File saved.".into()),
                    Ok(false) => {}
                    Err(error) => self.error = Some(error),
                }
            }
            AttachmentsMessage::DriveFileLinkLoaded(id, result) if self.attachments.opening_drive_file.as_ref() == Some(&id) => {
                self.attachments.opening_drive_file = None;
                match result {
                    Ok(url) => return self.update(Message::Attachments(AttachmentsMessage::OpenAttachment(url))),
                    Err(error) => self.error = Some(error),
                }
            }
            _ => {}
        }
        self.finish_update()
    }
}
