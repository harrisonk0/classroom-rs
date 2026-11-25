//! Data shared by the native desktop client and its optional offline cache.

mod cache;
mod demo;
pub use cache::{CacheError, clear_cache, load_cache, save_cache};

use chrono::{DateTime, Days, NaiveDate, TimeZone, Weekday};
use std::{collections::{BTreeMap, BTreeSet}, time::{SystemTime, UNIX_EPOCH}};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Course {
    pub id: i64,
    pub title: String,
    pub subtitle: String,
    pub description: String,
    pub room: String,
    #[serde(default)]
    pub state: CourseState,
    #[serde(default)]
    pub hidden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reordering_sort_key: Option<Vec<u8>>,
    #[serde(default)]
    pub theme: Option<CourseTheme>,
    #[serde(skip)]
    pub meet_url: Option<String>,
    #[serde(skip)]
    pub calendar_url: Option<String>,
    #[serde(default)]
    pub stream_classwork_display: Option<StreamClassworkDisplay>,
    #[serde(skip)]
    pub owner_ids: Vec<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamClassworkDisplay {
    Condensed,
    Expanded,
    Hidden,
}

impl StreamClassworkDisplay {
    pub(crate) fn from_wire(value: i32) -> Option<Self> {
        match value {
            1 => Some(Self::Condensed),
            2 => Some(Self::Expanded),
            3 => Some(Self::Hidden),
            _ => None,
        }
    }

    pub fn shows(self, kind: CourseworkKind) -> bool {
        self != Self::Hidden || kind == CourseworkKind::Announcement
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CourseTheme {
    pub color_rgb: Option<[u8; 3]>,
    /// Remote artwork may include an asset key. It is never cached on disk.
    pub artwork_url: Option<String>,
}

impl Course {
    pub fn from_wire(course: crate::wire::Course) -> Option<Self> {
        Some(Self {
            id: course.course_id?.id?,
            title: course.title.filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "Untitled class".into()),
            subtitle: course.subtitle.unwrap_or_default(),
            description: course.description.unwrap_or_default(),
            room: course.room.unwrap_or_default(),
            state: CourseState::from_wire(course.course_state),
            hidden: course.current_user_setting.and_then(|settings| decode_class_hidden(settings.course_visibility_setting)),
            reordering_sort_key: course.reordering_sort_key.filter(|key| !key.is_empty()),
            theme: None,
            meet_url: None,
            calendar_url: None,
            stream_classwork_display: course.course_setting.and_then(|setting| setting.classwork_in_stream_display_type)
                .and_then(StreamClassworkDisplay::from_wire),
            owner_ids: vec![],
        })
    }

    pub fn shows_on_home(&self) -> bool {
        self.state.shows_on_home() && self.hidden != Some(true)
    }
}

pub(crate) fn decode_class_hidden(value: Option<i32>) -> Option<bool> {
    match value.and_then(|value| crate::wire::CourseVisibilitySetting::try_from(value).ok()) {
        Some(crate::wire::CourseVisibilitySetting::Visible) => Some(false),
        Some(crate::wire::CourseVisibilitySetting::HiddenByUser) => Some(true),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CourseState {
    #[default]
    Unknown,
    Active,
    Archived,
    Other,
}

impl CourseState {
    pub fn shows_on_home(self) -> bool {
        matches!(self, Self::Active | Self::Unknown)
    }

    pub fn from_wire(state: Option<i32>) -> Self {
        match state {
            None | Some(0) => Self::Unknown,
            Some(1) => Self::Active,
            Some(2) => Self::Archived,
            Some(_) => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CourseworkKind {
    Announcement,
    Assignment,
    Question,
    Material,
}

impl CourseworkKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Announcement => "Announcement",
            Self::Assignment => "Assignment",
            Self::Question => "Question",
            Self::Material => "Material",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttachmentKind {
    Link,
    DriveFile,
    Video,
    Form,
    Other,
}

impl AttachmentKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Link => "Link",
            Self::DriveFile => "Drive file",
            Self::Video => "Video",
            Self::Form => "Form",
            Self::Other => "Attachment",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    #[serde(default)]
    pub remote_id: Option<i64>,
    pub title: String,
    /// Source-provided URL. The caller must validate it before opening.
    pub url: Option<String>,
    pub kind: AttachmentKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drive_sharing: Option<DriveSharing>,
    #[serde(skip)]
    pub drive_reference: Option<String>,
    #[serde(skip)]
    pub drive_parent_reference: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DriveSharing {
    IndividualCopy,
    View,
    Edit,
    CopyToKeep,
}

impl DriveSharing {
    pub fn label(self) -> &'static str {
        match self {
            Self::IndividualCopy => "Personal copy",
            Self::View => "View only",
            Self::Edit => "Class can edit",
            Self::CopyToKeep => "Copy to keep",
        }
    }
}

impl Attachment {
    pub fn is_personal_copy_of(&self, template: &Self) -> bool {
        template.drive_sharing == Some(DriveSharing::IndividualCopy)
            && template.drive_reference.as_deref().zip(self.drive_parent_reference.as_deref())
                .is_some_and(|(template, parent)| template == parent && self.drive_reference.as_deref().is_some_and(|id| id != parent))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubmissionStatus {
    #[default]
    Unknown,
    Assigned,
    TurnedIn,
    TurnedInLate,
    Returned,
    ReturnedLate,
    Reclaimed,
    Missing,
    Excused,
}

impl SubmissionStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Status unavailable",
            Self::Assigned => "Assigned",
            Self::TurnedIn => "Turned in",
            Self::TurnedInLate => "Turned in late",
            Self::Returned => "Returned",
            Self::ReturnedLate => "Returned · late",
            Self::Reclaimed => "Unsubmitted",
            Self::Missing => "Missing",
            Self::Excused => "Excused",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradeCalculation {
    TotalPoints,
    WeightedCategories,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CourseGrade {
    pub percent: f64,
    pub calculation: GradeCalculation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Grade {
    pub earned: f64,
    pub maximum: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Submission {
    #[serde(default)]
    pub remote_id: Option<i64>,
    #[serde(default)]
    pub remote_state: Option<i32>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    pub status: SubmissionStatus,
    /// A grade released to the signed-in student. Never contain a draft grade.
    pub grade: Option<Grade>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_turned_in_millis: Option<i64>,
    #[serde(skip)]
    pub question_answer: Option<QuestionAnswer>,
    #[serde(skip)]
    pub rubric_scores: BTreeMap<String, CriterionScore>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubmissionEvent {
    pub status: SubmissionStatus,
    pub timestamp_millis: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CriterionScore {
    pub rating_id: Option<String>,
    pub points: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rubric {
    pub title: Option<String>,
    pub criteria: Vec<RubricCriterion>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RubricCriterion {
    pub id: Option<String>,
    pub title: String,
    pub description: String,
    pub ratings: Vec<RubricRating>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RubricRating {
    pub id: Option<String>,
    pub title: String,
    pub description: String,
    pub points: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Topic {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CourseRole {
    Teacher,
    Student,
    #[default]
    Unknown,
}

impl CourseRole {
    pub fn label(self) -> &'static str {
        match self {
            Self::Teacher => "Teacher",
            Self::Student => "Student",
            Self::Unknown => "Member",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CourseMember {
    pub id: String,
    pub display_name: String,
    pub role: CourseRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    #[serde(skip)]
    pub author_id: Option<i64>,
    pub author: String,
    pub body: String,
    pub created_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestionDetails {
    ShortAnswer,
    MultipleChoice(Vec<String>),
}

impl QuestionDetails {
    pub fn label(&self) -> &'static str {
        match self {
            Self::ShortAnswer => "Short-answer question",
            Self::MultipleChoice(_) => "Multiple-choice question",
        }
    }

    pub fn accepts(&self, answer: &QuestionAnswer) -> bool {
        match (self, answer) {
            (Self::ShortAnswer, QuestionAnswer::ShortAnswer(value)) => !value.trim().is_empty(),
            (Self::MultipleChoice(choices), QuestionAnswer::MultipleChoice(value)) => {
                !value.trim().is_empty() && matching_choice(choices, value).is_some()
            }
            _ => false,
        }
    }
}

pub fn matching_choice(choices: &[String], answer: &str) -> Option<usize> {
    let mut matches = choices.iter().enumerate().filter(|(_, choice)| choice.as_str() == answer);
    let first = matches.next()?.0;
    matches.next().is_none().then_some(first)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionAnswer {
    ShortAnswer(String),
    MultipleChoice(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Coursework {
    pub id: String,
    pub course_id: i64,
    pub title: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub description_styles: Vec<DescriptionStyle>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub description_bullets: Vec<usize>,
    pub due_label: Option<String>,
    #[serde(default)]
    pub due_at_millis: Option<i64>,
    #[serde(default)]
    pub due_has_time: Option<bool>,
    #[serde(default)]
    pub maximum_points: Option<f64>,
    pub kind: CourseworkKind,
    #[serde(skip)]
    pub author_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posted_at_millis: Option<i64>,
    #[serde(default)]
    pub question: Option<QuestionDetails>,
    #[serde(skip)]
    pub question_sharing: Option<QuestionSharing>,
    #[serde(skip)]
    pub originality_enabled: Option<bool>,
    #[serde(default)]
    pub allow_edit_after_turn_in: bool,
    #[serde(default)]
    pub topic_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classwork_sort_key: Option<Vec<u8>>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub submission: Option<Submission>,
    /// Only comments visible to the signed-in student. None means not fetched.
    #[serde(default)]
    pub comments: Option<Vec<Comment>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionSharing {
    Private,
    Course,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DescriptionStyle {
    pub start: usize,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl DescriptionStyle {
    pub fn is_plain(self) -> bool {
        !(self.bold || self.italic || self.underline)
    }
}

impl Coursework {
    pub fn due_date(&self, timezone: &impl TimeZone) -> Option<NaiveDate> {
        let due = DateTime::from_timestamp_millis(self.due_at_millis?)?;
        Some(if self.has_due_time() { due.with_timezone(timezone).date_naive() } else { due.date_naive() })
    }

    fn has_due_time(&self) -> bool {
        // Older offline copies only saved the formatted label alongside the timestamp.
        self.due_has_time.unwrap_or_else(|| {
            self.due_label.as_ref().is_some_and(|label| label.contains(':'))
        })
    }
}

pub fn monday_of(date: NaiveDate) -> NaiveDate {
    date.week(Weekday::Mon).first_day()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotSource {
    Demo,
    Live,
    Cache,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub source: SnapshotSource,
    pub courses: Vec<Course>,
    /// Presence distinguishes a fetched, empty class stream from one not yet fetched.
    pub coursework: BTreeMap<i64, Vec<Coursework>>,
    /// A present list, including an empty one, means that topics were fetched.
    #[serde(default)]
    pub topics: BTreeMap<i64, Vec<Topic>>,
    /// A present list, including an empty one, means that people were fetched.
    #[serde(default)]
    pub roster: BTreeMap<i64, Vec<CourseMember>>,
    /// Unix timestamp in seconds of the last successful class-list refresh.
    pub updated_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClassworkGroupId {
    NoTopic,
    Topic(String),
    UnresolvedTopic,
}

pub struct ClassworkSection<'a> {
    pub id: ClassworkGroupId,
    pub title: &'a str,
    pub items: Vec<&'a Coursework>,
    pub confirmed_order: bool,
}

impl Snapshot {
    pub fn live(courses: Vec<Course>) -> Self {
        Self {
            source: SnapshotSource::Live,
            courses,
            coursework: BTreeMap::new(),
            topics: BTreeMap::new(),
            roster: BTreeMap::new(),
            updated_at: SystemTime::now().duration_since(UNIX_EPOCH).ok()
                .map(|duration| duration.as_secs()),
        }
    }

    pub fn coursework_for(&self, id: i64) -> Option<&[Coursework]> {
        self.coursework.get(&id).map(Vec::as_slice)
    }

    pub fn classwork_sections(&self, course_id: i64, selected_topic: Option<&str>) -> Vec<ClassworkSection<'_>> {
        let topics = self.topics_for(course_id).unwrap_or_default();
        let mut items_by_topic: BTreeMap<Option<&str>, Vec<_>> = BTreeMap::new();
        for item in self.coursework_for(course_id).unwrap_or_default() {
            let topic = item.topic_id.as_deref();
            if item.kind != CourseworkKind::Announcement
                && selected_topic.is_none_or(|selected| topic == Some(selected))
            {
                items_by_topic.entry(topic).or_default().push(item);
            }
        }
        let mut confirmed_topics = BTreeSet::new();
        for (&topic, items) in &mut items_by_topic {
            let mut seen = BTreeSet::new();
            if items.iter().all(|item| item.classwork_sort_key.as_deref()
                .is_some_and(|key| !key.is_empty() && seen.insert(key)))
            {
                items.sort_by(|left, right| left.classwork_sort_key.cmp(&right.classwork_sort_key));
                confirmed_topics.insert(topic);
            }
        }

        let mut sections = Vec::new();
        if let Some(items) = items_by_topic.remove(&None) {
            sections.push(ClassworkSection {
                id: ClassworkGroupId::NoTopic, title: "No topic", items,
                confirmed_order: confirmed_topics.contains(&None),
            });
        }
        for topic in topics {
            if let Some(items) = items_by_topic.remove(&Some(topic.id.as_str())) {
                sections.push(ClassworkSection {
                    id: ClassworkGroupId::Topic(topic.id.clone()), title: &topic.title, items,
                    confirmed_order: confirmed_topics.contains(&Some(topic.id.as_str())),
                });
            }
        }
        // Topic titles may be unavailable even when the published items were returned.
        if !items_by_topic.is_empty() {
            sections.push(ClassworkSection {
                id: ClassworkGroupId::UnresolvedTopic,
                title: "Topic unavailable",
                items: items_by_topic.into_values().flatten().collect(),
                confirmed_order: false,
            });
        }
        sections
    }

    pub fn upcoming_work(
        &self, now: &DateTime<impl TimeZone>, class: Option<i64>, limit: usize,
    ) -> Vec<(&Coursework, &Course)> {
        let today = now.date_naive();
        let Some(last_day) = today.checked_add_days(Days::new(6)) else { return vec![] };
        let timezone = now.timezone();
        let mut items = Vec::new();
        for course in self.courses.iter().filter(|course| {
            course.shows_on_home() && class.is_none_or(|id| course.id == id)
        }) {
            for item in self.coursework_for(course.id).unwrap_or_default() {
                if !matches!(item.kind, CourseworkKind::Assignment | CourseworkKind::Question)
                    || item.submission.as_ref().is_some_and(|submission| {
                        !matches!(submission.status, SubmissionStatus::Assigned | SubmissionStatus::Reclaimed | SubmissionStatus::Unknown)
                    })
                {
                    continue;
                }
                let Some(day) = item.due_date(&timezone) else { continue };
                let timed = item.has_due_time();
                if (today..=last_day).contains(&day)
                    && (!timed || item.due_at_millis.is_some_and(|due| due >= now.timestamp_millis()))
                {
                    // Date-only work remains due through the end of its calendar day.
                    items.push(((day, !timed, item.due_at_millis), (item, course)));
                }
            }
        }
        items.sort_by_key(|(deadline, _)| *deadline);
        items.into_iter().take(limit).map(|(_, entry)| entry).collect()
    }

    pub fn topics_for(&self, id: i64) -> Option<&[Topic]> {
        self.topics.get(&id).map(Vec::as_slice)
    }

    pub fn roster_for(&self, id: i64) -> Option<&[CourseMember]> {
        self.roster.get(&id).map(Vec::as_slice)
    }

    pub fn set_submissions(
        &mut self,
        course_id: i64,
        submissions: impl IntoIterator<Item = (String, Submission)>,
    ) {
        let Some(items) = self.coursework.get_mut(&course_id) else {
            return;
        };
        let mut by_item: BTreeMap<_, _> = submissions.into_iter().collect();
        for item in items {
            item.submission = by_item.remove(&item.id).map(|mut submission| {
                if let Some(grade) = &mut submission.grade {
                    grade.maximum = item.maximum_points.or(grade.maximum);
                }
                submission
            });
        }
    }

    fn remove_private_cache_fields(&mut self) {
        self.roster.clear();
        for course in &mut self.courses {
            course.theme = course.theme.as_ref().and_then(|theme| theme.color_rgb)
                .map(|color_rgb| CourseTheme { color_rgb: Some(color_rgb), artwork_url: None });
        }
        for items in self.coursework.values_mut() {
            for item in items {
                item.submission = None;
                item.comments = None;
                for attachment in &mut item.attachments {
                    if attachment.url.as_ref().is_some_and(|url| url.contains(['?', '#'])) {
                        // A share or download key can be encoded in the URL.
                        attachment.url = None;
                    }
                }
            }
        }
    }
}
