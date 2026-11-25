use super::{Snapshot, SnapshotSource};
use std::{error::Error, fmt, fs, io, path::Path, time::Duration};
use ring::digest::{SHA256, digest};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};

#[derive(Debug)]
pub enum CacheError {
    MissingAccount,
    InvalidSnapshot,
    Io(io::Error),
    Sqlite(rusqlite::Error),
    Encoding(serde_json::Error),
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingAccount => f.write_str("Offline storage needs a specific account."),
            Self::InvalidSnapshot => f.write_str("Only a successful live refresh can update offline storage."),
            Self::Io(error) => write!(f, "Offline storage could not be opened: {error}"),
            Self::Sqlite(error) => write!(f, "Offline storage operation failed: {error}"),
            Self::Encoding(error) => write!(f, "Offline snapshot format error: {error}"),
        }
    }
}

impl Error for CacheError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            Self::Encoding(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for CacheError {
    fn from(error: io::Error) -> Self { Self::Io(error) }
}
impl From<rusqlite::Error> for CacheError {
    fn from(error: rusqlite::Error) -> Self { Self::Sqlite(error) }
}
impl From<serde_json::Error> for CacheError {
    fn from(error: serde_json::Error) -> Self { Self::Encoding(error) }
}

fn account_key(account_id: &str) -> Result<[u8; 32], CacheError> {
    let account = account_id.trim().to_ascii_lowercase();
    if account.is_empty() {
        return Err(CacheError::MissingAccount);
    }
    let mut key = [0; 32];
    key.copy_from_slice(digest(&SHA256, account.as_bytes()).as_ref());
    Ok(key)
}

fn existing_database(path: &Path, flags: OpenFlags) -> Result<Option<Connection>, CacheError> {
    match fs::metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let db = Connection::open_with_flags(path, flags)?;
    db.busy_timeout(Duration::from_secs(2))?;
    Ok(Some(db))
}

fn read_snapshot(db: &Connection, account: &[u8; 32]) -> Result<Option<Snapshot>, CacheError> {
    let payload: Option<Vec<u8>> = db.query_row(
        "SELECT payload FROM classroom_snapshots_v1 WHERE account_key = ?1",
        params![account.as_slice()],
        |row| row.get(0),
    ).optional()?;
    if let Some(payload) = payload {
        let mut snapshot: Snapshot = serde_json::from_slice(&payload)?;
        snapshot.source = SnapshotSource::Cache;
        snapshot.remove_private_cache_fields();
        return Ok(Some(snapshot));
    }
    Ok(None)
}

fn create_private_database_file(path: &Path) -> Result<(), CacheError> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Open a saved snapshot for this account. Never creates a cache.
pub fn load_cache(
    path: impl AsRef<Path>,
    account_id: &str,
) -> Result<Option<Snapshot>, CacheError> {
    let account = account_key(account_id)?;
    let Some(db) = existing_database(path.as_ref(), OpenFlags::SQLITE_OPEN_READ_ONLY)? else {
        return Ok(None);
    };
    read_snapshot(&db, &account)
}

/// Store a successful live refresh. Call only after the user opts into offline storage.
///
/// Older coursework and topics are retained for current classes if this refresh
/// did not fetch them. A fetched empty list replaces the old list for that class.
/// Grades, submissions, comments, rosters and course artwork URLs are kept in memory only.
pub fn save_cache(
    path: impl AsRef<Path>,
    account_id: &str,
    snapshot: &Snapshot,
) -> Result<(), CacheError> {
    if snapshot.source != SnapshotSource::Live {
        return Err(CacheError::InvalidSnapshot);
    }
    let account = account_key(account_id)?;
    let path = path.as_ref();
    create_private_database_file(path)?;
    let mut db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    db.busy_timeout(Duration::from_secs(2))?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS classroom_snapshots_v1 (
            account_key BLOB PRIMARY KEY NOT NULL CHECK(length(account_key) = 32),
            payload BLOB NOT NULL
        ) WITHOUT ROWID;",
    )?;

    let mut merged = snapshot.clone();
    let enrolled = snapshot.courses.iter().map(|course| course.id)
        .collect::<std::collections::BTreeSet<_>>();
    merged.coursework.retain(|course_id, _| enrolled.contains(course_id));
    merged.topics.retain(|course_id, _| enrolled.contains(course_id));
    if let Some(previous) = read_snapshot(&tx, &account)? {
        for (course_id, items) in previous.coursework {
            if enrolled.contains(&course_id) {
                merged.coursework.entry(course_id).or_insert(items);
            }
        }
        for (course_id, topics) in previous.topics {
            if enrolled.contains(&course_id) {
                merged.topics.entry(course_id).or_insert(topics);
            }
        }
    }
    merged.remove_private_cache_fields();
    let payload = serde_json::to_vec(&merged)?;
    tx.execute(
        "INSERT INTO classroom_snapshots_v1 (account_key, payload) VALUES (?1, ?2)
         ON CONFLICT(account_key) DO UPDATE SET payload = excluded.payload",
        params![account.as_slice(), payload],
    )?;
    tx.commit()?;
    Ok(())
}

/// Remove the selected account's persisted data without touching other accounts.
pub fn clear_cache(path: impl AsRef<Path>, account_id: &str) -> Result<(), CacheError> {
    let account = account_key(account_id)?;
    let Some(db) = existing_database(path.as_ref(), OpenFlags::SQLITE_OPEN_READ_WRITE)? else {
        return Ok(());
    };
    db.execute_batch("PRAGMA secure_delete = ON;")?;
    db.execute(
        "DELETE FROM classroom_snapshots_v1 WHERE account_key = ?1",
        params![account.as_slice()],
    )?;
    Ok(())
}
