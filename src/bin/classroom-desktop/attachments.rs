use super::*;
use iced::widget::column;

impl App {
    pub(super) fn can_edit_stream_attachments(&self, course: i64) -> bool {
        self.stream.open_stream_composer == Some(course) && self.can_post_stream(course)
            && self.stream.sending_stream_post != Some(course) && self.stream.uploading_stream_file != Some(course)
    }

    pub(super) fn can_add_submission_drive_file(&self, course: i64, item: i64) -> bool {
        self.can_compose_comment(course, item) && self.student_work.submissions_checked.contains(&course)
            && !self.page_errors.contains_key(&PageRead::Detail(DetailRead::Submissions(course)))
            && self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(course))
                .and_then(|items| items.iter().find(|entry| entry.id.parse::<i64>().ok() == Some(item)))
                .is_some_and(|entry| entry.kind == CourseworkKind::Assignment
                    && entry.submission.as_ref().and_then(desktop_auth::SubmissionAction::for_submission)
                        == Some(desktop_auth::SubmissionAction::TurnIn))
    }

    pub(super) fn validate_file_for_staging(&mut self, course: i64, item: i64, path: PathBuf) -> Task<Message> {
        self.attachments.choosing_submission_file = true;
        let checked_path = path.clone();
        Task::perform(async move {
            tokio::task::spawn_blocking(move || validate_submission_file(&checked_path)).await
                .map_err(|_| "Could not check the selected file.".to_string())?
        }, move |result| Message::Submissions(SubmissionsMessage::SubmissionFileValidated(course, item, path.clone(), result)))
    }

    pub(super) fn stage_submission_file(&mut self, course: i64, item: i64, path: PathBuf, result: Result<(), String>) {
        match result {
            Ok(()) => {
                self.attachments.submission_file = Some((course, item, path));
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub(super) fn discard_submission_capture(&mut self) {
        #[cfg(target_os = "macos")]
        {
            self.attachments.capture_generation = self.attachments.capture_generation.wrapping_add(1);
            self.attachments.submission_capture = None;
        }
    }

    #[cfg(target_os = "macos")]
    fn capture_target(&self) -> Option<(i64, i64)> {
        let Screen::Item(course) = self.screen else { return None };
        let item = self.item_id.as_deref()?.parse().ok()?;
        self.can_add_submission_drive_file(course, item).then_some((course, item))
    }

    #[cfg(target_os = "macos")]
    pub(super) fn update_capture(&mut self, action: CaptureAction) -> Task<Message> {
        match action {
            CaptureAction::StartScreen if self.attachments.submission_capture.is_none() && self.capture_target().is_some()
                && !self.has_pending_request() =>
            {
                let generation = self.attachments.capture_generation;
                let (task, handle) = Task::perform(screen_recording::capture(),
                    move |result| Message::Navigation(NavigationMessage::Capture(CaptureAction::MovieCaptured(generation, "Screen recording", result)))).abortable();
                self.attachments.submission_capture = Some(CaptureState::RecordingScreen { _handle: handle.abort_on_drop() });
                self.error = None;
                self.notice = None;
                return task;
            }
            CaptureAction::StartCamera if self.attachments.submission_capture.is_none() && self.capture_target().is_some()
                && !self.has_pending_request() =>
            {
                let generation = self.attachments.capture_generation;
                self.attachments.submission_capture = Some(CaptureState::AskingCamera);
                self.error = None;
                self.notice = None;
                let permission = camera::request_permission();
                return Task::perform(async move { permission.await.unwrap_or(false) },
                    move |allowed| Message::Navigation(NavigationMessage::Capture(CaptureAction::CameraPermission(generation, allowed))));
            }
            CaptureAction::CameraPermission(generation, allowed) if generation == self.attachments.capture_generation
                && matches!(self.attachments.submission_capture, Some(CaptureState::AskingCamera)) =>
            {
                if !allowed || self.capture_target().is_none() {
                    self.attachments.submission_capture = None;
                    if !allowed {
                        self.error = Some("Camera access is off. Allow Classroom for Mac in System Settings under Privacy & Security → Camera.".into());
                    }
                    return Task::none();
                }
                self.attachments.submission_capture = None;
                match camera::start() {
                    Ok((control, result)) => {
                        self.attachments.submission_capture = Some(CaptureState::RecordingCamera { control, phase: CameraPhase::Framing, frame: None });
                        return Task::perform(async move { result.await.unwrap_or_else(|_| Err("The camera worker stopped unexpectedly.".into())) },
                            move |result| Message::Navigation(NavigationMessage::Capture(CaptureAction::MovieCaptured(generation, "Camera recording", result))));
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            CaptureAction::CameraFrame => {
                if let Some(CaptureState::RecordingCamera { control, frame, .. }) = &mut self.attachments.submission_capture
                    && let Some((width, height, pixels)) = control.take_frame()
                {
                    *frame = Some(image::Handle::from_rgba(width, height, pixels));
                }
            }
            CaptureAction::StartCameraRecording if self.capture_target().is_some() => {
                if let Some(CaptureState::RecordingCamera { phase, frame: Some(_), .. }) = &mut self.attachments.submission_capture
                    && *phase == CameraPhase::Framing
                {
                    *phase = CameraPhase::AskingMicrophone;
                    self.error = None;
                    let generation = self.attachments.capture_generation;
                    let permission = audio::request_permission();
                    return Task::perform(async move { permission.await.unwrap_or(false) },
                        move |allowed| Message::Navigation(NavigationMessage::Capture(CaptureAction::CameraMicrophonePermission(generation, allowed))));
                }
            }
            CaptureAction::CameraMicrophonePermission(generation, allowed) if generation == self.attachments.capture_generation => {
                if let Some(CaptureState::RecordingCamera { control, phase, .. }) = &mut self.attachments.submission_capture
                    && *phase == CameraPhase::AskingMicrophone
                {
                    if allowed {
                        *phase = CameraPhase::Recording;
                        control.begin();
                    } else {
                        *phase = CameraPhase::Framing;
                        self.error = Some("Microphone access is off. Allow Classroom for Mac in System Settings under Privacy & Security → Microphone.".into());
                    }
                }
            }
            CaptureAction::StopCamera => {
                if let Some(CaptureState::RecordingCamera { control, phase, .. }) = &mut self.attachments.submission_capture
                    && *phase == CameraPhase::Recording
                {
                    *phase = CameraPhase::Finishing;
                    control.stop();
                }
            }
            CaptureAction::MovieCaptured(generation, label, result) if generation == self.attachments.capture_generation
                && matches!(self.attachments.submission_capture, Some(CaptureState::RecordingScreen { .. } | CaptureState::RecordingCamera { .. })) =>
            {
                self.attachments.submission_capture = None;
                match result {
                    Ok(Some(draft)) => self.attachments.submission_capture = Some(CaptureState::MovieDraft { label, draft }),
                    Ok(None) => {}
                    Err(error) => self.error = Some(error),
                }
            }
            CaptureAction::Start if self.attachments.submission_capture.is_none() && self.capture_target().is_some()
                && !self.has_pending_request() =>
            {
                let permission = audio::request_permission();
                let generation = self.attachments.capture_generation;
                self.attachments.submission_capture = Some(CaptureState::AskingPermission);
                self.error = None;
                self.notice = None;
                return Task::perform(async move { permission.await.unwrap_or(false) },
                    move |allowed| Message::Navigation(NavigationMessage::Capture(CaptureAction::Permission(generation, allowed))));
            }
            CaptureAction::Permission(generation, allowed) if generation == self.attachments.capture_generation
                && matches!(self.attachments.submission_capture, Some(CaptureState::AskingPermission)) =>
            {
                self.attachments.submission_capture = None;
                if self.capture_target().is_none() { return Task::none(); }
                if !allowed {
                    self.error = Some("Microphone access is off. Allow Classroom for Mac in System Settings under Privacy & Security → Microphone.".into());
                    return Task::none();
                }
                match audio::Recording::start() {
                    Ok(recording) => {
                        let (limit, handle) = Task::perform(tokio::time::sleep(audio::MAX_DURATION),
                            move |()| Message::Navigation(NavigationMessage::Capture(CaptureAction::TimeLimit(generation)))).abortable();
                        self.attachments.submission_capture = Some(CaptureState::Recording(recording, handle.abort_on_drop()));
                        return limit;
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            CaptureAction::TimeLimit(generation) if generation == self.attachments.capture_generation => {
                return self.update_capture(CaptureAction::Stop);
            }
            CaptureAction::Stop if matches!(self.attachments.submission_capture, Some(CaptureState::Recording(..))) => {
                if let Some(CaptureState::Recording(recording, _limit)) = self.attachments.submission_capture.take() {
                    match recording.finish() {
                        Ok(draft) => self.attachments.submission_capture = Some(CaptureState::Draft(draft)),
                        Err(error) => self.error = Some(error),
                    }
                }
            }
            CaptureAction::Preview if !self.student_work.sending_submission => {
                if let Some(CaptureState::MovieDraft { draft, .. }) = &self.attachments.submission_capture {
                    let generation = self.attachments.capture_generation;
                    return Task::perform(draft.clone().preview(),
                        move |result| Message::Navigation(NavigationMessage::Capture(CaptureAction::ScreenPreviewFinished(generation, result))));
                }
                if let Some(CaptureState::Draft(draft)) = &self.attachments.submission_capture
                    && let Err(error) = draft.play_preview()
                {
                    self.error = Some(error);
                }
            }
            CaptureAction::ScreenPreviewFinished(generation, Err(error)) if generation == self.attachments.capture_generation => {
                self.error = Some(error);
            }
            CaptureAction::StopPreview => {
                if let Some(CaptureState::Draft(draft)) = &self.attachments.submission_capture {
                    draft.stop_preview();
                }
            }
            CaptureAction::Discard if !self.student_work.sending_submission => self.discard_submission_capture(),
            CaptureAction::Attach if !self.has_pending_request() => {
                if let (Some((course, item)), Some((keep_alive, path)), Some(session)) =
                    (self.capture_target(), self.attachments.submission_capture.as_ref().and_then(CaptureState::recording_file),
                        self.session.clone())
                {
                    let generation = self.attachments.capture_generation;
                    self.student_work.sending_submission = true;
                    self.error = None;
                    self.notice = None;
                    return Task::perform(async move {
                        let result = desktop_auth::attach_submission_file(&session, course, item, path).await;
                        drop(keep_alive);
                        result
                    }, move |result| Message::Navigation(NavigationMessage::Capture(CaptureAction::Attached(course, generation, result))));
                }
            }
            CaptureAction::Attached(course, generation, result) => {
                self.student_work.sending_submission = false;
                match result {
                    Ok(()) => {
                        if self.attachments.capture_generation == generation {
                            self.discard_submission_capture();
                        }
                        self.notice = Some("Recording attached to your work.".into());
                    }
                    Err(error) => self.error = Some(error),
                }
                return self.update(Message::Reads(ReadsMessage::LoadDetail(DetailRead::Submissions(course))));
            }
            _ => {}
        }
        Task::none()
    }

    #[cfg(target_os = "macos")]
    pub(super) fn submission_capture_controls(&self, ready: bool) -> Element<'_, Message> {
        let capture_button = |label: &'static str, action, enabled: bool| {
            button(label).style(outline_style).padding([7, 10])
                .on_press_maybe(enabled.then_some(Message::Navigation(NavigationMessage::Capture(action))))
        };
        let discard = capture_button("Discard", CaptureAction::Discard, !self.student_work.sending_submission);
        match &self.attachments.submission_capture {
            None => row![
                capture_button("Record audio", CaptureAction::Start, ready),
                capture_button("Record video", CaptureAction::StartCamera, ready),
                capture_button("Record screen", CaptureAction::StartScreen, ready),
            ].spacing(8).wrap().vertical_spacing(8).into(),
            Some(CaptureState::AskingPermission) => row![
                text("Waiting for microphone access…").size(14).style(move |theme| themed_text(theme, MUTED)), discard,
            ].spacing(8).align_y(Alignment::Center).into(),
            Some(CaptureState::AskingCamera) => row![
                text("Waiting for camera access…").size(14).style(move |theme| themed_text(theme, MUTED)), discard,
            ].spacing(8).align_y(Alignment::Center).wrap().vertical_spacing(8).into(),
            Some(CaptureState::Recording(..)) => row![
                text("Recording audio · 5 min limit").size(14).style(move |theme| themed_text(theme, FG)),
                capture_button("Stop recording", CaptureAction::Stop, true), discard,
            ].spacing(8).align_y(Alignment::Center).wrap().vertical_spacing(8).into(),
            Some(CaptureState::RecordingScreen { .. }) => row![
                text("Select an area in the macOS toolbar. Use the menu bar to stop.").size(14).style(move |theme| themed_text(theme, FG)),
                discard,
            ].spacing(8).align_y(Alignment::Center).wrap().vertical_spacing(8).into(),
            Some(CaptureState::RecordingCamera { phase, frame, .. }) => {
                let preview: Element<'_, Message> = match frame {
                    Some(frame) => image(frame.clone()).width(Fill).height(220).content_fit(ContentFit::Contain).into(),
                    None => container(text("Opening camera…").size(14)).center_x(Fill).center_y(220).into(),
                };
                let (status, action) = match phase {
                    CameraPhase::Framing => ("Camera preview", capture_button("Start recording", CaptureAction::StartCameraRecording, frame.is_some())),
                    CameraPhase::AskingMicrophone => ("Waiting for microphone access…", capture_button("Start recording", CaptureAction::StartCameraRecording, false)),
                    CameraPhase::Recording => ("Recording video and audio · 5 min limit", capture_button("Stop recording", CaptureAction::StopCamera, true)),
                    CameraPhase::Finishing => ("Saving camera recording…", capture_button("Stop recording", CaptureAction::StopCamera, false)),
                };
                column![
                    preview,
                    row![text(status).size(14), action, discard].spacing(8).align_y(Alignment::Center).wrap().vertical_spacing(8),
                ].spacing(6).into()
            }
            Some(CaptureState::Draft(draft)) => {
                let seconds = draft.duration.as_secs();
                column![
                    text(format!("Audio recording · {}:{:02}", seconds / 60, seconds % 60)).size(14),
                    row![
                        capture_button("Play preview", CaptureAction::Preview, !self.student_work.sending_submission),
                        capture_button("Stop preview", CaptureAction::StopPreview, draft.is_playing()),
                        capture_button("Attach recording", CaptureAction::Attach, ready),
                        discard,
                    ].spacing(8).wrap().vertical_spacing(8),
                ].spacing(6).into()
            }
            Some(CaptureState::MovieDraft { label, .. }) => column![
                text(*label).size(14),
                row![
                    capture_button("Preview in QuickTime", CaptureAction::Preview, !self.student_work.sending_submission),
                    capture_button("Attach recording", CaptureAction::Attach, ready),
                    discard,
                ].spacing(8).wrap().vertical_spacing(8),
            ].spacing(6).into(),
        }
    }

    pub(super) fn can_use_drive_picker(&self, course: i64) -> bool {
        self.attachments.drive_picker.as_ref().is_some_and(|picker| {
            picker.course == course && match picker.item {
                Some(item) => self.can_add_submission_drive_file(course, item),
                None => self.can_edit_stream_attachments(course),
            }
        })
    }

    pub(super) fn drive_picker_panel<'a>(
        &'a self, picker: &'a DrivePicker, ready: bool, already_attached: impl Fn(&DriveFileAttachment) -> bool,
    ) -> Element<'a, Message> {
        let course = picker.course;
        let placeholder = if !picker.folder_path.is_empty() || picker.section == DriveSection::MyDrive {
            "Search this folder"
        } else {
            "Search Drive by name"
        };
        let search = text_input(placeholder, &picker.query).width(Fill).padding(7)
            .on_input(move |query| Message::Attachments(AttachmentsMessage::EditDriveSearch(course, query)))
            .on_submit_maybe(ready.then_some(Message::Attachments(AttachmentsMessage::SearchDriveFiles(course, false))));
        let mut panel = column![
            row![
                text("Google Drive").size(15).font(medium()).style(move |theme| themed_text(theme, FG)).width(Fill),
                button("Close").style(quiet_style).padding([4, 7]).on_press(Message::Attachments(AttachmentsMessage::CloseDrivePicker)),
            ].align_y(Alignment::Center),
            row([DriveSection::Recent, DriveSection::MyDrive, DriveSection::Shared].into_iter().map(|section| {
                let selected = picker.section == section;
                button(section.label()).padding([4, 7]).style(move |theme, status| nav_style(theme, selected, status))
                    .on_press_maybe((ready && (!selected || !picker.folder_path.is_empty()))
                        .then_some(Message::Attachments(AttachmentsMessage::BrowseDrive(course, DriveNavigation::Section(section))))).into()
            })).spacing(5).wrap().vertical_spacing(5),
            row![
                search,
                button("Search").style(outline_style).padding([6, 8])
                    .on_press_maybe(ready.then_some(Message::Attachments(AttachmentsMessage::SearchDriveFiles(course, false)))),
            ].spacing(6).align_y(Alignment::Center),
        ].spacing(7);
        if let Some(folder) = picker.folder_path.last() {
            panel = panel.push(row![
                button("‹ Back").style(quiet_style).padding([4, 6])
                    .on_press_maybe(ready.then_some(Message::Attachments(AttachmentsMessage::BrowseDrive(course, DriveNavigation::Back)))),
                text(truncate(&folder.name, 55)).size(14).font(medium()).style(move |theme| themed_text(theme, FG)).width(Fill),
            ].spacing(6).align_y(Alignment::Center));
        }
        if let Some((append, error)) = &picker.error {
            panel = panel.push(row![
                text(error).size(13).style(move |theme| themed_text(theme, color!(0x9b2722))).width(Fill),
                button("Retry").style(quiet_style).padding([4, 7])
                    .on_press_maybe((ready && !picker.loading && picker.query.trim() == picker.submitted_query)
                        .then_some(Message::Attachments(AttachmentsMessage::SearchDriveFiles(course, *append)))),
            ].spacing(7).align_y(Alignment::Center));
        }
        if let Some(page) = &picker.file_page {
            let mut rows = column![].spacing(4);
            for folder in &page.folders {
                rows = rows.push(button(row![
                    icon(Icon::Folder, BLUE, 16),
                    text(&folder.name).size(14).style(move |theme| themed_text(theme, FG)).width(Fill).wrapping(text::Wrapping::WordOrGlyph),
                    icon(Icon::Arrow, MUTED, 14),
                ].spacing(7).align_y(Alignment::Center)).padding([5, 8]).width(Fill).style(quiet_style)
                    .on_press_maybe(ready.then(|| Message::Attachments(AttachmentsMessage::BrowseDrive(course, DriveNavigation::OpenFolder(folder.id.clone()))))));
            }
            for file in &page.files {
                let attached = already_attached(file);
                rows = rows.push(row![
                    text(file.name()).size(14).style(move |theme| themed_text(theme, FG)).width(Fill).wrapping(text::Wrapping::WordOrGlyph),
                    button(if attached { "Added" } else { "Attach" }).style(quiet_style).padding([5, 8])
                        .on_press_maybe((ready && !attached).then(|| Message::Attachments(AttachmentsMessage::AttachExistingDriveFile(course, file.id().into())))),
                ].spacing(7).align_y(Alignment::Center));
            }
            if page.files.is_empty() && page.folders.is_empty() && !picker.loading && picker.error.is_none() {
                rows = rows.push(text("No matching files or folders.").size(13).style(move |theme| themed_text(theme, MUTED)));
            }
            let mut list = scrollable(container(rows).padding(Padding { right: 12.0, ..Padding::ZERO })).height(125);
            if ready && picker.can_auto_page() {
                let generation = self.attachments.drive_picker_generation;
                list = list.on_scroll(move |viewport| {
                    let remaining = viewport.content_bounds().height - viewport.bounds().height - viewport.absolute_offset().y;
                    Message::Attachments(AttachmentsMessage::DriveScrolled(course, generation, remaining <= 60.0))
                });
            }
            panel = panel.push(list);
            if page.incomplete {
                panel = panel.push(text("Drive reported incomplete search results.").size(13).style(move |theme| themed_text(theme, MUTED)));
            }
        }
        if picker.loading {
            panel = panel.push(text("Checking Drive…").size(13).style(move |theme| themed_text(theme, MUTED)));
        }
        container(panel).padding(10).width(Fill).style(|theme| outline_surface(theme, 8.0)).into()
    }
}

#[derive(Default)]
pub(super) struct DrivePicker {
    pub(super) course: i64,
    pub(super) item: Option<i64>,
    pub(super) query: String,
    pub(super) submitted_query: String,
    pub(super) file_page: Option<DriveFilePage>,
    pub(super) error: Option<(bool, String)>,
    pub(super) listed_tokens: BTreeSet<String>,
    pub(super) loading: bool,
    pub(super) section: DriveSection,
    pub(super) folder_path: Vec<DriveFolder>,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum DriveSection {
    #[default]
    Recent,
    MyDrive,
    Shared,
}

impl DriveSection {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Recent => "Recent",
            Self::MyDrive => "My Drive",
            Self::Shared => "Shared with me",
        }
    }
}

#[derive(Clone)]
pub(super) enum DriveNavigation {
    Section(DriveSection),
    OpenFolder(String),
    Back,
}

impl DrivePicker {
    pub(super) fn accept_page(&mut self, mut page: DriveFilePage, append: bool) {
        if append && let Some(previous) = &mut self.file_page {
            self.listed_tokens.extend(previous.next_page.take());
            if page.next_page.as_ref().is_some_and(|token| self.listed_tokens.contains(token)) {
                page.next_page = None;
                page.incomplete = true;
            }
            let known: BTreeSet<_> = previous.files.iter().map(|file| file.id().to_owned()).collect();
            previous.files.extend(page.files.drain(..).filter(|file| !known.contains(file.id())));
            let known: BTreeSet<_> = previous.folders.iter().map(|folder| folder.id.clone()).collect();
            previous.folders.extend(page.folders.drain(..).filter(|folder| !known.contains(&folder.id)));
            previous.next_page = page.next_page;
            previous.incomplete |= page.incomplete;
        } else {
            self.file_page = Some(page);
        }
    }

    pub(super) fn can_auto_page(&self) -> bool {
        !self.loading && self.error.is_none() && self.query.trim() == self.submitted_query
            && self.file_page.as_ref().is_some_and(|page| page.next_page.is_some())
    }

    pub(super) fn needs_more_for_viewport(&self) -> bool {
        self.can_auto_page() && self.file_page.as_ref()
            .is_some_and(|page| page.files.len() + page.folders.len() <= 4)
    }

    pub(super) fn location(&self) -> DriveLocation {
        if let Some(folder) = self.folder_path.last() {
            return DriveLocation::Folder(folder.id.clone());
        }
        match self.section {
            DriveSection::Recent => DriveLocation::Recent,
            DriveSection::MyDrive => DriveLocation::Folder("root".into()),
            DriveSection::Shared => DriveLocation::Shared,
        }
    }

    pub(super) fn browse(&mut self, navigation: DriveNavigation) -> bool {
        match navigation {
            DriveNavigation::Section(section) => {
                self.section = section;
                self.folder_path.clear();
            }
            DriveNavigation::OpenFolder(id) => {
                let Some(folder) = self.file_page.as_ref()
                    .and_then(|page| page.folders.iter().find(|folder| folder.id == id))
                else { return false };
                self.folder_path.push(folder.clone());
            }
            DriveNavigation::Back => {
                if self.folder_path.pop().is_none() {
                    return false;
                }
            }
        }
        self.query.clear();
        true
    }
}

pub(super) fn trusted_attachment_host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    if url.chars().any(|c| c.is_whitespace() || c.is_control() || c == '\\') {
        return None;
    }
    let host = rest.split(['/', '?', '#']).next()?;
    let mut labels = host.rsplit('.');
    let tld = labels.next()?;
    if tld.len() < 2 || !tld.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let labels: Vec<_> = labels.collect();
    (!labels.is_empty()
        && labels.iter().all(|label| valid_host_label(label))
        && !matches!(tld.to_ascii_lowercase().as_str(), "localhost" | "local" | "internal"))
        .then_some(host)
}

pub(super) fn eligible_personal_copy_template(work: &Coursework) -> Option<String> {
    if work.kind != CourseworkKind::Assignment || work.submission.is_some() {
        return None;
    }
    let mut templates = work.attachments.iter().filter(|attachment|
        attachment.kind == desktop_data::AttachmentKind::DriveFile
            && attachment.drive_sharing == Some(DriveSharing::IndividualCopy));
    let reference = templates.next()?.drive_reference.as_ref()?;
    templates.all(|template| template.drive_reference.as_ref() != Some(reference))
        .then(|| reference.clone())
}

pub(super) fn matching_personal_copy<'a>(item: &'a Coursework, template: &desktop_data::Attachment) -> Option<&'a desktop_data::Attachment> {
    let mut matches = item.submission.as_ref()?.attachments.iter().filter(|file| file.is_personal_copy_of(template));
    let copy = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    (copy.kind == desktop_data::AttachmentKind::DriveFile).then_some(copy)
}

pub(super) fn validate_submission_file(path: &Path) -> Result<(), String> {
    let value = path.to_str().ok_or("This file path is not valid UTF-8.")?;
    if value.len() > 8192 || value.bytes().any(|byte| byte.is_ascii_control()) {
        return Err("This file path cannot be uploaded. Choose another file.".into());
    }
    let metadata = fs::metadata(path).map_err(|_| "Couldn't read this file. Choose another file.")?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err("Choose a nonempty regular file.".into());
    }
    Ok(())
}

pub(super) async fn choose_attachment_file() -> Result<Option<PathBuf>, String> {
    let output = tokio::process::Command::new("/usr/bin/osascript")
        .args(["-e", "POSIX path of (choose file with prompt \"Choose a file to attach\")"])
        .output().await.map_err(|_| "Could not open the file picker.")?;
    if !output.status.success() {
        if String::from_utf8_lossy(&output.stderr).contains("(-128)") {
            return Ok(None);
        }
        return Err("Could not choose a file.".into());
    }
    let path = String::from_utf8(output.stdout).map_err(|_| "The picker returned an unreadable path.")?;
    let path = path.strip_suffix('\n').unwrap_or(&path);
    Ok(Some(PathBuf::from(path)))
}

#[derive(Default)]
pub(super) struct AttachmentsState {
    pub(super) drive_picker: Option<DrivePicker>,
    pub(super) drive_picker_generation: u64,
    pub(super) opening_drive_file: Option<String>,
    pub(super) preparing_drive_preview: bool,
    pub(super) saving_drive_file: bool,
    pub(super) attachment_link: String,
    pub(super) submission_file: Option<(i64, i64, PathBuf)>,
    #[cfg(target_os = "macos")]
    pub(super) submission_capture: Option<CaptureState>,
    #[cfg(target_os = "macos")]
    pub(super) capture_generation: u64,
    pub(super) choosing_submission_file: bool,
    pub(super) pending_remove_material: Option<(i64, i64, i64)>,
    pub(super) personal_copy_checks: BTreeMap<(i64, i64), (String, bool)>,
    pub(super) pending_personal_copy: Option<(i64, i64, String)>,
    pub(super) personal_copy_generation: u64,
    pub(super) personal_copy_loading_generation: Option<u64>,
    pub(super) new_submission_document_open: bool,
}
