use super::{LiveSession, check_class, classwork, current_user_id};
use crate::desktop_data::{CourseGrade, GradeCalculation};
use std::collections::{BTreeMap, BTreeSet};

async fn load_gradebook_settings(
    session: &LiveSession, courses: &[i64],
) -> Result<BTreeMap<i64, classwork::GradebookSettings>, String> {
    let ids: BTreeSet<_> = courses.iter().copied().collect();
    let response: classwork::GradebookSettingsLookupResponse = session.rpc(
        "GradebookSettingsLookup",
        classwork::GradebookSettingsLookupRequest {
            read_mask: Some(classwork::GradebookSettingsReadMask {
                include_grade_display_setting: Some(true),
                include_grade_calculation_type: Some(true),
            }),
            course_ids: ids.iter().map(|id| classwork::CourseId { id: Some(*id) }).collect(),
        },
        if courses.len() == 1 { courses.first().copied() } else { None },
    ).await?;
    select_settings(response, &ids)
}

fn select_settings(
    response: classwork::GradebookSettingsLookupResponse, courses: &BTreeSet<i64>,
) -> Result<BTreeMap<i64, classwork::GradebookSettings>, String> {
    let mut settings = BTreeMap::new();
    for row in response.results {
        let Some(course_id) = row.course_id.as_ref().and_then(|id| id.id) else { continue };
        let Some(value) = row.gradebook_settings else { continue };
        if !courses.contains(&course_id) || value.course_id.as_ref() != row.course_id.as_ref() {
            continue;
        }
        if settings.insert(course_id, value).is_some() {
            return Err("Classroom returned conflicting course grading settings.".into());
        }
    }
    Ok(settings)
}

fn published_calculation(settings: &classwork::GradebookSettings) -> Option<GradeCalculation> {
    if settings.grade_display_setting != Some(1) { return None; }
    match settings.grade_calculation_type? {
        1 => Some(GradeCalculation::TotalPoints),
        2 => Some(GradeCalculation::WeightedCategories),
        _ => None,
    }
}

pub async fn load_course_grade(session: &LiveSession, course_id: i64) -> Result<Option<CourseGrade>, String> {
    check_class(course_id)?;
    let settings = load_gradebook_settings(session, &[course_id]).await?;
    let Some(calculation) = settings.get(&course_id).and_then(published_calculation) else { return Ok(None) };
    own_course_grade(session, course_id, calculation).await
}

async fn own_course_grade(
    session: &LiveSession, course_id: i64, calculation: GradeCalculation,
) -> Result<Option<CourseGrade>, String> {
    let user_id = current_user_id(session).await?;
    let key = classwork::CourseUserId {
        course_id: Some(classwork::CourseId { id: Some(course_id) }),
        user_id: Some(classwork::UserId { id: Some(user_id) }),
    };
    let response: classwork::CourseUserLookupResponse = session.rpc(
        "LookupCourseUser",
        classwork::CourseUserLookupRequest {
            course_user_ids: vec![key.clone()],
            read_mask: Some(classwork::CourseUserReadMask {
                include_course_role: Some(true), include_student_average: Some(true),
                ..Default::default()
            }),
        }, Some(course_id),
    ).await?;
    select_own_average(response, &key, calculation)
}

fn select_own_average(
    response: classwork::CourseUserLookupResponse, key: &classwork::CourseUserId, calculation: GradeCalculation,
) -> Result<Option<CourseGrade>, String> {
    let mut own = response.results.into_iter().filter(|entry| entry.course_user_id.as_ref() == Some(key))
        .filter_map(|entry| entry.course_user)
        .filter(|value| value.course_user_id.as_ref() == Some(key) && value.course_role == Some(3));
    let Some(entry) = own.next() else { return Ok(None) };
    if own.next().is_some() {
        return Err("Classroom returned conflicting overall grade records.".into());
    }
    // Android divides the integer by 10,000 at 0x17fed2c, then passes that
    // percentage to the student profile formatter at 0x1a38f14.
    Ok(entry.student_average.filter(|average| *average >= 0)
        .map(|average| CourseGrade { percent: f64::from(average) / 10_000.0, calculation }))
}

pub async fn gradebook_coverage(
    session: &LiveSession, courses: &[i64],
) -> Result<(usize, BTreeMap<i32, usize>, usize), String> {
    let settings = load_gradebook_settings(session, courses).await?;
    let mut visibility = BTreeMap::new();
    let mut published = 0;
    for (course, value) in &settings {
        *visibility.entry(value.grade_display_setting.unwrap_or_default()).or_insert(0) += 1;
        if let Some(calculation) = published_calculation(value) {
            published += usize::from(own_course_grade(session, *course, calculation).await?.is_some());
        }
    }
    Ok((settings.len(), visibility, published))
}


#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    fn settings(course: i64, display: Option<i32>, calculation: Option<i32>) -> classwork::GradebookSettings {
        classwork::GradebookSettings { course_id: Some(classwork::CourseId { id: Some(course) }),
            grade_display_setting: display, grade_calculation_type: calculation }
    }

    #[test]
    fn only_explicit_student_visibility_allows_reading_the_overall_grade() {
        for visibility in [None, Some(0), Some(2), Some(3), Some(4)] {
            assert_eq!(published_calculation(&settings(1, visibility, Some(1))), None);
        }
        assert_eq!(published_calculation(&settings(1, Some(1), Some(1))), Some(GradeCalculation::TotalPoints));
        assert_eq!(published_calculation(&settings(1, Some(1), Some(2))), Some(GradeCalculation::WeightedCategories));
        assert_eq!(published_calculation(&settings(1, Some(1), Some(0))), None);
    }

    #[test]
    fn gradebook_settings_cannot_escape_the_requested_course() {
        let row = |outer, inner| classwork::GradebookSettingsLookupResult {
            course_id: Some(classwork::CourseId { id: Some(outer) }),
            gradebook_settings: Some(settings(inner, Some(1), Some(2))),
        };
        let request = BTreeSet::from([1, 2]);
        let response = classwork::GradebookSettingsLookupResponse {
            results: vec![row(1, 1), row(2, 7), row(5, 5)],
        };
        let gradebooks = select_settings(response, &request).unwrap();
        assert_eq!(gradebooks.keys().copied().collect::<Vec<_>>(), [1]);
        assert!(select_settings(classwork::GradebookSettingsLookupResponse {
            results: vec![row(1, 1), row(1, 1)],
        }, &request).is_err());
    }

    #[test]
    fn own_totals_require_both_ids_student_role_and_explicit_numeric_presence() {
        let identity = |course, user| classwork::CourseUserId {
            course_id: Some(classwork::CourseId { id: Some(course) }),
            user_id: Some(classwork::UserId { id: Some(user) }),
        };
        let own = identity(1, 9);
        let lookup = |outer, inner, role, average| classwork::CourseUserLookupResult {
            course_user_id: Some(outer),
            course_user: Some(classwork::CourseUser {
                course_user_id: Some(inner), course_role: Some(role), student_average: average,
                ..Default::default()
            }),
        };
        let valid = lookup(own.clone(), own.clone(), 3, Some(0));
        let invalid = [
            lookup(identity(1, 8), identity(1, 8), 3, Some(50)),
            lookup(own.clone(), identity(2, 9), 3, Some(50)),
            lookup(own.clone(), own.clone(), 2, Some(50)),
        ];
        let mut rows = invalid.to_vec();
        rows.push(valid);
        let grade = select_own_average(classwork::CourseUserLookupResponse { results: rows },
            &own, GradeCalculation::TotalPoints).unwrap().unwrap();
        assert_eq!(grade.percent, 0.0);
        let withheld = lookup(own.clone(), own.clone(), 3, None);
        assert_eq!(select_own_average(classwork::CourseUserLookupResponse { results: vec![withheld] },
            &own, GradeCalculation::TotalPoints), Ok(None));
    }

    #[test]
    fn protobuf_overall_grade_decodes_to_percentage_points() {
        let key = classwork::CourseUserId {
            course_id: Some(classwork::CourseId { id: Some(1) }),
            user_id: Some(classwork::UserId { id: Some(9) }),
        };
        for (wire, expected) in [
            ([0x30, 0xc0, 0x84, 0x3d], 100.0), // 1,000,000
            ([0x30, 0xe4, 0xc0, 0x35], 87.6644), // 876,644
            ([0x30, 0xd0, 0xae, 0x42], 108.7312), // 1,087,312
        ] {
            let course_user = classwork::CourseUser {
                course_user_id: Some(key.clone()),
                course_role: Some(3),
                ..classwork::CourseUser::decode(wire.as_slice()).unwrap()
            };
            let result = select_own_average(classwork::CourseUserLookupResponse {
                results: vec![classwork::CourseUserLookupResult {
                    course_user_id: Some(key.clone()),
                    course_user: Some(course_user),
                }],
            }, &key, GradeCalculation::TotalPoints).unwrap().unwrap();
            assert_eq!(result.percent, expected);
        }
    }
}
