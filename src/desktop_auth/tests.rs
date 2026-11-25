use super::*;
use super::courses::student_meet_url;
use super::coursework::{
    classwork_request, question_details,
};
use super::submissions::question_answer_request;
use super::people::class_teacher_names;
use super::class_notifications::{
    own_class_notifications, update_class_notifications_request,
};
use super::comments::decode_private_feedback;
use super::enrollment::decline_invitation_request;

#[test]
fn class_teacher_names_only_use_the_course_owner_ids() {
    let names = BTreeMap::from([(1, " Alice ".into()), (2, "Bob".into()), (3, "Not an owner".into())]);
    assert_eq!(class_teacher_names(&[(7, vec![9, 1]), (8, vec![2]), (10, vec![9])], &names),
        BTreeMap::from([(7, "Alice".into()), (8, "Bob".into())]));
}

#[test]
fn decline_invitation_encodes_android_course_id_and_invitee() {
    assert_eq!(
        decline_invitation_request(23, 156).encode_to_vec(),
        [0x0a, 7, 0x0a, 2, 0x08, 23, 0x10, 0x9c, 0x01, 0x10, 2],
    );
}

#[test]
fn class_notifications_use_the_current_user_preference_and_no_student_mute_field() {
    for muted in [false, true] {
        let request = update_class_notifications_request(7, muted);
        let [entry] = request.course_mutation.as_slice() else { panic!("Expected one course") };
        let course = wire::Course::decode(entry.course.as_deref().unwrap()).unwrap();
        assert_eq!(course.course_id.unwrap().id, Some(7));
        assert!(course.muted_user_id.is_empty());
        assert_eq!(course.current_user_setting.unwrap().notifications_muted, Some(muted));
        let mask = mutation::CourseUpdateMask::decode(entry.course_update_mask.as_deref().unwrap()).unwrap();
        assert!(mask.course_setting_update_mask.is_none());
        assert_eq!(mask.current_user_setting_update_mask.unwrap().update_notifications_muted, Some(true));
    }
}

#[test]
fn class_notifications_only_use_unique_active_classes_and_default_to_enabled() {
    let row = |course, state, notifications_muted| wire::Course {
        course_id: Some(wire::CourseId { id: Some(course) }), course_state: Some(state),
        current_user_setting: Some(wire::CourseUserSetting { notifications_muted, ..Default::default() }),
        muted_user_id: vec![vec![8, 7]], ..Default::default()
    };
    let response = wire::QueryCourseResponse { course: vec![
        row(1, 1, None), row(2, 1, Some(true)), row(3, 2, Some(true)),
        row(4, 1, Some(true)), row(4, 1, Some(false)), row(5, 1, Some(true)),
    ] };
    assert_eq!(own_class_notifications(response, &BTreeSet::from([1, 2, 3, 4])),
        BTreeMap::from([(1, false), (2, true)]));
}

#[test]
fn short_answer_wire_shape_sets_answer_and_state_in_one_update() {
    let id = classwork::SubmissionId {
        id: Some(27),
        stream_item_id: Some(classwork::StreamItemId {
            id: Some(34), course_id: Some(classwork::CourseId { id: Some(56) }),
        }),
    };
    let request = question_answer_request(id.clone(), QuestionAnswer::ShortAnswer("A\nB".to_owned()));
    let decoded = mutation::WriteSubmissionRequest::decode(request.encode_to_vec().as_slice()).unwrap();
    assert_eq!(decoded.request_header.unwrap().operation, Some(3));
    assert_eq!(decoded.submission_mutation.len(), 1);
    let update = &decoded.submission_mutation[0];
    assert_eq!(update.submission_id.as_ref(), Some(&id));
    assert!(update.add_materials.is_empty());
    let submission = update.submission.as_ref().unwrap();
    assert_eq!(submission.submission_id.as_ref(), Some(&id));
    assert_eq!(submission.current_state, Some(2));
    assert_eq!(submission.question_submission.as_ref().unwrap().encode_to_vec(),
        [0x12, 3, b'A', b'\n', b'B']);
    assert_eq!(update.submission_update_mask.as_ref().unwrap().encode_to_vec(),
        [0x08, 1, 0x42, 2, 0x08, 1]);
}

#[test]
fn multiple_choice_request_encodes_the_choice_field_and_shared_update_mask() {
    let id = classwork::SubmissionId { id: Some(42), stream_item_id: None };
    let request = question_answer_request(id, QuestionAnswer::MultipleChoice("Option A".into()));
    let decoded = mutation::WriteSubmissionRequest::decode(request.encode_to_vec().as_slice()).unwrap();
    let mutation = &decoded.submission_mutation[0];
    let answer = mutation.submission.as_ref().unwrap().question_submission.as_ref().unwrap();
    assert_eq!(answer.encode_to_vec(), [0x08, 2, 0x1a, 8, b'O', b'p', b't', b'i', b'o', b'n', b' ', b'A']);
    assert_eq!(mutation.submission_update_mask.as_ref().unwrap().encode_to_vec(), [0x08, 1, 0x42, 2, 0x08, 1]);
    assert_eq!(mutation.submission.as_ref().unwrap().current_state, Some(2));
}

#[test]
fn editing_checks_permission_state_and_exact_original_answer() {
    let snapshot = Snapshot::demo();
    let mut question = snapshot.coursework_for(4).unwrap()[0].clone();
    question.submission = Some(crate::desktop_data::Submission {
        remote_id: Some(27), remote_state: Some(2), status: SubmissionStatus::TurnedIn,
        question_answer: Some(QuestionAnswer::ShortAnswer("Original response".into())),
        ..Default::default()
    });
    let revision = ShortAnswerRevision { submission_id: 27, answer: "Original response".into() };
    assert!(!question_submission_is_current(&question, Some(&revision)));
    question.allow_edit_after_turn_in = true;
    assert!(question_submission_is_current(&question, Some(&revision)));
    assert!(!question_submission_is_current(&question, None));
    assert!(!question_submission_is_current(&question, Some(&ShortAnswerRevision {
        submission_id: 28, answer: revision.answer.clone(),
    })));
    assert!(!question_submission_is_current(&question, Some(&ShortAnswerRevision {
        submission_id: 27, answer: "Other answer".into(),
    })));
    question.submission.as_mut().unwrap().status = SubmissionStatus::Returned;
    assert!(!question_submission_is_current(&question, Some(&revision)));
    question.submission.as_mut().unwrap().status = SubmissionStatus::TurnedInLate;
    assert!(question_submission_is_current(&question, Some(&revision)));
    question.submission.as_mut().unwrap().grade = Some(Grade { earned: 0.0, maximum: Some(5.0) });
    assert!(!question_submission_is_current(&question, Some(&revision)));
}

#[test]
fn first_question_answer_requires_an_editable_existing_own_submission() {
    let snapshot = Snapshot::demo();
    let mut question = snapshot.coursework_for(4).unwrap()[0].clone();
    assert!(!can_turn_in_question(&question));
    question.submission = Some(crate::desktop_data::Submission {
        remote_id: Some(27), remote_state: Some(4), ..Default::default()
    });
    assert!(can_turn_in_question(&question));
    question.submission.as_mut().unwrap().question_answer = Some(QuestionAnswer::ShortAnswer("Existing answer".into()));
    assert!(!can_turn_in_question(&question));
    question.submission.as_mut().unwrap().question_answer = None;
    question.question = Some(QuestionDetails::MultipleChoice(vec!["A".into()]));
    assert!(can_turn_in_question(&question));
    question.kind = CourseworkKind::Assignment;
    assert!(!can_turn_in_question(&question));
}

#[test]
fn question_choice_order_and_answer_type_survive_wire_decoding() {
    let source = classwork::QuestionInfo {
        question_type: Some(2),
        multiple_choice: Some(classwork::MultipleChoice { choice: vec!["B".into(), "A".into()] }),
    };
    let read = classwork::QuestionInfo::decode(source.encode_to_vec().as_slice()).unwrap();
    assert_eq!(question_details(read), Some(QuestionDetails::MultipleChoice(vec!["B".into(), "A".into()])));
    assert_eq!(question_details(classwork::QuestionInfo { question_type: Some(1), multiple_choice: None }),
        Some(QuestionDetails::ShortAnswer));
    assert!(question_details(classwork::QuestionInfo { question_type: Some(2), multiple_choice: None }).is_none());
    for (kind, expected) in [
        (1, Some(QuestionAnswer::ShortAnswer("Explanation".into()))),
        (2, Some(QuestionAnswer::MultipleChoice("A".into()))),
        (0, None),
    ] {
        let raw = classwork::QuestionSubmission { question_answer_type: Some(kind),
            short_answer: Some("Explanation".into()), multiple_choice_answer: Some("A".into()) };
        assert_eq!(own_question_answer(classwork::QuestionSubmission::decode(raw.encode_to_vec().as_slice()).unwrap()), expected);
    }
}

#[test]
fn class_folders_match_requested_classes_and_signed_in_user() {
    use classwork::*;
    let build = |course, user, inner_user, url: &str| {
        let key = |user| CourseUserId {
            course_id: Some(CourseId { id: Some(course) }),
            user_id: Some(UserId { id: Some(user) }),
        };
        CourseUserLookupResult {
            course_user_id: Some(key(user)),
            course_user: Some(CourseUser {
                course_user_id: Some(key(inner_user)),
                course_role: Some(2),
                course_folder: Some(Folder { open_url: Some(url.into()) }),
                ..Default::default()
            }),
        }
    };
    let good = "https://drive.google.com/drive/folders/test";
    let response = CourseUserLookupResponse { results: vec![
        build(1, 15, 15, good),
        build(2, 14, 14, good),
        build(3, 15, 14, good),
        build(4, 15, 15, "https://drive.google.com.example.invalid/folders/test"),
        build(5, 15, 15, "https://drive.google.com@other.example.invalid/folders/test"),
        build(6, 15, 15, good),
    ] };
    let decoded = CourseUserLookupResponse::decode(response.encode_to_vec().as_slice()).unwrap();
    assert_eq!(own_class_details(decoded.clone(), &BTreeSet::from([1]), 15).roles, BTreeMap::from([(1, CourseRole::Teacher)]));
    assert_eq!(own_class_details(decoded, &BTreeSet::from([1, 2, 3, 4, 5]), 15).folders,
        BTreeMap::from([(1, good.to_owned())]));
}

#[test]
fn meet_link_requires_the_student_visible_state_and_google_meet_host() {
    let link = "https://meet.google.com/lookup/demo";
    for state in [None, Some(0), Some(1), Some(2), Some(3), Some(4)] {
        let info = wire::VideoCallInfo { video_call_url_state: state,
            video_call_url: Some(link.into()), ..Default::default() };
        assert_eq!(student_meet_url(info).as_deref(), (state == Some(3)).then_some(link));
    }
    let unsafe_link = wire::VideoCallInfo { video_call_url_state: Some(3),
        video_call_url: Some("https://meet.google.com.demo.invalid/lookup/session".into()),
        ..Default::default() };
    assert!(student_meet_url(unsafe_link).is_none());
}

#[test]
fn a_late_rejection_does_not_invalidate_a_renewed_token() {
    let rejected = Arc::new(Zeroizing::new("old-fixture".to_owned()));
    let fresh = Arc::new(Zeroizing::new("new-fixture".to_owned()));
    let mut token = ClassroomToken { bearer: fresh.clone(), expires_at: Some(1_100) };
    assert!(!token.expires_within(1_000, 30));
    assert!(token.expires_within(1_070, 30));
    assert!(!token.invalidate(&rejected));
    assert!(!token.expires_within(1_000, 0));
    assert!(token.invalidate(&fresh));
    assert!(token.expires_within(1_000, 0));
}

#[tokio::test]
async fn a_rejected_read_retries_once_using_the_concurrently_renewed_token() {
    use std::sync::atomic::AtomicUsize;
    use tokio::{net::TcpListener, sync::oneshot};
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::{Request, Response, transport::{Endpoint, Server}};
    use wire::homeroom_data_service_server::{HomeroomDataService, HomeroomDataServiceServer};

    struct ReadServer {
        credentials: Arc<Mutex<SessionCredentials>>,
        calls: Arc<AtomicUsize>,
    }

    #[tonic::async_trait]
    impl HomeroomDataService for ReadServer {
        async fn query_course(&self, request: Request<wire::QueryCourseRequest>)
            -> Result<Response<wire::QueryCourseResponse>, Status>
        {
            assert_eq!(request.get_ref(), &course_details_request());
            let bearer = request.metadata().get("authorization").unwrap();
            if self.calls.fetch_add(1, Ordering::Relaxed) == 0 {
                assert_eq!(bearer, "Bearer old-fixture");
                // Simulate another in-flight read finishing renewal ahead of this rejection.
                self.credentials.lock().await.token = ClassroomToken {
                    bearer: Arc::new(Zeroizing::new("new-fixture".into())), expires_at: None,
                };
                return Err(Status::unauthenticated("synthetic expiry"));
            }
            assert_eq!(bearer, "Bearer new-fixture");
            Ok(Response::new(wire::QueryCourseResponse::default()))
        }

        async fn query_stream_item(&self, _: Request<wire::StreamItemQueryRequest>)
            -> Result<Response<wire::StreamItemQueryResponse>, Status>
        {
            Err(Status::unimplemented("outside this fixture"))
        }

        async fn query_submission(&self, _: Request<wire::QuerySubmissionRequest>)
            -> Result<Response<wire::QuerySubmissionResponse>, Status>
        {
            Err(Status::unimplemented("outside this fixture"))
        }
    }

    let credentials = Arc::new(Mutex::new(SessionCredentials {
        token: ClassroomToken { bearer: Arc::new(Zeroizing::new("old-fixture".into())), expires_at: None },
        failure: None,
    }));
    let calls = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = Endpoint::from_shared(format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let (stop, stopped) = oneshot::channel();
    let service = ReadServer { credentials: credentials.clone(), calls: calls.clone() };
    let server = tokio::spawn(Server::builder().add_service(HomeroomDataServiceServer::new(service))
        .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async { let _ = stopped.await; }));
    let session = LiveSession {
        snapshot: Snapshot::live(vec![]),
        account: "local-fixture@example.org".into(),
        credentials,
        needs_sign_in: Arc::new(AtomicBool::new(false)),
        channel: endpoint.connect().await.unwrap(),
    };
    assert!(refresh(&session).await.is_ok());
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert!(!session.needs_sign_in());
    drop(session);
    stop.send(()).unwrap();
    server.await.unwrap().unwrap();
}

#[tokio::test]
#[ignore = "requires the owner's saved Google Workspace session and sends live read-only requests"]
async fn live_saved_session_renews_after_simulated_expiry() {
    let account = remembered_account().expect("A saved Google Workspace account is required");
    let session = restore_session(account).await.expect("Saved session should restore");
    let previous = {
        let mut credentials = session.credentials.lock().await;
        credentials.token.expires_at = Some(0);
        credentials.token.bearer.clone()
    };

    let (first, second) = tokio::join!(session.bearer_for_request(None), session.bearer_for_request(None));
    let first = first.expect("First read acquires a renewed token");
    let second = second.expect("Concurrent read uses the renewed token");
    assert!(!Arc::ptr_eq(&first, &previous));
    assert!(Arc::ptr_eq(&first, &second));
    assert!(!session.needs_sign_in());

    let (courses, user) = tokio::join!(refresh(&session), check_current_user(&session));
    let courses = courses.expect("Renewed token should read courses");
    assert!(!courses.courses.is_empty());
    user.expect("Renewed token should read the signed-in user's identity");
    println!("Renewed in memory and verified course and identity reads; courses={}", courses.courses.len());
}

#[test]
fn feedback_requires_active_private_comment_on_the_exact_submission() {
    let target = classwork::SubmissionId {
        id: Some(100),
        stream_item_id: Some(classwork::StreamItemId {
            id: Some(200),
            course_id: Some(classwork::CourseId { id: Some(300) }),
        }),
    };
    let feedback = classwork::Comment {
        comment_id: Some(classwork::CommentId {
            id: Some(4),
            submission_id: Some(target.clone()),
            ..Default::default()
        }),
        status: Some(2),
        comment: Some("Synthetic feedback".into()),
        visibility: Some(classwork::CommentVisibility { visibility_type: Some(1) }),
        ..Default::default()
    };
    assert_eq!(decode_private_feedback(feedback.clone(), &target).unwrap().0, 4);
    let mut wrong_course = target.clone();
    wrong_course.stream_item_id.as_mut().unwrap().course_id.as_mut().unwrap().id = Some(301);
    assert!(decode_private_feedback(feedback.clone(), &wrong_course).is_none());
    let mut public = feedback.clone();
    public.visibility = Some(classwork::CommentVisibility { visibility_type: Some(2) });
    assert!(decode_private_feedback(public, &target).is_none());
    let mut trashed = feedback;
    trashed.status = Some(3);
    assert!(decode_private_feedback(trashed, &target).is_none());
}

#[test]
fn classwork_read_limits_the_request_to_active_published_items_in_one_class() {
    let request = classwork_request(412);
    assert_eq!(request.filter.len(), 1);
    let filter = classwork::StreamFilter::decode(request.filter[0].as_slice()).unwrap();
    assert_eq!(filter.course_id[0].id, Some(412));
    assert_eq!(filter.allowed_status, [2]);
    assert_eq!(filter.publication_status, [2]);
    let mask = classwork::StreamReadMask::decode(request.read_mask.unwrap().as_slice()).unwrap();
    let info = mask.assignment_mask.unwrap().stream_item_info_mask.unwrap();
    assert_eq!(info.include_title, Some(true));
    assert_eq!(info.include_creation_timestamp, Some(true));
    assert_eq!(info.publication_info_mask.unwrap().include_publication_timestamp, Some(true));
}

#[test]
fn classwork_decode_refuses_items_from_other_courses() {
    let item = classwork::StreamItem {
        assignment: Some(classwork::Assignment {
            stream_item_info: Some(classwork::StreamItemInfo {
                stream_item_id: Some(classwork::StreamItemId {
                    id: Some(9),
                    course_id: Some(classwork::CourseId { id: Some(412) }),
                }),
                title: Some("Synthetic assignment".into()),
                creation_timestamp: Some(1_700_000_000_000),
                publication_info: Some(classwork::PublicationInfo {
                    publication_timestamp: Some(1_700_000_060_000),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            task_info: None,
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(coursework(411, item.clone()).is_none());
    let decoded = coursework(412, item.clone()).unwrap();
    assert_eq!(decoded.id, "9");
    assert_eq!(decoded.title, "Synthetic assignment");
    assert_eq!(decoded.posted_at_millis, Some(1_700_000_060_000));
    let mut item = item;
    item.assignment.as_mut().unwrap().stream_item_info.as_mut().unwrap().publication_info = None;
    assert_eq!(coursework(412, item).unwrap().posted_at_millis, Some(1_700_000_000_000));
}

#[test]
fn sign_in_normalizes_and_bounds_account_input() {
    assert_eq!(normalize_account(" Fixture@EXAMPLE.ORG ").unwrap(), "fixture@example.org");
    assert_eq!(normalize_account("fixture@school.example").unwrap(), "fixture@school.example");
    assert!(normalize_account("fixture@-example.org").is_err());
    assert!(normalize_account("fixture@example..org").is_err());
    assert!(normalize_account("fixture@example.org@example.org").is_err());
    let long = format!("{}@{}.{}.example", "a".repeat(128), "b".repeat(63), "c".repeat(63));
    assert!(normalize_account(&long).is_err());
    assert!(normalize_account("fixture@example.org\nsecond@example.org").is_err());
}
