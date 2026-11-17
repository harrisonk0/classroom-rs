use super::{LiveSession, classwork, require_class_moderator};
use crate::desktop_data::Topic;
use prost::Message;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopicOrderDirection {
    Up,
    Down,
}

#[derive(Clone, PartialEq, Message)]
struct ReorderTopicRequest {
    #[prost(message, optional, tag = "1")]
    topic_id: Option<classwork::TopicId>,
    #[prost(message, optional, tag = "2")]
    next_topic_id: Option<classwork::TopicId>,
    #[prost(bytes = "vec", optional, tag = "3")]
    next_topic_key: Option<Vec<u8>>,
    #[prost(message, optional, tag = "4")]
    read_mask: Option<classwork::TopicReadMask>,
}

#[derive(Clone, PartialEq, Message)]
struct ReorderTopicResponse {
    #[prost(message, optional, tag = "1")]
    moved_topic: Option<OrderedTopicWire>,
}

// The app's ordinary Topic record does not need to retain the opaque ordering key.
#[derive(Clone, PartialEq, Message)]
struct OrderedTopicWire {
    #[prost(message, optional, tag = "1")]
    topic_id: Option<classwork::TopicId>,
    #[prost(string, optional, tag = "2")]
    name: Option<String>,
    #[prost(bytes = "vec", optional, tag = "4")]
    classwork_sort_key: Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, Message)]
struct TopicOrderResponse {
    #[prost(message, repeated, tag = "2")]
    topic: Vec<OrderedTopicWire>,
}

struct OrderedTopic {
    topic: Topic,
    sort_key: Option<Vec<u8>>,
}

pub async fn reorder_topic(
    session: &LiveSession, course: i64, topic_id: &str, direction: TopicOrderDirection,
) -> Result<Vec<Topic>, String> {
    if topic_id.is_empty() {
        return Err("Choose an existing topic to move.".into());
    }
    require_class_moderator(session, course).await?;
    let before = read_topic_order(session, course).await?;
    let original: Vec<_> = before.iter().map(|entry| entry.topic.id.as_str()).collect();
    let expected = move_topic(&original, topic_id, direction)?;
    if expected == original {
        return Ok(before.into_iter().map(|entry| entry.topic).collect());
    }
    let position = expected.iter().position(|id| *id == topic_id)
        .ok_or("Classroom did not return the selected topic.")?;
    let successor = if let Some(next) = expected.get(position + 1) {
        Some(before.iter().find(|entry| entry.topic.id == *next)
            .ok_or("Classroom did not return the next topic.")?)
    } else {
        None
    };
    let request = reorder_request(course, topic_id, successor)?;
    let response: ReorderTopicResponse = session.write_rpc("ReorderTopic", request, Some(course)).await?;
    if response.moved_topic.and_then(|entry| entry.topic_id) != Some(topic_key(course, topic_id)) {
        return Err("Classroom did not confirm the topic move. Refresh Classwork before retrying.".into());
    }
    let after = read_topic_order(session, course).await?;
    if !after.iter().map(|entry| entry.topic.id.as_str()).eq(expected) {
        return Err("Classroom returned a different topic order. Refresh Classwork before retrying.".into());
    }
    Ok(after.into_iter().map(|entry| entry.topic).collect())
}

fn topic_key(course: i64, id: &str) -> classwork::TopicId {
    classwork::TopicId { id: Some(id.to_owned()), course_id: Some(classwork::CourseId { id: Some(course) }) }
}

fn move_topic<'a>(original: &[&'a str], topic_id: &str, direction: TopicOrderDirection) -> Result<Vec<&'a str>, String> {
    let position = original.iter().position(|id| *id == topic_id)
        .ok_or("Classroom did not return the selected topic.")?;
    let destination = match direction {
        TopicOrderDirection::Up => position.saturating_sub(1),
        TopicOrderDirection::Down => (position + 1).min(original.len() - 1),
    };
    let mut expected = original.to_vec();
    expected.swap(position, destination);
    Ok(expected)
}

fn reorder_request(course: i64, topic_id: &str, successor: Option<&OrderedTopic>) -> Result<ReorderTopicRequest, String> {
    let next_topic_key = successor.map(|entry| entry.sort_key.clone().filter(|key| !key.is_empty())
        .ok_or("Classroom did not return an ordering key for the next topic.")).transpose()?;
    Ok(ReorderTopicRequest {
        topic_id: Some(topic_key(course, topic_id)),
        next_topic_id: successor.map(|entry| topic_key(course, &entry.topic.id)),
        next_topic_key,
        read_mask: Some(topic_order_mask()),
    })
}

fn topic_order_mask() -> classwork::TopicReadMask {
    classwork::TopicReadMask { include_name: Some(true), include_classwork_sort_key: Some(true), ..Default::default() }
}

async fn read_topic_order(session: &LiveSession, course: i64) -> Result<Vec<OrderedTopic>, String> {
    let response: TopicOrderResponse = session.rpc("TopicQuery", classwork::TopicQueryRequest {
        request_header: Some(vec![]),
        response_order: Some(2), // CLASSWORK_ORDER
        read_mask: Some(topic_order_mask()),
        filter: vec![classwork::TopicFilter { course_id: vec![classwork::CourseId { id: Some(course) }] }],
    }, Some(course)).await?;
    decode_topic_order(response, course)
}

fn decode_topic_order(response: TopicOrderResponse, course: i64) -> Result<Vec<OrderedTopic>, String> {
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::new();
    for row in response.topic {
        let Some(key) = row.topic_id else { return Err("Classroom returned a topic without an identifier.".into()); };
        if key.course_id.as_ref().and_then(|parent| parent.id) != Some(course) {
            return Err("Classroom returned a topic outside the selected class.".into());
        }
        let id = key.id.filter(|id| !id.is_empty()).ok_or("Classroom returned an empty topic identifier.")?;
        if !seen.insert(id.clone()) {
            return Err("Classroom returned conflicting topic identifiers.".into());
        }
        let title = row.name.ok_or("Classroom returned a topic without a name.")?;
        ordered.push(OrderedTopic { topic: Topic { id, title }, sort_key: row.classwork_sort_key });
    }
    Ok(ordered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire_topic(course: i64, id: &str, key: Option<Vec<u8>>) -> OrderedTopicWire {
        OrderedTopicWire {
            topic_id: Some(topic_key(course, id)),
            name: Some(id.to_owned()),
            classwork_sort_key: key,
        }
    }

    #[test]
    fn orders_moves_as_displayed_and_leaves_edges_in_place() {
        let original = ["a", "b", "c"];
        assert_eq!(move_topic(&original, "b", TopicOrderDirection::Up).unwrap(), ["b", "a", "c"]);
        assert_eq!(move_topic(&original, "a", TopicOrderDirection::Down).unwrap(), ["b", "a", "c"]);
        assert_eq!(move_topic(&original, "a", TopicOrderDirection::Up).unwrap(), original);
        assert_eq!(move_topic(&original, "c", TopicOrderDirection::Down).unwrap(), original);
        assert!(move_topic(&original, "missing", TopicOrderDirection::Down).is_err());
    }

    #[test]
    fn encodes_the_successor_id_and_exact_ordering_key_or_leaves_both_unset() {
        let entries = decode_topic_order(TopicOrderResponse { topic: vec![wire_topic(7, "c", Some(vec![0xfd, 0x02]))] }, 7).unwrap();
        assert_eq!(reorder_request(7, "a", entries.first()).unwrap().encode_to_vec(), [
            0x0a, 7, 0x0a, 1, b'a', 0x12, 2, 0x08, 7,
            0x12, 7, 0x0a, 1, b'c', 0x12, 2, 0x08, 7,
            0x1a, 2, 0xfd, 0x02, 0x22, 4, 0x08, 1, 0x18, 1,
        ]);
        let end = reorder_request(7, "a", None).unwrap();
        assert!(end.next_topic_id.is_none());
        assert!(end.next_topic_key.is_none());
        let missing = decode_topic_order(TopicOrderResponse { topic: vec![wire_topic(7, "c", None)] }, 7).unwrap();
        assert!(reorder_request(7, "a", missing.first()).is_err());
    }

    #[test]
    fn refuses_out_of_scope_and_duplicate_results_and_keeps_server_order() {
        let row = wire_topic(7, "a", Some(vec![1]));
        let other = wire_topic(8, "b", Some(vec![2]));
        assert!(decode_topic_order(TopicOrderResponse { topic: vec![row.clone(), other] }, 7).is_err());
        assert!(decode_topic_order(TopicOrderResponse { topic: vec![row.clone(), row.clone()] }, 7).is_err());
        let entries = decode_topic_order(TopicOrderResponse {
            topic: vec![wire_topic(7, "b", Some(vec![2])), row],
        }, 7).unwrap();
        assert!(entries.iter().map(|entry| entry.topic.id.as_str()).eq(["b", "a"]));
    }
}
