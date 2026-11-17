use super::{LiveSession, classwork, course_member_key, current_user_id, mutation, wire};
use crate::desktop_data::{CourseState, decode_class_hidden};
use std::collections::{BTreeMap, BTreeSet};

fn visibility_mask() -> classwork::CourseUserReadMask {
    classwork::CourseUserReadMask {
        include_course_role: Some(true),
        course_user_setting_read_mask: Some(classwork::CourseUserSettingReadMask {
            include_course_visibility_setting: Some(true),
        }),
        ..Default::default()
    }
}

pub async fn class_visibility_for(
    session: &LiveSession, courses: Vec<i64>,
) -> Result<BTreeMap<i64, Option<bool>>, String> {
    let ids = courses.into_iter().filter(|&id| id > 0).collect();
    let own_user = current_user_id(session).await?;
    lookup_visibility(session, &ids, own_user).await
}

async fn lookup_visibility(
    session: &LiveSession, courses: &BTreeSet<i64>, user: i64,
) -> Result<BTreeMap<i64, Option<bool>>, String> {
    let response: classwork::CourseUserLookupResponse = session.read_rpc(
        "LookupCourseUser", classwork::CourseUserLookupRequest {
            course_user_ids: courses.iter().map(|&course| course_member_key(course, user)).collect(),
            read_mask: Some(visibility_mask()),
        },
    ).await?;
    Ok(decode_visibility(response, courses, user))
}

fn decode_visibility(
    response: classwork::CourseUserLookupResponse, courses: &BTreeSet<i64>, user: i64,
) -> BTreeMap<i64, Option<bool>> {
    let mut states = BTreeMap::new();
    let mut duplicates = BTreeSet::new();
    for result in response.results {
        let Some(key) = result.course_user_id else { continue };
        let Some(id) = key.course_id.as_ref().and_then(|course| course.id) else { continue };
        let Some(member) = result.course_user else { continue };
        if !courses.contains(&id) || key != course_member_key(id, user) || member.course_user_id != Some(key)
            || member.course_role != Some(3)
        {
            continue;
        }
        let hidden = member.course_user_setting.and_then(|settings| decode_class_hidden(settings.course_visibility_setting));
        if states.insert(id, hidden).is_some() {
            duplicates.insert(id);
        }
    }
    states.retain(|id, _| !duplicates.contains(id));
    states
}

fn update_visibility(course: i64, user: i64, hidden: bool) -> mutation::UpdateCourseUserRequest {
    let value = if hidden { wire::CourseVisibilitySetting::HiddenByUser } else { wire::CourseVisibilitySetting::Visible };
    mutation::UpdateCourseUserRequest {
        course_user: Some(classwork::CourseUser {
            course_user_id: Some(course_member_key(course, user)),
            course_user_setting: Some(classwork::CourseUserSetting { course_visibility_setting: Some(value.into()) }),
            ..Default::default()
        }),
        update_mask: Some(mutation::CourseUserUpdateMask {
            course_user_setting_update_mask: Some(mutation::CourseUserSettingUpdateMask {
                update_course_visibility_setting: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        }),
        read_mask: Some(visibility_mask()),
    }
}

pub async fn set_class_hidden(session: &LiveSession, course: i64, hidden: bool) -> Result<(), String> {
    if !session.snapshot.courses.iter().any(|entry| entry.id == course && entry.state == CourseState::Active) {
        return Err("Choose an active class first.".into());
    }
    let own_user = current_user_id(session).await?;
    let ids = BTreeSet::from([course]);
    let before = lookup_visibility(session, &ids, own_user).await?;
    let Some(&current) = before.get(&course) else {
        return Err("Classroom did not confirm your student membership in this class.".into());
    };
    if current == Some(hidden) {
        return Ok(());
    }
    let _: () = session.write_rpc(
        "UpdateCourseUser", update_visibility(course, own_user, hidden), Some(course),
    ).await?;
    let after = lookup_visibility(session, &ids, own_user).await?;
    if after.get(&course) != Some(&Some(hidden)) {
        return Err("Classroom did not confirm the change to class visibility.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn visibility_update_only_changes_the_scoped_membership_preference() {
        let mut bytes = [
            0x0a, 15, 0x0a, 8, 0x0a, 2, 0x08, 7, 0x12, 2, 0x08, 11,
            0x8a, 1, 2, 0x40, 2,
            0x12, 4, 0x12, 2, 0x28, 1,
            0x1a, 6, 0x08, 1, 0x7a, 2, 0x28, 1,
        ];
        assert_eq!(update_visibility(7, 11, true).encode_to_vec(), bytes);
        bytes[16] = 1;
        assert_eq!(update_visibility(7, 11, false).encode_to_vec(), bytes);
    }

    #[test]
    fn visibility_lookup_requires_both_keys_and_a_student_role() {
        let row = |course, outer, inner, role, hidden| classwork::CourseUserLookupResult {
            course_user_id: Some(course_member_key(course, outer)),
            course_user: Some(classwork::CourseUser {
                course_user_id: Some(course_member_key(course, inner)),
                course_role: Some(role),
                course_user_setting: Some(classwork::CourseUserSetting { course_visibility_setting: Some(hidden) }),
                ..Default::default()
            }),
        };
        let response = classwork::CourseUserLookupResponse { results: vec![
            row(1, 7, 7, 3, 2), row(2, 7, 7, 3, 1), row(3, 7, 7, 3, 0),
            row(4, 8, 8, 3, 2), row(5, 7, 8, 3, 2), row(6, 7, 7, 2, 2),
            row(7, 7, 7, 3, 1), row(7, 7, 7, 3, 2),
        ] };
        assert_eq!(decode_visibility(response, &BTreeSet::from_iter(1..=7), 7),
            BTreeMap::from([(1, Some(true)), (2, Some(false)), (3, None)]));
    }
}
