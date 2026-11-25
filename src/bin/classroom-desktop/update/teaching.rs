use super::*;

impl App {
    pub(super) fn update_teaching(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SetCoursePostingPolicy(policy) if !self.saving_stream_settings && self.tab == Tab::ClassSettings => {
                if let Some(editor) = self.class_stream_settings.as_mut().filter(|editor| editor.original.stream.posting.is_some()) {
                    editor.draft.stream.posting = Some(policy);
                }
            }
            Message::SetCourseStreamDisplay(display) if !self.saving_stream_settings && self.tab == Tab::ClassSettings => {
                if let Some(editor) = self.class_stream_settings.as_mut().filter(|editor| editor.original.stream.classwork.is_some()) {
                    editor.draft.stream.classwork = Some(display);
                }
            }
            Message::EditCourseTitle(value) if !self.saving_stream_settings && self.tab == Tab::ClassSettings => {
                if let Some(editor) = &mut self.class_stream_settings {
                    editor.draft.details.title = value;
                }
            }
            Message::EditCourseSection(value) if !self.saving_stream_settings && self.tab == Tab::ClassSettings => {
                if let Some(editor) = &mut self.class_stream_settings {
                    editor.draft.details.subtitle = value;
                }
            }
            Message::EditCourseRoom(value) if !self.saving_stream_settings && self.tab == Tab::ClassSettings => {
                if let Some(editor) = &mut self.class_stream_settings {
                    editor.draft.details.room = value;
                }
            }
            Message::EditCourseDescription(action) if !self.saving_stream_settings && self.tab == Tab::ClassSettings => {
                if let Some(editor) = &mut self.class_stream_settings {
                    editor.description.perform(action);
                }
            }
            Message::SaveCourseStreamSettings if !self.has_pending_request() && self.tab == Tab::ClassSettings => {
                let (Some(editor), Some(session)) = (self.class_stream_settings.as_ref(), self.session.clone()) else {
                    return Task::none();
                };
                let (course, original, draft) = (editor.course, editor.original.clone(), editor.changes());
                if !self.can_edit_class_stream_settings(course) || draft == original || !draft.details.valid() {
                    return Task::none();
                }
                self.saving_stream_settings = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::save_teacher_class_settings(&session, course, original, draft).await
                }, move |result| Message::CourseStreamSettingsSaved(course, result));
            }
            Message::CourseStreamSettingsSaved(course, result) => {
                self.saving_stream_settings = false;
                match result {
                    Ok(settings) => {
                        if let Some(entry) = self.snapshot.as_mut().and_then(|snapshot| snapshot.courses.iter_mut().find(|entry| entry.id == course)) {
                            entry.title = settings.details.title.clone();
                            entry.subtitle = settings.details.subtitle.clone();
                            entry.description = settings.details.description.clone();
                            entry.room = settings.details.room.clone();
                            entry.stream_classwork_display = settings.stream.classwork;
                        }
                        self.stream_access.remove(&course);
                        if self.screen == Screen::Course(course) && self.tab == Tab::ClassSettings {
                            self.class_stream_settings = Some(ClassStreamSettingsEditor::new(course, settings));
                        }
                        self.notice = Some("Class settings saved.".into());
                    }
                    Err(error) => {
                        if self.screen == Screen::Course(course) && self.tab == Tab::ClassSettings {
                            self.class_stream_settings = None;
                            self.queued_page_load = true;
                        }
                        self.error = Some(error);
                    }
                }
            }
            Message::ConfirmStudentMute(course, student, current)
                if self.can_change_student_mute(course, student, current) && !self.has_pending_request() =>
            {
                self.pending_student_mute = Some((course, student, current));
            }
            Message::CancelStudentMute => self.pending_student_mute = None,
            Message::ChangeStudentMute(course, student, current)
                if self.pending_student_mute == Some((course, student, current)) && !self.has_pending_request()
                    && self.can_change_student_mute(course, student, current) =>
            {
                let Some(session) = self.session.clone() else { return Task::none(); };
                self.pending_student_mute = None;
                self.changing_student_mute = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::set_student_muted(&session, course, student, current, !current).await
                }, move |result| Message::StudentMuteChanged(course, student, result));
            }
            Message::StudentMuteChanged(course, student, result) => {
                self.changing_student_mute = false;
                match result {
                    Ok(muted) => {
                        if let Some(states) = self.student_muting.get_mut(&course) {
                            states.insert(student, muted);
                        }
                        self.notice = Some(if muted { "Student muted." } else { "Student unmuted." }.into());
                    }
                    Err(error) => {
                        self.student_muting.remove(&course);
                        self.error = Some(error);
                        self.queued_page_load |= self.screen == Screen::Course(course) && self.tab == Tab::People;
                    }
                }
            }
            Message::ToggleTopicManager if !self.has_pending_request() && self.tab == Tab::Classwork => {
                self.topic_manager = !self.topic_manager;
            }
            Message::OpenTopicEditor(course, action) if self.can_manage_topics(course) && !self.has_pending_request() => {
                self.topic_editor = Some((course, action));
                self.error = None;
            }
            Message::EditTopicName(value) if !self.saving_topic && value.len() <= 1_600 => {
                if let Some((_, desktop_auth::TeacherTopicAction::Create(name)
                    | desktop_auth::TeacherTopicAction::Rename { name, .. })) = &mut self.topic_editor
                {
                    *name = value;
                }
            }
            Message::CloseTopicEditor if !self.saving_topic => self.topic_editor = None,
            Message::SaveTopicEditor if !self.has_pending_request() => {
                let (Some((course, action)), Some(session)) = (self.topic_editor.clone(), self.session.clone())
                    else { return Task::none(); };
                if !self.can_manage_topics(course) || !topic_action_valid(&action) { return Task::none(); }
                let removed = match &action {
                    desktop_auth::TeacherTopicAction::Delete(topic) => Some(topic.id.clone()),
                    _ => None,
                };
                self.saving_topic = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::mutate_teacher_topic(&session, course, action).await },
                    move |result| Message::TeacherTopicChanged(course, removed.clone(), result));
            }
            Message::TeacherTopicChanged(course, removed, result) => {
                self.saving_topic = false;
                match result {
                    Ok(topics) => {
                        self.apply_teacher_topics(course, topics, removed.as_deref());
                        if self.topic_editor.as_ref().is_some_and(|(id, _)| *id == course) {
                            self.topic_editor = None;
                        }
                        self.notice = Some(if removed.is_some() { "Topic deleted." } else { "Topic saved." }.into());
                    }
                    Err(error) => {
                        self.error = Some(error);
                        self.teacher_topics.remove(&course);
                        self.queued_page_load |= self.screen == Screen::Course(course) && self.tab == Tab::Classwork;
                    }
                }
            }
            Message::MoveTeacherTopic(course, id, direction) if self.can_manage_topics(course)
                && !self.has_pending_request() && self.topic_editor.is_none()
                && self.teacher_topics.get(&course).is_some_and(|topics| topics.iter().any(|topic| topic.id == id)) =>
            {
                let Some(session) = self.session.clone() else { return Task::none(); };
                self.saving_topic = true;
                self.error = None;
                return Task::perform(async move { desktop_auth::reorder_topic(&session, course, &id, direction).await },
                    move |result| Message::TeacherTopicChanged(course, None, result));
            }
            Message::OpenItemTopic(course, id, draft) if self.can_manage_topics(course)
                && !self.has_pending_request() && self.teacher_topics.contains_key(&course) =>
            {
                let item = if draft {
                    self.teacher_drafts.get(&course).and_then(|rows| rows.iter().find(|row| row.item.id == id)).map(|row| &row.item)
                } else {
                    self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(course))
                        .and_then(|items| items.iter().find(|item| item.id == id))
                };
                if let (Some(item), Ok(id)) = (item, id.parse::<i64>())
                    && item.kind != CourseworkKind::Announcement
                {
                    self.item_topic_editor = Some(ItemTopicEditor {
                        target: desktop_auth::TopicItemTarget { course, item: id, kind: item.kind,
                            draft, original: item.topic_id.clone() },
                        title: item.title.clone(), selected: item.topic_id.clone(),
                    });
                    self.error = None;
                }
            }
            Message::SelectItemTopic(topic) if !self.saving_topic => {
                if let Some(editor) = &mut self.item_topic_editor
                    && topic.id.as_ref().is_none_or(|id| self.teacher_topics.get(&editor.target.course)
                        .is_some_and(|topics| topics.iter().any(|topic| &topic.id == id)))
                {
                    editor.selected = topic.id;
                }
            }
            Message::CloseItemTopic if !self.saving_topic => self.item_topic_editor = None,
            Message::SaveItemTopic if !self.has_pending_request() => {
                let (Some(editor), Some(session)) = (&self.item_topic_editor, self.session.clone()) else { return Task::none(); };
                if !self.can_manage_topics(editor.target.course) || editor.selected == editor.target.original {
                    return Task::none();
                }
                let target = editor.target.clone();
                let result_target = target.clone();
                let selected = editor.selected.clone();
                self.saving_topic = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::change_classwork_topic(&session, target, selected).await },
                    move |result| Message::ItemTopicSaved(result_target.clone(), result));
            }
            Message::ItemTopicSaved(target, result) => {
                self.saving_topic = false;
                match result {
                    Ok(topic) => {
                        self.apply_item_topic(&target, topic);
                        if self.item_topic_editor.as_ref().is_some_and(|editor| editor.target == target) {
                            self.item_topic_editor = None;
                        }
                        self.notice = Some("Classwork topic updated.".into());
                        if !target.draft {
                            return self.update(Message::LoadCoursework(target.course));
                        }
                    }
                    Err(error) => {
                        self.error = Some(error);
                        if self.item_topic_editor.as_ref().is_some_and(|editor| editor.target == target) {
                            self.item_topic_editor = None;
                        }
                        let request = if target.draft { Message::LoadDetail(DetailRead::TeacherDrafts(target.course)) }
                            else { Message::LoadCoursework(target.course) };
                        return self.update(request);
                    }
                }
            }
            Message::ReorderClasswork(course, id, direction) if self.can_manage_topics(course)
                && !self.has_pending_request() && self.item_topic_editor.is_none() =>
            {
                let (Some(snapshot), Some(session)) = (&self.snapshot, self.session.clone()) else { return Task::none(); };
                let Some(section) = snapshot.classwork_sections(course, None).into_iter().find(|section|
                    section.confirmed_order && section.items.iter().any(|item| item.id == id))
                    else { return Task::none(); };
                let ClassworkGroupId::Topic(topic) = section.id else { return Task::none(); };
                let order = section.items.into_iter().map(|item| item.id.clone()).collect::<Vec<_>>();
                self.saving_topic = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::reorder_classwork(&session, course, &id, &topic, &order, direction).await
                }, move |result| Message::ClassworkReordered(course, result.map(Arc::new)));
            }
            Message::ClassworkReordered(course, result) => {
                self.saving_topic = false;
                match result {
                    Ok(contents) => {
                        self.notice = Some("Classwork order updated.".into());
                        return self.update(Message::CourseworkLoaded(course, Ok(contents)));
                    }
                    Err(error) => {
                        let reload = self.update(Message::LoadCoursework(course));
                        self.error = Some(error);
                        return reload;
                    }
                }
            }
            Message::GradebookEditMode(enabled) if !self.has_pending_request() => {
                if let Screen::Course(course) = self.screen
                    && self.can_edit_gradebook(course)
                {
                    self.gradebook_edit_mode = enabled;
                    self.gradebook_edit = None;
                }
            }
            Message::EditGradebookCell(course, item, student) if self.gradebook_edit_mode
                && self.gradebook_edit.is_none() && !self.has_pending_request() && self.can_edit_gradebook(course) =>
            {
                return self.begin_gradebook_edit(course, item, student);
            }
            Message::GradebookGradeInput(input) if !self.saving_teacher_grade && input.len() <= 20 => {
                if let Some(edit) = &mut self.gradebook_edit {
                    edit.input = input;
                }
            }
            Message::CancelGradebookEdit if !self.saving_teacher_grade => self.gradebook_edit = None,
            Message::SaveGradebookEdit(advance) if !self.has_pending_request() => {
                let Some(edit) = &self.gradebook_edit else { return Task::none(); };
                if !self.can_edit_gradebook(edit.course) { return Task::none(); }
                let (course, item) = (edit.course, edit.item);
                let expected = edit.expected.clone();
                let (student, submission) = (expected.student_id, expected.submission_id);
                if advance && grade_input_matches_saved(&edit.input, &expected) {
                    self.gradebook_edit = None;
                    if let Some(next) = self.gradebooks.get(&course)
                        .and_then(|grid| next_editable_gradebook_student(grid, item, student))
                    {
                        return self.begin_gradebook_edit(course, item, next);
                    }
                    return Task::none();
                }
                let (Some(grade), Some(session)) = (edit.grade(self.gradebook_maximum(course, item)), self.session.clone())
                    else { return Task::none(); };
                self.saving_teacher_grade = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::write_teacher_grade(&session, course, item, expected,
                        desktop_auth::TeacherGradingAction::SaveDraft(grade)).await
                }, move |result| Message::GradebookEditSaved(course, item, student, submission, grade, advance, result));
            }
            Message::GradebookEditSaved(course, item, student, submission, grade, advance, result) => {
                self.saving_teacher_grade = false;
                let grid = self.gradebooks.get_mut(&course);
                match result {
                    Ok(()) => {
                        if let Some(cell) = grid.and_then(|grid| grid.cells.get_mut(&(student, item)))
                            && cell.submission_id == submission
                        {
                            cell.draft_grade = Some(grade);
                        }
                        if self.gradebook_edit.as_ref().is_some_and(|edit| edit.course == course && edit.item == item
                            && edit.expected.student_id == student && edit.expected.submission_id == submission)
                        {
                            self.gradebook_edit = None;
                            self.notice = Some("Draft grade saved.".into());
                            if advance && let Some(next) = self.gradebooks.get(&course)
                                .and_then(|grid| next_editable_gradebook_student(grid, item, student))
                            {
                                return self.begin_gradebook_edit(course, item, next);
                            }
                        }
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            #[cfg(target_os = "macos")]
            Message::ExportGrades(course, item) if self.can_read_pages() && !self.has_pending_request()
                && self.class_roles.get(&course) == Some(&CourseRole::Teacher) =>
            {
                return self.export_grades(course, item);
            }
            #[cfg(target_os = "macos")]
            Message::GradesExported(result) => {
                self.exporting_grades = false;
                match result {
                    Ok(true) => self.notice = Some("Grade CSV saved.".into()),
                    Ok(false) => {}
                    Err(error) => self.error = Some(error),
                }
            }
            Message::ToggleTeacherDraft(course, item) if self.screen == Screen::Course(course)
                && self.class_roles.get(&course) == Some(&CourseRole::Teacher)
                && self.teacher_drafts.get(&course).is_some_and(|drafts| drafts.iter().any(|draft| draft.item.id == item)) =>
            {
                let key = (course, item);
                self.open_teacher_draft = (self.open_teacher_draft.as_ref() != Some(&key)).then_some(key);
            }
            Message::OpenAssignmentEditor(course, item) if self.can_edit_assignment_drafts(course)
                && !self.has_pending_request() && self.assignment_editor.is_none() && self.assignment_publication.is_none() =>
            {
                let original = if let Some(id) = item {
                    let Some(draft) = self.teacher_drafts.get(&course).and_then(|drafts| drafts.iter().find(|draft|
                        draft.item.id == id && draft.item.kind == CourseworkKind::Assignment
                        && draft.publication == desktop_auth::DraftPublication::Draft)) else { return Task::none(); };
                    Some(draft.clone())
                } else {
                    None
                };
                self.assignment_editor = Some(AssignmentEditor::new(course, original));
                self.error = None;
            }
            Message::EditAssignmentTitle(title) if !self.saving_assignment_draft && title.chars().count() <= 3_000 => {
                if let Some(editor) = &mut self.assignment_editor {
                    editor.title = title;
                }
            }
            Message::EditAssignmentInstructions(action) if !self.saving_assignment_draft => {
                if let Some(editor) = self.assignment_editor.as_mut().filter(|editor| editor.instructions_editable()) {
                    editor.instructions.perform(action);
                }
            }
            Message::EditAssignmentDueDate(value) if !self.saving_assignment_draft && value.len() <= 16 => {
                if let Some(editor) = &mut self.assignment_editor {
                    editor.settings.due_date = value;
                }
            }
            Message::EditAssignmentDueTime(value) if !self.saving_assignment_draft && value.len() <= 12 => {
                if let Some(editor) = &mut self.assignment_editor {
                    editor.settings.due_time = value;
                }
            }
            Message::EditAssignmentPoints(value) if !self.saving_assignment_draft && value.len() <= 32 => {
                if let Some(editor) = &mut self.assignment_editor {
                    editor.settings.points = value;
                }
            }
            Message::CloseAssignmentEditor if !self.saving_assignment_draft => self.assignment_editor = None,
            Message::SaveAssignmentDraft if !self.has_pending_request() => {
                let Some(editor) = self.assignment_editor.as_ref().filter(|editor| self.can_edit_assignment_drafts(editor.course))
                    else { return Task::none() };
                let (course, original, title, instructions) =
                    (editor.course, editor.original.clone(), editor.title.clone(), editor.instructions_text());
                if !desktop_auth::assignment_draft_text_valid(&title, &instructions) { return Task::none(); }
                let fields = match editor.field_changes() {
                    Ok(fields) => fields,
                    Err(error) => {
                        self.error = Some(error);
                        return Task::none();
                    }
                };
                let Some(session) = self.session.clone() else { return Task::none() };
                self.saving_assignment_draft = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::save_assignment_draft_with_fields(&session, course, original, title, instructions, fields).await
                }, move |result| Message::AssignmentDraftSaved(course, result));
            }
            Message::AssignmentDraftSaved(course, result) => {
                self.saving_assignment_draft = false;
                match result {
                    Ok(draft) => {
                        self.assignment_editor = None;
                        self.open_teacher_draft = Some((course, draft.item.id));
                        self.teacher_drafts.remove(&course);
                        self.notice = Some("Assignment draft saved.".into());
                        self.queued_page_load |= self.screen == Screen::Course(course);
                    }
                    Err(error) => {
                        self.error = Some(error);
                        self.teacher_drafts.remove(&course);
                        self.queued_page_load |= self.screen == Screen::Course(course);
                    }
                }
            }
            Message::OpenAssignmentPublication(course, id, scheduling) if self.can_edit_assignment_drafts(course)
                && !self.has_pending_request() && self.assignment_editor.is_none() && self.assignment_publication.is_none() =>
            {
                use desktop_auth::DraftPublication;
                let Some(draft) = self.teacher_drafts.get(&course).and_then(|drafts| drafts.iter().find(|draft|
                    draft.item.id == id && draft.item.kind == CourseworkKind::Assignment))
                    else { return Task::none(); };
                let schedule = match (scheduling, draft.publication) {
                    (false, DraftPublication::Draft) => None,
                    (true, DraftPublication::Draft | DraftPublication::Scheduled(None)) => Some(String::new()),
                    (true, DraftPublication::Scheduled(Some(time))) => Some(DateTime::from_timestamp_millis(time)
                        .map_or_else(String::new, |date| date.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string())),
                    _ => return Task::none(),
                };
                self.assignment_publication = Some(AssignmentPublicationPrompt { draft: draft.clone(), schedule });
                self.error = None;
            }
            Message::EditAssignmentSchedule(value) if !self.saving_assignment_draft && value.len() <= 32 => {
                if let Some(schedule) = self.assignment_publication.as_mut().and_then(|prompt| prompt.schedule.as_mut()) {
                    *schedule = value;
                }
            }
            Message::CloseAssignmentPublication if !self.saving_assignment_draft => self.assignment_publication = None,
            Message::FilterStudentWork(filter) => {
                if let Some(work) = &mut self.teacher_work {
                    work.filter = filter;
                    work.select(None);
                }
            }
            Message::SelectTeacherStudent(student) => {
                if let Some(work) = &mut self.teacher_work
                    && work.students.iter().any(|row| row.student_id == student && work.filter.includes(row))
                {
                    work.select((work.selected != Some(student)).then_some(student));
                    self.queued_page_load = true;
                }
            }
            Message::EditTeacherGrade(value) if !self.saving_teacher_grade && value.len() <= 20 => {
                if let Some(work) = &mut self.teacher_work {
                    work.grade = value;
                    work.confirming_return = false;
                }
            }
            Message::ConfirmReturnTeacherWork(confirm) if !self.saving_teacher_grade => {
                if let Some(work) = &mut self.teacher_work {
                    work.confirming_return = confirm;
                }
            }
            Message::SaveTeacherGrade | Message::ReturnTeacherWork if !self.has_pending_request() => {
                let Some(work) = &self.teacher_work else { return Task::none(); };
                let (course, item) = (work.course, work.item);
                if !self.can_read_teacher_work(course, item) || !self.can_grade_teacher_work(course) {
                    return Task::none();
                }
                let (Some(row), Some(session)) = (work.selected_row().cloned(), self.session.clone()) else { return Task::none(); };
                let action = if matches!(message, Message::SaveTeacherGrade) {
                    let Ok(value) = work.grade.trim().parse::<f64>() else { return Task::none(); };
                    desktop_auth::TeacherGradingAction::SaveDraft(value)
                } else if work.confirming_return && grade_input_matches_saved(&work.grade, &row) {
                    desktop_auth::TeacherGradingAction::Return
                } else {
                    return Task::none();
                };
                let maximum = self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(course))
                    .and_then(|items| items.iter().find(|entry| entry.id.parse::<i64>().ok() == Some(item)))
                    .and_then(|entry| entry.maximum_points);
                if !action.allowed(&row, maximum) { return Task::none(); }
                self.saving_teacher_grade = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::write_teacher_grade(&session, course, item, row, action).await
                }, move |result| Message::TeacherGradeWritten(course, item, action, result));
            }
            Message::TeacherGradeWritten(course, item, action, result) => {
                self.saving_teacher_grade = false;
                self.gradebooks.remove(&course);
                if self.can_read_teacher_work(course, item) {
                    if let Some(work) = &mut self.teacher_work {
                        work.confirming_return = false;
                    }
                    match result {
                        Ok(()) => {
                            self.notice = Some(match action {
                                desktop_auth::TeacherGradingAction::SaveDraft(_) => "Draft grade saved.",
                                desktop_auth::TeacherGradingAction::Return => "Work returned.",
                            }.into());
                        }
                        Err(error) => self.error = Some(error),
                    }
                    return self.update(Message::LoadDetail(DetailRead::TeacherSubmissions(course, item)));
                }
            }
            Message::SelectBulkReturn(student, selected) if !self.has_pending_request() => {
                if let Some(work) = &mut self.teacher_work
                    && work.returnable().any(|row| row.student_id == student)
                {
                    if selected { work.bulk_selected.insert(student); } else { work.bulk_selected.remove(&student); }
                    work.bulk_confirming = false;
                }
            }
            Message::SelectVisibleReturns(selected) if !self.has_pending_request() => {
                if let Some(work) = &mut self.teacher_work {
                    let visible: BTreeSet<_> = work.returnable().filter(|row| work.filter.includes(row))
                        .map(|row| row.student_id).collect();
                    if selected {
                        work.bulk_selected.extend(visible);
                    } else {
                        work.bulk_selected.retain(|id| !visible.contains(id));
                    }
                    work.bulk_confirming = false;
                }
            }
            Message::ClearBulkReturns if !self.has_pending_request() => {
                if let Some(work) = &mut self.teacher_work {
                    work.bulk_selected.clear();
                    work.bulk_confirming = false;
                }
            }
            Message::ConfirmBulkReturn(confirm) if !self.has_pending_request() => {
                if let Some(work) = &mut self.teacher_work {
                    let has_selected = work.selected_returns().next().is_some();
                    work.bulk_confirming = confirm && has_selected;
                    work.confirming_return = false;
                }
            }
            Message::ReturnSelectedTeacherWork if !self.has_pending_request() => {
                let Some(work) = &self.teacher_work else { return Task::none(); };
                let (course, item) = (work.course, work.item);
                if !work.bulk_confirming || !self.can_read_teacher_work(course, item) || !self.can_grade_teacher_work(course) {
                    return Task::none();
                }
                let selected: Vec<_> = work.selected_returns().cloned().collect();
                let count = selected.len();
                if count == 0 || count != work.bulk_selected.len() { return Task::none(); }
                let Some(session) = self.session.clone() else { return Task::none(); };
                self.saving_teacher_grade = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::return_teacher_submissions(&session, course, item, selected).await
                }, move |result| Message::BulkTeacherWorkReturned(course, item, count, result));
            }
            Message::BulkTeacherWorkReturned(course, item, requested, result) => {
                self.saving_teacher_grade = false;
                self.gradebooks.remove(&course);
                if self.can_read_teacher_work(course, item) {
                    if let Some(work) = &mut self.teacher_work {
                        work.bulk_selected.clear();
                        work.bulk_confirming = false;
                    }
                    match result {
                        Ok(confirmed) if confirmed == requested => self.notice = Some(format!("{confirmed} submissions returned.")),
                        Ok(confirmed) => self.error = Some(format!("Classroom confirmed {confirmed} of {requested} returns. Review the updated submissions before retrying.")),
                        Err(error) => self.error = Some(error),
                    }
                    return self.update(Message::LoadDetail(DetailRead::TeacherSubmissions(course, item)));
                }
            }
            Message::EditTeacherFeedback(key, text) if text.len() <= 10_000
                && self.selected_teacher_feedback_key() == Some(key) && !self.posting_comment =>
            {
                self.teacher_feedback.entry(key).or_default().draft = text;
            }
            Message::SendTeacherFeedback(key) if !self.has_pending_request()
                && self.selected_teacher_feedback_key() == Some(key) && self.can_grade_teacher_work(key.course) =>
            {
                let (Some(session), Some(thread)) = (self.session.clone(), self.teacher_feedback.get(&key)) else {
                    return Task::none();
                };
                if thread.unconfirmed || thread.comments.is_none() || thread.draft.trim().is_empty() {
                    return Task::none();
                }
                let draft = thread.draft.clone();
                self.posting_comment = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move { desktop_auth::post_teacher_private_feedback(&session, key, draft).await },
                    move |result| Message::TeacherFeedbackSent(key, result));
            }
            Message::TeacherFeedbackSent(key, result) => {
                self.posting_comment = false;
                if let Some(thread) = self.teacher_feedback.get_mut(&key) {
                    thread.comments = None;
                    self.page_errors.remove(&PageRead::Detail(DetailRead::TeacherFeedback(key)));
                    match result {
                        Ok(()) => {
                            thread.draft.clear();
                            self.notice = Some("Private feedback sent.".into());
                        }
                        Err(error) => {
                            thread.unconfirmed = true;
                            self.error = Some(error);
                        }
                    }
                }
                self.queued_page_load |= self.selected_teacher_feedback_key().is_some();
            }
            Message::ConfirmAssignmentPublication if !self.has_pending_request() => {
                let Some(prompt) = &self.assignment_publication else { return Task::none(); };
                let course = prompt.draft.item.course_id;
                if !self.can_edit_assignment_drafts(course) { return Task::none(); }
                let (Some(session), Some(action)) = (self.session.clone(), prompt.action()) else { return Task::none(); };
                let draft = prompt.draft.clone();
                self.saving_assignment_draft = true;
                self.error = None;
                self.notice = None;
                return Task::perform(async move {
                    desktop_auth::change_assignment_publication(&session, course, draft, action).await
                }, move |result| Message::AssignmentPublicationChanged(course, action, result));
            }
            Message::AssignmentPublicationChanged(course, action, result) => {
                self.saving_assignment_draft = false;
                self.teacher_drafts.remove(&course);
                self.queued_page_load |= self.screen == Screen::Course(course);
                let published = action == desktop_auth::AssignmentPublication::Publish;
                match result {
                    Ok(()) => {
                        self.assignment_publication = None;
                        self.notice = Some(if published { "Assignment published." } else { "Assignment scheduled." }.into());
                    }
                    Err(error) => self.error = Some(error),
                }
                if published {
                    return self.update(Message::LoadCoursework(course));
                }
            }
            _ => {}
        }
        self.finish_update()
    }
}
