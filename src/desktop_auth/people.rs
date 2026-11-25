use super::{
    wire, classwork, check_class, https_host, LiveSession, artwork_host_is_allowed, BTreeMap,
    BTreeSet, CourseMember, CourseRole, CourseState, Message,
};

pub async fn load_roster(session: &LiveSession, course_id: i64) -> Result<Vec<CourseMember>, String> {
    check_class(course_id)?;
    let mut request = crate::course_list_candidate();
    let filter = request.filter_options.as_mut().ok_or("Missing People filter.")?;
    filter.base_filter_options = None;
    filter.course_id = vec![wire::CourseId { id: Some(course_id) }];
    request.course_read_mask = Some(wire::CourseReadMask {
        include_student_user_id: Some(true),
        include_teacher_user_id: Some(true),
        ..Default::default()
    });
    let response: wire::QueryCourseResponse = session.rpc("QueryCourse", request, Some(course_id)).await?;
    let matching = response.course.into_iter()
        .find(|course| course.course_id.as_ref().and_then(|id| id.id) == Some(course_id))
        .ok_or("Google did not return the requested class roster.")?;
    let mut roles = BTreeMap::new();
    for (role, identifiers) in [
        (CourseRole::Student, matching.student_user_id),
        (CourseRole::Teacher, matching.teacher_user_id),
    ] {
        for bytes in identifiers {
            if let Ok(user) = classwork::UserId::decode(bytes.as_slice())
                && let Some(id) = user.id.filter(|id| *id > 0)
            {
                roles.insert(id, role);
            }
        }
    }
    if roles.len() > 500 {
        return Err("This class has too many members for one roster request.".into());
    }
    if roles.is_empty() {
        return Err("Google did not provide a visible roster for this class.".into());
    }
    let names = load_user_names(session, roles.keys().copied().collect()).await?;
    Ok(roles.into_iter().filter_map(|(id, role)| {
        Some(CourseMember { id: id.to_string(), display_name: names.get(&id)?.clone(), role })
    }).collect())
}

pub(super) async fn load_user_names(session: &LiveSession, ids: BTreeSet<i64>) -> Result<BTreeMap<i64, String>, String> {
    Ok(load_user_profiles(session, ids, false).await?.into_iter()
        .filter_map(|(id, profile)| Some((id, profile.name?))).collect())
}

#[derive(Clone)]
pub struct UserProfile {
    pub name: Option<String>,
    pub photo_url: Option<String>,
}

pub async fn load_stream_profiles(session: &LiveSession, ids: BTreeSet<i64>) -> Result<BTreeMap<i64, UserProfile>, String> {
    load_user_profiles(session, ids, true).await
}

#[cfg(test)]
#[test]
fn stream_profile_photo_fields_match_the_wire_tags() {
    let mask = classwork::UserReadMask {
        include_name: Some(true), include_photo_url: Some(true), ..Default::default()
    };
    assert_eq!(mask.encode_to_vec(), [0x08, 0x01, 0x20, 0x01]);
    let user = classwork::User::decode(&[0x2a, 0x03, b'p', b'i', b'c'][..]).unwrap();
    assert_eq!(user.photo_url.as_deref(), Some("pic"));
}

async fn load_user_profiles(session: &LiveSession, ids: BTreeSet<i64>, photos: bool) -> Result<BTreeMap<i64, UserProfile>, String> {
    use classwork::*;
    if ids.is_empty() { return Ok(BTreeMap::new()) }
    let response: QueryUserResponse = session.read_rpc("QueryUser", QueryUserRequest {
        user_read_mask: Some(UserReadMask { include_name: Some(true), include_photo_url: photos.then_some(true), ..Default::default() }),
        filter_options: Some(UserFilterOptions {
            base_filter_options: Some(FilterOptions {
                user_filter: ids.iter().map(|id| UserFilter {
                    use_current_user: None,
                    user_id: Some(UserId { id: Some(*id) }),
                }).collect(),
            }),
        }),
        ..Default::default()
    }).await?;
    Ok(response.user.into_iter().filter_map(|user| {
        let id = user.user_id?.id?;
        if !ids.contains(&id) { return None; }
        let photo_url = user.photo_url.map(|url| if url.starts_with("//") { format!("https:{url}") } else { url })
            .filter(|url| photos && artwork_host_is_allowed(url));
        Some((id, UserProfile { name: user.name.filter(|name| !name.trim().is_empty()), photo_url }))
    }).collect())
}

pub async fn load_class_teacher_names(session: &LiveSession, courses: &[(i64, Vec<i64>)]) -> Result<BTreeMap<i64, String>, String> {
    let ids = courses.iter().flat_map(|(_, ids)| ids.iter().copied()).collect();
    let names = load_user_names(session, ids).await?;
    Ok(class_teacher_names(courses, &names))
}

pub(super) fn class_teacher_names(courses: &[(i64, Vec<i64>)], names: &BTreeMap<i64, String>) -> BTreeMap<i64, String> {
    courses.iter().filter_map(|(course, owners)| {
        owners.iter().filter_map(|owner| names.get(owner).map(|name| name.trim()))
            .find(|name| !name.is_empty()).map(|name| (*course, name.to_owned()))
    }).collect()
}

pub(super) async fn current_user_id(session: &LiveSession) -> Result<i64, String> {
    let response: classwork::QueryUserResponse = session.read_rpc("QueryUser", classwork::QueryUserRequest {
        user_read_mask: Some(classwork::UserReadMask { include_name: Some(true), ..Default::default() }),
        filter_options: Some(classwork::UserFilterOptions {
            base_filter_options: Some(classwork::FilterOptions {
                user_filter: vec![classwork::UserFilter { use_current_user: Some(true), user_id: None }],
            }),
        }),
        ..Default::default()
    }).await?;
    match response.user.as_slice() {
        [user] => user.user_id.as_ref().and_then(|id| id.id).filter(|id| *id > 0)
            .ok_or_else(|| "Google did not provide the signed-in Classroom user ID.".into()),
        _ => Err("Google did not identify a single signed-in Classroom user.".into()),
    }
}

pub async fn check_current_user(session: &LiveSession) -> Result<(), String> {
    current_user_id(session).await.map(|_| ())
}

pub async fn own_work_link(
    session: &LiveSession, course: i64,
) -> Result<crate::classroom_links::ClassroomLink, String> {
    check_class(course)?;
    let student = current_user_id(session).await?;
    Ok(crate::classroom_links::ClassroomLink::YourWork { course, student })
}

pub(super) async fn require_class_moderator(session: &LiveSession, course: i64) -> Result<(), String> {
    check_class(course)?;
    if !session.snapshot.courses.iter().any(|entry| entry.id == course && entry.state == CourseState::Active) {
        return Err("Choose an active class.".into());
    }
    let details = load_own_class_details(session, vec![course]).await?;
    if details.roles.get(&course) != Some(&CourseRole::Teacher) {
        return Err("Classroom did not confirm teacher access to moderate this class.".into());
    }
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct OwnClassDetails {
    pub folders: BTreeMap<i64, String>,
    pub roles: BTreeMap<i64, CourseRole>,
}

pub async fn load_class_folders(
    session: &LiveSession, courses: Vec<i64>,
) -> Result<BTreeMap<i64, String>, String> {
    Ok(load_own_class_details(session, courses).await?.folders)
}

pub async fn load_own_class_details(
    session: &LiveSession, courses: Vec<i64>,
) -> Result<OwnClassDetails, String> {
    let courses: BTreeSet<_> = courses.into_iter().filter(|id| *id > 0).collect();
    if courses.is_empty() {
        return Ok(OwnClassDetails::default());
    }
    let own_user = current_user_id(session).await?;
    let ids = courses.iter().map(|id| classwork::CourseUserId {
        course_id: Some(classwork::CourseId { id: Some(*id) }),
        user_id: Some(classwork::UserId { id: Some(own_user) }),
    }).collect();
    let response: classwork::CourseUserLookupResponse = session.read_rpc(
        "LookupCourseUser",
        classwork::CourseUserLookupRequest {
            course_user_ids: ids,
            read_mask: Some(classwork::CourseUserReadMask {
                include_course_role: Some(true),
                course_folder_read_mask: Some(classwork::FolderReadMask { include_open_url: Some(true) }),
                ..Default::default()
            }),
        },
    ).await?;
    Ok(own_class_details(response, &courses, own_user))
}

pub(super) fn own_class_details(
    response: classwork::CourseUserLookupResponse, requested: &BTreeSet<i64>, own_user: i64,
) -> OwnClassDetails {
    let mut details = OwnClassDetails::default();
    let mut seen = BTreeSet::new();
    for row in response.results {
        let (Some(key), Some(entry)) = (row.course_user_id, row.course_user) else { continue };
        let Some(id) = key.course_id.as_ref().and_then(|key| key.id) else { continue };
        if key.user_id.as_ref().and_then(|user| user.id) != Some(own_user)
            || entry.course_user_id.as_ref() != Some(&key) || !requested.contains(&id)
        {
            continue;
        }
        if !seen.insert(id) {
            details.folders.remove(&id);
            details.roles.remove(&id);
            continue;
        }
        if let Some(role) = match entry.course_role {
            Some(2) => Some(CourseRole::Teacher),
            Some(3) => Some(CourseRole::Student),
            _ => None,
        } {
            details.roles.insert(id, role);
        }
        if let Some(url) = entry.course_folder.and_then(|folder| folder.open_url)
            && https_host(&url).as_deref() == Some("drive.google.com")
        {
            details.folders.insert(id, url);
        }
    }
    details
}

pub(super) fn course_member_key(course: i64, own_user: i64) -> classwork::CourseUserId {
    classwork::CourseUserId {
        course_id: Some(classwork::CourseId { id: Some(course) }),
        user_id: Some(classwork::UserId { id: Some(own_user) }),
    }
}
