use super::*;
use chrono::Local;

impl Snapshot {
    /// Synthetic classes. Never write these to an account's cache.
    pub fn demo() -> Self {
        let courses = [
            (1, "English Literature", "Year 12 · Group A", "Read and discuss poetry, plays, and prose from different periods.", "E12", Some([44, 95, 134])),
            (2, "Mathematics", "Year 12 · Group B", "Explore algebra, functions, statistics, and applied mathematics.", "M04", Some([32, 107, 88])),
            (3, "Computer Science", "Year 12 · Group A", "Programming, data representation, and how computers work.", "ICT 2", Some([90, 64, 137])),
            (4, "History", "Year 12 · Group A", "Study how evidence shapes our understanding of the past.", "H03", Some([134, 65, 54])),
            (5, "Physics", "Year 12 · Group B", "Learn about motion, waves, and electricity through practical work.", "Lab 3", Some([37, 101, 120])),
            (6, "Art & Design", "Year 12 · Group A", "Develop a portfolio through drawing, research, and experiments in different media.", "Studio 1", None),
        ]
        .into_iter()
        .map(|(id, title, subtitle, description, room, color_rgb)| Course {
            id,
            title: title.into(),
            subtitle: subtitle.into(),
            description: description.into(),
            room: room.into(),
            state: if id == 6 { CourseState::Archived } else { CourseState::Active },
            hidden: None,
            reordering_sort_key: None,
            theme: color_rgb.map(|color| CourseTheme {
                color_rgb: Some(color),
                artwork_url: None,
            }),
            meet_url: None,
            calendar_url: None,
            stream_classwork_display: None,
            owner_ids: vec![],
        })
        .collect();
        let mut coursework: BTreeMap<i64, Vec<Coursework>> = (1..=6)
            .map(|course_id| (course_id, Vec::new()))
            .collect();
        let monday = monday_of(Local::now().date_naive());
        for (course_id, id, kind, title, description, due_weekday) in [
            (1, "demo-poetry-notice", CourseworkKind::Announcement, "This week in English", "Bring your annotated poem to our next lesson. We’ll compare interpretations before starting the assignment.", None),
            (1, "demo-poetry", CourseworkKind::Assignment, "Poetry comparison", "Compare how two poems present a shared theme. Support your argument with examples from each text.", Some(Weekday::Fri)),
            (1, "demo-poetry-reading", CourseworkKind::Material, "Selected poems and discussion prompts", "Read the poems and jot down three questions before class.", None),
            (2, "demo-functions", CourseworkKind::Assignment, "Functions practice", "Sketch the graphs and explain how the parameters transform them.", Some(Weekday::Wed)),
            (3, "demo-sort", CourseworkKind::Assignment, "Comparing sorting algorithms", "Implement two sorting algorithms and explain where each performs well.", Some(Weekday::Thu)),
            (3, "demo-source", CourseworkKind::Material, "Programming workshop notes", "A reminder of the examples and terminology from this week’s session.", None),
            (4, "demo-historical", CourseworkKind::Question, "Comparing historical accounts", "What can each source tell us about the event? Consider who wrote it and why.", None),
            (5, "demo-waves", CourseworkKind::Assignment, "Waves practical", "Summarise your measurements and discuss the main sources of uncertainty.", Some(Weekday::Mon)),
            (6, "demo-portfolio", CourseworkKind::Announcement, "Portfolio planning session", "Bring two pieces you would like to develop further.", None),
        ] {
            let due_date = due_weekday.and_then(|weekday| {
                monday.checked_add_days(Days::new(weekday.num_days_from_monday().into()))
            });
            coursework.entry(course_id).or_default().push(Coursework {
                id: id.into(),
                course_id,
                title: title.into(),
                description: description.into(),
                description_styles: vec![],
                description_bullets: vec![],
                due_label: due_date.map(|date| format!("Due {}", date.format("%A"))),
                due_at_millis: due_date.and_then(|date| date.and_hms_opt(0, 0, 0))
                    .map(|date| date.and_utc().timestamp_millis()),
                due_has_time: due_date.map(|_| false),
                maximum_points: None,
                kind,
                author_id: None,
                posted_at_millis: None,
                question: (kind == CourseworkKind::Question).then_some(QuestionDetails::ShortAnswer),
                question_sharing: None,
                originality_enabled: None,
                allow_edit_after_turn_in: false,
                topic_id: (course_id == 1 && kind != CourseworkKind::Announcement)
                    .then(|| "demo-poetry-unit".into()),
                classwork_sort_key: None,
                attachments: Vec::new(),
                submission: None,
                comments: None,
            });
        }
        if let Some(english) = coursework.get_mut(&1) {
            for item in english {
                match item.id.as_str() {
                    "demo-poetry-notice" => {
                        item.comments = Some(vec![Comment {
                            id: "demo-comment-1".into(),
                            author_id: None,
                            author: "Morgan Lane".into(),
                            body: "Thanks! I’ll bring my annotated copy.".into(),
                            created_label: Some("Monday".into()),
                        }]);
                    }
                    "demo-poetry" => {
                        item.submission = Some(Submission {
                            remote_id: None,
                            remote_state: None,
                            attachments: Vec::new(),
                            status: SubmissionStatus::Assigned,
                            grade: None,
                            last_turned_in_millis: None,
                            question_answer: None,
                            rubric_scores: BTreeMap::new(),
                        });
                    }
                    "demo-poetry-reading" => item.attachments.push(Attachment {
                        remote_id: None,
                        title: "Poetry notes".into(),
                        url: None,
                        kind: AttachmentKind::DriveFile,
                        drive_sharing: None,
                        drive_reference: None,
                        drive_parent_reference: None,
                    }),
                    _ => {}
                }
            }
        }
        if let Some(items) = coursework.get_mut(&2) {
            if let Some(item) = items.first_mut() {
                item.submission = Some(Submission {
                    remote_id: None,
                    remote_state: None,
                    attachments: Vec::new(),
                    status: SubmissionStatus::Returned,
                    grade: Some(Grade { earned: 18.0, maximum: Some(20.0) }),
                    last_turned_in_millis: None,
                    question_answer: None,
                    rubric_scores: BTreeMap::new(),
                });
            }
        }
        for (course_id, status) in [(4, SubmissionStatus::Assigned), (5, SubmissionStatus::Missing)] {
            if let Some(item) = coursework.get_mut(&course_id).and_then(|items| items.first_mut()) {
                item.submission = Some(Submission { status, ..Default::default() });
            }
        }
        let topic = Topic { id: "demo-poetry-unit".into(), title: "Poetry".into() };
        let topics = BTreeMap::from([(1, vec![topic])]);
        let roster = BTreeMap::from([(1, vec![
            CourseMember {
                id: "demo-teacher-1".into(),
                display_name: "Taylor Ellis".into(),
                role: CourseRole::Teacher,
            },
            CourseMember {
                id: "demo-student-1".into(),
                display_name: "Morgan Lane".into(),
                role: CourseRole::Student,
            },
        ])]);
        Self { source: SnapshotSource::Demo, courses, coursework, topics, roster, updated_at: None }
    }
}
