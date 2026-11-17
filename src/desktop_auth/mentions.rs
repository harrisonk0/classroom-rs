use super::{LiveSession, classwork, load_roster};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct ClassMention {
    pub id: i64,
    pub name: String,
    pub email: String,
}

pub async fn load_class_mentions(session: &LiveSession, course: i64) -> Result<Vec<ClassMention>, String> {
    let roster = load_roster(session, course).await?;
    let ids: BTreeSet<_> = roster.iter().filter_map(|member| member.id.parse::<i64>().ok())
        .filter(|id| *id > 0).collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let response: classwork::QueryUserResponse = session.read_rpc(
        "QueryUser",
        classwork::QueryUserRequest {
            user_read_mask: Some(classwork::UserReadMask {
                include_email: Some(true),
                ..Default::default()
            }),
            filter_options: Some(classwork::UserFilterOptions {
                base_filter_options: Some(classwork::FilterOptions {
                    user_filter: ids.iter().map(|id| classwork::UserFilter {
                        user_id: Some(classwork::UserId { id: Some(*id) }),
                        use_current_user: None,
                    }).collect(),
                }),
            }),
            ..Default::default()
        },
    ).await?;
    let emails = select_emails(response, &ids);
    Ok(roster.into_iter().filter_map(|member| {
        let id = member.id.parse::<i64>().ok()?;
        Some(ClassMention { id, name: member.display_name, email: emails.get(&id)?.clone() })
    }).collect())
}

fn select_emails(response: classwork::QueryUserResponse, ids: &BTreeSet<i64>) -> BTreeMap<i64, String> {
    let mut values = BTreeMap::new();
    let mut duplicates = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for user in response.user {
        let Some(id) = user.user_id.and_then(|user| user.id).filter(|id| ids.contains(id)) else { continue };
        if !seen.insert(id) {
            duplicates.insert(id);
            continue;
        }
        let Some(email) = user.email.filter(|email| valid_email(email)) else { continue };
        values.insert(id, email);
    }
    for id in duplicates {
        values.remove(&id);
    }
    values
}

fn valid_email(email: &str) -> bool {
    if email.len() > 320 || email.bytes().any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control()) {
        return false;
    }
    let Some((local, domain)) = email.split_once('@') else { return false };
    !local.is_empty() && !domain.is_empty()
        && local.bytes().all(|byte| byte.is_ascii_alphanumeric() || b".!#$%&'*+/=?^_`{|}~-".contains(&byte))
        && domain.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn emails_are_restricted_to_exact_roster_and_ambiguous_entries_are_removed() {
        let entry = |id, email: &str| classwork::User {
            user_id: Some(classwork::UserId { id: Some(id) }),
            email: Some(email.into()),
            ..Default::default()
        };
        let response = classwork::QueryUserResponse { user: vec![
            entry(1, "one@example.org"), entry(2, "two@example.org"), entry(2, "other@example.org"),
            entry(3, "not-a-member@example.org"), entry(4, "bad email@example.org"),
        ] };
        let selected = select_emails(response, &BTreeSet::from([1, 2, 4]));
        assert_eq!(selected, BTreeMap::from([(1, "one@example.org".into())]));
        assert_eq!(classwork::UserReadMask { include_email: Some(true), ..Default::default() }.encode_to_vec(), [0x10, 0x01]);
    }
}
