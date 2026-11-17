use super::{CourseContents, LiveSession, classwork, load_coursework, require_class_moderator};
use crate::desktop_data::{Coursework, CourseworkKind};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassworkOrderDirection {
    Up,
    Down,
}

pub async fn reorder_classwork(
    session: &LiveSession,
    course: i64,
    stream_item_id: &str,
    expected_topic: &str,
    expected_order: &[String],
    direction: ClassworkOrderDirection,
) -> Result<CourseContents, String> {
    if expected_topic.is_empty() {
        return Err("Choose a post in an existing Classwork topic.".into());
    }
    require_class_moderator(session, course).await?;
    let before = load_coursework(session, course).await?;
    let moving = before.items.iter().find(|entry| entry.id == stream_item_id
        && entry.kind != CourseworkKind::Announcement && entry.course_id == course)
        .ok_or("Classroom did not return the selected Classwork post.")?;
    let topic = moving.topic_id.as_deref();
    if topic != Some(expected_topic) {
        return Err("The Classwork post's topic changed. Refresh Classwork before moving it.".into());
    }
    let current = group_order(&before.items, course, topic)?;
    let Some(Move { expected, predecessor }) = plan_move(&current, expected_order, stream_item_id, direction)? else {
        return Ok(before);
    };
    let request = reorder_request(course, moving, predecessor)?;
    let moved_key = request.stream_item_id.clone();
    let response: classwork::ReorderClassworkResponse = session
        .write_rpc("ReorderClasswork", request, Some(course)).await?;
    if !response.moved_stream_item.as_ref().and_then(stream_item_info)
        .is_some_and(|info| info.stream_item_id == moved_key
            && info.classwork_sort_key.as_ref().is_some_and(|key| !key.is_empty()))
    {
        return Err("Classroom did not confirm the move. Refresh Classwork before retrying.".into());
    }
    let after = load_coursework(session, course).await?;
    let order = group_order(&after.items, course, topic)?;
    if !order.iter().map(|item| item.id.as_str()).eq(expected) {
        return Err("Classroom returned a different post order. Refresh Classwork before retrying.".into());
    }
    Ok(after)
}

fn stream_item_info(item: &classwork::StreamItem) -> Option<&classwork::StreamItemInfo> {
    item.assignment.as_ref().and_then(|row| row.stream_item_info.as_ref())
        .or_else(|| item.question.as_ref().and_then(|row| row.stream_item_info.as_ref()))
        .or_else(|| item.supplement.as_ref().and_then(|row| row.stream_item_info.as_ref()))
}

fn group_order<'a>(items: &'a [Coursework], course: i64, topic: Option<&str>) -> Result<Vec<&'a Coursework>, String> {
    let mut seen_ids = BTreeSet::new();
    let mut seen_keys = BTreeSet::new();
    let mut group = Vec::new();
    for entry in items.iter().filter(|entry| entry.topic_id.as_deref() == topic
        && entry.kind != CourseworkKind::Announcement)
    {
        if entry.course_id != course || !seen_ids.insert(entry.id.as_str()) {
            return Err("Classroom returned conflicting post identifiers.".into());
        }
        let key = entry.classwork_sort_key.as_deref().filter(|key| !key.is_empty())
            .ok_or("Classroom did not return an ordering key for every post in this topic.")?;
        if !seen_keys.insert(key) {
            return Err("Classroom returned conflicting post ordering keys.".into());
        }
        group.push(entry);
    }
    group.sort_by(|left, right| left.classwork_sort_key.cmp(&right.classwork_sort_key));
    Ok(group)
}

struct Move<'a> {
    expected: Vec<&'a str>,
    predecessor: Option<&'a Coursework>,
}

fn plan_move<'a>(
    current: &[&'a Coursework],
    expected_order: &[String],
    stream_item_id: &str,
    direction: ClassworkOrderDirection,
) -> Result<Option<Move<'a>>, String> {
    if !current.iter().map(|item| item.id.as_str()).eq(expected_order.iter().map(String::as_str)) {
        return Err("The Classwork order changed. Refresh Classwork before moving this post.".into());
    }
    let index = current.iter().position(|item| item.id == stream_item_id)
        .ok_or("Classroom did not return the selected Classwork post.")?;
    let destination = match direction {
        ClassworkOrderDirection::Up => index.saturating_sub(1),
        ClassworkOrderDirection::Down => (index + 1).min(current.len() - 1),
    };
    if index == destination {
        return Ok(None);
    }
    let mut order = current.to_vec();
    order.swap(index, destination);
    let predecessor = destination.checked_sub(1).map(|previous| order[previous]);
    let expected = order.into_iter().map(|item| item.id.as_str()).collect();
    Ok(Some(Move { expected, predecessor }))
}

fn reorder_request(
    course: i64,
    moving: &Coursework,
    predecessor: Option<&Coursework>,
) -> Result<classwork::ReorderClassworkRequest, String> {
    let course_id = classwork::CourseId { id: Some(course) };
    let stream_id = |item: &Coursework| {
        let id = item.id.parse::<i64>().ok().filter(|id| *id > 0)
            .ok_or("Classroom returned an invalid post identifier.")?;
        Ok::<_, &str>(classwork::StreamItemId { id: Some(id), course_id: Some(course_id) })
    };
    let info = classwork::StreamItemInfoReadMask {
        include_topic_id: Some(true),
        include_classwork_sort_key: Some(true),
        ..Default::default()
    };
    let mask = classwork::StreamReadMask {
        assignment_mask: Some(classwork::AssignmentReadMask {
            stream_item_info_mask: Some(info), ..Default::default()
        }),
        question_mask: Some(classwork::QuestionReadMask {
            stream_item_info_mask: Some(info), ..Default::default()
        }),
        supplement_mask: Some(classwork::SupplementReadMask { stream_item_info_mask: Some(info) }),
        ..Default::default()
    };
    Ok(classwork::ReorderClassworkRequest {
        topic_id: Some(classwork::TopicId { id: moving.topic_id.clone(), course_id: Some(course_id) }),
        stream_item_id: Some(stream_id(moving)?),
        prev_stream_item_id: predecessor.map(stream_id).transpose()?,
        prev_stream_item_key: predecessor.and_then(|item| item.classwork_sort_key.clone()),
        read_mask: Some(mask),
    })
}

#[cfg(test)]
mod tests {
    use prost::Message;

    use super::*;

    fn item(id: i64, topic: Option<&str>, key: Option<&[u8]>) -> Coursework {
        Coursework {
            id: id.to_string(),
            course_id: 7,
            title: String::new(),
            description: String::new(),
            description_styles: vec![],
            description_bullets: vec![],
            due_label: None,
            due_at_millis: None,
            due_has_time: None,
            maximum_points: None,
            kind: CourseworkKind::Assignment,
            author_id: None,
            question: None,
            question_sharing: None,
            originality_enabled: None,
            allow_edit_after_turn_in: false,
            topic_id: topic.map(str::to_owned),
            classwork_sort_key: key.map(<[u8]>::to_vec),
            attachments: vec![],
            submission: None,
            comments: None,
        }
    }

    #[test]
    fn requests_exact_predecessor_and_course_scoped_topic() {
        let moving = item(1, Some("t"), Some(&[1]));
        let predecessor = item(2, Some("t"), Some(&[0xff, 0x10]));
        let request = reorder_request(7, &moving, Some(&predecessor)).unwrap();
        let bytes = request.encode_to_vec();
        assert_eq!(&bytes[..26], &[
            0x0a, 7, 0x0a, 1, b't', 0x12, 2, 0x08, 7,
            0x12, 6, 0x08, 1, 0x12, 2, 0x08, 7,
            0x1a, 6, 0x08, 2, 0x12, 2, 0x08, 7,
            0x22,
        ]);
        assert_eq!(&bytes[26..30], &[2, 0xff, 0x10, 0x2a]);
        assert_eq!(classwork::ReorderClassworkRequest::decode(bytes.as_slice()).unwrap(), request);
    }

    #[test]
    fn omits_predecessor_at_start_and_uses_course_only_for_untagged() {
        let moving = item(2, None, Some(&[2]));
        let request = reorder_request(7, &moving, None).unwrap();
        assert_eq!(request.topic_id.as_ref().unwrap().encode_to_vec(), [0x12, 2, 0x08, 7]);
        assert!(request.prev_stream_item_id.is_none());
        assert!(request.prev_stream_item_key.is_none());
    }

    #[test]
    fn plans_within_a_single_keyed_topic_and_rejects_ambiguous_order() {
        let mut items = [
            item(3, Some("t"), Some(&[0xff])),
            item(2, Some("t"), Some(&[0x81, 3])),
            item(8, None, None),
            item(1, Some("t"), Some(&[0x81])),
        ];
        let group = group_order(&items, 7, Some("t")).unwrap();
        assert!(group.iter().map(|item| item.id.as_str()).eq(["1", "2", "3"]));
        let expected = ["1", "2", "3"].map(str::to_owned);
        let start = plan_move(&group, &expected, "2", ClassworkOrderDirection::Up).unwrap().unwrap();
        assert!(start.predecessor.is_none());
        assert_eq!(start.expected, ["2", "1", "3"]);
        let later = plan_move(&group, &expected, "2", ClassworkOrderDirection::Down).unwrap().unwrap();
        assert_eq!(later.predecessor.unwrap().id, "3");
        assert_eq!(later.expected, ["1", "3", "2"]);
        assert!(plan_move(&group, &expected, "1", ClassworkOrderDirection::Up).unwrap().is_none());
        assert!(plan_move(&group, &expected, "8", ClassworkOrderDirection::Up).is_err());
        let stale = ["1", "3", "2"].map(str::to_owned);
        assert!(plan_move(&group, &stale, "2", ClassworkOrderDirection::Up).is_err());
        items[3].classwork_sort_key = items[0].classwork_sort_key.clone();
        assert!(group_order(&items, 7, Some("t")).is_err());
        items[3].classwork_sort_key = None;
        assert!(group_order(&items, 7, Some("t")).is_err());
    }
}
