use super::*;

impl App {
    pub(super) fn update_navigation(&mut self, message: Message) -> Task<Message> {
        match message {
            #[cfg(target_os = "macos")]
            Message::Capture(action) => return self.update_capture(action),
            Message::ShowClassroomLink if self.snapshot.is_some() && !self.verify_live_course_count => {
                self.classroom_link_input.get_or_insert_with(String::new);
            }
            Message::EditClassroomLink(value) if value.len() <= 4096 && self.classroom_link_input.is_some() => {
                self.invalidate_classroom_link_check();
                self.classroom_link_input = Some(value);
            }
            Message::CloseClassroomLink => {
                self.invalidate_classroom_link_check();
                self.classroom_link_input = None;
            }
            Message::OpenClassroomLink if !self.has_pending_request() && !self.verify_live_course_count
                && !self.checking_classroom_link =>
            {
                if let Some(value) = &self.classroom_link_input {
                    if let Some(target) = ClassroomLink::parse(value) {
                        return self.open_classroom_link(target);
                    }
                    self.error = Some("Paste a supported classroom.google.com link.".into());
                }
            }
            Message::ClassroomWorkLinkChecked(generation, target, result)
                if self.checking_classroom_link && generation == self.classroom_link_generation =>
            {
                self.checking_classroom_link = false;
                match result {
                    Ok(own) if own == target && self.snapshot.as_ref().is_some_and(|snapshot|
                        snapshot.courses.iter().any(|course| course.id == target.course())) =>
                    {
                        self.classroom_link_input = None;
                        return self.update(Message::OpenYourWork(target.course()));
                    }
                    Ok(own) if own == target => self.error = Some("That class is no longer in the current course list.".into()),
                    Ok(_) => self.error = Some("This Your work link belongs to another Classroom account.".into()),
                    Err(error) => self.error = Some(error),
                }
            }
            Message::CopyClassroomLink(target) => {
                if self.snapshot.as_ref().is_some_and(|snapshot| snapshot.courses.iter().any(|course| course.id == target.course()))
                    && let Some(url) = target.web_url()
                {
                    self.error = None;
                    self.notice = Some("Classroom link copied.".into());
                    return iced::clipboard::write(url);
                }
            }
            Message::Navigate(screen) => {
                if self.screen != screen {
                    self.invalidate_classroom_link_check();
                    self.invalidate_people_email();
                    self.comment_mentions_generation = self.comment_mentions_generation.wrapping_add(1);
                    self.comment_mentions = None;
                    if screen == Screen::CreateClass {
                        if !self.can_create_class() || self.has_pending_request() {
                            return Task::none();
                        }
                        self.new_class = desktop_auth::NewClass::default();
                    }
                    self.open_teacher_draft = None;
                    self.assignment_editor = None;
                    self.assignment_publication = None;
                    self.teacher_work = None;
                    self.teacher_feedback.clear();
                    self.gradebook_page = 0;
                    self.gradebook_student = None;
                    self.class_stream_settings = None;
                    self.pending_student_mute = None;
                    self.topic_manager = false;
                    self.topic_editor = None;
                    self.item_topic_editor = None;
                    self.gradebook_edit_mode = false;
                    self.gradebook_edit = None;
                    self.discard_submission_capture();
                    self.confirming_comment_delete = None;
                    self.drive_picker = None;
                    self.opening_joined_course = None;
                    self.confirming_decline = None;
                    self.open_course_menu = None;
                    self.move_course_menu = None;
                    self.course_drag = None;
                    self.confirm_hide_course = None;
                    self.confirm_class_archive = None;
                    self.confirming_stream_delete = None;
                    self.screen = screen;
                    self.confirming_unenroll = None;
                    self.confirming_question_answer = None;
                    self.pending_personal_copy = None;
                    if screen != Screen::Join {
                        self.class_code.clear();
                    }
                    self.tab = Tab::Stream;
                    self.selected_topic = None;
                    self.classwork_toggles.clear();
                    self.collapsed_classwork_groups.clear();
                    self.item_id = None;
                    if screen == Screen::Settings && !self.loading_class_notifications && !self.loading_email_notifications
                        && self.saving_class_notifications.is_none() && self.saving_email_notification.is_none()
                    {
                        self.reset_notifications();
                    }
                    self.retry_visible_reads_on_entry();
                }
                self.queued_page_load = true;
            }
            Message::OpenYourWork(course_id) => {
                self.invalidate_classroom_link_check();
                self.screen = Screen::Course(course_id);
                self.tab = Tab::YourWork;
                self.overall_grades.remove(&course_id);
                self.work_filter = WorkFilter::All;
                self.selected_topic = None;
                self.classwork_toggles.clear();
                self.collapsed_classwork_groups.clear();
                self.item_id = None;
                self.retry_visible_reads_on_entry();
                self.queued_page_load = true;
            }
            Message::OpenGradebook(course) if self.class_roles.get(&course) == Some(&CourseRole::Teacher) => {
                if self.screen != Screen::Course(course) {
                    let navigate = self.update(Message::Navigate(Screen::Course(course)));
                    return Task::batch([navigate, self.update(Message::SetTab(Tab::Grades))]);
                }
                return self.update(Message::SetTab(Tab::Grades));
            }
            Message::OpenGradebookCell(course, item, student) if self.screen == Screen::Course(course)
                && self.tab == Tab::Grades && self.class_roles.get(&course) == Some(&CourseRole::Teacher)
                && self.gradebooks.get(&course).is_some_and(|grid| grid.cells.contains_key(&(student, item))) =>
            {
                let open = self.update(Message::OpenItem(course, item.to_string()));
                self.gradebook_student = Some((course, item, student));
                return open;
            }
            Message::GradebookColumnPage(page) if self.tab == Tab::Grades && self.gradebook_edit.is_none() => self.gradebook_page = page,
            Message::SetTab(tab) => {
                if tab == Tab::Grades && !matches!(self.screen, Screen::Course(course)
                    if self.class_roles.get(&course) == Some(&CourseRole::Teacher))
                {
                    return Task::none();
                }
                if tab == Tab::ClassSettings && !matches!(self.screen, Screen::Course(course)
                    if self.can_edit_class_stream_settings(course))
                {
                    return Task::none();
                }
                if self.tab != tab {
                    self.invalidate_classroom_link_check();
                    self.invalidate_people_email();
                    self.drive_picker = None;
                    self.pending_student_mute = None;
                    self.topic_editor = None;
                    self.item_topic_editor = None;
                    self.topic_manager = false;
                    self.class_stream_settings = None;
                    self.gradebook_edit_mode = false;
                    self.gradebook_edit = None;
                    self.tab = tab;
                    self.confirming_stream_delete = None;
                    self.retry_visible_reads_on_entry();
                }
                self.queued_page_load = true;
            },
            Message::ToggleNavigation => self.sidebar = !self.sidebar,
            Message::Search(search) => self.search = search,
            Message::SortCourses => {
                self.alphabetical = !self.alphabetical;
                self.course_drag = None;
            }
            Message::ToggleRubric(course, item) if self.visible_rubric(course, item).is_some() => {
                self.rubric_open = !self.rubric_open;
            }
            Message::ToggleRubricCriterion(course, item, index)
                if self.rubric_open && self.visible_rubric(course, item).is_some_and(|rubric| index < rubric.criteria.len()) =>
            {
                if !self.open_criteria.remove(&index) {
                    self.open_criteria.insert(index);
                }
            }
            Message::SetRubricCriteria(course, item, expanded) if self.rubric_open => {
                if let Some(rubric) = self.visible_rubric(course, item) {
                    self.open_criteria = if expanded { (0..rubric.criteria.len()).collect() } else { BTreeSet::new() };
                }
            }
            Message::OpenItem(course, item) => {
                self.invalidate_classroom_link_check();
                self.comment_mentions_generation = self.comment_mentions_generation.wrapping_add(1);
                self.comment_mentions = None;
                self.teacher_work = None;
                self.teacher_feedback.clear();
                self.gradebook_student = None;
                self.gradebook_edit = None;
                self.discard_submission_capture();
                self.confirming_comment_delete = None;
                self.drive_picker = None;
                self.new_submission_document_open = false;
                self.confirming_stream_delete = None;
                self.confirming_question_answer = None;
                self.rubric_open = false;
                self.open_criteria.clear();
                self.attachment_link.clear();
                self.submission_file = None;
                self.pending_remove_material = None;
                self.pending_personal_copy = None;
                self.personal_copy_checks.retain(|(id, _), _| *id != course);
                self.pending_submission = None;
                self.item_id = Some(item);
                self.previous_screen = Some(self.screen);
                self.screen = Screen::Item(course);
                self.retry_visible_reads_on_entry();
                self.queued_page_load = true;
            }
            Message::CloseItem => {
                self.comment_mentions_generation = self.comment_mentions_generation.wrapping_add(1);
                self.comment_mentions = None;
                self.teacher_work = None;
                self.teacher_feedback.clear();
                self.gradebook_student = None;
                self.discard_submission_capture();
                self.confirming_comment_delete = None;
                self.drive_picker = None;
                self.confirming_question_answer = None;
                self.attachment_link.clear();
                self.submission_file = None;
                self.pending_remove_material = None;
                self.pending_personal_copy = None;
                self.pending_submission = None;
                self.screen = self.previous_screen.take().unwrap_or(Screen::Home);
                self.item_id = None;
                self.retry_visible_reads_on_entry();
                self.queued_page_load = true;
            }
            Message::SelectTopic(topic) if self.selected_topic != topic => {
                if let Some(topic) = &topic {
                    self.collapsed_classwork_groups.remove(&ClassworkGroupId::Topic(topic.clone()));
                }
                self.selected_topic = topic;
                self.classwork_toggles.clear();
            }
            Message::ToggleClassworkItem(course, item)
                if self.screen == Screen::Course(course) && self.tab == Tab::Classwork =>
            {
                if !self.classwork_toggles.remove(&item) {
                    self.classwork_toggles.insert(item);
                }
            }
            Message::ToggleClassworkGroup(course, group)
                if self.screen == Screen::Course(course) && self.tab == Tab::Classwork =>
            {
                if !self.collapsed_classwork_groups.remove(&group) {
                    self.collapsed_classwork_groups.insert(group);
                }
            }
            Message::SetClassworkGroups(course, collapse)
                if self.screen == Screen::Course(course) && self.tab == Tab::Classwork =>
            {
                self.collapsed_classwork_groups.clear();
                if collapse && let Some(snapshot) = &self.snapshot {
                    self.collapsed_classwork_groups.extend(snapshot.classwork_sections(course, None)
                        .into_iter().map(|section| section.id));
                }
            }
            _ => {}
        }
        self.finish_update()
    }
}
