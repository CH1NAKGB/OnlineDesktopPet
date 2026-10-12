//! Synchronous cloud store. Call from `spawn_blocking`; share using `Arc<Store>`.
//! The caller authenticates legacy claims and authorizes recipient/partner access.

use std::{
    io::Cursor,
    path::Path,
    sync::{Mutex, MutexGuard},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow, ensure};
use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use image::{ImageFormat, ImageReader, Limits};
use rsa::rand_core::{OsRng, RngCore};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use shared_protocol::cloud::{AuthResult, CloudBody, CloudMessage, CloudSticker, ImageData};
use shared_protocol::{EncryptedFrame, SecureChannel};

const SESSION_MS: i64 = 30 * 24 * 60 * 60 * 1_000;
const MAX_ACCOUNTS: i64 = 1_000;
const MAX_SESSIONS: i64 = 8;
const MAX_DATABASE_BYTES: i64 = 512 * 1024 * 1024;
const WAL_CHECKPOINT_BYTES: i64 = 4 * 1024 * 1024;
const MAX_MESSAGES: i64 = 5_000;
const MAX_STICKERS: i64 = 20;
// Count encrypted serialized bytes, including base64 expansion and record metadata.
const MAX_HISTORY_BYTES: i64 = 32 * 1024 * 1024;
const MAX_STICKER_BYTES: i64 = 2 * 1024 * 1024;
const PAGE_SIZE: usize = 5;
const MAX_MEDIA_BYTES: usize = 256 * 1024;
const MAX_DIMENSION: u32 = 1024;
const KEY_MARKER: &str = "chicklink-cloud-store-v1";

pub struct Store {
    connection: Mutex<Connection>,
    channel: SecureChannel,
}

impl Store {
    /// Opens (or creates) the database and verifies the supplied encryption key.
    /// The key is never written to disk. Missing/corrupt markers fail closed.
    pub fn open(path: &Path, key: [u8; 32]) -> Result<Self> {
        let mut connection = db(Connection::open(path))?;
        db(connection.busy_timeout(Duration::from_secs(5)))?;
        db(connection.execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = FULL;"))?;
        let page_size: i64 =
            db(connection.pragma_query_value(None, "page_size", |row| row.get(0)))?;
        ensure!(
            page_size > 0 && page_size <= MAX_DATABASE_BYTES,
            "invalid database page size"
        );
        let max_pages = MAX_DATABASE_BYTES / page_size;
        db(connection.pragma_update(None, "max_page_count", max_pages))?;
        let actual_max: i64 =
            db(connection.pragma_query_value(None, "max_page_count", |row| row.get(0)))?;
        // SQLite cannot lower the limit beneath an existing oversized database.
        ensure!(actual_max <= max_pages, "cloud database exceeds size limit");
        let channel = SecureChannel::new(key);
        let tx = db(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
        let initialized: bool = db(tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='cloud_meta')",
            [],
            |row| row.get(0),
        ))?;
        if initialized {
            let marker: Option<Vec<u8>> = db(tx
                .query_row(
                    "SELECT value FROM cloud_meta WHERE name = 'key_verification'",
                    [],
                    |row| row.get(0),
                )
                .optional())?;
            let marker = marker.ok_or_else(|| anyhow!("cloud key verification failed"))?;
            let value: String =
                decrypt(&channel, &marker).map_err(|_| anyhow!("cloud key verification failed"))?;
            ensure!(value == KEY_MARKER, "cloud key verification failed");
        } else {
            let existing: bool = db(tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name IN
                 ('cloud_accounts', 'cloud_sessions', 'cloud_messages', 'cloud_stickers'))",
                [],
                |row| row.get(0),
            ))?;
            ensure!(!existing, "cloud key verification failed");
            db(tx.execute_batch(
                "CREATE TABLE cloud_meta (name TEXT PRIMARY KEY, value BLOB NOT NULL);
                 CREATE TABLE cloud_accounts (
                    user_id TEXT PRIMARY KEY,
                    username TEXT NOT NULL UNIQUE,
                    password_hash TEXT NOT NULL,
                    recovery_hash BLOB NOT NULL,
                    legacy_id TEXT UNIQUE
                 );
                 CREATE TABLE cloud_sessions (
                    token_hash BLOB PRIMARY KEY,
                    user_id TEXT NOT NULL REFERENCES cloud_accounts(user_id),
                    device_id TEXT NOT NULL,
                    expires_ms INTEGER NOT NULL
                 );
                 CREATE INDEX cloud_sessions_user ON cloud_sessions(user_id);
                 CREATE TABLE cloud_messages (
                    seq INTEGER PRIMARY KEY AUTOINCREMENT,
                    sender_id TEXT NOT NULL,
                    recipient_id TEXT NOT NULL,
                    client_id TEXT NOT NULL,
                    payload BLOB NOT NULL,
                    UNIQUE(sender_id, client_id)
                 );
                 CREATE INDEX cloud_messages_sender ON cloud_messages(sender_id, seq);
                 CREATE INDEX cloud_messages_recipient ON cloud_messages(recipient_id, seq);
                 CREATE TABLE cloud_stickers (
                    seq INTEGER PRIMARY KEY AUTOINCREMENT,
                    id TEXT NOT NULL UNIQUE,
                    user_id TEXT NOT NULL REFERENCES cloud_accounts(user_id),
                    payload BLOB NOT NULL
                 );
                 CREATE INDEX cloud_stickers_user ON cloud_stickers(user_id, seq);",
            ))?;
            db(tx.execute(
                "INSERT INTO cloud_meta(name, value) VALUES ('key_verification', ?1)",
                params![encrypt(&channel, &KEY_MARKER)?],
            ))?;
        }
        db(tx.commit())?;
        // Autocheckpoint and journal retention bound ordinary WAL growth. External
        // long-lived readers can delay checkpoints; this is not a filesystem quota.
        db(connection.pragma_update(None, "journal_mode", "WAL"))?;
        db(connection.pragma_update(
            None,
            "wal_autocheckpoint",
            (WAL_CHECKPOINT_BYTES / page_size).max(1),
        ))?;
        db(connection.pragma_update(None, "journal_size_limit", WAL_CHECKPOINT_BYTES))?;
        Ok(Self {
            connection: Mutex::new(connection),
            channel,
        })
    }

    /// Usernames are trimmed and lowercased. Password length is 10..=128 bytes.
    /// A validated legacy ID becomes the account ID, preserving existing partners.
    pub fn register(
        &self,
        username: &str,
        password: &str,
        device_id: &str,
        legacy_user_id: Option<&str>,
    ) -> Result<AuthResult> {
        let username = normalize_username(username)?;
        validate_password(password)?;
        validate_label(device_id, 128, "invalid device id")?;
        if let Some(id) = legacy_user_id {
            validate_user_id(id)?;
        }
        let user_id = match legacy_user_id {
            Some(id) => id.to_owned(),
            None => format!("cloud_{}", random_hex::<16>()?),
        };
        let password_hash = hash_password(password)?;
        let mut connection = self.lock()?;
        let tx = db(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
        let accounts: i64 =
            db(tx.query_row("SELECT COUNT(*) FROM cloud_accounts", [], |row| row.get(0)))?;
        ensure!(accounts < MAX_ACCOUNTS, "account capacity reached");
        let exists: bool = db(tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM cloud_accounts WHERE username = ?1 OR user_id = ?2 OR legacy_id = ?2)",
            params![username, user_id], |row| row.get(0),
        ))?;
        ensure!(!exists, "account or legacy identity unavailable");
        let recovery_code = random_hex::<24>()?;
        db(tx.execute(
            "INSERT INTO cloud_accounts(user_id, username, password_hash, recovery_hash, legacy_id)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                user_id,
                username,
                password_hash,
                secret_hash("recovery", &recovery_code),
                legacy_user_id
            ],
        ))?;
        let token = create_session(&tx, &user_id, device_id)?;
        db(tx.commit())?;
        Ok(AuthResult {
            user_id,
            username,
            token,
            recovery_code: Some(recovery_code),
        })
    }

    pub fn login(&self, username: &str, password: &str, device_id: &str) -> Result<AuthResult> {
        let username = normalize_username(username)?;
        validate_password(password)?;
        validate_label(device_id, 128, "invalid device id")?;
        let account: Option<(String, String)> = {
            let connection = self.lock()?;
            db(connection
                .query_row(
                    "SELECT user_id, password_hash FROM cloud_accounts WHERE username = ?1",
                    params![username],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional())?
        };
        let (user_id, password_hash) = account.ok_or_else(|| anyhow!("invalid credentials"))?;
        verify_password(password, &password_hash)?;
        let mut connection = self.lock()?;
        let tx = db(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
        // Recovery may have changed the password while Argon2 ran without the mutex.
        let unchanged: bool = db(tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM cloud_accounts WHERE user_id = ?1 AND password_hash = ?2)",
            params![user_id, password_hash],
            |row| row.get(0),
        ))?;
        ensure!(unchanged, "invalid credentials");
        let token = create_session(&tx, &user_id, device_id)?;
        db(tx.commit())?;
        Ok(AuthResult {
            user_id,
            username,
            token,
            recovery_code: None,
        })
    }

    /// Consumes the recovery code, replaces the password/code, and revokes every
    /// previous session atomically before issuing the recovering device a token.
    pub fn recover(
        &self,
        username: &str,
        recovery_code: &str,
        new_password: &str,
        device_id: &str,
    ) -> Result<AuthResult> {
        let username = normalize_username(username)?;
        validate_password(new_password)?;
        validate_label(device_id, 128, "invalid device id")?;
        ensure!(valid_secret(recovery_code, 48), "invalid credentials");
        let recovery_hash = secret_hash("recovery", recovery_code);
        {
            let connection = self.lock()?;
            let valid: bool = db(connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM cloud_accounts WHERE username = ?1 AND recovery_hash = ?2)",
                params![username, recovery_hash], |row| row.get(0),
            ))?;
            ensure!(valid, "invalid credentials");
        }
        let password_hash = hash_password(new_password)?;
        let mut connection = self.lock()?;
        let tx = db(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
        let user_id: Option<String> = db(tx
            .query_row(
                "SELECT user_id FROM cloud_accounts WHERE username = ?1 AND recovery_hash = ?2",
                params![username, recovery_hash],
                |row| row.get(0),
            )
            .optional())?;
        let user_id = user_id.ok_or_else(|| anyhow!("invalid credentials"))?;
        let recovery_code = random_hex::<24>()?;
        db(tx.execute(
            "UPDATE cloud_accounts SET password_hash = ?1, recovery_hash = ?2 WHERE user_id = ?3",
            params![
                password_hash,
                secret_hash("recovery", &recovery_code),
                user_id
            ],
        ))?;
        db(tx.execute(
            "DELETE FROM cloud_sessions WHERE user_id = ?1",
            params![user_id],
        ))?;
        let token = create_session(&tx, &user_id, device_id)?;
        db(tx.commit())?;
        Ok(AuthResult {
            user_id,
            username,
            token,
            recovery_code: Some(recovery_code),
        })
    }

    /// Resumption does not extend the token's fixed 30-day lifetime.
    pub fn resume(&self, token: &str, device_id: &str) -> Result<AuthResult> {
        ensure!(valid_secret(token, 64), "invalid session");
        validate_label(device_id, 128, "invalid device id")?;
        let connection = self.lock()?;
        let account: Option<(String, String)> = db(connection
            .query_row(
                "SELECT a.user_id, a.username FROM cloud_sessions s
             JOIN cloud_accounts a ON a.user_id = s.user_id
             WHERE s.token_hash = ?1 AND s.device_id = ?2 AND s.expires_ms > ?3",
                params![secret_hash("session", token), device_id, now_ms()?],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional())?;
        let (user_id, username) = account.ok_or_else(|| anyhow!("invalid session"))?;
        Ok(AuthResult {
            user_id,
            username,
            token: token.to_owned(),
            recovery_code: None,
        })
    }

    pub fn revoke(&self, token: &str) -> Result<()> {
        if valid_secret(token, 64) {
            db(self.lock()?.execute(
                "DELETE FROM cloud_sessions WHERE token_hash = ?1",
                params![secret_hash("session", token)],
            ))?;
        }
        Ok(())
    }

    pub fn resolve_token(&self, token: &str) -> Result<Option<String>> {
        if !valid_secret(token, 64) {
            return Ok(None);
        }
        db(self
            .lock()?
            .query_row(
                "SELECT user_id FROM cloud_sessions WHERE token_hash = ?1 AND expires_ms > ?2",
                params![secret_hash("session", token), now_ms()?],
                |row| row.get(0),
            )
            .optional())
    }

    pub fn is_claimed_legacy(&self, id: &str) -> Result<bool> {
        // Also protects newly generated account IDs from a legacy identity collision.
        db(self.lock()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM cloud_accounts WHERE legacy_id = ?1 OR user_id = ?1)",
            params![id],
            |row| row.get(0),
        ))
    }

    /// Recipient IDs may belong to legacy partners who have not registered yet.
    /// Both participants' stored-message quotas apply, including to those IDs.
    pub fn send_message(
        &self,
        user_id: &str,
        recipient_id: &str,
        client_id: &str,
        body: CloudBody,
    ) -> Result<CloudMessage> {
        validate_user_id(recipient_id)?;
        validate_label(client_id, 128, "invalid client id")?;
        validate_body(&body)?;
        let mut connection = self.lock()?;
        let tx = db(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
        require_account(&tx, user_id)?;
        let previous = db(tx
            .query_row(
                "SELECT seq, sender_id, recipient_id, client_id, payload FROM cloud_messages
             WHERE sender_id = ?1 AND client_id = ?2",
                params![user_id, client_id],
                stored_message,
            )
            .optional())?;
        if let Some(previous) = previous {
            let item = self.read_message(previous)?;
            ensure!(
                item.recipient_id == recipient_id && item.body == body,
                "client id already used for a different message"
            );
            return Ok(item);
        }
        let mut used_bytes = 0;
        for participant in [user_id, recipient_id] {
            let (count, bytes): (i64, i64) = db(tx.query_row(
                "SELECT COUNT(*), COALESCE(SUM(length(payload)), 0) FROM cloud_messages
                 WHERE sender_id = ?1 OR recipient_id = ?1",
                params![participant],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ))?;
            ensure!(count < MAX_MESSAGES, "message quota reached");
            used_bytes = used_bytes.max(bytes);
        }
        // Reserve the persistent sequence inside the transaction; plaintext is never bound to SQL.
        db(tx.execute(
            "INSERT INTO cloud_messages(sender_id, recipient_id, client_id, payload) VALUES (?1, ?2, ?3, X'')",
            params![user_id, recipient_id, client_id],
        ))?;
        let item = CloudMessage {
            seq: u64::try_from(tx.last_insert_rowid())
                .map_err(|_| anyhow!("invalid message sequence"))?,
            client_id: client_id.to_owned(),
            sender_id: user_id.to_owned(),
            recipient_id: recipient_id.to_owned(),
            body,
            unix_ms: now_ms()? as u64,
        };
        let payload = encrypt(&self.channel, &item)?;
        ensure!(
            used_bytes + payload.len() as i64 <= MAX_HISTORY_BYTES,
            "history storage quota reached"
        );
        db(tx.execute(
            "UPDATE cloud_messages SET payload = ?1 WHERE seq = ?2",
            params![payload, tx.last_insert_rowid()],
        ))?;
        db(tx.commit())?;
        Ok(item)
    }

    pub fn history(&self, user_id: &str, after: u64) -> Result<(Vec<CloudMessage>, u64, bool)> {
        let connection = self.lock()?;
        require_account(&connection, user_id)?;
        let Ok(cursor) = i64::try_from(after) else {
            return Ok((Vec::new(), after, false));
        };
        let mut statement = db(connection.prepare(
            "SELECT seq, sender_id, recipient_id, client_id, payload FROM cloud_messages
             WHERE (sender_id = ?1 OR recipient_id = ?1) AND seq > ?2 ORDER BY seq ASC LIMIT 6",
        ))?;
        let rows = db(statement.query_map(params![user_id, cursor], stored_message))?;
        let mut items = Vec::new();
        for row in rows {
            items.push(self.read_message(db(row)?)?);
        }
        let has_more = items.len() > PAGE_SIZE;
        items.truncate(PAGE_SIZE);
        let next_cursor = items.last().map_or(after, |item| item.seq);
        Ok((items, next_cursor, has_more))
    }

    pub fn save_sticker(
        &self,
        user_id: &str,
        name: &str,
        image: ImageData,
    ) -> Result<CloudSticker> {
        validate_label(name, 128, "invalid sticker name")?;
        validate_image(&image)?;
        let mut connection = self.lock()?;
        let tx = db(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
        require_account(&tx, user_id)?;
        let (count, bytes): (i64, i64) = db(tx.query_row(
            "SELECT COUNT(*), COALESCE(SUM(length(payload)), 0) FROM cloud_stickers WHERE user_id = ?1",
            params![user_id], |row| Ok((row.get(0)?, row.get(1)?)),
        ))?;
        ensure!(count < MAX_STICKERS, "sticker quota reached");
        let item = CloudSticker {
            id: random_hex::<16>()?,
            name: name.to_owned(),
            image,
        };
        // Encrypt ownership too, so swapping ciphertext between owners is detected.
        let payload = encrypt(&self.channel, &(user_id, &item))?;
        ensure!(
            bytes + payload.len() as i64 <= MAX_STICKER_BYTES,
            "sticker storage quota reached"
        );
        db(tx.execute(
            "INSERT INTO cloud_stickers(id, user_id, payload) VALUES (?1, ?2, ?3)",
            params![item.id, user_id, payload],
        ))?;
        db(tx.commit())?;
        Ok(item)
    }

    pub fn stickers(&self, user_id: &str) -> Result<Vec<CloudSticker>> {
        let connection = self.lock()?;
        require_account(&connection, user_id)?;
        let mut statement = db(connection.prepare(
            "SELECT id, payload FROM cloud_stickers WHERE user_id = ?1 ORDER BY seq ASC LIMIT 20",
        ))?;
        let rows = db(statement.query_map(params![user_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        }))?;
        let mut items = Vec::new();
        for row in rows {
            let (id, payload) = db(row)?;
            let (owner, item): (String, CloudSticker) = decrypt(&self.channel, &payload)?;
            ensure!(
                owner == user_id && item.id == id,
                "invalid encrypted record"
            );
            items.push(item);
        }
        Ok(items)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| anyhow!("cloud storage unavailable"))
    }

    fn read_message(&self, row: StoredMessage) -> Result<CloudMessage> {
        let item: CloudMessage = decrypt(&self.channel, &row.payload)?;
        ensure!(
            i64::try_from(item.seq).ok() == Some(row.seq)
                && item.sender_id == row.sender_id
                && item.recipient_id == row.recipient_id
                && item.client_id == row.client_id,
            "invalid encrypted record"
        );
        Ok(item)
    }
}

struct StoredMessage {
    seq: i64,
    sender_id: String,
    recipient_id: String,
    client_id: String,
    payload: Vec<u8>,
}

fn stored_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredMessage> {
    Ok(StoredMessage {
        seq: row.get(0)?,
        sender_id: row.get(1)?,
        recipient_id: row.get(2)?,
        client_id: row.get(3)?,
        payload: row.get(4)?,
    })
}

fn db<T>(result: rusqlite::Result<T>) -> Result<T> {
    result.map_err(|error| match error {
        rusqlite::Error::SqliteFailure(error, _) if error.code == rusqlite::ErrorCode::DiskFull => {
            anyhow!("cloud storage capacity reached")
        }
        _ => anyhow!("cloud storage unavailable"),
    })
}

fn encrypt<T: Serialize>(channel: &SecureChannel, value: &T) -> Result<Vec<u8>> {
    let frame = channel
        .encrypt(value)
        .map_err(|_| anyhow!("cloud encryption failed"))?;
    serde_json::to_vec(&frame).map_err(|_| anyhow!("cloud encryption failed"))
}

fn decrypt<T: DeserializeOwned>(channel: &SecureChannel, payload: &[u8]) -> Result<T> {
    let frame: EncryptedFrame =
        serde_json::from_slice(payload).map_err(|_| anyhow!("invalid encrypted record"))?;
    channel
        .decrypt(&frame)
        .map_err(|_| anyhow!("invalid encrypted record"))
}

fn now_ms() -> Result<i64> {
    let value = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| anyhow!("system clock unavailable"))?
        .as_millis();
    ensure!(
        value <= (i64::MAX - SESSION_MS) as u128,
        "system clock unavailable"
    );
    Ok(value as i64)
}

fn random_hex<const N: usize>() -> Result<String> {
    let mut bytes = [0; N];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| anyhow!("secure randomness unavailable"))?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(N * 2);
    for byte in bytes {
        text.push(HEX[(byte >> 4) as usize] as char);
        text.push(HEX[(byte & 15) as usize] as char);
    }
    Ok(text)
}

fn secret_hash(domain: &str, secret: &str) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(secret.as_bytes());
    hasher.finalize().to_vec()
}

fn valid_secret(secret: &str, length: usize) -> bool {
    secret.len() == length
        && secret
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn argon2() -> Result<Argon2<'static>> {
    let params = Params::new(19 * 1024, 2, 1, Some(32))
        .map_err(|_| anyhow!("password service unavailable"))?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

fn hash_password(password: &str) -> Result<String> {
    let mut salt = [0; 16];
    OsRng
        .try_fill_bytes(&mut salt)
        .map_err(|_| anyhow!("secure randomness unavailable"))?;
    let salt =
        SaltString::encode_b64(&salt).map_err(|_| anyhow!("password service unavailable"))?;
    argon2()?
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| anyhow!("password service unavailable"))
}

fn verify_password(password: &str, encoded: &str) -> Result<()> {
    ensure!(encoded.len() <= 256, "invalid credentials");
    let parsed = PasswordHash::new(encoded).map_err(|_| anyhow!("invalid credentials"))?;
    // PasswordVerifier reads costs from the stored PHC string, not the instance.
    ensure!(
        parsed.algorithm.as_str() == "argon2id"
            && parsed.version == Some(19)
            && parsed.params.to_string() == "m=19456,t=2,p=1"
            && parsed.hash.is_some_and(|hash| hash.len() == 32),
        "invalid credentials"
    );
    argon2()?
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| anyhow!("invalid credentials"))
}

fn create_session(connection: &Connection, user_id: &str, device_id: &str) -> Result<String> {
    let now = now_ms()?;
    db(connection.execute(
        "DELETE FROM cloud_sessions WHERE expires_ms <= ?1",
        params![now],
    ))?;
    db(connection.execute(
        "DELETE FROM cloud_sessions WHERE user_id = ?1 AND device_id = ?2",
        params![user_id, device_id],
    ))?;
    // Keep the seven newest existing devices; the new token occupies the eighth slot.
    db(connection.execute(
        "DELETE FROM cloud_sessions WHERE token_hash IN (
            SELECT token_hash FROM cloud_sessions WHERE user_id = ?1
            ORDER BY expires_ms DESC, token_hash LIMIT -1 OFFSET ?2
         )",
        params![user_id, MAX_SESSIONS - 1],
    ))?;
    let token = random_hex::<32>()?;
    db(connection.execute(
        "INSERT INTO cloud_sessions(token_hash, user_id, device_id, expires_ms) VALUES (?1, ?2, ?3, ?4)",
        params![secret_hash("session", &token), user_id, device_id, now + SESSION_MS],
    ))?;
    Ok(token)
}

fn require_account(connection: &Connection, user_id: &str) -> Result<()> {
    let exists: bool = db(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM cloud_accounts WHERE user_id = ?1)",
        params![user_id],
        |row| row.get(0),
    ))?;
    ensure!(exists, "unknown account");
    Ok(())
}

fn normalize_username(username: &str) -> Result<String> {
    ensure!(username.len() <= 256, "invalid username");
    let username = username.trim().to_lowercase();
    ensure!(
        !username.is_empty()
            && username.chars().count() <= 64
            && !username
                .chars()
                .any(|c| c.is_control() || c.is_whitespace()),
        "invalid username"
    );
    Ok(username)
}

fn validate_password(password: &str) -> Result<()> {
    ensure!(
        (10..=128).contains(&password.len()),
        "password must be 10 to 128 bytes"
    );
    Ok(())
}

fn validate_label(value: &str, max_bytes: usize, error: &'static str) -> Result<()> {
    ensure!(
        !value.trim().is_empty()
            && value.len() <= max_bytes
            && !value.chars().any(char::is_control),
        error
    );
    Ok(())
}

fn validate_user_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 64
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')),
        "invalid user id"
    );
    Ok(())
}

fn validate_body(body: &CloudBody) -> Result<()> {
    match body {
        CloudBody::Text { text } => {
            ensure!(
                !text.is_empty()
                    && text.len() <= shared_protocol::MAX_TEXT_CHARS * 4
                    && text.chars().count() <= shared_protocol::MAX_TEXT_CHARS,
                "invalid message text"
            );
            Ok(())
        }
        CloudBody::Image { image } => validate_image(image),
    }
}

fn validate_image(image: &ImageData) -> Result<()> {
    ensure!(
        image.data_base64.len() <= MAX_MEDIA_BYTES.div_ceil(3) * 4,
        "image exceeds 256 KiB"
    );
    ensure!(
        (1..=MAX_DIMENSION).contains(&image.width) && (1..=MAX_DIMENSION).contains(&image.height),
        "invalid image dimensions"
    );
    let expected = match image.mime.as_str() {
        "image/png" => ImageFormat::Png,
        "image/jpeg" => ImageFormat::Jpeg,
        _ => return Err(anyhow!("only PNG and JPEG images are supported")),
    };
    let bytes = BASE64
        .decode(&image.data_base64)
        .map_err(|_| anyhow!("invalid image data"))?;
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_MEDIA_BYTES,
        "image exceeds 256 KiB"
    );
    let format = image::guess_format(&bytes).map_err(|_| anyhow!("invalid image data"))?;
    ensure!(format == expected, "image format does not match MIME type");
    let mut reader = ImageReader::with_format(Cursor::new(&bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|_| anyhow!("invalid image data"))?;
    ensure!(
        decoded.width() == image.width && decoded.height() == image.height,
        "image dimensions do not match payload"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf, sync::Arc};

    const KEY: [u8; 32] = [0xa9; 32];
    const PASSWORD: &str = "private-password-never-persisted";

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("chicklink-cloud-{}", random_hex::<16>().unwrap()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> PathBuf {
            self.0.join("cloud.sqlite")
        }

        fn open(&self) -> Store {
            Store::open(&self.path(), KEY).unwrap()
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn register(store: &Store, username: &str) -> AuthResult {
        store.register(username, PASSWORD, "phone", None).unwrap()
    }

    fn text(value: &str) -> CloudBody {
        CloudBody::Text {
            text: value.to_owned(),
        }
    }

    fn media(format: ImageFormat, width: u32, height: u32) -> ImageData {
        let pixels = image::RgbImage::from_fn(width, height, |x, y| {
            image::Rgb([(x % 255) as u8, (y % 255) as u8, ((x + y) % 255) as u8])
        });
        let mut output = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(pixels)
            .write_to(&mut output, format)
            .unwrap();
        ImageData {
            mime: if format == ImageFormat::Png {
                "image/png"
            } else {
                "image/jpeg"
            }
            .into(),
            data_base64: BASE64.encode(output.into_inner()),
            width,
            height,
        }
    }

    fn large_media() -> ImageData {
        let mut pixels = vec![0; 256 * 256 * 3];
        OsRng.fill_bytes(&mut pixels);
        let pixels = image::RgbImage::from_raw(256, 256, pixels).unwrap();
        let mut output = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(pixels)
            .write_to(&mut output, ImageFormat::Png)
            .unwrap();
        ImageData {
            mime: "image/png".into(),
            data_base64: BASE64.encode(output.into_inner()),
            width: 256,
            height: 256,
        }
    }

    #[test]
    fn account_and_session_metadata_are_bounded() {
        let dir = TestDirectory::new();
        let store = dir.open();
        let alice = register(&store, "alice");
        {
            let mut connection = store.lock().unwrap();
            let tx = connection.transaction().unwrap();
            for index in 1..MAX_ACCOUNTS {
                tx.execute(
                    "INSERT INTO cloud_accounts(user_id, username, password_hash, recovery_hash)
                     SELECT ?1, ?1, password_hash, recovery_hash FROM cloud_accounts WHERE user_id = ?2",
                    params![format!("seed-{index}"), alice.user_id],
                ).unwrap();
            }
            tx.commit().unwrap();
        }
        assert_eq!(
            store
                .register("overflow", PASSWORD, "phone", Some("unclaimed"))
                .unwrap_err()
                .to_string(),
            "account capacity reached"
        );
        assert!(!store.is_claimed_legacy("unclaimed").unwrap());
        // Exercise the same transactional helper without repeatedly paying Argon2's cost.
        let mut tokens = Vec::new();
        {
            let mut connection = store.lock().unwrap();
            for index in 0..MAX_SESSIONS + 3 {
                let tx = connection.transaction().unwrap();
                let device = format!("device-{index}");
                tokens.push((
                    create_session(&tx, &alice.user_id, &device).unwrap(),
                    device,
                ));
                tx.commit().unwrap();
            }
            let count: i64 = connection
                .query_row("SELECT COUNT(*) FROM cloud_sessions", [], |row| row.get(0))
                .unwrap();
            assert_eq!(count, MAX_SESSIONS);
        }
        assert!(store.login("alice", PASSWORD, "phone").is_ok());
        let valid = tokens
            .iter()
            .filter(|(token, _)| store.resolve_token(token).unwrap().is_some())
            .count();
        assert_eq!(valid, (MAX_SESSIONS - 1) as usize);
        let (token, device) = tokens.last().unwrap();
        // The latest token can share a millisecond with others; every surviving
        // token still enforces device binding irrespective of eviction tie order.
        if store.resolve_token(token).unwrap().is_some() {
            assert!(store.resume(token, device).is_ok());
            assert!(store.resume(token, "wrong-device").is_err());
        }
    }

    #[test]
    fn database_size_limit_uses_actual_page_size_and_full_writes_roll_back() {
        let dir = TestDirectory::new();
        {
            let connection = Connection::open(dir.path()).unwrap();
            connection
                .execute_batch(
                    "PRAGMA page_size = 8192; CREATE TABLE page_size_probe (id INTEGER);",
                )
                .unwrap();
        }
        let store = dir.open();
        let alice = register(&store, "alice");
        let first = store
            .send_message(&alice.user_id, "legacy", "first", text("preserved"))
            .unwrap();
        {
            let connection = store.lock().unwrap();
            let value = |name: &str| {
                connection
                    .pragma_query_value(None, name, |row| row.get::<_, i64>(0))
                    .unwrap()
            };
            assert_eq!(value("page_size"), 8192);
            assert_eq!(
                value("max_page_count") * value("page_size"),
                MAX_DATABASE_BYTES
            );
            assert_eq!(
                value("wal_autocheckpoint") * value("page_size"),
                WAL_CHECKPOINT_BYTES
            );
            assert_eq!(value("journal_size_limit"), WAL_CHECKPOINT_BYTES);
            // Lower the same SQLite hard limit to test SQLITE_FULL without writing 512 MiB.
            connection
                .pragma_update(None, "max_page_count", value("page_count") + 2)
                .unwrap();
        }
        let error = store
            .send_message(
                &alice.user_id,
                "legacy",
                "too-large",
                CloudBody::Image {
                    image: large_media(),
                },
            )
            .unwrap_err();
        assert_eq!(error.to_string(), "cloud storage capacity reached");
        assert_eq!(
            store.history(&alice.user_id, 0).unwrap().0,
            vec![first.clone()]
        );
        assert_eq!(
            store
                .send_message(&alice.user_id, "legacy", "first", text("preserved"))
                .unwrap(),
            first
        );
        drop(store);
        let store = dir.open();
        assert_eq!(store.history(&alice.user_id, 0).unwrap().0, vec![first]);
        let connection = store.lock().unwrap();
        let pages: i64 = connection
            .pragma_query_value(None, "max_page_count", |row| row.get(0))
            .unwrap();
        assert_eq!(pages * 8192, MAX_DATABASE_BYTES);
    }

    #[test]
    fn gallery_byte_limit_keeps_encrypted_response_below_frame_limit() {
        use shared_protocol::{ServerEvent, ServerFrame, cloud::CloudResponse};
        let dir = TestDirectory::new();
        let store = dir.open();
        let alice = register(&store, "alice");
        let bob = register(&store, "bob");
        let image = large_media();
        let mut accepted = Vec::new();
        loop {
            match store.save_sticker(&alice.user_id, "large sticker", image.clone()) {
                Ok(item) => accepted.push(item),
                Err(error) => {
                    assert_eq!(error.to_string(), "sticker storage quota reached");
                    break;
                }
            }
            assert!(accepted.len() < MAX_STICKERS as usize);
        }
        assert!(!accepted.is_empty());
        let items = store.stickers(&alice.user_id).unwrap();
        assert_eq!(items, accepted);
        let event = ServerEvent::Cloud {
            response: CloudResponse::Stickers { items },
        };
        let frame = ServerFrame::Encrypted {
            frame: store.channel.encrypt(&event).unwrap(),
        };
        assert!(serde_json::to_vec(&frame).unwrap().len() < 4 * 1024 * 1024);
        let bytes: i64 = store
            .lock()
            .unwrap()
            .query_row(
                "SELECT SUM(length(payload)) FROM cloud_stickers",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(bytes <= MAX_STICKER_BYTES);
        assert!(
            store
                .save_sticker(&bob.user_id, "separate quota", image.clone())
                .is_ok()
        );
        drop(store);
        let store = dir.open();
        assert_eq!(store.stickers(&alice.user_id).unwrap(), accepted);
        assert!(
            store
                .save_sticker(&alice.user_id, "still full", image)
                .is_err()
        );
    }

    #[test]
    fn history_byte_limit_applies_to_both_participants_and_preserves_retries() {
        let dir = TestDirectory::new();
        let store = dir.open();
        let alice = register(&store, "alice");
        let bob = register(&store, "bob");
        let body = CloudBody::Image {
            image: large_media(),
        };
        let first = store
            .send_message(&alice.user_id, "legacy", "first", body.clone())
            .unwrap();
        let mut accepted = 1;
        loop {
            match store.send_message(
                &alice.user_id,
                "legacy",
                &format!("large-{accepted}"),
                body.clone(),
            ) {
                Ok(_) => accepted += 1,
                Err(error) => {
                    assert_eq!(error.to_string(), "history storage quota reached");
                    break;
                }
            }
            assert!(accepted < MAX_MESSAGES);
        }
        for recipient in [&alice.user_id, "legacy"] {
            assert_eq!(
                store
                    .send_message(&bob.user_id, recipient, "over-limit", body.clone())
                    .unwrap_err()
                    .to_string(),
                "history storage quota reached"
            );
        }
        assert_eq!(
            store
                .send_message(&alice.user_id, "legacy", "first", body)
                .unwrap(),
            first
        );
        let (count, bytes): (i64, i64) = store
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*), SUM(length(payload)) FROM cloud_messages",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, accepted);
        assert!(bytes <= MAX_HISTORY_BYTES);
        assert_eq!(store.history(&alice.user_id, 0).unwrap().0[0], first);
        assert!(
            store
                .send_message(&bob.user_id, "another-legacy", "valid", text("own space"))
                .is_ok()
        );
    }

    #[test]
    fn authentication_recovery_revocation_and_expiry() {
        let dir = TestDirectory::new();
        let store = dir.open();
        let initial = register(&store, "Alice");
        assert_eq!(initial.username, "alice");
        assert_eq!(initial.token.len(), 64);
        assert_eq!(initial.recovery_code.as_ref().unwrap().len(), 48);
        assert_eq!(
            store.resolve_token(&initial.token).unwrap(),
            Some(initial.user_id.clone())
        );
        assert!(store.resume(&initial.token, "other").is_err());
        assert!(store.login("alice", "incorrect-password", "phone").is_err());
        let second = store.login(" ALICE ", PASSWORD, "desktop").unwrap();
        assert!(second.recovery_code.is_none());
        assert!(store.resume(&initial.token, "phone").is_ok());
        assert!(
            store
                .recover("alice", &"0".repeat(48), "replacement-password", "new")
                .is_err()
        );
        assert!(
            store
                .recover(
                    "alice",
                    initial.recovery_code.as_ref().unwrap(),
                    "short",
                    "new"
                )
                .is_err()
        );
        assert!(store.resume(&second.token, "desktop").is_ok());
        let recovered = store
            .recover(
                "alice",
                initial.recovery_code.as_ref().unwrap(),
                "replacement-password",
                "new",
            )
            .unwrap();
        assert_ne!(recovered.recovery_code, initial.recovery_code);
        assert_eq!(recovered.user_id, initial.user_id);
        for token in [&initial.token, &second.token] {
            assert_eq!(store.resolve_token(token).unwrap(), None);
        }
        assert!(store.resume(&initial.token, "phone").is_err());
        assert!(store.login("alice", PASSWORD, "phone").is_err());
        assert!(
            store
                .recover(
                    "alice",
                    initial.recovery_code.as_ref().unwrap(),
                    PASSWORD,
                    "new"
                )
                .is_err()
        );
        assert!(store.resume(&recovered.token, "new").is_ok());
        store.revoke(&recovered.token).unwrap();
        store.revoke(&recovered.token).unwrap();
        assert_eq!(store.resolve_token(&recovered.token).unwrap(), None);
        let login = store.login("alice", "replacement-password", "new").unwrap();
        let replacement = store.login("alice", "replacement-password", "new").unwrap();
        assert_eq!(store.resolve_token(&login.token).unwrap(), None);
        store
            .lock()
            .unwrap()
            .execute("UPDATE cloud_sessions SET expires_ms = 0", [])
            .unwrap();
        assert_eq!(store.resolve_token(&replacement.token).unwrap(), None);
        assert!(store.resume(&replacement.token, "new").is_err());
        assert_eq!(store.resolve_token("invalid").unwrap(), None);
    }

    #[test]
    fn password_bounds_and_stored_argon_costs_are_enforced() {
        let dir = TestDirectory::new();
        let store = dir.open();
        for password in ["x".repeat(9), "x".repeat(129)] {
            assert!(store.register("alice", &password, "phone", None).is_err());
        }
        let alice = store
            .register("alice", &"a".repeat(10), "phone", None)
            .unwrap();
        store
            .register("bob", &"b".repeat(128), "phone", None)
            .unwrap();
        let hash: String = store
            .lock()
            .unwrap()
            .query_row(
                "SELECT password_hash FROM cloud_accounts WHERE user_id = ?1",
                params![alice.user_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        assert!(verify_password(&"a".repeat(10), &hash).is_ok());
        assert!(
            verify_password(&"a".repeat(10), &hash.replace("m=19456", "m=4294967295")).is_err()
        );
    }

    #[test]
    fn legacy_claim_is_unique_atomic_and_preserves_identity() {
        let dir = TestDirectory::new();
        let store = dir.open();
        assert!(!store.is_claimed_legacy("legacy-a").unwrap());
        let alice = store
            .register("alice", PASSWORD, "phone", Some("legacy-a"))
            .unwrap();
        assert_eq!(alice.user_id, "legacy-a");
        assert!(store.is_claimed_legacy("legacy-a").unwrap());
        assert!(
            store
                .register("bob", PASSWORD, "phone", Some("legacy-a"))
                .is_err()
        );
        assert!(
            store
                .register("ALICE", PASSWORD, "phone", Some("legacy-b"))
                .is_err()
        );
        assert!(!store.is_claimed_legacy("legacy-b").unwrap());
        assert!(store.login("bob", PASSWORD, "phone").is_err());
        drop(store);
        let store = dir.open();
        assert!(store.is_claimed_legacy("legacy-a").unwrap());
        assert_eq!(
            store.resume(&alice.token, "phone").unwrap().user_id,
            "legacy-a"
        );
    }

    #[test]
    fn concurrent_claim_and_recovery_have_one_winner() {
        let dir = TestDirectory::new();
        let stores = [Arc::new(dir.open()), Arc::new(dir.open())];
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = stores
            .iter()
            .enumerate()
            .map(|(index, store)| {
                let store = Arc::clone(store);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    store.register(&format!("user{index}"), PASSWORD, "phone", Some("legacy"))
                })
            })
            .collect();
        let successes: Vec<_> = handles
            .into_iter()
            .filter_map(|handle| handle.join().unwrap().ok())
            .collect();
        assert_eq!(successes.len(), 1);
        let account = &successes[0];
        let handles: Vec<_> = stores
            .iter()
            .map(|store| {
                let store = Arc::clone(store);
                let barrier = Arc::clone(&barrier);
                let account = account.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.recover(
                        &account.username,
                        account.recovery_code.as_ref().unwrap(),
                        "new-password",
                        "desktop",
                    )
                })
            })
            .collect();
        let recovered: Vec<_> = handles
            .into_iter()
            .filter_map(|handle| handle.join().unwrap().ok())
            .collect();
        assert_eq!(recovered.len(), 1);
        assert_eq!(stores[0].resolve_token(&account.token).unwrap(), None);
        assert!(stores[0].resume(&recovered[0].token, "desktop").is_ok());
    }

    #[test]
    fn history_dedup_isolation_and_sequences_survive_restart() {
        let dir = TestDirectory::new();
        let store = dir.open();
        let alice = register(&store, "alice");
        let bob = register(&store, "bob");
        let eve = register(&store, "eve");
        let first = store
            .send_message(&alice.user_id, &bob.user_id, "first", text("hello"))
            .unwrap();
        assert_eq!(
            store
                .send_message(&alice.user_id, &bob.user_id, "first", text("hello"))
                .unwrap(),
            first
        );
        assert!(
            store
                .send_message(&alice.user_id, &bob.user_id, "first", text("changed"))
                .is_err()
        );
        assert!(
            store
                .send_message(&alice.user_id, &eve.user_id, "first", text("hello"))
                .is_err()
        );
        assert!(store.history(&eve.user_id, 0).unwrap().0.is_empty());
        assert!(store.history("missing", 0).is_err());
        for index in 0..7 {
            store
                .send_message(
                    &bob.user_id,
                    &alice.user_id,
                    &format!("reply-{index}"),
                    text("reply"),
                )
                .unwrap();
        }
        store
            .send_message(&bob.user_id, &eve.user_id, "first", text("private to eve"))
            .unwrap();
        let (page, cursor, more) = store.history(&alice.user_id, 0).unwrap();
        assert_eq!(page.len(), 5);
        assert!(more);
        assert_eq!(cursor, page.last().unwrap().seq);
        assert!(page.windows(2).all(|pair| pair[0].seq < pair[1].seq));
        let (second_page, last, more) = store.history(&alice.user_id, cursor).unwrap();
        assert_eq!(second_page.len(), 3);
        assert!(!more);
        assert!(second_page.iter().all(|item| item.seq > cursor));
        assert_eq!(
            store.history(&alice.user_id, last).unwrap(),
            (vec![], last, false)
        );
        assert_eq!(
            store.history(&alice.user_id, u64::MAX).unwrap(),
            (vec![], u64::MAX, false)
        );
        drop(store);
        let store = dir.open();
        assert_eq!(store.history(&alice.user_id, 0).unwrap().0, page);
        assert_eq!(
            store
                .send_message(&alice.user_id, &bob.user_id, "first", text("hello"))
                .unwrap(),
            first
        );
        let newest = store
            .send_message(
                &alice.user_id,
                "legacy-recipient",
                "offline",
                text("offline"),
            )
            .unwrap();
        assert!(newest.seq > last + 1);
        let migrated = store
            .register("migrated", PASSWORD, "phone", Some("legacy-recipient"))
            .unwrap();
        assert_eq!(store.history(&migrated.user_id, 0).unwrap().0, vec![newest]);
    }

    #[test]
    fn secrets_and_payloads_are_encrypted_in_database_and_wal() {
        let dir = TestDirectory::new();
        let store = dir.open();
        store
            .lock()
            .unwrap()
            .execute_batch("PRAGMA journal_mode = WAL;")
            .unwrap();
        let alice = register(&store, "alice");
        let plaintext = "sensitive-message-body-that-must-not-appear-on-disk";
        store
            .send_message(&alice.user_id, "legacy", "text", text(plaintext))
            .unwrap();
        let image = media(ImageFormat::Png, 8, 8);
        store
            .send_message(
                &alice.user_id,
                "legacy",
                "image",
                CloudBody::Image {
                    image: image.clone(),
                },
            )
            .unwrap();
        let sticker = store
            .save_sticker(&alice.user_id, "sensitive-sticker-name", image.clone())
            .unwrap();
        let decoded = BASE64.decode(&image.data_base64).unwrap();
        let secrets = [
            PASSWORD.as_bytes(),
            plaintext.as_bytes(),
            image.data_base64.as_bytes(),
            decoded.as_slice(),
            alice.token.as_bytes(),
            alice.recovery_code.as_ref().unwrap().as_bytes(),
            sticker.name.as_bytes(),
            KEY.as_slice(),
        ];
        for entry in fs::read_dir(&dir.0).unwrap() {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            for secret in secrets {
                assert!(
                    !bytes.windows(secret.len()).any(|window| window == secret),
                    "plaintext found on disk"
                );
            }
        }
        drop(store);
        let store = dir.open();
        assert_eq!(store.stickers(&alice.user_id).unwrap(), vec![sticker]);
        assert_eq!(store.history(&alice.user_id, 0).unwrap().0.len(), 2);
        assert!(store.login("alice", PASSWORD, "desktop").is_ok());
    }

    #[test]
    fn wrong_key_missing_marker_and_tampered_records_fail_closed() {
        let dir = TestDirectory::new();
        let store = dir.open();
        assert!(Store::open(&dir.path(), [8; 32]).is_err());
        let alice = register(&store, "alice");
        let bob = register(&store, "bob");
        store
            .send_message(&alice.user_id, "legacy", "first", text("private"))
            .unwrap();
        store
            .lock()
            .unwrap()
            .execute(
                "UPDATE cloud_messages SET recipient_id = ?1",
                params![bob.user_id],
            )
            .unwrap();
        assert!(store.history(&bob.user_id, 0).is_err());
        store
            .save_sticker(&alice.user_id, "sticker", media(ImageFormat::Png, 2, 2))
            .unwrap();
        store
            .lock()
            .unwrap()
            .execute(
                "UPDATE cloud_stickers SET user_id = ?1",
                params![bob.user_id],
            )
            .unwrap();
        assert!(store.stickers(&bob.user_id).is_err());
        store
            .lock()
            .unwrap()
            .execute("DELETE FROM cloud_meta", [])
            .unwrap();
        drop(store);
        assert!(Store::open(&dir.path(), KEY).is_err());
    }

    #[test]
    fn validates_real_media_and_isolates_sticker_quota() {
        let dir = TestDirectory::new();
        let store = dir.open();
        let alice = register(&store, "alice");
        let bob = register(&store, "bob");
        let png = media(ImageFormat::Png, 8, 8);
        let jpeg = media(ImageFormat::Jpeg, 8, 8);
        assert!(validate_image(&jpeg).is_ok());
        let mut invalid = Vec::new();
        let mut wrong = png.clone();
        wrong.width += 1;
        invalid.push(wrong);
        let mut wrong = png.clone();
        wrong.mime = "image/jpeg".into();
        invalid.push(wrong);
        let mut wrong = png.clone();
        wrong.mime = "image/gif".into();
        invalid.push(wrong);
        let mut wrong = png.clone();
        wrong.data_base64 = "not-base64!".into();
        invalid.push(wrong);
        let mut wrong = png.clone();
        wrong.data_base64 = BASE64.encode(b"not an image");
        invalid.push(wrong);
        let mut wrong = png.clone();
        wrong.data_base64 = BASE64.encode([0; MAX_MEDIA_BYTES + 1]);
        invalid.push(wrong);
        let mut wrong = png.clone();
        wrong.width = 0;
        invalid.push(wrong);
        let mut wrong = png.clone();
        wrong.height = 1025;
        invalid.push(wrong);
        let mut wrong = png.clone();
        let bytes = BASE64.decode(&wrong.data_base64).unwrap();
        wrong.data_base64 = BASE64.encode(&bytes[..bytes.len() / 2]);
        invalid.push(wrong);
        let mut wrong = media(ImageFormat::Png, 1025, 1);
        wrong.width = 1;
        invalid.push(wrong);
        for image in invalid {
            assert!(
                store
                    .save_sticker(&alice.user_id, "invalid", image.clone())
                    .is_err()
            );
            assert!(
                store
                    .send_message(
                        &alice.user_id,
                        &bob.user_id,
                        "invalid",
                        CloudBody::Image { image }
                    )
                    .is_err()
            );
        }
        assert!(store.stickers(&alice.user_id).unwrap().is_empty());
        assert!(store.history(&alice.user_id, 0).unwrap().0.is_empty());
        for index in 0..MAX_STICKERS {
            store
                .save_sticker(&alice.user_id, &format!("sticker {index}"), png.clone())
                .unwrap();
        }
        assert!(store.save_sticker(&alice.user_id, "overflow", png).is_err());
        assert_eq!(store.stickers(&alice.user_id).unwrap().len(), 20);
        assert!(store.stickers(&bob.user_id).unwrap().is_empty());
        store
            .save_sticker(&bob.user_id, "bob sticker", jpeg)
            .unwrap();
        assert_eq!(store.stickers(&bob.user_id).unwrap().len(), 1);
    }

    #[test]
    fn message_quota_rejects_both_participants_without_deleting_or_breaking_dedup() {
        let dir = TestDirectory::new();
        let store = dir.open();
        let alice = register(&store, "alice");
        let bob = register(&store, "bob");
        let first = store
            .send_message(&alice.user_id, "legacy", "first", text("hello"))
            .unwrap();
        {
            let mut connection = store.lock().unwrap();
            let tx = connection.transaction().unwrap();
            for index in 2..=MAX_MESSAGES {
                let mut item = first.clone();
                item.seq = index as u64;
                item.client_id = format!("seed-{index}");
                tx.execute(
                    "INSERT INTO cloud_messages(seq, sender_id, recipient_id, client_id, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![index, item.sender_id, item.recipient_id, item.client_id, encrypt(&store.channel, &item).unwrap()],
                ).unwrap();
            }
            tx.commit().unwrap();
        }
        assert!(
            store
                .send_message(
                    &alice.user_id,
                    &bob.user_id,
                    "overflow",
                    text("full sender")
                )
                .is_err()
        );
        assert!(
            store
                .send_message(
                    &bob.user_id,
                    &alice.user_id,
                    "overflow",
                    text("full recipient")
                )
                .is_err()
        );
        assert!(
            store
                .send_message(
                    &bob.user_id,
                    "legacy",
                    "overflow",
                    text("full legacy recipient")
                )
                .is_err()
        );
        assert_eq!(
            store
                .send_message(&alice.user_id, "legacy", "first", text("hello"))
                .unwrap(),
            first
        );
        assert_eq!(store.history(&alice.user_id, 0).unwrap().0[0], first);
        let count: i64 = store
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM cloud_messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, MAX_MESSAGES);
        let next = store
            .send_message(
                &bob.user_id,
                "other-legacy",
                "success",
                text("space remains"),
            )
            .unwrap();
        assert_eq!(next.seq, (MAX_MESSAGES + 1) as u64);
    }
}
