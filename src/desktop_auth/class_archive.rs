use super::{LiveSession, check_class, classwork, load_own_class_details, mutation, wire};
use crate::desktop_data::{CourseRole, CourseState};
use prost::Message;

// CourseMutation carries the encoded mask, so only the field used by this action is needed.
#[derive(Clone, PartialEq, Message)]
struct CourseArchiveUpdateMask {
    #[prost(bool, optional, tag = "16")]
    update_course_state: Option<bool>,
}

pub async fn set_class_archived(
    session: &LiveSession,
    course: i64,
    archived: bool,
) -> Result<CourseState, String> {
    check_class(course)?;
    let membership = load_own_class_details(session, vec![course]).await?;
    if membership.roles.get(&course) != Some(&CourseRole::Teacher) {
        return Err("Classroom did not confirm teacher access to this class.".into());
    }

    let current = read_class_state(session, course).await?;
    let (expected, desired, desired_wire) = if archived {
        (CourseState::Active, CourseState::Archived, 2)
    } else {
        (CourseState::Archived, CourseState::Active, 1)
    };
    if current != expected {
        return Err("The class is no longer in the expected state. Refresh the class list first.".into());
    }

    let _: mutation::WriteCourseResponse = session
        .write_rpc("WriteCourse", archive_request(course, desired_wire), Some(course))
        .await
        .map_err(|error| format!("{error} Refresh the class list before retrying."))?;
    let confirmed = read_class_state(session, course)
        .await
        .map_err(|error| format!("{error} Refresh the class list before retrying."))?;
    if confirmed != desired {
        return Err("Classroom did not confirm the requested class state. Refresh the class list before retrying.".into());
    }
    Ok(confirmed)
}

pub async fn read_class_state(session: &LiveSession, course: i64) -> Result<CourseState, String> {
    check_class(course)?;
    let response = session
        .rpc("QueryCourse", class_state_request(course), Some(course))
        .await?;
    exact_class_state(response, course)
}

fn class_state_mask() -> wire::CourseReadMask {
    wire::CourseReadMask {
        include_course_state: Some(true),
        ..Default::default()
    }
}

fn class_state_request(course: i64) -> wire::QueryCourseRequest {
    wire::QueryCourseRequest {
        course_read_mask: Some(class_state_mask()),
        filter_options: Some(wire::CourseFilterOptions {
            course_id: vec![wire::CourseId { id: Some(course) }],
            ..Default::default()
        }),
    }
}

fn exact_class_state(response: wire::QueryCourseResponse, course: i64) -> Result<CourseState, String> {
    let [entry] = response.course.as_slice() else {
        return Err("Classroom did not return exactly one class.".into());
    };
    if entry.course_id.as_ref().and_then(|id| id.id) != Some(course) {
        return Err("Classroom returned a different class.".into());
    }
    match CourseState::from_wire(entry.course_state) {
        state @ (CourseState::Active | CourseState::Archived) => Ok(state),
        _ => Err("Classroom did not confirm an active or archived class.".into()),
    }
}

fn archive_request(course: i64, state: i32) -> mutation::WriteCourseRequest {
    mutation::WriteCourseRequest {
        request_header: Some(mutation::WriteRequestHeader { operation: Some(3) }),
        course_mutation: vec![mutation::CourseMutation {
            course_id: Some(classwork::CourseId { id: Some(course) }),
            course: Some(wire::Course {
                course_id: Some(wire::CourseId { id: Some(course) }),
                course_state: Some(state),
                ..Default::default()
            }.encode_to_vec()),
            course_update_mask: Some(CourseArchiveUpdateMask {
                update_course_state: Some(true),
            }.encode_to_vec()),
            ..Default::default()
        }],
        course_read_mask: Some(class_state_mask().encode_to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, state: Option<i32>) -> wire::Course {
        wire::Course {
            course_id: Some(wire::CourseId { id: Some(id) }),
            course_state: state,
            ..Default::default()
        }
    }

    #[test]
    fn archive_and_restore_encode_one_course_and_only_the_state_update_mask() {
        for state in [1, 2] {
            let request = archive_request(29, state);
            let decoded = mutation::WriteCourseRequest::decode(request.encode_to_vec().as_slice()).unwrap();
            assert_eq!(decoded.request_header.unwrap().operation, Some(3));
            let [course] = decoded.course_mutation.as_slice() else { panic!("Expected one course mutation"); };
            assert_eq!(course.course_id.as_ref().and_then(|id| id.id), Some(29));
            assert_eq!(wire::Course::decode(course.course.as_deref().unwrap()).unwrap(), row(29, Some(state)));
            assert_eq!(course.course_update_mask.as_deref(), Some([0x80, 0x01, 0x01].as_slice()));
            assert_eq!(wire::CourseReadMask::decode(decoded.course_read_mask.as_deref().unwrap()).unwrap(), class_state_mask());
        }
    }

    #[test]
    fn state_readback_is_scoped_to_the_requested_class_and_rejects_ambiguity() {
        let request = class_state_request(29);
        assert_eq!(request.filter_options.unwrap().course_id, [wire::CourseId { id: Some(29) }]);
        assert_eq!(request.course_read_mask, Some(class_state_mask()));
        assert_eq!(exact_class_state(wire::QueryCourseResponse { course: vec![row(29, Some(1))] }, 29), Ok(CourseState::Active));
        assert_eq!(exact_class_state(wire::QueryCourseResponse { course: vec![row(29, Some(2))] }, 29), Ok(CourseState::Archived));
        for course in [vec![], vec![row(30, Some(2))], vec![row(29, None)], vec![row(29, Some(4))],
            vec![row(29, Some(2)), row(29, Some(2))]]
        {
            assert!(exact_class_state(wire::QueryCourseResponse { course }, 29).is_err());
        }
    }
}
