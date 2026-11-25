use super::*;

#[derive(Clone)]
pub(in crate::desktop) enum StreamMessage {
    OpenStreamComposer(i64),
    EditStreamDraft(i64, text_editor::Action),
    FormatStreamDraft(i64, StreamFormat),
    OpenStreamLink(i64),
    EditStreamLink(i64, String),
    AddStreamLink(i64),
    CancelStreamLink(i64),
    RemoveStreamLink(i64, usize),
    ChooseStreamFile(i64),
    StreamFileChosen(i64, Result<Option<PathBuf>, String>),
    ClearStreamFile(i64),
    UploadStreamFile(i64),
    StreamFileUploaded(i64, Result<DriveFileAttachment, String>),
    RemoveStreamFile(i64, usize),
    CloseStreamComposer,
    SendStreamPost(i64),
    StreamPostSent(i64, Result<PublishedAnnouncement, String>),
    ConfirmDeleteStreamPost(i64, i64),
    CancelDeleteStreamPost,
    DeleteStreamPost(i64, i64),
    StreamPostDeleted(i64, i64, Result<(), String>),
}

impl App {
    pub(super) fn update_stream(&mut self, message: StreamMessage) -> Task<Message> {
        match message {
            StreamMessage::OpenStreamComposer(id) if self.can_post_stream(id) => {
                self.stream.stream_drafts.entry(id).or_default();
                self.stream.open_stream_composer = Some(id);
            }
            StreamMessage::EditStreamDraft(id, action) if self.stream.open_stream_composer == Some(id)
                && self.can_post_stream(id) && self.stream.sending_stream_post != Some(id) =>
            {
                if let Some(draft) = self.stream.stream_drafts.get_mut(&id) {
                    draft.edit(action);
                }
            }
            StreamMessage::FormatStreamDraft(id, format) if self.can_edit_stream_attachments(id) => {
                if let Some(draft) = self.stream.stream_drafts.get_mut(&id) {
                    draft.format(format);
                }
            }
            StreamMessage::OpenStreamLink(id) if self.can_edit_stream_attachments(id) => {
                if let Some(draft) = self.stream.stream_drafts.get_mut(&id) {
                    draft.link_input.get_or_insert_with(String::new);
                }
            }
            StreamMessage::EditStreamLink(id, value) if self.can_edit_stream_attachments(id) && value.len() <= 8192 => {
                if let Some(input) = self.stream.stream_drafts.get_mut(&id).and_then(|draft| draft.link_input.as_mut()) {
                    *input = value;
                }
            }
            StreamMessage::AddStreamLink(id) if self.can_edit_stream_attachments(id) => {
                if let Some(draft) = self.stream.stream_drafts.get_mut(&id)
                    && let Some(url) = draft.pending_link().map(str::to_owned)
                {
                    draft.links.push(url);
                    draft.link_input = None;
                }
            }
            StreamMessage::CancelStreamLink(id) if self.can_edit_stream_attachments(id) => {
                if let Some(draft) = self.stream.stream_drafts.get_mut(&id) {
                    draft.link_input = None;
                }
            }
            StreamMessage::RemoveStreamLink(id, index) if self.can_edit_stream_attachments(id) => {
                if let Some(draft) = self.stream.stream_drafts.get_mut(&id)
                    && index < draft.links.len()
                {
                    draft.links.remove(index);
                }
            }
            StreamMessage::ChooseStreamFile(id) if self.can_edit_stream_attachments(id) && !self.has_pending_request() => {
                self.stream.choosing_stream_file = Some(id);
                return Task::perform(choose_attachment_file(), move |result| Message::Stream(StreamMessage::StreamFileChosen(id, result)));
            }
            StreamMessage::StreamFileChosen(id, result) if self.stream.choosing_stream_file == Some(id) => {
                self.stream.choosing_stream_file = None;
                match result {
                    Ok(Some(path)) => {
                        if let Some(draft) = self.stream.stream_drafts.get_mut(&id) {
                            draft.selected_file = Some(path);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => self.error = Some(error),
                }
            }
            StreamMessage::ClearStreamFile(id) if self.can_edit_stream_attachments(id) => {
                if let Some(draft) = self.stream.stream_drafts.get_mut(&id) {
                    draft.selected_file = None;
                }
            }
            StreamMessage::UploadStreamFile(id) if self.can_edit_stream_attachments(id) && !self.has_pending_request() => {
                if let (Some(session), Some(path)) = (
                    self.session.clone(),
                    self.stream.stream_drafts.get(&id).and_then(|draft| draft.selected_file.clone()),
                ) {
                    self.stream.uploading_stream_file = Some(id);
                    self.error = None;
                    self.notice = None;
                    return Task::perform(async move { desktop_auth::upload_stream_file(&session, id, path).await },
                        move |result| Message::Stream(StreamMessage::StreamFileUploaded(id, result)));
                }
            }
            StreamMessage::StreamFileUploaded(id, result) if self.stream.uploading_stream_file == Some(id) => {
                self.stream.uploading_stream_file = None;
                match result {
                    Ok(file) => {
                        if let Some(draft) = self.stream.stream_drafts.get_mut(&id) {
                            draft.selected_file = None;
                            draft.files.push(file);
                        }
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            StreamMessage::RemoveStreamFile(id, index) if self.can_edit_stream_attachments(id) => {
                if let Some(draft) = self.stream.stream_drafts.get_mut(&id)
                    && index < draft.files.len()
                {
                    draft.files.remove(index);
                }
            }
            StreamMessage::CloseStreamComposer => {
                self.stream.open_stream_composer = None;
                self.attachments.drive_picker = None;
            }
            StreamMessage::SendStreamPost(id) if self.stream.open_stream_composer == Some(id)
                && self.can_post_stream(id) && !self.has_pending_request()
                && !self.page_errors.contains_key(&PageRead::Coursework(id)) =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                if let Some((body, styles, bullets, links, files)) = self.stream.stream_drafts.get(&id).and_then(|draft| {
                    let body = draft.body.text();
                    draft.can_send(&body).then(|| (body, draft.description_styles(), draft.bullets.iter().copied().collect(),
                        draft.links.clone(), draft.files.clone()))
                }) {
                    self.stream.sending_stream_post = Some(id);
                    self.error = None;
                    self.notice = None;
                    return Task::perform(async move { desktop_auth::post_stream_announcement(&session, id, body, styles, bullets, links, files).await },
                        move |result| Message::Stream(StreamMessage::StreamPostSent(id, result)));
                }
            }
            StreamMessage::StreamPostSent(id, result) if self.stream.sending_stream_post == Some(id) => {
                self.stream.sending_stream_post = None;
                match result {
                    Ok(PublishedAnnouncement { item, unconfirmed_attachments, formatting_confirmed }) => {
                        self.stream.stream_drafts.remove(&id);
                        self.attachments.drive_picker = None;
                        if self.stream.open_stream_composer == Some(id) {
                            self.stream.open_stream_composer = None;
                        }
                        if let Some(snapshot) = &mut self.snapshot {
                            let items = snapshot.coursework.entry(id).or_default();
                            items.retain(|existing| existing.id != item.id);
                            items.insert(0, item);
                        }
                        if unconfirmed_attachments == 0 {
                            if formatting_confirmed {
                                self.notice = Some("Posted to Stream.".into());
                            } else {
                                self.error = Some("Posted to Stream. Classroom didn’t confirm the formatting.".into());
                            }
                        } else {
                            let noun = if unconfirmed_attachments == 1 { "attachment" } else { "attachments" };
                            let formatting = if formatting_confirmed { "" } else { " or the formatting" };
                            self.error = Some(format!("Posted to Stream. Classroom didn’t confirm {unconfirmed_attachments} {noun}{formatting}."));
                        }
                    }
                    Err(error) => {
                        self.stream.stream_access.remove(&id);
                        if self.on_live_stream(id) {
                            let task = self.update(Message::Reads(ReadsMessage::LoadCoursework(id)));
                            self.error = Some(error);
                            return task;
                        }
                        self.error = Some(error);
                    }
                }
            }
            StreamMessage::ConfirmDeleteStreamPost(course, id) if self.can_delete_stream_post(course, id) => {
                self.stream.confirming_stream_delete = Some((course, id));
            }
            StreamMessage::CancelDeleteStreamPost => self.stream.confirming_stream_delete = None,
            StreamMessage::DeleteStreamPost(course, id) if self.stream.confirming_stream_delete == Some((course, id))
                && self.can_delete_stream_post(course, id) && !self.has_pending_request() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                let Some(item) = self.displayed_announcement(course, id) else { return Task::none(); };
                let moderation_author = if self.owns_announcement(item) { None } else { item.author_id };
                self.stream.confirming_stream_delete = None;
                self.stream.deleting_stream_post = Some((course, id));
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    if let Some(author) = moderation_author {
                        desktop_auth::moderate_stream_post(&session, course, id, author).await
                    } else {
                        desktop_auth::delete_own_stream_post(&session, course, id).await
                    }
                },
                    move |result| Message::Stream(StreamMessage::StreamPostDeleted(course, id, result)));
            }
            StreamMessage::StreamPostDeleted(course, id, result) if self.stream.deleting_stream_post == Some((course, id)) => {
                self.stream.deleting_stream_post = None;
                match result {
                    Ok(()) => {
                        if let Some(items) = self.snapshot.as_mut().and_then(|snapshot| snapshot.coursework.get_mut(&course)) {
                            let item_id = id.to_string();
                            items.retain(|item| item.id != item_id || item.kind != CourseworkKind::Announcement);
                        }
                        self.notice = Some("Post deleted.".into());
                    }
                    Err(error) => {
                        if self.on_live_stream(course) {
                            let refresh = self.update(Message::Reads(ReadsMessage::LoadCoursework(course)));
                            self.error = Some(error);
                            return refresh;
                        }
                        self.error = Some(error);
                    }
                }
            }
            _ => {}
        }
        self.finish_update()
    }
}
