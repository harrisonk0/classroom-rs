use super::{
    wire, classwork, check_class, classroom_event_time, https_host, LiveSession, drive_files,
    BTreeMap, BTreeSet, Arc, Attachment, AttachmentKind, drive_url_reference, CourseRole, Coursework,
    CourseworkKind, QuestionDetails, Topic, Stream, stream, load_submissions,
    load_own_class_details, Message, StreamExt,
};

pub(super) fn classwork_request(course_id: i64) -> wire::StreamItemQueryRequest {
    classwork_query(course_id, false)
}

fn classwork_query(course_id: i64, drafts: bool) -> wire::StreamItemQueryRequest {
    use classwork::*;
    let info = StreamItemInfoReadMask {
        include_title: Some(true),
        include_description: Some(true),
        description_read_method: Some(2),
        include_topic_id: Some(true),
        include_creator_user_id: Some(true),
        include_creation_timestamp: Some(true),
        include_classwork_sort_key: (!drafts).then_some(true),
        publication_info_mask: Some(PublicationInfoReadMask {
            include_publication_timestamp: (!drafts).then_some(true),
            include_publication_status: drafts.then_some(true),
            include_scheduled_timestamp: drafts.then_some(true),
            include_scheduled_status: drafts.then_some(true),
        }),
        material_mask: Some(MaterialReadMask {
            include_name: Some(true),
            include_mime_type: Some(true),
            include_open_url: Some(true),
            drive_item_mask: Some(DriveItemReadMask { include_sharing_option: Some(true) }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let task = TaskInfoReadMask {
        include_due_date: Some(true),
        include_user_specified_due_time: Some(true),
        include_grade_denominator: Some(true),
        ..Default::default()
    };
    let mask = StreamReadMask {
        assignment_mask: Some(AssignmentReadMask {
            stream_item_info_mask: Some(info.clone()),
            task_info_mask: Some(task.clone()),
            originality_settings_mask: (!drafts).then_some(OriginalitySettingsReadMask { include_state: Some(true) }),
        }),
        post_mask: Some(PostReadMask { stream_item_info_mask: Some(StreamItemInfoReadMask {
            include_creator_user_id: Some(true), include_classwork_sort_key: None, ..info.clone()
        }) }),
        question_mask: Some(QuestionReadMask {
            stream_item_info_mask: Some(info.clone()),
            task_info_mask: Some(TaskInfoReadMask {
                include_allow_edit_after_turn_in: Some(true),
                submissions_visibility_mask: Some(VisibilityReadMask { include_visibility_type: Some(true) }),
                ..task
            }),
            question_info_mask: Some(QuestionInfoReadMask { include_choices: Some(true) }),
        }),
        supplement_mask: Some(SupplementReadMask { stream_item_info_mask: Some(info) }),
    };
    let filter = StreamFilter {
        allowed_status: vec![2], // ACTIVE
        course_id: vec![CourseId { id: Some(course_id) }],
        publication_status: vec![if drafts { 3 } else { 2 }],
        ..Default::default()
    };
    wire::StreamItemQueryRequest {
        read_mask: Some(mask.encode_to_vec()),
        filter: vec![filter.encode_to_vec()],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftPublication {
    Draft,
    Scheduled(Option<i64>),
    Retrying(Option<i64>),
    Failed(Option<i64>),
}

#[derive(Debug, Clone)]
pub struct TeacherDraft {
    pub item: Coursework,
    pub publication: DraftPublication,
}

pub async fn load_teacher_drafts(session: &LiveSession, course_id: i64) -> Result<Vec<TeacherDraft>, String> {
    check_class(course_id)?;
    let membership = load_own_class_details(session, vec![course_id]).await?;
    if membership.roles.get(&course_id) != Some(&CourseRole::Teacher) {
        return Err("Classroom did not confirm teacher access for this class.".into());
    }
    let response: wire::StreamItemQueryResponse = session.rpc(
        "QueryStreamItem", classwork_query(course_id, true), Some(course_id),
    ).await?;
    let mut drafts = BTreeMap::new();
    for bytes in response.stream_item {
        let item = classwork::StreamItem::decode(bytes.as_slice())
            .map_err(|_| "Classroom returned an unreadable draft.")?;
        if let Some(draft) = teacher_draft(course_id, item) {
            drafts.insert(draft.item.id.clone(), draft);
        }
    }
    Ok(drafts.into_values().collect())
}

pub(super) fn teacher_draft(course_id: i64, item: classwork::StreamItem) -> Option<TeacherDraft> {
    let info = item.assignment.as_ref().and_then(|row| row.stream_item_info.as_ref())
        .or_else(|| item.post.as_ref()?.stream_item_info.as_ref())
        .or_else(|| item.question.as_ref()?.stream_item_info.as_ref())
        .or_else(|| item.supplement.as_ref()?.stream_item_info.as_ref())?;
    if info.status.is_some_and(|status| status != 2) {
        return None;
    }
    let publication = info.publication_info.as_ref()?;
    if publication.status != Some(3) {
        return None;
    }
    let timestamp = classroom_event_time(publication.scheduled_timestamp);
    let publication = match publication.scheduled_status {
        Some(1) => DraftPublication::Scheduled(timestamp),
        Some(2) => DraftPublication::Retrying(timestamp),
        Some(4) => DraftPublication::Failed(timestamp),
        Some(3) => return None,
        _ => DraftPublication::Draft,
    };
    Some(TeacherDraft { item: coursework(course_id, item)?, publication })
}

#[cfg(test)]
#[test]
fn teacher_drafts_require_an_explicit_draft_status_and_exact_course() {
    use classwork::*;
    let scheduled_at = 1_789_000_000_000;
    let item = |status, course| StreamItem {
        assignment: Some(Assignment {
            stream_item_info: Some(StreamItemInfo {
                stream_item_id: Some(StreamItemId { id: Some(49), course_id: Some(CourseId { id: Some(course) }) }),
                publication_info: Some(PublicationInfo {
                    status: Some(status), scheduled_timestamp: Some(scheduled_at), scheduled_status: Some(1), ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(teacher_draft(9, item(3, 9)).unwrap().publication, DraftPublication::Scheduled(Some(scheduled_at)));
    assert!(teacher_draft(9, item(2, 9)).is_none());
    assert!(teacher_draft(9, item(3, 8)).is_none());
    let filter = |drafts| StreamFilter::decode(classwork_query(9, drafts).filter[0].as_slice()).unwrap().publication_status;
    assert_eq!(filter(false), [2]);
    assert_eq!(filter(true), [3]);
}

pub(super) fn coursework(course_id: i64, item: classwork::StreamItem) -> Option<Coursework> {
    let originality_enabled = match item.assignment.as_ref().and_then(|assignment| assignment.originality_settings.as_ref())
        .and_then(|settings| settings.state)
    {
        Some(1) => Some(true),
        Some(2) => Some(false),
        _ => None,
    };
    let (info, kind, task, question) = if let Some(assignment) = item.assignment {
        (assignment.stream_item_info?, CourseworkKind::Assignment, assignment.task_info, None)
    } else if let Some(post) = item.post {
        (post.stream_item_info?, CourseworkKind::Announcement, None, None)
    } else if let Some(question) = item.question {
        (question.stream_item_info?, CourseworkKind::Question, question.task_info,
            question.question_info.and_then(question_details))
    } else {
        (item.supplement?.stream_item_info?, CourseworkKind::Material, None, None)
    };
    // Live payloads use Unix milliseconds. Reject unset or out-of-range values.
    let due_at_millis = task.as_ref().and_then(|task| task.due_date)
        .filter(|time| (946_684_800_000..4_102_444_800_000).contains(time));
    let timed = task.as_ref().and_then(|task| task.user_specified_due_time) == Some(true);
    let due_label = due_at_millis.and_then(chrono::DateTime::from_timestamp_millis)
        .map(|date| {
            if timed {
                format!("Due {}", date.with_timezone(&chrono::Local).format("%-d %b, %-I:%M %p"))
            } else {
                format!("Due {}", date.format("%-d %b"))
            }
        });
    let id = info.stream_item_id?;
    if id.course_id.as_ref().and_then(|course| course.id) != Some(course_id) {
        return None;
    }
    let rich = info.description_rich.as_ref();
    let description = rich.and_then(|rich| rich.text.clone()).or(info.description).unwrap_or_default();
    let description_styles = rich.map_or_else(Vec::new, |rich| decode_description_styles(rich, &description));
    let description_bullets = rich.map_or_else(Vec::new, |rich| decode_description_bullets(rich, &description));
    let question_sharing = if question.is_some() {
        question_sharing(task.as_ref().and_then(|task| task.submissions_visibility.as_ref()))
    } else {
        None
    };
    Some(Coursework {
        id: id.id?.to_string(),
        course_id,
        title: info.title.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| kind.label().into()),
        description,
        description_styles,
        description_bullets,
        due_label,
        due_at_millis,
        due_has_time: due_at_millis.map(|_| timed),
        maximum_points: task.as_ref().and_then(|task| task.grade_denominator)
            .filter(|points| points.is_finite() && *points > 0.0),
        kind,
        author_id: info.creator_user_id.and_then(|user| user.id).filter(|id| *id > 0),
        posted_at_millis: classroom_event_time(info.publication_info.and_then(|publication| publication.publication_timestamp))
            .or_else(|| classroom_event_time(info.creation_timestamp)),
        question,
        question_sharing,
        originality_enabled,
        allow_edit_after_turn_in: kind == CourseworkKind::Question
            && task.as_ref().and_then(|task| task.allow_edit_after_turn_in) == Some(true),
        topic_id: info.topic_id.into_iter()
            .find(|topic| topic.course_id.as_ref().and_then(|course| course.id) == Some(course_id))
            .and_then(|topic| topic.id),
        classwork_sort_key: info.classwork_sort_key.filter(|key| !key.is_empty()),
        attachments: info.material.into_iter().filter_map(material).collect(),
        submission: None,
        comments: None,
    })
}

pub(super) fn decode_description_styles(rich: &classwork::RichText, text: &str) -> Vec<crate::desktop_data::DescriptionStyle> {
    let offsets = utf16_positions(text);
    let mut runs = BTreeMap::new();
    for style in &rich.style {
        let Some(&start) = offsets.get(&style.start.unwrap_or(0)) else { continue };
        runs.insert(start, crate::desktop_data::DescriptionStyle {
            start, bold: style.r#type.contains(&2), italic: style.r#type.contains(&3),
            underline: style.r#type.contains(&4),
        });
    }
    runs.into_values().collect()
}

fn utf16_positions(text: &str) -> BTreeMap<i64, usize> {
    let mut offsets = BTreeMap::new();
    let mut utf16 = 0;
    for (byte, character) in text.char_indices() {
        offsets.insert(utf16, byte);
        utf16 += character.len_utf16() as i64;
    }
    offsets
}

pub(super) fn decode_description_bullets(rich: &classwork::RichText, text: &str) -> Vec<usize> {
    let offsets = utf16_positions(text);
    rich.block_style.iter().filter_map(|block| {
        if block.r#type != Some(1) {
            return None;
        }
        let &start = offsets.get(&block.start.unwrap_or(0))?;
        (start == 0 || text.as_bytes().get(start - 1) == Some(&b'\n')).then_some(start)
    }).collect::<BTreeSet<_>>().into_iter().collect()
}

pub(super) fn material(item: classwork::Material) -> Option<Attachment> {
    use crate::desktop_data::DriveSharing;
    let drive_sharing = match item.drive_item.and_then(|drive| drive.sharing_option) {
        Some(1) => Some(DriveSharing::IndividualCopy),
        Some(2) => Some(DriveSharing::View),
        Some(3) => Some(DriveSharing::Edit),
        Some(4) => Some(DriveSharing::CopyToKeep),
        _ => None,
    };
    let url = item.open_url.filter(|value| https_host(value).is_some()).or_else(|| {
        if item.reference_type == Some(4) {
            item.reference.clone().filter(|value| https_host(value).is_some())
        } else {
            None
        }
    });
    let mime = item.mime_type.as_deref().unwrap_or_default();
    let kind = match url.as_deref().and_then(https_host).as_deref() {
        Some("www.youtube.com" | "youtu.be" | "youtube.com") => AttachmentKind::Video,
        Some("forms.gle") => AttachmentKind::Form,
        Some("docs.google.com") if url.as_ref().is_some_and(|url| url.contains("/forms/")) => AttachmentKind::Form,
        Some("drive.google.com" | "docs.google.com") => AttachmentKind::DriveFile,
        _ if mime.starts_with("video/") => AttachmentKind::Video,
        _ if mime.starts_with("application/vnd.google-apps.") || drive_sharing.is_some()
            || item.reference_type == Some(2) => AttachmentKind::DriveFile,
        _ if url.is_some() => AttachmentKind::Link,
        _ => AttachmentKind::Other,
    };
    let title = item.name.filter(|name| !name.trim().is_empty());
    let drive_reference = (item.reference_type == Some(2) || drive_sharing.is_some() || kind == AttachmentKind::DriveFile)
        .then(|| item.reference.filter(|value| drive_files::valid_drive_id(value))).flatten()
        .or_else(|| url.as_deref().and_then(drive_url_reference));
    if title.is_none() && url.is_none() && drive_reference.is_none() {
        return None;
    }
    Some(Attachment {
        remote_id: item.id.filter(|id| *id > 0), title: title.unwrap_or_else(|| kind.label().into()), url, kind,
        drive_sharing, drive_reference,
        drive_parent_reference: item.parent_reference.filter(|value| drive_files::valid_drive_id(value)),
    })
}

#[derive(Debug, Clone)]
pub struct CourseContents {
    pub items: Vec<Coursework>,
    pub topics: Option<Vec<Topic>>,
}

pub async fn load_coursework(session: &LiveSession, course_id: i64) -> Result<CourseContents, String> {
    let items = load_coursework_items(session, course_id).await?;
    let topics = load_referenced_topics(session, course_id, &items).await;
    Ok(CourseContents { items, topics })
}

async fn load_coursework_items(session: &LiveSession, course_id: i64) -> Result<Vec<Coursework>, String> {
    check_class(course_id)?;
    let response: wire::StreamItemQueryResponse = session.rpc(
        "QueryStreamItem", classwork_request(course_id), Some(course_id),
    ).await?;
    let count = response.stream_item.len();
    let mut items = Vec::with_capacity(count);
    for bytes in response.stream_item {
        let item = classwork::StreamItem::decode(bytes.as_slice())
            .map_err(|_| "Google returned a Classwork response this build can't decode.".to_owned())?;
        if let Some(item) = coursework(course_id, item) {
            items.push(item);
        }
    }
    if count != 0 && items.is_empty() {
        return Err("Google returned items without readable IDs for this class.".into());
    }
    Ok(items)
}

async fn load_referenced_topics(session: &LiveSession, course_id: i64, items: &[Coursework]) -> Option<Vec<Topic>> {
    if items.iter().any(|item| item.topic_id.is_some()) {
        load_topics(session, course_id).await.ok()
    } else {
        None
    }
}

#[derive(Debug, Clone)]
pub struct AssignmentRefresh {
    pub course_id: i64,
    pub contents: Option<CourseContents>,
    pub submissions: Option<Vec<(String, crate::desktop_data::Submission)>>,
}

pub fn assignment_updates(
    session: Arc<LiveSession>, courses: Vec<i64>,
) -> impl Stream<Item = AssignmentRefresh> + Send + 'static {
    stream::iter(courses).map(move |course_id| {
        let session = session.clone();
        async move {
            let Ok(items) = load_coursework_items(&session, course_id).await else {
                return AssignmentRefresh { course_id, contents: None, submissions: None };
            };
            let (topics, submissions) = tokio::join!(
                load_referenced_topics(&session, course_id, &items),
                async {
                    if items.iter().any(|item| matches!(item.kind, CourseworkKind::Assignment | CourseworkKind::Question)) {
                        load_submissions(&session, course_id).await.ok()
                    } else {
                        None
                    }
                },
            );
            AssignmentRefresh { course_id, contents: Some(CourseContents { items, topics }), submissions }
        }
    }).buffer_unordered(3)
}

pub(super) async fn load_topics(session: &LiveSession, course_id: i64) -> Result<Vec<Topic>, String> {
    use classwork::*;
    let response: TopicQueryResponse = session.rpc("TopicQuery", TopicQueryRequest {
        request_header: Some(vec![]),
        response_order: Some(2),
        read_mask: Some(TopicReadMask {
            include_name: Some(true),
            include_classwork_sort_key: Some(true),
            ..Default::default()
        }),
        filter: vec![TopicFilter { course_id: vec![CourseId { id: Some(course_id) }] }],
    }, Some(course_id)).await?;
    Ok(response.topic.into_iter().filter_map(|topic| {
        let key = topic.topic_id?;
        (key.course_id?.id? == course_id).then_some(crate::desktop_data::Topic { id: key.id?, title: topic.name? })
    }).collect())
}

pub(super) fn question_details(info: classwork::QuestionInfo) -> Option<QuestionDetails> {
    match info.question_type? {
        1 => Some(QuestionDetails::ShortAnswer),
        2 => Some(QuestionDetails::MultipleChoice(info.multiple_choice?.choice)),
        _ => None,
    }
}

fn question_sharing(visibility: Option<&classwork::Visibility>) -> Option<crate::desktop_data::QuestionSharing> {
    use crate::desktop_data::QuestionSharing;
    match visibility?.visibility_type? {
        1 => Some(QuestionSharing::Private),
        2 => Some(QuestionSharing::Course),
        _ => None,
    }
}

#[cfg(test)]
#[test]
fn question_sharing_requires_a_known_explicit_visibility() {
    use crate::desktop_data::QuestionSharing::{Course, Private};
    for (value, expected) in [(Some(1), Some(Private)), (Some(2), Some(Course)), (Some(0), None), (Some(99), None), (None, None)] {
        assert_eq!(question_sharing(Some(&classwork::Visibility { visibility_type: value })), expected);
    }
    assert_eq!(question_sharing(None), None);
}
