use super::{LiveSession, refresh, wire};
use crate::desktop_data::{Course, CourseState};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoursePlacement {
    Beginning,
    Before(i64),
    After(i64),
    End,
}

fn available_for_move(course: &Course) -> bool {
    course.shows_on_home() && course.state == CourseState::Active
}

pub(super) fn sort_visible_classes(courses: &mut [Course]) {
    let positions: Vec<_> = courses.iter().enumerate().filter(|(_, course)| available_for_move(course))
        .map(|(index, _)| index).collect();
    if positions.iter().any(|&index| courses[index].reordering_sort_key.is_none()) {
        return;
    }
    let mut ordered: Vec<_> = positions.iter().map(|&index| courses[index].clone()).collect();
    ordered.sort_by(|a, b| a.reordering_sort_key.cmp(&b.reordering_sort_key));
    for (position, course) in positions.into_iter().zip(ordered) {
        courses[position] = course;
    }
}

pub async fn reorder_class(session: &LiveSession, course: i64, placement: CoursePlacement) -> Result<Vec<Course>, String> {
    let current = refresh(session).await?;
    let order = active_order(&current.courses);
    let (index, remaining) = move_destination(&order, course, placement)?;
    let mut expected = remaining.clone();
    expected.insert(index, course);
    if order == expected {
        return Ok(current.courses);
    }
    let response: wire::ReorderCourseResponse = session.write_rpc(
        "ReorderCourse", reorder_request(course, &remaining, index), Some(course),
    ).await?;
    if response.moved_course.and_then(|course| course.course_id).and_then(|id| id.id) != Some(course) {
        return Err("Classroom did not confirm the class move.".into());
    }
    let updated = refresh(session).await?;
    if active_order(&updated.courses) != expected {
        return Err("Classroom returned a different class order. Refresh and try again.".into());
    }
    Ok(updated.courses)
}

fn active_order(courses: &[Course]) -> Vec<i64> {
    courses.iter().filter(|course| available_for_move(course)).map(|course| course.id).collect()
}

fn move_destination(
    order: &[i64], course: i64, placement: CoursePlacement,
) -> Result<(usize, Vec<i64>), String> {
    if order.len() < 2 || !order.contains(&course) || order.iter().any(|&id| id <= 0)
        || order.iter().collect::<BTreeSet<_>>().len() != order.len()
    {
        return Err("Choose a visible active class to move.".into());
    }
    let remaining: Vec<_> = order.iter().copied().filter(|&id| id != course).collect();
    let destination = match placement {
        CoursePlacement::Beginning => 0,
        CoursePlacement::End => remaining.len(),
        CoursePlacement::Before(anchor) | CoursePlacement::After(anchor) => {
            let index = remaining.iter().position(|id| *id == anchor)
                .ok_or_else(|| "The destination class is no longer visible.".to_owned())?;
            index + usize::from(matches!(placement, CoursePlacement::After(_)))
        }
    };
    Ok((destination, remaining))
}

fn reorder_request(course: i64, remaining: &[i64], index: usize) -> wire::ReorderCourseRequest {
    let id = |id| wire::CourseId { id: Some(id) };
    wire::ReorderCourseRequest {
        moved_course_id: Some(id(course)),
        before_id: index.checked_sub(1).and_then(|previous| remaining.get(previous)).copied().map(id),
        after_id: remaining.get(index).copied().map(id),
        read_mask: Some(wire::CourseReadMask {
            include_reordering_sort_key: Some(true),
            ..Default::default()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn request_sets_predecessor_and_successor_in_android_order() {
        let order = [4, 7, 9, 12];
        let (index, remaining) = move_destination(&order, 12, CoursePlacement::After(4)).unwrap();
        assert_eq!(reorder_request(12, &remaining, index).encode_to_vec(), [
            0x0a, 2, 0x08, 12, 0x12, 2, 0x08, 4, 0x1a, 2, 0x08, 7, 0x22, 3, 0xa0, 2, 1,
        ]);
        let (index, remaining) = move_destination(&order, 12, CoursePlacement::Beginning).unwrap();
        let first = reorder_request(12, &remaining, index);
        assert!(first.before_id.is_none());
        assert_eq!(first.after_id.unwrap().id, Some(4));
        let (index, remaining) = move_destination(&order, 4, CoursePlacement::End).unwrap();
        let last = reorder_request(4, &remaining, index);
        assert!(last.after_id.is_none());
        assert_eq!(last.before_id.unwrap().id, Some(12));
        assert!(move_destination(&order, 4, CoursePlacement::Before(4)).is_err());
        assert!(move_destination(&order, 4, CoursePlacement::After(28)).is_err());
    }

    #[test]
    fn visible_order_uses_server_keys_and_preserves_other_sections() {
        let course = |id, key: Option<u8>, state, hidden| Course {
            id, state, hidden: Some(hidden), reordering_sort_key: key.map(|key| vec![key]), ..Default::default()
        };
        let mut courses = vec![
            course(1, Some(20), CourseState::Active, false),
            course(2, Some(2), CourseState::Archived, false),
            course(3, Some(1), CourseState::Active, true),
            course(4, Some(3), CourseState::Active, false),
        ];
        sort_visible_classes(&mut courses);
        assert_eq!(courses.iter().map(|course| course.id).collect::<Vec<_>>(), [4, 2, 3, 1]);
        courses[3].reordering_sort_key = None;
        courses[0].reordering_sort_key = Some(vec![99]);
        sort_visible_classes(&mut courses);
        assert_eq!(active_order(&courses), [4, 1]);
    }
}
