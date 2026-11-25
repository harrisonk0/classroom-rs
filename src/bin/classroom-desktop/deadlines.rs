use super::*;
use iced::widget::column;

impl App {
    pub(super) fn active_course_options(&self) -> Vec<ClassFilterOption> {
        let mut classes: Vec<_> = self.snapshot.iter().flat_map(|data| &data.courses)
            .filter(|course| home_course(course))
            .map(|course| ClassFilterOption { id: Some(course.id), label: course.title.clone() })
            .collect();
        classes.sort_by_cached_key(|course| course.label.to_lowercase());
        classes.insert(0, ClassFilterOption { id: None, label: "All classes".into() });
        classes
    }

    fn class_filter_picker(
        &self, class_id: Option<i64>, on_select: fn(ClassFilterOption) -> Message,
    ) -> Element<'_, Message> {
        let classes = self.active_course_options();
        let selected = classes.iter().find(|class| class.id == class_id).cloned();
        pick_list(classes, selected, on_select)
            .width(190).text_size(14).padding(8)
            .style(|_, status| {
                let active = matches!(status, pick_list::Status::Active);
                pick_list::Style {
                    text_color: FG,
                    placeholder_color: MUTED,
                    handle_color: ACTIVE_FG,
                    background: (if active { WHITE } else { ACTIVE_BG }).into(),
                    border: border(if active { BORDER } else { ACTIVE_FG }, 1.0, 8.0),
                }
            })
            .into()
    }

    pub(super) fn calendar_items(&self, monday: NaiveDate) -> BTreeMap<NaiveDate, Vec<(&Coursework, &Course)>> {
        let mut by_day: BTreeMap<_, Vec<_>> = BTreeMap::new();
        if let Some(data) = &self.snapshot {
            for course in data.courses.iter().filter(|course| {
                home_course(course) && self.calendar_class.is_none_or(|id| id == course.id)
            }) {
                for item in data.coursework_for(course.id).unwrap_or_default() {
                    if let Some(date) = item.due_date(&Local)
                        && monday_of(date) == monday
                    {
                        by_day.entry(date).or_default().push((item, course));
                    }
                }
            }
        }
        for items in by_day.values_mut() {
            items.sort_by_key(|(item, _)| item.due_at_millis);
        }
        by_day
    }

    pub(super) fn calendar(&self) -> Element<'_, Message> {
        let today = self.now.date_naive();
        let monday = self.calendar_monday;
        let sunday = monday + Days::new(6);
        let span = if monday.year() != sunday.year() {
            format!("{} – {}", monday.format("%-d %b %Y"), sunday.format("%-d %b %Y"))
        } else if monday.month() != sunday.month() {
            format!("{} – {}", monday.format("%-d %b"), sunday.format("%-d %b %Y"))
        } else {
            format!("{}–{}", monday.day(), sunday.format("%-d %b %Y"))
        };
        let mut toolbar = row![
            button("‹").padding([7, 12]).style(outline_style).on_press(Message::CalendarShift(-1)),
            button("›").padding([7, 12]).style(outline_style).on_press(Message::CalendarShift(1)),
            button("Today").padding([7, 12]).style(outline_style).on_press(Message::CalendarToday),
            text(span).size(18).font(medium()).color(FG),
            self.class_filter_picker(self.calendar_class, Message::CalendarFilter),
        ].spacing(9).align_y(Alignment::Center);
        if let Some(calendar) = self.calendar_class.and_then(|course| self.class_calendar_button(course)) {
            toolbar = toolbar.push(calendar);
        }
        let toolbar = toolbar.wrap().vertical_spacing(9);

        let items = self.calendar_items(monday);
        let mut body = column![text("Calendar").size(27).color(FG), toolbar].spacing(14);
        let status = self.page_read_status(PageRead::Assignments, self.assignments_loaded, "Loading due dates…");
        if let Some(status) = status {
            body = body.push(status);
        } else if items.is_empty() {
            body = body.push(text("No work due this week.").size(14).color(MUTED));
        }

        let week = responsive(move |size| {
            if size.width >= 670.0 {
                return Self::calendar_grid(monday, today, &items);
            }
            let mut agenda = column![].spacing(12);
            for offset in 0..7 {
                let date = monday + Days::new(offset);
                let mut day = column![
                    text(calendar_agenda_heading(date, today)).size(15).font(medium()).color(FG)
                ].spacing(7);
                for (item, course) in items.get(&date).into_iter().flatten() {
                    day = day.push(coursework_row(item, Some(&course.title)));
                }
                agenda = agenda.push(day);
            }
            agenda.into()
        }).height(Length::Shrink);
        scrollable(body.push(week).padding(26).width(Fill))
            .width(Fill).height(Fill).into()
    }

    pub(super) fn class_calendar_button(&self, course: i64) -> Option<Element<'_, Message>> {
        let snapshot = self.snapshot.as_ref()?;
        if self.verify_live_course_count || (snapshot.source != SnapshotSource::Live && self.screenshot_directory.is_none()) {
            return None;
        }
        let url = snapshot.courses.iter().find(|entry| entry.id == course)?.calendar_url.as_ref()?;
        Some(button(row![icon(Icon::Calendar, BLUE, 17), text("Google Calendar")].spacing(6).align_y(Alignment::Center))
            .padding([8, 12]).style(outline_style).on_press(Message::OpenAttachment(url.clone())).into())
    }

    fn calendar_grid<'a>(
        monday: NaiveDate,
        today: NaiveDate,
        items: &BTreeMap<NaiveDate, Vec<(&'a Coursework, &'a Course)>>,
    ) -> Element<'a, Message> {
        let mut days = row![].spacing(5);
        for offset in 0..7 {
            let date = monday + Days::new(offset);
            let is_today = today == date;
            let heading = column![
                text(date.format("%a").to_string()).size(12).color(MUTED),
                text(date.day().to_string()).size(18).font(medium())
                    .color(if is_today { ACTIVE_FG } else { FG }),
            ].spacing(1).align_x(Alignment::Center);
            let mut day = column![
                container(heading).width(Fill).center_x(Fill).padding([8, 2])
                    .style(move |_| surface(if is_today { ACTIVE_BG } else { SOFT }, 6.0)),
            ].spacing(5).width(Fill);
            for (item, course) in items.get(&date).into_iter().flatten() {
                let accent = course_color(course);
                let entry = row![
                    container(space().width(3).height(28)).style(move |_| surface(accent, 2.0)),
                    column![
                        text(truncate(&item.title, 36)).size(13).color(FG).width(Fill)
                            .wrapping(text::Wrapping::WordOrGlyph),
                        text(truncate(&course.title, 22)).size(11).color(MUTED).width(Fill)
                            .wrapping(text::Wrapping::WordOrGlyph),
                    ].spacing(3).width(Fill),
                ].spacing(4).align_y(Alignment::Center);
                let open = button(entry).padding([6, 4]).width(Fill).style(outline_style)
                    .on_press(Message::OpenItem(item.course_id, item.id.clone()));
                let details = column![
                    text(&item.title).size(14).font(medium()).color(FG),
                    text(&course.title).size(13).color(MUTED),
                    text(item.due_label.as_deref().unwrap_or_default()).size(13).color(MUTED),
                ].spacing(3);
                day = day.push(tooltip(open, details, tooltip::Position::Bottom)
                    .style(|_| outline_surface(8.0)));
            }
            days = days.push(container(day).width(Length::FillPortion(1)).padding(4)
                .style(|_| outline_surface(7.0)));
        }

        days.into()
    }

    pub(super) fn todo_items(&self, today: NaiveDate, filter: TodoFilter) -> BTreeMap<TodoGroup, Vec<(&Coursework, &Course)>> {
        let mut groups: BTreeMap<_, Vec<_>> = BTreeMap::new();
        if let Some(data) = &self.snapshot {
            for course in data.courses.iter().filter(|course| {
                home_course(course) && self.todo_class.is_none_or(|id| id == course.id)
            }) {
                for item in data.coursework_for(course.id).unwrap_or_default().iter().filter(|item| has_submission(item)) {
                    let status = item.submission.as_ref().map(|submission| submission.status);
                    let group = match TodoFilter::from_status(status) {
                        Some(status_filter) if status_filter == filter => {
                            let completed = filter == TodoFilter::Done && status != Some(SubmissionStatus::Excused);
                            TodoGroup::for_deadline(item.due_date(&Local), today, completed)
                        }
                        None if filter == TodoFilter::Assigned => TodoGroup::StatusUnavailable,
                        _ => continue,
                    };
                    groups.entry(group).or_default().push((item, course));
                }
            }
        }
        for items in groups.values_mut() {
            items.sort_by_key(|(item, _)| item.due_at_millis);
            if filter != TodoFilter::Assigned {
                items.reverse();
            }
        }
        groups
    }

    pub(super) fn todo(&self) -> Element<'_, Message> {
        let filters = row(TodoFilter::ALL.into_iter().map(|filter| {
            let active = self.todo_filter == filter;
            button(filter.label()).padding([8, 12]).style(move |theme, status| nav_style(theme, active, status))
                .on_press(Message::SetTodoFilter(filter)).into()
        })).spacing(6);
        let toolbar = row![filters, self.class_filter_picker(self.todo_class, Message::FilterTodoClass)]
            .spacing(14).align_y(Alignment::Center).wrap().vertical_spacing(8);
        let mut body = column![text("To-do").size(27).color(FG), toolbar].spacing(14);
        let status = self.page_read_status(PageRead::Assignments, self.assignments_loaded, "Loading assignments…");
        let pending = status.is_some();
        if let Some(status) = status {
            body = body.push(status);
        }
        let groups = self.todo_items(self.now.date_naive(), self.todo_filter);
        let standard = self.todo_filter.groups();
        let sections = standard.iter().copied().chain(groups.keys().copied().filter(|group| !standard.contains(group)));
        for group in sections {
            let entries = groups.get(&group).map(Vec::as_slice).unwrap_or_default();
            if pending && entries.is_empty() { continue; }
            let closed = self.collapsed_todo_groups.contains(&(self.todo_filter, group));
            let heading = button(row![
                text(group.label()).size(16).font(medium()).color(FG),
                space().width(Fill),
                text(entries.len().to_string()).size(14).color(MUTED),
                text(if entries.is_empty() || closed { "⌄" } else { "⌃" }).size(16).color(MUTED),
            ].spacing(12).align_y(Alignment::Center))
                .style(outline_style).width(Fill).padding([10, 13])
                .on_press_maybe((!entries.is_empty()).then_some(Message::ToggleTodoGroup(group)));
            body = body.push(heading);
            if !closed {
                for (item, course) in entries {
                    body = body.push(coursework_row(item, Some(&course.title)));
                }
            }
        }
        scrollable(body.padding(26).width(Fill)).width(Fill).height(Fill).into()
    }
}

pub(super) fn calendar_agenda_heading(date: NaiveDate, today: NaiveDate) -> String {
    match (date - today).num_days() {
        -1 => "Yesterday".into(),
        0 => "Today".into(),
        1 => "Tomorrow".into(),
        _ => date.format("%-d %b").to_string(),
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum TodoFilter {
    #[default]
    Assigned,
    Missing,
    Done,
}

impl TodoFilter {
    pub(super) const ALL: [Self; 3] = [Self::Assigned, Self::Missing, Self::Done];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Assigned => "Assigned",
            Self::Missing => "Missing",
            Self::Done => "Done",
        }
    }

    pub(super) fn from_status(status: Option<SubmissionStatus>) -> Option<Self> {
        match status? {
            SubmissionStatus::Assigned | SubmissionStatus::Reclaimed => Some(Self::Assigned),
            SubmissionStatus::Missing => Some(Self::Missing),
            SubmissionStatus::TurnedIn | SubmissionStatus::Returned | SubmissionStatus::TurnedInLate
                | SubmissionStatus::ReturnedLate | SubmissionStatus::Excused => Some(Self::Done),
            SubmissionStatus::Unknown => None,
        }
    }

    pub(super) fn groups(self) -> &'static [TodoGroup] {
        use TodoGroup::*;
        match self {
            Self::Assigned => &[NoDueDate, ThisWeek, NextWeek, Later],
            Self::Missing => &[ThisWeek, LastWeek, Earlier],
            Self::Done => &[NoDueDate, DoneEarly, ThisWeek, LastWeek, Earlier],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum TodoGroup {
    NoDueDate,
    DoneEarly,
    ThisWeek,
    NextWeek,
    Later,
    LastWeek,
    Earlier,
    StatusUnavailable,
}

impl TodoGroup {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::NoDueDate => "No due date",
            Self::DoneEarly => "Done early",
            Self::ThisWeek => "This week",
            Self::NextWeek => "Next week",
            Self::Later => "Later",
            Self::LastWeek => "Last week",
            Self::Earlier => "Earlier",
            Self::StatusUnavailable => "Status unavailable",
        }
    }

    pub(super) fn for_deadline(
        due: Option<NaiveDate>, today: NaiveDate, completed: bool,
    ) -> Self {
        let Some(due) = due else { return Self::NoDueDate };
        if completed && due > today {
            return Self::DoneEarly;
        }
        let weeks = (monday_of(due) - monday_of(today)).num_weeks();
        match weeks {
            0 => Self::ThisWeek,
            1 => Self::NextWeek,
            -1 => Self::LastWeek,
            weeks if weeks > 1 => Self::Later,
            _ => Self::Earlier,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkFilter {
    #[default]
    All,
    Assigned,
    ReturnedWithGrade,
    Missing,
}

impl WorkFilter {
    pub(super) const ALL: [Self; 4] = [Self::All, Self::Assigned, Self::ReturnedWithGrade, Self::Missing];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Assigned => "Assigned",
            Self::ReturnedWithGrade => "Returned with grade",
            Self::Missing => "Missing",
        }
    }

    pub(super) fn includes(self, item: &Coursework) -> bool {
        let status = TodoFilter::from_status(item.submission.as_ref().map(|submission| submission.status));
        match self {
            Self::All => true,
            Self::Assigned => status == Some(TodoFilter::Assigned),
            Self::ReturnedWithGrade => item.submission.as_ref().is_some_and(|submission| submission.grade.is_some()),
            Self::Missing => status == Some(TodoFilter::Missing),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct ClassFilterOption {
    pub(super) id: Option<i64>,
    pub(super) label: String,
}

impl fmt::Display for ClassFilterOption {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}
