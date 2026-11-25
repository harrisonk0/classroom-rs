use super::*;

impl App {
    pub(super) fn update_screenshots(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ScreenshotReady(shot, image) => {
                let Some(directory) = &self.screenshot_directory else { return Task::none() };
                if let Err(error) = save_screenshot(directory, shot, &image) {
                    eprintln!("Could not save screenshot: {error}");
                    return iced::exit();
                }
                match shot {
                    Shot::Dashboard => {
                        self.invitations = Some(vec![Course {
                            id: 700, title: "Art & Design".into(), subtitle: "Year 12 · Group A".into(),
                            ..Default::default()
                        }]);
                        return schedule_capture(Shot::Invitations);
                    }
                    Shot::Invitations => {
                        self.confirming_decline = Some(700);
                        return schedule_capture(Shot::DeclineInvitation);
                    }
                    Shot::DeclineInvitation => {
                        self.confirming_decline = None;
                        self.invitations = None;
                        if let Some(course) = self.snapshot.as_ref().and_then(|data| data.courses.first()) {
                            self.screen = Screen::Course(course.id);
                            self.tab = Tab::Classwork;
                            return schedule_capture(Shot::Classwork);
                        }
                    }
                    Shot::Classwork => {
                        self.selected_topic = Some("demo-poetry-unit".into());
                        return schedule_capture(Shot::ClassworkTopic);
                    }
                    Shot::ClassworkTopic => {
                        self.selected_topic = None;
                        self.screen = Screen::Course(2);
                        self.tab = Tab::YourWork;
                        self.overall_grades.insert(2, Some(CourseGrade {
                            percent: 87.6644, calculation: GradeCalculation::TotalPoints,
                        }));
                        return schedule_capture(Shot::YourWork);
                    }
                    Shot::YourWork => {
                        self.overall_grades.remove(&2);
                        self.screen = Screen::ToDo;
                        return schedule_capture(Shot::ToDo);
                    }
                    Shot::ToDo => {
                        if let Some(question) = self.snapshot.as_mut()
                            .and_then(|data| data.coursework.get_mut(&4))
                            .and_then(|items| items.first_mut())
                        {
                            question.question = Some(QuestionDetails::MultipleChoice(vec![
                                "Source A".into(), "Source B".into(), "Both sources".into(),
                            ]));
                            question.question_sharing = Some(QuestionSharing::Course);
                            question.submission = Some(Submission {
                                status: SubmissionStatus::TurnedIn,
                                question_answer: Some(QuestionAnswer::MultipleChoice("Both sources".into())),
                                last_turned_in_millis: Some(1_784_600_123_000),
                                ..Default::default()
                            });
                            self.screen = Screen::Item(4);
                            self.item_id = Some(question.id.clone());
                            return schedule_capture(Shot::Question);
                        }
                        self.screen = Screen::Calendar;
                        return schedule_capture(Shot::Calendar);
                    }
                    Shot::Question => {
                        if let Some(question) = self.snapshot.as_mut()
                            .and_then(|data| data.coursework.get_mut(&4))
                            .and_then(|items| items.first_mut())
                        {
                            question.id = "42".into();
                            question.question = Some(QuestionDetails::ShortAnswer);
                            question.question_sharing = None;
                            question.submission = Some(Submission {
                                status: SubmissionStatus::Assigned,
                                remote_id: Some(67),
                                remote_state: Some(4),
                                ..Default::default()
                            });
                            self.item_id = Some(question.id.clone());
                            self.short_answer_drafts.insert((4, 42), text_editor::Content::with_text(
                                "Source A describes the event from a witness’s perspective. Source B explains how it was interpreted later.",
                            ));
                            return schedule_capture(Shot::ShortAnswer);
                        }
                        self.screen = Screen::Calendar;
                        return schedule_capture(Shot::Calendar);
                    }
                    Shot::ShortAnswer => {
                        if let Some(question) = self.snapshot.as_mut()
                            .and_then(|data| data.coursework.get_mut(&4))
                            .and_then(|items| items.first_mut())
                        {
                            let original = "Source A is a witness’s account.";
                            question.allow_edit_after_turn_in = true;
                            question.submission = Some(Submission {
                                status: SubmissionStatus::TurnedIn,
                                remote_id: Some(67),
                                remote_state: Some(2),
                                question_answer: Some(QuestionAnswer::ShortAnswer(original.into())),
                                ..Default::default()
                            });
                            self.short_answer_drafts.remove(&(4, 42));
                            return schedule_capture(Shot::SubmittedShortAnswer);
                        }
                        self.screen = Screen::Calendar;
                        return schedule_capture(Shot::Calendar);
                    }
                    Shot::SubmittedShortAnswer => {
                        self.short_answer_drafts.insert((4, 42), text_editor::Content::with_text(
                            "Source A is a witness’s account. Source B describes how the event was interpreted later.",
                        ));
                        self.short_answer_revisions.insert((4, 42), desktop_auth::ShortAnswerRevision {
                            submission_id: 67, answer: "Source A is a witness’s account.".into(),
                        });
                        return schedule_capture(Shot::ShortAnswerEdit);
                    }
                    Shot::ShortAnswerEdit => {
                        if let Some(question) = self.snapshot.as_mut()
                            .and_then(|data| data.coursework.get_mut(&4))
                            .and_then(|items| items.first_mut())
                        {
                            question.question = Some(QuestionDetails::MultipleChoice(vec![
                                "Source A".into(), "Source B".into(), "Both sources".into(),
                            ]));
                            question.allow_edit_after_turn_in = false;
                            question.submission = Some(Submission {
                                status: SubmissionStatus::Assigned,
                                remote_id: Some(67),
                                remote_state: Some(4),
                                ..Default::default()
                            });
                            self.short_answer_drafts.remove(&(4, 42));
                            self.short_answer_revisions.remove(&(4, 42));
                            self.choice_drafts.insert((4, 42), "Both sources".into());
                            self.confirming_question_answer = Some((4, 42));
                            return schedule_capture(Shot::MultipleChoiceDraft);
                        }
                        self.screen = Screen::Calendar;
                        return schedule_capture(Shot::Calendar);
                    }
                    Shot::MultipleChoiceDraft => {
                        self.confirming_question_answer = None;
                        if let Some(item) = self.snapshot.as_mut().and_then(|data| data.coursework.get_mut(&1))
                            .and_then(|items| items.iter_mut().find(|item| item.kind == CourseworkKind::Assignment))
                        {
                            item.id = "101".into();
                            item.submission = Some(Submission {
                                status: SubmissionStatus::Returned,
                                grade: Some(desktop_data::Grade { earned: 8.0, maximum: Some(10.0) }),
                                rubric_scores: BTreeMap::from([("analysis".into(), desktop_data::CriterionScore {
                                    rating_id: Some("clear".into()), points: Some(8.0),
                                })]),
                                ..Default::default()
                            });
                            self.screen = Screen::Item(1);
                            self.item_id = Some(item.id.clone());
                            self.rubrics.insert((1, 101), Some(Rubric {
                                title: Some("Poetry comparison".into()),
                                criteria: vec![desktop_data::RubricCriterion {
                                    id: Some("analysis".into()),
                                    title: "Analysis".into(),
                                    description: "Explain how each poem develops the theme.".into(),
                                    ratings: vec![
                                        desktop_data::RubricRating {
                                            id: Some("detailed".into()), title: "Detailed".into(),
                                            description: "Compares both poems using specific textual evidence.".into(), points: Some(10.0),
                                        },
                                        desktop_data::RubricRating {
                                            id: Some("clear".into()), title: "Clear".into(),
                                            description: "Explains the theme using clear examples.".into(), points: Some(8.0),
                                        },
                                    ],
                                }],
                            }));
                            self.rubric_open = true;
                            self.open_criteria.insert(0);
                            return schedule_capture(Shot::Rubric);
                        }
                        self.screen = Screen::Calendar;
                        return schedule_capture(Shot::Calendar);
                    }
                    Shot::Rubric => {
                        self.screen = Screen::Calendar;
                        return schedule_capture(Shot::Calendar);
                    }
                    Shot::Calendar => {
                        self.calendar_class = Some(1);
                        if let Some(course) = self.snapshot.as_mut()
                            .and_then(|snapshot| snapshot.courses.iter_mut().find(|course| course.id == 1))
                        {
                            course.calendar_url = Some("https://calendar.google.com/calendar/u/0?cid=fixture".into());
                        }
                        return schedule_capture(Shot::ClassCalendar);
                    }
                    Shot::ClassCalendar => {
                        self.calendar_class = None;
                        return window::latest().then(|id| match id {
                            Some(id) => window::resize(id, iced::Size::new(810.0, 565.0))
                                .chain(schedule_capture(Shot::CompactCalendar)),
                            None => iced::exit(),
                        });
                    }
                    Shot::CompactCalendar => {
                        self.screen = Screen::Course(1);
                        self.tab = Tab::Classwork;
                        self.selected_topic = Some("demo-poetry-unit".into());
                        return schedule_capture(Shot::CompactClasswork);
                    }
                    Shot::CompactClasswork => {
                        self.screen = Screen::Settings;
                        self.class_notifications = Some(self.class_notification_courses()
                            .enumerate().map(|(index, course)| (course.id, index == 1)).collect());
                        self.email_notifications = Some(EmailNotificationSettings {
                            enabled: Some(true),
                            categories: vec![desktop_auth::EmailNotificationCategory {
                                label: Some("Comments".into()),
                                groups: vec![
                                    desktop_auth::EmailNotificationGroup { tag: 1, label: "Comments on your posts".into(), enabled: Some(true) },
                                    desktop_auth::EmailNotificationGroup { tag: 2, label: "Comments that mention you".into(), enabled: Some(false) },
                                ],
                            }],
                        });
                        return window::latest().then(|id| match id {
                            Some(id) => window::resize(id, iced::Size::new(1200.0, 810.0))
                                .chain(schedule_capture(Shot::Settings)),
                            None => Task::none(),
                        });
                    }
                    Shot::Settings => {
                        self.screen = Screen::Course(1);
                        self.tab = Tab::Stream;
                        self.stream_access.insert(1, StreamAccess::PostAndComment);
                        self.open_stream_composer = Some(1);
                        let sample = "Read the first chapter before Friday.\nBring one question.\nReview your class notes.";
                        let mut styles = vec![0; sample.len()];
                        styles[..22].fill(1);
                        styles[29..35].fill(2 | 4);
                        self.stream_drafts.insert(1, StreamDraft {
                            body: text_editor::Content::with_text(sample),
                            styles,
                            bullets: paragraph_starts(sample).skip(1).collect(),
                            links: vec!["https://www.bl.uk/learning".into()],
                            ..Default::default()
                        });
                        return schedule_capture(Shot::StreamComposer);
                    }
                    Shot::StreamComposer => {
                        if let Some(draft) = self.stream_drafts.get_mut(&1) {
                            draft.selected_file = None;
                        }
                        self.drive_picker = Some(DrivePicker {
                            course: 1,
                            file_page: Some(DriveFilePage {
                                folders: vec![DriveFolder { id: "fixture_folder".into(), name: "English Literature".into() }],
                                files: [
                                    ("fixture_reading", "Course reading notes", "application/vnd.google-apps.document"),
                                    ("fixture_study", "Poetry study guide.pdf", "application/pdf"),
                                    ("fixture_examples", "Worked examples", "application/vnd.google-apps.presentation"),
                                ].into_iter().filter_map(|(id, name, mime)| {
                                    DriveFileAttachment::existing(id.into(), name.into(), mime.into())
                                }).collect(),
                                next_page: None,
                                incomplete: false,
                            }),
                            section: DriveSection::MyDrive,
                            ..Default::default()
                        });
                        return schedule_capture(Shot::DrivePicker);
                    }
                    Shot::DrivePicker => {
                        if let Some(item) = self.snapshot.as_mut().and_then(|snapshot| snapshot.coursework.get_mut(&1))
                            .and_then(|items| items.iter_mut().find(|item| item.kind == CourseworkKind::Assignment))
                        {
                            item.id = "42".into();
                            item.submission = Some(Submission {
                                remote_id: Some(123),
                                remote_state: Some(4),
                                status: SubmissionStatus::Assigned,
                                ..Default::default()
                            });
                            self.screen = Screen::Item(1);
                            self.item_id = Some("42".into());
                            if let Some(picker) = &mut self.drive_picker {
                                picker.item = Some(42);
                            }
                        }
                        return schedule_capture(Shot::SubmissionDrivePicker);
                    }
                    Shot::SubmissionDrivePicker => {
                        self.screen = Screen::Course(1);
                        self.item_id = None;
                        self.drive_picker = None;
                        self.open_stream_composer = None;
                        if let Some(post) = self.snapshot.as_mut().and_then(|snapshot| snapshot.coursework.get_mut(&1))
                            .and_then(|items| items.iter_mut().find(|item| item.kind == CourseworkKind::Announcement))
                        {
                            post.id = "91".into();
                            post.author_id = Some(1);
                            self.stream_viewer_id = Some(1);
                            self.confirming_stream_delete = Some((1, 91));
                        }
                        return schedule_capture(Shot::StreamDelete);
                    }
                    Shot::StreamDelete => {
                        self.confirming_stream_delete = None;
                        if let Some(comment) = self.snapshot.as_mut()
                            .and_then(|snapshot| snapshot.coursework.get_mut(&1))
                            .and_then(|items| items.iter_mut().find(|item| item.id == "91"))
                            .and_then(|item| item.comments.as_mut()).and_then(|comments| comments.first_mut())
                        {
                            comment.id = "92".into();
                            comment.author_id = Some(1);
                            self.stream_viewer_id = Some(1);
                            self.screen = Screen::Item(1);
                            self.item_id = Some("91".into());
                            self.confirming_comment_delete = Some(CommentTarget { course: 1, item: 91, id: 92, audience: CommentAudience::Class });
                        }
                        return schedule_capture(Shot::CommentDelete);
                    }
                    Shot::CommentDelete => {
                        self.confirming_comment_delete = None;
                        self.screen = Screen::Course(1);
                        self.item_id = None;
                        return window::latest().then(|id| match id {
                            Some(id) => window::resize(id, iced::Size::new(810.0, 565.0))
                                .chain(schedule_capture(Shot::CompactStream)),
                            None => Task::none(),
                        });
                    }
                    Shot::CompactStream => {
                        self.screen = Screen::Home;
                        self.show_hidden_classes = true;
                        if let Some(snapshot) = &mut self.snapshot {
                            snapshot.courses.iter_mut().for_each(|course| {
                                course.hidden = Some(course.id == 5);
                            });
                            snapshot.courses.retain(|course| course.id <= 3 || course.id == 5);
                        }
                        self.open_course_menu = Some(5);
                        return window::latest().then(|id| match id {
                            Some(id) => window::resize(id, iced::Size::new(1200.0, 1020.0))
                                .chain(schedule_capture(Shot::HiddenClasses)),
                            None => Task::none(),
                        });
                    }
                    Shot::HiddenClasses => {
                        self.open_course_menu = Some(1);
                        self.move_course_menu = Some(1);
                        return window::latest().then(|id| match id {
                            Some(id) => window::resize(id, iced::Size::new(1200.0, 810.0))
                                .chain(schedule_capture(Shot::MoveCourse)),
                            None => Task::none(),
                        });
                    }
                    Shot::MoveCourse => {
                        self.move_course_menu = None;
                        self.class_roles.insert(1, CourseRole::Teacher);
                        self.confirm_class_archive = Some(1);
                        return schedule_capture(Shot::ArchiveClass);
                    }
                    Shot::ArchiveClass => {
                        self.screen = Screen::Home;
                        self.open_course_menu = None;
                        self.move_course_menu = None;
                        self.confirm_class_archive = None;
                        self.class_roles.remove(&1);
                        self.classroom_link_input = ClassroomLink::Item { course: 1, item: 42, kind: ItemLinkKind::Assignment }.web_url();
                        return schedule_capture(Shot::ClassroomLink);
                    }
                    Shot::ClassroomLink => {
                        self.classroom_link_input = None;
                        self.screen = Screen::Course(1);
                        self.tab = Tab::Stream;
                        self.set_demo_stream_display(StreamClassworkDisplay::Condensed);
                        return schedule_capture(Shot::CondensedStream);
                    }
                    Shot::CondensedStream => {
                        self.set_demo_stream_display(StreamClassworkDisplay::Expanded);
                        if let Some(item) = self.snapshot.as_mut().and_then(|snapshot| snapshot.coursework.get_mut(&1))
                            .and_then(|items| items.iter_mut().find(|item| item.kind == CourseworkKind::Assignment))
                        {
                            item.attachments = vec![desktop_data::Attachment {
                                title: "Poetry reading notes".into(),
                                kind: desktop_data::AttachmentKind::Link,
                                url: Some("https://example.org/reading".into()),
                                remote_id: None,
                                drive_sharing: None,
                                drive_reference: None,
                                drive_parent_reference: None,
                            }];
                        }
                        return schedule_capture(Shot::ExpandedStream);
                    }
                    Shot::ExpandedStream => {
                        self.set_demo_stream_display(StreamClassworkDisplay::Hidden);
                        return schedule_capture(Shot::HiddenStream);
                    }
                    Shot::HiddenStream => {
                        if let Some(item) = self.snapshot.as_mut().and_then(|snapshot| snapshot.coursework.get_mut(&1))
                            .and_then(|items| items.iter_mut().find(|item| item.kind == CourseworkKind::Assignment))
                        {
                            item.id = "42".into();
                            item.submission = Some(Submission {
                                remote_id: Some(123), remote_state: Some(4), status: SubmissionStatus::Assigned,
                                ..Default::default()
                            });
                            self.screen = Screen::Item(1);
                            self.item_id = Some("42".into());
                            self.pending_submission = Some((1, 42, desktop_auth::SubmissionAction::TurnIn));
                        }
                        return schedule_capture(Shot::MarkDone);
                    }
                    Shot::MarkDone => {
                        self.pending_submission = None;
                        self.new_submission_document_open = true;
                        return schedule_capture(Shot::NewSubmissionDocument);
                    }
                    Shot::NewSubmissionDocument => {
                        #[cfg(target_os = "macos")]
                        {
                            let colours = [[224, 226, 230, 255], [215, 187, 101, 255], [111, 184, 191, 255],
                                [115, 179, 138, 255], [172, 137, 181, 255], [202, 119, 117, 255], [104, 142, 192, 255]];
                            let (width, height) = (420, 240);
                            let pixels: Vec<_> = (0..height).flat_map(|_| (0..width)
                                .flat_map(|x| colours[x * colours.len() / width])).collect();
                            self.new_submission_document_open = false;
                            self.submission_capture = Some(CaptureState::RecordingCamera {
                                control: camera::Control::screenshot_fixture(), phase: CameraPhase::Framing,
                                frame: Some(image::Handle::from_rgba(width as u32, height as u32, pixels)),
                            });
                            return schedule_capture(Shot::CameraFraming);
                        }
                    }
                    #[cfg(target_os = "macos")]
                    Shot::CameraFraming => {
                        self.discard_submission_capture();
                        if let Some(item) = self.snapshot.as_mut().and_then(|snapshot| snapshot.coursework.get_mut(&1))
                            .and_then(|items| items.iter_mut().find(|item| item.kind == CourseworkKind::Assignment))
                            && let Some(submission) = &mut item.submission
                        {
                            let template = desktop_data::Attachment {
                                remote_id: None,
                                title: "Poetry comparison worksheet".into(),
                                url: Some("https://docs.google.com/document/d/demo-worksheet/edit".into()),
                                kind: desktop_data::AttachmentKind::DriveFile,
                                drive_sharing: Some(DriveSharing::IndividualCopy),
                                drive_reference: Some("demo-worksheet".into()),
                                drive_parent_reference: None,
                            };
                            submission.attachments.push(desktop_data::Attachment {
                                title: "Sam — Poetry comparison worksheet".into(),
                                url: Some("https://docs.google.com/document/d/demo-personal-worksheet/edit".into()),
                                drive_sharing: None,
                                drive_reference: Some("demo-personal-worksheet".into()),
                                drive_parent_reference: template.drive_reference.clone(),
                                ..template.clone()
                            });
                            item.attachments.push(template);
                            item.originality_enabled = Some(true);
                        }
                        return schedule_capture(Shot::PersonalCopy);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::PersonalCopy => {
                        self.screen = Screen::Course(1);
                        self.tab = Tab::Classwork;
                        self.class_roles.insert(1, CourseRole::Teacher);
                        if let Some(item) = self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(1))
                            .and_then(|items| items.iter().find(|item| item.kind == CourseworkKind::Assignment))
                        {
                            let mut draft = item.clone();
                            draft.title = "Poetry workshop".into();
                            draft.id = "draft-poetry".into();
                            let mut scheduled = draft.clone();
                            scheduled.title = "Comparative essay".into();
                            scheduled.id = "scheduled-essay".into();
                            self.teacher_drafts.insert(1, vec![
                                desktop_auth::TeacherDraft { item: draft, publication: desktop_auth::DraftPublication::Draft },
                                desktop_auth::TeacherDraft { item: scheduled, publication: desktop_auth::DraftPublication::Scheduled(Some(1_800_000_000_000)) },
                            ]);
                        }
                        return schedule_capture(Shot::TeacherDrafts);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::TeacherDrafts => {
                        let mut editor = AssignmentEditor::new(1, None);
                        editor.title = "Poetry comparison".into();
                        editor.instructions = text_editor::Content::with_text("Compare how two poets present a shared theme.");
                        editor.settings.due_date = "2027-01-26".into();
                        editor.settings.due_time = "16:00".into();
                        self.assignment_editor = Some(editor);
                        return schedule_capture(Shot::AssignmentEditor);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::AssignmentEditor => {
                        self.assignment_editor = None;
                        self.assignment_publication = self.teacher_drafts.get(&1).and_then(|drafts| drafts.iter()
                            .find(|draft| draft.publication == desktop_auth::DraftPublication::Draft))
                            .map(|draft| AssignmentPublicationPrompt { draft: draft.clone(), schedule: Some("2027-01-19 09:00".into()) });
                        return schedule_capture(Shot::AssignmentSchedule);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::AssignmentSchedule => {
                        self.assignment_publication = None;
                        self.screen = Screen::Item(1);
                        self.item_id = Some("42".into());
                        let row = |student_id, name: &str, status, released_grade, draft_grade| desktop_auth::TeacherSubmission {
                            student_id, submission_id: student_id + 100,
                            remote_state: Some(match status { SubmissionStatus::Returned => 3, SubmissionStatus::TurnedIn => 2, _ => 4 }),
                            name: Some(name.into()), status, released_grade, draft_grade, attachments: Vec::new(), answer: None,
                        };
                        self.teacher_work = Some(TeacherWork {
                            course: 1, item: 42, filter: StudentWorkFilter::All, selected: Some(12),
                            grade: "16".into(), confirming_return: true,
                            bulk_selected: BTreeSet::new(), bulk_confirming: false,
                            students: vec![
                                row(11, "Alex Bell", SubmissionStatus::Returned, Some(18.0), None),
                                row(12, "Jamie Lane", SubmissionStatus::TurnedIn, None, Some(16.0)),
                                row(13, "Morgan Vale", SubmissionStatus::Assigned, None, None),
                                row(14, "Sam Ellis", SubmissionStatus::Missing, None, None),
                            ],
                        });
                        if let Some(item) = self.snapshot.as_mut().and_then(|snapshot| snapshot.coursework.get_mut(&1))
                            .and_then(|items| items.iter_mut().find(|item| item.id == "42"))
                        {
                            item.maximum_points = Some(20.0);
                        }
                        return schedule_capture(Shot::TeacherStudentWork);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::TeacherStudentWork => {
                        self.screen = Screen::Course(1);
                        self.tab = Tab::Grades;
                        if let Some(items) = self.snapshot.as_mut().and_then(|snapshot| snapshot.coursework.get_mut(&1))
                            && let Some(template) = items.iter().find(|item| item.id == "42").cloned()
                        {
                            for (id, title) in [("43", "Poetry workshop"), ("44", "Comparative essay")] {
                                let mut item = template.clone();
                                item.id = id.into();
                                item.title = title.into();
                                items.push(item);
                            }
                        }
                        if let Some(work) = self.teacher_work.take() {
                            let students = work.students.iter().map(|row| CourseMember {
                                id: row.student_id.to_string(), display_name: row.name.clone().unwrap_or_default(), role: CourseRole::Student,
                            }).collect();
                            let cells = work.students.into_iter().flat_map(|row| {
                                let mark = desktop_auth::TeacherGradebookCell {
                                    submission_id: row.submission_id, remote_state: row.remote_state,
                                    status: row.status, released_grade: row.released_grade, draft_grade: row.draft_grade,
                                };
                                [((row.student_id, 42), mark),
                                    ((row.student_id, 43), desktop_auth::TeacherGradebookCell { status: SubmissionStatus::Assigned,
                                        submission_id: row.submission_id + 100, remote_state: Some(4), released_grade: None, draft_grade: None }),
                                    ((row.student_id, 44), desktop_auth::TeacherGradebookCell { status: SubmissionStatus::Returned,
                                        submission_id: row.submission_id + 200, remote_state: Some(3), released_grade: Some(20.0), draft_grade: None })]
                            }).collect();
                            self.gradebooks.insert(1, desktop_auth::TeacherGradebook { students, cells });
                        }
                        return schedule_capture(Shot::TeacherGradebook);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::TeacherGradebook => {
                        self.gradebook_edit_mode = true;
                        self.gradebook_edit = self.gradebooks.get(&1).and_then(|grid| grid.cells.get(&(12, 42)))
                            .map(|cell| GradebookEdit { course: 1, item: 42, expected: cell.grading_row(12), input: "17.5".into() });
                        return schedule_capture(Shot::TeacherGradebookEdit);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::TeacherGradebookEdit => {
                        self.gradebook_edit = None;
                        self.screen = Screen::Item(1);
                        self.item_id = Some("42".into());
                        let students = self.gradebooks.get(&1).map(|grid| grid.students.iter().filter_map(|student| {
                            let id = student.id.parse().ok()?;
                            let mut row = grid.cells.get(&(id, 42))?.grading_row(id);
                            row.name = Some(student.display_name.clone());
                            if id == 14 {
                                row.status = SubmissionStatus::TurnedIn;
                                row.remote_state = Some(2);
                            }
                            Some(row)
                        }).collect()).unwrap_or_default();
                        self.teacher_work = Some(TeacherWork { course: 1, item: 42, students, filter: StudentWorkFilter::All,
                            selected: None, grade: String::new(), confirming_return: false,
                            bulk_selected: BTreeSet::from([12, 14]), bulk_confirming: true });
                        return schedule_capture(Shot::BulkReturn);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::BulkReturn => {
                        if let Some(work) = &mut self.teacher_work {
                            work.bulk_selected.clear();
                            work.bulk_confirming = false;
                            work.select(Some(12));
                        }
                        if let Some(key) = self.selected_teacher_feedback_key() {
                            self.teacher_feedback.insert(key, TeacherFeedbackThread {
                                comments: Some(vec![Comment {
                                    id: "57".into(), author_id: None, author: "Jamie Lane".into(),
                                    body: "I added the quotations from both poems.".into(), created_label: None,
                                }]),
                                draft: "Thanks. Please explain how your second quotation supports your comparison.".into(),
                                unconfirmed: false,
                            });
                        }
                        return schedule_capture(Shot::TeacherFeedback);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::TeacherFeedback => {
                        self.teacher_work = None;
                        self.teacher_feedback.clear();
                        if let Some(post) = self.snapshot.as_mut().and_then(|snapshot| snapshot.coursework.get_mut(&1))
                            .and_then(|items| items.iter_mut().find(|item| item.kind == CourseworkKind::Announcement))
                        {
                            post.id = "77".into();
                            post.author_id = Some(900);
                            post.comments = Some(vec![Comment {
                                id: "81".into(), author_id: Some(901), author: "Sam Ellis".into(),
                                body: "Is the workshop still happening on Friday?".into(), created_label: None,
                            }]);
                            self.item_id = Some("77".into());
                            self.confirming_comment_delete = Some(CommentTarget { course: 1, item: 77, id: 81, audience: CommentAudience::Class });
                        }
                        return schedule_capture(Shot::TeacherModerationComment);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::TeacherModerationComment => {
                        self.screen = Screen::Course(1);
                        self.tab = Tab::Stream;
                        self.confirming_comment_delete = None;
                        self.confirming_stream_delete = Some((1, 77));
                        return schedule_capture(Shot::TeacherModerationPost);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::TeacherModerationPost => {
                        self.tab = Tab::ClassSettings;
                        self.confirming_stream_delete = None;
                        let original = desktop_auth::TeacherClassSettings {
                            details: desktop_auth::ClassDetails { title: "English Literature".into(),
                                subtitle: "Year 12 · Group A".into(), description: "Read and discuss poetry, prose, and drama.".into(),
                                room: "B14".into() },
                            stream: desktop_auth::CourseStreamSettings { posting: Some(desktop_auth::StreamPostingPolicy::PostAndComment),
                                classwork: Some(StreamClassworkDisplay::Condensed) },
                        };
                        let mut editor = ClassStreamSettingsEditor::new(1, original);
                        editor.draft.details.room = "B16".into();
                        editor.draft.stream.posting = Some(desktop_auth::StreamPostingPolicy::CommentOnly);
                        editor.draft.stream.classwork = Some(StreamClassworkDisplay::Expanded);
                        self.class_stream_settings = Some(editor);
                        return schedule_capture(Shot::ClassStreamSettings);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::ClassStreamSettings => {
                        self.tab = Tab::People;
                        if let Some(member) = self.snapshot.as_mut().and_then(|snapshot| snapshot.roster.get_mut(&1))
                            .and_then(|members| members.iter_mut().find(|member| member.role == CourseRole::Student))
                        {
                            member.id = "81".into();
                            self.student_muting.insert(1, BTreeMap::from([(81, false)]));
                            self.pending_student_mute = Some((1, 81, false));
                        }
                        return schedule_capture(Shot::StudentMuting);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::StudentMuting => {
                        self.tab = Tab::Classwork;
                        self.topic_manager = true;
                        let topic = desktop_data::Topic { id: "demo-poetry-unit".into(), title: "Poetry".into() };
                        self.teacher_topics.insert(1, vec![topic.clone(), desktop_data::Topic {
                            id: "demo-reading".into(), title: "Independent reading".into(),
                        }]);
                        self.topic_editor = Some((1, desktop_auth::TeacherTopicAction::Delete(topic)));
                        return schedule_capture(Shot::TopicManagement);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::TopicManagement => {
                        self.topic_manager = false;
                        self.topic_editor = None;
                        if let Some(item) = self.snapshot.as_ref().and_then(|snapshot| snapshot.coursework_for(1))
                            .and_then(|items| items.iter().find(|item| item.kind == CourseworkKind::Assignment))
                        {
                            self.item_topic_editor = Some(ItemTopicEditor {
                                target: desktop_auth::TopicItemTarget { course: 1, item: item.id.parse().unwrap_or(42),
                                    kind: item.kind, draft: false, original: item.topic_id.clone() },
                                title: item.title.clone(), selected: Some("demo-reading".into()),
                            });
                        }
                        return schedule_capture(Shot::ItemTopic);
                    }
                    #[cfg(target_os = "macos")]
                    Shot::ItemTopic => {}
                }
                return iced::exit();
            }
            _ => {}
        }
        self.finish_update()
    }
}
