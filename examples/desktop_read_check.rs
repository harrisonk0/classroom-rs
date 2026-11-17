#[cfg(feature = "desktop")]
#[tokio::main]
async fn main() {
    use classroom_rust::{desktop_auth as auth, desktop_data::CourseState};
    use iced::futures::StreamExt;
    let Some(account) = auth::remembered_account() else {
        println!("No saved account preference.");
        return;
    };
    let session = match auth::restore_session(account).await {
        Ok(session) => session,
        Err(error) => { println!("Session failed: {error}"); return; }
    };
    if let Some(mode @ ("--attachment-save-check" | "--attachment-preview-check")) = std::env::args().nth(1).as_deref() {
        let args: Vec<_> = std::env::args().skip(2).collect();
        let [course, item, file] = args.as_slice() else {
            panic!("Pass the course ID, item ID and Drive ID of one visible binary attachment");
        };
        let course = course.parse().expect("Read a positive class ID");
        assert!(session.snapshot.courses.iter().any(|entry| entry.id == course));
        let contents = auth::load_coursework(&session, course).await.expect("Read the selected class's published work");
        let attachment = contents.items.iter().filter(|entry| &entry.id == item)
            .flat_map(|entry| &entry.attachments)
            .find(|entry| entry.drive_reference.as_deref() == Some(file.as_str()))
            .expect("Confirm the selected attachment belongs to the published item");
        let directory = tempfile::tempdir().expect("Create a temporary download destination");
        let output = if mode == "--attachment-preview-check" {
            auth::preview_drive_file(&session, file, attachment.url.as_deref(), directory.path())
                .await.expect("Prepare the exact attachment for Quick Look")
        } else {
            let output = directory.path().join("attachment.bin");
            auth::save_drive_file(&session, file, attachment.url.as_deref(), &output)
                .await.expect("Download the exact attachment without modifying Drive");
            output
        };
        let bytes = std::fs::metadata(&output).expect("Read the saved file length").len();
        assert!(bytes > 0);
        directory.close().expect("Remove the temporary download");
        println!("Selected attachment saved and temporary file removed; bytes={bytes}");
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--mention-coverage") {
        let course = session.snapshot.courses.iter().find(|course| course.shows_on_home())
            .expect("Read one visible class");
        let members = auth::load_class_mentions(&session, course.id).await.expect("Read the visible class mention list");
        println!("Addressable class members returned={}", members.len());
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--your-work-link") {
        use classroom_rust::classroom_links::ClassroomLink;
        let supplied = std::env::args().nth(2).map(|url|
            ClassroomLink::parse(&url).expect("Read a valid Classroom Your work URL"));
        let course = supplied.map(ClassroomLink::course).or_else(|| session.snapshot.courses.iter()
            .find(|course| course.shows_on_home()).map(|course| course.id)).expect("Read an enrolled class");
        assert!(session.snapshot.courses.iter().any(|entry| entry.id == course), "Read an enrolled class");
        let own = auth::own_work_link(&session, course).await.expect("Read the signed-in Classroom identity");
        assert_eq!(ClassroomLink::parse(&own.web_url().unwrap()), Some(own));
        if let Some(supplied) = supplied {
            assert_eq!(supplied, own, "The supplied route must match the signed-in account");
        }
        let submissions = auth::load_submissions(&session, course).await.expect("Read only the signed-in user’s work");
        println!("Your work route matched the signed-in account; submissions={}", submissions.len());
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--class-creation-permission") {
        let allowed = auth::can_create_classes(&session).await.expect("Read account class creation permission");
        println!("Account can create classes={allowed}");
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--personal-copy-access") {
        let courses = session.snapshot.courses.iter().filter(|course| course.state == CourseState::Archived)
            .map(|course| course.id).collect();
        let session = std::sync::Arc::new(session);
        let mut updates = Box::pin(auth::assignment_updates(session.clone(), courses));
        while let Some(update) = updates.next().await {
            let (Some(contents), Some(submissions)) = (update.contents, update.submissions) else { continue };
            for item in contents.items {
                let Some(own) = submissions.iter().find(|(id, _)| id == &item.id).map(|(_, data)| data) else { continue };
                for template in &item.attachments {
                    let mut copies = own.attachments.iter().filter(|copy| copy.is_personal_copy_of(template));
                    let Some(id) = copies.next().and_then(|copy| copy.drive_reference.as_deref()) else { continue };
                    if copies.next().is_some() { continue; }
                    let url = auth::lookup_drive_file_link(&session, id).await.expect("Read one exact personal-copy Drive link");
                    assert_eq!(auth::drive_url_reference(&url).as_deref(), Some(id));
                    println!("Drive returned a browser link for an existing, exact personal copy");
                    return;
                }
            }
        }
        panic!("No existing personal copy was eligible for a metadata check");
    }
    if std::env::args().nth(1).as_deref() == Some("--originality-settings") {
        use classroom_rust::desktop_data::CourseworkKind;
        let courses = session.snapshot.courses.iter().filter(|course| course.state == CourseState::Active)
            .map(|course| course.id).collect::<Vec<_>>();
        let reads = iced::futures::stream::iter(courses.iter().copied()).map(|course| {
            let session = &session;
            async move { auth::load_coursework(session, course).await }
        }).buffer_unordered(3).collect::<Vec<_>>().await;
        let mut counts = [0, 0, 0];
        for read in reads {
            for item in read.expect("Read published assignment settings").items.into_iter()
                .filter(|item| item.kind == CourseworkKind::Assignment)
            {
                let index = match item.originality_enabled { Some(true) => 0, Some(false) => 1, None => 2 };
                counts[index] += 1;
                assert_eq!(auth::originality_report_url(item.course_id, item.id.parse().unwrap(), index == 0).is_some(), index == 0);
            }
        }
        println!("Active classes={}; assignment originality enabled={}; disabled={}; omitted={}",
            courses.len(), counts[0], counts[1], counts[2]);
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--classwork-order-check") {
        let mut args = std::env::args().skip(2);
        let course: i64 = args.next().expect("Pass a course ID and exact titles in visible order")
            .parse().expect("Pass a numeric Classroom course ID");
        assert!(session.snapshot.courses.iter().any(|entry| entry.id == course));
        let contents = auth::load_coursework(&session, course).await.expect("Read published classwork");
        let mut snapshot = session.snapshot.clone();
        snapshot.coursework.insert(course, contents.items);
        if let Some(topics) = contents.topics {
            snapshot.topics.insert(course, topics);
        }
        let items = snapshot.coursework_for(course).expect("Read published classwork");
        let mut sorted = Vec::new();
        let mut visible_ids = Vec::new();
        for title in args {
            let mut matches = items.iter().filter(|item| item.title == title);
            let item = matches.next().expect("Each visible title must match a returned item");
            assert!(matches.next().is_none(), "Visible title is ambiguous");
            sorted.push(item.classwork_sort_key.as_deref().expect("Each item needs a sort key"));
            visible_ids.push(item.id.as_str());
        }
        assert!(sorted.len() >= 2, "Supply at least two visible classwork titles");
        assert!(snapshot.classwork_sections(course, None).iter().any(|section|
            section.items.windows(visible_ids.len()).any(|window|
                window.iter().map(|item| item.id.as_str()).eq(visible_ids.iter().copied()))),
            "The native topic order differs from the observed web order");
        println!("Visible order matched {} items; native order matches; byte order ascending={}; descending={}",
            sorted.len(), sorted.windows(2).all(|pair| pair[0] < pair[1]),
            sorted.windows(2).all(|pair| pair[0] > pair[1]));
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--class-archive-read") {
        for state in [CourseState::Active, CourseState::Archived] {
            let course = session.snapshot.courses.iter().find(|course| course.state == state)
                .expect("The test account needs an active and an archived class");
            let actual = auth::read_class_state(&session, course.id).await
                .expect("Read exact class state");
            assert_eq!(actual, state);
            println!("Exact class read confirmed {state:?}");
        }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--item-topic-lookups") {
        use classroom_rust::desktop_data::CourseworkKind::*;
        let mut checked = [false; 3];
        for course in session.snapshot.courses.iter().filter(|course| course.shows_on_home()) {
            let items = auth::load_coursework(&session, course.id).await.expect("Read published coursework");
            for item in items.items {
                let index = match item.kind { Assignment => 0, Question => 1, Material => 2, Announcement => continue };
                if checked[index] { continue; }
                let Ok(id) = item.id.parse() else { continue };
                let target = auth::TopicItemTarget { course: course.id, item: id, kind: item.kind, draft: false, original: item.topic_id };
                assert!(auth::check_item_topic_read(&session, &target).await.unwrap_or_else(|error| panic!("Topic lookup type {index}: {error}")));
                checked[index] = true;
            }
            if checked.iter().all(|seen| *seen) { break; }
        }
        println!("Exact topic lookup matched for assignments={}, questions={}, materials={}", checked[0], checked[1], checked[2]);
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--class-roles") {
        use classroom_rust::desktop_data::CourseRole;
        let classes: Vec<_> = session.snapshot.courses.iter().map(|course| course.id).collect();
        let details = auth::load_own_class_details(&session, classes.clone()).await.expect("Read own class roles");
        let teacher: Vec<_> = details.roles.iter().filter_map(|(id, role)| (*role == CourseRole::Teacher).then_some(*id)).collect();
        let student = details.roles.values().filter(|role| **role == CourseRole::Student).count();
        println!("Classes={}; confirmed student={student}; confirmed teacher={}; role omitted={}",
            classes.len(), teacher.len(), classes.len() - details.roles.len());
        for course in teacher {
            let drafts = auth::load_teacher_drafts(&session, course).await.expect("Read teacher-visible drafts");
            println!("Teacher-visible unpublished items={}", drafts.len());
        }
        return;
    }
    if let Some(mode @ ("--document-sharing" | "--document-sharing-archived")) = std::env::args().nth(1).as_deref() {
        use classroom_rust::desktop_data::DriveSharing::*;
        let courses: Vec<_> = session.snapshot.courses.iter().filter(|course| {
            if mode == "--document-sharing-archived" { course.state == CourseState::Archived } else { course.shows_on_home() }
        })
            .map(|course| course.id).collect();
        let expected = courses.len();
        let mut updates = Box::pin(auth::assignment_updates(std::sync::Arc::new(session), courses));
        let mut classes = 0;
        let mut counts = [0; 4];
        let mut linked_copies = 0;
        let mut openable_copies = 0;
        let mut own_parent_refs = 0;
        while let Some(update) = updates.next().await {
            let Some(contents) = update.contents else { continue };
            classes += 1;
            for item in contents.items {
                let submission = update.submissions.as_ref().and_then(|submissions| {
                    submissions.iter().find(|(id, _)| id == &item.id).map(|(_, data)| data)
                });
                if let Some(submission) = submission {
                    own_parent_refs += submission.attachments.iter().filter(|attachment| attachment.drive_parent_reference.is_some()).count();
                }
                for attachment in &item.attachments {
                    match attachment.drive_sharing {
                        Some(IndividualCopy) => {
                            counts[0] += 1;
                            if let Some(submission) = submission {
                                let mut copies = submission.attachments.iter().filter(|file| file.is_personal_copy_of(attachment));
                                if let Some(copy) = copies.next() {
                                    linked_copies += 1;
                                    if copies.next().is_none() && copy.url.as_deref().and_then(auth::drive_url_reference) == copy.drive_reference {
                                        openable_copies += 1;
                                    }
                                }
                            }
                        }
                        Some(View) => counts[1] += 1,
                        Some(Edit) => counts[2] += 1,
                        Some(CopyToKeep) => counts[3] += 1,
                        None => {}
                    }
                }
            }
        }
        println!("Classes read={classes}/{expected}; personal templates={}; view-only={}; class-editable={}; copy-to-keep={}; own parent references={own_parent_refs}; matched copies={linked_copies}; verified editor URLs={openable_copies}",
            counts[0], counts[1], counts[2], counts[3]);
        if classes != expected { std::process::exit(1); }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--submission-history") {
        use classroom_rust::desktop_data::SubmissionStatus::*;
        let mut checked = 0;
        let mut denied = 0;
        let mut populated = 0;
        let mut events = 0;
        let mut submitted = 0;
        let mut with_timestamps = 0;
        for course in session.snapshot.courses.iter().filter(|course| course.shows_on_home()) {
            let submissions = auth::load_submissions(&session, course.id).await.expect("Read own submissions");
            for (item, submission) in submissions {
                if !matches!(submission.status, Returned | ReturnedLate | TurnedIn | TurnedInLate) {
                    continue;
                }
                submitted += 1;
                with_timestamps += usize::from(submission.last_turned_in_millis.is_some());
                if checked == 6 { continue; }
                let (Ok(item), Some(id)) = (item.parse(), submission.remote_id) else { continue };
                checked += 1;
                match auth::load_own_submission_history(&session, course.id, item, id).await {
                    Ok(history) => {
                        populated += usize::from(!history.is_empty());
                        events += history.len();
                    }
                    Err(error) => {
                        denied += 1;
                        if denied == 1 {
                            println!("First own history read failure: {error}");
                        }
                    }
                }
            }
        }
        println!("Own histories checked={checked}; failed={denied}; populated={populated}; decoded events={events}; turned-in or returned={submitted}; with timestamp={with_timestamps}");
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--class-teachers") {
        let courses: Vec<_> = session.snapshot.courses.iter().filter(|course| course.shows_on_home())
            .map(|course| (course.id, course.owner_ids.clone())).collect();
        let names = auth::load_class_teacher_names(&session, &courses).await.expect("Read owner display names");
        println!("Visible classes={}; with owner IDs={}; with display names={}", courses.len(),
            courses.iter().filter(|(_, ids)| !ids.is_empty()).count(), names.len());
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--stream-display") {
        use classroom_rust::desktop_data::StreamClassworkDisplay::*;
        let classes: Vec<_> = session.snapshot.courses.iter().filter(|course| course.shows_on_home()).collect();
        let count = |setting| classes.iter().filter(|course| course.stream_classwork_display == setting).count();
        println!("Visible classes={}; condensed={}; expanded={}; hidden={}; unspecified={}",
            classes.len(), count(Some(Condensed)), count(Some(Expanded)), count(Some(Hidden)), count(None));
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--class-calendar-links") {
        let visible: Vec<_> = session.snapshot.courses.iter().filter(|course| course.shows_on_home())
            .filter_map(|course| course.calendar_url.as_deref()).collect();
        let total = session.snapshot.courses.iter().filter(|course| course.calendar_url.is_some()).count();
        let distinct = visible.iter().copied().collect::<std::collections::BTreeSet<_>>().len();
        println!("Calendar links returned={total}; visible class calendar links={}; distinct visible links={distinct}", visible.len());
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--classroom-link") {
        use classroom_rust::classroom_links::{ClassroomLink, ItemLinkKind};
        for course in session.snapshot.courses.iter().filter(|course| course.shows_on_home()) {
            let target = ClassroomLink::Course { course: course.id, classwork: false };
            assert_eq!(ClassroomLink::parse(&target.web_url().unwrap()), Some(target));
            let contents = auth::load_coursework(&session, course.id).await.expect("Load visible assignments");
            if let Some(item) = contents.items.iter().find(|item| item.kind == classroom_rust::desktop_data::CourseworkKind::Assignment) {
                let item_target = ClassroomLink::Item { course: course.id, item: item.id.parse().unwrap(), kind: ItemLinkKind::Assignment };
                assert_eq!(ClassroomLink::parse(&item_target.web_url().unwrap()), Some(item_target));
                println!("Live enrolled class and visible assignment IDs round-tripped through Classroom URLs.");
                return;
            }
        }
        println!("No visible assignment was available for the link test.");
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--comment-target") {
        for course in session.snapshot.courses.iter().filter(|course| course.shows_on_home()) {
            let contents = auth::load_coursework(&session, course.id).await.expect("Read classwork");
            for item in contents.items.iter().take(5) {
                let Some(item_id) = item.id.parse().ok() else { continue };
                let comments = auth::load_comments(&session, course.id, item_id).await.expect("Read existing comments");
                if let Some(comment_id) = comments.iter().find_map(|comment| comment.id.parse().ok()) {
                    let matched = auth::check_class_comment_target(&session, course.id, item_id, comment_id)
                        .await.expect("Read the exact existing comment");
                    println!("Exact class comment read; matched active thread and author={matched}");
                    assert!(matched);
                    return;
                }
            }
        }
        println!("No existing comment was available in the sampled items.");
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--rich-text") {
        let courses: Vec<_> = session.snapshot.courses.iter().filter(|course| course.shows_on_home())
            .map(|course| course.id).collect();
        let total = courses.len();
        let results: Vec<_> = iced::futures::stream::iter(courses).map(|course| {
            let session = &session;
            async move { auth::load_coursework(session, course).await }
        }).buffer_unordered(3).collect().await;
        let loaded = results.iter().filter(|result| result.is_ok()).count();
        let items: Vec<_> = results.iter().filter_map(|result| result.as_ref().ok())
            .flat_map(|contents| &contents.items).collect();
        println!("Rich text class reads={loaded}/{total}; items={}; descriptions={}; styled descriptions={}",
            items.len(), items.iter().filter(|item| !item.description.is_empty()).count(),
            items.iter().filter(|item| item.description_styles.iter().any(|style| style.bold || style.italic || style.underline)).count());
        println!("Descriptions with bullet markers={}; bullet markers={}",
            items.iter().filter(|item| !item.description_bullets.is_empty()).count(),
            items.iter().map(|item| item.description_bullets.len()).sum::<usize>());
        assert_eq!(loaded, total);
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--drive-picker") {
        match auth::list_drive_files(&session, auth::DriveLocation::Recent, String::new(), None).await {
            Ok(page) => {
                println!("Drive picker first page OK; visible files={}; next page={}; incomplete={}",
                    page.files.len(), page.next_page.is_some(), page.incomplete);
                if let Some(token) = page.next_page {
                    match auth::list_drive_files(&session, auth::DriveLocation::Recent, String::new(), Some(token)).await {
                        Ok(next) => println!("Drive picker second page OK; visible files={}", next.files.len()),
                        Err(error) => { println!("Drive picker next page failed: {error}"); std::process::exit(1); }
                    }
                }
            }
            Err(error) => { println!("Drive picker read failed: {error}"); std::process::exit(1); }
        }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--drive-folders") {
        use auth::DriveLocation;
        let mut root = auth::list_drive_files(&session, DriveLocation::Folder("root".into()), String::new(), None)
            .await.expect("Read My Drive root");
        println!("My Drive root OK; folders={}; files={}; next page={}",
            root.folders.len(), root.files.len(), root.next_page.is_some());
        let mut pages = 1;
        while root.folders.is_empty() && pages < 5 {
            let Some(cursor) = root.next_page.take() else { break };
            root = auth::list_drive_files(&session, DriveLocation::Folder("root".into()), String::new(), Some(cursor))
                .await.expect("Read the next page of My Drive root");
            pages += 1;
        }
        if let Some(folder) = root.folders.first() {
            let child = auth::list_drive_files(&session, DriveLocation::Folder(folder.id.clone()), String::new(), None)
                .await.expect("Read a folder from My Drive");
            println!("My Drive child OK; folders={}; files={}", child.folders.len(), child.files.len());
        } else {
            println!("No My Drive folders found in {pages} sampled pages.");
        }
        let shared = auth::list_drive_files(&session, DriveLocation::Shared, String::new(), None)
            .await.expect("Read Shared with me");
        println!("Shared with me OK; folders={}; files={}", shared.folders.len(), shared.files.len());
        return;
    }
    if matches!(std::env::args().nth(1).as_deref(), Some("--submission-drive-picker" | "--mark-as-done")) {
        let courses = session.snapshot.courses.iter().filter(|course| course.shows_on_home())
            .map(|course| course.id).collect::<Vec<_>>();
        let total = courses.len();
        let updates: Vec<_> = auth::assignment_updates(std::sync::Arc::new(session), courses).collect().await;
        let editable: Vec<_> = updates.iter().filter_map(|update| update.contents.as_ref().map(|contents| (update, contents)))
            .flat_map(|(update, contents)| contents.items.iter().filter_map(|item| {
                (item.kind == classroom_rust::desktop_data::CourseworkKind::Assignment).then_some(item)?;
                update.submissions.as_ref()?.iter().find(|(id, _)| id == &item.id).map(|(_, submission)| submission)
            })).filter(|submission| auth::SubmissionAction::for_submission(submission) == Some(auth::SubmissionAction::TurnIn))
            .collect();
        println!("Classes sampled={}/{total}; editable own assignment submissions={}; editable submissions with no attachments={}",
            updates.iter().filter(|update| update.contents.is_some()).count(), editable.len(),
            editable.iter().filter(|submission| submission.attachments.is_empty()).count());
        return;
    }
    if matches!(std::env::args().nth(1).as_deref(), Some("--stream-links" | "--stream-attachments")) {
        use classroom_rust::desktop_data::{AttachmentKind, CourseworkKind};
        let courses: Vec<_> = session.snapshot.courses.iter().filter(|course| course.shows_on_home())
            .map(|course| course.id).collect();
        let results: Vec<_> = iced::futures::stream::iter(courses).map(|course| {
            let session = &session;
            async move { auth::load_coursework(session, course).await }
        }).buffer_unordered(3).collect().await;
        let posts = results.iter().filter_map(|result| result.as_ref().ok()).flat_map(|contents| &contents.items)
            .filter(|item| item.kind == CourseworkKind::Announcement);
        let (count, links, drive) = posts.fold((0, 0, 0), |(count, links, drive), item| {
            let links_on_post = item.attachments.iter().filter(|attachment| attachment.kind == AttachmentKind::Link
                && attachment.url.is_some()).count();
            let files_on_post = item.attachments.iter().filter(|attachment| attachment.kind == AttachmentKind::DriveFile
                && attachment.url.is_some()).count();
            (count + 1, links + links_on_post, drive + files_on_post)
        });
        println!("Read {} visible classes; announcements={count}; HTTPS link attachments={links}; Drive attachments={drive}",
            results.iter().filter(|result| result.is_ok()).count());
        assert!(results.iter().all(Result::is_ok));
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--class-order") {
        let classes: Vec<_> = session.snapshot.courses.iter().filter(|course| course.shows_on_home()).collect();
        let keys: Vec<_> = classes.iter().filter_map(|course| course.reordering_sort_key.as_deref()).collect();
        let sizes: std::collections::BTreeSet<_> = keys.iter().map(|key| key.len()).collect();
        println!("Visible classes={}; with sort keys={}; sort-key lengths={sizes:?}; returned keys ascending={}; descending={}",
            classes.len(), keys.len(), keys.is_sorted(), keys.is_sorted_by(|a, b| a >= b));
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--class-visibility") {
        let active: Vec<_> = session.snapshot.courses.iter().filter(|course| course.state == CourseState::Active).collect();
        let states = auth::class_visibility_for(&session, active.iter().map(|course| course.id).collect())
            .await.expect("Read the signed-in student's class visibility settings");
        println!("Active classes={}; joined students={}; explicit visibility={}; hidden={}; course list explicit={}; agreements={}",
            active.len(), states.len(), states.values().filter(|state| state.is_some()).count(),
            states.values().filter(|&&state| state == Some(true)).count(),
            active.iter().filter(|course| course.hidden.is_some()).count(),
            active.iter().filter(|course| states.get(&course.id) == Some(&course.hidden)).count());
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--class-comment-policy") {
        let courses = session.snapshot.courses.iter().filter(|course| course.state == CourseState::Active)
            .map(|course| course.id).collect();
        let access = auth::load_stream_access(&session, courses).await.expect("Read enrolled class permissions");
        for policy in [auth::StreamAccess::PostAndComment, auth::StreamAccess::CommentOnly, auth::StreamAccess::ReadOnly] {
            let Some((&course, _)) = access.iter().find(|(_, value)| **value == policy) else { continue };
            let (_, actual) = auth::load_stream_permission(&session, course).await.expect("Read individual class permission");
            assert_eq!(policy, actual);
            let contents = auth::load_coursework(&session, course).await.expect("Read class items");
            let thread_read = if let Some(id) = contents.items.first().and_then(|item| item.id.parse().ok()) {
                auth::load_comments(&session, course, id).await.expect("Read comments for the existing item");
                true
            } else {
                false
            };
            println!("{actual:?}: composer={}; existing thread read={thread_read}", actual.can_comment());
        }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--stream-post-lookup") {
        for course in session.snapshot.courses.iter().filter(|course| course.state == CourseState::Active) {
            let contents = auth::load_coursework(&session, course.id).await.expect("Read class Stream");
            let Some(post) = contents.items.iter().find(|item| {
                item.kind == classroom_rust::desktop_data::CourseworkKind::Announcement
            }) else { continue };
            let id = post.id.parse().expect("Numeric Stream item");
            let (author, active) = auth::check_stream_post_lookup(&session, course.id, id).await
                .expect("Look up the selected announcement without changing it");
            println!("Stream lookup matched requested post; list author={}; lookup author={author}; active={active}",
                post.author_id.is_some());
            return;
        }
        println!("No existing announcement was available for lookup.");
        return;
    }
    if let Some(mode @ ("--stream-access" | "--stream-access-single")) = std::env::args().nth(1).as_deref() {
        use auth::StreamAccess::*;
        let courses: Vec<_> = session.snapshot.courses.iter().filter(|course| course.state == CourseState::Active)
            .take(if mode == "--stream-access-single" { 1 } else { usize::MAX })
            .map(|course| course.id).collect();
        let total = courses.len();
        match auth::load_stream_access(&session, courses).await {
            Ok(access) => println!("Stream access={}/{total}; can post={}; comment only={}; read only={}",
                access.len(), access.values().filter(|&&value| value == PostAndComment).count(),
                access.values().filter(|&&value| value == CommentOnly).count(),
                access.values().filter(|&&value| value == ReadOnly).count()),
            Err(error) => { println!("Stream access read failed: {error}"); std::process::exit(1); }
        }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--email-notifications") {
        match auth::load_email_notifications(&session).await {
            Ok(settings) => println!("Email notifications present={}; categories={}; groups={}; groups with value={}",
                settings.enabled.is_some(), settings.categories.len(),
                settings.categories.iter().map(|category| category.groups.len()).sum::<usize>(),
                settings.categories.iter().flat_map(|category| &category.groups).filter(|group| group.enabled.is_some()).count()),
            Err(error) => { println!("Email notifications read failed: {error}"); std::process::exit(1); }
        }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--class-notifications") {
        let active: Vec<_> = session.snapshot.courses.iter().filter(|course| course.state == CourseState::Active)
            .map(|course| course.id).collect();
        let total = active.len();
        match auth::load_class_notifications(&session, active).await {
            Ok(rows) => println!("Class notification settings={}/{total}; muted={}",
                rows.len(), rows.values().filter(|&&muted| muted).count()),
            Err(error) => { println!("Class notifications read failed: {error}"); std::process::exit(1); }
        }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--class-links") {
        let courses: Vec<_> = session.snapshot.courses.iter().filter(|course| course.state == CourseState::Active)
            .map(|course| course.id).collect();
        let expected = courses.len();
        let meet = session.snapshot.courses.iter().filter(|course| course.meet_url.is_some()).count();
        match auth::load_class_folders(&session, courses).await {
            Ok(folders) => println!("Active classes={expected}; own Drive folder links={}; visible Meet links={meet}", folders.len()),
            Err(error) => { println!("Class links check failed: {error}"); std::process::exit(1); }
        }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--gradebook-open-check") {
        let Some(course) = session.snapshot.courses.iter().find(|course| course.state == CourseState::Active) else {
            println!("No active class available for the Your work check.");
            return;
        };
        match auth::load_course_grade(&session, course.id).await {
            Ok(grade) => println!("Your work grade check succeeded; published_total={}", grade.is_some()),
            Err(error) => { println!("Your work grade check failed: {error}"); std::process::exit(1); }
        }
        return;
    }
    if let Some(mode @ ("--gradebook" | "--gradebook-archived")) = std::env::args().nth(1).as_deref() {
        let state = if mode == "--gradebook-archived" { CourseState::Archived } else { CourseState::Active };
        let courses: Vec<_> = session.snapshot.courses.iter().filter(|course| course.state == state)
            .map(|course| course.id).collect();
        match auth::gradebook_coverage(&session, &courses).await {
            Ok((read, visibility, published)) => println!("Gradebook settings={read}/{} visibility={visibility:?} published_totals={published}", courses.len()),
            Err(error) => { println!("Gradebook read failed: {error}"); std::process::exit(1); }
        }
        return;
    }
    if let Some(mode @ ("--rubrics" | "--rubrics-archived")) = std::env::args().nth(1).as_deref() {
        use classroom_rust::desktop_data::CourseworkKind;
        let session = std::sync::Arc::new(session);
        let (state, class_limit, assignment_limit) = if mode == "--rubrics-archived" {
            (CourseState::Archived, 5, 20)
        } else {
            (CourseState::Active, usize::MAX, usize::MAX)
        };
        let courses = session.snapshot.courses.iter().filter(|course| course.state == state)
            .take(class_limit).map(|course| course.id).collect();
        let overview: Vec<_> = auth::assignment_updates(session.clone(), courses).collect().await;
        let targets: Vec<_> = overview.iter().flat_map(|entry| entry.contents.iter())
            .flat_map(|contents| &contents.items)
            .filter(|item| item.kind == CourseworkKind::Assignment)
            .filter_map(|item| Some((item.course_id, item.id.parse::<i64>().ok()?))).take(assignment_limit).collect();
        let score_count: usize = overview.iter().flat_map(|entry| entry.submissions.iter().flatten())
            .map(|(_, submission)| submission.rubric_scores.len()).sum();
        let reads: Vec<_> = iced::futures::stream::iter(targets).map(|(course, item)| {
            let session = session.clone();
            async move { auth::load_rubric(&session, course, item).await }
        }).buffer_unordered(3).collect().await;
        let rubrics: Vec<_> = reads.iter().filter_map(|result| result.as_ref().ok()?.as_ref()).collect();
        let failures = reads.iter().filter(|result| result.is_err()).count();
        println!("Rubric requests={} failures={} found={} criteria={} released_scores={score_count}",
            reads.len(), failures, rubrics.len(), rubrics.iter().map(|rubric| rubric.criteria.len()).sum::<usize>());
        if let Some(Err(error)) = reads.iter().find(|result| result.is_err()) {
            println!("Rubric read failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--questions") {
        use classroom_rust::desktop_data::{CourseworkKind, QuestionAnswer, QuestionDetails, QuestionSharing, SubmissionStatus};
        let ids = session.snapshot.courses.iter().filter(|course| course.state == CourseState::Active)
            .map(|course| course.id).collect::<Vec<_>>();
        let overview: Vec<_> = auth::assignment_updates(std::sync::Arc::new(session), ids.clone()).collect().await;
        let mut counts = [0; 5];
        let mut classes = 0;
        let mut matching_choice_answers = 0;
        let mut editable_records = 0;
        let mut first_short_answers = 0;
        let mut first_multiple_choice_answers = 0;
        let mut allow_edit_after_turn_in = 0;
        let mut submitted_answers_open_for_editing = 0;
        let mut sharing = [0; 3];
        let mut shared_multiple_choice_answered = 0;
        for entry in overview {
            let Some(contents) = entry.contents else { continue };
            classes += 1;
            for item in contents.items.iter().filter(|item| item.kind == CourseworkKind::Question) {
                allow_edit_after_turn_in += usize::from(item.allow_edit_after_turn_in);
                sharing[match item.question_sharing {
                    Some(QuestionSharing::Private) => 0,
                    Some(QuestionSharing::Course) => 1,
                    None => 2,
                }] += 1;
                match &item.question {
                    Some(QuestionDetails::ShortAnswer) => counts[0] += 1,
                    Some(QuestionDetails::MultipleChoice(options)) => {
                        counts[1] += 1;
                        counts[3] += options.len();
                    }
                    None => counts[2] += 1,
                }
                if let Some((_, submission)) = entry.submissions.iter().flatten().find(|(id, _)| *id == item.id) {
                    counts[4] += usize::from(submission.question_answer.is_some());
                    if let (Some(QuestionDetails::MultipleChoice(choices)), Some(QuestionAnswer::MultipleChoice(answer)))
                        = (&item.question, &submission.question_answer)
                        && choices.iter().filter(|choice| *choice == answer).count() == 1
                    {
                        matching_choice_answers += 1;
                        if item.question_sharing == Some(QuestionSharing::Course)
                            && matches!(submission.status, SubmissionStatus::TurnedIn | SubmissionStatus::TurnedInLate
                                | SubmissionStatus::Returned | SubmissionStatus::ReturnedLate)
                        {
                            shared_multiple_choice_answered += 1;
                        }
                    }
                    if auth::SubmissionAction::for_submission(submission) == Some(auth::SubmissionAction::TurnIn) {
                        editable_records += 1;
                    }
                    let mut question = item.clone();
                    question.submission = Some(submission.clone());
                    submitted_answers_open_for_editing += usize::from(auth::editable_short_answer(&question).is_some());
                    if auth::can_turn_in_question(&question) {
                        match question.question {
                            Some(QuestionDetails::ShortAnswer) => first_short_answers += 1,
                            Some(QuestionDetails::MultipleChoice(_)) => first_multiple_choice_answers += 1,
                            None => {}
                        }
                    }
                }
            }
        }
        println!("Question coverage classes={classes}/{} short={} multiple_choice={} unidentified={} options={} own_answers={} matching_choice_answers={matching_choice_answers} editable_records={editable_records} first_short_answers={first_short_answers} first_multiple_choice_answers={first_multiple_choice_answers} allow_edit_after_turn_in={allow_edit_after_turn_in} submitted_answers_open_for_editing={submitted_answers_open_for_editing}",
            ids.len(), counts[0], counts[1], counts[2], counts[3], counts[4]);
        if classes != ids.len() { std::process::exit(1); }
        println!("Question sharing private={} course={} omitted={} shared_multiple_choice_answered={shared_multiple_choice_answered}",
            sharing[0], sharing[1], sharing[2]);
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--classwork-layout") {
        let mut snapshot = session.snapshot.clone();
        let ids = snapshot.courses.iter().filter(|course| course.state == CourseState::Active)
            .map(|course| course.id).collect::<Vec<_>>();
        let reads: Vec<_> = iced::futures::stream::iter(ids.iter().copied()).map(|id| {
            let session = &session;
            async move { (id, auth::load_coursework(session, id).await) }
        }).buffer_unordered(3).collect().await;
        let mut classes = 0;
        for (id, result) in reads {
            if let Ok(contents) = result {
                classes += 1;
                snapshot.coursework.insert(id, contents.items);
                if let Some(topics) = contents.topics {
                    snapshot.topics.insert(id, topics);
                }
            }
        }
        let counts = ids.iter().flat_map(|id| snapshot.classwork_sections(*id, None))
            .fold((0, 0, 0, 0), |(items, named, untagged, unresolved), section| {
                use classroom_rust::desktop_data::ClassworkGroupId::*;
                let items = items + section.items.len();
                match section.id {
                    Topic(_) => (items, named + 1, untagged, unresolved),
                    NoTopic => (items, named, untagged + 1, unresolved),
                    UnresolvedTopic => (items, named, untagged, unresolved + 1),
                }
            });
        println!("Classwork classes={classes}/{} items={} named_topics={} no_topic_groups={} unresolved_topics={}",
            ids.len(), counts.0, counts.1, counts.2, counts.3);
        let mut total_keys = 0;
        let mut all_keyed = 0;
        let mut mixed_keys = 0;
        let mut ascending = 0;
        let mut descending = 0;
        for section in ids.iter().flat_map(|id| snapshot.classwork_sections(*id, None)) {
            let keys: Vec<_> = section.items.iter().filter_map(|item| item.classwork_sort_key.as_deref()).collect();
            total_keys += keys.len();
            if section.items.len() >= 2 && keys.len() == section.items.len() {
                all_keyed += 1;
                ascending += usize::from(keys.windows(2).all(|pair| pair[0] < pair[1]));
                descending += usize::from(keys.windows(2).all(|pair| pair[0] > pair[1]));
            } else if !keys.is_empty() && keys.len() < section.items.len() {
                mixed_keys += 1;
            }
        }
        println!("Classwork sort keys={total_keys}; fully keyed groups with >=2 posts={all_keyed}; mixed groups={mixed_keys}; display ascending={ascending}; descending={descending}");
        let topic_ids = snapshot.topics.values().flatten().map(|topic| topic.id.as_str()).collect::<Vec<_>>();
        let numerical = topic_ids.iter().filter(|id| id.parse::<i64>().is_ok_and(|id| id > 0)).count();
        println!("Published topics with positive numeric IDs={numerical}/{}", topic_ids.len());
        if classes != ids.len() {
            std::process::exit(1);
        }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--identity") {
        match auth::check_current_user(&session).await {
            Ok(()) => println!("Scoped current-user lookup returned one identity."),
            Err(error) => { println!("Current-user lookup failed: {error}"); std::process::exit(1); }
        }
        return;
    }
    let courses = &session.snapshot.courses;
    println!("courses={} active={} archived={} themed={}", courses.len(),
        courses.iter().filter(|c| c.state == CourseState::Active).count(),
        courses.iter().filter(|c| c.state == CourseState::Archived).count(),
        courses.iter().filter(|c| c.theme.is_some()).count());
    if std::env::args().nth(1).as_deref() == Some("--invitations") {
        match auth::load_invitations(&session).await {
            Ok(rows) => println!("Student invitations read OK; count={}", rows.len()),
            Err(error) => { println!("Student invitations read failed: {error}"); std::process::exit(1); }
        }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--feedback") {
        let selected = courses.iter().filter(|c| c.state == CourseState::Active).map(|c| c.id).collect();
        let batch: Vec<_> = auth::assignment_updates(std::sync::Arc::new(session.clone()), selected).collect().await;
        let mut sampled = 0;
        for (course, item) in batch.iter().flat_map(|entry| {
            entry.submissions.iter().flatten().filter_map(|(id, submission)| {
                submission.remote_id?;
                Some((entry.course_id, id.parse::<i64>().ok()?))
            })
        }).take(5) {
            sampled += 1;
            match auth::load_private_feedback(&session, course, item).await {
                Ok(comments) => println!("Private feedback read OK; count={}", comments.len()),
                Err(error) => println!("Private feedback failed: {error}"),
            }
        }
        println!("Own submissions sampled={sampled}");
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--overview") {
        let active = courses.iter().filter(|course| course.state == CourseState::Active)
            .map(|course| course.id).collect::<Vec<_>>();
        let expected = active.len();
        let start = std::time::Instant::now();
        let mut first = None;
        let overview: Vec<_> = auth::assignment_updates(std::sync::Arc::new(session), active)
            .inspect(|result| {
                if result.contents.is_some() {
                    first.get_or_insert_with(|| start.elapsed());
                }
            })
            .collect().await;
        if let Some(first) = first {
            println!("Overview timing first class={}ms full batch={}ms", first.as_millis(), start.elapsed().as_millis());
        }
        let loaded = overview.iter().filter(|result| result.contents.is_some()).count();
        let items = overview.iter().filter_map(|result| result.contents.as_ref())
            .flat_map(|contents| &contents.items)
            .filter(|item| matches!(item.kind, classroom_rust::desktop_data::CourseworkKind::Assignment
                | classroom_rust::desktop_data::CourseworkKind::Question)).count();
        let grades = overview.iter().filter_map(|result| result.submissions.as_ref())
            .flatten().filter(|(_, submission)| submission.grade.is_some()).count();
        let mut status_counts = std::collections::BTreeMap::new();
        for assignment in overview.iter().filter_map(|result| result.contents.as_ref())
            .flat_map(|contents| &contents.items)
            .filter(|item| matches!(item.kind, classroom_rust::desktop_data::CourseworkKind::Assignment
                | classroom_rust::desktop_data::CourseworkKind::Question))
        {
            let status = overview.iter().find(|entry| entry.course_id == assignment.course_id)
                .and_then(|entry| entry.submissions.as_ref())
                .and_then(|entries| entries.iter().find(|(id, _)| *id == assignment.id))
                .map(|(_, submission)| submission.status).unwrap_or_default();
            *status_counts.entry(status.label()).or_insert(0usize) += 1;
        }
        println!("Assignment status counts={status_counts:?}");
        let local = chrono::Local;
        let week = classroom_rust::desktop_data::monday_of(chrono::Local::now().date_naive());
        let due_dates: Vec<_> = overview.iter().filter_map(|result| result.contents.as_ref())
            .flat_map(|contents| &contents.items)
            .filter_map(|item| item.due_date(&local))
            .collect();
        let due_this_week = due_dates.iter().filter(|date| {
            classroom_rust::desktop_data::monday_of(**date) == week
        }).count();
        println!("Calendar decoded deadlines={}; due this week={due_this_week}", due_dates.len());
        let own_attachments = overview.iter().filter_map(|result| result.submissions.as_ref())
            .flatten().map(|(_, submission)| submission.attachments.len()).sum::<usize>();
        let attachment_ids = overview.iter().filter_map(|result| result.submissions.as_ref())
            .flatten().flat_map(|(_, submission)| &submission.attachments)
            .filter(|material| material.remote_id.is_some()).count();
        println!("Overview loaded={loaded}/{expected}; assignments and questions={items}; released grades={grades}; own attachments={own_attachments}; with IDs={attachment_ids}");
        if loaded != expected { std::process::exit(1); }
        return;
    }
    let artwork_only = std::env::args().nth(1).as_deref() == Some("--artwork");
    let artwork = courses.iter().filter(|c| c.state == CourseState::Active)
        .filter_map(|course| course.theme.as_ref()?.artwork_url.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let limit = if artwork_only { artwork.len() } else { 3 };
    let artwork: Vec<_> = artwork.into_iter().take(limit).collect();
    let requested = artwork.len();
    let decoded = auth::load_artworks(artwork).await.len();
    println!("Artwork decoded={decoded}/{requested}");
    if artwork_only {
        if decoded != requested || decoded == 0 {
            std::process::exit(1);
        }
        return;
    }
    for course in courses.iter().filter(|c| c.state == CourseState::Active).take(5) {
    match auth::load_coursework(&session, course.id).await {
        Ok(contents) => {
            println!("Classwork OK; items={} topics={}", contents.items.len(),
                contents.topics.as_ref().map_or(0, Vec::len));
            println!("Assignments with decoded dates={}; attachments={}",
                contents.items.iter().filter(|item| item.due_at_millis.is_some()).count(),
                contents.items.iter().map(|item| item.attachments.len()).sum::<usize>());
            println!("Openable HTTPS links={}; items with maximum points={}",
                contents.items.iter().flat_map(|item| &item.attachments)
                    .filter(|attachment| attachment.url.is_some()).count(),
                contents.items.iter().filter(|item| item.maximum_points.is_some()).count());
            if let Some(id) = contents.items.first().and_then(|item| item.id.parse().ok()) {
                match auth::load_comments(&session, course.id, id).await {
                    Ok(comments) => println!("Comments OK; count={}", comments.len()),
                    Err(error) => println!("Comments failed: {error}"),
                }
            }
        },
        Err(error) => println!("Classwork failed: {error}"),
    }
    match auth::load_submissions(&session, course.id).await {
        Ok(items) => println!("Own submissions OK; items={} released_grades={}",
            items.len(), items.iter().filter(|(_, item)| item.grade.is_some()).count()),
        Err(error) => println!("Own submissions failed: {error}"),
    }
    match auth::load_roster(&session, course.id).await {
        Ok(items) => println!("People OK; members={}", items.len()),
        Err(error) => println!("People failed: {error}"),
    }
    }
}
#[cfg(not(feature = "desktop"))]
fn main() { eprintln!("Requires desktop feature."); }
