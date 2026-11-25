use super::*;

pub(super) struct AssignmentEditor {
    pub(super) course: i64,
    pub(super) original: Option<desktop_auth::TeacherDraft>,
    pub(super) title: String,
    pub(super) instructions: text_editor::Content,
    pub(super) settings: AssignmentFieldInputs,
}

pub(super) struct AssignmentFieldInputs {
    pub(super) due_date: String,
    pub(super) due_time: String,
    pub(super) points: String,
}

impl AssignmentFieldInputs {
    pub(super) fn from_item(item: Option<&Coursework>) -> Self {
        let points = match item {
            None => "100".into(),
            Some(item) => item.maximum_points.map_or_else(String::new, |points| points.to_string()),
        };
        let due_date = item.and_then(|item| item.due_date(&Local))
            .map_or_else(String::new, |date| date.format("%Y-%m-%d").to_string());
        let due_time = item.filter(|item| item.due_has_time == Some(true))
            .and_then(|item| DateTime::from_timestamp_millis(item.due_at_millis?))
            .map_or_else(String::new, |due| due.with_timezone(&Local).format("%H:%M").to_string());
        Self { due_date, due_time, points }
    }

    pub(super) fn changes(&self, initial: &Self) -> Result<desktop_auth::AssignmentFieldChanges, String> {
        let due = (self.due_date.trim() != initial.due_date || self.due_time.trim() != initial.due_time)
            .then(|| desktop_auth::parse_assignment_due(&self.due_date, &self.due_time)).transpose()?;
        let points = (self.points.trim() != initial.points)
            .then(|| desktop_auth::parse_assignment_points(&self.points)).transpose()?;
        Ok(desktop_auth::AssignmentFieldChanges { due, points })
    }
}

impl AssignmentEditor {
    pub(super) fn new(course: i64, original: Option<desktop_auth::TeacherDraft>) -> Self {
        let item = original.as_ref().map(|draft| &draft.item);
        let title = item.map_or_else(String::new, |item| item.title.clone());
        let initial = item.map_or("", |item| item.description.as_str());
        let instructions = text_editor::Content::with_text(initial);
        let settings = AssignmentFieldInputs::from_item(item);
        Self { course, original, title, instructions, settings }
    }

    pub(super) fn field_changes(&self) -> Result<desktop_auth::AssignmentFieldChanges, String> {
        self.settings.changes(&AssignmentFieldInputs::from_item(self.original.as_ref().map(|draft| &draft.item)))
    }

    pub(super) fn instructions_editable(&self) -> bool {
        self.original.as_ref().is_none_or(|draft| draft.item.description_styles.is_empty()
            && draft.item.description_bullets.is_empty())
    }

    pub(super) fn instructions_text(&self) -> String {
        if self.instructions_editable() {
            self.instructions.text()
        } else {
            self.original.as_ref().map_or_else(String::new, |draft| draft.item.description.clone())
        }
    }
}

pub(super) struct AssignmentPublicationPrompt {
    pub(super) draft: desktop_auth::TeacherDraft,
    pub(super) schedule: Option<String>,
}

pub(super) struct CommentMentions {
    pub(super) course: i64,
    pub(super) item: i64,
    pub(super) entries: Option<Vec<desktop_auth::ClassMention>>,
    pub(super) error: Option<String>,
}

#[derive(Clone, Copy)]
pub(super) enum NewClassField {
    Name,
    Section,
    Subject,
    Room,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum StudentWorkFilter {
    #[default]
    All,
    Assigned,
    TurnedIn,
    Graded,
    Returned,
}

impl StudentWorkFilter {
    pub(super) const ALL: [Self; 5] = [Self::All, Self::Assigned, Self::TurnedIn, Self::Graded, Self::Returned];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Assigned => "Assigned",
            Self::TurnedIn => "Turned in",
            Self::Graded => "Graded",
            Self::Returned => "Returned",
        }
    }

    pub(super) fn includes(self, row: &desktop_auth::TeacherSubmission) -> bool {
        use SubmissionStatus::*;
        match self {
            Self::All => true,
            Self::Assigned => matches!(row.status, Assigned | Missing | Reclaimed),
            Self::TurnedIn => matches!(row.status, TurnedIn | TurnedInLate),
            Self::Graded => matches!(row.status, Returned | ReturnedLate) && row.released_grade.is_some(),
            Self::Returned => matches!(row.status, Returned | ReturnedLate) && row.released_grade.is_none(),
        }
    }
}

pub(super) struct TeacherWork {
    pub(super) course: i64,
    pub(super) item: i64,
    pub(super) students: Vec<desktop_auth::TeacherSubmission>,
    pub(super) filter: StudentWorkFilter,
    pub(super) selected: Option<i64>,
    pub(super) grade: String,
    pub(super) confirming_return: bool,
    pub(super) bulk_selected: BTreeSet<i64>,
    pub(super) bulk_confirming: bool,
}

#[derive(Default)]
pub(super) struct TeacherFeedbackThread {
    pub(super) comments: Option<Vec<Comment>>,
    pub(super) draft: String,
    pub(super) unconfirmed: bool,
}

pub(super) struct ClassStreamSettingsEditor {
    pub(super) course: i64,
    pub(super) original: desktop_auth::TeacherClassSettings,
    pub(super) draft: desktop_auth::TeacherClassSettings,
    pub(super) description: text_editor::Content,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TopicChoice {
    pub(super) id: Option<String>,
    pub(super) label: String,
}

impl fmt::Display for TopicChoice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}

pub(super) struct ItemTopicEditor {
    pub(super) target: desktop_auth::TopicItemTarget,
    pub(super) title: String,
    pub(super) selected: Option<String>,
}

impl ClassStreamSettingsEditor {
    pub(super) fn new(course: i64, original: desktop_auth::TeacherClassSettings) -> Self {
        let description = text_editor::Content::with_text(&original.details.description);
        Self { course, draft: original.clone(), original, description }
    }

    pub(super) fn changes(&self) -> desktop_auth::TeacherClassSettings {
        let mut draft = self.draft.clone();
        draft.details.description = self.description.text();
        draft
    }
}

pub(super) struct GradebookEdit {
    pub(super) course: i64,
    pub(super) item: i64,
    pub(super) expected: desktop_auth::TeacherSubmission,
    pub(super) input: String,
}

impl GradebookEdit {
    pub(super) fn grade(&self, maximum: Option<f64>) -> Option<f64> {
        let grade = self.input.trim().parse().ok()?;
        desktop_auth::TeacherGradingAction::SaveDraft(grade).allowed(&self.expected, maximum).then_some(grade)
    }
}


impl TeacherWork {
    pub(super) fn returnable(&self) -> impl Iterator<Item = &desktop_auth::TeacherSubmission> {
        self.students.iter().filter(|row| desktop_auth::TeacherGradingAction::Return.allowed(row, None))
    }

    pub(super) fn selected_returns(&self) -> impl Iterator<Item = &desktop_auth::TeacherSubmission> {
        self.returnable().filter(|row| self.bulk_selected.contains(&row.student_id))
    }

    pub(super) fn selected_row(&self) -> Option<&desktop_auth::TeacherSubmission> {
        self.students.iter().find(|row| self.selected == Some(row.student_id))
    }

    pub(super) fn select(&mut self, selected: Option<i64>) {
        self.selected = selected;
        self.grade = self.selected_row().and_then(|row| row.draft_grade.or(row.released_grade))
            .map_or_else(String::new, |grade| grade.to_string());
        self.confirming_return = false;
    }
}

impl AssignmentPublicationPrompt {
    pub(super) fn action(&self) -> Option<desktop_auth::AssignmentPublication> {
        let action = match &self.schedule {
            None => desktop_auth::AssignmentPublication::Publish,
            Some(value) => desktop_auth::AssignmentPublication::Schedule(local_schedule_millis(&Local, value)?),
        };
        action.allowed_for(self.draft.publication).then_some(action)
    }
}

pub(super) fn local_schedule_millis(zone: &impl TimeZone, value: &str) -> Option<i64> {
    let date = NaiveDateTime::parse_from_str(value.trim(), "%Y-%m-%d %H:%M").ok()?;
    zone.from_local_datetime(&date).single().map(|date| date.timestamp_millis())
}
