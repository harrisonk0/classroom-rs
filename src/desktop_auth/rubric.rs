use super::{LiveSession, check_class, classwork};
use crate::desktop_data::{CriterionScore, Rubric, RubricCriterion, RubricRating, SubmissionStatus};
use std::collections::BTreeMap;

pub async fn load_rubric(session: &LiveSession, course_id: i64, item_id: i64) -> Result<Option<Rubric>, String> {
    check_class(course_id)?;
    if item_id <= 0 {
        return Err("Choose a published assignment first.".into());
    }
    let parent = classwork::RubricParentId {
        assignment_id: Some(classwork::RubricAssignmentId { course_id: Some(course_id), id: Some(item_id) }),
    };
    let response: classwork::ListRubricsResponse = session.rpc("ListRubrics", classwork::ListRubricsRequest {
        filter_options: Some(classwork::RubricFilter { parent_id: Some(parent.clone()) }),
    }, Some(course_id)).await?;
    decode_rubric(response, &parent)
}

fn decode_rubric(response: classwork::ListRubricsResponse, parent: &classwork::RubricParentId)
    -> Result<Option<Rubric>, String>
{
    if response.rubric.is_empty() {
        return Ok(None);
    }
    let mut matching = response.rubric.into_iter().filter(|item| item.parent_id.as_ref() == Some(parent));
    let selected = matching.next().ok_or("Classroom did not return a rubric for this assignment.")?;
    if matching.next().is_some() {
        return Err("Classroom returned several rubrics for this assignment.".into());
    }
    Ok(Some(Rubric {
        title: selected.title.filter(|title| !title.trim().is_empty()),
        criteria: selected.criterion.into_iter().map(|criterion| RubricCriterion {
            id: criterion.id,
            title: criterion.title.filter(|title| !title.trim().is_empty()).unwrap_or_else(|| "Criterion".into()),
            description: criterion.description.unwrap_or_default(),
            ratings: criterion.rating.into_iter().map(|rating| RubricRating {
                id: rating.id,
                title: rating.title.unwrap_or_default(),
                description: rating.description.unwrap_or_default(),
                points: rating.points.filter(|points| points.is_finite() && *points >= 0.0),
            }).collect(),
        }).collect(),
    }))
}

pub(super) fn released_scores(
    status: SubmissionStatus, score: Option<classwork::RubricScore>,
) -> BTreeMap<String, CriterionScore> {
    if !matches!(status, SubmissionStatus::Returned | SubmissionStatus::ReturnedLate) {
        return BTreeMap::new();
    }
    score.into_iter().flat_map(|score| score.criterion_score).filter(|(id, _)| !id.is_empty())
        .filter_map(|(id, score)| {
            let rating_id = score.rating_id.filter(|id| !id.is_empty());
            let points = score.points.filter(|points| points.is_finite() && *points >= 0.0);
            (rating_id.is_some() || points.is_some()).then_some((id, CriterionScore { rating_id, points }))
        }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn rubric_matches_the_assignment_and_keeps_rating_order() {
        let parent = classwork::RubricParentId {
            assignment_id: Some(classwork::RubricAssignmentId { course_id: Some(7), id: Some(8) }),
        };
        let rubric = classwork::Rubric {
            parent_id: Some(parent.clone()),
            title: Some("Essay".into()),
            criterion: vec![classwork::RubricCriterion {
                id: Some("structure".into()), title: Some("Structure".into()), description: Some("Show an argument".into()),
                rating: vec![
                    classwork::RubricRating { id: Some("detailed".into()), title: Some("Detailed".into()),
                        description: Some("Supports each claim".into()), points: Some(5.0) },
                    classwork::RubricRating { id: Some("basic".into()), title: Some("Basic".into()),
                        description: None, points: Some(0.0) },
                ],
            }],
        };
        let response = classwork::ListRubricsResponse { rubric: vec![rubric.clone()] };
        let decoded = classwork::ListRubricsResponse::decode(response.encode_to_vec().as_slice()).unwrap();
        let result = decode_rubric(decoded, &parent).unwrap().unwrap();
        assert_eq!(result.criteria[0].ratings[0].title, "Detailed");
        assert_eq!(result.criteria[0].ratings[1].points, Some(0.0));
        assert!(decode_rubric(response, &classwork::RubricParentId::default()).is_err());
        assert!(decode_rubric(classwork::ListRubricsResponse { rubric: vec![rubric.clone(), rubric] }, &parent).is_err());
        assert_eq!(decode_rubric(classwork::ListRubricsResponse::default(), &parent), Ok(None));
    }

    #[test]
    fn criterion_scores_show_only_on_returned_own_submissions() {
        let source = classwork::RubricScore { criterion_score: [
            ("structure".into(), classwork::CriterionScore { rating_id: Some("basic".into()), points: Some(0.0) }),
            ("grammar".into(), classwork::CriterionScore { rating_id: None, points: Some(f64::NAN) }),
        ].into_iter().collect() };
        for status in [SubmissionStatus::Unknown, SubmissionStatus::Assigned, SubmissionStatus::TurnedIn] {
            assert!(released_scores(status, Some(source.clone())).is_empty());
        }
        let returned = released_scores(SubmissionStatus::ReturnedLate, Some(source));
        assert_eq!(returned.len(), 1);
        assert_eq!(returned["structure"].rating_id.as_deref(), Some("basic"));
        assert_eq!(returned["structure"].points, Some(0.0));
    }
}
