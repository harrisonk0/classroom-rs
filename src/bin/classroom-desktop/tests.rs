use super::*;

#[test]
fn confirmed_topic_deletion_preserves_posts_and_keeps_empty_topics_out_of_the_published_cache() {
    let (mut app, snapshot) = fixture();
    let original = snapshot.coursework_for(1).unwrap().to_vec();
    let id = original.iter().find_map(|item| item.topic_id.clone()).unwrap();
    app.install_snapshot(snapshot);
    app.screen = Screen::Course(1);
    app.selected_topic = Some(id.clone());
    let empty = desktop_data::Topic { id: "empty".into(), title: "Independent reading".into() };
    app.apply_teacher_topics(1, vec![empty.clone()], Some(&id));
    let snapshot = app.snapshot.as_ref().unwrap();
    assert_eq!(snapshot.coursework_for(1).unwrap().len(), original.len());
    for (item, old) in snapshot.coursework_for(1).unwrap().iter().zip(original) {
        assert_eq!(item.id, old.id);
        assert_eq!(item.title, old.title);
        assert_ne!(item.topic_id.as_deref(), Some(id.as_str()));
    }
    assert!(snapshot.topics_for(1).unwrap().is_empty());
    assert_eq!(app.teacher_topics[&1], [empty]);
    assert!(app.selected_topic.is_none());
}

#[test]
fn class_settings_queues_the_read_only_with_a_teacher_role() {
    let (mut app, snapshot) = fixture();
    app.screen = Screen::Course(1);
    app.tab = Tab::ClassSettings;
    app.class_roles.insert(1, CourseRole::Student);
    assert_eq!(app.next_page_read(&snapshot), None);
    app.class_roles.insert(1, CourseRole::Teacher);
    assert_eq!(app.next_page_read(&snapshot), Some(PageRead::Detail(DetailRead::ClassStreamSettings(1))));
}

#[test]
fn only_a_teacher_of_the_class_can_moderate_another_members_public_comment() {
    let (mut app, _) = fixture();
    let comment = Comment { id: "17".into(), author_id: Some(10), author: "Class member".into(),
        body: "A test comment".into(), created_label: None };
    app.stream_viewer_id = Some(12);
    app.class_roles.insert(1, CourseRole::Student);
    app.class_roles.insert(2, CourseRole::Teacher);
    assert!(!app.may_delete_visible_comment(1, CommentAudience::Class, &comment));
    assert!(app.may_delete_visible_comment(2, CommentAudience::Class, &comment));
    assert!(!app.may_delete_visible_comment(2, CommentAudience::Private, &comment));
    app.stream_viewer_id = Some(10);
    assert!(app.may_delete_visible_comment(1, CommentAudience::Class, &comment));
}

#[test]
fn multiline_comment_drafts_keep_unicode_and_separate_threads() {
    let mut drafts = BTreeMap::new();
    let insert = |value: &str| text_editor::Action::Edit(text_editor::Edit::Paste(Arc::new(value.into())));
    edit_comment_draft(&mut drafts, (1, 42), insert("First 📝\n\nSecond line"));
    edit_comment_draft(&mut drafts, (1, 43), insert("Other thread"));
    assert_eq!(drafts[&(1, 42)].text(), "First 📝\n\nSecond line");
    assert_eq!(drafts[&(1, 43)].text(), "Other thread");
    edit_comment_draft(&mut drafts, (1, 42), insert(&"x".repeat(10_000)));
    assert_eq!(drafts[&(1, 42)].text(), "First 📝\n\nSecond line");
}

#[test]
fn failed_comment_send_preserves_multiline_drafts_until_confirmed() {
    let (mut app, snapshot) = fixture();
    app.install_snapshot(snapshot);
    let key = (1, 42);
    for drafts in [&mut app.comment_drafts, &mut app.private_drafts] {
        drafts.insert(key, text_editor::Content::with_text("First 📝\nSecond"));
    }
    let _ = app.update(Message::CommentPosted(1, 42, Err("Unconfirmed".into())));
    let _ = app.update(Message::PrivateCommentPosted(1, 42, Err("Unconfirmed".into())));
    assert_eq!(app.comment_drafts[&key].text(), "First 📝\nSecond");
    assert_eq!(app.private_drafts[&key].text(), "First 📝\nSecond");
    let _ = app.update(Message::Navigate(Screen::Home));
    assert_eq!(app.comment_drafts[&key].text(), "First 📝\nSecond");
    assert_eq!(app.private_drafts[&key].text(), "First 📝\nSecond");
    let _ = app.update(Message::CommentPosted(1, 42, Ok(())));
    let _ = app.update(Message::PrivateCommentPosted(1, 42, Ok(())));
    assert!(!app.comment_drafts.contains_key(&key));
    assert!(!app.private_drafts.contains_key(&key));
}

#[test]
fn switching_student_keeps_private_drafts_on_their_original_submissions() {
    let (mut app, _) = fixture();
    let row = |student_id| desktop_auth::TeacherSubmission {
        student_id, submission_id: 100 + student_id, remote_state: Some(2), name: None,
        status: SubmissionStatus::TurnedIn, released_grade: None, draft_grade: None, attachments: Vec::new(), answer: None,
    };
    app.screen = Screen::Item(1);
    app.item_id = Some("42".into());
    app.class_roles.insert(1, CourseRole::Teacher);
    app.teacher_work = Some(TeacherWork {
        course: 1, item: 42, filter: StudentWorkFilter::All, students: vec![row(7), row(9)], selected: Some(7),
        grade: String::new(), confirming_return: false, bulk_selected: BTreeSet::new(), bulk_confirming: false,
    });
    let first = app.selected_teacher_feedback_key().unwrap();
    let _ = app.update(Message::EditTeacherFeedback(first, "First thread".into()));
    let _ = app.update(Message::SelectTeacherStudent(9));
    let second = app.selected_teacher_feedback_key().unwrap();
    let _ = app.update(Message::EditTeacherFeedback(first, "Stale edit".into()));
    let _ = app.update(Message::EditTeacherFeedback(second, "Second thread".into()));
    assert_eq!(app.teacher_feedback[&first].draft, "First thread");
    assert_eq!(app.teacher_feedback[&second].draft, "Second thread");
    let _ = app.update(Message::CloseItem);
    assert!(app.teacher_feedback.is_empty());
}

#[test]
fn bulk_return_selection_respects_filter_and_skips_unreturnable_rows() {
    let (mut app, _) = fixture();
    let row = |student_id, status, remote_state| desktop_auth::TeacherSubmission {
        student_id, submission_id: 100 + student_id, remote_state: Some(remote_state),
        name: None, status, released_grade: None, draft_grade: None, attachments: Vec::new(), answer: None,
    };
    app.teacher_work = Some(TeacherWork {
        course: 1, item: 42, filter: StudentWorkFilter::TurnedIn, selected: None, grade: String::new(),
        confirming_return: false, bulk_confirming: false, bulk_selected: BTreeSet::new(),
        students: vec![row(1, SubmissionStatus::TurnedIn, 2), row(2, SubmissionStatus::Assigned, 4)],
    });
    let _ = app.update(Message::SelectVisibleReturns(true));
    let _ = app.update(Message::SelectBulkReturn(2, true));
    assert_eq!(app.teacher_work.as_ref().unwrap().bulk_selected, BTreeSet::from([1]));
    let _ = app.update(Message::FilterStudentWork(StudentWorkFilter::Assigned));
    let _ = app.update(Message::ConfirmBulkReturn(true));
    assert!(app.teacher_work.as_ref().unwrap().bulk_confirming);
    let _ = app.update(Message::ClearBulkReturns);
    assert!(app.teacher_work.as_ref().unwrap().bulk_selected.is_empty());
}

#[test]
fn gradebook_advances_in_roster_order_past_unavailable_cells() {
    let student = |id: i64| CourseMember { id: id.to_string(), display_name: id.to_string(), role: CourseRole::Student };
    let cell = |submission_id, remote_state| desktop_auth::TeacherGradebookCell {
        submission_id, remote_state, status: SubmissionStatus::Assigned, released_grade: None, draft_grade: None,
    };
    let grid = desktop_auth::TeacherGradebook {
        students: vec![student(5), student(9), student(4), student(3)],
        cells: BTreeMap::from([
            ((5, 8), cell(100, Some(4))), ((9, 8), cell(101, Some(6))),
            ((4, 10), cell(102, Some(4))), ((3, 8), cell(103, Some(2))),
        ]),
    };
    assert_eq!(next_editable_gradebook_student(&grid, 8, 5), Some(3));
    assert_eq!(next_editable_gradebook_student(&grid, 8, 3), None);
}

#[test]
fn published_item_queues_class_submissions_only_for_a_confirmed_teacher() {
    let (mut app, mut snapshot) = fixture();
    let item = snapshot.coursework.get_mut(&1).unwrap().iter_mut()
        .find(|row| row.kind == CourseworkKind::Assignment).unwrap();
    item.id = "42".into();
    app.screen = Screen::Item(1);
    app.item_id = Some(item.id.clone());
    app.class_roles.insert(1, CourseRole::Student);
    assert_ne!(app.next_page_read(&snapshot), Some(PageRead::Detail(DetailRead::TeacherSubmissions(1, 42))));
    app.class_roles.insert(1, CourseRole::Teacher);
    assert_eq!(app.next_page_read(&snapshot), Some(PageRead::Detail(DetailRead::TeacherSubmissions(1, 42))));
}

#[test]
fn schedule_input_interprets_the_time_in_the_displayed_local_timezone() {
    let zone = chrono::FixedOffset::east_opt(3600).unwrap();
    let expected = DateTime::parse_from_rfc3339("2027-12-21T12:15:00Z").unwrap().timestamp_millis();
    assert_eq!(local_schedule_millis(&zone, "2027-12-21 13:15"), Some(expected));
    assert_eq!(local_schedule_millis(&zone, "2027-02-29 13:15"), None);
    assert_eq!(local_schedule_millis(&zone, "tomorrow"), None);
}

#[test]
fn classwork_queues_unpublished_reads_only_for_a_confirmed_teacher() {
    let (mut app, snapshot) = fixture();
    app.screen = Screen::Course(1);
    app.tab = Tab::Classwork;
    for role in [None, Some(CourseRole::Student), Some(CourseRole::Teacher)] {
        if let Some(role) = role {
            app.class_roles.insert(1, role);
        }
        let read = app.next_page_read(&snapshot);
        assert_eq!(read, Some(PageRead::Detail(if role == Some(CourseRole::Teacher) {
            DetailRead::TeacherTopics(1)
        } else {
            DetailRead::Submissions(1)
        })));
    }
    app.teacher_topics.insert(1, Vec::new());
    assert_eq!(app.next_page_read(&snapshot), Some(PageRead::Detail(DetailRead::TeacherDrafts(1))));
}

#[test]
fn personal_copy_checks_reset_with_snapshot_and_discard_stale_reads() {
    let (mut app, snapshot) = fixture();
    app.install_snapshot(snapshot.clone());
    app.personal_copy_checks.insert((1, 42), ("template".into(), true));
    app.pending_personal_copy = Some((1, 42, "template".into()));
    app.personal_copy_loading_generation = Some(app.personal_copy_generation);
    app.install_snapshot(snapshot);
    assert!(app.personal_copy_checks.is_empty());
    assert!(app.pending_personal_copy.is_none());
    let _ = app.update(Message::DetailLoaded(DetailRead::PersonalCopy(1, 42),
        Ok(DetailData::PersonalCopy("template".into(), true))));
    assert!(app.personal_copy_checks.is_empty());
    app.personal_copy_checks.insert((1, 42), ("template".into(), true));
    app.pending_personal_copy = Some((1, 42, "template".into()));
    let _ = app.update(Message::SessionForgotten(Ok(())));
    assert!(app.personal_copy_checks.is_empty());
    assert!(app.pending_personal_copy.is_none());
}

#[test]
fn personal_copy_preparation_requires_one_unambiguous_template_and_no_submission() {
    let (_, snapshot) = fixture();
    let mut work = snapshot.coursework_for(1).unwrap().iter()
        .find(|item| item.kind == CourseworkKind::Assignment).unwrap().clone();
    work.submission = None;
    let template = desktop_data::Attachment {
        title: "Worksheet".into(), remote_id: None, kind: desktop_data::AttachmentKind::DriveFile,
        url: None, drive_sharing: Some(DriveSharing::IndividualCopy),
        drive_reference: Some("template-id".into()), drive_parent_reference: None,
    };
    work.attachments = vec![template.clone()];
    assert_eq!(eligible_personal_copy_template(&work).as_deref(), Some("template-id"));
    work.attachments.push(template.clone());
    assert_eq!(eligible_personal_copy_template(&work), None);
    work.attachments[1].drive_reference = Some("other-template".into());
    assert_eq!(eligible_personal_copy_template(&work).as_deref(), Some("template-id"));
    work.submission = Some(Submission::default());
    assert_eq!(eligible_personal_copy_template(&work), None);
}

#[test]
fn a_personal_template_only_opens_an_exact_current_submission_copy() {
    let (mut app, snapshot) = fixture();
    app.verify_live_course_count = false;
    let mut item = snapshot.coursework_for(1).unwrap().iter()
        .find(|item| item.kind == CourseworkKind::Assignment).unwrap().clone();
    let template = desktop_data::Attachment {
        title: "Worksheet".into(), remote_id: None, kind: desktop_data::AttachmentKind::DriveFile,
        url: Some("https://docs.google.com/document/d/source/edit".into()),
        drive_sharing: Some(DriveSharing::IndividualCopy), drive_reference: Some("source".into()),
        drive_parent_reference: None,
    };
    let mine = desktop_data::Attachment {
        url: Some("https://docs.google.com/document/d/mine/edit".into()),
        drive_reference: Some("mine".into()), drive_parent_reference: Some("source".into()),
        drive_sharing: None, ..template.clone()
    };
    assert_eq!(matching_personal_copy(&item, &template), None);
    item.submission = Some(Submission { attachments: vec![mine.clone()], ..Default::default() });
    assert!(matches!(
        matching_personal_copy(&item, &template).and_then(|copy| app.attachment_action(copy)),
        Some(Message::OpenAttachment(url)) if Some(&url) == mine.url.as_ref()
    ));
    item.submission.as_mut().unwrap().attachments[0].url = template.url.clone();
    let copy = matching_personal_copy(&item, &template).unwrap();
    assert_eq!(copy.drive_reference.as_deref(), Some("mine"));
    assert!(app.attachment_action(copy).is_none());
    item.submission.as_mut().unwrap().attachments.push(mine);
    assert_eq!(matching_personal_copy(&item, &template), None);
}

#[cfg(target_os = "macos")]
#[test]
fn leaving_an_assignment_ignores_its_pending_microphone_permission() {
    let (mut app, snapshot) = fixture();
    app.install_snapshot(snapshot);
    app.screen = Screen::Item(1);
    app.item_id = Some("42".into());
    app.submission_capture = Some(CaptureState::AskingPermission);
    let generation = app.capture_generation;
    let _ = app.update(Message::Navigate(Screen::Home));
    let _ = app.update(Message::Capture(CaptureAction::Permission(generation, true)));
    assert_eq!(app.screen, Screen::Home);
    assert!(app.submission_capture.is_none());
    assert_ne!(app.capture_generation, generation);
}

#[test]
fn class_member_email_link_uses_one_exact_recipient_and_escapes_mailto_fields() {
    let members = vec![
        desktop_auth::ClassMention { id: 7, name: "Teacher".into(), email: "first+tag@example.org".into() },
        desktop_auth::ClassMention { id: 8, name: "Student".into(), email: "second@example.org".into() },
    ];
    assert_eq!(class_member_email_url(&members, 7).as_deref(), Some("mailto:first%2Btag@example.org"));
    assert!(class_member_email_url(&members, 9).is_none());
    let mut duplicate = members.clone();
    duplicate.push(duplicate[0].clone());
    assert!(class_member_email_url(&duplicate, 7).is_none());
}

#[test]
fn leaving_people_invalidates_pending_email_lookup() {
    let (mut app, snapshot) = fixture();
    app.install_snapshot(snapshot);
    app.screen = Screen::Course(1);
    app.tab = Tab::People;
    app.people_email_pending = Some((1, 7));
    let generation = app.people_email_generation;
    let _ = app.update(Message::SetTab(Tab::Stream));
    assert_eq!(app.people_email_pending, None);
    assert_ne!(app.people_email_generation, generation);
    let _ = app.update(Message::ClassMemberEmailLoaded(generation, 1, 7, Ok(Vec::new())));
    assert!(app.error.is_none());
}

fn fixture() -> (App, Snapshot) {
    let (app, _) = App::boot(None, true);
    let mut snapshot = Snapshot::demo();
    snapshot.source = SnapshotSource::Live;
    (app, snapshot)
}

#[test]
fn dropped_files_stage_only_nonempty_regular_files_and_preserve_existing_selection() {
    let directory = tempfile::tempdir().unwrap();
    let good = directory.path().join("notes.txt");
    let empty = directory.path().join("empty.txt");
    fs::write(&good, "text").unwrap();
    fs::write(&empty, "").unwrap();
    let (mut app, _) = fixture();
    app.stage_submission_file(1, 42, good.clone());
    assert_eq!(app.submission_file, Some((1, 42, good.clone())));
    for bad in [empty, directory.path().to_owned(), directory.path().join("missing")] {
        app.stage_submission_file(1, 42, bad);
        assert_eq!(app.submission_file, Some((1, 42, good.clone())));
        assert!(app.error.is_some());
    }
    let long = PathBuf::from("a".repeat(8193));
    assert!(validate_submission_file(&long).is_err());

    let _ = app.update(Message::SubmissionFileDropped(good.clone()));
    assert_eq!(app.submission_file, Some((1, 42, good)));
    assert!(!app.sending_submission);
}

#[test]
fn comment_mention_preserves_draft_spacing_and_length_limit() {
    let mut draft = "Thanks".to_owned();
    append_comment_mention(&mut draft, "student@example.org");
    assert_eq!(draft, "Thanks +student@example.org ");
    append_comment_mention(&mut draft, "teacher@example.org");
    assert_eq!(draft, "Thanks +student@example.org +teacher@example.org ");
    let mut full = "x".repeat(9_999);
    append_comment_mention(&mut full, "student@example.org");
    assert_eq!(full.len(), 9_999);
}

#[test]
fn shared_question_link_requires_a_submitted_matching_answer_and_explicit_sharing() {
    let (_, snapshot) = fixture();
    let mut question = snapshot.coursework.values().flatten()
        .find(|item| item.kind == CourseworkKind::Question).unwrap().clone();
    question.id = "42".into();
    question.course_id = 8;
    question.question = Some(QuestionDetails::ShortAnswer);
    question.question_sharing = Some(QuestionSharing::Course);
    question.submission = Some(Submission {
        status: SubmissionStatus::TurnedIn,
        question_answer: Some(QuestionAnswer::ShortAnswer("My answer".into())),
        ..Default::default()
    });
    let (label, url) = shared_question_link(&question).unwrap();
    assert_eq!(label, "See classmate answers in Classroom");
    assert_eq!(ClassroomLink::parse(&url), Some(ClassroomLink::Item {
        course: 8, item: 42, kind: ItemLinkKind::ShortAnswer,
    }));
    for status in [SubmissionStatus::Assigned, SubmissionStatus::Missing, SubmissionStatus::Reclaimed, SubmissionStatus::Unknown] {
        question.submission.as_mut().unwrap().status = status;
        assert!(shared_question_link(&question).is_none());
    }
    question.submission.as_mut().unwrap().status = SubmissionStatus::Returned;
    assert!(shared_question_link(&question).is_some());
    question.submission.as_mut().unwrap().status = SubmissionStatus::ReturnedLate;
    assert!(shared_question_link(&question).is_some());
    question.submission.as_mut().unwrap().question_answer = Some(QuestionAnswer::ShortAnswer(" ".into()));
    assert!(shared_question_link(&question).is_none());
    question.submission.as_mut().unwrap().question_answer = Some(QuestionAnswer::ShortAnswer("My answer".into()));
    question.question_sharing = Some(QuestionSharing::Private);
    assert!(shared_question_link(&question).is_none());
    question.question_sharing = None;
    assert!(shared_question_link(&question).is_none());
    question.question_sharing = Some(QuestionSharing::Course);
    question.question = Some(QuestionDetails::MultipleChoice(vec!["One".into(), "Two".into()]));
    assert!(shared_question_link(&question).is_none());
    question.submission.as_mut().unwrap().question_answer = Some(QuestionAnswer::MultipleChoice("One".into()));
    let (label, url) = shared_question_link(&question).unwrap();
    assert_eq!(label, "See class summary in Classroom");
    assert_eq!(ClassroomLink::parse(&url), Some(ClassroomLink::Item {
        course: 8, item: 42, kind: ItemLinkKind::MultipleChoice,
    }));
    question.submission.as_mut().unwrap().question_answer = Some(QuestionAnswer::MultipleChoice("Absent".into()));
    assert!(shared_question_link(&question).is_none());
    question.question = None;
    assert!(shared_question_link(&question).is_none());
    question.kind = CourseworkKind::Assignment;
    assert!(shared_question_link(&question).is_none());
}

#[test]
fn shared_question_read_requires_current_student_submission_state() {
    assert!(shared_question_read_is_current(true, Some(CourseRole::Student), true, false, false));
    assert!(!shared_question_read_is_current(false, Some(CourseRole::Student), true, false, false));
    assert!(!shared_question_read_is_current(true, Some(CourseRole::Teacher), true, false, false));
    assert!(!shared_question_read_is_current(true, None, true, false, false));
    assert!(!shared_question_read_is_current(true, Some(CourseRole::Student), false, false, false));
    assert!(!shared_question_read_is_current(true, Some(CourseRole::Student), true, true, false));
    assert!(!shared_question_read_is_current(true, Some(CourseRole::Student), true, false, true));
}

#[test]
fn calendar_filters_by_week_and_active_class() {
    let (mut app, mut snapshot) = fixture();
    let today = Local::now().date_naive();
    app.calendar_monday = monday_of(today);
    let original_week = app.calendar_monday;
    let mut archived_item = snapshot.coursework_for(1).unwrap()[1].clone();
    archived_item.course_id = 6;
    snapshot.coursework.get_mut(&6).unwrap().push(archived_item);
    app.install_snapshot(snapshot);

    assert_eq!(app.calendar_items(app.calendar_monday).values().flatten().count(), 4);
    assert!(app.active_course_options().iter().all(|class| class.id != Some(6)));
    let class = app.active_course_options().into_iter().find(|class| class.id == Some(1)).unwrap();
    let _ = app.update(Message::CalendarFilter(class));
    let events: Vec<_> = app.calendar_items(app.calendar_monday).into_values().flatten().collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0.title, "Poetry comparison");

    let _ = app.update(Message::CalendarShift(1));
    assert_eq!(app.calendar_monday, original_week + Days::new(7));
    assert!(app.calendar_items(app.calendar_monday).is_empty());
    let _ = app.update(Message::CalendarShift(-1));
    assert_eq!(app.calendar_monday, original_week);
    let _ = app.update(Message::CalendarShift(-1));
    let _ = app.update(Message::CalendarToday);
    assert_eq!(app.calendar_monday, original_week);

    let _ = app.update(Message::Navigate(Screen::Calendar));
    let (id, item) = app.calendar_items(app.calendar_monday).into_values().flatten().next()
        .map(|(item, course)| (course.id, item.id.clone())).unwrap();
    let _ = app.update(Message::OpenItem(id, item));
    assert_eq!(app.screen, Screen::Item(1));
    let _ = app.update(Message::CloseItem);
    assert_eq!(app.screen, Screen::Calendar);
    assert_eq!(app.calendar_class, Some(1));
}

#[test]
fn calendar_preserves_date_only_deadlines_and_converts_timed_deadlines() {
    let (_, snapshot) = fixture();
    let mut item = snapshot.coursework_for(1).unwrap()[1].clone();
    let midnight = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap();
    item.due_at_millis = Some(midnight.timestamp_millis());
    let timezone = chrono::FixedOffset::west_opt(8 * 60 * 60).unwrap();
    let new_year = midnight.date_naive();

    item.due_has_time = Some(false);
    assert_eq!(item.due_date(&timezone), Some(new_year));
    item.due_has_time = Some(true);
    assert_eq!(item.due_date(&timezone), new_year.pred_opt());
    assert_eq!(monday_of(new_year), NaiveDate::from_ymd_opt(2025, 12, 29).unwrap());

    let mut cached = serde_json::to_value(&item).unwrap();
    cached.as_object_mut().unwrap().remove("due_has_time");
    for (label, expected) in [("Due 1 Jan", Some(new_year)), ("Due 31 Dec, 4:00 PM", new_year.pred_opt())] {
        cached["due_label"] = label.into();
        let legacy = serde_json::from_value::<Coursework>(cached.clone()).unwrap();
        assert_eq!(legacy.due_date(&timezone), expected);
    }
}

#[test]
fn folder_load_discards_the_previous_session_result_and_hides_links_offline() {
    let (mut app, mut snapshot) = fixture();
    app.install_snapshot(snapshot.clone());
    let old_generation = app.class_folders_generation;
    let url = "https://drive.google.com/drive/folders/test";
    app.reset_class_folders();
    let details = || desktop_auth::OwnClassDetails { folders: BTreeMap::from([(1, url.into())]), ..Default::default() };
    let _ = app.update(Message::ClassFoldersLoaded(old_generation, Ok(details())));
    assert!(app.class_folder_url(1).is_none());
    let _ = app.update(Message::ClassFoldersLoaded(app.class_folders_generation, Ok(details())));
    assert_eq!(app.class_folder_url(1), Some(url));
    snapshot.source = SnapshotSource::Cache;
    app.install_snapshot(snapshot);
    assert!(app.class_folder_url(1).is_none());
}

#[test]
fn edited_short_answer_requires_a_change_and_survives_a_failed_request() {
    let (mut app, _) = fixture();
    let key = (4, 42);
    let original = "Original".to_owned();
    let revision = desktop_auth::ShortAnswerRevision {
        submission_id: 67, answer: original.clone(),
    };
    app.short_answer_drafts.insert(key, text_editor::Content::with_text(&original));
    app.short_answer_revisions.insert(key, revision);
    assert!(app.question_draft(key, &QuestionDetails::ShortAnswer).is_none());
    app.short_answer_drafts.insert(key, text_editor::Content::with_text("Revised"));
    assert_eq!(app.question_draft(key, &QuestionDetails::ShortAnswer),
        Some(QuestionAnswer::ShortAnswer("Revised".into())));
    let _ = app.update(Message::QuestionAnswerSent(key.0, key.1, Err("Retry after checking.".into())));
    assert_eq!(app.question_draft(key, &QuestionDetails::ShortAnswer),
        Some(QuestionAnswer::ShortAnswer("Revised".into())));
    assert_eq!(app.error.as_deref(), Some("Retry after checking."));
}

#[test]
fn a_student_choice_is_highlighted_only_when_the_answer_matches_one_option() {
    let choices = vec!["Alpha".into(), "Beta".into(), "Alpha".into()];
    assert_eq!(matching_choice(&choices, "Beta"), Some(1));
    assert_eq!(matching_choice(&choices, "Alpha"), None);
    assert_eq!(matching_choice(&choices, "1"), None);
    let question = QuestionDetails::MultipleChoice(choices);
    assert!(question.accepts(&QuestionAnswer::MultipleChoice("Beta".into())));
    assert!(!question.accepts(&QuestionAnswer::ShortAnswer("Beta".into())));
    assert!(!question.accepts(&QuestionAnswer::MultipleChoice("Alpha".into())));
}

#[test]
fn classwork_uses_returned_sort_keys_and_retains_fallback_order() {
    let (_, mut snapshot) = fixture();
    let items = snapshot.coursework.get_mut(&1).unwrap();
    items.retain(|item| item.kind != CourseworkKind::Announcement);
    assert!(items.len() >= 2);
    for (index, item) in items.iter_mut().enumerate() {
        item.topic_id = Some("demo-poetry-unit".into());
        item.classwork_sort_key = Some(vec![200 - index as u8]);
    }
    let response: Vec<_> = items.iter().map(|item| item.id.clone()).collect();
    let display_order = |snapshot: &Snapshot| snapshot.classwork_sections(1, Some("demo-poetry-unit"))
        .into_iter().flat_map(|section| section.items).map(|item| item.id.clone()).collect::<Vec<_>>();
    assert_eq!(display_order(&snapshot), response.iter().rev().cloned().collect::<Vec<_>>());

    let items = snapshot.coursework.get_mut(&1).unwrap();
    items[0].classwork_sort_key = items[1].classwork_sort_key.clone();
    assert_eq!(display_order(&snapshot), response);
    snapshot.coursework.get_mut(&1).unwrap()[0].classwork_sort_key = None;
    assert_eq!(display_order(&snapshot), response);
}

#[test]
fn classwork_groups_only_published_work_and_keeps_untagged_items_first() {
    let (_, mut snapshot) = fixture();
    let topics = snapshot.topics.get_mut(&1).unwrap();
    topics.insert(0, desktop_data::Topic { id: "topic-first".into(), title: "First topic".into() });
    topics.push(desktop_data::Topic { id: "empty".into(), title: "Empty topic".into() });
    let items = snapshot.coursework.get_mut(&1).unwrap();
    let template = items[1].clone();
    for (id, topic) in [("untagged", None), ("first", Some("topic-first")), ("unresolved", Some("missing"))] {
        let mut item = template.clone();
        item.id = id.into();
        item.topic_id = topic.map(str::to_owned);
        items.push(item);
    }

    let groups = snapshot.classwork_sections(1, None);
    assert_eq!(groups.iter().map(|group| group.title).collect::<Vec<_>>(),
        ["No topic", "First topic", "Poetry", "Topic unavailable"]);
    assert_eq!(groups[0].items[0].id, "untagged");
    assert_eq!(groups[2].items.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(),
        ["demo-poetry", "demo-poetry-reading"]);
    assert_eq!(groups.iter().map(|group| group.items.len()).sum::<usize>(), 5);
    assert_eq!(snapshot.classwork_sections(1, Some("topic-first"))[0].items[0].id, "first");
    assert!(snapshot.classwork_sections(1, Some("empty")).is_empty());
}

#[test]
fn classwork_item_and_topic_preview_state_survives_opening_instructions() {
    let (mut app, snapshot) = fixture();
    app.install_snapshot(snapshot);
    let _ = app.update(Message::Navigate(Screen::Course(1)));
    let _ = app.update(Message::SetTab(Tab::Classwork));
    let item = "demo-poetry";
    let topic = "demo-poetry-unit";
    let key = ClassworkGroupId::Topic(topic.into());
    assert!(!app.classwork_item_expanded(item));

    let _ = app.update(Message::SetClassworkGroups(1, true));
    assert!(app.collapsed_classwork_groups.contains(&key));
    let _ = app.update(Message::ToggleClassworkGroup(1, key.clone()));
    assert!(!app.collapsed_classwork_groups.contains(&key));
    let _ = app.update(Message::ToggleClassworkItem(1, item.into()));
    assert!(app.classwork_item_expanded(item));
    let _ = app.update(Message::OpenItem(1, item.into()));
    assert_eq!(app.screen, Screen::Item(1));
    let _ = app.update(Message::CloseItem);
    assert_eq!((app.screen, app.tab), (Screen::Course(1), Tab::Classwork));
    assert!(app.classwork_item_expanded(item));

    let _ = app.update(Message::SelectTopic(Some(topic.into())));
    assert!(app.classwork_item_expanded(item));
    let _ = app.update(Message::ToggleClassworkItem(1, item.into()));
    assert!(!app.classwork_item_expanded(item));
    let _ = app.update(Message::SelectTopic(None));
    assert!(!app.classwork_item_expanded(item));
    let _ = app.update(Message::SetClassworkGroups(1, false));
    assert!(app.collapsed_classwork_groups.is_empty());
    let _ = app.update(Message::Navigate(Screen::Course(2)));
    assert!(app.classwork_toggles.is_empty());
}

#[test]
fn home_due_soon_uses_local_days_and_open_work_across_active_classes() {
    let (mut app, mut snapshot) = fixture();
    let template = snapshot.coursework_for(1).unwrap()[1].clone();
    snapshot.coursework.clear();
    let now = DateTime::parse_from_rfc3339("2026-12-31T15:00:00-08:00").unwrap();
    let mut add = |course, id: &str, deadline: &str, timed, status| {
        let mut item = template.clone();
        item.course_id = course;
        item.id = id.into();
        item.due_at_millis = DateTime::parse_from_rfc3339(deadline).ok().map(|date| date.timestamp_millis());
        item.due_has_time = Some(timed);
        item.submission = Some(Submission { status, ..Default::default() });
        snapshot.coursework.entry(course).or_default().push(item);
    };
    use SubmissionStatus::*;
    add(2, "today-date", "2026-12-31T00:00:00Z", false, Assigned);
    add(1, "today-time", "2027-01-01T00:00:00Z", true, Assigned);
    add(1, "past-time", "2026-12-31T22:00:00Z", true, Assigned);
    add(1, "past-date", "2026-12-30T00:00:00Z", false, Assigned);
    add(1, "done", "2027-01-01T00:00:00Z", false, TurnedIn);
    add(1, "missing", "2027-01-01T00:00:00Z", false, Missing);
    add(1, "undated", "", false, Assigned);
    add(2, "last-day", "2027-01-06T00:00:00Z", false, Reclaimed);
    add(2, "outside-week", "2027-01-07T00:00:00Z", false, Assigned);
    add(6, "archived", "2026-12-31T00:00:00Z", false, Assigned);

    let upcoming = |selected, limit| snapshot.upcoming_work(&now, selected, limit)
        .into_iter().map(|(item, _)| item.id.as_str()).collect::<Vec<_>>();
    assert_eq!(upcoming(None, 5), ["today-time", "today-date", "last-day"]);
    assert_eq!(upcoming(None, 2), ["today-time", "today-date"]);
    assert_eq!(upcoming(Some(2), 5), ["today-date", "last-day"]);

    app.install_snapshot(snapshot);
    let _ = app.update(Message::OpenItem(1, "today-time".into()));
    assert_eq!(app.screen, Screen::Item(1));
    let _ = app.update(Message::CloseItem);
    assert_eq!(app.screen, Screen::Home);
}

#[test]
fn todo_groups_deadlines_across_local_week_boundaries() {
    let today = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let deadline = |year, month, day| NaiveDate::from_ymd_opt(year, month, day);
    for (due, group) in [
        (None, TodoGroup::NoDueDate),
        (deadline(2025, 12, 28), TodoGroup::LastWeek),
        (deadline(2025, 12, 29), TodoGroup::ThisWeek),
        (deadline(2026, 1, 4), TodoGroup::ThisWeek),
        (deadline(2026, 1, 5), TodoGroup::NextWeek),
        (deadline(2026, 1, 12), TodoGroup::Later),
        (deadline(2025, 12, 21), TodoGroup::Earlier),
    ] {
        assert_eq!(TodoGroup::for_deadline(due, today, false), group);
    }
    assert_eq!(TodoGroup::for_deadline(deadline(2026, 1, 2), today, true), TodoGroup::DoneEarly);
}

#[test]
fn todo_keeps_standard_groups_and_calendar_uses_relative_day_labels() {
    use TodoGroup::*;
    assert_eq!(TodoFilter::Assigned.groups(), [NoDueDate, ThisWeek, NextWeek, Later]);
    assert_eq!(TodoFilter::Missing.groups(), [ThisWeek, LastWeek, Earlier]);
    assert_eq!(TodoFilter::Done.groups(), [NoDueDate, DoneEarly, ThisWeek, LastWeek, Earlier]);
    let today = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
    assert_eq!(calendar_agenda_heading(today - Days::new(1), today), "Yesterday");
    assert_eq!(calendar_agenda_heading(today, today), "Today");
    assert_eq!(calendar_agenda_heading(today + Days::new(1), today), "Tomorrow");
    assert_eq!(calendar_agenda_heading(today + Days::new(2), today), "4 Jan");
}

#[test]
fn todo_routes_unknown_work_class_filters_and_group_collapsing() {
    let (mut app, mut snapshot) = fixture();
    let today = Local::now().date_naive();
    let mut archived_item = snapshot.coursework_for(1).unwrap()[1].clone();
    archived_item.course_id = 6;
    snapshot.coursework.get_mut(&6).unwrap().push(archived_item);
    app.install_snapshot(snapshot);

    let groups = app.todo_items(today, app.todo_filter);
    assert_eq!(groups[&TodoGroup::ThisWeek][0].0.id, "demo-poetry");
    assert!(groups[&TodoGroup::StatusUnavailable].iter().any(|(item, _)| item.id == "demo-sort"));
    assert!(groups.values().flatten().all(|(_, course)| course.id != 6));

    let filter = app.active_course_options().into_iter().find(|class| class.id == Some(5)).unwrap();
    let _ = app.update(Message::FilterTodoClass(filter));
    assert!(app.todo_items(today, app.todo_filter).is_empty());
    let _ = app.update(Message::SetTodoFilter(TodoFilter::Missing));
    let groups = app.todo_items(today, app.todo_filter);
    assert_eq!(groups[&TodoGroup::ThisWeek][0].0.id, "demo-waves");
    let _ = app.update(Message::ToggleTodoGroup(TodoGroup::ThisWeek));
    assert!(app.collapsed_todo_groups.contains(&(TodoFilter::Missing, TodoGroup::ThisWeek)));
    assert!(!app.collapsed_todo_groups.contains(&(TodoFilter::Assigned, TodoGroup::ThisWeek)));
    let _ = app.update(Message::ToggleTodoGroup(TodoGroup::ThisWeek));
    assert!(app.collapsed_todo_groups.is_empty());

    let all_classes = app.active_course_options().remove(0);
    let _ = app.update(Message::FilterTodoClass(all_classes));
    let _ = app.update(Message::SetTodoFilter(TodoFilter::Done));
    assert_eq!(app.todo_items(today, app.todo_filter).values().flatten().count(), 1);
}

#[test]
fn your_work_routes_to_the_class_and_filters_only_released_grades() {
    let (mut app, snapshot) = fixture();
    app.install_snapshot(snapshot);
    let _ = app.update(Message::OpenYourWork(2));
    assert_eq!((app.screen, app.tab), (Screen::Course(2), Tab::YourWork));
    let math = &app.snapshot.as_ref().unwrap().coursework_for(2).unwrap()[0];
    let missing = &app.snapshot.as_ref().unwrap().coursework_for(5).unwrap()[0];
    assert!(WorkFilter::ReturnedWithGrade.includes(math));
    assert!(!WorkFilter::Assigned.includes(math));
    assert!(WorkFilter::Missing.includes(missing));
    assert!(!WorkFilter::ReturnedWithGrade.includes(missing));

    let _ = app.update(Message::SetWorkFilter(WorkFilter::Missing));
    let _ = app.update(Message::OpenYourWork(1));
    assert_eq!(app.work_filter, WorkFilter::All);
}

#[test]
fn home_invitations_are_nonblocking_and_reject_stale_results() {
    let (mut app, snapshot) = fixture();
    let duplicate = snapshot.courses[0].clone();
    app.install_snapshot(snapshot);
    app.loading_invitations = true;
    assert!(!app.has_pending_request());
    let old = app.invitation_generation;
    app.invalidate_invitation_read();
    let invited = Course { id: 799, title: "Art fixture".into(), ..Default::default() };
    let _ = app.update(Message::InvitationsLoaded(old, Ok(vec![invited.clone()])));
    assert!(app.invitations.is_none());
    let _ = app.update(Message::InvitationsLoaded(app.invitation_generation, Ok(vec![duplicate, invited])));
    assert_eq!(app.pending_invitations().map(|course| course.id).collect::<Vec<_>>(), [799]);
    assert!(!app.can_respond_to_invitation(799)); // The fixture has no live session.
}

#[test]
fn confirmed_join_waits_for_the_joined_class_in_the_refreshed_list() {
    let (mut app, mut snapshot) = fixture();
    app.install_snapshot(snapshot.clone());
    app.screen = Screen::Join;
    let invited = Course { id: 799, title: "Art fixture".into(), ..Default::default() };
    app.invitations = Some(vec![invited.clone()]);
    let _ = app.update(Message::MembershipFinished(Ok(MembershipChange::Joined(799))));
    assert_eq!(app.screen, Screen::Home);
    assert!(app.pending_invitations().next().is_none());
    assert_eq!(app.opening_joined_course, Some(799));
    let _ = app.update(Message::Refreshed(Ok(snapshot.clone())));
    assert_eq!(app.screen, Screen::Home);
    snapshot.courses.push(invited);
    let _ = app.update(Message::Refreshed(Ok(snapshot)));
    assert_eq!((app.screen, app.tab), (Screen::Course(799), Tab::Stream));
    assert!(app.opening_joined_course.is_none());
}

#[test]
fn declining_an_invitation_keeps_the_current_page_and_other_classes() {
    let (mut app, snapshot) = fixture();
    let enrolled: Vec<_> = snapshot.courses.iter().map(|course| course.id).collect();
    app.install_snapshot(snapshot);
    app.screen = Screen::Join;
    let first = Course { id: 701, title: "Art fixture".into(), ..Default::default() };
    let second = Course { id: 702, title: "Maths fixture".into(), ..Default::default() };
    app.invitations = Some(vec![first.clone(), second.clone()]);
    let previous_generation = app.invitation_generation;
    let _ = app.update(Message::InvitationDeclined(Ok(vec![second])));
    let _ = app.update(Message::InvitationsLoaded(previous_generation, Ok(vec![first])));
    assert_eq!(app.screen, Screen::Join);
    assert_eq!(app.pending_invitations().map(|course| course.id).collect::<Vec<_>>(), [702]);
    assert_eq!(app.snapshot.unwrap().courses.iter().map(|course| course.id).collect::<Vec<_>>(), enrolled);
}

#[test]
fn opening_pages_selects_the_required_reads() {
    let (mut app, mut snapshot) = fixture();
    snapshot.coursework.remove(&1);
    snapshot.roster.remove(&1);
    for (screen, tab, expected) in [
        (Screen::Home, Tab::Stream, PageRead::Assignments),
        (Screen::Calendar, Tab::Stream, PageRead::Assignments),
        (Screen::ToDo, Tab::Stream, PageRead::Assignments),
        (Screen::Join, Tab::Stream, PageRead::Invitations),
        (Screen::Settings, Tab::Stream, PageRead::ClassNotifications),
        (Screen::Course(1), Tab::Stream, PageRead::Coursework(1)),
        (Screen::Course(1), Tab::Classwork, PageRead::Coursework(1)),
        (Screen::Course(1), Tab::YourWork, PageRead::Coursework(1)),
        (Screen::Course(1), Tab::People, PageRead::Detail(DetailRead::Roster(1))),
    ] {
        app.screen = screen;
        app.tab = tab;
        app.invitations = (screen == Screen::Home).then(Vec::new);
        assert_eq!(app.next_page_read(&snapshot), Some(expected), "{screen:?} {tab:?}");
    }

    app.tab = Tab::YourWork;
    let snapshot = Snapshot::demo();
    assert_eq!(app.next_page_read(&snapshot), Some(PageRead::Detail(DetailRead::Submissions(1))));
    app.submissions_checked.insert(1);
    assert_eq!(app.next_page_read(&snapshot), Some(PageRead::Detail(DetailRead::OverallGrade(1))));
    let _ = app.update(Message::DetailLoaded(DetailRead::OverallGrade(1), Ok(DetailData::OverallGrade(None))));
    assert_eq!(app.next_page_read(&snapshot), None);
}

#[test]
fn opening_classwork_fetches_submission_status_after_coursework() {
    let (mut app, mut snapshot) = fixture();
    let items = snapshot.coursework.remove(&1).unwrap();
    app.install_snapshot(snapshot);
    let _ = app.update(Message::Navigate(Screen::Course(1)));
    let _ = app.update(Message::SetTab(Tab::Classwork));
    assert_eq!(app.next_page_read(app.snapshot.as_ref().unwrap()), Some(PageRead::Coursework(1)));

    let _ = app.update(Message::CourseworkLoaded(1, Ok(CourseContents { items, topics: None })));
    let submissions = DetailRead::Submissions(1);
    assert_eq!(app.next_page_read(app.snapshot.as_ref().unwrap()), Some(PageRead::Detail(submissions)));
    let _ = app.update(Message::DetailLoaded(submissions, Ok(DetailData::Submissions(vec![]))));
    assert_eq!(app.next_page_read(app.snapshot.as_ref().unwrap()), None);
}

#[test]
fn drive_auto_pages_short_results_and_stops_at_a_repeated_cursor() {
    let mut picker = DrivePicker::default();
    let page = || DriveFilePage { files: vec![], folders: vec![], next_page: Some("cursor".into()), incomplete: false };
    picker.accept_page(page(), false);
    assert!(picker.needs_more_for_viewport());
    picker.error = Some((true, "Try again".into()));
    assert!(!picker.can_auto_page());
    picker.error = None;
    picker.query = "unsent query".into();
    assert!(!picker.can_auto_page());
    picker.query.clear();
    picker.accept_page(page(), true);
    assert!(!picker.can_auto_page());
    assert!(picker.file_page.unwrap().incomplete);
}

#[test]
fn returning_to_a_page_retries_only_its_failed_reads() {
    let (mut app, mut snapshot) = fixture();
    snapshot.roster.remove(&1);
    app.install_snapshot(snapshot.clone());
    let roster = PageRead::Detail(DetailRead::Roster(1));
    let other_roster = PageRead::Detail(DetailRead::Roster(2));
    app.page_errors.insert(roster, "Timed out".into());
    app.page_errors.insert(other_roster, "Timed out".into());

    let _ = app.update(Message::Navigate(Screen::Course(1)));
    assert!(app.page_errors.contains_key(&roster));
    let _ = app.update(Message::SetTab(Tab::People));
    assert_eq!(app.next_page_read(&snapshot), Some(roster));
    assert!(app.page_errors.contains_key(&other_roster));
    let _ = app.update(Message::DetailLoaded(DetailRead::Roster(1), Err("Timed out again".into())));
    assert_eq!(app.next_page_read(&snapshot), None);
}

#[test]
fn stream_checks_permissions_automatically_and_keeps_an_unconfirmed_draft() {
    let (mut app, snapshot) = fixture();
    app.install_snapshot(snapshot.clone());
    let _ = app.update(Message::Navigate(Screen::Course(1)));
    assert_eq!(app.next_page_read(&snapshot), Some(PageRead::StreamAccess(1)));
    let generation = app.stream_access_generation;
    let _ = app.update(Message::StreamAccessLoaded(generation, 1, Ok((9, StreamAccess::ReadOnly))));
    let _ = app.update(Message::OpenStreamComposer(1));
    assert!(app.open_stream_composer.is_none());
    app.stream_drafts.insert(1, StreamDraft {
        body: text_editor::Content::with_text("Unconfirmed draft"),
        links: vec!["https://example.org/lesson".into()],
        ..Default::default()
    });
    app.sending_stream_post = Some(1);
    let _ = app.update(Message::StreamPostSent(1, Err("Unconfirmed".into())));
    assert_eq!(app.stream_drafts[&1].body.text().trim(), "Unconfirmed draft");
    assert_eq!(app.stream_drafts[&1].links, ["https://example.org/lesson"]);
    assert!(!app.stream_access.contains_key(&1));
    assert!(!valid_stream_draft(" \n "));
}

#[test]
fn pending_local_file_cannot_be_silently_left_out_of_a_post() {
    let (mut app, snapshot) = fixture();
    app.install_snapshot(snapshot);
    app.stream_drafts.insert(1, StreamDraft {
        body: text_editor::Content::with_text("Draft with file"),
        ..Default::default()
    });
    app.choosing_stream_file = Some(1);
    let path = PathBuf::from("Reading notes.pdf");
    let _ = app.update(Message::StreamFileChosen(1, Ok(Some(path.clone()))));
    assert!(!app.stream_drafts[&1].can_send("Draft with file"));
    app.uploading_stream_file = Some(1);
    let _ = app.update(Message::StreamFileUploaded(1, Err("Drive did not confirm the upload.".into())));
    assert_eq!(app.stream_drafts[&1].selected_file.as_ref(), Some(&path));
    assert_eq!(app.stream_drafts[&1].body.text().trim(), "Draft with file");
    assert!(!app.stream_drafts[&1].can_send("Draft with file"));
}

#[test]
fn stream_formatting_survives_insertions_and_handles_non_ascii_selection() {
    let mut draft = StreamDraft { body: text_editor::Content::with_text("a📝b"), ..Default::default() };
    draft.body.move_to(text_editor::Cursor {
        position: text_editor::Position { line: 0, column: 5 },
        selection: Some(text_editor::Position { line: 0, column: 1 }),
    });
    draft.format(StreamFormat::Underline);
    assert_eq!(draft.description_styles().iter().filter(|style| style.underline).map(|style| style.start).collect::<Vec<_>>(), [1]);
    draft.edit(text_editor::Action::Move(text_editor::Motion::DocumentStart));
    draft.edit(text_editor::Action::Move(text_editor::Motion::DocumentStart));
    assert!(draft.body.cursor().selection.is_none());
    assert_eq!(draft.body.cursor().position.column, 0);
    draft.edit(text_editor::Action::Edit(text_editor::Edit::Insert('x')));
    assert_eq!(draft.body.text(), "xa📝b");
    assert_eq!(draft.description_styles().iter().filter(|style| style.underline).map(|style| style.start).collect::<Vec<_>>(), [2]);
}

#[test]
fn stream_bullets_follow_paragraph_edits_and_enter_on_empty_bullet_ends_the_list() {
    let mut draft = StreamDraft { body: text_editor::Content::with_text("Read 📝"), ..Default::default() };
    draft.edit(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
    draft.format(StreamFormat::BulletList);
    assert_eq!(draft.bullets, BTreeSet::from([0]));
    draft.edit(text_editor::Action::Edit(text_editor::Edit::Enter));
    assert_eq!(draft.body.text(), "Read 📝\n");
    assert_eq!(draft.bullets, BTreeSet::from([0, "Read 📝\n".len()]));
    draft.edit(text_editor::Action::Edit(text_editor::Edit::Enter));
    assert_eq!(draft.body.text(), "Read 📝\n");
    assert_eq!(draft.bullets, BTreeSet::from([0]));
}

#[test]
fn drive_folder_navigation_uses_only_returned_folders_and_resets_search() {
    let mut picker = DrivePicker { query: "old search".into(), ..Default::default() };
    assert!(picker.browse(DriveNavigation::Section(DriveSection::MyDrive)));
    assert!(matches!(picker.location(), DriveLocation::Folder(id) if id == "root"));
    assert!(picker.query.is_empty());
    picker.accept_page(DriveFilePage {
        folders: vec![DriveFolder { id: "folder_fixture".into(), name: "Notes".into() }],
        files: vec![], next_page: Some("page_cursor".into()), incomplete: false,
    }, false);
    assert!(!picker.browse(DriveNavigation::OpenFolder("unknown".into())));
    assert!(picker.browse(DriveNavigation::OpenFolder("folder_fixture".into())));
    assert!(matches!(picker.location(), DriveLocation::Folder(id) if id == "folder_fixture"));
    assert!(picker.browse(DriveNavigation::Back));
    assert!(matches!(picker.location(), DriveLocation::Folder(id) if id == "root"));
    assert!(picker.browse(DriveNavigation::Section(DriveSection::Shared)));
    assert!(matches!(picker.location(), DriveLocation::Shared));
}

#[test]
fn stream_delete_only_removes_the_confirmed_announcement() {
    let (mut app, mut snapshot) = fixture();
    let items = snapshot.coursework.get_mut(&1).unwrap();
    let announcement = items.iter_mut().find(|item| item.kind == CourseworkKind::Announcement).unwrap();
    announcement.id = "91".into();
    announcement.author_id = Some(9);
    app.stream_viewer_id = Some(10);
    assert!(!app.owns_announcement(announcement));
    app.stream_viewer_id = Some(9);
    assert!(app.owns_announcement(announcement));
    let before = items.len();
    app.install_snapshot(snapshot);
    app.deleting_stream_post = Some((1, 91));
    let _ = app.update(Message::StreamPostDeleted(1, 92, Ok(())));
    let _ = app.update(Message::StreamPostDeleted(1, 91, Err("Unconfirmed".into())));
    assert_eq!(app.snapshot.as_ref().unwrap().coursework_for(1).unwrap().len(), before);
    app.deleting_stream_post = Some((1, 91));
    let _ = app.update(Message::StreamPostDeleted(1, 91, Ok(())));
    let items = app.snapshot.as_ref().unwrap().coursework_for(1).unwrap();
    assert_eq!(items.len(), before - 1);
    assert!(items.iter().all(|item| item.id != "91"));
}

#[test]
fn deleting_a_private_comment_preserves_the_class_thread_with_the_same_id() {
    let (mut app, mut snapshot) = fixture();
    let fixture = Comment {
        id: "19".into(), author_id: Some(12), author: "Student".into(),
        body: "Sample comment".into(), created_label: None,
    };
    let item = &mut snapshot.coursework.get_mut(&1).unwrap()[0];
    item.id = "42".into();
    item.comments = Some(vec![fixture.clone()]);
    app.install_snapshot(snapshot);
    app.stream_viewer_id = Some(11);
    assert!(!app.owns_comment(&fixture));
    app.stream_viewer_id = Some(12);
    assert!(app.owns_comment(&fixture));
    app.private_feedback.insert((1, 42), vec![fixture]);
    let target = CommentTarget { course: 1, item: 42, id: 19, audience: CommentAudience::Private };
    app.deleting_comment = Some(target);
    let _ = app.update(Message::CommentDeleted(target, Ok(())));
    assert!(app.private_feedback[&(1, 42)].is_empty());
    assert_eq!(app.snapshot.as_ref().unwrap().coursework_for(1).unwrap()[0].comments.as_ref().unwrap().len(), 1);
}

#[test]
fn dragging_a_class_uses_its_direction_and_skips_hidden_targets() {
    let (mut app, mut snapshot) = fixture();
    snapshot.courses.iter_mut().find(|course| course.id == 2).unwrap().hidden = Some(true);
    app.install_snapshot(snapshot);
    assert_eq!(app.drop_placement(1, 3), Some(CoursePlacement::After(3)));
    assert_eq!(app.drop_placement(3, 1), Some(CoursePlacement::Before(1)));
    assert_eq!(app.drop_placement(1, 2), None);
    assert_eq!(app.drop_placement(1, 1), None);
    assert!(app.course_move_options(1).iter().all(|option| option.placement != CoursePlacement::After(2)));
}

#[test]
fn settings_use_confirmed_values_and_ignore_stale_notification_reads() {
    let (mut app, snapshot) = fixture();
    app.install_snapshot(snapshot);
    let _ = app.update(Message::Navigate(Screen::Settings));
    let generation = app.notifications_generation;
    let _ = app.update(Message::ClassNotificationsLoaded(generation, Ok(BTreeMap::from([(1, false)]))));
    let _ = app.update(Message::SetClassNotifications(1, false));
    assert!(app.saving_class_notifications.is_none());
    assert_eq!(app.class_notifications.as_ref().unwrap().get(&1), Some(&false));

    app.saving_class_notifications = Some(1);
    let _ = app.update(Message::ClassNotificationsSaved(1, Ok(true)));
    assert_eq!(app.class_notifications.as_ref().unwrap().get(&1), Some(&true));
    let _ = app.update(Message::Navigate(Screen::Home));
    let _ = app.update(Message::Navigate(Screen::Settings));
    let _ = app.update(Message::ClassNotificationsLoaded(generation, Ok(BTreeMap::from([(1, false)]))));
    assert!(app.class_notifications.is_none());
}

#[test]
fn assignment_editor_preserves_existing_fields_and_can_clear_them_independently() {
    let (_, snapshot) = fixture();
    let mut item = snapshot.coursework_for(1).unwrap().iter()
        .find(|item| item.kind == CourseworkKind::Assignment).unwrap().clone();
    let date = NaiveDate::from_ymd_opt(2027, 1, 26).unwrap();
    item.due_at_millis = Some(date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis());
    item.due_has_time = Some(false);
    item.maximum_points = Some(42.5);
    let draft = desktop_auth::TeacherDraft { item, publication: desktop_auth::DraftPublication::Draft };
    let mut editor = AssignmentEditor::new(1, Some(draft));
    assert_eq!(editor.settings.due_date, "2027-01-26");
    assert_eq!(editor.settings.due_time, "");
    assert_eq!(editor.settings.points, "42.5");
    editor.title.push_str(" revised");
    let unchanged = editor.field_changes().unwrap();
    assert!(unchanged.due.is_none() && unchanged.points.is_none());

    editor.settings.due_date.clear();
    let cleared_date = editor.field_changes().unwrap();
    assert_eq!(cleared_date.due, Some(None));
    assert!(cleared_date.points.is_none());
    editor.settings.points = "0".into();
    assert_eq!(editor.field_changes().unwrap().points, Some(None));
    editor.settings.due_time = "16:00".into();
    assert!(editor.field_changes().is_err());
}

#[test]
fn new_assignment_prefills_default_points_and_only_sends_explicit_changes() {
    let mut editor = AssignmentEditor::new(1, None);
    assert_eq!(editor.settings.points, "100");
    assert!(editor.field_changes().unwrap().points.is_none());
    editor.settings.points = "25".into();
    assert_eq!(editor.field_changes().unwrap().points, Some(Some(25.0)));
    editor.settings.points = "-1".into();
    assert!(editor.field_changes().is_err());
}

#[test]
fn archive_and_restore_update_overviews_after_a_matching_confirmation() {
    let (mut app, snapshot) = fixture();
    let count = snapshot.courses.len();
    app.install_snapshot(snapshot);
    app.todo_class = Some(1);
    app.calendar_class = Some(1);
    app.changing_class_archive = Some((1, true));
    let _ = app.update(Message::ClassArchiveChanged(1, false, Ok(CourseState::Active)));
    assert_eq!(app.changing_class_archive, Some((1, true)));
    let _ = app.update(Message::ClassArchiveChanged(1, true, Ok(CourseState::Archived)));
    let data = app.snapshot.as_ref().unwrap();
    assert_eq!(data.courses.len(), count);
    assert_eq!(data.courses.iter().find(|course| course.id == 1).unwrap().state, CourseState::Archived);
    assert!(data.upcoming_work(&Local::now(), Some(1), 5).is_empty());
    assert!(!app.assignment_courses().contains(&1));
    assert!(app.active_course_options().iter().all(|option| option.id != Some(1)));
    assert_eq!((app.todo_class, app.calendar_class), (None, None));
    app.changing_class_archive = Some((1, false));
    let _ = app.update(Message::ClassArchiveChanged(1, false, Ok(CourseState::Active)));
    assert!(app.assignment_courses().contains(&1));
    assert!(app.active_course_options().iter().any(|option| option.id == Some(1)));
}

#[test]
fn hidden_class_keeps_enrollment_and_leaves_overviews_until_unhidden() {
    let (mut app, snapshot) = fixture();
    let count = snapshot.courses.len();
    app.install_snapshot(snapshot);
    app.calendar_class = Some(1);
    app.todo_class = Some(1);
    app.changing_course_visibility = Some((1, true));
    let _ = app.update(Message::CourseVisibilityChanged(2, true, Ok(())));
    let _ = app.update(Message::CourseVisibilityChanged(1, true, Ok(())));
    let data = app.snapshot.as_ref().unwrap();
    let course = data.courses.iter().find(|course| course.id == 1).unwrap();
    assert!(hidden_course(course));
    assert_eq!(course.state, CourseState::Active);
    assert_eq!(data.courses.len(), count);
    assert!(data.upcoming_work(&Local::now(), Some(1), 5).is_empty());
    assert!(!app.assignment_courses().contains(&1));
    assert!(app.active_course_options().iter().all(|option| option.id != Some(1)));
    assert_eq!((app.calendar_class, app.todo_class), (None, None));

    app.changing_course_visibility = Some((1, false));
    let _ = app.update(Message::CourseVisibilityChanged(1, false, Ok(())));
    assert!(app.assignment_courses().contains(&1));
    assert!(app.active_course_options().iter().any(|option| option.id == Some(1)));
}

#[test]
fn opening_an_unloaded_item_fetches_classwork_then_comments() {
    let (mut app, mut snapshot) = fixture();
    let mut items = snapshot.coursework.remove(&1).unwrap();
    items[0].id = "42".into();
    items[0].comments = None;
    app.install_snapshot(snapshot.clone());

    let _ = app.update(Message::OpenItem(1, "42".into()));
    assert_eq!(app.next_page_read(&snapshot), Some(PageRead::Coursework(1)));
    let _ = app.update(Message::CourseworkLoaded(1, Ok(CourseContents { items, topics: None })));
    assert_eq!(app.next_page_read(app.snapshot.as_ref().unwrap()), Some(PageRead::StreamAccess(1)));
    app.loading_stream_access.insert(1);
    assert_eq!(app.next_page_read(app.snapshot.as_ref().unwrap()),
        Some(PageRead::Detail(DetailRead::Comments(1, 42))));
}

#[test]
fn your_work_link_requires_matching_identity_and_ignores_cancelled_checks() {
    let (mut app, snapshot) = fixture();
    app.install_snapshot(snapshot);
    let target = ClassroomLink::YourWork { course: 1, student: 14 };
    let wrong = ClassroomLink::YourWork { course: 1, student: 13 };
    let begin_check = |app: &mut App| {
        app.classroom_link_input = target.web_url();
        app.invalidate_classroom_link_check();
        app.checking_classroom_link = true;
        app.classroom_link_generation
    };
    let generation = begin_check(&mut app);
    let _ = app.update(Message::ClassroomWorkLinkChecked(generation, target, Ok(wrong)));
    assert_eq!(app.screen, Screen::Home);
    assert!(app.classroom_link_input.is_some());
    assert!(app.error.is_some());

    let generation = begin_check(&mut app);
    let _ = app.update(Message::CloseClassroomLink);
    let _ = app.update(Message::ClassroomWorkLinkChecked(generation, target, Ok(target)));
    assert_eq!(app.screen, Screen::Home);

    let generation = begin_check(&mut app);
    let _ = app.update(Message::ClassroomWorkLinkChecked(generation, target, Ok(target)));
    assert_eq!(app.screen, Screen::Course(1));
    assert_eq!(app.tab, Tab::YourWork);
    assert!(app.classroom_link_input.is_none());
}

#[test]
fn classroom_link_opens_the_item_and_back_returns_to_its_class() {
    let (mut app, mut snapshot) = fixture();
    app.verify_live_course_count = false;
    snapshot.coursework.get_mut(&1).unwrap()[0].id = "42".into();
    app.install_snapshot(snapshot);
    app.screen = Screen::Course(2);
    app.classroom_link_input = Some("https://classroom.google.com/c/MQ/a/NDI/details".into());
    let _ = app.update(Message::OpenClassroomLink);
    assert_eq!(app.screen, Screen::Item(1));
    assert_eq!(app.item_id.as_deref(), Some("42"));
    let _ = app.update(Message::CloseItem);
    assert_eq!((app.screen, app.tab), (Screen::Course(1), Tab::Classwork));
    app.classroom_link_input = Some("https://classroom.google.com/c/999999999".into());
    let _ = app.update(Message::OpenClassroomLink);
    assert_eq!(app.screen, Screen::Course(1));
    assert!(app.error.is_some());
}

#[test]
fn topic_link_opens_classwork_and_keeps_selection_when_items_load() {
    let (mut app, mut snapshot) = fixture();
    app.verify_live_course_count = false;
    let mut items = snapshot.coursework.remove(&1).unwrap();
    items.iter_mut().find(|item| item.kind == CourseworkKind::Assignment).unwrap()
        .topic_id = Some("456".into());
    app.install_snapshot(snapshot);
    app.screen = Screen::Course(2);
    app.classroom_link_input = Some("https://classroom.google.com/u/0/w/MQ/tc/NDU2".into());
    let _ = app.update(Message::OpenClassroomLink);
    assert_eq!((app.screen, app.tab), (Screen::Course(1), Tab::Classwork));
    assert_eq!(app.selected_topic.as_deref(), Some("456"));
    let _ = app.update(Message::CourseworkLoaded(1, Ok(CourseContents { items, topics: None })));
    assert_eq!(app.selected_topic.as_deref(), Some("456"));
    assert_eq!(app.snapshot.as_ref().unwrap().classwork_sections(1, Some("456")).len(), 1);
}

#[test]
fn material_and_announcement_links_open_matching_items() {
    let (mut app, mut snapshot) = fixture();
    app.verify_live_course_count = false;
    let mut item = snapshot.coursework_for(1).unwrap()[0].clone();
    item.id = "42".into();
    item.kind = CourseworkKind::Material;
    snapshot.coursework.insert(1, vec![item.clone()]);
    app.install_snapshot(snapshot);
    app.classroom_link_input = Some("https://classroom.google.com/c/MQ/m/NDI/details".into());
    let _ = app.update(Message::OpenClassroomLink);
    assert_eq!((app.screen, app.tab), (Screen::Item(1), Tab::Classwork));
    assert_eq!(item_link_kind(&item), Some(ItemLinkKind::Material));
    let _ = app.update(Message::CloseItem);

    item.kind = CourseworkKind::Announcement;
    app.snapshot.as_mut().unwrap().coursework.insert(1, vec![item.clone()]);
    app.classroom_link_input = Some("https://classroom.google.com/c/MQ/p/NDI/details".into());
    let _ = app.update(Message::OpenClassroomLink);
    assert_eq!((app.screen, app.tab), (Screen::Item(1), Tab::Stream));
    assert_eq!(item_link_kind(&item), Some(ItemLinkKind::Announcement));
}

#[test]
fn class_calendar_shortcuts_require_a_live_course_and_never_survive_an_offline_copy() {
    let (mut app, mut snapshot) = fixture();
    app.verify_live_course_count = false;
    snapshot.courses[0].calendar_url = Some("https://calendar.google.com/calendar/u/0?cid=fixture".into());
    app.install_snapshot(snapshot.clone());
    assert!(app.class_calendar_button(1).is_some());
    assert!(app.class_calendar_button(2).is_none());
    snapshot.source = SnapshotSource::Cache;
    app.install_snapshot(snapshot);
    assert!(app.class_calendar_button(1).is_none());
}

#[test]
fn hiding_classwork_on_stream_does_not_remove_it_from_classwork() {
    let (_, mut snapshot) = fixture();
    snapshot.courses[0].stream_classwork_display = Some(StreamClassworkDisplay::Hidden);
    let display = snapshot.courses[0].stream_classwork_display.unwrap();
    let kinds: Vec<_> = snapshot.coursework_for(1).unwrap().iter().map(|item| item.kind).collect();
    assert_eq!(kinds.iter().copied().filter(|&kind| display.shows(kind)).collect::<Vec<_>>(),
        [CourseworkKind::Announcement]);
    assert_eq!(snapshot.classwork_sections(1, None).iter().map(|section| section.items.len()).sum::<usize>(), 2);
}

#[test]
fn mark_done_label_applies_only_to_an_editable_submission_without_attachments() {
    use desktop_auth::SubmissionAction::{TurnIn, Unsubmit};
    let mut submission = Submission::default();
    assert!(assignment_is_mark_done(TurnIn, Some(&submission)));
    assert!(!assignment_is_mark_done(Unsubmit, Some(&submission)));
    assert!(!assignment_is_mark_done(TurnIn, None));
    submission.attachments.push(desktop_data::Attachment {
        title: "Sample".into(), kind: desktop_data::AttachmentKind::Link,
        url: Some("https://example.org".into()), remote_id: Some(1),
        drive_sharing: None, drive_reference: None, drive_parent_reference: None,
    });
    assert!(!assignment_is_mark_done(TurnIn, Some(&submission)));
}

#[test]
fn failed_submission_update_is_shown_as_an_error() {
    let (mut app, _) = fixture();
    app.sending_submission = true;
    let _ = app.update(Message::SubmissionSent(1, Err("Classroom did not update the submission.".into())));
    assert!(!app.sending_submission);
    assert_eq!(app.error.as_deref(), Some("Classroom did not update the submission."));
    assert!(app.notice.is_none());
}

#[test]
fn uncertain_comment_result_triggers_one_automatic_read_with_cached_comments() {
    let (mut app, mut snapshot) = fixture();
    let item = &mut snapshot.coursework.get_mut(&1).unwrap()[0];
    item.id = "42".into();
    item.comments = Some(vec![]);
    app.install_snapshot(snapshot.clone());
    let _ = app.update(Message::OpenItem(1, "42".into()));
    let _ = app.update(Message::CommentPosted(1, 42, Err("Connection ended before result".into())));
    assert_eq!(app.next_page_read(&snapshot), Some(PageRead::StreamAccess(1)));
    app.loading_stream_access.insert(1);
    assert_eq!(app.next_page_read(&snapshot), Some(PageRead::Detail(DetailRead::Comments(1, 42))));
    let _ = app.update(Message::DetailLoaded(DetailRead::Comments(1, 42), Err("Read timed out".into())));
    assert_ne!(app.next_page_read(&snapshot), Some(PageRead::Detail(DetailRead::Comments(1, 42))));
    assert!(app.comment_unconfirmed.contains(&(1, 42)));
}

#[test]
fn item_keeps_loading_after_a_section_fails_without_retrying_in_a_loop() {
    let (mut app, mut snapshot) = fixture();
    let assignment = snapshot.coursework.get_mut(&1).unwrap()
        .iter_mut().find(|item| item.kind == CourseworkKind::Assignment).unwrap();
    assignment.id = "42".into();
    assignment.submission.as_mut().unwrap().remote_id = Some(43);

    app.screen = Screen::Item(1);
    app.item_id = Some("42".into());
    app.page_errors.insert(PageRead::StreamAccess(1), "Permission request failed.".into());
    let comments = PageRead::Detail(DetailRead::Comments(1, 42));
    let submissions = PageRead::Detail(DetailRead::Submissions(1));
    let feedback = PageRead::Detail(DetailRead::PrivateFeedback(1, 42));
    assert_eq!(app.next_page_read(&snapshot), Some(comments));
    app.page_errors.insert(comments, "Comments request failed.".into());
    assert_eq!(app.next_page_read(&snapshot), Some(submissions));
    app.submissions_checked.insert(1);
    assert_eq!(app.next_page_read(&snapshot), Some(PageRead::Detail(DetailRead::Rubric(1, 42))));
    app.rubrics.insert((1, 42), None);
    assert_eq!(app.next_page_read(&snapshot), Some(feedback));
    app.private_feedback.insert((1, 42), vec![]);
    assert_eq!(app.next_page_read(&snapshot), None);
}

#[test]
fn assignment_batch_keeps_completed_classes_when_navigation_cancels_pending_reads() {
    let (mut app, mut snapshot) = fixture();
    snapshot.coursework.remove(&1);
    snapshot.coursework.remove(&2);
    app.install_snapshot(snapshot);
    app.screen = Screen::Home;
    let (_task, handle) = Task::<Message>::none().abortable();
    app.assignment_generation = 1;
    app.assignment_batch = Some(AssignmentBatch {
        handle: handle.clone(),
        remaining: BTreeSet::from([1, 2]),
        failed_classwork: 0,
        failed_status: 0,
    });
    let (_, data) = fixture();
    let ready = desktop_auth::AssignmentRefresh {
        course_id: 1,
        contents: Some(CourseContents { items: data.coursework_for(1).unwrap().to_vec(), topics: None }),
        submissions: Some(vec![]),
    };
    let _ = app.update(Message::AssignmentLoaded(1, ready));
    assert!(app.snapshot.as_ref().unwrap().coursework_for(1).is_some());
    assert!(app.snapshot.as_ref().unwrap().coursework_for(2).is_none());
    assert!(!app.assignments_loaded);
    assert!(app.assignment_batch.is_some());

    let _ = app.update(Message::Navigate(Screen::Calendar));
    let _ = app.update(Message::Navigate(Screen::ToDo));
    assert!(!handle.is_aborted());
    let _ = app.update(Message::Navigate(Screen::Course(2)));
    assert!(handle.is_aborted());
    assert!(app.assignment_batch.is_none());
    assert!(!app.queued_page_load);

    let (_task, handle) = Task::<Message>::none().abortable();
    app.screen = Screen::Calendar;
    app.assignment_batch = Some(AssignmentBatch {
        handle,
        remaining: BTreeSet::from([2]),
        failed_classwork: 0,
        failed_status: 0,
    });
    let late = desktop_auth::AssignmentRefresh {
        course_id: 2,
        contents: Some(CourseContents { items: vec![], topics: None }),
        submissions: None,
    };
    let _ = app.update(Message::AssignmentLoaded(1, late));
    assert!(app.snapshot.as_ref().unwrap().coursework_for(2).is_none());
    assert!(!app.assignment_courses().contains(&1));
    assert!(app.assignment_courses().contains(&2));

    let _ = app.update(Message::AssignmentLoaded(app.assignment_generation, desktop_auth::AssignmentRefresh {
        course_id: 2, contents: None, submissions: None,
    }));
    assert!(app.assignments_loaded);
    assert!(app.assignment_batch.is_none());
    assert!(app.page_errors.contains_key(&PageRead::Assignments));
    assert!(app.assignment_courses().contains(&2));
}

#[test]
fn page_navigation_remains_queued_during_an_existing_request() {
    let (mut app, _) = fixture();
    app.loading_coursework = Some(2);
    let _ = app.update(Message::Navigate(Screen::Course(1)));
    let _ = app.update(Message::SetTab(Tab::People));
    assert!(app.queued_page_load);
    assert_eq!((app.screen, app.tab), (Screen::Course(1), Tab::People));

    let _ = app.update(Message::CourseworkLoaded(2, Ok(CourseContents { items: vec![], topics: None })));
    assert!(!app.queued_page_load);
}
