use super::*;

#[derive(Clone)]
pub(in crate::desktop) enum NavigationMessage {
    #[cfg(target_os = "macos")]
    Capture(CaptureAction),
    Navigate(Screen),
    ShowClassroomLink,
    EditClassroomLink(String),
    OpenClassroomLink,
    ClassroomWorkLinkChecked(u64, ClassroomLink, Result<ClassroomLink, String>),
    CloseClassroomLink,
    CopyClassroomLink(ClassroomLink),
    OpenYourWork(i64),
    OpenGradebook(i64),
    OpenGradebookCell(i64, i64, i64),
    GradebookColumnPage(usize),
    SetTab(Tab),
    ToggleNavigation,
    Search(String),
    SortCourses,
    ToggleRubric(i64, i64),
    ToggleRubricCriterion(i64, i64, usize),
    SetRubricCriteria(i64, i64, bool),
    OpenItem(i64, String),
    CloseItem,
    SelectTopic(Option<String>),
    ToggleClassworkItem(i64, String),
    ToggleClassworkGroup(i64, ClassworkGroupId),
    SetClassworkGroups(i64, bool),
}

impl App {
    pub(super) fn update_navigation(&mut self, message: NavigationMessage) -> Task<Message> {
        match message {
            #[cfg(target_os = "macos")]
            NavigationMessage::Capture(action) => return self.update_capture(action),
            NavigationMessage::ShowClassroomLink if self.snapshot.is_some() && !self.verify_live_course_count => {
                self.classroom_link_input.get_or_insert_with(String::new);
            }
            NavigationMessage::EditClassroomLink(value) if value.len() <= 4096 && self.classroom_link_input.is_some() => {
                self.invalidate_classroom_link_check();
                self.classroom_link_input = Some(value);
            }
            NavigationMessage::CloseClassroomLink => {
                self.invalidate_classroom_link_check();
                self.classroom_link_input = None;
            }
            NavigationMessage::OpenClassroomLink if !self.has_pending_request() && !self.verify_live_course_count
                && !self.checking_classroom_link =>
            {
                if let Some(value) = &self.classroom_link_input {
                    if let Some(target) = ClassroomLink::parse(value) {
                        return self.open_classroom_link(target);
                    }
                    self.error = Some("Paste a supported classroom.google.com link.".into());
                }
            }
            NavigationMessage::ClassroomWorkLinkChecked(generation, target, result)
                if self.checking_classroom_link && generation == self.classroom_link_generation =>
            {
                self.checking_classroom_link = false;
                match result {
                    Ok(own) if own == target && self.snapshot.as_ref().is_some_and(|snapshot|
                        snapshot.courses.iter().any(|course| course.id == target.course())) =>
                    {
                        self.classroom_link_input = None;
                        return self.update(Message::Navigation(NavigationMessage::OpenYourWork(target.course())));
                    }
                    Ok(own) if own == target => self.error = Some("That class is no longer in the current course list.".into()),
                    Ok(_) => self.error = Some("This Your work link belongs to another Classroom account.".into()),
                    Err(error) => self.error = Some(error),
                }
            }
            NavigationMessage::CopyClassroomLink(target) => {
                if self.snapshot.as_ref().is_some_and(|snapshot| snapshot.courses.iter().any(|course| course.id == target.course()))
                    && let Some(url) = target.web_url()
                {
                    self.error = None;
                    self.notice = Some("Classroom link copied.".into());
                    return iced::clipboard::write(url);
                }
            }
            NavigationMessage::Navigate(screen) => {
                if self.screen != screen {
                    self.invalidate_classroom_link_check();
                    self.invalidate_people_email();
                    self.comments.comment_mentions_generation = self.comments.comment_mentions_generation.wrapping_add(1);
                    self.comments.comment_mentions = None;
                    if screen == Screen::CreateClass {
                        if !self.can_create_class() || self.has_pending_request() {
                            return Task::none();
                        }
                        self.new_class = desktop_auth::NewClass::default();
                    }
                    self.classwork.open_teacher_draft = None;
                    self.classwork.assignment_editor = None;
                    self.classwork.assignment_publication = None;
                    self.grading.teacher_work = None;
                    self.grading.teacher_feedback.clear();
                    self.grading.gradebook_page = 0;
                    self.grading.gradebook_student = None;
                    self.class_stream_settings = None;
                    self.people.pending_student_mute = None;
                    self.classwork.topic_manager = false;
                    self.classwork.topic_editor = None;
                    self.classwork.item_topic_editor = None;
                    self.grading.gradebook_edit_mode = false;
                    self.grading.gradebook_edit = None;
                    self.discard_submission_capture();
                    self.comments.confirming_comment_delete = None;
                    self.attachments.drive_picker = None;
                    self.opening_joined_course = None;
                    self.confirming_decline = None;
                    self.home.open_course_menu = None;
                    self.home.move_course_menu = None;
                    self.home.course_drag = None;
                    self.home.confirm_hide_course = None;
                    self.home.confirm_class_archive = None;
                    self.stream.confirming_stream_delete = None;
                    self.screen = screen;
                    self.confirming_unenroll = None;
                    self.student_work.confirming_question_answer = None;
                    self.attachments.pending_personal_copy = None;
                    if screen != Screen::Join {
                        self.class_code.clear();
                    }
                    self.tab = Tab::Stream;
                    self.classwork.selected_topic = None;
                    self.classwork.classwork_toggles.clear();
                    self.classwork.collapsed_classwork_groups.clear();
                    self.item_id = None;
                    if screen == Screen::Settings && !self.settings.loading_class_notifications && !self.settings.loading_email_notifications
                        && self.settings.saving_class_notifications.is_none() && self.settings.saving_email_notification.is_none()
                    {
                        self.reset_notifications();
                    }
                    self.retry_visible_reads_on_entry();
                }
                self.queued_page_load = true;
            }
            NavigationMessage::OpenYourWork(course_id) => {
                self.invalidate_classroom_link_check();
                self.screen = Screen::Course(course_id);
                self.tab = Tab::YourWork;
                self.student_work.overall_grades.remove(&course_id);
                self.student_work.work_filter = WorkFilter::All;
                self.classwork.selected_topic = None;
                self.classwork.classwork_toggles.clear();
                self.classwork.collapsed_classwork_groups.clear();
                self.item_id = None;
                self.retry_visible_reads_on_entry();
                self.queued_page_load = true;
            }
            NavigationMessage::OpenGradebook(course) if self.class_roles.get(&course) == Some(&CourseRole::Teacher) => {
                if self.screen != Screen::Course(course) {
                    let navigate = self.update(Message::Navigation(NavigationMessage::Navigate(Screen::Course(course))));
                    return Task::batch([navigate, self.update(Message::Navigation(NavigationMessage::SetTab(Tab::Grades)))]);
                }
                return self.update(Message::Navigation(NavigationMessage::SetTab(Tab::Grades)));
            }
            NavigationMessage::OpenGradebookCell(course, item, student) if self.screen == Screen::Course(course)
                && self.tab == Tab::Grades && self.class_roles.get(&course) == Some(&CourseRole::Teacher)
                && self.grading.gradebooks.get(&course).is_some_and(|grid| grid.cells.contains_key(&(student, item))) =>
            {
                let open = self.update(Message::Navigation(NavigationMessage::OpenItem(course, item.to_string())));
                self.grading.gradebook_student = Some((course, item, student));
                return open;
            }
            NavigationMessage::GradebookColumnPage(page) if self.tab == Tab::Grades && self.grading.gradebook_edit.is_none() => self.grading.gradebook_page = page,
            NavigationMessage::SetTab(tab) => {
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
                    self.attachments.drive_picker = None;
                    self.people.pending_student_mute = None;
                    self.classwork.topic_editor = None;
                    self.classwork.item_topic_editor = None;
                    self.classwork.topic_manager = false;
                    self.class_stream_settings = None;
                    self.grading.gradebook_edit_mode = false;
                    self.grading.gradebook_edit = None;
                    self.tab = tab;
                    self.stream.confirming_stream_delete = None;
                    self.retry_visible_reads_on_entry();
                }
                self.queued_page_load = true;
            },
            NavigationMessage::ToggleNavigation => self.sidebar = !self.sidebar,
            NavigationMessage::Search(search) => self.home.search = search,
            NavigationMessage::SortCourses => {
                self.home.alphabetical = !self.home.alphabetical;
                self.home.course_drag = None;
            }
            NavigationMessage::ToggleRubric(course, item) if self.visible_rubric(course, item).is_some() => {
                self.student_work.rubric_open = !self.student_work.rubric_open;
            }
            NavigationMessage::ToggleRubricCriterion(course, item, index)
                if self.student_work.rubric_open && self.visible_rubric(course, item).is_some_and(|rubric| index < rubric.criteria.len()) =>
            {
                if !self.student_work.open_criteria.remove(&index) {
                    self.student_work.open_criteria.insert(index);
                }
            }
            NavigationMessage::SetRubricCriteria(course, item, expanded) if self.student_work.rubric_open => {
                if let Some(rubric) = self.visible_rubric(course, item) {
                    self.student_work.open_criteria = if expanded { (0..rubric.criteria.len()).collect() } else { BTreeSet::new() };
                }
            }
            NavigationMessage::OpenItem(course, item) => {
                self.invalidate_classroom_link_check();
                self.comments.comment_mentions_generation = self.comments.comment_mentions_generation.wrapping_add(1);
                self.comments.comment_mentions = None;
                self.grading.teacher_work = None;
                self.grading.teacher_feedback.clear();
                self.grading.gradebook_student = None;
                self.grading.gradebook_edit = None;
                self.discard_submission_capture();
                self.comments.confirming_comment_delete = None;
                self.attachments.drive_picker = None;
                self.attachments.new_submission_document_open = false;
                self.stream.confirming_stream_delete = None;
                self.student_work.confirming_question_answer = None;
                self.student_work.rubric_open = false;
                self.student_work.open_criteria.clear();
                self.attachments.attachment_link.clear();
                self.attachments.submission_file = None;
                self.attachments.pending_remove_material = None;
                self.attachments.pending_personal_copy = None;
                self.attachments.personal_copy_checks.retain(|(id, _), _| *id != course);
                self.student_work.pending_submission = None;
                self.item_id = Some(item);
                self.previous_screen = Some(self.screen);
                self.screen = Screen::Item(course);
                self.retry_visible_reads_on_entry();
                self.queued_page_load = true;
            }
            NavigationMessage::CloseItem => {
                self.comments.comment_mentions_generation = self.comments.comment_mentions_generation.wrapping_add(1);
                self.comments.comment_mentions = None;
                self.grading.teacher_work = None;
                self.grading.teacher_feedback.clear();
                self.grading.gradebook_student = None;
                self.discard_submission_capture();
                self.comments.confirming_comment_delete = None;
                self.attachments.drive_picker = None;
                self.student_work.confirming_question_answer = None;
                self.attachments.attachment_link.clear();
                self.attachments.submission_file = None;
                self.attachments.pending_remove_material = None;
                self.attachments.pending_personal_copy = None;
                self.student_work.pending_submission = None;
                self.screen = self.previous_screen.take().unwrap_or(Screen::Home);
                self.item_id = None;
                self.retry_visible_reads_on_entry();
                self.queued_page_load = true;
            }
            NavigationMessage::SelectTopic(topic) if self.classwork.selected_topic != topic => {
                if let Some(topic) = &topic {
                    self.classwork.collapsed_classwork_groups.remove(&ClassworkGroupId::Topic(topic.clone()));
                }
                self.classwork.selected_topic = topic;
                self.classwork.classwork_toggles.clear();
            }
            NavigationMessage::ToggleClassworkItem(course, item)
                if self.screen == Screen::Course(course) && self.tab == Tab::Classwork =>
            {
                if !self.classwork.classwork_toggles.remove(&item) {
                    self.classwork.classwork_toggles.insert(item);
                }
            }
            NavigationMessage::ToggleClassworkGroup(course, group)
                if self.screen == Screen::Course(course) && self.tab == Tab::Classwork =>
            {
                if !self.classwork.collapsed_classwork_groups.remove(&group) {
                    self.classwork.collapsed_classwork_groups.insert(group);
                }
            }
            NavigationMessage::SetClassworkGroups(course, collapse)
                if self.screen == Screen::Course(course) && self.tab == Tab::Classwork =>
            {
                self.classwork.collapsed_classwork_groups.clear();
                if collapse && let Some(snapshot) = &self.snapshot {
                    self.classwork.collapsed_classwork_groups.extend(snapshot.classwork_sections(course, None)
                        .into_iter().map(|section| section.id));
                }
            }
            _ => {}
        }
        self.finish_update()
    }
}
