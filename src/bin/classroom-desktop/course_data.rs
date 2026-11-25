use super::*;

impl App {
    pub(super) fn reset_class_folders(&mut self) {
        self.class_folders_generation = self.class_folders_generation.wrapping_add(1);
        self.class_folders.clear();
        self.class_roles.clear();
        self.teacher_drafts.clear();
        self.open_teacher_draft = None;
        self.assignment_editor = None;
        self.assignment_publication = None;
        self.teacher_work = None;
        self.teacher_feedback.clear();
        self.gradebooks.clear();
        self.class_stream_settings = None;
        self.student_muting.clear();
        self.pending_student_mute = None;
        self.teacher_topics.clear();
        self.topic_editor = None;
        self.item_topic_editor = None;
        self.topic_manager = false;
        self.gradebook_student = None;
        self.gradebook_edit_mode = false;
        self.gradebook_edit = None;
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
            move |result| Message::ClassFoldersLoaded(generation, result))
    }

    pub(super) fn fetch_class_creation_permission(&mut self) -> Task<Message> {
        self.class_creation_generation = self.class_creation_generation.wrapping_add(1);
        self.class_creation_allowed = false;
        let generation = self.class_creation_generation;
        if let Some(session) = self.session.clone().filter(|_| self.can_read_pages()) {
            Task::perform(async move { desktop_auth::can_create_classes(&session).await },
                move |result| Message::ClassCreationPermission(generation, result))
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
            move |result| Message::ClassTeachersLoaded(generation, result))
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
        self.stream_profiles_requested.clear();
        self.stream_profiles.clear();
        self.stream_profile_pictures.clear();
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
            .filter(|id| !self.stream_profiles_requested.contains(id))
            .collect();
        let ids: BTreeSet<_> = ids.into_iter().take(64).collect();
        if ids.is_empty() { return Task::none(); }
        self.stream_profiles_requested.extend(ids.iter().copied());
        let generation = self.artwork_generation;
        Task::perform(async move { desktop_auth::load_stream_profiles(&session, ids).await },
            move |result| Message::StreamProfilesLoaded(generation, result))
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
            Message::ArtworkLoaded(generation, url, image)
        })
    }

    pub(super) fn artwork_for<'a>(&'a self, course: &Course) -> Option<&'a image::Handle> {
        self.artwork.get(course.theme.as_ref()?.artwork_url.as_ref()?)
    }

    pub(super) fn save_offline(&mut self) {
        let (Some(session), Some(snapshot)) = (&self.session, &self.snapshot) else { return };
        let result = cache_path().ok_or_else(|| "Offline storage isn't available.".to_string())
            .and_then(|path| desktop_data::save_cache(path, &session.account, snapshot)
                .map_err(|_| "The offline copy couldn't be saved.".into()));
        self.finish_cache_action(result, "Offline copy saved on this Mac.");
    }

    pub(super) fn open_offline(&mut self) {
        let Some(session) = &self.session else { return };
        let result = cache_path().ok_or_else(|| "Offline storage isn't available.".to_string())
            .and_then(|path| desktop_data::load_cache(path, &session.account)
                .map_err(|_| "The offline copy couldn't be opened.".into()));
        match result {
            Ok(Some(snapshot)) => {
                self.install_snapshot(snapshot);
                self.error = None;
                self.notice = None;
                self.screen = Screen::Home;
                self.demo = false;
            }
            Ok(None) => self.error = Some("There is no saved copy for that email.".into()),
            Err(error) => self.error = Some(error),
        }
    }

    pub(super) fn delete_offline(&mut self) {
        let Some(session) = &self.session else { return };
        let result = cache_path().ok_or_else(|| "Offline storage isn't available.".to_string())
            .and_then(|path| desktop_data::clear_cache(path, &session.account)
                .map_err(|_| "The offline copy couldn't be removed.".into()));
        if result.is_ok() && self.snapshot.as_ref().is_some_and(|data| data.source == SnapshotSource::Cache) {
            self.install_snapshot(session.snapshot.clone());
        }
        self.finish_cache_action(result, "Offline copy removed from this Mac.");
    }

    fn finish_cache_action(&mut self, result: Result<(), String>, success: &str) {
        self.error = result.as_ref().err().cloned();
        self.notice = result.is_ok().then(|| success.to_owned());
    }

    pub(super) fn reset_personal_copy(&mut self) {
        self.personal_copy_generation = self.personal_copy_generation.wrapping_add(1);
        self.personal_copy_checks.clear();
        self.pending_personal_copy = None;
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
            self.submissions_checked.clear();
        }
        if let Screen::Course(id) | Screen::Item(id) = self.screen
            && !snapshot.courses.iter().any(|course| course.id == id)
        {
            self.discard_submission_capture();
            self.screen = Screen::Home;
            self.tab = Tab::Stream;
        }
        for filter in [&mut self.calendar_class, &mut self.todo_class] {
            if filter.is_some_and(|id| !snapshot.courses.iter()
                .any(|course| course.id == id && course.state.shows_on_home()))
            {
                *filter = None;
            }
        }
        self.snapshot = Some(snapshot);
    }
}
