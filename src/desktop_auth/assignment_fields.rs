use chrono::{Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};

use super::{TeacherDraft, classwork, mutation};

const FIRST_DEADLINE: i64 = 946_684_800_000;
const END_DEADLINES: i64 = 4_102_444_800_000;
const MAX_EXACT_POINTS: u64 = 9_007_199_254_740_991;
const POINTS_ERROR: &str = "Enter a non-negative whole number of points or leave it blank for Ungraded.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignmentDue {
    Date(NaiveDate),
    Timed(NaiveDateTime),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AssignmentFieldChanges {
    /// Skip the field, clear the due date, or set a new deadline.
    pub due: Option<Option<AssignmentDue>>,
    /// Keep the existing/default points, set Ungraded, or enter an explicit total.
    pub points: Option<Option<f64>>,
}

pub fn parse_assignment_due(date: &str, time: &str) -> Result<Option<AssignmentDue>, String> {
    let date = date.trim();
    let time = time.trim();
    if date.is_empty() {
        return if time.is_empty() {
            Ok(None)
        } else {
            Err("Enter a due date before adding a time.".into())
        };
    }
    let date = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|_| "Enter a valid due date in YYYY-MM-DD format.")?;
    let due = if time.is_empty() {
        AssignmentDue::Date(date)
    } else {
        let time = NaiveTime::parse_from_str(time, "%H:%M")
            .map_err(|_| "Enter a valid due time in 24-hour HH:MM format.")?;
        AssignmentDue::Timed(date.and_time(time))
    };
    encode_due(due)?;
    Ok(Some(due))
}

pub fn parse_assignment_points(input: &str) -> Result<Option<f64>, String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(None);
    }
    let points = input.parse::<u64>().map_err(|_| POINTS_ERROR)?;
    match points {
        0 => Ok(None),
        1..=MAX_EXACT_POINTS => Ok(Some(points as f64)),
        _ => Err(POINTS_ERROR.into()),
    }
}

fn validate_points(points: Option<f64>) -> Result<Option<f64>, String> {
    match points {
        None | Some(0.0) => Ok(None),
        Some(value) if (1.0..=MAX_EXACT_POINTS as f64).contains(&value) && value.fract() == 0.0 => {
            Ok(Some(value))
        }
        _ => Err(POINTS_ERROR.into()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WireDue {
    millis: i64,
    timed: bool,
}

fn encode_due(due: AssignmentDue) -> Result<WireDue, String> {
    let utc = match due {
        // Date-only deadlines retain the displayed UTC calendar day without a local shift.
        AssignmentDue::Date(date) => date.and_time(NaiveTime::MIN).and_utc(),
        AssignmentDue::Timed(date_time) => Local.from_local_datetime(&date_time).single()
            .ok_or("That due time is ambiguous or unavailable in your local time zone.")?
            .to_utc(),
    };
    let millis = utc.timestamp_millis();
    if (FIRST_DEADLINE..END_DEADLINES).contains(&millis) {
        Ok(WireDue { millis, timed: matches!(due, AssignmentDue::Timed(_)) })
    } else {
        Err("Choose a due date between 2000 and 2099.".into())
    }
}

fn draft_due(draft: &TeacherDraft) -> Option<WireDue> {
    draft.item.due_at_millis.map(|millis| WireDue {
        millis,
        timed: draft.item.due_has_time.unwrap_or(false),
    })
}

fn equivalent_due(left: Option<WireDue>, right: Option<WireDue>, compare_minutes: bool) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) if left.timed == right.timed => {
            let unit = match (left.timed, compare_minutes) {
                (true, false) => 1,
                (true, true) => 60_000,
                (false, _) => 86_400_000,
            };
            left.millis.div_euclid(unit) == right.millis.div_euclid(unit)
        }
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ValidatedAssignmentFields {
    due: Option<Option<WireDue>>,
    points: Option<Option<f64>>,
}

impl AssignmentFieldChanges {
    pub(super) fn validate(self) -> Result<ValidatedAssignmentFields, String> {
        let due = self.due.map(|value| value.map(encode_due).transpose()).transpose()?;
        let points = self.points.map(validate_points).transpose()?;
        Ok(ValidatedAssignmentFields { due, points })
    }
}

impl ValidatedAssignmentFields {
    pub(super) fn for_insert(mut self) -> Self {
        self.due = self.due.or(Some(None));
        self.points = self.points.or(Some(Some(100.0)));
        self
    }

    pub(super) fn for_update(self, original: &TeacherDraft, current: &TeacherDraft) -> Result<Self, String> {
        let current_due = draft_due(current);
        let current_points = current.item.maximum_points;
        if (self.due.is_some() && draft_due(original) != current_due)
            || (self.points.is_some() && original.item.maximum_points != current_points)
        {
            return Err("The due date or points changed elsewhere. Reopen the draft before saving.".into());
        }
        Ok(Self {
            due: self.due.filter(|due| !equivalent_due(current_due, *due, true)),
            points: self.points.filter(|points| *points != current_points),
        })
    }

    pub(super) fn task_info(self) -> classwork::TaskInfo {
        let due = self.due.flatten();
        classwork::TaskInfo {
            // The Android writer leaves dueDate unset when its date argument is null.
            // On update the corresponding mask stays true to distinguish clear from unchanged.
            due_date: due.map(|value| value.millis),
            user_specified_due_time: due.map(|value| value.timed),
            grade_denominator: self.points.map(|value| value.unwrap_or(0.0)),
            ..Default::default()
        }
    }

    pub(super) fn update_mask(self) -> Option<mutation::TaskInfoUpdateMask> {
        (self.due.is_some() || self.points.is_some()).then_some(mutation::TaskInfoUpdateMask {
            update_due_date: self.due.map(|_| true),
            update_grade_denominator: self.points.map(|_| true),
        })
    }

    pub(super) fn confirms(self, draft: &TeacherDraft) -> bool {
        self.due.is_none_or(|expected| equivalent_due(draft_due(draft), expected, false))
            && self.points.is_none_or(|expected| draft.item.maximum_points == expected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadline_input_keeps_date_only_in_utc_and_rejects_invalid_values() {
        assert!(parse_assignment_due("", "11:30").is_err());
        assert!(parse_assignment_due("2028-02-29", "24:00").is_err());
        let due = encode_due(parse_assignment_due("2028-02-29", "").unwrap().unwrap()).unwrap();
        assert_eq!(due.millis % 86_400_000, 0);
        assert!(!due.timed);
        assert!(equivalent_due(Some(due), Some(WireDue { millis: due.millis + 80_000_000, ..due }), false));
        let timed = WireDue { timed: true, ..due };
        let seconds = WireDue { millis: timed.millis + 30_000, ..timed };
        assert!(!equivalent_due(Some(timed), Some(seconds), false));
        assert!(equivalent_due(Some(timed), Some(seconds), true));
        assert!(!equivalent_due(Some(due), Some(timed), true));
    }

    #[test]
    fn assignment_points_are_finite_exact_nonnegative_integers() {
        assert_eq!(parse_assignment_points(""), Ok(None));
        assert_eq!(parse_assignment_points("0"), Ok(None));
        assert_eq!(parse_assignment_points(" 100 "), Ok(Some(100.0)));
        for invalid in ["-1", "0.01", "NaN", "inf", "9007199254740992", "9007199254740990.1"] {
            assert!(parse_assignment_points(invalid).is_err());
        }
    }
}
