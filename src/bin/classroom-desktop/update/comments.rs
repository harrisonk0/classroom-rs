use super::*;

#[derive(Clone)]
pub(in crate::desktop) enum CommentsMessage {
    EditComment(i64, i64, text_editor::Action),
    OpenCommentMentions(i64, i64),
    EmailClassMember(i64, i64),
    ClassMemberEmailLoaded(u64, i64, i64, Result<Vec<desktop_auth::ClassMention>, String>),
    ClassMemberEmailOpened(bool),
    CommentMentionsLoaded(u64, i64, i64, Result<Vec<desktop_auth::ClassMention>, String>),
    InsertCommentMention(i64, i64, i64),
    CloseCommentMentions,
    PostComment(i64, i64),
    EditPrivateComment(i64, i64, text_editor::Action),
    PostPrivateComment(i64, i64),
    ConfirmDeleteComment(CommentTarget),
    CancelDeleteComment,
    DeleteComment(CommentTarget),
    CommentDeleted(CommentTarget, Result<(), String>),
    PrivateCommentPosted(i64, i64, Result<(), String>),
    CommentPosted(i64, i64, Result<(), String>),
}

impl App {
    pub(super) fn update_comments(&mut self, message: CommentsMessage) -> Task<Message> {
        match message {
            CommentsMessage::EditPrivateComment(course, item, action)
                if self.can_compose_comment(course, item) && !self.comments.posting_comment =>
            {
                edit_comment_draft(&mut self.comments.private_drafts, (course, item), action);
            }
            CommentsMessage::ConfirmDeleteComment(target) if self.may_delete_comment(target) => {
                self.comments.confirming_comment_delete = Some(target);
            }
            CommentsMessage::CancelDeleteComment => self.comments.confirming_comment_delete = None,
            CommentsMessage::DeleteComment(target) if self.comments.confirming_comment_delete == Some(target)
                && self.may_delete_comment(target) && !self.has_pending_request() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                let Some(comment) = self.displayed_comment(target) else { return Task::none(); };
                let moderation_author = if self.owns_comment(comment) { None } else { comment.author_id };
                self.comments.confirming_comment_delete = None;
                self.comments.deleting_comment = Some(target);
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    if let Some(author) = moderation_author {
                        desktop_auth::moderate_class_comment(&session, target.course, target.item, target.id, author).await
                    } else {
                        desktop_auth::delete_own_comment(&session, target.course, target.item, target.id, target.audience).await
                    }
                }, move |result| Message::Comments(CommentsMessage::CommentDeleted(target, result)));
            }
            CommentsMessage::CommentDeleted(target, result) if self.comments.deleting_comment == Some(target) => {
                self.comments.deleting_comment = None;
                let id = target.id.to_string();
                match result {
                    Ok(()) => {
                        let comments = if target.audience == CommentAudience::Private {
                            self.comments.private_feedback.get_mut(&(target.course, target.item))
                        } else {
                            self.snapshot.as_mut().and_then(|snapshot| snapshot.coursework.get_mut(&target.course))
                                .and_then(|items| items.iter_mut().find(|item| item.id.parse::<i64>().ok() == Some(target.item)))
                                .and_then(|item| item.comments.as_mut())
                        };
                        if let Some(comments) = comments {
                            comments.retain(|comment| comment.id != id);
                        }
                        self.notice = Some("Comment deleted.".into());
                    }
                    Err(error) => {
                        self.error = Some(error);
                        if self.screen == Screen::Item(target.course)
                            && self.item_id.as_ref().and_then(|id| id.parse().ok()) == Some(target.item)
                        {
                            let read = match target.audience {
                                CommentAudience::Class => DetailRead::Comments(target.course, target.item),
                                CommentAudience::Private => DetailRead::PrivateFeedback(target.course, target.item),
                            };
                            return self.update(Message::Reads(ReadsMessage::LoadDetail(read)));
                        }
                    }
                }
            }
            CommentsMessage::PostPrivateComment(course, item)
                if !self.has_pending_request() && self.can_compose_comment(course, item)
                    && !self.comments.private_unconfirmed.contains(&(course, item)) =>
            {
                if let (Some(session), Some(draft)) = (self.session.clone(),
                    self.comments.private_drafts.get(&(course, item)).map(text_editor::Content::text)
                        .filter(|draft| !draft.trim().is_empty()))
                {
                    self.comments.posting_comment = true;
                    self.error = None;
                    self.notice = None;
                    return Task::perform(async move {
                        desktop_auth::post_private_feedback(&session, course, item, draft).await
                    }, move |result| Message::Comments(CommentsMessage::PrivateCommentPosted(course, item, result)));
                }
            }
            CommentsMessage::PrivateCommentPosted(course, item, result) => {
                self.comments.posting_comment = false;
                match result {
                    Ok(()) => {
                        self.comments.private_drafts.remove(&(course, item));
                        self.notice = Some("Private comment sent.".into());
                        return self.update(Message::Reads(ReadsMessage::LoadDetail(DetailRead::PrivateFeedback(course, item))));
                    }
                    Err(error) => {
                        self.comments.private_unconfirmed.insert((course, item));
                        self.error = Some(error);
                        self.queued_page_load = true;
                    }
                }
            }
            CommentsMessage::EditComment(course, item, action)
                if self.can_compose_class_comment(course, item) && !self.comments.posting_comment =>
            {
                edit_comment_draft(&mut self.comments.comment_drafts, (course, item), action);
            }
            CommentsMessage::EmailClassMember(course, member) if self.can_email_class_member(course, member)
                && self.people.people_email_pending.is_none() && !self.has_pending_request() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.people.people_email_generation = self.people.people_email_generation.wrapping_add(1);
                let generation = self.people.people_email_generation;
                self.people.people_email_pending = Some((course, member));
                self.error = None;
                return Task::perform(async move { desktop_auth::load_class_mentions(&session, course).await },
                    move |result| Message::Comments(CommentsMessage::ClassMemberEmailLoaded(generation, course, member, result)));
            }
            CommentsMessage::ClassMemberEmailLoaded(generation, course, member, result)
                if generation == self.people.people_email_generation && self.people.people_email_pending == Some((course, member)) =>
            {
                self.people.people_email_pending = None;
                if !self.can_email_class_member(course, member) { return Task::none(); }
                match result {
                    Ok(members) => {
                        let Some(url) = class_member_email_url(&members, member) else {
                            self.error = Some("No email address is available for this class member.".into());
                            return Task::none();
                        };
                        return Task::perform(async move {
                            tokio::process::Command::new("/usr/bin/open").arg(url)
                                .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
                                .status().await.is_ok_and(|status| status.success())
                        }, |value| Message::Comments(CommentsMessage::ClassMemberEmailOpened(value)));
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            CommentsMessage::ClassMemberEmailOpened(false) => self.error = Some("The email app couldn't be opened.".into()),
            CommentsMessage::ClassMemberEmailOpened(true) => {}
            CommentsMessage::OpenCommentMentions(course, item) if self.can_compose_class_comment(course, item)
                && !self.has_pending_request() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.comments.comment_mentions_generation = self.comments.comment_mentions_generation.wrapping_add(1);
                let generation = self.comments.comment_mentions_generation;
                self.comments.comment_mentions = Some(CommentMentions { course, item, entries: None, error: None });
                return Task::perform(async move { desktop_auth::load_class_mentions(&session, course).await },
                    move |result| Message::Comments(CommentsMessage::CommentMentionsLoaded(generation, course, item, result)));
            }
            CommentsMessage::CommentMentionsLoaded(generation, course, item, result)
                if generation == self.comments.comment_mentions_generation && self.can_compose_class_comment(course, item) =>
            {
                if let Some(picker) = self.comments.comment_mentions.as_mut()
                    .filter(|picker| picker.course == course && picker.item == item)
                {
                    match result {
                        Ok(entries) => picker.entries = Some(entries),
                        Err(error) => picker.error = Some(error),
                    }
                }
            }
            CommentsMessage::InsertCommentMention(course, item, member) if self.can_compose_class_comment(course, item)
                && !self.has_pending_request() =>
            {
                let Some(email) = self.comments.comment_mentions.as_ref()
                    .filter(|picker| picker.course == course && picker.item == item)
                    .and_then(|picker| picker.entries.as_ref())
                    .and_then(|entries| entries.iter().find(|entry| entry.id == member))
                    .map(|entry| entry.email.clone())
                    else { return Task::none(); };
                let draft = self.comments.comment_drafts.entry((course, item)).or_insert_with(text_editor::Content::new);
                let mut value = draft.text();
                append_comment_mention(&mut value, &email);
                *draft = text_editor::Content::with_text(&value);
                draft.move_to(text_editor::Cursor {
                    position: text_editor::Position { line: value.lines().count().saturating_sub(1),
                        column: value.rsplit('\n').next().map_or(0, str::len) },
                    selection: None,
                });
                self.comments.comment_mentions = None;
            }
            CommentsMessage::CloseCommentMentions => {
                self.comments.comment_mentions_generation = self.comments.comment_mentions_generation.wrapping_add(1);
                self.comments.comment_mentions = None;
            }
            CommentsMessage::PostComment(course, item)
                if !self.has_pending_request() && self.can_compose_class_comment(course, item)
                    && !self.comments.comment_unconfirmed.contains(&(course, item)) =>
            {
                if let (Some(session), Some(draft)) = (self.session.clone(),
                    self.comments.comment_drafts.get(&(course, item)).map(text_editor::Content::text)
                        .filter(|draft| !draft.trim().is_empty()))
                {
                    self.comments.posting_comment = true;
                    self.error = None;
                    self.notice = None;
                    return Task::perform(async move {
                        desktop_auth::post_class_comment(&session, course, item, draft).await
                    }, move |result| Message::Comments(CommentsMessage::CommentPosted(course, item, result)));
                }
            }
            CommentsMessage::CommentPosted(course, item, result) => {
                self.comments.posting_comment = false;
                match result {
                    Ok(()) => {
                        self.comments.comment_drafts.remove(&(course, item));
                        self.notice = Some("Class comment posted.".into());
                        return self.update(Message::Reads(ReadsMessage::LoadDetail(DetailRead::Comments(course, item))));
                    }
                    Err(error) => {
                        self.stream.stream_access.remove(&course);
                        self.page_errors.remove(&PageRead::StreamAccess(course));
                        self.comments.comment_unconfirmed.insert((course, item));
                        self.error = Some(error);
                        self.queued_page_load = true;
                    }
                }
            }
            _ => {}
        }
        self.finish_update()
    }
}
