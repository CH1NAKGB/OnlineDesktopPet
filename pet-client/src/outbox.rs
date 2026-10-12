//! Persistent pending messages. One Outbox instance owns each server/account queue.
//! The parent sends `next()`, acknowledges server-confirmed client IDs, and explicitly
//! retries blocked entries. No authentication credentials are stored here.

use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use shared_protocol::cloud::CloudBody;

const MAX_ENTRIES: usize = 32;
const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub client_id: String,
    pub recipient_id: String,
    pub body: CloudBody,
    pub blocked: bool,
}

pub struct Outbox {
    path: PathBuf,
    entries: Vec<Entry>,
}

impl Outbox {
    pub fn load(server_url: &str, user_id: &str) -> io::Result<Self> {
        if server_url.is_empty() || user_id.is_empty() {
            return Err(invalid_input("Outbox requires a server and an account"));
        }
        Self::load_path(root()?.join(filename(server_url, user_id)))
    }

    fn load_path(path: PathBuf) -> io::Result<Self> {
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Self {
                    path,
                    entries: Vec::new(),
                });
            }
            Err(error) => return Err(error),
        };
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES as u64 {
            return Err(invalid_data("Outbox file is not regular or exceeds 8 MiB"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(invalid_data("Outbox file exceeds 8 MiB"));
        }
        // Parser diagnostics can contain message content; return a fixed error instead.
        let entries: Vec<Entry> = serde_json::from_slice(&bytes)
            .map_err(|_| invalid_data("Outbox file is corrupt; it has not been modified"))?;
        if entries.len() > MAX_ENTRIES {
            return Err(invalid_data("Outbox contains more than 32 messages"));
        }
        let mut ids = HashSet::new();
        for entry in &entries {
            if entry.client_id.is_empty()
                || entry.recipient_id.is_empty()
                || !ids.insert(&entry.client_id)
            {
                return Err(invalid_data(
                    "Outbox contains invalid or duplicate message IDs",
                ));
            }
        }
        Ok(Self { path, entries })
    }

    /// An existing ID is accepted only with the same recipient and body; its blocked
    /// state is preserved. IDs are scoped to this pending queue, not past deliveries.
    pub fn enqueue(
        &mut self,
        id: impl Into<String>,
        recipient: impl Into<String>,
        body: CloudBody,
    ) -> io::Result<()> {
        let id = id.into();
        let recipient = recipient.into();
        if id.is_empty() || recipient.is_empty() {
            return Err(invalid_input("Message ID and recipient cannot be empty"));
        }
        if let Some(existing) = self.entries.iter().find(|entry| entry.client_id == id) {
            if existing.recipient_id != recipient || existing.body != body {
                return Err(invalid_input("Message ID already has different content"));
            }
            return self.persist(&self.entries);
        }
        if self.entries.len() >= MAX_ENTRIES {
            return Err(invalid_input("Outbox is full (32 messages)"));
        }
        let mut entries = self.entries.clone();
        entries.push(Entry {
            client_id: id,
            recipient_id: recipient,
            body,
            blocked: false,
        });
        self.commit(entries)
    }

    pub fn next(&self) -> Option<&Entry> {
        self.entries.iter().find(|entry| !entry.blocked)
    }

    /// Unknown IDs are an idempotent no-op, so repeated acknowledgements are safe.
    pub fn ack(&mut self, id: &str) -> io::Result<()> {
        let mut entries = self.entries.clone();
        entries.retain(|entry| entry.client_id != id);
        self.commit(entries)
    }

    /// Unknown IDs are an idempotent no-op. Blocking never changes queue order.
    pub fn block(&mut self, id: &str) -> io::Result<()> {
        let mut entries = self.entries.clone();
        if let Some(entry) = entries.iter_mut().find(|entry| entry.client_id == id) {
            entry.blocked = true;
        }
        self.commit(entries)
    }

    pub fn retry_all(&mut self) -> io::Result<()> {
        let mut entries = self.entries.clone();
        for entry in &mut entries {
            entry.blocked = false;
        }
        self.commit(entries)
    }

    pub fn clear(&mut self) -> io::Result<()> {
        self.commit(Vec::new())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn blocked_count(&self) -> usize {
        self.entries.iter().filter(|entry| entry.blocked).count()
    }

    fn commit(&mut self, entries: Vec<Entry>) -> io::Result<()> {
        self.persist(&entries)?;
        self.entries = entries;
        Ok(())
    }

    fn persist(&self, entries: &[Entry]) -> io::Result<()> {
        let mut writer = LimitedWriter(Vec::new());
        serde_json::to_writer(&mut writer, entries)
            .map_err(|_| invalid_input("Outbox cannot exceed an 8 MiB JSON file"))?;
        atomic_write(&self.path, &writer.0)
    }
}

// Bound serialized bytes too: escaped text can be much larger than its input.
struct LimitedWriter(Vec<u8>);

impl Write for LimitedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_FILE_BYTES.saturating_sub(self.0.len()) {
            return Err(invalid_input("Outbox file exceeds 8 MiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn filename(server_url: &str, user_id: &str) -> String {
    // Length prefixes prevent ambiguous pairs such as ("ab", "c") and ("a", "bc").
    let mut digest = Sha256::new();
    digest.update((server_url.len() as u64).to_le_bytes());
    digest.update(server_url.as_bytes());
    digest.update((user_id.len() as u64).to_le_bytes());
    digest.update(user_id.as_bytes());
    format!("{:x}.json", digest.finalize())
}

fn root() -> io::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|path| path.join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|path| path.join(".local/share"))
        });
    base.map(|path| path.join("ChickLink/cloud/outbox"))
        .ok_or_else(|| io::Error::other("Cannot locate outbox directory"))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid_input("Invalid outbox path"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(parent)?;

    let temporary = parent.join(format!(".outbox-{:032x}.tmp", rand::random::<u128>()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    let written = file.write_all(bytes).and_then(|_| file.sync_all());
    // Close before replacing, including on Windows; std::fs::rename replaces an
    // existing file using the platform implementation, as in cloud_client.
    drop(file);
    let result = written.and_then(|_| fs::rename(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn invalid_input(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "chicklink-outbox-test-{:032x}",
                rand::random::<u128>()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> PathBuf {
            self.0.join("queue.json")
        }
        fn load(&self) -> Outbox {
            Outbox::load_path(self.path()).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn text(value: &str) -> CloudBody {
        CloudBody::Text { text: value.into() }
    }

    #[test]
    fn restart_ack_and_clear_preserve_fifo() {
        let fixture = Fixture::new();
        let mut queue = fixture.load();
        assert_eq!(queue.len(), 0);
        assert!(queue.next().is_none());
        queue.enqueue("a", "bob", text("first")).unwrap();
        queue.enqueue("b", "bob", text("second")).unwrap();
        let mut queue = fixture.load();
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.next().unwrap().client_id, "a");
        queue.ack("a").unwrap();
        queue.ack("a").unwrap();
        assert_eq!(fixture.load().next().unwrap().client_id, "b");
        queue.clear().unwrap();
        assert_eq!(fixture.load().len(), 0);
    }

    #[test]
    fn same_id_is_idempotent_but_cannot_change_content_or_recipient() {
        let fixture = Fixture::new();
        let mut queue = fixture.load();
        queue.enqueue("a", "bob", text("original")).unwrap();
        queue.block("a").unwrap();
        queue.enqueue("a", "bob", text("original")).unwrap();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.blocked_count(), 1);
        let before = fs::read(fixture.path()).unwrap();
        assert!(queue.enqueue("a", "bob", text("different")).is_err());
        assert!(queue.enqueue("a", "carol", text("original")).is_err());
        assert_eq!(fs::read(fixture.path()).unwrap(), before);
        assert_eq!(fixture.load().len(), 1);
    }

    #[test]
    fn block_retry_survive_restart_and_do_not_reorder() {
        let fixture = Fixture::new();
        let mut queue = fixture.load();
        queue.enqueue("a", "bob", text("first")).unwrap();
        queue.enqueue("b", "bob", text("second")).unwrap();
        queue.block("a").unwrap();
        let mut queue = fixture.load();
        assert_eq!(queue.next().unwrap().client_id, "b");
        queue.block("b").unwrap();
        queue.block("unknown").unwrap();
        assert!(fixture.load().next().is_none());
        assert_eq!(fixture.load().blocked_count(), 2);
        queue.retry_all().unwrap();
        assert_eq!(fixture.load().blocked_count(), 0);
        assert_eq!(fixture.load().next().unwrap().client_id, "a");
    }

    #[test]
    fn capacity_rejection_never_evicts_existing_messages() {
        let fixture = Fixture::new();
        let mut queue = fixture.load();
        for id in 0..MAX_ENTRIES {
            queue
                .enqueue(id.to_string(), "bob", text("message"))
                .unwrap();
        }
        let before = fs::read(fixture.path()).unwrap();
        assert!(queue.enqueue("overflow", "bob", text("message")).is_err());
        queue.enqueue("0", "bob", text("message")).unwrap();
        assert_eq!(queue.len(), MAX_ENTRIES);
        assert_eq!(fs::read(fixture.path()).unwrap(), before);
        queue.ack("0").unwrap();
        queue.enqueue("new", "bob", text("new message")).unwrap();
        assert_eq!(queue.next().unwrap().client_id, "1");
        assert_eq!(queue.entries.last().unwrap().client_id, "new");
    }

    #[test]
    fn byte_limit_includes_json_escaping_and_preserves_old_file() {
        let fixture = Fixture::new();
        let mut queue = fixture.load();
        queue.enqueue("a", "bob", text("keep me")).unwrap();
        let before = fs::read(fixture.path()).unwrap();
        // Each control character expands to six JSON bytes.
        let oversized = text(&"\0".repeat(MAX_FILE_BYTES / 6 + 1));
        assert!(queue.enqueue("b", "bob", oversized).is_err());
        assert_eq!(queue.len(), 1);
        assert_eq!(fs::read(fixture.path()).unwrap(), before);
    }

    #[test]
    fn exact_file_size_limit_is_accepted() {
        let fixture = Fixture::new();
        let mut queue = fixture.load();
        let empty = Entry {
            client_id: "a".into(),
            recipient_id: "bob".into(),
            body: text(""),
            blocked: false,
        };
        let overhead = serde_json::to_vec(&[empty]).unwrap().len();
        queue
            .enqueue("a", "bob", text(&"x".repeat(MAX_FILE_BYTES - overhead)))
            .unwrap();
        assert_eq!(
            fs::metadata(fixture.path()).unwrap().len(),
            MAX_FILE_BYTES as u64
        );
        assert_eq!(fixture.load().len(), 1);
        assert!(queue.enqueue("b", "bob", text("one too many")).is_err());
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn namespace_is_server_and_account_scoped_and_unambiguous() {
        assert_eq!(filename("server", "alice"), filename("server", "alice"));
        assert_ne!(filename("server-a", "alice"), filename("server-b", "alice"));
        assert_ne!(filename("server", "alice"), filename("server", "bob"));
        assert_ne!(filename("ab", "c"), filename("a", "bc"));
        let name = filename("../../server", "../alice");
        assert_eq!(name.len(), 64 + ".json".len());
        assert!(name[..64].bytes().all(|byte| byte.is_ascii_hexdigit()));
        let fixture = Fixture::new();
        let first = fixture.0.join(filename("server-a", "alice"));
        let second = fixture.0.join(filename("server-b", "alice"));
        Outbox::load_path(first.clone())
            .unwrap()
            .enqueue("a", "bob", text("private"))
            .unwrap();
        assert_eq!(Outbox::load_path(first).unwrap().len(), 1);
        assert_eq!(Outbox::load_path(second).unwrap().len(), 0);
    }

    #[test]
    fn corrupt_duplicate_and_oversized_loads_do_not_overwrite() {
        let fixture = Fixture::new();
        let corrupt = b"invalid-json-private-message";
        fs::write(fixture.path(), corrupt).unwrap();
        let error = Outbox::load_path(fixture.path()).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(!error.to_string().contains("private-message"));
        assert_eq!(fs::read(fixture.path()).unwrap(), corrupt);
        let entry = Entry {
            client_id: "same".into(),
            recipient_id: "bob".into(),
            body: text("message"),
            blocked: false,
        };
        for entries in [
            vec![entry.clone(), entry.clone()],
            vec![entry; MAX_ENTRIES + 1],
        ] {
            let bytes = serde_json::to_vec(&entries).unwrap();
            fs::write(fixture.path(), &bytes).unwrap();
            assert!(Outbox::load_path(fixture.path()).is_err());
            assert_eq!(fs::read(fixture.path()).unwrap(), bytes);
        }
        File::create(fixture.path())
            .unwrap()
            .set_len(MAX_FILE_BYTES as u64 + 1)
            .unwrap();
        assert!(Outbox::load_path(fixture.path()).is_err());
        assert_eq!(
            fs::metadata(fixture.path()).unwrap().len(),
            MAX_FILE_BYTES as u64 + 1
        );
    }

    #[test]
    fn failed_replacement_never_commits_memory_or_removes_prior_data() {
        let fixture = Fixture::new();
        let mut queue = fixture.load();
        queue.enqueue("a", "bob", text("first")).unwrap();
        queue.enqueue("b", "bob", text("second")).unwrap();
        queue.block("a").unwrap();
        let original = fs::read(fixture.path()).unwrap();
        let backup = fixture.0.join("original.json");
        fs::rename(fixture.path(), &backup).unwrap();
        // Replacing a directory with a file fails even when tests run as root.
        fs::create_dir(fixture.path()).unwrap();
        assert!(queue.enqueue("c", "bob", text("third")).is_err());
        assert!(queue.ack("a").is_err());
        assert!(queue.block("b").is_err());
        assert!(queue.retry_all().is_err());
        assert!(queue.clear().is_err());
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.blocked_count(), 1);
        assert_eq!(queue.next().unwrap().client_id, "b");
        assert_eq!(fs::read(backup).unwrap(), original);
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn unix_directory_and_file_are_private_after_replacement() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::new();
        let mut queue = fixture.load();
        queue.enqueue("a", "bob", text("message")).unwrap();
        queue.block("a").unwrap();
        assert_eq!(
            fs::metadata(&fixture.0).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(fixture.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
