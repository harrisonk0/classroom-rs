use std::collections::BTreeSet;

use prost::Message;

use super::{
    LiveSession, classwork, course_member_key, current_user_id, drive_files, drive_url_reference,
    material, own_class_details, wire,
};
use crate::desktop_data::{Attachment, AttachmentKind, CourseRole, DriveSharing};

const UNCERTAIN: &str = "Classroom did not confirm that your work was prepared. Refresh the assignment before trying again.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersonalCopyPreparation {
    Available(Attachment),
    PreparedWithoutCopy,
    ExistingSubmissionWithoutCopy,
}

pub async fn check_personal_copy_absent(
    session: &LiveSession, course: i64, item: i64, template_reference: String,
) -> Result<bool, String> {
    if course <= 0 || item <= 0 || !drive_files::valid_drive_id(&template_reference) {
        return Err("Choose a personal-copy template on an active assignment.".into());
    }
    let key = classwork::StreamItemId {
        id: Some(item), course_id: Some(classwork::CourseId { id: Some(course) }),
    };
    let student = current_user_id(session).await?;
    tokio::try_join!(
        require_active_course(session, course),
        require_student_membership(session, course, student),
        load_template(session, course, &key, &template_reference),
    )?;
    Ok(read_own_submission(session, course, &key, student).await?.is_none())
}

pub async fn prepare_personal_copy(
    session: &LiveSession, course: i64, item: i64, template_reference: String,
) -> Result<PersonalCopyPreparation, String> {
    if course <= 0 || item <= 0 || !drive_files::valid_drive_id(&template_reference) {
        return Err("Choose a personal-copy template on an active assignment.".into());
    }
    let key = classwork::StreamItemId {
        id: Some(item), course_id: Some(classwork::CourseId { id: Some(course) }),
    };
    let student = current_user_id(session).await?;
    let ((), (), template) = tokio::try_join!(
        require_active_course(session, course),
        require_student_membership(session, course, student),
        load_template(session, course, &key, &template_reference),
    )?;

    let current = read_own_submission(session, course, &key, student).await?;
    if let Some(existing) = current {
        return match confirmed_copy(existing, &template)? {
            Some(copy) => Ok(PersonalCopyPreparation::Available(copy)),
            None => Ok(PersonalCopyPreparation::ExistingSubmissionWithoutCopy),
        };
    }

    // The mutation is separately invoked after a confirmed absent submission.
    // The write transport will not retry an ambiguous or unauthenticated request.
    let reply: PersonalCopyQueryResponse = session.write_rpc(
        "QuerySubmission", submission_request(&key, student, Some(true)), Some(course),
    ).await.map_err(|error| format!("{error} Refresh the assignment before trying again."))?;
    let proposed = select_submission(reply, &key, student).map_err(|_| UNCERTAIN)?;
    let confirmed = read_own_submission(session, course, &key, student).await
        .map_err(|_| UNCERTAIN)?.ok_or(UNCERTAIN)?;
    if proposed.as_ref().is_some_and(|entry| entry.submission_id != confirmed.submission_id) {
        return Err(UNCERTAIN.into());
    }
    match confirmed_copy(confirmed, &template)? {
        Some(copy) => Ok(PersonalCopyPreparation::Available(copy)),
        None => Ok(PersonalCopyPreparation::PreparedWithoutCopy),
    }
}

async fn require_active_course(session: &LiveSession, course: i64) -> Result<(), String> {
    let request = wire::QueryCourseRequest {
        course_read_mask: Some(wire::CourseReadMask { include_course_state: Some(true), ..Default::default() }),
        filter_options: Some(wire::CourseFilterOptions {
            base_filter_options: Some(wire::FilterOptions {
                user_filter: vec![wire::UserFilter { use_current_user: Some(true) }],
            }),
            course_id: vec![wire::CourseId { id: Some(course) }],
            ..Default::default()
        }),
    };
    let result: wire::QueryCourseResponse = session.rpc("QueryCourse", request, Some(course)).await?;
    match result.course.as_slice() {
        [row] if row.course_id.as_ref().and_then(|id| id.id) == Some(course) && row.course_state == Some(1) => Ok(()),
        _ => Err("Classroom did not confirm that this class is active.".into()),
    }
}

async fn require_student_membership(session: &LiveSession, course: i64, student: i64) -> Result<(), String> {
    let request = classwork::CourseUserLookupRequest {
        course_user_ids: vec![course_member_key(course, student)],
        read_mask: Some(classwork::CourseUserReadMask { include_course_role: Some(true), ..Default::default() }),
    };
    let response: classwork::CourseUserLookupResponse = session.read_rpc("LookupCourseUser", request).await?;
    let details = own_class_details(response, &BTreeSet::from([course]), student);
    if details.roles.get(&course) == Some(&CourseRole::Student) {
        Ok(())
    } else {
        Err("Classroom did not confirm your student membership in this class.".into())
    }
}

async fn load_template(
    session: &LiveSession, course: i64, key: &classwork::StreamItemId, reference: &str,
) -> Result<Attachment, String> {
    let request = classwork::StreamItemLookupRequest {
        stream_item_id: vec![key.clone()],
        read_mask: Some(classwork::StreamReadMask {
            assignment_mask: Some(classwork::AssignmentReadMask {
                stream_item_info_mask: Some(classwork::StreamItemInfoReadMask {
                    material_mask: Some(classwork::MaterialReadMask {
                        drive_item_mask: Some(classwork::DriveItemReadMask { include_sharing_option: Some(true) }),
                        ..material_mask()
                    }),
                    publication_info_mask: Some(classwork::PublicationInfoReadMask {
                        include_publication_status: Some(true), ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
    };
    let reply: classwork::StreamItemLookupResponse = session.rpc("LookupStreamItem", request, Some(course)).await?;
    let error = "Classroom did not confirm the selected personal-copy template on this published assignment.";
    let [row] = reply.result.as_slice() else { return Err(error.into()); };
    row.stream_item.as_ref().and_then(|item| assignment_template(item, key, reference)).ok_or_else(|| error.into())
}

fn assignment_template(item: &classwork::StreamItem, key: &classwork::StreamItemId, reference: &str) -> Option<Attachment> {
    let info = match item {
        classwork::StreamItem {
            stream_type: Some(2), assignment: Some(assignment), post: None, question: None, supplement: None,
        } => assignment.stream_item_info.as_ref()?,
        _ => return None,
    };
    if info.stream_item_id.as_ref() != Some(key) || info.status != Some(2)
        || info.publication_info.as_ref()?.status != Some(2)
    {
        return None;
    }
    let mut matches = info.material.iter().cloned().filter_map(material).filter(|attachment| {
        attachment.kind == AttachmentKind::DriveFile
            && attachment.drive_sharing == Some(DriveSharing::IndividualCopy)
            && attachment.drive_reference.as_deref() == Some(reference)
    });
    let selected = matches.next()?;
    matches.next().is_none().then_some(selected)
}

fn material_mask() -> classwork::MaterialReadMask {
    classwork::MaterialReadMask {
        include_name: Some(true),
        include_mime_type: Some(true),
        include_open_url: Some(true),
        include_parent_reference: Some(true),
        ..Default::default()
    }
}

async fn read_own_submission(
    session: &LiveSession, course: i64, key: &classwork::StreamItemId, student: i64,
) -> Result<Option<classwork::Submission>, String> {
    let reply: PersonalCopyQueryResponse = session.rpc(
        "QuerySubmission", submission_request(key, student, None), Some(course),
    ).await?;
    select_submission(reply, key, student)
}

fn submission_request(
    key: &classwork::StreamItemId, student: i64, create_if_needed: Option<bool>,
) -> PersonalCopyQueryRequest {
    PersonalCopyQueryRequest {
        read_mask: Some(classwork::SubmissionReadMask {
            include_student_user_id: Some(true),
            assignment_submission_mask: Some(classwork::AssignmentSubmissionReadMask {
                material_mask: Some(material_mask()),
            }),
            ..Default::default()
        }),
        filter_options: Some(classwork::SubmissionFilter {
            base_filter_options: Some(classwork::FilterOptions {
                user_filter: vec![classwork::UserFilter {
                    use_current_user: None, user_id: Some(classwork::UserId { id: Some(student) }),
                }],
            }),
            stream_item_id: vec![key.clone()],
            ..Default::default()
        }),
        create_if_needed,
    }
}

fn select_submission(
    reply: PersonalCopyQueryResponse, key: &classwork::StreamItemId, student: i64,
) -> Result<Option<classwork::Submission>, String> {
    let mut rows = reply.submission.into_iter();
    let Some(submission) = rows.next() else { return Ok(None); };
    let is_own = submission.student_user_id.as_ref().and_then(|id| id.id) == Some(student);
    let is_exact = submission.submission_id.as_ref().is_some_and(|id| {
        id.id.is_some_and(|id| id > 0) && id.stream_item_id.as_ref() == Some(key)
    });
    if rows.next().is_none() && is_own && is_exact && submission.question_submission.is_none() {
        Ok(Some(submission))
    } else {
        Err("Classroom did not return a unique submission for this assignment and account.".into())
    }
}

fn confirmed_copy(submission: classwork::Submission, template: &Attachment) -> Result<Option<Attachment>, String> {
    let mut matches = submission.assignment_submission.into_iter()
        .flat_map(|assignment| assignment.material)
        .filter(|material| material.parent_reference.as_deref() == template.drive_reference.as_deref());
    let Some(copy) = matches.next() else { return Ok(None); };
    let uncertain = "Classroom returned a possible personal copy but could not confirm its Drive link.";
    if matches.next().is_some() {
        return Err(uncertain.into());
    }
    let copy = material(copy).filter(|copy| {
        copy.is_personal_copy_of(template)
            && copy.url.as_deref().is_none_or(|url|
                drive_url_reference(url).as_deref() == copy.drive_reference.as_deref())
    }).ok_or(uncertain)?;
    Ok(Some(copy))
}

// Android's assignment-details call at 0x194795c reaches the scoped request
// builder at 0x1940030. The user and stream-item filters are added before
// 0x194022c sets createIfNeeded. The builder at 0x197eab8–0x197eac4 registers
// this boolean at tag 4. Only the dedicated write sends it.
#[derive(Clone, PartialEq, Message)]
struct PersonalCopyQueryRequest {
    #[prost(message, optional, tag = "2")]
    read_mask: Option<classwork::SubmissionReadMask>,
    #[prost(message, optional, tag = "3")]
    filter_options: Option<classwork::SubmissionFilter>,
    #[prost(bool, optional, tag = "4")]
    create_if_needed: Option<bool>,
}

#[derive(Clone, PartialEq, Message)]
struct PersonalCopyQueryResponse {
    #[prost(message, repeated, tag = "2")]
    submission: Vec<classwork::Submission>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> classwork::StreamItemId {
        classwork::StreamItemId {
            id: Some(9), course_id: Some(classwork::CourseId { id: Some(7) }),
        }
    }

    fn owned_submission() -> classwork::Submission {
        classwork::Submission {
            submission_id: Some(classwork::SubmissionId { id: Some(100), stream_item_id: Some(key()) }),
            student_user_id: Some(classwork::UserId { id: Some(42) }),
            ..Default::default()
        }
    }

    #[test]
    fn personal_copy_request_can_only_initialize_one_students_exact_item() {
        let read = submission_request(&key(), 42, None);
        let write = submission_request(&key(), 42, Some(true));
        assert!(!read.encode_to_vec().ends_with(&[0x20, 0x01]));
        assert_eq!(write.encode_to_vec(), [read.encode_to_vec(), vec![0x20, 0x01]].concat());
        let filter = write.filter_options.unwrap();
        assert_eq!(filter.stream_item_id, vec![key()]);
        assert!(filter.course_id.is_empty() && filter.submission_id.is_empty());
        assert_eq!(filter.base_filter_options.unwrap().user_filter, vec![classwork::UserFilter {
            use_current_user: None, user_id: Some(classwork::UserId { id: Some(42) }),
        }]);
    }

    #[test]
    fn personal_copy_submissions_distinguish_confirmed_absence_and_invalid_responses() {
        let row = owned_submission();
        let reply = |submission| PersonalCopyQueryResponse { submission };
        assert_eq!(select_submission(reply(vec![]), &key(), 42), Ok(None));
        assert_eq!(select_submission(reply(vec![row.clone()]), &key(), 42), Ok(Some(row.clone())));
        assert!(select_submission(reply(vec![row.clone()]), &key(), 43).is_err());
        assert!(select_submission(reply(vec![row.clone(), row]), &key(), 42).is_err());
    }

    #[test]
    fn personal_copy_requires_a_unique_published_individual_template_and_distinct_file_link() {
        let original = "TemplateAbc123";
        let copy_id = "MyCopyAbc123";
        let original_material = classwork::Material {
            name: Some("Worksheet".into()),
            reference: Some(original.into()),
            reference_type: Some(2),
            drive_item: Some(classwork::DriveItem { sharing_option: Some(1) }),
            ..Default::default()
        };
        let published_item = |materials| classwork::StreamItem {
            stream_type: Some(2),
            assignment: Some(classwork::Assignment {
                stream_item_info: Some(classwork::StreamItemInfo {
                    stream_item_id: Some(key()),
                    status: Some(2),
                    material: materials,
                    publication_info: Some(classwork::PublicationInfo { status: Some(2), ..Default::default() }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let with_material = published_item(vec![original_material.clone()]);
        let template = assignment_template(&with_material, &key(), original).unwrap();
        assert!(assignment_template(
            &published_item(vec![original_material.clone(), original_material]), &key(), original,
        ).is_none());
        let mut submission = owned_submission();
        assert_eq!(confirmed_copy(submission.clone(), &template), Ok(None));
        let mut file = classwork::Material {
            reference: Some(copy_id.into()),
            reference_type: Some(2),
            parent_reference: Some(original.into()),
            open_url: Some(format!("https://docs.google.com/document/d/{copy_id}/edit")),
            ..Default::default()
        };
        submission.assignment_submission = Some(classwork::AssignmentSubmission { material: vec![file.clone()] });
        let copy = confirmed_copy(submission.clone(), &template).unwrap().unwrap();
        assert_eq!(copy.drive_reference.as_deref(), Some(copy_id));
        file.open_url = None;
        submission.assignment_submission = Some(classwork::AssignmentSubmission { material: vec![file.clone()] });
        assert_eq!(confirmed_copy(submission.clone(), &template).unwrap().unwrap().drive_reference.as_deref(), Some(copy_id));
        file.open_url = Some(format!("https://docs.google.com/document/d/{original}/edit"));
        submission.assignment_submission = Some(classwork::AssignmentSubmission { material: vec![file] });
        assert!(confirmed_copy(submission, &template).is_err());
    }
}
