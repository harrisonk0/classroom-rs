use super::{
    classwork, mutation, check_class, LiveSession, load_submissions, load_stream_permission, load_user_names,
};

pub async fn load_comments(session: &LiveSession, course_id: i64, stream_item_id: i64) -> Result<Vec<crate::desktop_data::Comment>, String> {
    use classwork::*;
    check_class(course_id)?;
    if stream_item_id <= 0 { return Err("Choose a published class item first.".into()) }
    let response: QueryCommentResponse = session.read_rpc("QueryComment", QueryCommentRequest {
        comment_read_mask: Some(CommentReadMask {
            include_comment: Some(true),
            include_creator_user_id: Some(true),
            ..Default::default()
        }),
        filter_options: Some(CommentFilterOptions {
            stream_item_id: vec![StreamItemId {
                id: Some(stream_item_id),
                course_id: Some(CourseId { id: Some(course_id) }),
            }],
            ..Default::default()
        }),
    }).await?;
    let received = response.comment.len();
    let entries = response.comment.into_iter().filter_map(|comment| {
        let key = comment.comment_id?;
        if comment.status != Some(2) || key.submission_id.is_some()
            || comment.visibility.is_some_and(|visibility| visibility.visibility_type == Some(1)) { return None }
        let parent = key.stream_item_id?;
        if parent.id != Some(stream_item_id) || parent.course_id?.id != Some(course_id) { return None }
        Some((key.id?, comment.creator_user_id.and_then(|user| user.id), comment.comment?))
    }).collect::<Vec<_>>();
    if received != 0 && entries.is_empty() {
        return Err("None of the returned comments could be verified as active comments on this item.".into());
    }
    // Failure to resolve a display name never authorizes an additional fallback query.
    let authors = load_user_names(session, entries.iter().filter_map(|(_, id, _)| *id).take(500).collect()).await
        .unwrap_or_default();
    Ok(entries.into_iter().map(|(id, author, body)| crate::desktop_data::Comment {
        id: id.to_string(),
        author_id: author,
        author: author.and_then(|id| authors.get(&id).cloned()).unwrap_or_else(|| "Class member".into()),
        body,
        created_label: None,
    }).collect())
}

pub async fn load_private_feedback(
    session: &LiveSession, course_id: i64, item_id: i64,
) -> Result<Vec<crate::desktop_data::Comment>, String> {
    use classwork::*;
    check_class(course_id)?;
    let (_, mine) = load_submissions(session, course_id).await?.into_iter()
        .find(|(id, _)| id == &item_id.to_string())
        .ok_or("Classroom did not return your submission for this assignment.")?;
    let target = SubmissionId {
        id: Some(mine.remote_id.ok_or("The submission could not be identified.")?),
        stream_item_id: Some(StreamItemId {
            id: Some(item_id),
            course_id: Some(CourseId { id: Some(course_id) }),
        }),
    };
    query_submission_feedback(session, course_id, target).await
}

pub(super) async fn query_submission_feedback(
    session: &LiveSession, course_id: i64, target: classwork::SubmissionId,
) -> Result<Vec<crate::desktop_data::Comment>, String> {
    use classwork::*;
    let result: QueryCommentResponse = session.rpc(
        "QueryComment", QueryCommentRequest {
            comment_read_mask: Some(CommentReadMask {
                include_comment: Some(true),
                include_creator_user_id: Some(true),
                visibility_mask: Some(CommentVisibilityReadMask { include_visibility_type: Some(true) }),
                ..Default::default()
            }),
            filter_options: Some(CommentFilterOptions {
                submission_id: vec![target.clone()],
                visibility: vec![CommentVisibility { visibility_type: Some(1) }],
                ..Default::default()
            }),
        }, Some(course_id)
    ).await?;
    let count = result.comment.len();
    let entries = result.comment.into_iter()
        .filter_map(|entry| decode_private_feedback(entry, &target)).collect::<Vec<_>>();
    if count > 0 && entries.is_empty() {
        return Err("Classroom returned feedback without a matching private submission identity.".into());
    }
    let ids = entries.iter().filter_map(|(_, id, _)| *id).take(500).collect();
    let authors = load_user_names(session, ids).await.unwrap_or_default();
    Ok(entries.into_iter().map(|(id, author, body)| crate::desktop_data::Comment {
        id: id.to_string(),
        author_id: author,
        author: author.and_then(|id| authors.get(&id)).cloned().unwrap_or_else(|| "Class member".into()),
        body,
        created_label: None,
    }).collect())
}

pub(super) fn decode_private_feedback(
    entry: classwork::Comment, target: &classwork::SubmissionId,
) -> Option<(i64, Option<i64>, String)> {
    let id = entry.comment_id?;
    if entry.status != Some(2) || entry.visibility?.visibility_type != Some(1)
        || id.submission_id.as_ref() != Some(target)
    {
        return None;
    }
    Some((id.id?, entry.creator_user_id.and_then(|author| author.id), entry.comment?))
}

/// Send a single class-visible comment after the user explicitly clicks Post.
pub async fn post_class_comment(
    session: &LiveSession, course_id: i64, item_id: i64, body: String,
) -> Result<(), String> {
    check_class(course_id)?;
    if item_id <= 0 { return Err("Choose a published class item.".into()); }
    let (_, access) = load_stream_permission(session, course_id).await?;
    if !access.can_comment() {
        return Err("Classroom has disabled class comments for this account in this class.".into());
    }
    insert_comment(session, course_id, classwork::CommentId {
        stream_item_id: Some(classwork::StreamItemId {
            id: Some(item_id), course_id: Some(classwork::CourseId { id: Some(course_id) }),
        }),
        parent_type: Some(2),
        ..Default::default()
    }, 2, &body).await
}

pub async fn post_private_feedback(
    session: &LiveSession, course_id: i64, item_id: i64, body: String,
) -> Result<(), String> {
    check_class(course_id)?;
    let (_, mine) = load_submissions(session, course_id).await?.into_iter()
        .find(|(id, _)| id == &item_id.to_string())
        .ok_or("Classroom did not return your submission for this assignment.")?;
    let own_id = mine.remote_id.ok_or("The submission could not be identified.")?;
    insert_comment(session, course_id, classwork::CommentId {
        submission_id: Some(classwork::SubmissionId {
            id: Some(own_id),
            stream_item_id: Some(classwork::StreamItemId {
                id: Some(item_id), course_id: Some(classwork::CourseId { id: Some(course_id) }),
            }),
        }),
        parent_type: Some(3),
        ..Default::default()
    }, 1, &body).await
}

pub(super) async fn insert_comment(
    session: &LiveSession, course_id: i64, target: classwork::CommentId, visibility: i32, body: &str,
) -> Result<(), String> {
    let body = body.trim();
    if body.is_empty() || body.len() > 10_000 {
        return Err("Enter a comment of up to 10,000 bytes.".into());
    }
    let request = mutation::WriteCommentRequest {
        request_header: Some(mutation::WriteRequestHeader { operation: Some(2) }),
        comment_mutation: vec![mutation::CommentMutation {
            comment_id: None,
            comment: Some(classwork::Comment {
                comment_id: Some(target.clone()),
                comment: Some(body.to_owned()),
                visibility: Some(classwork::CommentVisibility { visibility_type: Some(visibility) }),
                ..Default::default()
            }),
            comment_update_mask: None,
        }],
        comment_read_mask: Some(classwork::CommentReadMask {
            include_comment: Some(true),
            visibility_mask: Some(classwork::CommentVisibilityReadMask { include_visibility_type: Some(true) }),
            ..Default::default()
        }),
    };
    let response: mutation::WriteCommentResponse = session.write_rpc(
        "WriteComment", request, Some(course_id)
    ).await.map_err(|error| format!("{error} Refresh comments before posting again."))?;
    if response.comment.iter().any(|comment| {
        comment.comment_id.as_ref().is_some_and(|id| {
            id.id.is_some_and(|id| id > 0)
                && if visibility == 1 {
                    id.submission_id == target.submission_id
                        && comment.visibility.as_ref().and_then(|value| value.visibility_type) == Some(1)
                } else {
                    id.submission_id.is_none() && id.stream_item_id == target.stream_item_id
                        && comment.visibility.as_ref().and_then(|value| value.visibility_type) != Some(1)
                }
        })
    }) {
        Ok(())
    } else {
        Err("Google returned no confirmation. Refresh comments before posting again.".into())
    }
}
