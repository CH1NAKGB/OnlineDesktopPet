//! Local cloud state only; networking and persistence of responses belong to the parent.
//! Persist Authenticated/LoggedOut and every History/Message even without a window.
//! Only a completed, contiguous Sync page may advance save_cursor. Live messages never do.

use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use shared_protocol::cloud::CloudMessage;

// Intentionally no Debug: tokens must never appear in diagnostic output.
#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    pub user_id: String,
    pub username: String,
    pub token: String,
    #[serde(default)]
    pub server_url: String,
}

static STORE_LOCK: Mutex<()> = Mutex::new(());
static HISTORY_INDEX: OnceLock<Mutex<HashMap<String, HistoryIndex>>> = OnceLock::new();
pub(crate) const MAX_HISTORY_ITEMS: usize = 100;

#[derive(Default)]
struct HistoryIndex {
    sequences: HashSet<u64>,
    clients: HashSet<(String, String)>,
    recent: Vec<CloudMessage>,
}

impl HistoryIndex {
    fn contains(&self, item: &CloudMessage) -> bool {
        self.sequences.contains(&item.seq)
            || (!item.client_id.is_empty()
                && self
                    .clients
                    .contains(&(item.sender_id.clone(), item.client_id.clone())))
    }

    fn insert(&mut self, item: &CloudMessage) {
        if self.contains(item) {
            return;
        }
        self.sequences.insert(item.seq);
        if !item.client_id.is_empty() {
            self.clients
                .insert((item.sender_id.clone(), item.client_id.clone()));
        }
        merge_message(&item.sender_id, &mut self.recent, item);
    }

    fn load(user_id: &str, path: &Path) -> io::Result<Self> {
        let mut index = Self::default();
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(index),
            Err(error) => return Err(error),
        };
        for line in BufReader::new(file).lines() {
            if let Ok(item) = serde_json::from_str::<CloudMessage>(&line?) {
                if belongs_to(user_id, &item) {
                    index.insert(&item);
                }
            }
        }
        Ok(index)
    }
}

pub fn load_session() -> Option<Session> {
    let session: Session =
        serde_json::from_slice(&fs::read(root().ok()?.join("session.json")).ok()?).ok()?;
    (!session.user_id.is_empty() && !session.token.is_empty() && !session.server_url.is_empty())
        .then_some(session)
}

pub fn save_session(session: &Session) -> io::Result<()> {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let bytes =
        serde_json::to_vec(session).map_err(|_| io::Error::other("Cannot encode session"))?;
    atomic_write(&root()?.join("session.json"), &bytes)
}

pub fn clear_session() -> io::Result<()> {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    match fs::remove_file(root()?.join("session.json")) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

pub fn device_id() -> String {
    static DEVICE: OnceLock<String> = OnceLock::new();
    DEVICE
        .get_or_init(|| {
            let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let path = root().ok().map(|root| root.join("device-id"));
            if let Some(value) = path.as_ref().and_then(|path| fs::read_to_string(path).ok()) {
                if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return value;
                }
            }
            let id = new_message_id();
            if let Some(path) = path {
                let _ = atomic_write(&path, id.as_bytes());
            }
            id
        })
        .clone()
}

pub fn new_message_id() -> String {
    let entropy: [u8; 32] = rand::random();
    format!("{:x}", Sha256::digest(entropy))
}

/// Latest 100 messages, in sequence order. Older history remains on disk.
pub fn load_cloud_history(session: &Session) -> Vec<CloudMessage> {
    let Ok(path) = account_path(session, "history.jsonl") else {
        return Vec::new();
    };
    let mut indexes = HISTORY_INDEX
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    history_index(session, &path, &mut indexes)
        .map(|index| index.recent.clone())
        .unwrap_or_default()
}

/// Returns true only for a newly persisted message, false for a known duplicate.
pub fn append_cloud_history(session: &Session, item: &CloudMessage) -> io::Result<bool> {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if !belongs_to(&session.user_id, item) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Message belongs to another account",
        ));
    }
    let path = account_path(session, "history.jsonl")?;
    let mut indexes = HISTORY_INDEX
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let index = history_index(session, &path, &mut indexes)?;
    append_indexed(&path, index, item)
}

fn history_index<'a>(
    session: &Session,
    path: &Path,
    indexes: &'a mut HashMap<String, HistoryIndex>,
) -> io::Result<&'a mut HistoryIndex> {
    let key = account_namespace(session)?;
    if !indexes.contains_key(&key) {
        indexes.insert(key.clone(), HistoryIndex::load(&session.user_id, path)?);
    }
    Ok(indexes.get_mut(&key).unwrap())
}

fn append_indexed(path: &Path, index: &mut HistoryIndex, item: &CloudMessage) -> io::Result<bool> {
    if index.contains(item) {
        return Ok(false);
    }
    private_root(path.parent().unwrap())?;
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    private_options(&mut options);
    let mut file = options.open(&path)?;
    private_file(&file)?;
    let mut bytes =
        serde_json::to_vec(item).map_err(|_| io::Error::other("Cannot encode message"))?;
    // A failed previous append may have left a partial final line.
    bytes.insert(0, b'\n');
    bytes.push(b'\n');
    file.write_all(&bytes)?;
    file.sync_data()?;
    index.insert(item);
    Ok(true)
}

pub fn cursor(session: &Session) -> u64 {
    account_path(session, "cursor")
        .ok()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

/// Call only after all items in a Sync page have been durably appended.
pub fn save_cursor(session: &Session, cursor: u64) -> io::Result<()> {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    atomic_write(
        &account_path(session, "cursor")?,
        cursor.to_string().as_bytes(),
    )
}

pub(crate) fn belongs_to(user_id: &str, item: &CloudMessage) -> bool {
    !user_id.is_empty() && (item.sender_id == user_id || item.recipient_id == user_id)
}

fn same_message(a: &CloudMessage, b: &CloudMessage) -> bool {
    a.seq == b.seq
        || (!a.client_id.is_empty() && a.client_id == b.client_id && a.sender_id == b.sender_id)
}

/// Merge only the active account; delayed live events may precede older Sync pages.
pub(crate) fn merge_message(user_id: &str, items: &mut Vec<CloudMessage>, item: &CloudMessage) {
    if belongs_to(user_id, item) && !items.iter().any(|old| same_message(old, item)) {
        let index = items.partition_point(|old| old.seq < item.seq);
        items.insert(index, item.clone());
        if items.len() > MAX_HISTORY_ITEMS {
            items.remove(0);
        }
    }
}

fn account_namespace(session: &Session) -> io::Result<String> {
    if session.server_url.trim().is_empty() || session.user_id.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Cloud storage requires a server and an account",
        ));
    }
    let mut digest = Sha256::new();
    digest.update((session.server_url.len() as u64).to_le_bytes());
    digest.update(session.server_url.as_bytes());
    digest.update((session.user_id.len() as u64).to_le_bytes());
    digest.update(session.user_id.as_bytes());
    Ok(format!("{:x}", digest.finalize()))
}

fn account_path(session: &Session, suffix: &str) -> io::Result<PathBuf> {
    // Never fall back to the old user-ID-only files: their server is unknown.
    let namespace = account_namespace(session)?;
    Ok(root()?.join(format!("{namespace}-{suffix}")))
}

fn root() -> io::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|p| p.join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|p| p.join(".local/share"))
        });
    base.map(|p| p.join("ChickLink/cloud"))
        .ok_or_else(|| io::Error::other("Cannot locate cloud data directory"))
}

fn private_root(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(path)?;
    Ok(())
}

fn private_options(options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let _ = options;
}

fn private_file(file: &File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    let _ = file;
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Invalid cloud path"))?;
    private_root(parent)?;
    let temporary = parent.join(format!(".{}.tmp", new_message_id()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        private_options(&mut options);
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_protocol::cloud::CloudBody;

    fn message(seq: u64, sender: &str, recipient: &str) -> CloudMessage {
        CloudMessage {
            seq,
            client_id: format!("id-{seq}"),
            sender_id: sender.into(),
            recipient_id: recipient.into(),
            body: CloudBody::Text {
                text: "hello".into(),
            },
            unix_ms: seq,
        }
    }

    fn session(server: &str, user_id: &str) -> Session {
        Session {
            user_id: user_id.into(),
            username: "test".into(),
            token: "test".into(),
            server_url: server.into(),
        }
    }

    #[test]
    fn merges_offline_gaps_and_deduplicates_without_cross_account_leaks() {
        let mut items = Vec::new();
        for item in [
            message(9, "a", "b"),
            message(2, "b", "a"),
            message(9, "a", "b"),
            message(3, "c", "d"),
        ] {
            merge_message("a", &mut items, &item);
        }
        assert_eq!(
            items.iter().map(|item| item.seq).collect::<Vec<_>>(),
            vec![2, 9]
        );
        let mut retry = message(10, "a", "b");
        retry.client_id = "id-9".into();
        merge_message("a", &mut items, &retry);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn account_filenames_are_hashed_and_ids_are_unique() {
        let path = account_path(&session("server", "../../secret/account"), "cursor").unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(name.len(), 64 + "-cursor".len());
        assert!(!name.contains("secret"));
        assert_ne!(new_message_id(), new_message_id());
    }

    #[test]
    fn namespace_is_unambiguous_and_never_reuses_legacy_paths() {
        let alice = session("server-a", "alice");
        let other_server = session("server-b", "alice");
        let other_account = session("server-a", "bob");
        for suffix in ["history.jsonl", "cursor"] {
            let path = account_path(&alice, suffix).unwrap();
            assert_ne!(path, account_path(&other_server, suffix).unwrap());
            assert_ne!(path, account_path(&other_account, suffix).unwrap());
            let legacy_name = format!("{:x}-{suffix}", Sha256::digest(b"alice"));
            assert_ne!(path.file_name().unwrap().to_str().unwrap(), legacy_name);
        }
        assert_ne!(
            account_namespace(&session("ab", "c")).unwrap(),
            account_namespace(&session("a", "bc")).unwrap()
        );
    }

    #[test]
    fn missing_server_fails_closed_for_history_cursor_and_cache() {
        let missing = session("", "alice");
        assert!(load_cloud_history(&missing).is_empty());
        assert_eq!(cursor(&missing), 0);
        assert!(append_cloud_history(&missing, &message(1, "alice", "bob")).is_err());
        assert!(save_cursor(&missing, 99).is_err());
        assert!(account_namespace(&missing).is_err());
        assert!(account_namespace(&session("server", "")).is_err());
    }

    #[test]
    fn server_scoped_cache_disk_and_append_result_remain_independent() {
        let directory =
            std::env::temp_dir().join(format!("chicklink-namespace-test-{}", new_message_id()));
        let first = session("server-a", "alice");
        let second = session("server-b", "alice");
        let path_for = |owner: &Session, suffix: &str| {
            directory.join(account_path(owner, suffix).unwrap().file_name().unwrap())
        };
        let first_path = path_for(&first, "history.jsonl");
        let second_path = path_for(&second, "history.jsonl");
        let first_message = message(7, "alice", "bob");
        let mut second_message = first_message.clone();
        second_message.body = CloudBody::Text {
            text: "different server".into(),
        };
        let mut indexes = HashMap::new();
        assert!(
            append_indexed(
                &first_path,
                history_index(&first, &first_path, &mut indexes).unwrap(),
                &first_message
            )
            .unwrap()
        );
        assert!(
            !append_indexed(
                &first_path,
                history_index(&first, &first_path, &mut indexes).unwrap(),
                &first_message
            )
            .unwrap()
        );
        assert!(
            history_index(&second, &second_path, &mut indexes)
                .unwrap()
                .recent
                .is_empty()
        );
        assert!(
            append_indexed(
                &second_path,
                history_index(&second, &second_path, &mut indexes).unwrap(),
                &second_message
            )
            .unwrap()
        );
        assert_eq!(indexes.len(), 2);
        assert_eq!(
            history_index(&first, &first_path, &mut indexes)
                .unwrap()
                .recent,
            vec![first_message.clone()]
        );
        indexes.clear();
        assert_eq!(
            history_index(&first, &first_path, &mut indexes)
                .unwrap()
                .recent,
            vec![first_message.clone()]
        );
        assert!(
            !append_indexed(
                &first_path,
                history_index(&first, &first_path, &mut indexes).unwrap(),
                &first_message
            )
            .unwrap()
        );
        assert_eq!(
            history_index(&second, &second_path, &mut indexes)
                .unwrap()
                .recent,
            vec![second_message]
        );
        let first_cursor = path_for(&first, "cursor");
        let second_cursor = path_for(&second, "cursor");
        atomic_write(&first_cursor, b"99").unwrap();
        assert!(!second_cursor.exists());
        atomic_write(&second_cursor, b"3").unwrap();
        assert_eq!(fs::read_to_string(first_cursor).unwrap(), "99");
        assert_eq!(fs::read_to_string(second_cursor).unwrap(), "3");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn legacy_session_defaults_to_no_trusted_server() {
        let session: Session =
            serde_json::from_str(r#"{"user_id":"alice","username":"Alice","token":"secret"}"#)
                .unwrap();
        assert!(session.server_url.is_empty());
        let session = Session {
            server_url: "wss://example.test/ws".into(),
            ..session
        };
        let encoded = serde_json::to_vec(&session).unwrap();
        let decoded: Session = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.server_url, "wss://example.test/ws");
    }

    #[test]
    fn bounded_history_keeps_latest_sequences_after_out_of_order_sync() {
        let mut items = Vec::new();
        for seq in (1..=250).rev() {
            merge_message("a", &mut items, &message(seq, "a", "b"));
        }
        assert_eq!(items.len(), MAX_HISTORY_ITEMS);
        assert_eq!(items.first().unwrap().seq, 151);
        assert_eq!(items.last().unwrap().seq, 250);
    }

    #[test]
    fn index_deduplicates_sequence_and_sender_scoped_client_ids() {
        let mut index = HistoryIndex::default();
        let original = message(10, "a", "b");
        index.insert(&original);
        assert!(index.contains(&original));
        let mut retry = message(11, "a", "b");
        retry.client_id = original.client_id.clone();
        assert!(index.contains(&retry));
        retry.sender_id = "b".into();
        assert!(!index.contains(&retry));
        assert!(!HistoryIndex::default().contains(&original));
    }

    #[cfg(unix)]
    #[test]
    fn private_store_permissions_and_atomic_replacement() {
        use std::os::unix::fs::PermissionsExt;
        let directory =
            std::env::temp_dir().join(format!("chicklink-cloud-test-{}", new_message_id()));
        let path = directory.join("session.json");
        atomic_write(&path, b"first").unwrap();
        atomic_write(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }
}
