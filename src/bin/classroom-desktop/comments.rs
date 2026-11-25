use super::*;

pub(super) fn edit_comment_draft(
    drafts: &mut BTreeMap<(i64, i64), text_editor::Content>,
    key: (i64, i64),
    action: text_editor::Action,
) {
    let draft = drafts.entry(key).or_insert_with(text_editor::Content::new);
    let original = draft.text();
    draft.perform(action);
    if draft.text().len() > 10_000 {
        *draft = text_editor::Content::with_text(&original);
    }
}

pub(super) fn append_comment_mention(draft: &mut String, email: &str) {
    let prefix = if draft.is_empty() || draft.ends_with(char::is_whitespace) { "" } else { " " };
    if draft.len() + prefix.len() + email.len() + 2 <= 10_000 {
        draft.push_str(prefix);
        draft.push('+');
        draft.push_str(email);
        draft.push(' ');
    }
}

pub(super) struct CommentsState {
    pub(super) comment_drafts: BTreeMap<(i64, i64), text_editor::Content>,
    pub(super) empty_comment_draft: text_editor::Content,
    pub(super) comment_mentions_generation: u64,
    pub(super) comment_mentions: Option<CommentMentions>,
    pub(super) comment_unconfirmed: BTreeSet<(i64, i64)>,
    pub(super) posting_comment: bool,
    pub(super) private_feedback: BTreeMap<(i64, i64), Vec<Comment>>,
    pub(super) private_drafts: BTreeMap<(i64, i64), text_editor::Content>,
    pub(super) confirming_comment_delete: Option<CommentTarget>,
    pub(super) deleting_comment: Option<CommentTarget>,
    pub(super) private_unconfirmed: BTreeSet<(i64, i64)>,
}

impl CommentsState {
    pub(super) fn new() -> Self {
        Self {
            comment_drafts: BTreeMap::new(),
            empty_comment_draft: text_editor::Content::new(),
            comment_mentions_generation: 0,
            comment_mentions: None,
            comment_unconfirmed: BTreeSet::new(),
            posting_comment: false,
            private_feedback: BTreeMap::new(),
            private_drafts: BTreeMap::new(),
            confirming_comment_delete: None,
            deleting_comment: None,
            private_unconfirmed: BTreeSet::new(),
        }
    }
}
