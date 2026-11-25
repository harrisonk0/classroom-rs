use super::{
    wire, classwork, mutation, check_class, classroom_event_time, rubric, LiveSession, BTreeMap,
    Coursework, CourseworkKind, Grade, QuestionAnswer, QuestionDetails, SubmissionStatus,
    load_coursework, material, Message,
};

pub(super) fn own_question_answer(answer: classwork::QuestionSubmission) -> Option<QuestionAnswer> {
    let short = answer.short_answer.filter(|text| !text.trim().is_empty());
    let choice = answer.multiple_choice_answer.filter(|text| !text.trim().is_empty());
    match (answer.question_answer_type, short, choice) {
        (Some(1) | None | Some(0), Some(text), None) => Some(QuestionAnswer::ShortAnswer(text)),
        (Some(2) | None | Some(0), None, Some(text)) => Some(QuestionAnswer::MultipleChoice(text)),
        (Some(1), Some(text), Some(_)) => Some(QuestionAnswer::ShortAnswer(text)),
        (Some(2), Some(_), Some(text)) => Some(QuestionAnswer::MultipleChoice(text)),
        _ => None,
    }
}

pub async fn load_submissions(session: &LiveSession, course_id: i64) -> Result<Vec<(String, crate::desktop_data::Submission)>, String> {
    use classwork::*;
    check_class(course_id)?;
    let response: wire::QuerySubmissionResponse = session.rpc("QuerySubmission", wire::QuerySubmissionRequest {
        submission_read_mask: Some(SubmissionReadMask {
            include_current_display_state_v2: Some(true),
            include_current_state: Some(true),
            include_live_grade_numerator: Some(true),
            include_last_turned_in_timestamp: Some(true),
            question_submission_mask: Some(QuestionSubmissionReadMask { include_answer: Some(true) }),
            live_rubric_score_mask: Some(RubricScoreReadMask {
                criterion_score_mask: Some(CriterionScoreReadMask { include_rating_id: Some(true), include_points: Some(true) }),
            }),
            assignment_submission_mask: Some(AssignmentSubmissionReadMask {
                material_mask: Some(MaterialReadMask {
                    include_name: Some(true),
                    include_mime_type: Some(true),
                    include_open_url: Some(true),
                    include_parent_reference: Some(true),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        }.encode_to_vec()),
        filter_options: Some(SubmissionFilter {
            base_filter_options: Some(FilterOptions {
                user_filter: vec![UserFilter { use_current_user: Some(true), user_id: None }],
            }),
            course_id: vec![CourseId { id: Some(course_id) }],
            ..Default::default()
        }.encode_to_vec()),
    }, Some(course_id)).await?;

    let mut results = BTreeMap::new();
    for bytes in response.submission {
        let wire = classwork::Submission::decode(bytes.as_slice())
            .map_err(|_| "Google returned a submission this build can't decode.".to_owned())?;
        let Some(submission_id) = wire.submission_id else { continue };
        let Some(key) = submission_id.stream_item_id else { continue };
        if key.course_id.and_then(|course| course.id) != Some(course_id) { continue }
        let Some(id) = key.id else { continue };
        let state = wire.current_display_state_v2.unwrap_or_default();
        let status = submission_display_status(state);
        let grade = if matches!(state, 5 | 8 | 10) {
            wire.live_grade_numerator.filter(|value| value.is_finite())
                .map(|earned| Grade { earned, maximum: None })
        } else {
            None
        };
        let entry = crate::desktop_data::Submission {
            remote_id: submission_id.id.filter(|id| *id > 0),
            remote_state: wire.current_state,
            attachments: wire.assignment_submission.into_iter()
                .flat_map(|assignment| assignment.material).filter_map(material).collect(),
            status,
            grade,
            last_turned_in_millis: classroom_event_time(wire.last_turned_in_timestamp),
            question_answer: wire.question_submission.and_then(own_question_answer),
            rubric_scores: rubric::released_scores(status, wire.live_rubric_score),
        };
        results.entry(id.to_string())
            .and_modify(|old: &mut crate::desktop_data::Submission| old.remote_id = None)
            .or_insert(entry);
    }
    Ok(results.into_iter().collect())
}

pub(super) fn submission_display_status(state: i32) -> SubmissionStatus {
    match state {
        1 => SubmissionStatus::Assigned,
        2 => SubmissionStatus::Missing,
        3 => SubmissionStatus::TurnedIn,
        4 => SubmissionStatus::TurnedInLate,
        5..=8 => SubmissionStatus::Returned,
        9 | 10 => SubmissionStatus::ReturnedLate,
        11 => SubmissionStatus::Excused,
        _ => SubmissionStatus::Unknown,
    }
}

/// Uses the signed-in student filter and reads history for one exact submission.
pub async fn load_own_submission_history(
    session: &LiveSession, course_id: i64, item_id: i64, submission_id: i64,
) -> Result<Vec<crate::desktop_data::SubmissionEvent>, String> {
    use classwork::*;
    check_class(course_id)?;
    if item_id <= 0 || submission_id <= 0 {
        return Err("Classroom has not confirmed a submission for this item.".into());
    }
    let key = SubmissionId {
        id: Some(submission_id),
        stream_item_id: Some(StreamItemId { id: Some(item_id), course_id: Some(CourseId { id: Some(course_id) }) }),
    };
    let response: wire::QuerySubmissionResponse = session.rpc("QuerySubmission", wire::QuerySubmissionRequest {
        submission_read_mask: Some(SubmissionReadMask {
            include_submission_history: Some(true),
            ..Default::default()
        }.encode_to_vec()),
        filter_options: Some(SubmissionFilter {
            base_filter_options: Some(FilterOptions {
                user_filter: vec![UserFilter { use_current_user: Some(true), user_id: None }],
            }),
            submission_id: vec![key.clone()],
            ..Default::default()
        }.encode_to_vec()),
    }, Some(course_id)).await?;

    let mut matching = None;
    for bytes in response.submission {
        let submission = classwork::Submission::decode(bytes.as_slice())
            .map_err(|_| "Google returned an unreadable submission history.")?;
        if submission.submission_id.as_ref() == Some(&key)
            && matching.replace(submission.submission_history).is_some()
        {
            return Err("Google returned more than one history for this submission.".into());
        }
    }
    matching.map(decode_submission_history)
        .ok_or_else(|| "Google did not return the exact submission in the history lookup.".into())
}

fn decode_submission_history(history: Vec<classwork::SubmissionHistory>) -> Vec<crate::desktop_data::SubmissionEvent> {
    use crate::desktop_data::SubmissionEvent;
    let mut seen = std::collections::BTreeSet::new();
    let mut events: Vec<_> = history.into_iter().filter_map(|history| {
        let event = history.state_history?;
        let state = event.state?;
        let status = match state {
            2 => SubmissionStatus::TurnedIn,
            3 => SubmissionStatus::Returned,
            5 => SubmissionStatus::Reclaimed,
            6 => SubmissionStatus::Excused,
            _ => return None,
        };
        let timestamp_millis = classroom_event_time(event.state_timestamp)?;
        seen.insert((timestamp_millis, state)).then_some(SubmissionEvent { status, timestamp_millis })
    }).collect();
    events.sort_by_key(|event| std::cmp::Reverse(event.timestamp_millis));
    events
}

#[cfg(test)]
#[test]
fn submission_history_keeps_only_distinct_state_events_with_millisecond_timestamps() {
    use classwork::{SubmissionHistory, SubmissionStateHistory};
    let at = 1_784_600_123_000;
    let event = |state, timestamp| SubmissionHistory {
        state_history: Some(SubmissionStateHistory { state: Some(state), state_timestamp: Some(timestamp) }),
    };
    let decoded = decode_submission_history(vec![
        event(2, at), event(4, at), event(3, at + 1), event(5, at),
        event(2, at), event(2, at / 1000), SubmissionHistory::default(),
    ]);
    assert_eq!(decoded.iter().map(|event| (event.status, event.timestamp_millis)).collect::<Vec<_>>(), [
        (SubmissionStatus::Returned, at + 1), (SubmissionStatus::TurnedIn, at), (SubmissionStatus::Reclaimed, at),
    ]);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionAction {
    TurnIn,
    Unsubmit,
}

impl SubmissionAction {
    pub fn label(self) -> &'static str {
        match self { Self::TurnIn => "Turn in", Self::Unsubmit => "Unsubmit" }
    }

    pub fn for_submission(submission: &crate::desktop_data::Submission) -> Option<Self> {
        submission.remote_id?;
        match submission.remote_state? {
            4 | 5 => Some(Self::TurnIn),
            2 => Some(Self::Unsubmit),
            _ => None,
        }
    }

    fn state(self) -> i32 {
        match self { Self::TurnIn => 2, Self::Unsubmit => 5 }
    }
}

/// State transition for an existing submission belonging to the current user.
pub async fn change_submission(
    session: &LiveSession,
    course_id: i64,
    item_id: i64,
    action: SubmissionAction,
) -> Result<(), String> {
    check_class(course_id)?;
    let item_key = item_id.to_string();
    let (_, submission) = load_submissions(session, course_id).await?
        .into_iter().find(|(id, _)| id == &item_key)
        .ok_or("Classroom did not return an existing submission for this item.")?;
    if SubmissionAction::for_submission(&submission) != Some(action) {
        return Err("Submission status changed. Refresh your work before trying again.".into());
    }
    let submission_id = classwork::SubmissionId {
        id: submission.remote_id,
        stream_item_id: Some(classwork::StreamItemId {
            id: Some(item_id),
            course_id: Some(classwork::CourseId { id: Some(course_id) }),
        }),
    };
    let response: mutation::WriteSubmissionResponse = session.write_rpc(
        "WriteSubmission", mutation::WriteSubmissionRequest {
            request_header: Some(mutation::WriteRequestHeader { operation: Some(3) }),
            submission_mutation: vec![mutation::SubmissionMutation {
                submission_id: Some(submission_id.clone()),
                submission: Some(classwork::Submission {
                    submission_id: Some(submission_id.clone()),
                    current_state: Some(action.state()),
                    ..Default::default()
                }),
                submission_update_mask: Some(mutation::SubmissionUpdateMask {
                    update_current_state: Some(true),
                    ..Default::default()
                }),
                add_materials: vec![],
            }],
            submission_read_mask: Some(classwork::SubmissionReadMask {
                include_current_state: Some(true),
                ..Default::default()
            }),
        }, Some(course_id),
    ).await.map_err(|error| format!("{error} Refresh your work before trying again."))?;
    if response.submission.iter().any(|entry| {
        entry.submission_id.as_ref() == Some(&submission_id) && entry.current_state == Some(action.state())
    }) {
        Ok(())
    } else {
        Err("Classroom returned no confirmation. Refresh your work before trying again.".into())
    }
}

pub fn can_turn_in_question(item: &Coursework) -> bool {
    item.kind == CourseworkKind::Question && item.question.is_some()
        && item.submission.as_ref().is_some_and(|submission| {
            submission.question_answer.is_none()
                && SubmissionAction::for_submission(submission) == Some(SubmissionAction::TurnIn)
        })
}

#[derive(Clone, PartialEq, Eq)]
pub struct ShortAnswerRevision {
    pub submission_id: i64,
    pub answer: String,
}

pub fn editable_short_answer(item: &Coursework) -> Option<(i64, &str)> {
    if item.kind != CourseworkKind::Question || item.question != Some(QuestionDetails::ShortAnswer)
        || !item.allow_edit_after_turn_in
    {
        return None;
    }
    let submission = item.submission.as_ref()?;
    if submission.remote_state != Some(2) || submission.grade.is_some()
        || !matches!(submission.status, SubmissionStatus::TurnedIn | SubmissionStatus::TurnedInLate)
    {
        return None;
    }
    let QuestionAnswer::ShortAnswer(answer) = submission.question_answer.as_ref()? else { return None };
    Some((submission.remote_id?, answer.as_str()))
}

pub fn question_submission_is_current(item: &Coursework, revision: Option<&ShortAnswerRevision>) -> bool {
    match revision {
        Some(revision) => editable_short_answer(item) == Some((revision.submission_id, revision.answer.as_str())),
        None => can_turn_in_question(item),
    }
}

/// Turn in a first answer or update a permitted short answer for the current user.
pub async fn turn_in_question(
    session: &LiveSession, course_id: i64, item_id: i64, answer: QuestionAnswer,
    revision: Option<ShortAnswerRevision>,
) -> Result<(), String> {
    check_class(course_id)?;
    if item_id <= 0 {
        return Err("Open a published question first.".into());
    }
    let item_key = item_id.to_string();
    let (items, submissions) = tokio::try_join!(
        load_coursework(session, course_id),
        load_submissions(session, course_id),
    )?;
    let mut question = items.items.into_iter().find(|item| item.id == item_key)
        .ok_or("This question is no longer available.")?;
    question.submission = submissions.into_iter().find(|(id, _)| id == &item_key)
        .map(|(_, submission)| submission);
    if !question_submission_is_current(&question, revision.as_ref()) {
        return Err("The answer or its editing permissions changed in Classroom. Review your work before trying again.".into());
    }
    if !question.question.as_ref().is_some_and(|details| details.accepts(&answer)) {
        return Err("This answer no longer matches the published question.".into());
    }
    if let (Some(revision), QuestionAnswer::ShortAnswer(text)) = (&revision, &answer)
        && text == &revision.answer
    {
        return Err("Make a change to your answer before turning it in.".into());
    }
    let id = classwork::SubmissionId {
        id: question.submission.and_then(|submission| submission.remote_id),
        stream_item_id: Some(classwork::StreamItemId {
            id: Some(item_id),
            course_id: Some(classwork::CourseId { id: Some(course_id) }),
        }),
    };
    let response: mutation::WriteSubmissionResponse = session.write_rpc(
        "WriteSubmission", question_answer_request(id.clone(), answer.clone()), Some(course_id),
    ).await.map_err(|error| format!("{error} Check your work before trying again."))?;
    if response.submission.iter().any(|entry| {
        entry.submission_id.as_ref() == Some(&id)
            && entry.current_state == Some(SubmissionAction::TurnIn.state())
            && entry.question_submission.clone().and_then(own_question_answer).as_ref() == Some(&answer)
    }) {
        Ok(())
    } else {
        Err("Classroom did not confirm this answer. Check your work before trying again.".into())
    }
}

pub(super) fn question_answer_request(id: classwork::SubmissionId, answer: QuestionAnswer) -> mutation::WriteSubmissionRequest {
    let question_submission = match answer {
        QuestionAnswer::ShortAnswer(text) => classwork::QuestionSubmission {
            short_answer: Some(text),
            ..Default::default()
        },
        QuestionAnswer::MultipleChoice(choice) => classwork::QuestionSubmission {
            multiple_choice_answer: Some(choice),
            question_answer_type: Some(2),
            ..Default::default()
        },
    };
    mutation::WriteSubmissionRequest {
        request_header: Some(mutation::WriteRequestHeader { operation: Some(3) }),
        submission_mutation: vec![mutation::SubmissionMutation {
            submission_id: Some(id.clone()),
            submission: Some(classwork::Submission {
                submission_id: Some(id),
                current_state: Some(SubmissionAction::TurnIn.state()),
                question_submission: Some(question_submission),
                ..Default::default()
            }),
            submission_update_mask: Some(mutation::SubmissionUpdateMask {
                update_current_state: Some(true),
                question_submission_mask: Some(mutation::QuestionSubmissionUpdateMask { update_answer: Some(true) }),
                ..Default::default()
            }),
            add_materials: vec![],
        }],
        submission_read_mask: Some(classwork::SubmissionReadMask {
            include_current_state: Some(true),
            question_submission_mask: Some(classwork::QuestionSubmissionReadMask { include_answer: Some(true) }),
            ..Default::default()
        }),
    }
}
