use super::{LiveSession, classwork, load_topics, mutation, require_class_moderator};
use crate::desktop_data::{CourseworkKind, Topic};

#[derive(Debug, Clone)]
pub enum TeacherTopicAction {
    Create(String),
    Rename { topic: Topic, name: String },
    Delete(Topic),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicItemTarget {
    pub course: i64,
    pub item: i64,
    pub kind: CourseworkKind,
    pub draft: bool,
    pub original: Option<String>,
}

pub async fn change_classwork_topic(
    session: &LiveSession, target: TopicItemTarget, topic: Option<String>,
) -> Result<Option<String>, String> {
    if target.item <= 0 || target.kind == CourseworkKind::Announcement || topic == target.original {
        return Err("Choose a different topic for an existing Classwork item.".into());
    }
    let topics = load_teacher_topics(session, target.course).await?;
    if let Some(id) = &topic
        && (id.is_empty() || topics.iter().filter(|entry| &entry.id == id).count() != 1)
    {
        return Err("Classroom did not confirm that topic in this class.".into());
    }
    let (current, info) = load_item_topic(session, &target).await?;
    if current != target.original {
        return Err("The item's topic changed. Review Classwork before retrying.".into());
    }
    let request = item_topic_request(&target, info, topic.as_deref())?;
    let response: mutation::StreamItemUpsertResponse = session.write_rpc(
        "UpdateStreamItem", request, Some(target.course),
    ).await.map_err(|error| format!("{error} Review Classwork before retrying."))?;
    if response.result.len() > 1 || response.result.first().and_then(|result| result.status.as_ref())
        .and_then(|status| status.code).is_some_and(|code| code != 0)
    {
        return Err("Classroom rejected the topic change.".into());
    }
    let (confirmed, _) = load_item_topic(session, &target).await?;
    if confirmed != topic {
        return Err("Classroom did not confirm the requested topic.".into());
    }
    Ok(confirmed)
}

fn item_key(target: &TopicItemTarget) -> classwork::StreamItemId {
    classwork::StreamItemId { id: Some(target.item), course_id: Some(classwork::CourseId { id: Some(target.course) }) }
}

fn item_topic_mask(kind: CourseworkKind) -> classwork::StreamReadMask {
    use classwork::*;
    let info = StreamItemInfoReadMask {
        include_topic_id: Some(true),
        publication_info_mask: Some(PublicationInfoReadMask { include_publication_status: Some(true), ..Default::default() }),
        ..Default::default()
    };
    let mut mask = StreamReadMask::default();
    match kind {
        CourseworkKind::Assignment => mask.assignment_mask = Some(AssignmentReadMask { stream_item_info_mask: Some(info), ..Default::default() }),
        CourseworkKind::Question => mask.question_mask = Some(QuestionReadMask { stream_item_info_mask: Some(info), ..Default::default() }),
        CourseworkKind::Material => mask.supplement_mask = Some(SupplementReadMask { stream_item_info_mask: Some(info) }),
        CourseworkKind::Announcement => {}
    }
    mask
}

async fn load_item_topic(session: &LiveSession, target: &TopicItemTarget) -> Result<(Option<String>, i32), String> {
    let key = item_key(target);
    let response: classwork::StreamItemLookupResponse = session.rpc("LookupStreamItem", classwork::StreamItemLookupRequest {
        stream_item_id: vec![key],
        read_mask: Some(item_topic_mask(target.kind)),
    }, Some(target.course)).await?;
    let [result] = response.result.as_slice() else { return Err("Classroom did not return the selected item.".into()); };
    result.stream_item.as_ref().and_then(|item| decode_item_topic(target, item))
        .ok_or_else(|| "Classroom did not confirm the selected item's identity, state and topic.".into())
}

pub async fn check_item_topic_read(session: &LiveSession, target: &TopicItemTarget) -> Result<bool, String> {
    if target.draft {
        return Err("This diagnostic reads only published coursework.".into());
    }
    load_item_topic(session, target).await.map(|(topic, _)| topic == target.original)
}

fn decode_item_topic(target: &TopicItemTarget, item: &classwork::StreamItem) -> Option<(Option<String>, i32)> {
    let kind = item.stream_type?;
    let info = match (target.kind, kind, &item.assignment, &item.question, &item.supplement, &item.post) {
        (CourseworkKind::Assignment, 2, Some(value), None, None, None) => value.stream_item_info.as_ref(),
        (CourseworkKind::Question, 4 | 5, None, Some(value), None, None) => value.stream_item_info.as_ref(),
        (CourseworkKind::Material, 7, None, None, Some(value), None) => value.stream_item_info.as_ref(),
        _ => None,
    }?;
    if info.stream_item_id.as_ref() != Some(&item_key(target))
        || info.status != Some(2)
        || info.publication_info.as_ref()?.status != Some(if target.draft { 3 } else { 2 })
    {
        return None;
    }
    match info.topic_id.as_slice() {
        [] => Some((None, kind)),
        [topic] if topic.course_id.as_ref().and_then(|id| id.id) == Some(target.course) =>
            Some((Some(topic.id.clone().filter(|id| !id.is_empty())?), kind)),
        _ => None,
    }
}

fn item_topic_request(target: &TopicItemTarget, kind: i32, topic: Option<&str>) -> Result<mutation::StreamItemUpdateRequest, String> {
    let info = classwork::StreamItemInfo {
        stream_item_id: Some(item_key(target)),
        topic_id: topic.into_iter().map(|id| topic_key(target.course, Some(id))).collect(),
        ..Default::default()
    };
    let mask = mutation::AssignmentUpdateMask {
        stream_item_info_mask: Some(mutation::StreamItemInfoUpdateMask {
            update_topic_id: Some(4), // REPLACE_ALL
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut item = classwork::StreamItem { stream_type: Some(kind), ..Default::default() };
    let mut update = mutation::StreamItemUpdateMask::default();
    match (target.kind, kind) {
        (CourseworkKind::Assignment, 2) => {
            item.assignment = Some(classwork::Assignment { stream_item_info: Some(info), ..Default::default() });
            update.assignment_mask = Some(mask);
        }
        (CourseworkKind::Question, 4 | 5) => {
            item.question = Some(classwork::Question { stream_item_info: Some(info), ..Default::default() });
            update.question_mask = Some(mask);
        }
        (CourseworkKind::Material, 7) => {
            item.supplement = Some(classwork::Supplement { stream_item_info: Some(info) });
            update.supplement_mask = Some(mask);
        }
        _ => return Err("Classroom returned an unsupported Classwork item.".into()),
    }
    Ok(mutation::StreamItemUpdateRequest {
        stream_item: vec![item], update_mask: Some(update), read_mask: Some(item_topic_mask(target.kind)),
        ..Default::default()
    })
}

pub fn normalized_topic_name(text: &str) -> Option<String> {
    let name = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!name.is_empty() && name.chars().count() <= 100 && !name.chars().any(char::is_control)).then_some(name)
}

pub async fn load_teacher_topics(session: &LiveSession, course: i64) -> Result<Vec<Topic>, String> {
    require_class_moderator(session, course).await?;
    load_topics(session, course).await
}

pub async fn mutate_teacher_topic(
    session: &LiveSession, course: i64, action: TeacherTopicAction,
) -> Result<Vec<Topic>, String> {
    let before = load_teacher_topics(session, course).await?;
    let (target, name) = match action {
        TeacherTopicAction::Create(name) => {
            let name = new_name(&before, None, &name)?;
            let _: () = session.write_rpc("TopicInsert", insert_topic_request(course, &name), Some(course)).await?;
            (None, Some(name))
        }
        TeacherTopicAction::Rename { topic, name } => {
            verify_topic(&before, &topic)?;
            let id = topic.id;
            let name = new_name(&before, Some(&id), &name)?;
            let _: () = session.write_rpc("TopicUpdate", rename_topic_request(course, &id, &name), Some(course)).await?;
            (Some(id), Some(name))
        }
        TeacherTopicAction::Delete(topic) => {
            verify_topic(&before, &topic)?;
            let id = topic.id;
            let key = topic_key(course, Some(&id));
            let reply: mutation::TopicDeleteResponse = session.write_rpc(
                "TopicDelete", mutation::TopicDeleteRequest { topic_id: Some(key.clone()) }, Some(course),
            ).await?;
            if reply.topic_id.is_some_and(|value| value != key) {
                return Err("Classroom returned a different topic. Reopen Classwork before retrying.".into());
            }
            (Some(id), None)
        }
    };
    let after = load_topics(session, course).await?;
    if !topic_change_confirmed(&before, &after, target.as_deref(), name.as_deref()) {
        return Err("Classroom did not confirm the topic change. Review Classwork before retrying.".into());
    }
    Ok(after)
}

fn verify_topic(before: &[Topic], topic: &Topic) -> Result<(), String> {
    let mut matching = before.iter().filter(|entry| entry.id == topic.id);
    if !topic.id.is_empty() && matching.next() == Some(topic) && matching.next().is_none() {
        Ok(())
    } else {
        Err("The topic changed or was removed. Review the current topics before retrying.".into())
    }
}

fn new_name(before: &[Topic], id: Option<&str>, name: &str) -> Result<String, String> {
    let name = normalized_topic_name(name).ok_or("Enter a topic name of up to 100 characters.")?;
    if before.iter().any(|entry| entry.title == name) {
        return Err(if id.is_some_and(|id| before.iter().any(|entry| entry.id == id && entry.title == name)) {
            "The topic already has this name."
        } else {
            "This class already has a topic with that name."
        }.into());
    }
    Ok(name)
}

fn topic_key(course: i64, id: Option<&str>) -> classwork::TopicId {
    classwork::TopicId { id: id.map(str::to_owned), course_id: Some(classwork::CourseId { id: Some(course) }) }
}

fn topic_mask() -> classwork::TopicReadMask {
    classwork::TopicReadMask { include_name: Some(true), ..Default::default() }
}

fn insert_topic_request(course: i64, name: &str) -> mutation::TopicInsertRequest {
    mutation::TopicInsertRequest {
        topic: Some(classwork::Topic { topic_id: Some(topic_key(course, None)), name: Some(name.into()) }),
        read_mask: Some(topic_mask()),
    }
}

fn rename_topic_request(course: i64, id: &str, name: &str) -> mutation::TopicUpdateRequest {
    mutation::TopicUpdateRequest {
        topic: Some(classwork::Topic { topic_id: Some(topic_key(course, Some(id))), name: Some(name.into()) }),
        update_mask: Some(mutation::TopicUpdateMask { update_name: Some(true) }),
        read_mask: Some(topic_mask()),
    }
}

fn topic_change_confirmed(before: &[Topic], after: &[Topic], id: Option<&str>, name: Option<&str>) -> bool {
    match (id, name) {
        (None, Some(name)) => after.iter().filter(|entry| entry.title == name && !entry.id.is_empty()
            && !before.iter().any(|old| old.id == entry.id)).count() == 1,
        (Some(id), Some(name)) => {
            let mut matches = after.iter().filter(|entry| entry.id == id);
            matches.next().is_some_and(|entry| entry.title == name) && matches.next().is_none()
        }
        (Some(id), None) => !after.iter().any(|topic| topic.id == id),
        (None, None) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn topic_move_masks_only_the_topic_and_clears_with_an_empty_list() {
        let target = TopicItemTarget { course: 9, item: 21, kind: CourseworkKind::Assignment,
            draft: false, original: Some("old".into()) };
        for topic in [Some("new"), None] {
            let request = item_topic_request(&target, 2, topic).unwrap();
            let request = mutation::StreamItemUpdateRequest::decode(request.encode_to_vec().as_slice()).unwrap();
            let [item] = request.stream_item.as_slice() else { panic!("expected one item") };
            let info = item.assignment.as_ref().unwrap().stream_item_info.as_ref().unwrap();
            assert_eq!(info.stream_item_id.as_ref(), Some(&item_key(&target)));
            assert_eq!(info.topic_id, topic.into_iter().map(|id| topic_key(9, Some(id))).collect::<Vec<_>>());
            assert!(info.title.is_none() && info.description.is_none() && info.publication_info.is_none());
            let mask = request.update_mask.unwrap().assignment_mask.unwrap().stream_item_info_mask.unwrap();
            assert_eq!(mask.update_topic_id, Some(4));
            assert!(mask.update_status.is_none() && mask.publication_info_mask.is_none() && mask.update_title.is_none());
        }
    }

    #[test]
    fn topic_requests_use_the_course_parent_and_only_rename_the_name_field() {
        let create = insert_topic_request(12, "Reading");
        let create = mutation::TopicInsertRequest::decode(create.encode_to_vec().as_slice()).unwrap();
        assert_eq!(create.topic.unwrap().topic_id, Some(topic_key(12, None)));
        let rename = rename_topic_request(12, "a", "Poetry");
        assert_eq!(rename.topic.unwrap().topic_id, Some(topic_key(12, Some("a"))));
        assert_eq!(rename.update_mask.unwrap().update_name, Some(true));
        let deleted = mutation::TopicDeleteRequest { topic_id: Some(topic_key(12, Some("a"))) };
        assert_eq!(mutation::TopicDeleteRequest::decode(deleted.encode_to_vec().as_slice()).unwrap(), deleted);
    }

    #[test]
    fn topic_readback_distinguishes_a_new_id_from_preexisting_and_duplicate_topics() {
        let topic = |id: &str, title: &str| Topic { id: id.into(), title: title.into() };
        let existing = vec![topic("a", "Poetry")];
        assert_eq!(normalized_topic_name("  Week\tOne ").as_deref(), Some("Week One"));
        assert!(!topic_change_confirmed(&existing, &existing, None, Some("Poetry")));
        assert!(topic_change_confirmed(&existing, &[topic("a", "Poetry"), topic("b", "Week One")], None, Some("Week One")));
        assert!(topic_change_confirmed(&existing, &[topic("a", "Reading")], Some("a"), Some("Reading")));
        assert!(!topic_change_confirmed(&existing, &[topic("a", "Reading"), topic("a", "Poetry")], Some("a"), Some("Reading")));
        assert!(topic_change_confirmed(&existing, &[], Some("a"), None));
        assert!(!topic_change_confirmed(&existing, &existing, Some("a"), None));
    }
}
