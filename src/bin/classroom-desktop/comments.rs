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
