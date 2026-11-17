use super::{LiveSession, classwork, mutation, insert_comment, load_coursework, load_own_class_details, load_user_names, material, own_question_answer, query_submission_feedback,
    submission_display_status, wire};
use crate::desktop_data::{Attachment, CourseRole, QuestionAnswer, SubmissionStatus};
use prost::Message;
use iced::futures::{TryStreamExt, StreamExt, stream};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct TeacherSubmission {
    pub student_id: i64,
    pub submission_id: i64,
    pub remote_state: Option<i32>,
    pub name: Option<String>,
    pub status: SubmissionStatus,
    pub released_grade: Option<f64>,
    pub draft_grade: Option<f64>,
    pub attachments: Vec<Attachment>,
    pub answer: Option<QuestionAnswer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TeacherFeedbackKey {
    pub course: i64,
    pub item: i64,
    pub student: i64,
    pub submission: i64,
}

pub async fn load_teacher_private_feedback(
    session: &LiveSession, key: TeacherFeedbackKey,
) -> Result<Vec<crate::desktop_data::Comment>, String> {
    let submission = confirm_teacher_feedback_target(session, key).await?;
    query_submission_feedback(session, key.course, submission).await
}

pub async fn post_teacher_private_feedback(
    session: &LiveSession, key: TeacherFeedbackKey, body: String,
) -> Result<(), String> {
    let submission = confirm_teacher_feedback_target(session, key).await?;
    insert_comment(session, key.course, classwork::CommentId {
        submission_id: Some(submission),
        parent_type: Some(3),
        ..Default::default()
    }, 1, &body).await
}

async fn confirm_teacher_feedback_target(
    session: &LiveSession, key: TeacherFeedbackKey,
) -> Result<classwork::SubmissionId, String> {
    if [key.course, key.item, key.student, key.submission].iter().any(|id| *id <= 0) {
        return Err("Choose a student submission first.".into());
    }
    let membership = load_own_class_details(session, vec![key.course]).await?;
    if membership.roles.get(&key.course) != Some(&CourseRole::Teacher) {
        return Err("Classroom did not confirm teacher access to this private thread.".into());
    }
    let target = submission_key(key.course, key.item, key.submission);
    let response: wire::QuerySubmissionResponse = session.rpc(
        "QuerySubmission", feedback_target_request(&target), Some(key.course),
    ).await?;
    if !has_exact_feedback_target(response, key.student, &target)? {
        return Err("Classroom did not confirm the selected student's submission.".into());
    }
    Ok(target)
}

fn feedback_target_request(target: &classwork::SubmissionId) -> wire::QuerySubmissionRequest {
    wire::QuerySubmissionRequest {
        filter_options: Some(classwork::SubmissionFilter {
            submission_id: vec![target.clone()], ..Default::default()
        }.encode_to_vec()),
        submission_read_mask: Some(classwork::SubmissionReadMask {
            include_student_user_id: Some(true), ..Default::default()
        }.encode_to_vec()),
    }
}

fn has_exact_feedback_target(response: wire::QuerySubmissionResponse, student: i64, target: &classwork::SubmissionId) -> Result<bool, String> {
    let [bytes] = response.submission.as_slice() else { return Ok(false); };
    let row = classwork::Submission::decode(bytes.as_slice()).map_err(|_| "Unreadable submission identity.")?;
    Ok(row.submission_id.as_ref() == Some(target) && row.student_user_id.as_ref().and_then(|user| user.id) == Some(student))
}

pub async fn load_teacher_submissions(
    session: &LiveSession, course: i64, item: i64,
) -> Result<Vec<TeacherSubmission>, String> {
    let mut rows = load_teacher_rows(session, course, item).await?;
    let names = load_user_names(session, rows.iter().map(|row| row.student_id).collect()).await.unwrap_or_default();
    for row in &mut rows {
        row.name = names.get(&row.student_id).filter(|name| !name.trim().is_empty()).cloned();
    }
    rows.sort_by(|left, right| {
        let name_key = |row: &TeacherSubmission| row.name.as_deref().unwrap_or("").to_lowercase();
        (left.name.is_none(), name_key(left), left.student_id).cmp(&(right.name.is_none(), name_key(right), right.student_id))
    });
    Ok(rows)
}

async fn load_teacher_rows(session: &LiveSession, course: i64, item: i64) -> Result<Vec<TeacherSubmission>, String> {
    super::check_class(course)?;
    if item <= 0 {
        return Err("Choose a published assignment or question.".into());
    }
    let membership = load_own_class_details(session, vec![course]).await?;
    if membership.roles.get(&course) != Some(&CourseRole::Teacher) {
        return Err("Classroom did not confirm teacher access to this class.".into());
    }
    let parent = classwork::StreamItemId {
        id: Some(item), course_id: Some(classwork::CourseId { id: Some(course) }),
    };
    let result: wire::QuerySubmissionResponse = session.rpc(
        "QuerySubmission", teacher_submissions_request(&parent), Some(course),
    ).await?;
    decode_teacher_submissions(result, &parent)
}

fn teacher_submissions_request(parent: &classwork::StreamItemId) -> wire::QuerySubmissionRequest {
    submission_query(vec![parent.clone()], true)
}

fn submission_query(parents: Vec<classwork::StreamItemId>, review: bool) -> wire::QuerySubmissionRequest {
    use classwork::*;
    wire::QuerySubmissionRequest {
        submission_read_mask: Some(SubmissionReadMask {
            include_student_user_id: Some(true),
            include_current_state: Some(true),
            include_current_display_state_v2: Some(true),
            include_live_grade_numerator: Some(true),
            include_draft_grade_numerator: Some(true),
            question_submission_mask: review.then_some(QuestionSubmissionReadMask { include_answer: Some(true) }),
            assignment_submission_mask: review.then_some(AssignmentSubmissionReadMask {
                material_mask: Some(MaterialReadMask {
                    include_name: Some(true), include_mime_type: Some(true), include_open_url: Some(true),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        }.encode_to_vec()),
        filter_options: Some(SubmissionFilter { stream_item_id: parents, ..Default::default() }.encode_to_vec()),
    }
}

#[derive(Debug, Clone)]
pub struct TeacherGradebook {
    pub students: Vec<crate::desktop_data::CourseMember>,
    pub cells: BTreeMap<(i64, i64), TeacherGradebookCell>,
}

#[derive(Debug, Clone, Copy)]
pub struct TeacherGradebookCell {
    pub submission_id: i64,
    pub remote_state: Option<i32>,
    pub status: SubmissionStatus,
    pub released_grade: Option<f64>,
    pub draft_grade: Option<f64>,
}

impl TeacherGradebookCell {
    pub fn grading_row(self, student_id: i64) -> TeacherSubmission {
        TeacherSubmission {
            student_id, submission_id: self.submission_id, remote_state: self.remote_state,
            name: None, status: self.status, released_grade: self.released_grade, draft_grade: self.draft_grade,
            attachments: Vec::new(), answer: None,
        }
    }

    pub fn can_edit(self) -> bool {
        self.submission_id > 0 && matches!(self.remote_state, Some(2..=5))
    }
}

pub async fn load_teacher_gradebook(
    session: &LiveSession, course: i64, items: Vec<i64>,
) -> Result<TeacherGradebook, String> {
    super::check_class(course)?;
    let membership = load_own_class_details(session, vec![course]).await?;
    if membership.roles.get(&course) != Some(&CourseRole::Teacher) {
        return Err("Classroom did not confirm teacher access to this gradebook.".into());
    }
    let mut students: Vec<_> = super::load_roster(session, course).await?.into_iter()
        .filter(|student| student.role == CourseRole::Student && student.id.parse::<i64>().is_ok())
        .collect();
    students.sort_by_key(|student| student.display_name.to_lowercase());
    let student_ids: BTreeSet<_> = students.iter().filter_map(|student| student.id.parse::<i64>().ok()).collect();
    let items: Vec<_> = items.into_iter().filter(|&id| id > 0).collect::<BTreeSet<_>>().into_iter().collect();
    let mut cells = BTreeMap::new();
    if !student_ids.is_empty() {
        let coursework_batches: Vec<_> = items.chunks(12).map(<[i64]>::to_vec).collect();
        let batches: Vec<BTreeMap<_, _>> = stream::iter(coursework_batches).map(|batch| {
            let student_ids = &student_ids;
            async move {
                let parents = batch.iter().map(|&id| classwork::StreamItemId {
                    id: Some(id), course_id: Some(classwork::CourseId { id: Some(course) }),
                }).collect();
                let response = session.rpc("QuerySubmission", submission_query(parents, false), Some(course)).await?;
                gradebook_cells(response, course, &batch, student_ids)
            }
        }).buffer_unordered(2).try_collect().await?;
        cells.extend(batches.into_iter().flatten());
    }
    Ok(TeacherGradebook { students, cells })
}

fn gradebook_cells(
    response: wire::QuerySubmissionResponse, course: i64, items: &[i64], students: &BTreeSet<i64>,
) -> Result<BTreeMap<(i64, i64), TeacherGradebookCell>, String> {
    let mut cells = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for bytes in response.submission {
        let row = classwork::Submission::decode(bytes.as_slice()).map_err(|_| "Unreadable gradebook row.")?;
        let Some(submission) = row.submission_id else { continue; };
        let (Some(submission_id), Some(parent)) = (submission.id.filter(|id| *id > 0), submission.stream_item_id)
            else { continue; };
        let Some(item) = parent.id.filter(|item| items.contains(item)) else { continue };
        if parent.course_id.and_then(|key| key.id) != Some(course) {
            continue;
        }
        let Some(student) = row.student_user_id.and_then(|user| user.id).filter(|id| students.contains(id)) else { continue };
        let key = (student, item);
        if !seen.insert(key) {
            cells.remove(&key);
            continue;
        }
        cells.insert(key, TeacherGradebookCell {
            submission_id, remote_state: row.current_state,
            status: submission_display_status(row.current_display_state_v2.unwrap_or_default()),
            released_grade: row.live_grade_numerator.filter(|value| value.is_finite()),
            draft_grade: row.draft_grade_numerator.filter(|value| value.is_finite()),
        });
    }
    Ok(cells)
}

fn decode_teacher_submissions(
    response: wire::QuerySubmissionResponse, parent: &classwork::StreamItemId,
) -> Result<Vec<TeacherSubmission>, String> {
    let mut rows = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    let total = response.submission.len();
    for bytes in response.submission {
        let row = classwork::Submission::decode(bytes.as_slice()).map_err(|_| "Unreadable student submission.")?;
        let Some(submission_id) = row.submission_id.as_ref().filter(|id| id.stream_item_id.as_ref() == Some(parent))
            .and_then(|key| key.id).filter(|id| *id > 0) else { continue; };
        let Some(student_id) = row.student_user_id.and_then(|student| student.id).filter(|id| *id > 0) else { continue; };
        matched += 1;
        if !seen.insert(student_id) {
            rows.remove(&student_id);
            continue;
        }
        let status = submission_display_status(row.current_display_state_v2.unwrap_or_default());
        let released_grade = row.live_grade_numerator.filter(|value| value.is_finite());
        let draft_grade = row.draft_grade_numerator.filter(|value| value.is_finite());
        rows.insert(student_id, TeacherSubmission {
            student_id, submission_id, remote_state: row.current_state, name: None, status, released_grade, draft_grade,
            attachments: row.assignment_submission.into_iter().flat_map(|submission| submission.material)
                .filter_map(material).collect(),
            answer: row.question_submission.and_then(own_question_answer),
        });
    }
    if total > 0 && (matched == 0 || rows.is_empty()) {
        return Err("Classroom returned no unambiguous submissions for this assignment.".into());
    }
    Ok(rows.into_values().collect())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TeacherGradingAction {
    SaveDraft(f64),
    Return,
}

impl TeacherGradingAction {
    pub fn allowed(self, row: &TeacherSubmission, maximum_points: Option<f64>) -> bool {
        if row.submission_id <= 0 || row.student_id <= 0 || !matches!(row.remote_state, Some(2..=5)) {
            return false;
        }
        match self {
            Self::SaveDraft(value) => maximum_points.is_some_and(|points| points.is_finite() && points > 0.0)
                && value.is_finite() && value >= 0.0 && Some(value) != row.draft_grade.or(row.released_grade),
            Self::Return => row.remote_state == Some(2) || row.draft_grade.is_some_and(|value| Some(value) != row.released_grade),
        }
    }
}

pub async fn write_teacher_grade(
    session: &LiveSession, course: i64, item: i64, expected: TeacherSubmission, action: TeacherGradingAction,
) -> Result<(), String> {
    let current = load_teacher_rows(session, course, item).await?.into_iter()
        .find(|row| row.student_id == expected.student_id && row.submission_id == expected.submission_id)
        .ok_or("Classroom did not return the selected student's submission.")?;
    if !same_grade_state(&current, &expected) {
        return Err("The submission changed. Review the latest grade before trying again.".into());
    }
    let published = published_teacher_item(session, course, item).await?;
    if !action.allowed(&current, published.maximum_points) {
        return Err("Classroom did not return a submission eligible for this action.".into());
    }
    let key = submission_key(course, item, current.submission_id);
    let request = teacher_grade_request(key.clone(), &current, action);
    let response = session.write_rpc("WriteSubmission", request, Some(course)).await
        .map_err(|error| format!("{error} Check Student work before retrying."))?;
    confirm_teacher_grade(response, &key, &current, action)
}

pub async fn return_teacher_submissions(
    session: &LiveSession, course: i64, item: i64, expected: Vec<TeacherSubmission>,
) -> Result<usize, String> {
    if expected.is_empty() {
        return Err("Select at least one submission to return.".into());
    }
    let current: BTreeMap<_, _> = load_teacher_rows(session, course, item).await?.into_iter()
        .map(|row| (row.student_id, row)).collect();
    let published = published_teacher_item(session, course, item).await?;
    let mut distinct = BTreeSet::new();
    let mut selected = Vec::with_capacity(expected.len());
    for original in &expected {
        let Some(row) = current.get(&original.student_id).filter(|row| row.submission_id == original.submission_id
            && same_grade_state(row, original) && TeacherGradingAction::Return.allowed(row, published.maximum_points))
        else {
            return Err("A selected submission changed. Review Student work and select the submissions again.".into());
        };
        if !distinct.insert(row.submission_id) {
            return Err("The selection contains a duplicate submission.".into());
        }
        selected.push((submission_key(course, item, row.submission_id), row));
    }
    let request = teacher_grade_batch(&selected, TeacherGradingAction::Return);
    let response = session.write_rpc("WriteSubmission", request, Some(course)).await
        .map_err(|error| format!("{error} Check Student work before retrying the return."))?;
    Ok(confirmed_returns(response, &selected))
}

async fn published_teacher_item(session: &LiveSession, course: i64, item: i64) -> Result<crate::desktop_data::Coursework, String> {
    load_coursework(session, course).await?.items.into_iter()
        .find(|row| row.id.parse::<i64>().ok() == Some(item) && matches!(row.kind,
            crate::desktop_data::CourseworkKind::Assignment | crate::desktop_data::CourseworkKind::Question))
        .ok_or_else(|| "The assignment is no longer available as published work.".into())
}

fn same_grade_state(left: &TeacherSubmission, right: &TeacherSubmission) -> bool {
    (left.remote_state, left.status, left.draft_grade, left.released_grade)
        == (right.remote_state, right.status, right.draft_grade, right.released_grade)
}

fn submission_key(course: i64, item: i64, submission: i64) -> classwork::SubmissionId {
    classwork::SubmissionId {
        id: Some(submission),
        stream_item_id: Some(classwork::StreamItemId {
            id: Some(item), course_id: Some(classwork::CourseId { id: Some(course) }),
        }),
    }
}

fn teacher_grade_request(key: classwork::SubmissionId, row: &TeacherSubmission, action: TeacherGradingAction) -> mutation::WriteSubmissionRequest {
    teacher_grade_batch(&[(key, row)], action)
}

fn teacher_grade_batch(rows: &[(classwork::SubmissionId, &TeacherSubmission)], action: TeacherGradingAction) -> mutation::WriteSubmissionRequest {
    mutation::WriteSubmissionRequest {
        request_header: Some(mutation::WriteRequestHeader { operation: Some(3) }),
        submission_mutation: rows.iter().map(|(key, row)| teacher_grade_mutation(key.clone(), row, action)).collect(),
        submission_read_mask: Some(classwork::SubmissionReadMask {
            include_current_state: Some(true), include_student_user_id: Some(true),
            include_live_grade_numerator: Some(true), include_draft_grade_numerator: Some(true),
            ..Default::default()
        }),
    }
}

fn teacher_grade_mutation(key: classwork::SubmissionId, row: &TeacherSubmission, action: TeacherGradingAction) -> mutation::SubmissionMutation {
    let (state, released, draft) = match action {
        TeacherGradingAction::SaveDraft(value) => (None, None, Some(value)),
        TeacherGradingAction::Return => (Some(3), row.draft_grade, row.draft_grade),
    };
    mutation::SubmissionMutation {
            submission_id: Some(key),
            submission: Some(classwork::Submission {
                current_state: state, live_grade_numerator: released, draft_grade_numerator: draft,
                ..Default::default()
            }),
            submission_update_mask: Some(mutation::SubmissionUpdateMask {
                update_current_state: state.map(|_| true),
                update_live_grade_numerator: released.map(|_| true),
                update_draft_grade_numerator: draft.map(|_| true),
                ..Default::default()
            }),
            add_materials: Vec::new(),
    }
}

fn confirm_teacher_grade(
    response: mutation::WriteSubmissionResponse, key: &classwork::SubmissionId,
    row: &TeacherSubmission, action: TeacherGradingAction,
) -> Result<(), String> {
    let uncertain = || "Classroom did not confirm the submission change. Check Student work before retrying.".to_owned();
    let [updated] = response.submission.as_slice() else { return Err(uncertain()); };
    teacher_grade_matches(updated, key, row, action).then_some(()).ok_or_else(uncertain)
}

fn teacher_grade_matches(
    updated: &classwork::Submission, key: &classwork::SubmissionId, row: &TeacherSubmission, action: TeacherGradingAction,
) -> bool {
    if updated.submission_id.as_ref() != Some(key)
        || updated.student_user_id.as_ref().and_then(|id| id.id) != Some(row.student_id)
    {
        return false;
    }
    match action {
        TeacherGradingAction::SaveDraft(value) => updated.draft_grade_numerator == Some(value)
            && updated.live_grade_numerator == row.released_grade && updated.current_state == row.remote_state,
        TeacherGradingAction::Return => updated.current_state == Some(3)
            && updated.live_grade_numerator == row.draft_grade.or(row.released_grade),
    }
}

fn confirmed_returns(
    response: mutation::WriteSubmissionResponse, rows: &[(classwork::SubmissionId, &TeacherSubmission)],
) -> usize {
    let mut replies = BTreeMap::<i64, Option<&classwork::Submission>>::new();
    for reply in &response.submission {
        if let Some(id) = reply.submission_id.as_ref().and_then(|key| key.id) {
            replies.entry(id).and_modify(|entry| *entry = None).or_insert(Some(reply));
        }
    }
    rows.iter().filter(|(key, row)| replies.get(&row.submission_id).copied().flatten()
        .is_some_and(|reply| teacher_grade_matches(reply, key, row, TeacherGradingAction::Return))).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_thread_preflight_requires_the_exact_student_and_submission() {
        let target = submission_key(4, 7, 110);
        let request = feedback_target_request(&target);
        let filter = classwork::SubmissionFilter::decode(request.filter_options.unwrap().as_slice()).unwrap();
        assert_eq!(filter.submission_id, [target.clone()]);
        assert!(filter.course_id.is_empty() && filter.stream_item_id.is_empty());
        let row = classwork::Submission {
            submission_id: Some(target.clone()), student_user_id: Some(classwork::UserId { id: Some(10) }),
            ..Default::default()
        }.encode_to_vec();
        let response = wire::QuerySubmissionResponse { submission: vec![row.clone()] };
        assert!(has_exact_feedback_target(response.clone(), 10, &target).unwrap());
        assert!(!has_exact_feedback_target(response.clone(), 11, &target).unwrap());
        assert!(!has_exact_feedback_target(response, 10, &submission_key(5, 7, 110)).unwrap());
        assert!(!has_exact_feedback_target(wire::QuerySubmissionResponse { submission: vec![row.clone(), row] }, 10, &target).unwrap());
    }

    #[test]
    fn bulk_return_keeps_per_student_grades_and_counts_exact_unique_replies() {
        let row = |student_id, submission_id, draft_grade| TeacherSubmission {
            student_id, submission_id, remote_state: Some(2), name: None, status: SubmissionStatus::TurnedIn,
            released_grade: None, draft_grade, attachments: Vec::new(), answer: None,
        };
        let first = row(10, 110, Some(0.0));
        let second = row(11, 111, None);
        let selected = [(submission_key(4, 7, 110), &first), (submission_key(4, 7, 111), &second)];
        let request = teacher_grade_batch(&selected, TeacherGradingAction::Return);
        let decoded = mutation::WriteSubmissionRequest::decode(request.encode_to_vec().as_slice()).unwrap();
        assert_eq!(decoded.submission_mutation.len(), 2);
        let mut replies: Vec<_> = decoded.submission_mutation.into_iter().zip(selected.iter()).map(|(mutation, (_, row))| {
            let mut submission = mutation.submission.unwrap();
            submission.submission_id = mutation.submission_id;
            submission.student_user_id = Some(classwork::UserId { id: Some(row.student_id) });
            submission
        }).collect();
        assert_eq!(replies[0].live_grade_numerator, Some(0.0));
        assert!(replies[1].live_grade_numerator.is_none());
        replies.reverse();
        assert_eq!(confirmed_returns(mutation::WriteSubmissionResponse { submission: replies.clone() }, &selected), 2);
        assert_eq!(confirmed_returns(mutation::WriteSubmissionResponse { submission: vec![replies[0].clone()] }, &selected), 1);
        replies.push(replies[0].clone());
        replies[1].student_user_id = Some(classwork::UserId { id: Some(1111) });
        assert_eq!(confirmed_returns(mutation::WriteSubmissionResponse { submission: replies }, &selected), 0);
    }

    fn key(course: i64, item: i64) -> classwork::StreamItemId {
        classwork::StreamItemId { id: Some(item), course_id: Some(classwork::CourseId { id: Some(course) }) }
    }

    #[test]
    fn grade_save_and_return_use_separate_masks_and_confirm_exact_student() {
        let key = classwork::SubmissionId { id: Some(30), stream_item_id: Some(key(11, 22)) };
        let row = TeacherSubmission {
            student_id: 31, submission_id: 30, remote_state: Some(2), name: None, status: SubmissionStatus::TurnedIn,
            released_grade: None, draft_grade: Some(0.0), attachments: Vec::new(), answer: None,
        };
        let save = teacher_grade_request(key.clone(), &row, TeacherGradingAction::SaveDraft(9.5));
        let save = mutation::WriteSubmissionRequest::decode(save.encode_to_vec().as_slice()).unwrap();
        let mask = save.submission_mutation[0].submission_update_mask.as_ref().unwrap();
        assert_eq!((mask.update_current_state, mask.update_live_grade_numerator, mask.update_draft_grade_numerator),
            (None, None, Some(true)));
        let mut updated = save.submission_mutation[0].submission.clone().unwrap();
        updated.submission_id = Some(key.clone());
        updated.student_user_id = Some(classwork::UserId { id: Some(31) });
        updated.current_state = Some(2);
        let response = mutation::WriteSubmissionResponse { submission: vec![updated] };
        assert!(confirm_teacher_grade(response.clone(), &key, &row, TeacherGradingAction::SaveDraft(9.5)).is_ok());
        let mut wrong = row.clone();
        wrong.student_id = 32;
        assert!(confirm_teacher_grade(response, &key, &wrong, TeacherGradingAction::SaveDraft(9.5)).is_err());

        let returned = teacher_grade_request(key.clone(), &row, TeacherGradingAction::Return);
        let mask = returned.submission_mutation[0].submission_update_mask.as_ref().unwrap();
        assert_eq!((mask.update_current_state, mask.update_live_grade_numerator, mask.update_draft_grade_numerator),
            (Some(true), Some(true), Some(true)));
        let mut updated = returned.submission_mutation[0].submission.clone().unwrap();
        assert_eq!(updated.live_grade_numerator, Some(0.0));
        updated.submission_id = Some(key.clone());
        updated.student_user_id = Some(classwork::UserId { id: Some(31) });
        assert!(confirm_teacher_grade(mutation::WriteSubmissionResponse { submission: vec![updated] },
            &key, &row, TeacherGradingAction::Return).is_ok());

        let ungraded = teacher_grade_request(key, &TeacherSubmission { draft_grade: None, ..row }, TeacherGradingAction::Return);
        let mask = ungraded.submission_mutation[0].submission_update_mask.as_ref().unwrap();
        assert_eq!((mask.update_current_state, mask.update_live_grade_numerator, mask.update_draft_grade_numerator),
            (Some(true), None, None));
    }

    #[test]
    fn teacher_request_is_bound_to_one_assignment_with_teacher_grade_fields() {
        let parent = key(11, 22);
        let request = teacher_submissions_request(&parent);
        let filters = classwork::SubmissionFilter::decode(request.filter_options.unwrap().as_slice()).unwrap();
        assert_eq!(filters.stream_item_id, [parent]);
        assert!(filters.course_id.is_empty());
        assert!(filters.base_filter_options.is_none());
        let mask = classwork::SubmissionReadMask::decode(request.submission_read_mask.unwrap().as_slice()).unwrap();
        assert_eq!(mask.include_student_user_id, Some(true));
        assert_eq!(mask.include_draft_grade_numerator, Some(true));
    }

    #[test]
    fn gradebook_matrix_keeps_only_requested_work_for_enrolled_students() {
        let request = submission_query(vec![key(11, 22), key(11, 23)], false);
        let filter = classwork::SubmissionFilter::decode(request.filter_options.unwrap().as_slice()).unwrap();
        assert_eq!(filter.stream_item_id, [key(11, 22), key(11, 23)]);
        let mask = classwork::SubmissionReadMask::decode(request.submission_read_mask.unwrap().as_slice()).unwrap();
        assert!(mask.question_submission_mask.is_none() && mask.assignment_submission_mask.is_none());
        let submission = |student, course, assignment, grade| classwork::Submission {
            submission_id: Some(classwork::SubmissionId {
                id: Some(student + 100), stream_item_id: Some(key(course, assignment)),
            }),
            student_user_id: Some(classwork::UserId { id: Some(student) }),
            current_state: Some(3),
            live_grade_numerator: Some(grade), ..Default::default()
        }.encode_to_vec();
        let cells = gradebook_cells(wire::QuerySubmissionResponse { submission: vec![
            submission(1, 11, 22, 1.0), submission(1, 11, 23, 2.0),
            submission(2, 11, 22, 3.0), submission(2, 11, 22, 4.0),
            submission(3, 11, 22, 5.0), submission(1, 12, 22, 6.0), submission(1, 11, 24, 7.0),
        ] }, 11, &[22, 23], &BTreeSet::from([1, 2])).unwrap();
        assert_eq!(cells.keys().copied().collect::<Vec<_>>(), [(1, 22), (1, 23)]);
        assert_eq!(cells[&(1, 23)].released_grade, Some(2.0));
        let row = cells[&(1, 23)].grading_row(1);
        assert_eq!((row.student_id, row.submission_id, row.remote_state), (1, 101, Some(3)));
    }

    #[test]
    fn decode_picks_exact_parent_and_separates_draft_and_released_scores() {
        let submission = |student, course, item, score, draft| classwork::Submission {
            submission_id: Some(classwork::SubmissionId {
                id: Some(student + 100), stream_item_id: Some(key(course, item)),
            }),
            student_user_id: Some(classwork::UserId { id: Some(student) }),
            current_display_state_v2: Some(8),
            live_grade_numerator: Some(score),
            draft_grade_numerator: Some(draft),
            ..Default::default()
        }.encode_to_vec();
        let response = wire::QuerySubmissionResponse { submission: vec![
            submission(5, 11, 22, 0.0, 9.0),
            submission(6, 11, 23, 10.0, 10.0),
            submission(7, 12, 22, 10.0, 10.0),
            submission(8, 11, 22, 10.0, 10.0),
            submission(8, 11, 22, 10.0, 11.0),
        ] };
        let rows = decode_teacher_submissions(response, &key(11, 22)).unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.student_id, 5);
        assert_eq!(row.status, SubmissionStatus::Returned);
        assert_eq!((row.released_grade, row.draft_grade), (Some(0.0), Some(9.0)));
    }
}
