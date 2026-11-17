use super::{LiveSession, DriveFileAttachment, check_class, classwork, course_member_key, coursework, current_user_id, decode_description_bullets, decode_description_styles, https_host,
    link_material, material, material_matches, mutation, require_class_moderator, upload_drive_file, wire};
use crate::desktop_data::{CourseState, Coursework, DescriptionStyle};
use prost::Message;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamAccess {
    PostAndComment,
    CommentOnly,
    ReadOnly,
}

impl StreamAccess {
    pub fn can_comment(self) -> bool {
        matches!(self, Self::PostAndComment | Self::CommentOnly)
    }
}

pub async fn load_stream_access(
    session: &LiveSession, courses: Vec<i64>,
) -> Result<BTreeMap<i64, StreamAccess>, String> {
    let courses = courses.into_iter().filter(|id| *id > 0).collect::<BTreeSet<_>>();
    if courses.is_empty() {
        return Ok(BTreeMap::new());
    }
    let user = current_user_id(session).await?;
    stream_access_for(session, &courses, user).await
}

pub async fn load_stream_permission(session: &LiveSession, course: i64) -> Result<(i64, StreamAccess), String> {
    check_class(course)?;
    let user = current_user_id(session).await?;
    let access = stream_access_for(session, &BTreeSet::from([course]), user).await?
        .remove(&course).ok_or_else(|| "Classroom did not return your Stream permissions.".to_owned())?;
    Ok((user, access))
}

async fn stream_access_for(
    session: &LiveSession, courses: &BTreeSet<i64>, user: i64,
) -> Result<BTreeMap<i64, StreamAccess>, String> {
    let single_course = courses.first().copied().filter(|_| courses.len() == 1);
    let filter_options = if let Some(id) = single_course {
        wire::CourseFilterOptions {
            course_id: vec![wire::CourseId { id: Some(id) }],
            ..Default::default()
        }
    } else {
        wire::CourseFilterOptions {
            base_filter_options: Some(wire::FilterOptions {
                user_filter: vec![wire::UserFilter { use_current_user: Some(true) }],
            }),
            latest_enrollment_event: vec![2], // ADDED
            ..Default::default()
        }
    };
    let course_request = wire::QueryCourseRequest {
        course_read_mask: Some(wire::CourseReadMask {
            include_course_state: Some(true),
            include_muted_user_id: Some(true),
            course_setting_read_mask: Some(wire::CourseSettingReadMask {
                include_student_stream_posting_policy: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        }),
        filter_options: Some(filter_options),
    };
    let member_request = classwork::CourseUserLookupRequest {
        course_user_ids: courses.iter().map(|&id| course_member_key(id, user)).collect(),
        read_mask: Some(classwork::CourseUserReadMask {
            include_course_role: Some(true),
            ..Default::default()
        }),
    };
    let (course_rows, members) = tokio::try_join!(
        session.rpc("QueryCourse", course_request, single_course),
        session.read_rpc("LookupCourseUser", member_request),
    )?;
    Ok(decode_stream_access(course_rows, members, courses, user))
}

fn decode_stream_access(
    course_rows: wire::QueryCourseResponse, members: classwork::CourseUserLookupResponse,
    courses: &BTreeSet<i64>, user: i64,
) -> BTreeMap<i64, StreamAccess> {
    let mut roles = BTreeMap::new();
    for row in members.results {
        let Some(key) = row.course_user_id else { continue };
        let Some(id) = key.course_id.as_ref().and_then(|course| course.id) else { continue };
        let Some(member) = row.course_user else { continue };
        if !courses.contains(&id) || key != course_member_key(id, user) || member.course_user_id != Some(key) {
            continue;
        }
        roles.entry(id).and_modify(|value| *value = None).or_insert(member.course_role);
    }
    let mut access = BTreeMap::new();
    for course in course_rows.course {
        let Some(id) = course.course_id.as_ref().and_then(|course| course.id) else { continue };
        let policy = course.course_setting.and_then(|setting| setting.student_stream_posting_policy);
        let muted = course.muted_user_id.iter().try_fold(false, |muted, bytes| {
            let id = classwork::UserId::decode(bytes.as_slice()).ok()?.id?;
            Some(muted || id == user)
        });
        let value = match (course.course_state, roles.get(&id).copied().flatten(), muted, policy) {
            (Some(1), Some(2), _, _) => Some(StreamAccess::PostAndComment),
            (Some(1), Some(3), Some(false), Some(2)) => Some(StreamAccess::PostAndComment),
            (Some(1), Some(3), Some(false), Some(3)) => Some(StreamAccess::CommentOnly),
            (Some(1), Some(3), Some(true), _) | (Some(1), Some(3), _, Some(4)) => Some(StreamAccess::ReadOnly),
            (Some(2..=5), Some(2 | 3), _, _) => Some(StreamAccess::ReadOnly),
            _ => None,
        };
        if courses.contains(&id) {
            access.entry(id).and_modify(|entry| *entry = None).or_insert(value);
        }
    }
    access.into_iter().filter_map(|(id, access)| Some((id, access?))).collect()
}

#[derive(Clone)]
pub struct PublishedAnnouncement {
    pub item: Coursework,
    pub unconfirmed_attachments: usize,
    pub formatting_confirmed: bool,
}

pub async fn upload_stream_file(session: &LiveSession, course: i64, path: PathBuf) -> Result<DriveFileAttachment, String> {
    let (_, access) = load_stream_permission(session, course).await?;
    if access != StreamAccess::PostAndComment {
        return Err("Classroom has not allowed you to post in this class.".into());
    }
    upload_drive_file(session, path).await
}

pub async fn post_stream_announcement(
    session: &LiveSession, course: i64, body: String, styles: Vec<DescriptionStyle>, bullets: Vec<usize>, links: Vec<String>, files: Vec<DriveFileAttachment>,
) -> Result<PublishedAnnouncement, String> {
    check_class(course)?;
    if body.trim().is_empty() || body.trim().len() > 10_000 {
        return Err("Enter a message under 10,000 bytes.".into());
    }
    let rich = rich_post_body(&body, &styles, &bullets);
    let body = body.trim();
    let mut seen = BTreeSet::new();
    let links: Vec<_> = links.iter().map(|url| url.trim()).filter(|url| seen.insert(*url)).collect();
    if links.iter().any(|url| https_host(url).is_none()) {
        return Err("Enter valid HTTPS links for the announcement.".into());
    }
    let materials: Vec<_> = announcement_materials(links, files);
    let own_user = current_user_id(session).await?;
    let access = stream_access_for(session, &BTreeSet::from([course]), own_user).await?;
    if access.get(&course) != Some(&StreamAccess::PostAndComment) {
        return Err("Classroom has not allowed you to post in this class.".into());
    }
    let response = session.write_rpc(
        "InsertStreamItem", announcement_request(course, own_user, body, rich.clone(), &materials), Some(course),
    ).await.map_err(|error| format!("{error} Check the Stream before posting again."))?;
    let (item, returned_materials) = confirm_announcement(response, course, own_user, body)?;
    let expected_styles = rich.as_ref().map(|rich| decode_description_styles(rich, body));
    let expected_bullets = rich.as_ref().map_or_else(Vec::new, |rich| decode_description_bullets(rich, body));
    let matches_formatting = |item: &Coursework| expected_styles.as_deref().is_none_or(|expected| {
        expected == item.description_styles && expected_bullets == item.description_bullets
    });
    let mut published = PublishedAnnouncement {
        formatting_confirmed: matches_formatting(&item),
        item, unconfirmed_attachments: count_missing_attachments(&materials, &returned_materials),
    };
    if (published.unconfirmed_attachments > 0 || !published.formatting_confirmed)
        && let Ok(id) = published.item.id.parse()
        && let Ok(Some(info)) = lookup_stream_post_with_mask(session, course, &post_key(course, id), post_read_mask(!materials.is_empty(), rich.is_some())).await
        && info.creator_user_id.and_then(|id| id.id) == Some(own_user)
    {
        published.unconfirmed_attachments = count_missing_attachments(&materials, &info.material);
        published.item.attachments = info.material.into_iter().filter_map(material).collect();
        if let Some(rich) = info.description_rich.filter(|rich| rich.text.as_deref() == Some(body)) {
            published.item.description_styles = decode_description_styles(&rich, body);
            published.item.description_bullets = decode_description_bullets(&rich, body);
            published.formatting_confirmed = matches_formatting(&published.item);
        }
    }
    Ok(published)
}

fn announcement_materials(links: Vec<&str>, files: Vec<DriveFileAttachment>) -> Vec<classwork::Material> {
    links.into_iter().map(link_material).chain(files.into_iter().map(|file| file.0)).collect()
}

fn count_missing_attachments(sent: &[classwork::Material], returned: &[classwork::Material]) -> usize {
    sent.iter().filter(|material| !returned.iter().any(|item| material_matches(material, item))).count()
}

fn rich_post_body(body: &str, styles: &[DescriptionStyle], bullets: &[usize]) -> Option<classwork::RichText> {
    let text = body.trim();
    let start = body.len() - body.trim_start().len();
    let end = start + text.len();
    let mut retained = BTreeMap::new();
    for style in styles.iter().filter(|style| style.start < end && body.is_char_boundary(style.start)) {
        retained.insert(style.start.max(start), *style);
    }
    let first_paragraph = body[..start].rfind('\n').map_or(0, |newline| newline + 1);
    let bullet_starts = bullets.iter().copied().filter(|&offset| {
        (first_paragraph..end).contains(&offset) && (offset == 0 || body.as_bytes().get(offset - 1) == Some(&b'\n'))
    }).map(|offset| offset.max(start)).collect::<BTreeSet<_>>();
    if bullet_starts.is_empty() && retained.values().all(|style| style.is_plain()) {
        return None;
    }
    let mut previous = start;
    let mut offset = 0;
    let style = retained.into_iter().map(|(position, style)| {
        offset += body[previous..position].encode_utf16().count() as i64;
        previous = position;
        classwork::InlineStyle {
            r#type: [(style.bold, 2), (style.italic, 3), (style.underline, 4)].into_iter()
                .filter_map(|(enabled, value)| enabled.then_some(value)).collect(),
            start: Some(offset),
        }
    }).collect();
    let mut previous = start;
    let mut offset = 0;
    let block_style = bullet_starts.into_iter().map(|position| {
        offset += body[previous..position].encode_utf16().count() as i64;
        previous = position;
        classwork::BlockStyle { r#type: Some(1), start: Some(offset) }
    }).collect();
    Some(classwork::RichText { text: Some(text.into()), style, block_style })
}

fn announcement_request(
    course: i64, own_user: i64, body: &str, rich: Option<classwork::RichText>, materials: &[classwork::Material],
) -> mutation::StreamItemInsertRequest {
    let formatted = rich.is_some();
    mutation::StreamItemInsertRequest {
        stream_item: vec![classwork::StreamItem {
            stream_type: Some(3), // POST
            post: Some(classwork::Post {
                stream_item_info: Some(classwork::StreamItemInfo {
                    stream_item_id: Some(classwork::StreamItemId {
                        id: None,
                        course_id: Some(classwork::CourseId { id: Some(course) }),
                    }),
                    creator_user_id: Some(classwork::UserId { id: Some(own_user) }),
                    description: Some(body.into()),
                    description_rich: rich,
                    material: materials.to_vec(),
                    status: Some(2), // ACTIVE
                    publication_info: Some(classwork::PublicationInfo { status: Some(2), ..Default::default() }),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        }],
        read_mask: Some(post_read_mask(!materials.is_empty(), formatted)),
        ..Default::default()
    }
}

fn confirm_announcement(
    response: mutation::StreamItemUpsertResponse, course: i64, own_user: i64, body: &str,
) -> Result<(Coursework, Vec<classwork::Material>), String> {
    let uncertain = || "Classroom did not confirm the post. Check the Stream before posting again.".to_owned();
    let [result] = response.result.as_slice() else { return Err(uncertain()) };
    if let Some(code) = result.status.as_ref().and_then(|status| status.code).filter(|&code| code != 0) {
        return Err(format!("Classroom rejected the post (status {code})."));
    }
    let item = result.stream_item.as_ref().filter(|item| item.stream_type == Some(3)
        && item.assignment.is_none() && item.question.is_none() && item.supplement.is_none())
        .ok_or_else(uncertain)?;
    let info = item.post.as_ref().and_then(|post| post.stream_item_info.as_ref()).ok_or_else(uncertain)?;
    if info.stream_item_id.as_ref().and_then(|id| id.id).is_none_or(|id| id <= 0)
        || info.creator_user_id.as_ref().and_then(|id| id.id) != Some(own_user)
        || info.description_rich.as_ref().and_then(|rich| rich.text.as_deref()).or(info.description.as_deref()) != Some(body)
    {
        return Err(uncertain());
    }
    let resolved = coursework(course, item.clone()).ok_or_else(uncertain)?;
    Ok((resolved, info.material.clone()))
}

fn post_key(course: i64, id: i64) -> classwork::StreamItemId {
    classwork::StreamItemId { id: Some(id), course_id: Some(classwork::CourseId { id: Some(course) }) }
}

fn post_info<'a>(item: &'a classwork::StreamItem, key: &classwork::StreamItemId) -> Option<&'a classwork::StreamItemInfo> {
    if item.stream_type != Some(3) || item.assignment.is_some() || item.question.is_some() || item.supplement.is_some() {
        return None;
    }
    let info = item.post.as_ref()?.stream_item_info.as_ref()?;
    (info.stream_item_id.as_ref() == Some(key)).then_some(info)
}

async fn lookup_stream_post(
    session: &LiveSession, course: i64, key: &classwork::StreamItemId,
) -> Result<Option<classwork::StreamItemInfo>, String> {
    lookup_stream_post_with_mask(session, course, key, post_owner_mask()).await
}

async fn lookup_stream_post_with_mask(
    session: &LiveSession, course: i64, key: &classwork::StreamItemId, read_mask: classwork::StreamReadMask,
) -> Result<Option<classwork::StreamItemInfo>, String> {
    let response: classwork::StreamItemLookupResponse = session.rpc(
        "LookupStreamItem", classwork::StreamItemLookupRequest {
            stream_item_id: vec![key.clone()],
            read_mask: Some(read_mask),
        }, Some(course),
    ).await?;
    match response.result.as_slice() {
        [] => Ok(None),
        [result] => match &result.stream_item {
            Some(item) => post_info(item, key).cloned().map(Some)
                .ok_or_else(|| "Classroom returned a different Stream item.".into()),
            None => Ok(None),
        },
        _ => Err("Classroom returned more than one result for this post.".into()),
    }
}

fn post_read_mask(include_materials: bool, rich: bool) -> classwork::StreamReadMask {
    classwork::StreamReadMask {
        post_mask: Some(classwork::PostReadMask {
            stream_item_info_mask: Some(classwork::StreamItemInfoReadMask {
                include_description: Some(true),
                description_read_method: rich.then_some(2),
                include_creator_user_id: Some(true),
                include_creation_timestamp: Some(true),
                publication_info_mask: Some(classwork::PublicationInfoReadMask {
                    include_publication_timestamp: Some(true),
                    ..Default::default()
                }),
                material_mask: include_materials.then_some(classwork::MaterialReadMask {
                    include_name: Some(true), include_mime_type: Some(true), include_open_url: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        }),
        ..Default::default()
    }
}
fn post_owner_mask() -> classwork::StreamReadMask {
    classwork::StreamReadMask {
        post_mask: Some(classwork::PostReadMask {
            stream_item_info_mask: Some(classwork::StreamItemInfoReadMask {
                include_creator_user_id: Some(true),
                ..Default::default()
            }),
        }),
        ..Default::default()
    }
}

pub async fn check_stream_post_lookup(
    session: &LiveSession, course: i64, id: i64,
) -> Result<(bool, bool), String> {
    let key = post_key(course, id);
    let info = lookup_stream_post(session, course, &key).await?
        .ok_or_else(|| "Classroom did not find the selected Stream post.".to_owned())?;
    Ok((info.creator_user_id.and_then(|id| id.id).is_some(), info.status == Some(2)))
}

fn delete_post_request(key: classwork::StreamItemId) -> mutation::StreamItemUpdateRequest {
    mutation::StreamItemUpdateRequest {
        stream_item: vec![classwork::StreamItem {
            stream_type: Some(3),
            post: Some(classwork::Post {
                stream_item_info: Some(classwork::StreamItemInfo {
                    stream_item_id: Some(key),
                    status: Some(3), // TRASHED
                    ..Default::default()
                }),
            }),
            ..Default::default()
        }],
        update_mask: Some(mutation::StreamItemUpdateMask {
            post_mask: Some(mutation::PostUpdateMask {
                stream_item_info_mask: Some(mutation::StreamItemInfoUpdateMask { update_status: Some(true), ..Default::default() }),
            }),
            ..Default::default()
        }),
        read_mask: Some(post_owner_mask()),
        ..Default::default()
    }
}

fn delete_ack(response: mutation::StreamItemUpsertResponse, key: &classwork::StreamItemId) -> Result<bool, String> {
    let [result] = response.result.as_slice() else {
        return Err("Classroom did not confirm deletion of this post.".into());
    };
    let code = result.status.as_ref().and_then(|status| status.code);
    if let Some(code) = code.filter(|&code| code != 0) {
        return Err(format!("Classroom rejected the deletion (status {code})."));
    }
    match &result.stream_item {
        Some(item) => post_info(item, key).map(|info| info.status == Some(3))
            .ok_or_else(|| "Classroom returned a different post after deletion.".into()),
        None if code == Some(0) => Ok(false),
        None => Err("Classroom did not confirm deletion of this post.".into()),
    }
}

pub async fn delete_own_stream_post(session: &LiveSession, course: i64, id: i64) -> Result<(), String> {
    check_class(course)?;
    if id <= 0 || !session.snapshot.courses.iter().any(|entry| entry.id == course && entry.state == CourseState::Active) {
        return Err("Choose an announcement in an active class.".into());
    }
    let user = current_user_id(session).await?;
    delete_verified_stream_post(session, course, id, user).await
}

pub async fn moderate_stream_post(session: &LiveSession, course: i64, id: i64, author: i64) -> Result<(), String> {
    if id <= 0 || author <= 0 {
        return Err("Choose an existing Stream post.".into());
    }
    require_class_moderator(session, course).await?;
    delete_verified_stream_post(session, course, id, author).await
}

async fn delete_verified_stream_post(session: &LiveSession, course: i64, id: i64, author: i64) -> Result<(), String> {
    let key = post_key(course, id);
    let info = lookup_stream_post(session, course, &key).await?
        .ok_or_else(|| "This post is no longer available.".to_owned())?;
    if !active_post_author(&info, author) {
        return Err("Classroom did not confirm the active post and its author.".into());
    }
    let response = session.write_rpc("UpdateStreamItem", delete_post_request(key.clone()), Some(course)).await?;
    if delete_ack(response, &key)? {
        return Ok(());
    }
    if lookup_stream_post(session, course, &key).await?.is_some_and(|info| info.status != Some(3)) {
        return Err("Classroom still shows this post. Check the Stream before trying again.".into());
    }
    Ok(())
}

fn active_post_author(info: &classwork::StreamItemInfo, author: i64) -> bool {
    author > 0 && info.status == Some(2) && info.creator_user_id.as_ref().and_then(|id| id.id) == Some(author)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_moderation_checks_the_displayed_author_and_active_status() {
        let mut post = classwork::StreamItemInfo {
            stream_item_id: Some(post_key(4, 7)),
            status: Some(2), creator_user_id: Some(classwork::UserId { id: Some(15) }),
            ..Default::default()
        };
        assert!(active_post_author(&post, 15));
        assert!(!active_post_author(&post, 16));
        post.status = Some(3);
        assert!(!active_post_author(&post, 15));
    }

    #[test]
    fn stream_access_binds_role_and_mute_to_the_signed_in_member() {
        let row = |id, policy, user: Option<i64>| wire::Course {
            course_id: Some(wire::CourseId { id: Some(id) }),
            course_setting: Some(wire::CourseSetting { student_stream_posting_policy: Some(policy), ..Default::default() }),
            course_state: Some(1),
            muted_user_id: user.into_iter().map(|id| classwork::UserId { id: Some(id) }.encode_to_vec()).collect(),
            ..Default::default()
        };
        let role = |id, user, as_role| classwork::CourseUserLookupResult {
            course_user_id: Some(course_member_key(id, user)),
            course_user: Some(classwork::CourseUser {
                course_user_id: Some(course_member_key(id, user)), course_role: Some(as_role), ..Default::default()
            }),
        };
        let courses = wire::QueryCourseResponse {
            course: vec![row(1, 2, Some(17)), row(2, 3, None), row(3, 2, Some(9)), row(4, 4, None), row(5, 2, None)],
        };
        let members = classwork::CourseUserLookupResponse {
            results: vec![role(1, 9, 3), role(2, 9, 3), role(3, 9, 3), role(4, 9, 2), role(5, 17, 3)],
        };
        assert_eq!(decode_stream_access(courses, members, &BTreeSet::from([1, 2, 3, 4, 5]), 9), BTreeMap::from([
            (1, StreamAccess::PostAndComment), (2, StreamAccess::CommentOnly),
            (3, StreamAccess::ReadOnly), (4, StreamAccess::PostAndComment),
        ]));
    }

    #[test]
    fn announcement_encodes_one_published_post_without_client_item_id() {
        let request = announcement_request(7, 11, "Test", None, &[]);
        let encoded = request.encode_to_vec();
        assert_eq!(&encoded[..28], [
            0x0a, 28, 0x08, 3, 0x1a, 24, 0x0a, 22, 0x0a, 4, 0x12, 2, 0x08, 7,
            0x2a, 2, 0x08, 11, 0x3a, 4, 84, 101, 115, 116, 0x48, 2, 0x52, 2,
        ]);
        assert_eq!(&encoded[28..30], [0x08, 2]);
    }

    #[test]
    fn formatted_announcement_rebases_trimmed_utf16_offsets_and_keeps_plain_fallback() {
        let body = "  A📝B ";
        let styles = [
            DescriptionStyle { start: 0, bold: true, ..Default::default() },
            DescriptionStyle { start: 7, italic: true, underline: true, ..Default::default() },
        ];
        let rich = rich_post_body(body, &styles, &[]).unwrap();
        assert_eq!(rich.style.iter().map(|run| run.start).collect::<Vec<_>>(), [Some(0), Some(3)]);
        assert_eq!(rich.style[1].r#type, [3, 4]);
        let request = announcement_request(7, 11, body.trim(), Some(rich), &[]);
        let decoded = mutation::StreamItemInsertRequest::decode(request.encode_to_vec().as_slice()).unwrap();
        let info = decoded.stream_item[0].post.as_ref().unwrap().stream_item_info.as_ref().unwrap();
        assert_eq!(info.description.as_deref(), Some("A📝B"));
        let styles = decode_description_styles(info.description_rich.as_ref().unwrap(), "A📝B");
        assert_eq!(styles[1].start, "A📝".len());
        assert!(styles[1].underline && styles[1].italic);
        assert_eq!(decoded.read_mask.unwrap().post_mask.unwrap().stream_item_info_mask.unwrap().description_read_method, Some(2));
    }

    #[test]
    fn bullet_only_post_uses_paragraph_offsets_and_roundtrips_whitespace_and_emoji() {
        let body = "  First 📝\nSecond\nPlain  ";
        let second = body.find("Second").unwrap();
        let rich = rich_post_body(body, &[], &[0, second]).unwrap();
        assert!(rich.style.is_empty());
        assert_eq!(rich.block_style.iter().map(|style| (style.r#type, style.start)).collect::<Vec<_>>(),
            [(Some(1), Some(0)), (Some(1), Some("First 📝\n".encode_utf16().count() as i64))]);
        assert_eq!(decode_description_bullets(&rich, body.trim()), [0, second - 2]);
    }

    #[test]
    fn stream_links_use_the_repeated_material_field_and_request_readback() {
        let link = link_material("https://example.org/lesson");
        let request = announcement_request(7, 11, "Test", None, std::slice::from_ref(&link));
        let decoded = mutation::StreamItemInsertRequest::decode(request.encode_to_vec().as_slice()).unwrap();
        let materials = &decoded.stream_item[0].post.as_ref().unwrap().stream_item_info.as_ref().unwrap().material;
        assert_eq!(materials.len(), 1);
        assert_eq!(materials[0].reference_type, Some(4));
        assert_eq!(materials[0].reference.as_deref(), Some("https://example.org/lesson"));
        let mask = decoded.read_mask.unwrap().post_mask.unwrap().stream_item_info_mask.unwrap();
        assert_eq!(mask.material_mask.unwrap().include_open_url, Some(true));
        assert_eq!(count_missing_attachments(std::slice::from_ref(&link), materials), 0);
        assert_eq!(count_missing_attachments(&[link], &[]), 1);
    }

    #[test]
    fn stream_post_can_send_an_uploaded_drive_file_alongside_links() {
        let file = DriveFileAttachment(classwork::Material {
            name: Some("Reading notes.pdf".into()),
            reference: Some("drive_fixture".into()),
            reference_type: Some(2),
            mime_type: Some("application/pdf".into()),
            is_new_upload: Some(true),
            ..Default::default()
        });
        let materials = announcement_materials(vec!["https://example.org/lesson"], vec![file]);
        let request = announcement_request(7, 11, "Test", None, &materials);
        let decoded = mutation::StreamItemInsertRequest::decode(request.encode_to_vec().as_slice()).unwrap();
        let attached = &decoded.stream_item[0].post.as_ref().unwrap().stream_item_info.as_ref().unwrap().material;
        assert_eq!(attached, &materials);
        assert_eq!(attached[1].reference_type, Some(2));
        assert_eq!(attached[1].is_new_upload, Some(true));
        let returned = [attached[0].clone(), classwork::Material {
            open_url: Some("https://drive.google.com/file/d/drive_fixture/view".into()), ..Default::default()
        }];
        assert_eq!(count_missing_attachments(attached, &returned), 0);
        assert_eq!(count_missing_attachments(attached, &returned[..1]), 1);
    }

    #[test]
    fn post_confirmation_checks_per_item_failure_id_course_author_and_content() {
        let make_response = |course, author, code, body| {
            let mut request = announcement_request(course, author, body, None, &[]);
            let mut item = request.stream_item.pop().unwrap();
            item.post.as_mut().unwrap().stream_item_info.as_mut().unwrap().stream_item_id.as_mut().unwrap().id = Some(41);
            mutation::StreamItemUpsertResponse { result: vec![mutation::StreamItemUpsertResult {
                status: Some(mutation::StreamItemStatus { code: Some(code) }), stream_item: Some(item),
            }] }
        };
        assert_eq!(confirm_announcement(make_response(7, 11, 0, "Test"), 7, 11, "Test").unwrap().0.id, "41");
        for response in [
            make_response(7, 11, 7, "Test"), make_response(8, 11, 0, "Test"),
            make_response(7, 12, 0, "Test"), make_response(7, 11, 0, "Different"),
        ] {
            assert!(confirm_announcement(response, 7, 11, "Test").is_err());
        }
    }

    #[test]
    fn delete_request_updates_only_status_for_one_identified_post() {
        let request = delete_post_request(post_key(7, 9));
        assert_eq!(&request.encode_to_vec()[..24], [
            0x0a, 16, 0x08, 3, 0x1a, 12, 0x0a, 10, 0x0a, 6, 0x08, 9, 0x12, 2, 0x08, 7, 0x48, 3,
            0x12, 6, 0x12, 4, 0x0a, 2,
        ]);
        assert_eq!(&request.encode_to_vec()[24..26], [0x30, 1]);
    }

    #[test]
    fn delete_confirmation_refuses_wrong_targets_and_ambiguous_empty_results() {
        let key = post_key(7, 9);
        let reply = |item, code: Option<i32>| mutation::StreamItemUpsertResponse {
            result: vec![mutation::StreamItemUpsertResult {
                status: code.map(|code| mutation::StreamItemStatus { code: Some(code) }),
                stream_item: item,
            }],
        };
        let item = delete_post_request(key.clone()).stream_item.pop().unwrap();
        let wrong = delete_post_request(post_key(8, 9)).stream_item.pop().unwrap();
        assert_eq!(delete_ack(reply(Some(item.clone()), Some(0)), &key), Ok(true));
        assert_eq!(delete_ack(reply(None, Some(0)), &key), Ok(false));
        assert!(delete_ack(reply(Some(item), Some(7)), &key).is_err());
        assert!(delete_ack(reply(Some(wrong), Some(0)), &key).is_err());
        assert!(delete_ack(reply(None, None), &key).is_err());
    }
}
