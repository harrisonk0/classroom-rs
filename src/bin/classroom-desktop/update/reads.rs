use super::*;

impl App {
    pub(super) fn update_reads(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::LoadCoursework(id) if !self.verify_live_course_count && !self.has_pending_request() => {
                if let Some(session) = self.session.clone()
                    && self.snapshot.as_ref().is_some_and(|snapshot| {
                        snapshot.courses.iter().any(|course| course.id == id)
                    })
                {
                    self.loading_coursework = Some(id);
                    self.page_errors.remove(&PageRead::Coursework(id));
                    self.error = None;
                    self.notice = None;
                    let coursework = Task::perform(
                        async move { desktop_auth::load_coursework(&session, id).await },
                        move |result| Message::CourseworkLoaded(id, result),
                    );
                    let access = self.missing_stream_access(id).then_some(Message::LoadStreamAccess(id));
                    return Task::batch([coursework, access.map_or_else(Task::none, |message| self.update(message))]);
                }
            }
            Message::LoadStreamAccess(id) if self.can_read_visible_stream_access(id) && !self.loading_stream_access.contains(&id) => {
                let Some(session) = self.session.clone() else { return Task::none() };
                self.page_errors.remove(&PageRead::StreamAccess(id));
                self.loading_stream_access.insert(id);
                let generation = self.stream_access_generation;
                let access = Task::perform(async move { desktop_auth::load_stream_permission(&session, id).await },
                    move |result| Message::StreamAccessLoaded(generation, id, result));
                if self.screen == Screen::Item(id) {
                    self.queued_page_load = true;
                    return Task::batch([access, self.dequeue_page_load().map_or_else(Task::none, |message| self.update(message))]);
                }
                return access;
            }
            Message::StreamAccessLoaded(generation, id, result) if generation == self.stream_access_generation => {
                self.loading_stream_access.remove(&id);
                match result {
                    Ok((user, access)) => {
                        self.stream_viewer_id = Some(user);
                        self.stream_access.insert(id, access);
                    }
                    Err(error) => { self.page_errors.insert(PageRead::StreamAccess(id), error); }
                }
            }
            Message::LoadAssignments if self.can_read_pages() && !self.has_pending_request()
                && self.assignment_batch.is_none() && self.screen.uses_assignment_overview() =>
            {
                self.page_errors.remove(&PageRead::Assignments);
                let remaining = self.assignment_courses();
                self.assignments_loaded = remaining.is_empty();
                if !remaining.is_empty()
                    && let Some(session) = self.session.clone()
                {
                    self.assignment_generation = self.assignment_generation.wrapping_add(1);
                    let generation = self.assignment_generation;
                    let updates = desktop_auth::assignment_updates(session, remaining.iter().copied().collect());
                    let (task, handle) = Task::run(updates, move |result| {
                        Message::AssignmentLoaded(generation, result)
                    }).abortable();
                    self.assignment_batch = Some(AssignmentBatch {
                        handle,
                        remaining,
                        failed_classwork: 0,
                        failed_status: 0,
                    });
                    return task;
                }
            }
            Message::AssignmentLoaded(generation, result) if generation == self.assignment_generation => {
                self.apply_assignment_update(result);
            }
            Message::CourseworkLoaded(id, result) => {
                self.loading_coursework = None;
                if std::env::var_os("CLASSROOM_READ_ONLY_DIAGNOSTICS").is_some() {
                    println!("Classwork read completed; success={}", result.is_ok());
                }
                if self.screen == Screen::Item(id)
                    || (self.screen == Screen::Course(id) && self.tab != Tab::People)
                {
                    self.queued_page_load = true;
                }
                match result {
                    Ok(contents) => {
                        self.personal_copy_generation = self.personal_copy_generation.wrapping_add(1);
                        self.personal_copy_checks.retain(|(course, _), _| *course != id);
                        if self.pending_personal_copy.as_ref().is_some_and(|(course, _, _)| *course == id) {
                            self.pending_personal_copy = None;
                        }
                        self.gradebooks.remove(&id);
                        if let Some(snapshot) = &mut self.snapshot {
                            if self.screen == Screen::Course(id)
                                && let Some(topic) = self.selected_topic.as_ref()
                                && contents.topics.as_ref().is_some_and(|topics| !topics.iter().any(|entry| &entry.id == topic))
                                && !contents.items.iter().any(|item| item.topic_id.as_ref() == Some(topic))
                            {
                                self.selected_topic = None;
                                self.classwork_toggles.clear();
                            }
                            self.submissions_checked.remove(&id);
                            if self.assignments_completed.remove(&id) {
                                self.assignments_loaded = false;
                            }
                            snapshot.coursework.insert(id, contents.items);
                            if let Some(topics) = contents.topics {
                                snapshot.topics.insert(id, topics);
                            }
                        }
                    }
                    Err(error) => {
                        self.page_errors.insert(PageRead::Coursework(id), error);
                    }
                }
            }
            Message::LoadDetail(request) if !self.verify_live_course_count && !self.has_pending_request() => {
                if let Some(session) = self.session.clone() {
                    let requested_ids = match request {
                        DetailRead::TeacherGradebook(course) => self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(course))
                            .unwrap_or_default().iter().filter(|item| has_submission(item))
                            .filter_map(|item| item.id.parse::<i64>().ok()).collect(),
                        DetailRead::StudentMuting(course) => self.snapshot.as_ref().and_then(|snapshot| snapshot.roster_for(course))
                            .unwrap_or_default().iter().filter(|member| member.role == CourseRole::Student)
                            .filter_map(|member| member.id.parse::<i64>().ok()).collect(),
                        _ => Vec::new(),
                    };
                    let personal_template = match request {
                        DetailRead::PersonalCopy(course, item) => self.personal_copy_candidate(course, item),
                        _ => None,
                    };
                    if matches!(request, DetailRead::PersonalCopy(..)) {
                        self.personal_copy_loading_generation = Some(self.personal_copy_generation);
                    }
                    self.loading_detail = Some(request);
                    self.page_errors.remove(&PageRead::Detail(request));
                    return Task::perform(async move {
                        match request {
                            DetailRead::Roster(course) => desktop_auth::load_roster(&session, course).await
                                .map(DetailData::Roster),
                            DetailRead::Submissions(course) => desktop_auth::load_submissions(&session, course).await
                                .map(DetailData::Submissions),
                            DetailRead::PersonalCopy(course, item) => match personal_template {
                                Some(reference) => desktop_auth::check_personal_copy_absent(&session, course, item, reference.clone()).await
                                    .map(|absent| DetailData::PersonalCopy(reference, absent)),
                                None => Err("Personal-copy details changed. Refresh the assignment.".into()),
                            },
                            DetailRead::Comments(course, item) => desktop_auth::load_comments(&session, course, item).await
                                .map(DetailData::Comments),
                            DetailRead::PrivateFeedback(course, item) =>
                                desktop_auth::load_private_feedback(&session, course, item).await.map(DetailData::PrivateFeedback),
                            DetailRead::Rubric(course, item) => desktop_auth::load_rubric(&session, course, item).await
                                .map(DetailData::Rubric),
                            DetailRead::OverallGrade(course) => desktop_auth::load_course_grade(&session, course).await
                                .map(DetailData::OverallGrade),
                            DetailRead::TeacherDrafts(course) => desktop_auth::load_teacher_drafts(&session, course).await
                                .map(DetailData::TeacherDrafts),
                            DetailRead::TeacherSubmissions(course, item) => desktop_auth::load_teacher_submissions(&session, course, item).await
                                .map(DetailData::TeacherSubmissions),
                            DetailRead::TeacherFeedback(key) => desktop_auth::load_teacher_private_feedback(&session, key).await
                                .map(DetailData::TeacherFeedback),
                            DetailRead::TeacherGradebook(course) => desktop_auth::load_teacher_gradebook(&session, course, requested_ids).await
                                .map(DetailData::TeacherGradebook),
                            DetailRead::ClassStreamSettings(course) => desktop_auth::load_teacher_class_settings(&session, course).await
                                .map(DetailData::ClassStreamSettings),
                            DetailRead::StudentMuting(course) => desktop_auth::load_student_muting(&session, course, requested_ids).await
                                .map(DetailData::StudentMuting),
                            DetailRead::TeacherTopics(course) => desktop_auth::load_teacher_topics(&session, course).await
                                .map(DetailData::TeacherTopics),
                        }
                    }, move |result| Message::DetailLoaded(request, result));
                }
            }
            Message::DetailLoaded(request, result) => {
                self.loading_detail = None;
                if matches!(request, DetailRead::PersonalCopy(..))
                    && self.personal_copy_loading_generation.take() != Some(self.personal_copy_generation)
                {
                    self.queued_page_load = true;
                    return self.dequeue_page_load().map_or_else(Task::none, |read| self.update(read));
                }
                if std::env::var_os("CLASSROOM_READ_ONLY_DIAGNOSTICS").is_some() {
                    let section = match request {
                        DetailRead::Roster(_) => "People",
                        DetailRead::Submissions(_) => "Your work",
                        DetailRead::Comments(..) => "Class comments",
                        DetailRead::PersonalCopy(..) => "Personal copy",
                        DetailRead::PrivateFeedback(..) => "Private feedback",
                        DetailRead::Rubric(..) => "Rubric",
                        DetailRead::OverallGrade(_) => "Overall grade",
                        DetailRead::TeacherDrafts(_) => "Teacher drafts",
                        DetailRead::TeacherSubmissions(..) => "Teacher submissions",
                        DetailRead::TeacherFeedback(_) => "Teacher private feedback",
                        DetailRead::TeacherGradebook(_) => "Teacher gradebook",
                        DetailRead::ClassStreamSettings(_) => "Class Stream settings",
                        DetailRead::StudentMuting(_) => "Student mute states",
                        DetailRead::TeacherTopics(_) => "Teacher Classwork topics",
                    };
                    println!("{section} read completed; success={}", result.is_ok());
                }
                if matches!(self.screen, Screen::Item(_)) || self.tab == Tab::YourWork {
                    self.queued_page_load = true;
                }
                match (request, result) {
                    (_, Err(error)) => {
                        self.page_errors.insert(PageRead::Detail(request), error);
                    }
                    (DetailRead::Roster(course), Ok(DetailData::Roster(members))) => {
                        if let Some(snapshot) = &mut self.snapshot {
                            snapshot.roster.insert(course, members);
                        }
                        self.student_muting.remove(&course);
                        self.queued_page_load |= self.screen == Screen::Course(course) && self.tab == Tab::People;
                    }
                    (DetailRead::Submissions(course), Ok(DetailData::Submissions(submissions))) => {
                        self.personal_copy_generation = self.personal_copy_generation.wrapping_add(1);
                        if self.pending_personal_copy.as_ref().is_some_and(|(id, _, _)| *id == course) {
                            self.pending_personal_copy = None;
                        }
                        self.personal_copy_checks.retain(|(id, _), _| *id != course);
                        if let Some(snapshot) = &mut self.snapshot {
                            snapshot.set_submissions(course, submissions);
                            self.submissions_checked.insert(course);
                        }
                    }
                    (DetailRead::PersonalCopy(course, item), Ok(DetailData::PersonalCopy(reference, absent))) => {
                        self.personal_copy_checks.insert((course, item), (reference, absent));
                    }
                    (DetailRead::PrivateFeedback(course, item), Ok(DetailData::PrivateFeedback(comments))) => {
                        self.private_feedback.insert((course, item), comments);
                        self.private_unconfirmed.remove(&(course, item));
                    }
                    (DetailRead::OverallGrade(course), Ok(DetailData::OverallGrade(grade))) => {
                        self.overall_grades.insert(course, grade);
                    }
                    (DetailRead::TeacherDrafts(course), Ok(DetailData::TeacherDrafts(drafts)))
                        if self.class_roles.get(&course) == Some(&CourseRole::Teacher) =>
                    {
                        self.teacher_drafts.insert(course, drafts);
                        self.queued_page_load = true;
                    }
                    (DetailRead::TeacherSubmissions(course, item), Ok(DetailData::TeacherSubmissions(students)))
                        if self.can_read_teacher_work(course, item) =>
                    {
                        let previous = self.teacher_work.take().filter(|work| work.course == course && work.item == item);
                        let requested = self.gradebook_student.take()
                            .and_then(|(c, i, student)| (c == course && i == item).then_some(student));
                        let (filter, selected) = previous.as_ref().map_or((StudentWorkFilter::All, requested), |work| (work.filter, work.selected.or(requested)));
                        let mut work = TeacherWork { course, item, students, filter, selected: None,
                            grade: String::new(), confirming_return: false, bulk_selected: BTreeSet::new(), bulk_confirming: false };
                        work.select(selected.filter(|id| work.students.iter().any(|row| row.student_id == *id && filter.includes(row))));
                        self.teacher_work = Some(work);
                    }
                    (DetailRead::TeacherFeedback(key), Ok(DetailData::TeacherFeedback(comments)))
                        if self.selected_teacher_feedback_key() == Some(key) =>
                    {
                        let thread = self.teacher_feedback.entry(key).or_default();
                        thread.comments = Some(comments);
                        thread.unconfirmed = false;
                    }
                    (DetailRead::TeacherGradebook(course), Ok(DetailData::TeacherGradebook(gradebook)))
                        if self.class_roles.get(&course) == Some(&CourseRole::Teacher) =>
                    {
                        self.gradebooks.insert(course, gradebook);
                    }
                    (DetailRead::ClassStreamSettings(course), Ok(DetailData::ClassStreamSettings(settings)))
                        if self.tab == Tab::ClassSettings && self.can_edit_class_stream_settings(course) =>
                    {
                        self.class_stream_settings = Some(ClassStreamSettingsEditor::new(course, settings));
                    }
                    (DetailRead::StudentMuting(course), Ok(DetailData::StudentMuting(states)))
                        if self.class_roles.get(&course) == Some(&CourseRole::Teacher) =>
                    {
                        self.student_muting.insert(course, states);
                    }
                    (DetailRead::TeacherTopics(course), Ok(DetailData::TeacherTopics(topics)))
                        if self.class_roles.get(&course) == Some(&CourseRole::Teacher) =>
                    {
                        self.apply_teacher_topics(course, topics, None);
                        self.queued_page_load |= self.screen == Screen::Course(course) && self.tab == Tab::Classwork;
                    }
                    (DetailRead::Rubric(course, item), Ok(DetailData::Rubric(rubric))) => {
                        self.rubrics.insert((course, item), rubric);
                    }
                    (DetailRead::Comments(course, item), Ok(DetailData::Comments(comments))) => {
                        self.comment_unconfirmed.remove(&(course, item));
                        let item_id = item.to_string();
                        if let Some(item) = self.snapshot.as_mut()
                            .and_then(|snapshot| snapshot.coursework.get_mut(&course))
                            .and_then(|items| items.iter_mut().find(|item| item.id == item_id))
                        {
                            item.comments = Some(comments);
                        }
                    }
                    _ => {}
                }
            }
            Message::ClassFoldersLoaded(generation, result) if generation == self.class_folders_generation => {
                match result {
                    Ok(details) => {
                        if std::env::var_os("CLASSROOM_READ_ONLY_DIAGNOSTICS").is_some() {
                            println!("Class Drive folders ready; count={}", details.folders.len());
                            let _ = io::stdout().flush();
                        }
                        self.class_folders = details.folders;
                        self.class_roles = details.roles;
                        self.queued_page_load |= matches!(self.screen, Screen::Course(_) | Screen::Item(_));
                    }
                    Err(_) => self.class_folders_failed = true,
                }
            }
            Message::RetryClassFolders if self.class_folders_failed => return self.fetch_class_folders(),
            Message::ClassTeachersLoaded(generation, Ok(teachers)) if generation == self.teachers_generation => {
                self.class_teachers = teachers;
            }
            Message::AttachmentOpened => {}
            Message::AttachmentFailed => self.error = Some("The attachment couldn't be opened in your browser.".into()),
            Message::ArtworkLoaded(generation, url, image) if generation == self.artwork_generation => {
                self.artwork.insert(url, image);
            }
            Message::StreamProfilesLoaded(generation, Ok(profiles)) if generation == self.artwork_generation => {
                let urls = profiles.values().filter_map(|profile| profile.photo_url.as_ref())
                    .filter(|url| !self.stream_profile_pictures.contains_key(url.as_str())).cloned().collect();
                self.stream_profiles.extend(profiles);
                return Task::run(desktop_auth::profile_picture_updates(urls), move |(url, image)| {
                    Message::StreamProfilePictureLoaded(generation, url, image)
                });
            }
            Message::StreamProfilePictureLoaded(generation, url, image) if generation == self.artwork_generation => {
                self.stream_profile_pictures.insert(url, image);
            }
            _ => {}
        }
        self.finish_update()
    }
}
