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
    /// A small state change applied to the pet shared by a paired couple.
    SharedPetAction {
        recipient_id: String,
        action: SharedPetAction,
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
    SharedPetAction {
        sender_id: String,
        action: SharedPetAction,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SharedPetAction {
    Pet,
    SetSkin { skin: PetSkin },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PetSkin {
    Sunshine,
    Blueberry,
    Cocoa,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_pet_action_round_trips_over_json() {
        let event = ClientEvent::SharedPetAction {
            recipient_id: "other-owner".into(),
            action: SharedPetAction::SetSkin {
                skin: PetSkin::Blueberry,
            },
        };
        let json = serde_json::to_string(&event).expect("serialize shared pet action");
        let decoded: ClientEvent =
            serde_json::from_str(&json).expect("deserialize shared pet action");

        assert!(matches!(
            decoded,
            ClientEvent::SharedPetAction {
                recipient_id,
                action: SharedPetAction::SetSkin {
                    skin: PetSkin::Blueberry
                }
            } if recipient_id == "other-owner"
        ));
    }
}
