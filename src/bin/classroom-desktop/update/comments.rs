use super::*;

impl App {
    pub(super) fn update_comments(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::EditPrivateComment(course, item, action)
                if self.can_compose_comment(course, item) && !self.posting_comment =>
            {
                edit_comment_draft(&mut self.private_drafts, (course, item), action);
            }
            Message::ConfirmDeleteComment(target) if self.may_delete_comment(target) => {
                self.confirming_comment_delete = Some(target);
            }
            Message::CancelDeleteComment => self.confirming_comment_delete = None,
            Message::DeleteComment(target) if self.confirming_comment_delete == Some(target)
                && self.may_delete_comment(target) && !self.has_pending_request() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                let Some(comment) = self.displayed_comment(target) else { return Task::none(); };
                let moderation_author = if self.owns_comment(comment) { None } else { comment.author_id };
                self.confirming_comment_delete = None;
                self.deleting_comment = Some(target);
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    if let Some(author) = moderation_author {
                        desktop_auth::moderate_class_comment(&session, target.course, target.item, target.id, author).await
                    } else {
                        desktop_auth::delete_own_comment(&session, target.course, target.item, target.id, target.audience).await
                    }
                }, move |result| Message::CommentDeleted(target, result));
            }
            Message::CommentDeleted(target, result) if self.deleting_comment == Some(target) => {
                self.deleting_comment = None;
                let id = target.id.to_string();
                match result {
                    Ok(()) => {
                        let comments = if target.audience == CommentAudience::Private {
                            self.private_feedback.get_mut(&(target.course, target.item))
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
                            return self.update(Message::LoadDetail(read));
                        }
                    }
                }
            }
            Message::PostPrivateComment(course, item)
                if !self.has_pending_request() && self.can_compose_comment(course, item)
                    && !self.private_unconfirmed.contains(&(course, item)) =>
            {
                if let (Some(session), Some(draft)) = (self.session.clone(),
                    self.private_drafts.get(&(course, item)).map(text_editor::Content::text)
                        .filter(|draft| !draft.trim().is_empty()))
                {
                    self.posting_comment = true;
                    self.error = None;
                    self.notice = None;
                    return Task::perform(async move {
                        desktop_auth::post_private_feedback(&session, course, item, draft).await
                    }, move |result| Message::PrivateCommentPosted(course, item, result));
                }
            }
            Message::PrivateCommentPosted(course, item, result) => {
                self.posting_comment = false;
                match result {
                    Ok(()) => {
                        self.private_drafts.remove(&(course, item));
                        self.notice = Some("Private comment sent.".into());
                        return self.update(Message::LoadDetail(DetailRead::PrivateFeedback(course, item)));
                    }
                    Err(error) => {
                        self.private_unconfirmed.insert((course, item));
                        self.error = Some(error);
                        self.queued_page_load = true;
                    }
                }
            }
            Message::EditComment(course, item, action)
                if self.can_compose_class_comment(course, item) && !self.posting_comment =>
            {
                edit_comment_draft(&mut self.comment_drafts, (course, item), action);
            }
            Message::EmailClassMember(course, member) if self.can_email_class_member(course, member)
                && self.people_email_pending.is_none() && !self.has_pending_request() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.people_email_generation = self.people_email_generation.wrapping_add(1);
                let generation = self.people_email_generation;
                self.people_email_pending = Some((course, member));
                self.error = None;
                return Task::perform(async move { desktop_auth::load_class_mentions(&session, course).await },
                    move |result| Message::ClassMemberEmailLoaded(generation, course, member, result));
            }
            Message::ClassMemberEmailLoaded(generation, course, member, result)
                if generation == self.people_email_generation && self.people_email_pending == Some((course, member)) =>
            {
                self.people_email_pending = None;
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
                        }, Message::ClassMemberEmailOpened);
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Message::ClassMemberEmailOpened(false) => self.error = Some("The email app couldn't be opened.".into()),
            Message::ClassMemberEmailOpened(true) => {}
            Message::OpenCommentMentions(course, item) if self.can_compose_class_comment(course, item)
                && !self.has_pending_request() =>
            {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.comment_mentions_generation = self.comment_mentions_generation.wrapping_add(1);
                let generation = self.comment_mentions_generation;
                self.comment_mentions = Some(CommentMentions { course, item, entries: None, error: None });
                return Task::perform(async move { desktop_auth::load_class_mentions(&session, course).await },
                    move |result| Message::CommentMentionsLoaded(generation, course, item, result));
            }
            Message::CommentMentionsLoaded(generation, course, item, result)
                if generation == self.comment_mentions_generation && self.can_compose_class_comment(course, item) =>
            {
                if let Some(picker) = self.comment_mentions.as_mut()
                    .filter(|picker| picker.course == course && picker.item == item)
                {
                    match result {
                        Ok(entries) => picker.entries = Some(entries),
                        Err(error) => picker.error = Some(error),
                    }
                }
            }
            Message::InsertCommentMention(course, item, member) if self.can_compose_class_comment(course, item)
                && !self.has_pending_request() =>
            {
                let Some(email) = self.comment_mentions.as_ref()
                    .filter(|picker| picker.course == course && picker.item == item)
                    .and_then(|picker| picker.entries.as_ref())
                    .and_then(|entries| entries.iter().find(|entry| entry.id == member))
                    .map(|entry| entry.email.clone())
                    else { return Task::none(); };
                let draft = self.comment_drafts.entry((course, item)).or_insert_with(text_editor::Content::new);
                let mut value = draft.text();
                append_comment_mention(&mut value, &email);
                *draft = text_editor::Content::with_text(&value);
                draft.move_to(text_editor::Cursor {
                    position: text_editor::Position { line: value.lines().count().saturating_sub(1),
                        column: value.rsplit('\n').next().map_or(0, str::len) },
                    selection: None,
                });
                self.comment_mentions = None;
            }
            Message::CloseCommentMentions => {
                self.comment_mentions_generation = self.comment_mentions_generation.wrapping_add(1);
                self.comment_mentions = None;
            }
            Message::PostComment(course, item)
                if !self.has_pending_request() && self.can_compose_class_comment(course, item)
                    && !self.comment_unconfirmed.contains(&(course, item)) =>
            {
                if let (Some(session), Some(draft)) = (self.session.clone(),
                    self.comment_drafts.get(&(course, item)).map(text_editor::Content::text)
                        .filter(|draft| !draft.trim().is_empty()))
                {
                    self.posting_comment = true;
                    self.error = None;
                    self.notice = None;
                    return Task::perform(async move {
                        desktop_auth::post_class_comment(&session, course, item, draft).await
                    }, move |result| Message::CommentPosted(course, item, result));
                }
            }
            Message::CommentPosted(course, item, result) => {
                self.posting_comment = false;
                match result {
                    Ok(()) => {
                        self.comment_drafts.remove(&(course, item));
                        self.notice = Some("Class comment posted.".into());
                        return self.update(Message::LoadDetail(DetailRead::Comments(course, item)));
                    }
                    Err(error) => {
                        self.stream_access.remove(&course);
                        self.page_errors.remove(&PageRead::StreamAccess(course));
                        self.comment_unconfirmed.insert((course, item));
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
