//! Versioned messages exchanged between the desktop client and the relay.

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientEvent {
    Hello {
        protocol_version: u16,
        access_token: String,
        device_id: String,
    },
    Presence {
        state: PresenceState,
        /// Sent only when the local owner explicitly enables activity sharing.
        activity: Option<SharedActivity>,
    },
    DirectMessage {
        recipient_id: String,
        body: MessageBody,
    },
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    Ready {
        user_id: String,
    },
    Presence {
        user_id: String,
        state: PresenceState,
        activity: Option<SharedActivity>,
    },
    DirectMessage {
        sender_id: String,
        body: MessageBody,
    },
    Error {
        code: String,
        message: String,
    },
    Pong,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceState {
    Online,
    Away,
    Offline,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedActivity {
    /// A human-readable app label only. Never send process arguments, window title, or document path.
    pub app_label: String,
    pub updated_at_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MessageBody {
    Text {
        text: String,
    },
    Emote {
        code: String,
    },
    Image {
        url: String,
        width: u32,
        height: u32,
    },
}
