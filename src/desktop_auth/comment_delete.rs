use super::{LiveSession, check_class, classwork, current_user_id, load_submissions, mutation, require_class_moderator};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentAudience {
    Class,
    Private,
}

fn comment_key(course: i64, item: i64, comment: i64, submission: Option<i64>) -> classwork::CommentId {
    let stream = classwork::StreamItemId { id: Some(item), course_id: Some(classwork::CourseId { id: Some(course) }) };
    if let Some(submission) = submission {
        classwork::CommentId { id: Some(comment), parent_type: Some(3), stream_item_id: None,
            submission_id: Some(classwork::SubmissionId { id: Some(submission), stream_item_id: Some(stream) }) }
    } else {
        classwork::CommentId { id: Some(comment), parent_type: Some(2), stream_item_id: Some(stream), submission_id: None }
    }
}

fn matches_thread(target: &classwork::CommentId, comment: &classwork::Comment, audience: CommentAudience) -> bool {
    let Some(id) = &comment.comment_id else { return false };
    let visibility = comment.visibility.as_ref().and_then(|value| value.visibility_type);
    id.id == target.id && match audience {
        CommentAudience::Class => id.submission_id.is_none() && id.stream_item_id == target.stream_item_id && visibility == Some(2),
        CommentAudience::Private => id.submission_id == target.submission_id && visibility == Some(1),
    }
}

fn comment_identity_mask() -> classwork::CommentReadMask {
    classwork::CommentReadMask {
        include_creator_user_id: Some(true),
        visibility_mask: Some(classwork::CommentVisibilityReadMask { include_visibility_type: Some(true) }),
        ..Default::default()
    }
}

async fn read_target(
    session: &LiveSession, course: i64, target: &classwork::CommentId, audience: CommentAudience,
) -> Result<Option<classwork::Comment>, String> {
    let response: classwork::QueryCommentResponse = session.rpc(
        "QueryComment", classwork::QueryCommentRequest {
            comment_read_mask: Some(comment_identity_mask()),
            filter_options: Some(classwork::CommentFilterOptions {
                comment_id: vec![target.clone()],
                visibility: vec![classwork::CommentVisibility {
                    visibility_type: Some(if audience == CommentAudience::Class { 2 } else { 1 }),
                }],
                ..Default::default()
            }),
        }, Some(course),
    ).await?;
    let mut comments = response.comment.into_iter();
    let Some(comment) = comments.next() else { return Ok(None) };
    if comments.next().is_some() || !matches_thread(target, &comment, audience) {
        return Err("Classroom returned a different comment or thread.".into());
    }
    Ok(Some(comment))
}

pub async fn check_class_comment_target(session: &LiveSession, course: i64, item: i64, comment: i64) -> Result<bool, String> {
    Ok(read_target(session, course, &comment_key(course, item, comment, None), CommentAudience::Class).await?
        .is_some_and(|comment| comment.status == Some(2) && comment.creator_user_id.and_then(|author| author.id).is_some()))
}

fn delete_comment_request(target: classwork::CommentId) -> mutation::WriteCommentRequest {
    mutation::WriteCommentRequest {
        request_header: Some(mutation::WriteRequestHeader { operation: Some(4) }),
        comment_mutation: vec![mutation::CommentMutation {
            comment_id: Some(target),
            ..Default::default()
        }],
        comment_read_mask: Some(comment_identity_mask()),
        ..Default::default()
    }
}

pub async fn delete_own_comment(
    session: &LiveSession, course: i64, item: i64, comment: i64, audience: CommentAudience,
) -> Result<(), String> {
    check_class(course)?;
    if item <= 0 || comment <= 0 { return Err("Choose an existing comment.".into()); }
    let submission = if audience == CommentAudience::Private {
        Some(load_submissions(session, course).await?.into_iter().find(|(id, _)| id.parse() == Ok(item))
            .and_then(|(_, submission)| submission.remote_id).ok_or("Classroom did not return your submission.")?)
    } else {
        None
    };
    let own_user = current_user_id(session).await?;
    delete_verified_comment(session, course, comment_key(course, item, comment, submission), audience, own_user).await
}

pub async fn moderate_class_comment(
    session: &LiveSession, course: i64, item: i64, comment: i64, author: i64,
) -> Result<(), String> {
    if item <= 0 || comment <= 0 || author <= 0 { return Err("Choose an existing class comment.".into()); }
    require_class_moderator(session, course).await?;
    delete_verified_comment(session, course, comment_key(course, item, comment, None), CommentAudience::Class, author).await
}

async fn delete_verified_comment(
    session: &LiveSession, course: i64, target: classwork::CommentId, audience: CommentAudience, author: i64,
) -> Result<(), String> {
    let target = read_target(session, course, &target, audience).await?
        .filter(|entry| active_comment_author(entry, author))
        .and_then(|entry| entry.comment_id).ok_or("Classroom did not confirm the active comment and its author.")?;
    let _: mutation::WriteCommentResponse = session.write_rpc(
        "WriteComment", delete_comment_request(target.clone()), Some(course),
    ).await?;
    if read_target(session, course, &target, audience).await?.is_some_and(|entry| entry.status != Some(3)) {
        return Err("Classroom still shows this comment.".into());
    }
    Ok(())
}

fn active_comment_author(comment: &classwork::Comment, author: i64) -> bool {
    author > 0 && comment.status == Some(2) && comment.creator_user_id.as_ref().and_then(|id| id.id) == Some(author)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn delete_encodes_only_an_existing_comment_identifier() {
        let target = comment_key(1, 2, 3, Some(4));
        let request = delete_comment_request(target.clone());
        let decoded = mutation::WriteCommentRequest::decode(request.encode_to_vec().as_slice()).unwrap();
        assert_eq!(decoded.request_header.unwrap().operation, Some(4));
        let [entry] = decoded.comment_mutation.as_slice() else { panic!("Expected one mutation") };
        assert_eq!(entry.comment_id.as_ref(), Some(&target));
        assert!(entry.comment.is_none() && entry.comment_update_mask.is_none());
    }

    #[test]
    fn matching_private_identity_rejects_other_submissions_and_public_threads() {
        let target = comment_key(1, 2, 3, Some(4));
        let mut comment = classwork::Comment {
            comment_id: Some(target.clone()),
            visibility: Some(classwork::CommentVisibility { visibility_type: Some(1) }),
            ..Default::default()
        };
        assert!(matches_thread(&target, &comment, CommentAudience::Private));
        assert!(!matches_thread(&target, &comment, CommentAudience::Class));
        comment.comment_id = Some(comment_key(1, 2, 3, Some(5)));
        assert!(!matches_thread(&target, &comment, CommentAudience::Private));
    }

    #[test]
    fn class_moderation_preflight_requires_an_active_comment_by_the_displayed_author() {
        let key = comment_key(1, 12, 18, None);
        let mut comment = classwork::Comment {
            comment_id: Some(key.clone()), status: Some(2),
            visibility: Some(classwork::CommentVisibility { visibility_type: Some(2) }),
            creator_user_id: Some(classwork::UserId { id: Some(45) }), ..Default::default()
        };
        assert!(matches_thread(&key, &comment, CommentAudience::Class) && active_comment_author(&comment, 45));
        assert!(!active_comment_author(&comment, 46));
        comment.status = Some(3);
        assert!(!active_comment_author(&comment, 45));
    }
}
