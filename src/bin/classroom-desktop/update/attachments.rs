use super::*;

impl App {
    pub(super) fn update_attachments(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::OpenDrivePicker(id) if self.can_edit_stream_attachments(id) => {
                self.drive_picker = Some(DrivePicker { course: id, ..Default::default() });
                return self.update(Message::SearchDriveFiles(id, false));
            }
            Message::OpenSubmissionDrivePicker(course, item) if self.can_add_submission_drive_file(course, item)
                && !self.has_pending_request() =>
            {
                self.new_submission_document_open = false;
                self.drive_picker = Some(DrivePicker { course, item: Some(item), ..Default::default() });
                return self.update(Message::SearchDriveFiles(course, false));
            }
            Message::ToggleNewSubmissionDocument(course, item)
                if self.can_add_submission_drive_file(course, item) && !self.has_pending_request() =>
            {
                self.new_submission_document_open = !self.new_submission_document_open;
                self.drive_picker = None;
            }
            Message::CreateSubmissionDocument(course, item, kind)
                if self.new_submission_document_open && !self.has_pending_request()
                    && self.can_add_submission_drive_file(course, item) =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                let key = item.to_string();
                let Some(work) = self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(course))
                    .and_then(|items| items.iter().find(|entry| entry.id == key)) else { return Task::none() };
                let title = work.title.clone();
                self.new_submission_document_open = false;
                self.sending_submission = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::create_submission_document(&session, course, item, title, kind).await
                }, move |result| Message::SubmissionDocumentCreated(course, item, result));
            }
            Message::SubmissionDocumentCreated(course, item, result) => {
                self.sending_submission = false;
                let open = match result {
                    Ok(url) => {
                        self.notice = Some("New file attached to your work.".into());
                        if self.screen == Screen::Item(course)
                            && self.item_id.as_deref().and_then(|id| id.parse::<i64>().ok()) == Some(item)
                        {
                            self.update(Message::OpenAttachment(url))
                        } else {
                            Task::none()
                        }
                    }
                    Err(error) => {
                        self.error = Some(error);
                        Task::none()
                    }
                };
                return Task::batch([open, self.update(Message::LoadDetail(DetailRead::Submissions(course)))]);
            }
            Message::CloseDrivePicker => self.drive_picker = None,
            Message::EditDriveSearch(id, query) if query.len() <= 1024 => {
                if let Some(picker) = self.drive_picker.as_mut().filter(|picker| picker.course == id) {
                    picker.query = query;
                }
            }
            Message::BrowseDrive(id, navigation) if self.can_use_drive_picker(id) => {
                if let Some(picker) = self.drive_picker.as_mut().filter(|picker| picker.course == id)
                    && picker.browse(navigation)
                {
                    return self.update(Message::SearchDriveFiles(id, false));
                }
            }
            Message::DriveScrolled(id, generation, true) if generation == self.drive_picker_generation
                && self.drive_picker.as_ref().is_some_and(|picker| picker.course == id && picker.can_auto_page()) =>
            {
                return self.update(Message::SearchDriveFiles(id, true));
            }
            Message::SearchDriveFiles(id, append) if self.can_use_drive_picker(id) => {
                let Some(session) = self.session.clone() else { return Task::none() };
                if let Some(picker) = self.drive_picker.as_mut().filter(|picker| picker.course == id) {
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
                    self.drive_picker_generation = self.drive_picker_generation.wrapping_add(1);
                    let generation = self.drive_picker_generation;
                    picker.loading = true;
                    picker.error = None;
                    picker.submitted_query = query.clone();
                    let location = picker.location();
                    return Task::perform(async move { desktop_auth::list_drive_files(&session, location, query, page_token).await },
                        move |result| Message::DriveFilesLoaded(id, generation, append, result));
                }
            }
            Message::DriveFilesLoaded(id, generation, append, result) if generation == self.drive_picker_generation => {
                if let Some(picker) = self.drive_picker.as_mut().filter(|picker| picker.course == id) {
                    picker.loading = false;
                    match result {
                        Ok(page) => picker.accept_page(page, append),
                        Err(error) => picker.error = Some((append, error)),
                    }
                    if picker.needs_more_for_viewport() {
                        return self.update(Message::SearchDriveFiles(id, true));
                    }
                }
            }
            Message::AttachExistingDriveFile(id, file_id) if self.can_use_drive_picker(id) && !self.has_pending_request() => {
                if let Some(picker) = self.drive_picker.as_ref().filter(|picker| picker.course == id)
                    && let Some(file) = picker.file_page.as_ref()
                    .and_then(|page| page.files.iter().find(|file| file.id() == file_id))
                {
                    if let Some(item) = picker.item {
                        let Some(session) = self.session.clone() else { return Task::none() };
                        let file = file.clone();
                        self.sending_submission = true;
                        self.error = None;
                        self.notice = None;
                        return Task::perform(async move { desktop_auth::attach_submission_drive_file(&session, id, item, file).await },
                            move |result| Message::SubmissionDriveFileAttached(id, result));
                    }
                    if let Some(draft) = self.stream_drafts.get_mut(&id)
                        && !draft.files.iter().any(|existing| existing.id() == file_id)
                    {
                        draft.files.push(file.clone());
                    }
                }
            }
            Message::SubmissionDriveFileAttached(course, result) => {
                self.sending_submission = false;
                match result {
                    Ok(()) => {
                        self.notice = Some("Drive file attached to your work.".into());
                        return self.update(Message::LoadDetail(DetailRead::Submissions(course)));
                    }
                    Err(error) => {
                        self.error = Some(error);
                        let refresh = self.update(Message::LoadDetail(DetailRead::Submissions(course)));
                        return refresh;
                    }
                }
            }
            Message::OpenAttachment(url) if !self.verify_live_course_count => {
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
                        if opened { Message::AttachmentOpened } else { Message::AttachmentFailed }
                    });
                }
            }
            Message::OpenDriveFile(id) if self.can_read_pages() && self.opening_drive_file.is_none() => {
                let Some(session) = self.session.clone() else { return Task::none(); };
                self.error = None;
                self.opening_drive_file = Some(id.clone());
                return Task::perform(async move {
                    let result = desktop_auth::lookup_drive_file_link(&session, &id).await;
                    (id, result)
                }, |(id, result)| Message::DriveFileLinkLoaded(id, result));
            }
            #[cfg(target_os = "macos")]
            Message::PreviewDriveFile(attachment) if self.can_read_pages() && !self.preparing_drive_preview => {
                let (Some(session), Some(id)) = (self.session.clone(), attachment.drive_reference) else {
                    return Task::none();
                };
                self.preparing_drive_preview = true;
                self.error = None;
                self.notice = Some("Preparing Quick Look preview…".into());
                return Task::perform(async move {
                    let directory = Arc::new(tempfile::Builder::new().prefix("classroom-preview-").tempdir()
                        .map_err(|_| "Could not create a temporary preview folder.")?);
                    let path = desktop_auth::preview_drive_file(&session, &id, attachment.url.as_deref(), directory.path()).await?;
                    Ok((directory, path))
                }, Message::DrivePreviewReady);
            }
            #[cfg(target_os = "macos")]
            Message::DrivePreviewReady(result) => {
                self.preparing_drive_preview = false;
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
                        }, Message::DrivePreviewClosed);
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Message::DrivePreviewClosed(Err(error)) => self.error = Some(error),
            Message::DrivePreviewClosed(Ok(())) => {}
            #[cfg(target_os = "macos")]
            Message::SaveDriveFile(attachment) if self.can_read_pages() && !self.has_pending_request() => {
                let (Some(session), Some(id)) = (self.session.clone(), attachment.drive_reference.clone()) else {
                    return Task::none();
                };
                self.saving_drive_file = true;
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
                }, Message::DriveFileSaved);
            }
            Message::DriveFileSaved(result) => {
                self.saving_drive_file = false;
                match result {
                    Ok(true) => self.notice = Some("File saved.".into()),
                    Ok(false) => {}
                    Err(error) => self.error = Some(error),
                }
            }
            Message::DriveFileLinkLoaded(id, result) if self.opening_drive_file.as_ref() == Some(&id) => {
                self.opening_drive_file = None;
                match result {
                    Ok(url) => return self.update(Message::OpenAttachment(url)),
                    Err(error) => self.error = Some(error),
                }
            }
            _ => {}
        }
        self.finish_update()
    }
}
