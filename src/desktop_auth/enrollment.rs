use super::{
    wire, classwork, mutation, check_class, LiveSession, BTreeSet, Course, current_user_id,
    Message,
};

pub fn valid_class_code(code: &str) -> bool {
    let code = code.trim();
    (6..=8).contains(&code.len()) && code.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

pub async fn load_invitations(session: &LiveSession) -> Result<Vec<Course>, String> {
    let user_id = current_user_id(session).await?;
    invitations_for(session, user_id).await
}

async fn invitations_for(session: &LiveSession, user_id: i64) -> Result<Vec<Course>, String> {
    let request = wire::QueryCourseRequest {
        course_read_mask: Some(wire::CourseReadMask {
            include_title: Some(true),
            include_subtitle: Some(true),
            include_invited_student_user_id: Some(true),
            ..Default::default()
        }),
        filter_options: Some(wire::CourseFilterOptions {
            base_filter_options: Some(wire::FilterOptions {
                user_filter: vec![wire::UserFilter { use_current_user: Some(true) }],
            }),
            latest_enrollment_event: vec![3], // INVITED
            ..Default::default()
        }),
    };
    let response: wire::QueryCourseResponse = session.read_rpc("QueryCourse", request).await?;
    let returned = response.course.len();
    let mut seen = BTreeSet::new();
    let matched: Vec<_> = response.course.into_iter().filter(|course| {
        course.invited_student_user_id.iter().any(|bytes| {
            classwork::UserId::decode(bytes.as_slice()).is_ok_and(|id| id.id == Some(user_id))
        })
    }).filter_map(Course::from_wire).filter(|course| course.id > 0 && seen.insert(course.id)).collect();
    if returned != 0 && matched.is_empty() {
        return Err("Classroom returned no invitation that matches this student's identity.".into());
    }
    Ok(matched)
}

pub async fn join_class(session: &LiveSession, code: String) -> Result<i64, String> {
    if !valid_class_code(&code) {
        return Err("Enter the 6 to 8 character code from your teacher.".into());
    }
    let user_id = current_user_id(session).await?;
    let response: mutation::AddUserToCourseResponse = session.write_rpc(
        "AddUserToCourse", mutation::AddUserToCourseRequest {
            course_id: None,
            enrollment_code: Some(code.trim().to_owned()),
            user_id: Some(classwork::UserId { id: Some(user_id) }),
            course_role: Some(3),
            read_mask: Some(classwork::CourseUserReadMask { include_course_role: Some(true), ..Default::default() }),
        }, None,
    ).await.map_err(|error| format!("{error} Refresh Home before trying to join again."))?;
    response.course_user.and_then(|member| member.course_user_id)
        .filter(|id| id.user_id.as_ref().and_then(|user| user.id) == Some(user_id))
        .and_then(|id| id.course_id.and_then(|course| course.id))
        .filter(|id| *id > 0)
        .ok_or_else(|| "Google did not confirm enrollment. Refresh Home before trying again.".into())
}

pub async fn accept_invitation(session: &LiveSession, course_id: i64) -> Result<(), String> {
    check_class(course_id)?;
    let user_id = current_user_id(session).await?;
    if !invitations_for(session, user_id).await?.iter().any(|course| course.id == course_id) {
        return Err("This invitation is no longer pending. Refresh the invitation list.".into());
    }
    let response: mutation::AddUserToCourseResponse = session.write_rpc(
        "AddUserToCourse", mutation::AddUserToCourseRequest {
            course_id: Some(classwork::CourseId { id: Some(course_id) }),
            enrollment_code: None,
            user_id: Some(classwork::UserId { id: Some(user_id) }),
            course_role: Some(3),
            read_mask: Some(classwork::CourseUserReadMask { include_course_role: Some(true), ..Default::default() }),
        }, Some(course_id),
    ).await.map_err(|error| format!("{error} Refresh invitations before trying again."))?;
    let confirmed = response.course_user.and_then(|member| member.course_user_id)
        .is_some_and(|id| id.course_id.and_then(|course| course.id) == Some(course_id)
            && id.user_id.and_then(|user| user.id) == Some(user_id));
    if confirmed { Ok(()) } else {
        Err("Classroom did not confirm enrollment. Refresh Home and invitations before trying again.".into())
    }
}

pub(super) fn decline_invitation_request(course_id: i64, user_id: i64) -> mutation::DeleteCourseInvitationRequest {
    mutation::DeleteCourseInvitationRequest {
        course_invitation_id: Some(mutation::CourseInvitationId {
            course_id: Some(classwork::CourseId { id: Some(course_id) }),
            id: Some(user_id),
        }),
        invited_role: Some(mutation::InvitedRole::Student.into()),
    }
}

pub async fn decline_invitation(session: &LiveSession, course_id: i64) -> Result<Vec<Course>, String> {
    check_class(course_id)?;
    let user_id = current_user_id(session).await?;
    if !invitations_for(session, user_id).await?.iter().any(|course| course.id == course_id) {
        return Err("This invitation is no longer pending.".into());
    }
    // Android discards the response body. Confirm by reading the pending list.
    let _: () = session.write_rpc(
        "DeleteCourseInvitation", decline_invitation_request(course_id, user_id), Some(course_id),
    ).await?;
    let remaining = invitations_for(session, user_id).await
        .map_err(|error| format!("Couldn’t confirm that the invitation was declined. {error}"))?;
    if remaining.iter().any(|course| course.id == course_id) {
        return Err("Classroom still shows this invitation as pending.".into());
    }
    Ok(remaining)
}

pub async fn leave_class(session: &LiveSession, course_id: i64) -> Result<(), String> {
    check_class(course_id)?;
    let user_id = current_user_id(session).await?;
    let response: wire::QueryCourseResponse = session.rpc(
        "QueryCourse", wire::QueryCourseRequest {
            course_read_mask: Some(wire::CourseReadMask {
                include_student_user_id: Some(true),
                include_teacher_user_id: Some(true),
                ..Default::default()
            }),
            filter_options: Some(wire::CourseFilterOptions {
                base_filter_options: None,
                course_id: vec![wire::CourseId { id: Some(course_id) }],
                ..Default::default()
            }),
        }, Some(course_id),
    ).await?;
    let me = |encoded: &Vec<u8>| {
        classwork::UserId::decode(encoded.as_slice()).is_ok_and(|user| user.id == Some(user_id))
    };
    let is_student = response.course.into_iter().find(|course| {
        course.course_id.as_ref().and_then(|id| id.id) == Some(course_id)
    }).is_some_and(|course| {
        course.student_user_id.iter().any(me) && !course.teacher_user_id.iter().any(me)
    });
    if !is_student {
        return Err("Google did not confirm that you are enrolled as a student in this class.".into());
    }
    let _: mutation::RemoveUserFromCourseResponse = session.write_rpc(
        "RemoveUserFromCourse", mutation::RemoveUserFromCourseRequest {
            course_id: Some(classwork::CourseId { id: Some(course_id) }),
            user_id: Some(classwork::UserId { id: Some(user_id) }),
            course_role: Some(3),
        }, Some(course_id),
    ).await.map_err(|error| format!("{error} Refresh Home before trying again."))?;
    Ok(())
}
