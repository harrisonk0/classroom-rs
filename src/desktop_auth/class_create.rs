use prost::Message;

use super::{LiveSession, classwork, wire};

const UNCERTAIN: &str =
    "Classroom did not confirm the new class. Check Home before trying to create it again.";

#[derive(Debug, Clone, Default)]
pub struct NewClass {
    pub name: String,
    pub section: String,
    pub subject: String,
    pub room: String,
}

pub fn class_input_valid(input: &NewClass) -> bool {
    let name = input.name.trim();
    !name.is_empty()
        && [(name, 750), (input.section.trim(), 2_800), (input.subject.trim(), 750), (input.room.trim(), 650)]
            .into_iter().all(|(text, max)| text.chars().count() <= max && !text.chars().any(char::is_control))
}

pub async fn can_create_classes(session: &LiveSession) -> Result<bool, String> {
    read_class_creation_permission(session).await.map(|owner| owner.is_some())
}

pub async fn create_class(session: &LiveSession, input: NewClass) -> Result<i64, String> {
    if !class_input_valid(&input) {
        return Err("Enter a class name and check the lengths of the class details.".into());
    }
    let owner = read_class_creation_permission(session).await?
        .ok_or("Classroom has disabled class creation for this account.")?;
    let request = create_request(&input, owner);
    let course: CourseFields = session.write_rpc("CreateCourse", request, None).await
        .map_err(|error| format!("{error} Check Home before trying to create the class again."))?;
    let id = course.course_id.and_then(|id| id.id).filter(|id| *id > 0).ok_or(UNCERTAIN)?;
    let response: CreatedCourseQueryResponse = session.rpc("QueryCourse", lookup_request(id), Some(id)).await
        .map_err(|_| UNCERTAIN)?;
    let [course] = response.course.as_slice() else { return Err(UNCERTAIN.into()); };
    confirms_created_course(course, &input, id, owner).then_some(id).ok_or_else(|| UNCERTAIN.into())
}

async fn read_class_creation_permission(session: &LiveSession) -> Result<Option<i64>, String> {
    let reply: PermissionQueryResponse = session.read_rpc("QueryUser", permission_request()).await?;
    class_creation_permission(reply)
}

fn class_creation_permission(reply: PermissionQueryResponse) -> Result<Option<i64>, String> {
    let [user] = reply.user.as_slice() else {
        return Err("Classroom did not identify the signed-in account.".into());
    };
    let id = user.user_id.as_ref().and_then(|id| id.id).filter(|id| *id > 0)
        .ok_or("Classroom did not identify the signed-in account.")?;
    Ok(user.can_create_courses.unwrap_or(false).then_some(id))
}

fn permission_request() -> PermissionQueryRequest {
    PermissionQueryRequest {
        read_mask: Some(PermissionReadMask { include_can_create_courses: Some(true) }),
        filter_options: Some(classwork::UserFilterOptions {
            base_filter_options: Some(classwork::FilterOptions {
                user_filter: vec![classwork::UserFilter { use_current_user: Some(true), user_id: None }],
            }),
        }),
    }
}

fn create_request(input: &NewClass, owner: i64) -> CreateCourseRequest {
    let optional = |value: &str| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_owned())
    };
    CreateCourseRequest {
        course: Some(CourseFields {
            title: Some(input.name.trim().to_owned()),
            owner: vec![classwork::UserId { id: Some(owner) }],
            section: optional(&input.section),
            room: optional(&input.room),
            subject: optional(&input.subject).into_iter().map(|display_name| CourseSubject {
                subject_id: Some(CourseSubjectId { subject_type: Some(2) }),
                display_name: Some(display_name),
            }).collect(),
            ..Default::default()
        }),
        read_mask: Some(created_course_read_mask()),
    }
}

fn lookup_request(id: i64) -> CreatedCourseQueryRequest {
    CreatedCourseQueryRequest {
        read_mask: Some(created_course_read_mask()),
        filter_options: Some(wire::CourseFilterOptions {
            base_filter_options: Some(wire::FilterOptions {
                user_filter: vec![wire::UserFilter { use_current_user: Some(true) }],
            }),
            course_id: vec![wire::CourseId { id: Some(id) }],
            ..Default::default()
        }),
    }
}

fn created_course_read_mask() -> CreatedCourseReadMask {
    CreatedCourseReadMask {
        include_title: Some(true),
        include_owner: Some(true),
        include_section: Some(true),
        include_room: Some(true),
        include_state: Some(true),
        subject_read_mask: Some(CourseSubjectReadMask {
            include_display_name: Some(true),
            include_subject_type: Some(true),
        }),
    }
}

fn confirms_created_course(course: &CourseFields, input: &NewClass, id: i64, owner: i64) -> bool {
    let subject = input.subject.trim();
    course.course_id.as_ref().and_then(|id| id.id) == Some(id)
        && course.owner.iter().any(|user| user.id == Some(owner))
        && matches!(course.state, Some(1 | 3)) // ACTIVE or PROVISIONED
        && course.title.as_deref() == Some(input.name.trim())
        && course.section.as_deref().unwrap_or_default() == input.section.trim()
        && course.room.as_deref().unwrap_or_default() == input.room.trim()
        && (subject.is_empty() || course.subject.iter().any(|entry| entry.display_name.as_deref() == Some(subject)))
}

// Only the current user's ID and explicit canCreateCourses bit are needed.
// UserReadMask builder 0x19807b8–0x19807c4 and User builder 0x1974dac–0x1974db8.
#[derive(Clone, PartialEq, Message)]
struct PermissionQueryRequest {
    #[prost(message, optional, tag = "2")]
    read_mask: Option<PermissionReadMask>,
    #[prost(message, optional, tag = "3")]
    filter_options: Option<classwork::UserFilterOptions>,
}

#[derive(Clone, PartialEq, Message)]
struct PermissionReadMask {
    #[prost(bool, optional, tag = "8")]
    include_can_create_courses: Option<bool>,
}

#[derive(Clone, PartialEq, Message)]
struct PermissionQueryResponse {
    #[prost(message, repeated, tag = "2")]
    user: Vec<PermissionUser>,
}

#[derive(Clone, PartialEq, Message)]
struct PermissionUser {
    #[prost(message, optional, tag = "1")]
    user_id: Option<classwork::UserId>,
    #[prost(bool, optional, tag = "9")]
    can_create_courses: Option<bool>,
}

// CreateCourseRequest builder 0x198bec4, assembled at 0x1e754f8–0x1e75538.
// Course builder 0x1975a48. Subject is repeated and Android's custom branch
// writes its display name and SubjectId.SubjectType = CUSTOM (pool 0x20498).
// gRPC descriptor at 0x18045a8–0x18045cc loads PP+0x28908, whose type refs
// 84887 and 84277 identify CreateCourseRequest (class 5140) and Course (5387).
#[derive(Clone, PartialEq, Message)]
struct CreateCourseRequest {
    #[prost(message, optional, tag = "1")]
    course: Option<CourseFields>,
    #[prost(message, optional, tag = "2")]
    read_mask: Option<CreatedCourseReadMask>,
}

#[derive(Clone, PartialEq, Message)]
struct CourseFields {
    #[prost(message, optional, tag = "1")]
    course_id: Option<wire::CourseId>,
    #[prost(string, optional, tag = "6")]
    title: Option<String>,
    #[prost(message, repeated, tag = "7")]
    owner: Vec<classwork::UserId>,
    #[prost(string, optional, tag = "9")]
    section: Option<String>,
    #[prost(string, optional, tag = "15")]
    room: Option<String>,
    #[prost(int32, optional, tag = "21")]
    state: Option<i32>,
    #[prost(message, repeated, tag = "31")]
    subject: Vec<CourseSubject>,
}

#[derive(Clone, PartialEq, Message)]
struct CourseSubject {
    #[prost(message, optional, tag = "1")]
    subject_id: Option<CourseSubjectId>,
    #[prost(string, optional, tag = "2")]
    display_name: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
struct CourseSubjectId {
    #[prost(int32, optional, tag = "2")]
    subject_type: Option<i32>,
}

#[derive(Clone, PartialEq, Message)]
struct CreatedCourseReadMask {
    #[prost(bool, optional, tag = "5")]
    include_title: Option<bool>,
    #[prost(bool, optional, tag = "6")]
    include_owner: Option<bool>,
    #[prost(bool, optional, tag = "8")]
    include_section: Option<bool>,
    #[prost(bool, optional, tag = "14")]
    include_room: Option<bool>,
    #[prost(bool, optional, tag = "22")]
    include_state: Option<bool>,
    #[prost(message, optional, tag = "29")]
    subject_read_mask: Option<CourseSubjectReadMask>,
}

#[derive(Clone, PartialEq, Message)]
struct CourseSubjectReadMask {
    #[prost(bool, optional, tag = "1")]
    include_display_name: Option<bool>,
    #[prost(bool, optional, tag = "2")]
    include_subject_type: Option<bool>,
}

#[derive(Clone, PartialEq, Message)]
struct CreatedCourseQueryRequest {
    #[prost(message, optional, tag = "2")]
    read_mask: Option<CreatedCourseReadMask>,
    #[prost(message, optional, tag = "3")]
    filter_options: Option<wire::CourseFilterOptions>,
}

#[derive(Clone, PartialEq, Message)]
struct CreatedCourseQueryResponse {
    #[prost(message, repeated, tag = "2")]
    course: Vec<CourseFields>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_create_permission_requires_one_exact_current_user_and_explicit_grant() {
        let request = permission_request();
        assert_eq!(request.read_mask.unwrap().encode_to_vec(), [0x40, 0x01]);
        assert_eq!(request.filter_options.unwrap().base_filter_options.unwrap().user_filter,
            vec![classwork::UserFilter { use_current_user: Some(true), user_id: None }]);
        let mut user = PermissionUser { user_id: Some(classwork::UserId { id: Some(42) }), can_create_courses: Some(true) };
        assert_eq!(class_creation_permission(PermissionQueryResponse { user: vec![user.clone()] }), Ok(Some(42)));
        assert!(class_creation_permission(PermissionQueryResponse { user: vec![user.clone(), user.clone()] }).is_err());
        user.can_create_courses = None;
        assert_eq!(class_creation_permission(PermissionQueryResponse { user: vec![user] }), Ok(None));
    }

    #[test]
    fn class_create_payload_uses_android_title_owner_section_room_and_custom_subject_tags() {
        let input = NewClass { name: " Math ".into(), section: " P2 ".into(), subject: " Algebra ".into(), room: " 15 ".into() };
        assert!(class_input_valid(&input));
        let request = create_request(&input, 42);
        let course = request.course.unwrap();
        assert_eq!(course.encode_to_vec(), [
            0x32, 0x04, b'M', b'a', b't', b'h',
            0x3a, 0x02, 0x08, 0x2a,
            0x4a, 0x02, b'P', b'2',
            0x7a, 0x02, b'1', b'5',
            0xfa, 0x01, 0x0d, 0x0a, 0x02, 0x10, 0x02, 0x12, 0x07,
            b'A', b'l', b'g', b'e', b'b', b'r', b'a',
        ]);
        assert_eq!(lookup_request(99).filter_options.unwrap().course_id, vec![wire::CourseId { id: Some(99) }]);
    }

    #[test]
    fn class_create_confirmation_checks_id_owner_details_and_state() {
        let input = NewClass { name: "Math".into(), ..Default::default() };
        let mut course = create_request(&input, 42).course.unwrap();
        course.course_id = Some(wire::CourseId { id: Some(99) });
        course.state = Some(1);
        assert!(confirms_created_course(&course, &input, 99, 42));
        assert!(!confirms_created_course(&course, &input, 98, 42));
        assert!(!confirms_created_course(&course, &input, 99, 43));
        course.state = Some(2);
        assert!(!confirms_created_course(&course, &input, 99, 42));
        assert!(!class_input_valid(&NewClass { name: "   ".into(), ..Default::default() }));
    }
}
