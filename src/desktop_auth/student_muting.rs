use super::{LiveSession, classwork, course_member_key, mutation, require_class_moderator};
use std::collections::{BTreeMap, BTreeSet};

pub async fn load_student_muting(
    session: &LiveSession, course: i64, students: Vec<i64>,
) -> Result<BTreeMap<i64, bool>, String> {
    require_class_moderator(session, course).await?;
    let ids = students.into_iter().filter(|id| *id > 0).collect();
    read_student_muting(session, course, &ids).await
}

pub async fn set_student_muted(
    session: &LiveSession, course: i64, student: i64, expected: bool, muted: bool,
) -> Result<bool, String> {
    if student <= 0 || expected == muted {
        return Err("Choose an enrolled student and a different Stream permission.".into());
    }
    require_class_moderator(session, course).await?;
    let ids = BTreeSet::from([student]);
    if read_student_muting(session, course, &ids).await?.get(&student) != Some(&expected) {
        return Err("Classroom did not confirm this student's current mute status.".into());
    }
    let _: () = session.write_rpc("UpdateCourseUser", mute_request(course, student, muted), Some(course)).await?;
    if read_student_muting(session, course, &ids).await?.get(&student) != Some(&muted) {
        return Err("Classroom did not confirm the change to this student's mute status.".into());
    }
    Ok(muted)
}

async fn read_student_muting(session: &LiveSession, course: i64, students: &BTreeSet<i64>) -> Result<BTreeMap<i64, bool>, String> {
    if students.is_empty() { return Ok(BTreeMap::new()); }
    let response: classwork::CourseUserLookupResponse = session.rpc(
        "LookupCourseUser", classwork::CourseUserLookupRequest {
            course_user_ids: students.iter().map(|&student| course_member_key(course, student)).collect(),
            read_mask: Some(mute_read_mask()),
        }, Some(course),
    ).await?;
    Ok(student_mute_states(response, course, students))
}

fn mute_read_mask() -> classwork::CourseUserReadMask {
    classwork::CourseUserReadMask { include_is_muted: Some(true), include_course_role: Some(true), ..Default::default() }
}

fn mute_request(course: i64, student: i64, muted: bool) -> mutation::UpdateCourseUserRequest {
    mutation::UpdateCourseUserRequest {
        course_user: Some(classwork::CourseUser {
            course_user_id: Some(course_member_key(course, student)),
            is_muted: Some(muted),
            ..Default::default()
        }),
        update_mask: Some(mutation::CourseUserUpdateMask { update_is_muted: Some(true), ..Default::default() }),
        read_mask: Some(mute_read_mask()),
    }
}

fn student_mute_states(response: classwork::CourseUserLookupResponse, course: i64, students: &BTreeSet<i64>) -> BTreeMap<i64, bool> {
    let mut seen = BTreeSet::new();
    let mut result = BTreeMap::new();
    for row in response.results {
        let (Some(key), Some(member)) = (row.course_user_id, row.course_user) else { continue; };
        let Some(student) = key.user_id.as_ref().and_then(|id| id.id).filter(|student| students.contains(student)) else { continue; };
        if key != course_member_key(course, student) || member.course_user_id.as_ref() != Some(&key) || member.course_role != Some(3) {
            continue;
        }
        if !seen.insert(student) {
            result.remove(&student);
            continue;
        }
        if let Some(muted) = member.is_muted {
            result.insert(student, muted);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn teacher_muting_only_writes_the_exact_membership_mute_field() {
        for muted in [true, false] {
            let bytes = mute_request(7, 12, muted).encode_to_vec();
            let request = mutation::UpdateCourseUserRequest::decode(bytes.as_slice()).unwrap();
            let user = request.course_user.unwrap();
            assert_eq!(user.course_user_id, Some(course_member_key(7, 12)));
            assert_eq!(user.is_muted, Some(muted));
            assert!(user.course_user_setting.is_none());
            let mask = request.update_mask.unwrap();
            assert_eq!(mask.update_is_muted, Some(true));
            assert!(mask.course_user_setting_update_mask.is_none());
        }
    }

    #[test]
    fn mute_states_require_exact_students_and_unique_memberships() {
        let row = |course, student, role, muted| classwork::CourseUserLookupResult {
            course_user_id: Some(course_member_key(course, student)),
            course_user: Some(classwork::CourseUser {
                course_user_id: Some(course_member_key(course, student)), course_role: Some(role),
                is_muted: Some(muted), ..Default::default()
            }),
        };
        let response = classwork::CourseUserLookupResponse { results: vec![
            row(7, 12, 3, false), row(7, 13, 2, true), row(8, 14, 3, true),
            row(7, 15, 3, false), row(7, 15, 3, true), row(7, 22, 3, true),
        ] };
        assert_eq!(student_mute_states(response, 7, &BTreeSet::from([12, 13, 14, 15])),
            BTreeMap::from([(12, false)]));
    }
}
