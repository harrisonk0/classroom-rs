use super::*;

impl App {
    pub(super) fn can_compose_comment(&self, course: i64, item: i64) -> bool {
        if self.session.is_none() || self.verify_live_course_count
            || self.screen != Screen::Item(course)
            || self.item_id.as_deref().and_then(|id| id.parse::<i64>().ok()) != Some(item)
        {
            return false;
        }
        self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.source == SnapshotSource::Live && snapshot.courses.iter()
                .any(|entry| entry.id == course && entry.state == CourseState::Active)
        })
    }

    pub(super) fn personal_copy_candidate(&self, course: i64, item: i64) -> Option<String> {
        if !self.can_compose_comment(course, item)
            || self.class_roles.get(&course) != Some(&CourseRole::Student)
            || !self.student_work.submissions_checked.contains(&course)
            || self.page_errors.contains_key(&PageRead::Detail(DetailRead::Submissions(course)))
        {
            return None;
        }
        let work = self.snapshot.as_ref()?.coursework_for(course)?.iter()
            .find(|entry| entry.id.parse::<i64>().ok() == Some(item))?;
        eligible_personal_copy_template(work)
    }

    pub(super) fn personal_copy_ready(&self, course: i64, item: i64, reference: &str) -> bool {
        self.attachments.personal_copy_checks.get(&(course, item)).is_some_and(|(checked, absent)| *absent && checked == reference)
            && self.personal_copy_candidate(course, item).as_deref() == Some(reference)
    }

    pub(super) fn can_compose_class_comment(&self, course: i64, item: i64) -> bool {
        self.can_compose_comment(course, item)
            && self.stream.stream_access.get(&course).is_some_and(|access| access.can_comment())
    }

    pub(super) fn may_delete_comment(&self, target: CommentTarget) -> bool {
        self.can_compose_comment(target.course, target.item) && self.displayed_comment(target)
            .is_some_and(|comment| self.may_delete_visible_comment(target.course, target.audience, comment))
    }

    pub(super) fn displayed_comment(&self, target: CommentTarget) -> Option<&Comment> {
        let comments = match target.audience {
            CommentAudience::Private => self.comments.private_feedback.get(&(target.course, target.item)),
            CommentAudience::Class => self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(target.course))
                .and_then(|items| items.iter().find(|item| item.id.parse::<i64>().ok() == Some(target.item)))
                .and_then(|item| item.comments.as_ref()),
        }?;
        comments.iter().find(|comment| comment.id.parse::<i64>().ok() == Some(target.id))
    }

    pub(super) fn may_delete_visible_comment(&self, course: i64, audience: CommentAudience, comment: &Comment) -> bool {
        self.owns_comment(comment) || (audience == CommentAudience::Class && self.may_moderate_author(course, comment.author_id))
    }

    pub(super) fn may_moderate_author(&self, course: i64, author: Option<i64>) -> bool {
        self.class_roles.get(&course) == Some(&CourseRole::Teacher) && author.is_some_and(|id| id > 0)
    }

    pub(super) fn owns_comment(&self, comment: &Comment) -> bool {
        self.stream.stream_viewer_id.is_some() && self.stream.stream_viewer_id == comment.author_id
    }

    pub(super) fn current_question(&self, course: i64, item: i64) -> Option<&Coursework> {
        if !self.can_compose_comment(course, item)
            || self.page_errors.contains_key(&PageRead::Detail(DetailRead::Submissions(course)))
        {
            return None;
        }
        self.snapshot.as_ref()?.coursework_for(course)?
            .iter().find(|work| work.id == item.to_string())
    }

    pub(super) fn editable_question(&self, course: i64, item: i64) -> Option<&QuestionDetails> {
        let work = self.current_question(course, item)?;
        let revision = self.student_work.short_answer_revisions.get(&(course, item));
        if desktop_auth::question_submission_is_current(work, revision) {
            work.question.as_ref()
        } else {
            None
        }
    }

    pub(super) fn question_draft(&self, key: (i64, i64), details: &QuestionDetails) -> Option<QuestionAnswer> {
        let draft = match details {
            QuestionDetails::ShortAnswer => {
                let text = self.student_work.short_answer_drafts.get(&key)?.text();
                if self.student_work.short_answer_revisions.get(&key).is_some_and(|revision| revision.answer == text) {
                    return None;
                }
                QuestionAnswer::ShortAnswer(text)
            }
            QuestionDetails::MultipleChoice(_) => QuestionAnswer::MultipleChoice(self.student_work.choice_drafts.get(&key)?.clone()),
        };
        details.accepts(&draft).then_some(draft)
    }
}
