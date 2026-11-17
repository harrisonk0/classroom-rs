#![cfg(feature = "desktop")]

use classroom_rust::desktop_data::{
    Course, Snapshot, SnapshotSource, clear_cache, load_cache, save_cache,
};
use ring::rand::{SecureRandom, SystemRandom};
use std::{fs, path::PathBuf};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let mut random = [0; 8];
        SystemRandom::new().fill(&mut random).unwrap();
        let path = std::env::temp_dir().join(format!("classroom-cache-{:016x}", u64::from_ne_bytes(random)));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn sqlite_cache_is_account_scoped_and_accepts_only_live_snapshots() {
    let directory = TestDirectory::new();
    let database = directory.0.join("classroom.sqlite");
    let account = "student@example.invalid";
    let other = "other@example.invalid";
    assert!(load_cache(&database, account).unwrap().is_none());
    assert!(!database.exists());
    assert!(save_cache(&database, account, &Snapshot::demo()).is_err());
    let live = Snapshot::live(vec![Course { id: 42, title: "Generated fixture".into(), ..Default::default() }]);
    save_cache(&database, account, &live).unwrap();
    let cached = load_cache(&database, account).unwrap().unwrap();
    assert_eq!(cached.source, SnapshotSource::Cache);
    assert_eq!(cached.courses, live.courses);
    assert!(load_cache(&database, other).unwrap().is_none());
    assert!(save_cache(&database, account, &Snapshot::demo()).is_err());
    assert_eq!(load_cache(&database, account).unwrap().unwrap(), cached);
    clear_cache(&database, account).unwrap();
    assert!(load_cache(&database, account).unwrap().is_none());
}

#[test]
fn offline_copy_excludes_submission_attachments_and_link_share_keys() {
    let directory = TestDirectory::new();
    let database = directory.0.join("classroom.sqlite");
    let account = "student@example.invalid";
    let mut live = Snapshot::demo();
    live.source = SnapshotSource::Live;
    live.courses[0].meet_url = Some("https://meet.google.com/lookup/synthetic-meet-key".into());
    let item = live.coursework.values_mut().flatten()
        .find(|item| item.submission.is_some()).unwrap();
    let course_id = item.course_id;
    let item_id = item.id.clone();
    let attachment = classroom_rust::desktop_data::Attachment {
        remote_id: None,
        title: "Test attachment".into(),
        url: Some("https://example.invalid/file?key=synthetic-secret".into()),
        kind: classroom_rust::desktop_data::AttachmentKind::Link,
        drive_sharing: None,
        drive_reference: None,
        drive_parent_reference: None,
    };
    item.submission.as_mut().unwrap().attachments.push(attachment.clone());
    item.attachments.push(attachment);
    save_cache(&database, account, &live).unwrap();
    let cached = load_cache(&database, account).unwrap().unwrap();
    let item = cached.coursework_for(course_id).unwrap().iter()
        .find(|item| item.id == item_id).unwrap();
    assert!(cached.courses[0].meet_url.is_none());
    assert!(!fs::read(&database).unwrap().windows(b"synthetic-meet-key".len())
        .any(|window| window == b"synthetic-meet-key"));
    assert!(item.submission.is_none());
    assert!(item.attachments.last().unwrap().url.is_none());
    assert!(!fs::read(&database).unwrap().windows(b"synthetic-secret".len())
        .any(|window| window == b"synthetic-secret"));
}
