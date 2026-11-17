use super::{DraftPublication, LiveSession, TeacherDraft, classwork, mutation,
    current_user_id, load_teacher_drafts, require_class_moderator, teacher_draft};
use super::assignment_fields::{AssignmentFieldChanges, ValidatedAssignmentFields};
use crate::desktop_data::CourseworkKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignmentPublication {
    Publish,
    Schedule(i64),
}

impl AssignmentPublication {
    pub fn allowed_for(self, state: DraftPublication) -> bool {
        match self {
            Self::Publish => state == DraftPublication::Draft,
            Self::Schedule(time) => matches!(state, DraftPublication::Draft | DraftPublication::Scheduled(_))
                && state != DraftPublication::Scheduled(Some(time))
                && time > chrono::Utc::now().timestamp_millis() && time < 4_102_444_800_000,
        }
    }
}

pub async fn change_assignment_publication(
    session: &LiveSession, course: i64, original: TeacherDraft, action: AssignmentPublication,
) -> Result<(), String> {
    let id = original.item.id.parse::<i64>().ok().filter(|id| *id > 0)
        .ok_or("Classroom did not provide a draft ID.")?;
    if original.item.course_id != course || original.item.kind != CourseworkKind::Assignment
        || !action.allowed_for(original.publication)
    {
        return Err("Choose an assignment draft and a valid publication action.".into());
    }
    let current = load_teacher_drafts(session, course).await?.into_iter()
        .find(|draft| draft.item.id == original.item.id)
        .ok_or("The assignment is no longer available as a draft.")?;
    if current.item != original.item || current.publication != original.publication
        || !action.allowed_for(current.publication)
    {
        return Err("The assignment or its schedule has changed. Check Classwork and reopen it.".into());
    }
    let response = session.write_rpc("UpdateStreamItem", publication_update(course, id, action), Some(course)).await
        .map_err(|error| format!("{error} Check Classwork before retrying."))?;
    confirm_publication(response, course, id, action)
}

pub fn assignment_draft_text_valid(title: &str, instructions: &str) -> bool {
    !title.trim().is_empty() && title.chars().count() <= 3_000
        && !title.chars().any(char::is_control)
        && instructions.chars().count() <= 30_000
        && !instructions.contains('\0')
}

pub async fn save_assignment_draft(
    session: &LiveSession, course: i64, original: Option<TeacherDraft>, title: String, instructions: String,
) -> Result<TeacherDraft, String> {
    save_assignment_draft_with_fields(
        session, course, original, title, instructions, AssignmentFieldChanges::default(),
    ).await
}

pub async fn save_assignment_draft_with_fields(
    session: &LiveSession,
    course: i64,
    original: Option<TeacherDraft>,
    title: String,
    instructions: String,
    fields: AssignmentFieldChanges,
) -> Result<TeacherDraft, String> {
    let title = title.trim();
    if !assignment_draft_text_valid(title, &instructions) {
        return Err("Enter an assignment title and check the text length.".into());
    }
    let fields = fields.validate()?;
    require_class_moderator(session, course).await?;
    let (response, id, expected_fields): (mutation::StreamItemUpsertResponse, Option<i64>, _) = if let Some(original) = original.as_ref() {
        let id = original.item.id.parse::<i64>().ok().filter(|id| *id > 0)
            .ok_or("Classroom did not provide a draft ID.")?;
        if original.item.course_id != course || original.item.kind != CourseworkKind::Assignment
            || original.publication != DraftPublication::Draft
        {
            return Err("Choose an unscheduled assignment draft in this class.".into());
        }
        let current = lookup_assignment_draft(session, course, id).await?;
        let fields = fields.for_update(original, &current)?;
        let update = assignment_update(course, id, original, &current, title, &instructions, fields)?;
        (session.write_rpc("UpdateStreamItem", update, Some(course)).await
            .map_err(|error| format!("{error} Check the draft before retrying."))?, Some(id), fields)
    } else {
        let teacher = current_user_id(session).await?;
        let fields = fields.for_insert();
        let request = assignment_insert(course, teacher, title, &instructions, fields);
        (session.write_rpc("InsertStreamItem", request, Some(course)).await
            .map_err(|error| format!("{error} Check Classwork for the draft before trying again."))?, None, fields)
    };
    let acknowledgement = confirm_assignment_draft(response, course, id, title, &instructions)?;
    let id = acknowledgement.item.id.parse::<i64>().map_err(|_| DRAFT_UNCONFIRMED)?;
    let confirmed = lookup_assignment_draft(session, course, id).await.map_err(|_| DRAFT_UNCONFIRMED)?;
    let kept_instructions = original.as_ref().filter(|draft| draft.item.description == instructions);
    if confirmed.item.title != title || confirmed.item.description != instructions
        || kept_instructions.is_some_and(|draft| draft.item.description_styles != confirmed.item.description_styles
            || draft.item.description_bullets != confirmed.item.description_bullets)
        || !expected_fields.confirms(&confirmed)
    {
        return Err(DRAFT_UNCONFIRMED.into());
    }
    Ok(confirmed)
}

const DRAFT_UNCONFIRMED: &str = "Classroom did not confirm the draft. Reopen Classwork before trying again.";

async fn lookup_assignment_draft(session: &LiveSession, course: i64, id: i64) -> Result<TeacherDraft, String> {
    let key = classwork::StreamItemId {
        id: Some(id), course_id: Some(classwork::CourseId { id: Some(course) }),
    };
    let response: classwork::StreamItemLookupResponse = session.rpc(
        "LookupStreamItem",
        classwork::StreamItemLookupRequest { stream_item_id: vec![key], read_mask: Some(assignment_response_mask()) },
        Some(course),
    ).await?;
    let [result] = response.result.as_slice() else {
        return Err("Classroom did not return the selected assignment draft.".into());
    };
    let item = result.stream_item.clone()
        .ok_or("Classroom did not return the selected assignment draft.")?;
    exact_assignment_draft(course, Some(id), item)
        .ok_or_else(|| "Classroom did not confirm the selected assignment is still an editable draft.".into())
}

fn assignment_insert(
    course: i64, teacher: i64, title: &str, description: &str, fields: ValidatedAssignmentFields,
) -> mutation::StreamItemInsertRequest {
    let mut item = assignment_text_item(course, None, Some(title), Some(description));
    let assignment = item.assignment.as_mut().expect("assignment text item");
    assignment.task_info = Some(fields.task_info());
    let info = assignment.stream_item_info.as_mut().expect("assignment text info");
    info.creator_user_id = Some(classwork::UserId { id: Some(teacher) });
    info.status = Some(2);
    info.publication_info = Some(classwork::PublicationInfo { status: Some(3), ..Default::default() });
    mutation::StreamItemInsertRequest {
        stream_item: vec![item],
        read_mask: Some(assignment_response_mask()),
        ..Default::default()
    }
}

fn assignment_update(
    course: i64, id: i64, original: &TeacherDraft, current: &TeacherDraft, title: &str, description: &str,
    fields: ValidatedAssignmentFields,
) -> Result<mutation::StreamItemUpdateRequest, String> {
    if [original, current].iter().any(|draft| draft.publication != DraftPublication::Draft
        || draft.item.kind != CourseworkKind::Assignment || draft.item.course_id != course || draft.item.id != id.to_string())
    {
        return Err("Choose an unscheduled assignment draft in this class.".into());
    }
    if original.item.title != current.item.title || original.item.description != current.item.description
        || original.item.description_styles != current.item.description_styles
        || original.item.description_bullets != current.item.description_bullets
    {
        return Err("The draft changed elsewhere. Reopen it before saving.".into());
    }
    let title = (current.item.title != title).then_some(title);
    let description = (current.item.description != description).then_some(description);
    if description.is_some() && (!current.item.description_styles.is_empty() || !current.item.description_bullets.is_empty()) {
        return Err("These instructions use formatting. Keep them as written to change other assignment details.".into());
    }
    let text_mask = (title.is_some() || description.is_some()).then(|| mutation::StreamItemInfoUpdateMask {
        update_title: title.map(|_| true), update_description: description.map(|_| true), ..Default::default()
    });
    let task_mask = fields.update_mask();
    if text_mask.is_none() && task_mask.is_none() {
        return Err("There are no assignment changes to save.".into());
    }
    let mut item = assignment_text_item(course, Some(id), title, description);
    if task_mask.is_some() {
        item.assignment.as_mut().expect("assignment text item").task_info = Some(fields.task_info());
    }
    Ok(mutation::StreamItemUpdateRequest {
        stream_item: vec![item],
        update_mask: Some(mutation::StreamItemUpdateMask {
            assignment_mask: Some(mutation::AssignmentUpdateMask {
                stream_item_info_mask: text_mask,
                task_info_mask: task_mask,
            }),
            ..Default::default()
        }),
        read_mask: Some(assignment_response_mask()),
        ..Default::default()
    })
}

fn assignment_text_item(course: i64, id: Option<i64>, title: Option<&str>, description: Option<&str>) -> classwork::StreamItem {
    classwork::StreamItem {
        stream_type: Some(2),
        assignment: Some(classwork::Assignment {
            stream_item_info: Some(classwork::StreamItemInfo {
                stream_item_id: Some(classwork::StreamItemId {
                    id, course_id: Some(classwork::CourseId { id: Some(course) }),
                }),
                title: title.map(str::to_owned),
                description: description.map(str::to_owned),
                ..Default::default()
            }),
            task_info: None,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn assignment_response_mask() -> classwork::StreamReadMask {
    classwork::StreamReadMask {
        assignment_mask: Some(classwork::AssignmentReadMask {
            stream_item_info_mask: Some(classwork::StreamItemInfoReadMask {
                include_title: Some(true), include_description: Some(true), description_read_method: Some(2),
                publication_info_mask: Some(classwork::PublicationInfoReadMask {
                    include_publication_status: Some(true), include_scheduled_status: Some(true),
                    include_scheduled_timestamp: Some(true), ..Default::default()
                }),
                ..Default::default()
            }),
            task_info_mask: Some(classwork::TaskInfoReadMask {
                include_due_date: Some(true), include_user_specified_due_time: Some(true),
                include_grade_denominator: Some(true), ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn publication_update(course: i64, id: i64, action: AssignmentPublication) -> mutation::StreamItemUpdateRequest {
    let (status, time) = match action {
        AssignmentPublication::Publish => (Some(2), None),
        AssignmentPublication::Schedule(time) => (None, Some(time)),
    };
    let item = classwork::StreamItem {
        stream_type: Some(2),
        assignment: Some(classwork::Assignment {
            stream_item_info: Some(classwork::StreamItemInfo {
                stream_item_id: Some(classwork::StreamItemId {
                    id: Some(id), course_id: Some(classwork::CourseId { id: Some(course) }),
                }),
                publication_info: Some(classwork::PublicationInfo {
                    status, scheduled_timestamp: time, ..Default::default()
                }),
                ..Default::default()
            }),
            task_info: None,
            ..Default::default()
        }),
        ..Default::default()
    };
    mutation::StreamItemUpdateRequest {
        stream_item: vec![item],
        update_mask: Some(mutation::StreamItemUpdateMask {
            assignment_mask: Some(mutation::AssignmentUpdateMask {
                stream_item_info_mask: Some(mutation::StreamItemInfoUpdateMask {
                    publication_info_mask: Some(mutation::PublicationInfoUpdateMask {
                        update_status: status.map(|_| true), update_scheduled_timestamp: time.map(|_| true),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
        read_mask: Some(assignment_response_mask()),
        ..Default::default()
    }
}

fn confirm_publication(
    response: mutation::StreamItemUpsertResponse, course: i64, id: i64, action: AssignmentPublication,
) -> Result<(), String> {
    let uncertain = || "Classroom did not confirm the publication change. Check Classwork before retrying.".to_owned();
    let [result] = response.result.as_slice() else { return Err(uncertain()) };
    if let Some(code) = result.status.as_ref().and_then(|status| status.code).filter(|&code| code != 0) {
        return Err(format!("Classroom rejected the publication change (status {code})."));
    }
    let item = result.stream_item.as_ref().filter(|item| item.stream_type == Some(2)
        && item.post.is_none() && item.question.is_none() && item.supplement.is_none()).ok_or_else(uncertain)?;
    let info = item.assignment.as_ref().and_then(|assignment| assignment.stream_item_info.as_ref()).ok_or_else(uncertain)?;
    if info.stream_item_id.as_ref() != Some(&classwork::StreamItemId {
        id: Some(id), course_id: Some(classwork::CourseId { id: Some(course) }),
    }) {
        return Err(uncertain());
    }
    let publication = info.publication_info.as_ref().ok_or_else(uncertain)?;
    let confirmed = match action {
        AssignmentPublication::Publish => publication.status == Some(2),
        AssignmentPublication::Schedule(time) => publication.status == Some(3)
            && publication.scheduled_status == Some(1) && publication.scheduled_timestamp == Some(time),
    };
    confirmed.then_some(()).ok_or_else(uncertain)
}

fn confirm_assignment_draft(
    response: mutation::StreamItemUpsertResponse, course: i64, expected_id: Option<i64>, title: &str, description: &str,
) -> Result<TeacherDraft, String> {
    let [result] = response.result.as_slice() else { return Err(DRAFT_UNCONFIRMED.into()); };
    if let Some(code) = result.status.as_ref().and_then(|status| status.code).filter(|&code| code != 0) {
        return Err(format!("Classroom rejected the draft (status {code})."));
    }
    let item = result.stream_item.as_ref().filter(|item| item.stream_type == Some(2)
        && item.post.is_none() && item.question.is_none() && item.supplement.is_none())
        .ok_or(DRAFT_UNCONFIRMED)?;
    let draft = teacher_draft(course, item.clone()).ok_or(DRAFT_UNCONFIRMED)?;
    let actual_id = draft.item.id.parse::<i64>().map_err(|_| DRAFT_UNCONFIRMED)?;
    if actual_id <= 0 || expected_id.is_some_and(|id| id != actual_id)
        || draft.publication != DraftPublication::Draft || draft.item.kind != CourseworkKind::Assignment
        || draft.item.title != title || draft.item.description != description
    {
        return Err(DRAFT_UNCONFIRMED.into());
    }
    Ok(draft)
}

fn exact_assignment_draft(course: i64, expected_id: Option<i64>, item: classwork::StreamItem) -> Option<TeacherDraft> {
    let info = match &item {
        classwork::StreamItem {
            stream_type: Some(2), assignment: Some(assignment), post: None, question: None, supplement: None,
        } => assignment.stream_item_info.as_ref()?,
        _ => return None,
    };
    let publication = info.publication_info.as_ref()?;
    let actual_id = info.stream_item_id.as_ref()?.id?;
    if info.status != Some(2) || publication.status != Some(3)
        || !matches!(publication.scheduled_status, None | Some(0))
        || publication.scheduled_timestamp.is_some_and(|millis| millis != 0)
        || actual_id <= 0 || expected_id.is_some_and(|id| id != actual_id)
    {
        return None;
    }
    teacher_draft(course, item)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn publication_updates_mask_only_the_requested_field_on_the_exact_assignment() {
        let timestamp = 1_899_000_000_000;
        for action in [AssignmentPublication::Publish, AssignmentPublication::Schedule(timestamp)] {
            let update = publication_update(14, 66, action);
            let decoded = mutation::StreamItemUpdateRequest::decode(update.encode_to_vec().as_slice()).unwrap();
            let mask = decoded.update_mask.unwrap().assignment_mask.unwrap().stream_item_info_mask.unwrap();
            assert!(mask.update_title.is_none() && mask.update_description.is_none() && mask.update_status.is_none());
            let mask = mask.publication_info_mask.unwrap();
            let mut item = decoded.stream_item[0].clone();
            let info = item.assignment.as_mut().unwrap().stream_item_info.as_mut().unwrap();
            let publication = info.publication_info.as_mut().unwrap();
            match action {
                AssignmentPublication::Publish => {
                    assert_eq!((mask.update_status, mask.update_scheduled_timestamp), (Some(true), None));
                    assert_eq!((publication.status, publication.scheduled_timestamp), (Some(2), None));
                }
                AssignmentPublication::Schedule(_) => {
                    assert_eq!((mask.update_status, mask.update_scheduled_timestamp), (None, Some(true)));
                    assert_eq!((publication.status, publication.scheduled_timestamp), (None, Some(timestamp)));
                    publication.status = Some(3);
                    publication.scheduled_status = Some(1);
                }
            }
            let response = mutation::StreamItemUpsertResponse {
                result: vec![mutation::StreamItemUpsertResult { status: None, stream_item: Some(item) }],
            };
            assert!(confirm_publication(response.clone(), 14, 66, action).is_ok());
            assert!(confirm_publication(response.clone(), 14, 67, action).is_err());
            assert!(confirm_publication(response, 15, 66, action).is_err());
        }
    }

    #[test]
    fn new_assignment_is_an_active_draft_and_requires_a_confirmed_draft_response() {
        let request = assignment_insert(14, 23, "Essay", "Compare two sources.", ValidatedAssignmentFields::default().for_insert());
        let decoded = mutation::StreamItemInsertRequest::decode(request.encode_to_vec().as_slice()).unwrap();
        let mut item = decoded.stream_item[0].clone();
        let info = item.assignment.as_mut().unwrap().stream_item_info.as_mut().unwrap();
        assert_eq!(info.status, Some(2));
        assert_eq!(info.publication_info.as_ref().unwrap().status, Some(3));
        assert_eq!(info.creator_user_id.as_ref().unwrap().id, Some(23));
        assert_eq!(info.stream_item_id.as_ref().unwrap().id, None);
        info.stream_item_id.as_mut().unwrap().id = Some(61);
        let response = mutation::StreamItemUpsertResponse {
            result: vec![mutation::StreamItemUpsertResult { status: None, stream_item: Some(item) }],
        };
        assert!(confirm_assignment_draft(response.clone(), 14, None, "Essay", "Compare two sources.").is_ok());
        assert!(confirm_assignment_draft(response.clone(), 14, Some(62), "Essay", "Compare two sources.").is_err());
        assert!(confirm_assignment_draft(response, 15, None, "Essay", "Compare two sources.").is_err());
    }

    #[test]
    fn assignment_edit_combines_title_with_points_and_clears_only_the_requested_deadline() {
        let initial = AssignmentFieldChanges {
            due: Some(super::super::assignment_fields::parse_assignment_due("2028-02-29", "").unwrap()),
            points: None,
        }.validate().unwrap().for_insert();
        let mut item = assignment_insert(14, 23, "Essay", "Read first.", initial).stream_item.remove(0);
        item.assignment.as_mut().unwrap().stream_item_info.as_mut().unwrap().stream_item_id.as_mut().unwrap().id = Some(61);
        let mut draft = exact_assignment_draft(14, Some(61), item).unwrap();
        draft.item.description_bullets = vec![0];
        let patch = AssignmentFieldChanges { due: Some(None), points: Some(Some(75.0)) }
            .validate().unwrap().for_update(&draft, &draft).unwrap();

        let request = assignment_update(14, 61, &draft, &draft, "Revised essay", "Read first.", patch).unwrap();
        let assignment = request.stream_item[0].assignment.as_ref().unwrap();
        let info = assignment.stream_item_info.as_ref().unwrap();
        assert_eq!(info.title.as_deref(), Some("Revised essay"));
        assert!(info.description.is_none());
        assert!(info.publication_info.is_none());
        let task = assignment.task_info.as_ref().unwrap();
        assert_eq!((task.due_date, task.user_specified_due_time, task.grade_denominator), (None, None, Some(75.0)));
        let mask = request.update_mask.unwrap().assignment_mask.unwrap();
        assert_eq!(mask.task_info_mask.unwrap().encode_to_vec(), [0x08, 0x01, 0x18, 0x01]);
        let text_mask = mask.stream_item_info_mask.unwrap();
        assert_eq!((text_mask.update_title, text_mask.update_description), (Some(true), None));
        assert!(!patch.confirms(&draft));
        draft.item.maximum_points = Some(75.0);
        draft.item.due_at_millis = None;
        assert!(patch.confirms(&draft));
    }

    #[test]
    fn assignment_edit_ignores_unchanged_point_patches_and_rejects_stale_selected_fields() {
        let mut item = assignment_insert(14, 23, "Essay", "Read first.", ValidatedAssignmentFields::default().for_insert())
            .stream_item.remove(0);
        item.assignment.as_mut().unwrap().stream_item_info.as_mut().unwrap().stream_item_id.as_mut().unwrap().id = Some(61);
        let original = exact_assignment_draft(14, Some(61), item).unwrap();
        let mut current = original.clone();
        current.item.maximum_points = Some(50.0);
        let fields = AssignmentFieldChanges { due: None, points: Some(None) }.validate().unwrap();
        assert!(fields.for_update(&original, &current).is_err());
        let fields = AssignmentFieldChanges::default().validate().unwrap();
        assert!(fields.for_update(&original, &current).unwrap().update_mask().is_none());
        let fields = AssignmentFieldChanges { due: None, points: Some(Some(100.0)) }.validate().unwrap();
        assert!(fields.for_update(&original, &original).unwrap().update_mask().is_none());
    }

    #[test]
    fn assignment_insert_writes_custom_deadline_and_ungraded_in_one_active_draft() {
        let fields = AssignmentFieldChanges {
            due: Some(super::super::assignment_fields::parse_assignment_due("2028-02-29", "").unwrap()),
            points: Some(None),
        }.validate().unwrap().for_insert();
        let request = assignment_insert(14, 23, "Essay", "Read first.", fields);
        let mut item = request.stream_item.into_iter().next().unwrap();
        let assignment = item.assignment.as_mut().unwrap();
        let task = assignment.task_info.as_ref().unwrap();
        assert_eq!(task.grade_denominator, Some(0.0));
        assert_eq!(task.user_specified_due_time, Some(false));
        assert!(task.due_date.is_some());
        assignment.stream_item_info.as_mut().unwrap().stream_item_id.as_mut().unwrap().id = Some(61);
        let confirmed = exact_assignment_draft(14, Some(61), item.clone()).unwrap();
        assert!(fields.confirms(&confirmed));
        assert!(exact_assignment_draft(96, Some(61), item.clone()).is_none());
        assert!(exact_assignment_draft(14, Some(62), item.clone()).is_none());
        item.assignment.as_mut().unwrap().stream_item_info.as_mut().unwrap()
            .publication_info.as_mut().unwrap().scheduled_status = Some(1);
        assert!(exact_assignment_draft(14, Some(61), item).is_none());
    }

    #[test]
    fn text_update_preserves_publication_and_rejects_a_changed_or_formatted_description() {
        let mut item = assignment_insert(14, 23, "Essay", "Read first.", ValidatedAssignmentFields::default().for_insert()).stream_item.remove(0);
        item.assignment.as_mut().unwrap().stream_item_info.as_mut().unwrap().stream_item_id.as_mut().unwrap().id = Some(61);
        let old = teacher_draft(14, item).unwrap();
        let update = assignment_update(14, 61, &old, &old, "Revised essay", "", ValidatedAssignmentFields::default()).unwrap();
        let updated = update.stream_item[0].assignment.as_ref().unwrap().stream_item_info.as_ref().unwrap();
        assert!(updated.publication_info.is_none());
        assert!(updated.status.is_none());
        assert_eq!(updated.description.as_deref(), Some(""));
        let mask = update.update_mask.unwrap().assignment_mask.unwrap().stream_item_info_mask.unwrap();
        assert_eq!((mask.update_title, mask.update_description, mask.update_status), (Some(true), Some(true), None));

        let mut formatted = old.clone();
        formatted.item.description_bullets = vec![0];
        assert!(assignment_update(14, 61, &old, &formatted, "Other essay", "Read first.", ValidatedAssignmentFields::default()).is_err());
        assert!(assignment_update(14, 61, &formatted, &formatted, "Other essay", "Override formatting", ValidatedAssignmentFields::default()).is_err());
        let title = assignment_update(14, 61, &formatted, &formatted, "Other essay", "Read first.", ValidatedAssignmentFields::default()).unwrap();
        assert!(title.stream_item[0].assignment.as_ref().unwrap().stream_item_info.as_ref().unwrap().description.is_none());
    }
}
