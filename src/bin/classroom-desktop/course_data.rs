use super::*;

impl App {
    pub(super) fn reset_class_folders(&mut self) {
        self.class_folders_generation = self.class_folders_generation.wrapping_add(1);
        self.class_folders.clear();
        self.class_roles.clear();
        self.classwork.teacher_drafts.clear();
        self.classwork.open_teacher_draft = None;
        self.classwork.assignment_editor = None;
        self.classwork.assignment_publication = None;
        self.grading.teacher_work = None;
        self.grading.teacher_feedback.clear();
        self.grading.gradebooks.clear();
        self.class_stream_settings = None;
        self.people.student_muting.clear();
        self.people.pending_student_mute = None;
        self.classwork.teacher_topics.clear();
        self.classwork.topic_editor = None;
        self.classwork.item_topic_editor = None;
        self.classwork.topic_manager = false;
        self.grading.gradebook_student = None;
        self.grading.gradebook_edit_mode = false;
        self.grading.gradebook_edit = None;
        self.page_errors.retain(|read, _| !matches!(read, PageRead::Detail(DetailRead::TeacherDrafts(_) | DetailRead::TeacherGradebook(_))));
        self.class_folders_failed = false;
    }

    pub(super) fn fetch_class_folders(&mut self) -> Task<Message> {
        self.reset_class_folders();
        if !self.can_read_pages() {
            return Task::none();
        }
        let (Some(snapshot), Some(session)) = (&self.snapshot, self.session.clone()) else {
            return Task::none();
        };
        let courses = snapshot.courses.iter()
            .filter(|course| course.state != CourseState::Other)
            .map(|course| course.id).collect();
        let generation = self.class_folders_generation;
        Task::perform(async move { desktop_auth::load_own_class_details(&session, courses).await },
            move |result| Message::Reads(ReadsMessage::ClassFoldersLoaded(generation, result)))
    }

    pub(super) fn fetch_class_creation_permission(&mut self) -> Task<Message> {
        self.class_creation_generation = self.class_creation_generation.wrapping_add(1);
        self.class_creation_allowed = false;
        let generation = self.class_creation_generation;
        if let Some(session) = self.session.clone().filter(|_| self.can_read_pages()) {
            Task::perform(async move { desktop_auth::can_create_classes(&session).await },
                move |result| Message::Courses(CoursesMessage::ClassCreationPermission(generation, result)))
        } else {
            Task::none()
        }
    }

    pub(super) fn fetch_class_teachers(&mut self) -> Task<Message> {
        if !self.can_read_pages() {
            return Task::none();
        }
        let (Some(snapshot), Some(session)) = (&self.snapshot, self.session.clone()) else {
            return Task::none();
        };
        let courses: Vec<_> = snapshot.courses.iter()
            .filter(|course| !course.owner_ids.is_empty() && course.state != CourseState::Other)
            .map(|course| (course.id, course.owner_ids.clone())).collect();
        self.teachers_generation = self.teachers_generation.wrapping_add(1);
        let generation = self.teachers_generation;
        Task::perform(async move { desktop_auth::load_class_teacher_names(&session, &courses).await },
            move |result| Message::Reads(ReadsMessage::ClassTeachersLoaded(generation, result)))
    }

    pub(super) fn class_folder_url(&self, id: i64) -> Option<&str> {
        let snapshot = self.snapshot.as_ref()?;
        if snapshot.source != SnapshotSource::Live && self.screenshot_directory.is_none() {
            return None;
        }
        self.class_folders.get(&id).map(String::as_str)
    }

    pub(super) fn reset_artwork(&mut self) {
        self.artwork.clear();
        self.stream.stream_profiles_requested.clear();
        self.stream.stream_profiles.clear();
        self.stream.stream_profile_pictures.clear();
        self.artwork_generation = self.artwork_generation.wrapping_add(1);
    }

    pub(super) fn fetch_stream_profiles(&mut self) -> Task<Message> {
        let Screen::Course(course) = self.screen else { return Task::none(); };
        if self.tab != Tab::Stream || !self.can_read_pages() {
            return Task::none();
        }
        let (Some(session), Some(snapshot)) = (self.session.clone(), self.snapshot.as_ref()) else { return Task::none(); };
        let display = snapshot.courses.iter().find(|entry| entry.id == course)
            .and_then(|entry| entry.stream_classwork_display);
        let ids: BTreeSet<_> = snapshot.coursework_for(course).unwrap_or_default().iter()
            .filter(|item| display.is_none_or(|display| display.shows(item.kind)))
            .filter_map(|item| item.author_id)
            .filter(|id| !self.stream.stream_profiles_requested.contains(id))
            .collect();
        let ids: BTreeSet<_> = ids.into_iter().take(64).collect();
        if ids.is_empty() { return Task::none(); }
        self.stream.stream_profiles_requested.extend(ids.iter().copied());
        let generation = self.artwork_generation;
        Task::perform(async move { desktop_auth::load_stream_profiles(&session, ids).await },
            move |result| Message::Reads(ReadsMessage::StreamProfilesLoaded(generation, result)))
    }

    pub(super) fn fetch_artwork(&self) -> Task<Message> {
        if self.verify_live_course_count || self.screenshot_directory.is_some() || self.session.is_none() {
            return Task::none();
        }
        let Some(snapshot) = &self.snapshot else { return Task::none() };
        let ordered = snapshot.courses.iter().filter(|course| home_course(course))
            .chain(snapshot.courses.iter().filter(|course| !home_course(course)));
        let urls: Vec<_> = ordered
            .filter_map(|course| course.theme.as_ref()?.artwork_url.as_ref())
            .filter(|url| !self.artwork.contains_key(url.as_str()))
            .cloned()
            .collect();
        if urls.is_empty() {
            return Task::none();
        }
        let generation = self.artwork_generation;
        Task::run(desktop_auth::artwork_updates(urls), move |(url, image)| {
            Message::Reads(ReadsMessage::ArtworkLoaded(generation, url, image))
        })
    }

    pub(super) fn artwork_for<'a>(&'a self, course: &Course) -> Option<&'a image::Handle> {
        self.artwork.get(course.theme.as_ref()?.artwork_url.as_ref()?)
    }

    pub(super) fn reset_personal_copy(&mut self) {
        self.attachments.personal_copy_generation = self.attachments.personal_copy_generation.wrapping_add(1);
        self.attachments.personal_copy_checks.clear();
        self.attachments.pending_personal_copy = None;
    }

    pub(super) fn install_snapshot(&mut self, snapshot: Snapshot) {
        self.invalidate_people_email();
        self.reset_personal_copy();
        self.teachers_generation = self.teachers_generation.wrapping_add(1);
        self.class_teachers.clear();
        self.reset_notifications();
        self.reset_stream_access();
        if self.snapshot.as_ref().is_some_and(|old| old.source != snapshot.source) {
            self.reset_assignment_reads();
            self.student_work.submissions_checked.clear();
        }
        if let Screen::Course(id) | Screen::Item(id) = self.screen
            && !snapshot.courses.iter().any(|course| course.id == id)
        {
            self.discard_submission_capture();
            self.screen = Screen::Home;
            self.tab = Tab::Stream;
        }
        for filter in [&mut self.deadlines.calendar_class, &mut self.deadlines.todo_class] {
            if filter.is_some_and(|id| !snapshot.courses.iter()
                .any(|course| course.id == id && course.state.shows_on_home()))
            {
                *filter = None;
            }
        }
        self.snapshot = Some(snapshot);
    }
}
