use super::{
    wire, classwork, mutation, check_class, LiveSession, BTreeMap, BTreeSet, Message,
};

fn notification_read_mask() -> wire::CourseReadMask {
    wire::CourseReadMask {
        include_course_state: Some(true),
        current_user_setting_read_mask: Some(wire::CourseUserSettingReadMask {
            include_notifications_muted: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub async fn load_class_notifications(session: &LiveSession, courses: Vec<i64>) -> Result<BTreeMap<i64, bool>, String> {
    let ids: BTreeSet<_> = courses.into_iter().filter(|&id| id > 0).collect();
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut request = crate::course_list_candidate();
    request.course_read_mask = Some(notification_read_mask());
    let response: wire::QueryCourseResponse = session.read_rpc("QueryCourse", request).await?;
    Ok(own_class_notifications(response, &ids))
}

pub(super) fn own_class_notifications(response: wire::QueryCourseResponse, courses: &BTreeSet<i64>) -> BTreeMap<i64, bool> {
    let mut settings = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for row in response.course {
        let Some(course) = row.course_id.and_then(|id| id.id).filter(|course| courses.contains(course)) else { continue; };
        if row.course_state != Some(1) {
            continue;
        }
        if !seen.insert(course) {
            settings.remove(&course);
            continue;
        }
        // On an active enrolled class, an unset preference keeps notifications enabled.
        settings.insert(course, row.current_user_setting.and_then(|setting| setting.notifications_muted).unwrap_or(false));
    }
    settings
}

pub(super) fn update_class_notifications_request(course: i64, muted: bool) -> mutation::WriteCourseRequest {
    mutation::WriteCourseRequest {
        request_header: Some(mutation::WriteRequestHeader { operation: Some(3) }),
        course_mutation: vec![mutation::CourseMutation {
            course_id: Some(classwork::CourseId { id: Some(course) }),
            course: Some(wire::Course {
                course_id: Some(wire::CourseId { id: Some(course) }),
                current_user_setting: Some(wire::CourseUserSetting { notifications_muted: Some(muted), ..Default::default() }),
                ..Default::default()
            }.encode_to_vec()),
            course_update_mask: Some(mutation::CourseUpdateMask {
                current_user_setting_update_mask: Some(mutation::CourseUserSettingUpdateMask {
                    update_notifications_muted: Some(true), ..Default::default()
                }),
                ..Default::default()
            }.encode_to_vec()),
            ..Default::default()
        }],
        course_read_mask: Some(notification_read_mask().encode_to_vec()),
    }
}

pub async fn set_class_notifications(
    session: &LiveSession, course: i64, muted: bool,
) -> Result<bool, String> {
    check_class(course)?;
    let before = load_class_notifications(session, vec![course]).await?;
    let Some(&current) = before.get(&course) else {
        return Err("Classroom did not return your notification setting for this class.".into());
    };
    if current == muted {
        return Ok(current);
    }
    let _: mutation::WriteCourseResponse = session.write_rpc(
        "WriteCourse", update_class_notifications_request(course, muted), Some(course),
    ).await?;
    let after = load_class_notifications(session, vec![course]).await?;
    if after.get(&course) != Some(&muted) {
        return Err("Classroom did not confirm the notification change.".into());
    }
    Ok(muted)
}
