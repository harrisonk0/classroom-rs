use super::*;
use iced::widget::column;

const GRADEBOOK_FIELD: &str = "gradebook-score";

impl App {
    pub(super) fn can_edit_gradebook(&self, course: i64) -> bool {
        self.can_read_pages() && self.screen == Screen::Course(course) && self.tab == Tab::Grades
            && self.class_roles.get(&course) == Some(&CourseRole::Teacher) && self.can_grade_teacher_work(course)
    }

    pub(super) fn gradebook_maximum(&self, course: i64, item: i64) -> Option<f64> {
        self.snapshot.as_ref()?.coursework_for(course)?.iter()
            .find(|entry| has_submission(entry) && entry.id.parse::<i64>().ok() == Some(item))?.maximum_points
    }

    pub(super) fn begin_gradebook_edit(&mut self, course: i64, item: i64, student: i64) -> Task<Message> {
        if !self.can_edit_gradebook(course) || self.gradebook_maximum(course, item).is_none() {
            return Task::none();
        }
        let Some(cell) = self.gradebooks.get(&course).and_then(|grid| grid.cells.get(&(student, item))).filter(|cell| cell.can_edit())
            else { return Task::none(); };
        self.gradebook_edit = Some(GradebookEdit {
            course, item, expected: cell.grading_row(student),
            input: cell.draft_grade.or(cell.released_grade).map_or_else(String::new, |grade| grade.to_string()),
        });
        iced::widget::operation::focus(GRADEBOOK_FIELD)
    }

    pub(super) fn teacher_gradebook_panel(&self, course: i64) -> Element<'_, Message> {
        const COLUMN_LIMIT: usize = 7;
        const CELL_WIDTH: f32 = 142.0;
        const NAME_WIDTH: f32 = 164.0;
        if !self.can_read_pages() && self.screenshot_directory.is_none() {
            return text("Sign in to view grades.").size(14).color(MUTED).into();
        }
        if self.class_roles.get(&course) != Some(&CourseRole::Teacher) {
            return text("Checking gradebook access…").size(14).color(MUTED).into();
        }
        let items = self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(course));
        if items.is_none()
            && let Some(status) = self.page_read_status(PageRead::Coursework(course), false, "Loading classwork…")
        {
            return status;
        }
        let grid = self.gradebooks.get(&course);
        if let Some(status) = self.page_read_status(
            PageRead::Detail(DetailRead::TeacherGradebook(course)), grid.is_some(), "Loading gradebook…",
        ) {
            return status;
        }
        let Some(grid) = grid else { return space().into(); };
        let columns: Vec<_> = items.unwrap_or_default().iter().filter(|item| has_submission(item))
            .filter_map(|item| Some((item.id.parse::<i64>().ok()?, item))).collect();
        if columns.is_empty() || grid.students.is_empty() {
            return empty_panel(Icon::Check, "No grades to show", "Published work and enrolled students will appear here.");
        }
        let total_columns = columns.len();
        let pages = total_columns.div_ceil(COLUMN_LIMIT);
        let page = self.gradebook_page.min(pages - 1);
        let start = page * COLUMN_LIMIT;
        let stop = (start + COLUMN_LIMIT).min(columns.len());
        let columns = &columns[start..stop];
        let header_cell = |label: String, width| container(text(label).size(14).font(medium())
            .wrapping(text::Wrapping::WordOrGlyph)).padding(8).width(width);
        let mut heading = row![header_cell("Student".into(), NAME_WIDTH)].spacing(2);
        for (_, item) in columns {
            let mut header = column![
                text(&item.title).size(14).font(medium()).wrapping(text::Wrapping::WordOrGlyph),
            ].spacing(4);
            if let Some(points) = item.maximum_points {
                header = header.push(text(format!("{points} points")).size(12).color(MUTED));
            }
            heading = heading.push(container(header).padding(8).width(CELL_WIDTH));
        }
        let mut table = column![container(heading).style(|_| surface(SOFT, 5.0))].spacing(1);
        for student in &grid.students {
            let Ok(id) = student.id.parse::<i64>() else { continue; };
            let mut line = row![container(text(&student.display_name).size(14).wrapping(text::Wrapping::WordOrGlyph))
                .padding(8).width(NAME_WIDTH)].spacing(2).align_y(Alignment::Center);
            for (item, work) in columns {
                if let Some(cell) = grid.cells.get(&(id, *item)) {
                    let draft = cell.draft_grade.filter(|grade| Some(*grade) != cell.released_grade);
                    let label = if let Some(grade) = draft {
                        format!("{grade} · draft")
                    } else if let Some(grade) = cell.released_grade {
                        grade.to_string()
                    } else {
                        cell.status.label().to_owned()
                    };
                    let color = if cell.status == SubmissionStatus::Missing { color!(0xa42620) }
                        else if draft.is_some() { color!(0x137333) } else { FG };
                    if let Some(edit) = self.gradebook_edit.as_ref().filter(|edit| {
                        (edit.course, edit.item, edit.expected.student_id) == (course, *item, id)
                    }) {
                        let ready = !self.has_pending_request() && edit.grade(work.maximum_points).is_some();
                        let submit = ready || (!self.has_pending_request() && grade_input_matches_saved(&edit.input, &edit.expected));
                        let field = text_input("Points", &edit.input).id(GRADEBOOK_FIELD).padding(6).width(Fill)
                            .on_input_maybe((!self.saving_teacher_grade).then_some(Message::GradebookGradeInput))
                            .on_submit_maybe(submit.then_some(Message::SaveGradebookEdit(true)));
                        line = line.push(row![
                            field,
                            button("Save").padding([6, 7]).style(outline_style)
                                .on_press_maybe(ready.then_some(Message::SaveGradebookEdit(false))),
                        ].spacing(4).align_y(Alignment::Center).width(CELL_WIDTH));
                    } else {
                        let action = if self.gradebook_edit_mode && cell.can_edit() && work.maximum_points.is_some() {
                            Message::EditGradebookCell(course, *item, id)
                        } else {
                            Message::OpenGradebookCell(course, *item, id)
                        };
                        line = line.push(button(text(label).size(13).color(color)).style(quiet_style).padding(8)
                            .width(CELL_WIDTH).on_press_maybe(self.gradebook_edit.is_none().then_some(action)));
                    }
                } else {
                    line = line.push(container(text("—").size(13).color(MUTED)).padding(8).width(CELL_WIDTH));
                }
            }
            table = table.push(rule::horizontal(1).style(divider_style)).push(line);
        }
        let mut actions = row![].spacing(8);
        #[cfg(target_os = "macos")]
        {
            actions = actions.push(button(if self.exporting_grades { "Exporting…" } else { "Download grades (.csv)" })
                .style(outline_style).padding([7, 10])
                .on_press_maybe((!self.has_pending_request()).then_some(Message::ExportGrades(course, None))));
        }
        if self.can_edit_gradebook(course) || self.screenshot_directory.is_some() {
            actions = actions.push(button(if self.gradebook_edit_mode { "Done editing" } else { "Edit grades" })
                .style(outline_style).padding([7, 10])
                .on_press_maybe((!self.has_pending_request() && self.gradebook_edit.is_none())
                    .then_some(Message::GradebookEditMode(!self.gradebook_edit_mode))));
            if self.gradebook_edit.is_some() {
                actions = actions.push(button("Cancel cell edit").style(quiet_style).padding([7, 10])
                    .on_press_maybe((!self.saving_teacher_grade).then_some(Message::CancelGradebookEdit)));
            }
        }
        let mut page_view = column![text("Grades").size(23).font(medium()).color(FG), actions.wrap().vertical_spacing(6)].spacing(10);
        if pages > 1 {
            page_view = page_view.push(row![
                button("‹").style(outline_style).padding([6, 10])
                    .on_press_maybe((page > 0 && self.gradebook_edit.is_none()).then_some(Message::GradebookColumnPage(page.saturating_sub(1)))),
                text(format!("Classwork {}–{stop} of {total_columns}", start + 1)).size(13),
                button("›").style(outline_style).padding([6, 10])
                    .on_press_maybe((page + 1 < pages && self.gradebook_edit.is_none()).then_some(Message::GradebookColumnPage(page + 1))),
            ].spacing(8).align_y(Alignment::Center));
        }
        page_view.push(scrollable(table)
            .direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::default())).width(Fill)).into()
    }

    pub(super) fn can_read_teacher_work(&self, course: i64, item: i64) -> bool {
        self.can_read_pages() && self.screen == Screen::Item(course)
            && self.item_id.as_deref().and_then(|value| value.parse().ok()) == Some(item)
            && self.class_roles.get(&course) == Some(&CourseRole::Teacher)
            && self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(course))
                .is_some_and(|items| items.iter().any(|work| work.id.parse::<i64>().ok() == Some(item) && has_submission(work)))
    }

    pub(super) fn selected_teacher_feedback_key(&self) -> Option<desktop_auth::TeacherFeedbackKey> {
        let work = self.teacher_work.as_ref()?;
        if self.screen != Screen::Item(work.course) || self.item_id.as_deref()?.parse::<i64>().ok()? != work.item
            || self.class_roles.get(&work.course) != Some(&CourseRole::Teacher)
        {
            return None;
        }
        let row = work.selected_row()?;
        (row.student_id > 0 && row.submission_id > 0).then_some(desktop_auth::TeacherFeedbackKey {
            course: work.course, item: work.item, student: row.student_id, submission: row.submission_id,
        })
    }

    #[cfg(target_os = "macos")]
    pub(super) fn export_grades(&mut self, course: i64, item: Option<i64>) -> Task<Message> {
        let (Some(session), Some(snapshot)) = (self.session.clone(), self.snapshot.as_ref()) else { return Task::none(); };
        let Some(items) = snapshot.coursework_for(course) else { return Task::none(); };
        let (csv, suggested) = if let Some(item) = item {
            let Some(work) = self.teacher_work.as_ref().filter(|work| work.course == course && work.item == item
                && self.can_read_teacher_work(course, item)) else { return Task::none(); };
            let Some(assignment) = items.iter().find(|entry| entry.id.parse::<i64>().ok() == Some(item)) else { return Task::none(); };
            let grid = desktop_auth::TeacherGradebook {
                students: work.students.iter().map(|row| CourseMember {
                    id: row.student_id.to_string(), display_name: row.name.clone().unwrap_or_else(|| "Student name unavailable".into()),
                    role: CourseRole::Student,
                }).collect(),
                cells: work.students.iter().map(|row| ((row.student_id, item), desktop_auth::TeacherGradebookCell {
                    submission_id: row.submission_id, remote_state: row.remote_state,
                    status: row.status, released_grade: row.released_grade, draft_grade: None,
                })).collect(),
            };
            (grade_export::gradebook_csv(&grid, &[(item, assignment)]), grade_export::suggested_name(&assignment.title))
        } else {
            if self.screen != Screen::Course(course) || self.tab != Tab::Grades { return Task::none(); }
            let Some(grid) = self.gradebooks.get(&course) else { return Task::none(); };
            let Some(entry) = snapshot.courses.iter().find(|entry| entry.id == course) else { return Task::none(); };
            let columns: Vec<_> = items.iter().filter(|item| has_submission(item))
                .filter_map(|item| Some((item.id.parse::<i64>().ok()?, item))).collect();
            (grade_export::gradebook_csv(grid, &columns), grade_export::suggested_name(&entry.title))
        };
        self.exporting_grades = true;
        self.error = None;
        return Task::perform(async move {
            let details = desktop_auth::load_own_class_details(&session, vec![course]).await?;
            if details.roles.get(&course) != Some(&CourseRole::Teacher) {
                return Err("Classroom did not confirm teacher access for this export.".into());
            }
            let Some(path) = grade_export::choose_destination(suggested).await? else { return Ok(false); };
            tokio::task::spawn_blocking(move || grade_export::save_csv(&path, &csv)).await
                .map_err(|_| "The local CSV save did not finish.")??;
            Ok(true)
        }, Message::GradesExported);
    }

    pub(super) fn can_grade_teacher_work(&self, course: i64) -> bool {
        self.snapshot.as_ref().is_some_and(|snapshot| snapshot.courses.iter()
            .any(|entry| entry.id == course && entry.state == CourseState::Active))
    }

    pub(super) fn teacher_student_work_panel<'a>(&'a self, item: &'a Coursework) -> Element<'a, Message> {
        let mut panel = column![text(if item.kind == CourseworkKind::Question { "Student answers" } else { "Student work" })
            .size(18).font(medium()).color(FG)].spacing(10).width(Fill);
        if let Ok(id) = item.id.parse::<i64>() {
            let work = self.teacher_work.as_ref().filter(|work| work.course == item.course_id && work.item == id);
            if let Some(status) = self.page_read_status(
                PageRead::Detail(DetailRead::TeacherSubmissions(item.course_id, id)), work.is_some(), "Loading student submissions…",
            ) {
                panel = panel.push(status);
            }
            if let Some(work) = work {
                #[cfg(target_os = "macos")]
                {
                    panel = panel.push(button("Download these grades (.csv)").padding([7, 10]).style(outline_style)
                        .on_press_maybe((!self.has_pending_request()).then_some(Message::ExportGrades(item.course_id, Some(id)))));
                }
                let filters = StudentWorkFilter::ALL.into_iter().map(|filter| {
                    let count = work.students.iter().filter(|student| filter.includes(student)).count();
                    let selected = work.filter == filter;
                    button(text(format!("{} {count}", filter.label())).size(13))
                        .padding([7, 10]).style(move |theme, status| nav_style(theme, selected, status))
                        .on_press(Message::FilterStudentWork(filter)).into()
                });
                panel = panel.push(row(filters).spacing(6).wrap().vertical_spacing(6));
                let can_return = self.can_grade_teacher_work(item.course_id) || self.screenshot_directory.is_some();
                if can_return && work.returnable().next().is_some() {
                    panel = panel.push(self.bulk_return_controls(work));
                }
                let mut visible = work.students.iter().filter(|student| work.filter.includes(student)).peekable();
                if visible.peek().is_none() {
                    panel = panel.push(text("No student work in this view.").size(14).color(MUTED));
                }
                for student in visible {
                    let selected = work.selected == Some(student.student_id);
                    let name = student.name.as_deref().unwrap_or("Student name unavailable");
                    let student_row = button(row![
                        avatar(first_initial(name), BLUE, 28.0, 12.0),
                        text(name).size(15).width(Fill),
                        text(student.status.label()).size(13).color(MUTED),
                        text(if selected { "⌃" } else { "⌄" }).size(16).color(MUTED),
                    ].spacing(8).align_y(Alignment::Center)).width(Fill).padding([7, 10])
                    .style(move |theme, status| nav_style(theme, selected, status)).on_press(Message::SelectTeacherStudent(student.student_id));
                    if can_return {
                        let selection: Element<'_, Message> = if desktop_auth::TeacherGradingAction::Return.allowed(student, None) {
                            checkbox(work.bulk_selected.contains(&student.student_id))
                                .on_toggle_maybe((!self.has_pending_request()).then(|| {
                                    let id = student.student_id;
                                    move |checked| Message::SelectBulkReturn(id, checked)
                                })).into()
                        } else {
                            space().width(18).into()
                        };
                        panel = panel.push(row![selection, student_row].spacing(5).align_y(Alignment::Center));
                    } else {
                        panel = panel.push(student_row);
                    }
                    if selected {
                        let mut detail = column![].spacing(6);
                        let pending_grade = student.draft_grade.filter(|grade| Some(*grade) != student.released_grade);
                        for (label, grade) in [("Released grade", student.released_grade), ("Draft grade", pending_grade)] {
                            if let Some(grade) = grade {
                                let mark = item.maximum_points.map_or_else(|| grade.to_string(), |out_of| format!("{grade} / {out_of}"));
                                detail = detail.push(text(format!("{label} · {mark}")).size(14).color(FG));
                            }
                        }
                        if let Some(QuestionAnswer::ShortAnswer(answer) | QuestionAnswer::MultipleChoice(answer)) = &student.answer {
                            detail = detail.push(text(answer).size(15).color(FG).width(Fill)
                                .wrapping(text::Wrapping::WordOrGlyph));
                        }
                        for attachment in &student.attachments {
                            detail = detail.push(self.stream_attachment_row(item, attachment));
                        }
                        if student.released_grade.is_none() && student.draft_grade.is_none() && student.answer.is_none()
                            && student.attachments.is_empty()
                        {
                            detail = detail.push(text("No grade, answer, or attachment was returned.").size(13).color(MUTED));
                        }
                        if can_return && !work.bulk_confirming {
                            detail = detail.push(self.teacher_grading_controls(work, student, item.maximum_points));
                        }
                        if !work.bulk_confirming
                            && let Some(key) = self.selected_teacher_feedback_key()
                        {
                            detail = detail.push(self.teacher_feedback_panel(key, name, can_return));
                        }
                        panel = panel.push(container(detail).padding([4, 12]).width(Fill));
                    }
                }
            }
        }
        container(panel).padding(14).width(Fill).style(|_| outline_surface(9.0)).into()
    }

    fn bulk_return_controls<'a>(&'a self, work: &'a TeacherWork) -> Element<'a, Message> {
        let selected: Vec<_> = work.selected_returns().collect();
        let ready = !self.has_pending_request();
        let visible: Vec<_> = work.returnable().filter(|row| work.filter.includes(row)).collect();
        let all_selected = !visible.is_empty() && visible.iter().all(|row| work.bulk_selected.contains(&row.student_id));
        let mut actions = row![
            button(if all_selected { "Deselect visible" } else { "Select all visible" }).padding([7, 10]).style(quiet_style)
                .on_press_maybe((ready && !visible.is_empty()).then_some(Message::SelectVisibleReturns(!all_selected))),
        ].spacing(6).align_y(Alignment::Center);
        if !selected.is_empty() {
            actions = actions.push(text(format!("{} selected", selected.len())).size(13).color(MUTED));
            if !work.bulk_confirming {
                actions = actions.push(button("Return selected…").padding([7, 10]).style(outline_style)
                    .on_press_maybe(ready.then_some(Message::ConfirmBulkReturn(true))));
            }
            actions = actions.push(button("Clear selection").padding([7, 10]).style(quiet_style)
                .on_press_maybe(ready.then_some(Message::ClearBulkReturns)));
        }
        let mut panel = column![actions.wrap().vertical_spacing(6)].spacing(8);
        if work.bulk_confirming && !selected.is_empty() {
            let graded = selected.iter().filter(|row| row.draft_grade.or(row.released_grade).is_some()).count();
            panel = panel.push(text(format!("Return {} submissions? {graded} include a saved grade.", selected.len())).size(14))
                .push(row![
                    button("Return work").padding([7, 10]).style(primary_style)
                        .on_press_maybe(ready.then_some(Message::ReturnSelectedTeacherWork)),
                    button("Cancel").padding([7, 10]).style(quiet_style)
                        .on_press_maybe(ready.then_some(Message::ConfirmBulkReturn(false))),
                ].spacing(6));
        }
        panel.into()
    }

    fn teacher_feedback_panel<'a>(
        &'a self, key: desktop_auth::TeacherFeedbackKey, student: &'a str, compose: bool,
    ) -> Element<'a, Message> {
        let thread = self.teacher_feedback.get(&key);
        let comments = thread.and_then(|thread| thread.comments.as_ref());
        let mut panel = column![text(format!("Private feedback to {student}")).size(15).font(medium())].spacing(7);
        if let Some(status) = self.page_read_status(
            PageRead::Detail(DetailRead::TeacherFeedback(key)), comments.is_some(), "Loading private feedback…",
        ) {
            panel = panel.push(status);
        }
        if let Some(comments) = comments {
            if comments.is_empty() {
                panel = panel.push(text("No private comments on this submission.").size(13).color(MUTED));
            }
            for comment in comments {
                panel = panel.push(self.comment_card(key.course, None, CommentAudience::Private, comment));
            }
        }
        if compose {
            let draft = thread.map_or("", |thread| thread.draft.as_str());
            let unconfirmed = thread.is_some_and(|thread| thread.unconfirmed);
            if unconfirmed {
                panel = panel.push(text("Review the updated thread before sending this draft again.").size(13).color(MUTED));
            }
            let ready = !self.has_pending_request() && comments.is_some() && !unconfirmed && !draft.trim().is_empty();
            panel = panel.push(row![
                text_input("Add a private comment…", draft).padding(8).width(Fill)
                    .on_input_maybe((!self.posting_comment).then(|| move |value| Message::EditTeacherFeedback(key, value))),
                button("Send privately").style(primary_style).padding([8, 10])
                    .on_press_maybe(ready.then_some(Message::SendTeacherFeedback(key))),
            ].spacing(6).wrap().vertical_spacing(6));
        }
        container(panel).padding(10).width(Fill).style(|_| outline_surface(8.0)).into()
    }

    fn teacher_grading_controls<'a>(&'a self, work: &'a TeacherWork, student: &'a desktop_auth::TeacherSubmission,
        maximum: Option<f64>) -> Element<'a, Message> {
        use desktop_auth::TeacherGradingAction::{Return, SaveDraft};
        let ready = !self.has_pending_request();
        let mut controls = column![].spacing(8);
        if maximum.is_some() {
            let save = work.grade.trim().parse::<f64>().ok().is_some_and(|grade| SaveDraft(grade).allowed(student, maximum));
            controls = controls.push(row![
                text_input("Points", &work.grade).padding(7).width(100)
                    .on_input_maybe(ready.then_some(Message::EditTeacherGrade)),
                button("Save draft grade").padding([7, 10]).style(outline_style)
                    .on_press_maybe((ready && save).then_some(Message::SaveTeacherGrade)),
            ].spacing(7).align_y(Alignment::Center));
        }
        if Return.allowed(student, maximum) {
            let ready = ready && grade_input_matches_saved(&work.grade, student);
            if work.confirming_return {
                let message = student.draft_grade.or(student.released_grade).map_or_else(
                    || "Return this work without a grade?".to_owned(),
                    |grade| format!("Return this work with {grade} points?"),
                );
                controls = controls.push(text(message).size(14)).push(row![
                    button("Return work").padding([7, 10]).style(primary_style)
                        .on_press_maybe(ready.then_some(Message::ReturnTeacherWork)),
                    button("Cancel").padding([7, 10]).style(quiet_style)
                        .on_press_maybe((!self.saving_teacher_grade).then_some(Message::ConfirmReturnTeacherWork(false))),
                ].spacing(6));
            } else {
                controls = controls.push(button("Return…").padding([7, 10]).style(outline_style)
                    .on_press_maybe(ready.then_some(Message::ConfirmReturnTeacherWork(true))));
            }
        }
        controls.into()
    }
}

pub(super) fn grade_input_matches_saved(input: &str, row: &desktop_auth::TeacherSubmission) -> bool {
    let input = input.trim();
    match row.draft_grade.or(row.released_grade) {
        Some(saved) => input.parse::<f64>().ok() == Some(saved),
        None => input.is_empty(),
    }
}

pub(super) fn next_editable_gradebook_student(grid: &desktop_auth::TeacherGradebook, item: i64, student: i64) -> Option<i64> {
    let mut ids = grid.students.iter().filter_map(|student| student.id.parse::<i64>().ok());
    ids.find(|id| *id == student)?;
    ids.find(|id| grid.cells.get(&(*id, item)).is_some_and(|cell| cell.can_edit()))
}

pub(super) fn grade_label(submission: &Submission) -> Option<String> {
    let grade = submission.grade.as_ref()?;
    Some(match grade.maximum {
        Some(maximum) => format!("Released grade: {} / {maximum}", grade.earned),
        None => format!("Released grade: {}", grade.earned),
    })
}
