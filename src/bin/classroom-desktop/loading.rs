use super::*;

impl App {
    pub(super) fn assignment_courses(&self) -> BTreeSet<i64> {
        self.snapshot.iter().flat_map(|snapshot| &snapshot.courses)
            .filter(|course| home_course(course) && !self.assignments_completed.contains(&course.id))
            .map(|course| course.id).collect()
    }

    pub(super) fn apply_assignment_update(&mut self, result: desktop_auth::AssignmentRefresh) {
        let (Some(batch), Some(snapshot)) = (&mut self.assignment_batch, &mut self.snapshot) else {
            return;
        };
        let id = result.course_id;
        if !batch.remaining.remove(&id) {
            return;
        }

        if let Some(mut contents) = result.contents {
            self.personal_copy_generation = self.personal_copy_generation.wrapping_add(1);
            self.personal_copy_checks.retain(|(course, _), _| *course != id);
            if self.pending_personal_copy.as_ref().is_some_and(|(course, _, _)| *course == id) {
                self.pending_personal_copy = None;
            }
            if let Some(previous) = snapshot.coursework_for(id) {
                for item in &mut contents.items {
                    if let Some(old) = previous.iter().find(|old| old.id == item.id) {
                        item.comments = old.comments.clone();
                        if result.submissions.is_none() {
                            item.submission = old.submission.clone();
                        }
                    }
                }
            }
            let needs_status = contents.items.iter().any(has_submission);
            if let Some(topics) = contents.topics {
                snapshot.topics.insert(id, topics);
            }
            snapshot.coursework.insert(id, contents.items);
            if let Some(submissions) = result.submissions {
                snapshot.set_submissions(id, submissions);
                self.submissions_checked.insert(id);
                self.assignments_completed.insert(id);
            } else if needs_status {
                batch.failed_status += 1;
            } else {
                self.assignments_completed.insert(id);
            }
        } else {
            batch.failed_classwork += 1;
        }

        if batch.remaining.is_empty() {
            if let Some(error) = batch.error() {
                self.page_errors.insert(PageRead::Assignments, error);
            }
            self.assignments_loaded = true;
            self.assignment_batch = None;
            if self.screen == Screen::Home && std::env::var_os("CLASSROOM_READ_ONLY_DIAGNOSTICS").is_some() {
                println!("Home overview ready; classes={} due_soon={}",
                    self.assignments_completed.len(), snapshot.upcoming_work(&Local::now(), None, 5).len());
                let today = Local::now().date_naive();
                for filter in TodoFilter::ALL {
                    let groups = self.todo_items(today, filter);
                    let counts = groups.iter().map(|(group, items)| (group.label(), items.len())).collect::<Vec<_>>();
                    println!("To-do {} {counts:?}", filter.label());
                }
                if let Some(previous) = monday_of(today).checked_sub_days(Days::new(7)) {
                    let counts = self.calendar_items(previous).iter()
                        .map(|(date, items)| (date.to_string(), items.len())).collect::<Vec<_>>();
                    println!("Calendar previous week {counts:?}");
                }
                let _ = io::stdout().flush();
            }
        }
    }

    pub(super) fn dequeue_page_load(&mut self) -> Option<Message> {
        if self.queued_page_load && !self.has_pending_request() {
            self.queued_page_load = false;
            return self.visible_page_request();
        }
        None
    }

    pub(super) fn retry_visible_reads_on_entry(&mut self) {
        let item = self.item_id.as_ref().and_then(|id| id.parse::<i64>().ok());
        self.page_errors.retain(|read, _| {
            let visible = match (*read, self.screen) {
                (PageRead::Assignments, screen) => screen.uses_assignment_overview(),
                (PageRead::Invitations, Screen::Home | Screen::Join) => true,
                (PageRead::ClassNotifications | PageRead::EmailNotifications, Screen::Settings) => true,
                (PageRead::Coursework(course), Screen::Course(id)) => id == course && self.tab != Tab::People,
                (PageRead::StreamAccess(course), Screen::Course(id)) => id == course && self.tab == Tab::Stream,
                (PageRead::StreamAccess(course), Screen::Item(id)) => id == course,
                (PageRead::Coursework(course), Screen::Item(id)) => id == course,
                (PageRead::Detail(DetailRead::Roster(course)), Screen::Course(id)) => id == course && self.tab == Tab::People,
                (PageRead::Detail(DetailRead::StudentMuting(course)), Screen::Course(id)) => id == course && self.tab == Tab::People,
                (PageRead::Detail(DetailRead::TeacherTopics(course)), Screen::Course(id)) => id == course && self.tab == Tab::Classwork,
                (PageRead::Detail(DetailRead::Submissions(course)), Screen::Course(id)) =>
                    id == course && self.tab != Tab::People,
                (PageRead::Detail(DetailRead::OverallGrade(course)), Screen::Course(id)) =>
                    id == course && self.tab == Tab::YourWork,
                (PageRead::Detail(DetailRead::TeacherDrafts(course)), Screen::Course(id)) =>
                    id == course && matches!(self.tab, Tab::Classwork | Tab::Stream),
                (PageRead::Detail(DetailRead::TeacherGradebook(course)), Screen::Course(id)) =>
                    id == course && self.tab == Tab::Grades,
                (PageRead::Detail(DetailRead::ClassStreamSettings(course)), Screen::Course(id)) =>
                    id == course && self.tab == Tab::ClassSettings,
                (PageRead::Detail(DetailRead::Submissions(course)), Screen::Item(id)) => id == course,
                (PageRead::Detail(DetailRead::Comments(course, key) | DetailRead::PrivateFeedback(course, key)
                    | DetailRead::Rubric(course, key) | DetailRead::PersonalCopy(course, key)
                    | DetailRead::TeacherSubmissions(course, key)), Screen::Item(id)) =>
                    id == course && item == Some(key),
                (PageRead::Detail(DetailRead::TeacherFeedback(key)), Screen::Item(id)) =>
                    id == key.course && item == Some(key.item),
                _ => false,
            };
            !visible
        });
        if self.screen.uses_assignment_overview() && !self.assignment_courses().is_empty() {
            self.assignments_loaded = false;
        }
    }

    fn visible_page_request(&self) -> Option<Message> {
        if !self.can_read_pages() {
            return None;
        }
        self.next_page_read(self.snapshot.as_ref()?).map(PageRead::message)
    }

    pub(super) fn next_page_read(&self, snapshot: &Snapshot) -> Option<PageRead> {
        let read = match self.screen {
            Screen::Settings if self.class_notifications.is_none() && !self.loading_class_notifications
                && !self.page_errors.contains_key(&PageRead::ClassNotifications) =>
                PageRead::ClassNotifications,
            Screen::Settings if self.email_notifications.is_none() && !self.loading_email_notifications =>
                PageRead::EmailNotifications,
            Screen::Home | Screen::Join if self.invitations.is_none() && !self.loading_invitations
                && !self.page_errors.contains_key(&PageRead::Invitations) => PageRead::Invitations,
            Screen::Home | Screen::Calendar | Screen::ToDo if !self.assignments_loaded && self.assignment_batch.is_none() => {
                PageRead::Assignments
            }
            Screen::Course(course) if self.tab == Tab::People && snapshot.roster_for(course).is_none() => {
                PageRead::Detail(DetailRead::Roster(course))
            }
            Screen::Course(course) if self.tab == Tab::People && self.class_roles.get(&course) == Some(&CourseRole::Teacher)
                && snapshot.courses.iter().any(|entry| entry.id == course && entry.state == CourseState::Active)
                && !self.student_muting.contains_key(&course) =>
            {
                PageRead::Detail(DetailRead::StudentMuting(course))
            }
            Screen::Course(course) if self.tab == Tab::ClassSettings => {
                if self.class_roles.get(&course) != Some(&CourseRole::Teacher)
                    || self.class_stream_settings.as_ref().is_some_and(|editor| editor.course == course)
                {
                    return None;
                }
                PageRead::Detail(DetailRead::ClassStreamSettings(course))
            }
            Screen::Course(course) if self.tab != Tab::People => {
                let coursework = PageRead::Coursework(course);
                let submissions = PageRead::Detail(DetailRead::Submissions(course));
                let overall = PageRead::Detail(DetailRead::OverallGrade(course));
                let items = snapshot.coursework_for(course);
                let teacher = self.class_roles.get(&course) == Some(&CourseRole::Teacher);
                [
                    (items.is_none(), coursework),
                    (teacher && self.tab == Tab::Classwork && !self.teacher_topics.contains_key(&course)
                        && snapshot.courses.iter().any(|entry| entry.id == course && entry.state == CourseState::Active),
                        PageRead::Detail(DetailRead::TeacherTopics(course))),
                    (self.tab == Tab::Stream && snapshot.courses.iter().any(|entry| entry.id == course && entry.state == CourseState::Active)
                        && !self.stream_access.contains_key(&course) && !self.loading_stream_access.contains(&course), PageRead::StreamAccess(course)),
                    (teacher && matches!(self.tab, Tab::Classwork | Tab::Stream) && !self.teacher_drafts.contains_key(&course),
                        PageRead::Detail(DetailRead::TeacherDrafts(course))),
                    (teacher && self.tab == Tab::Grades && !self.gradebooks.contains_key(&course),
                        PageRead::Detail(DetailRead::TeacherGradebook(course))),
                    (!teacher && matches!(self.tab, Tab::Classwork | Tab::YourWork) && !self.submissions_checked.contains(&course)
                        && items.is_some_and(|items| items.iter().any(has_submission)), submissions),
                    (self.tab == Tab::Stream && !self.submissions_checked.contains(&course)
                        && !snapshot.upcoming_work(&Local::now(), Some(course), 1).is_empty(), submissions),
                    (self.tab == Tab::YourWork && !self.overall_grades.contains_key(&course), overall),
                ].into_iter().find_map(|(missing, read)| {
                    (missing && !self.page_errors.contains_key(&read)).then_some(read)
                })?
            }
            Screen::Item(course) if snapshot.coursework_for(course).is_none() => PageRead::Coursework(course),
            Screen::Item(course) => {
                let id = self.item_id.as_ref()?;
                let item = snapshot.coursework_for(course)?.iter().find(|item| &item.id == id)?;
                let item_id = id.parse().ok()?;
                let comments = PageRead::Detail(DetailRead::Comments(course, item_id));
                let submissions = PageRead::Detail(DetailRead::Submissions(course));
                let feedback = PageRead::Detail(DetailRead::PrivateFeedback(course, item_id));
                let rubric = PageRead::Detail(DetailRead::Rubric(course, item_id));
                let teacher = self.class_roles.get(&course) == Some(&CourseRole::Teacher);
                let teacher_work = PageRead::Detail(DetailRead::TeacherSubmissions(course, item_id));
                if let Some(key) = self.selected_teacher_feedback_key() {
                    let thread = PageRead::Detail(DetailRead::TeacherFeedback(key));
                    if self.teacher_feedback.get(&key).is_none_or(|data| data.comments.is_none())
                        && !self.page_errors.contains_key(&thread)
                    {
                        return Some(thread);
                    }
                }
                [
                    (teacher && has_submission(item) && !self.teacher_work.as_ref()
                        .is_some_and(|work| work.course == course && work.item == item_id), teacher_work),
                    (snapshot.courses.iter().any(|entry| entry.id == course && entry.state == CourseState::Active)
                        && !self.stream_access.contains_key(&course) && !self.loading_stream_access.contains(&course), PageRead::StreamAccess(course)),
                    (item.comments.is_none() || self.comment_unconfirmed.contains(&(course, item_id)), comments),
                    (!teacher && has_submission(item) && !self.submissions_checked.contains(&course), submissions),
                    (self.personal_copy_candidate(course, item_id).is_some()
                        && self.personal_copy_checks.get(&(course, item_id)).is_none_or(|(reference, _)|
                            self.personal_copy_candidate(course, item_id).as_ref() != Some(reference)),
                        PageRead::Detail(DetailRead::PersonalCopy(course, item_id))),
                    (item.kind == CourseworkKind::Assignment && !self.rubrics.contains_key(&(course, item_id)), rubric),
                    (!teacher && snapshot.source == SnapshotSource::Live
                        && item.submission.as_ref().is_some_and(|sub| sub.remote_id.is_some())
                        && (!self.private_feedback.contains_key(&(course, item_id))
                            || self.private_unconfirmed.contains(&(course, item_id))), feedback),
                ].into_iter().find_map(|(missing, read)| {
                    (missing && !self.page_errors.contains_key(&read)).then_some(read)
                })?
            }
            _ => return None,
        };
        (!self.page_errors.contains_key(&read)).then_some(read)
    }
}
