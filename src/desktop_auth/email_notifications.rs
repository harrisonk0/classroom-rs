use super::{LiveSession, classwork, mutation};
use std::collections::BTreeMap;

const LOCALE: &str = "en-GB";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailNotificationTarget {
    AllEmail,
    Group(i32),
}

#[derive(Debug, Clone)]
pub struct EmailNotificationSettings {
    pub enabled: Option<bool>,
    pub categories: Vec<EmailNotificationCategory>,
}

#[derive(Debug, Clone)]
pub struct EmailNotificationCategory {
    pub label: Option<String>,
    pub groups: Vec<EmailNotificationGroup>,
}

#[derive(Debug, Clone)]
pub struct EmailNotificationGroup {
    pub tag: i32,
    pub label: String,
    pub enabled: Option<bool>,
}

impl EmailNotificationSettings {
    pub fn value(&self, target: EmailNotificationTarget) -> Option<bool> {
        match target {
            EmailNotificationTarget::AllEmail => self.enabled,
            EmailNotificationTarget::Group(tag) => self.categories.iter().flat_map(|category| &category.groups)
                .find(|group| group.tag == tag)?.enabled,
        }
    }
}

fn read_mask() -> classwork::UserReadMask {
    classwork::UserReadMask {
        include_name: None,
        include_email: None,
        user_setting_read_mask: Some(classwork::UserSettingReadMask {
            include_receive_email_notifications: Some(true),
            user_notification_category_read_mask: Some(classwork::UserNotificationCategoryReadMask {
                include_localized_display_name: Some(true),
                user_notification_group_read_mask: Some(classwork::UserNotificationGroupReadMask {
                    include_group_tag: Some(true),
                    include_localized_display_name: Some(true),
                    include_receive_email_notifications: Some(true),
                }),
            }),
        }),
    }
}

pub async fn load_email_notifications(session: &LiveSession) -> Result<EmailNotificationSettings, String> {
    read_account_settings(session).await.map(|(_, settings)| settings)
}

pub async fn set_email_notification(
    session: &LiveSession, target: EmailNotificationTarget, enabled: bool,
) -> Result<EmailNotificationSettings, String> {
    let (user_id, before) = read_account_settings(session).await?;
    if matches!(target, EmailNotificationTarget::Group(_)) && before.enabled != Some(true) {
        return Err("Turn on email notifications before changing an individual preference.".into());
    }
    let current = before.value(target).ok_or_else(|| "Classroom did not return this preference.".to_owned())?;
    if current == enabled {
        return Ok(before);
    }
    let _: () = session.write_rpc("WriteUser", update_request(user_id, target, enabled), None).await?;
    let (confirmed_user, after) = read_account_settings(session).await?;
    if confirmed_user != user_id || after.value(target) != Some(enabled) {
        return Err("Classroom did not confirm the email notification change.".into());
    }
    Ok(after)
}

fn update_request(user: i64, target: EmailNotificationTarget, enabled: bool) -> mutation::WriteUserRequest {
    let mut settings = classwork::UserSetting::default();
    let mut mask = mutation::UserSettingUpdateMask::default();
    match target {
        EmailNotificationTarget::AllEmail => {
            settings.receive_email_notifications = Some(enabled);
            mask.update_receive_email_notifications = Some(true);
        }
        EmailNotificationTarget::Group(tag) => {
            settings.user_notification_category.push(classwork::UserNotificationCategory {
                user_notification_group: vec![classwork::UserNotificationGroup {
                    group_tag: Some(tag),
                    receive_email_notifications: Some(enabled),
                    ..Default::default()
                }],
                ..Default::default()
            });
            mask.user_notification_category_update_mask = Some(mutation::UserNotificationCategoryUpdateMask {
                user_notification_group_update_mask: Some(mutation::UserNotificationGroupUpdateMask {
                    update_receive_email_notifications: Some(true),
                }),
            });
        }
    }
    mutation::WriteUserRequest {
        request_header: Some(mutation::WriteRequestHeader { operation: Some(3) }),
        user_mutation: vec![mutation::UserMutation {
            user_id: Some(classwork::UserId { id: Some(user) }),
            user: Some(classwork::User { user_setting: Some(settings), ..Default::default() }),
            user_update_mask: Some(mutation::UserUpdateMask { user_setting_update_mask: Some(mask) }),
        }],
        user_read_mask: Some(read_mask()),
        locale: Some(LOCALE.into()),
    }
}

async fn read_account_settings(session: &LiveSession) -> Result<(i64, EmailNotificationSettings), String> {
    let response: classwork::QueryUserResponse = session.read_rpc("QueryUser", classwork::QueryUserRequest {
        user_read_mask: Some(read_mask()),
        filter_options: Some(classwork::UserFilterOptions {
            base_filter_options: Some(classwork::FilterOptions {
                user_filter: vec![classwork::UserFilter { use_current_user: Some(true), user_id: None }],
            }),
        }),
        locale: Some(LOCALE.into()),
    }).await?;
    let [user] = response.user.as_slice() else {
        return Err("Classroom did not identify a single signed-in user.".into());
    };
    let id = user.user_id.as_ref().and_then(|id| id.id).filter(|id| *id > 0)
        .ok_or_else(|| "Classroom did not identify the signed-in user.".to_owned())?;
    let settings = user.user_setting.as_ref()
        .ok_or_else(|| "Classroom did not return your email notification settings.".to_owned())?;
    Ok((id, decode_settings(settings)))
}

fn decode_settings(settings: &classwork::UserSetting) -> EmailNotificationSettings {
    let mut tag_counts = BTreeMap::new();
    for group in settings.user_notification_category.iter().flat_map(|category| &category.user_notification_group) {
        if let Some(tag) = group.group_tag {
            *tag_counts.entry(tag).or_insert(0) += 1;
        }
    }
    let categories = settings.user_notification_category.iter().filter_map(|category| {
        let groups: Vec<_> = category.user_notification_group.iter().filter_map(|group| {
            let tag = group.group_tag?;
            let label = group.localized_display_name.as_deref()?.trim();
            (tag > 0 && !label.is_empty() && tag_counts.get(&tag) == Some(&1)).then(|| EmailNotificationGroup {
                tag, label: label.into(), enabled: group.receive_email_notifications,
            })
        }).collect();
        (!groups.is_empty()).then(|| EmailNotificationCategory {
            label: category.localized_display_name.as_deref().map(str::trim).filter(|label| !label.is_empty()).map(str::to_owned),
            groups,
        })
    }).collect();
    EmailNotificationSettings { enabled: settings.receive_email_notifications, categories }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn account_off_sends_explicit_false_and_the_master_mask() {
        let request = update_request(7, EmailNotificationTarget::AllEmail, false);
        let mut encoded = request.encode_to_vec();
        // Fields 3 and 4 are the response read mask and locale.
        encoded.truncate(22);
        assert_eq!(encoded, [
            0x0a, 2, 0x08, 3, 0x12, 16, 0x0a, 2, 0x08, 7,
            0x12, 4, 0x22, 2, 0x08, 0, 0x1a, 4, 0x0a, 2, 0x08, 1,
        ]);
    }

    #[test]
    fn group_change_sends_only_one_tag_with_the_nested_update_mask() {
        let mut encoded = update_request(7, EmailNotificationTarget::Group(42), true).encode_to_vec();
        encoded.truncate(32);
        assert_eq!(encoded, [
            0x0a, 2, 0x08, 3, 0x12, 26, 0x0a, 2, 0x08, 7,
            0x12, 10, 0x22, 8, 0x42, 6, 0x12, 4, 0x08, 42, 0x20, 1,
            0x1a, 8, 0x0a, 6, 0x42, 4, 0x0a, 2, 0x08, 1,
        ]);
    }

    #[test]
    fn decoded_preferences_keep_false_and_exclude_an_ambiguous_tag() {
        let group = |tag, label: &str, enabled| classwork::UserNotificationGroup {
            group_tag: Some(tag),
            localized_display_name: Some(label.into()),
            receive_email_notifications: enabled,
        };
        let settings = decode_settings(&classwork::UserSetting {
            receive_email_notifications: Some(false),
            user_notification_category: vec![classwork::UserNotificationCategory {
                localized_display_name: Some("Comments".into()),
                user_notification_group: vec![
                    group(1, "Comments on your posts", Some(true)),
                    group(2, "Ambiguous", Some(true)), group(2, "Ambiguous", Some(false)),
                    group(3, "Missing value", None),
                ],
            }],
        });
        assert_eq!(settings.value(EmailNotificationTarget::AllEmail), Some(false));
        assert_eq!(settings.value(EmailNotificationTarget::Group(1)), Some(true));
        assert_eq!(settings.value(EmailNotificationTarget::Group(2)), None);
        assert_eq!(settings.value(EmailNotificationTarget::Group(3)), None);
        assert_eq!(settings.categories[0].groups.len(), 2);
    }
}
