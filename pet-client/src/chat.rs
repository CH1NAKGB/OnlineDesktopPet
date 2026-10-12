use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use shared_protocol::cloud::{CloudBody, CloudMessage};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatEntry {
    pub outgoing: bool,
    pub sender: String,
    pub text: String,
    pub unix_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_message: Option<CloudMessage>,
}

impl ChatEntry {
    pub fn new(outgoing: bool, sender: String, text: String) -> Self {
        Self {
            outgoing,
            sender,
            text,
            unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            account_id: None,
            seq: None,
            cloud_message: None,
        }
    }

    pub fn from_cloud(account_id: &str, item: &CloudMessage) -> Self {
        Self {
            outgoing: item.sender_id == account_id,
            sender: item.sender_id.clone(),
            text: match &item.body {
                CloudBody::Text { text } => text.clone(),
                CloudBody::Image { .. } => String::new(),
            },
            unix_ms: item.unix_ms,
            account_id: Some(account_id.to_owned()),
            seq: Some(item.seq),
            cloud_message: Some(item.clone()),
        }
    }
}

pub fn load_history() -> Vec<ChatEntry> {
    let Some(path) = history_path() else {
        return Vec::new();
    };
    let Ok(contents) = fs::read_to_string(path) else {
        return Vec::new();
    };
    contents
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

pub fn append_history(entry: &ChatEntry) {
    // Cloud data belongs exclusively in the account-scoped store.
    if entry.account_id.is_some() || entry.cloud_message.is_some() {
        return;
    }
    let Some(path) = history_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    if let Ok(json) = serde_json::to_string(entry) {
        let _ = writeln!(file, "{json}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_entries_still_deserialize() {
        let entry: ChatEntry = serde_json::from_str(
            r#"{"outgoing":false,"sender":"friend","text":"hello","unix_ms":42}"#,
        )
        .unwrap();
        assert!(entry.account_id.is_none());
        assert!(entry.seq.is_none());
        assert!(entry.cloud_message.is_none());
    }
}

fn history_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let root = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let root = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let root = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".local/share"))
        });
    root.map(|root| root.join("ChickLink/chat-history.jsonl"))
}
