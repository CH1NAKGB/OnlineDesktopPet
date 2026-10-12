//! Versioned messages exchanged between the desktop client and the relay.

pub mod cloud;

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, OsRng, rand_core::RngCore},
};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use rsa::{Oaep, RsaPrivateKey, RsaPublicKey};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::Sha256;

pub const PROTOCOL_VERSION: u16 = 2;
pub const MAX_TEXT_CHARS: usize = 2_000;
pub const PAIRING_CODE_LEN: usize = 8;
pub const SESSION_KEY_LEN: usize = 32;
pub const MAX_DISPLAY_NAME_CHARS: usize = 24;

/// The unencrypted first message of an application-encrypted WebSocket session.
/// The RSA-encrypted session key is safe to send over `ws://` only when the
/// client obtained the server public key through a trusted, out-of-band channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
    SecureHandshake { encrypted_session_key: String },
    Encrypted { frame: EncryptedFrame },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
    Encrypted { frame: EncryptedFrame },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedFrame {
    pub nonce: String,
    pub ciphertext: String,
}

#[derive(Clone)]
pub struct SecureChannel {
    key: [u8; SESSION_KEY_LEN],
}

impl SecureChannel {
    pub fn new(key: [u8; SESSION_KEY_LEN]) -> Self {
        Self { key }
    }

    pub fn generate() -> (Self, [u8; SESSION_KEY_LEN]) {
        let mut key = [0; SESSION_KEY_LEN];
        OsRng.fill_bytes(&mut key);
        (Self::new(key), key)
    }

    pub fn encrypt<T: Serialize>(&self, value: &T) -> Result<EncryptedFrame, String> {
        let plaintext = serde_json::to_vec(value).map_err(|error| error.to_string())?;
        let mut nonce_bytes = [0; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let cipher = Aes256Gcm::new_from_slice(&self.key).map_err(|error| error.to_string())?;
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_ref())
            .map_err(|error| error.to_string())?;
        Ok(EncryptedFrame {
            nonce: BASE64.encode(nonce_bytes),
            ciphertext: BASE64.encode(ciphertext),
        })
    }

    pub fn decrypt<T: DeserializeOwned>(&self, frame: &EncryptedFrame) -> Result<T, String> {
        let nonce = BASE64
            .decode(&frame.nonce)
            .map_err(|error| error.to_string())?;
        let nonce: [u8; 12] = nonce
            .try_into()
            .map_err(|_| "encrypted frame nonce must be 12 bytes".to_owned())?;
        let ciphertext = BASE64
            .decode(&frame.ciphertext)
            .map_err(|error| error.to_string())?;
        let cipher = Aes256Gcm::new_from_slice(&self.key).map_err(|error| error.to_string())?;
        let plaintext = cipher
            .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
            .map_err(|_| "encrypted frame authentication failed".to_owned())?;
        serde_json::from_slice(&plaintext).map_err(|error| error.to_string())
    }
}

pub fn encrypt_session_key(
    public_key: &RsaPublicKey,
    session_key: &[u8; SESSION_KEY_LEN],
) -> Result<String, String> {
    let encrypted = public_key
        .encrypt(&mut OsRng, Oaep::new::<Sha256>(), session_key)
        .map_err(|error| error.to_string())?;
    Ok(BASE64.encode(encrypted))
}

pub fn decrypt_session_key(
    private_key: &RsaPrivateKey,
    encrypted_session_key: &str,
) -> Result<[u8; SESSION_KEY_LEN], String> {
    let encrypted = BASE64
        .decode(encrypted_session_key)
        .map_err(|error| error.to_string())?;
    let session_key = private_key
        .decrypt(Oaep::new::<Sha256>(), &encrypted)
        .map_err(|_| "RSA session-key decryption failed".to_owned())?;
    session_key
        .try_into()
        .map_err(|_| "invalid decrypted session-key length".to_owned())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientEvent {
    Cloud {
        request: cloud::CloudRequest,
    },
    Hello {
        protocol_version: u16,
        access_token: String,
        device_id: String,
        #[serde(default)]
        display_name: Option<String>,
    },
    UpdateProfile {
        display_name: String,
    },
    RegisterPairingCode {
        code: String,
    },
    PairWithCode {
        code: String,
    },
    RequestUnpair,
    RespondUnpair {
        accept: bool,
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
    Cloud {
        response: cloud::CloudResponse,
    },
    Ready {
        user_id: String,
    },
    PairingCodeRegistered {
        code: String,
    },
    Paired {
        partner_id: String,
        #[serde(default)]
        partner_display_name: Option<String>,
        /// Lets both clients explain whether this device submitted the code.
        #[serde(default)]
        initiated_by_you: bool,
    },
    UnpairPending {
        partner_id: String,
    },
    UnpairRequested {
        requester_id: String,
        #[serde(default)]
        requester_display_name: Option<String>,
    },
    UnpairCancelled {
        partner_id: String,
    },
    Unpaired {
        former_partner_id: String,
    },
    ProfileUpdated {
        user_id: String,
        display_name: String,
    },
    Presence {
        user_id: String,
        state: PresenceState,
        activity: Option<SharedActivity>,
    },
    DirectMessage {
        sender_id: String,
        #[serde(default)]
        sender_display_name: Option<String>,
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

pub fn is_valid_pairing_code(code: &str) -> bool {
    code.len() == PAIRING_CODE_LEN
        && code.bytes().all(|byte| {
            matches!(byte, b'2'..=b'9' | b'A'..=b'Z') && !matches!(byte, b'I' | b'L' | b'O')
        })
}

pub fn normalize_display_name(name: &str) -> Option<String> {
    let name = name.trim();
    let count = name.chars().count();
    (count > 0 && count <= MAX_DISPLAY_NAME_CHARS && !name.chars().any(char::is_control))
        .then(|| name.to_owned())
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

impl MessageBody {
    pub fn is_valid(&self) -> bool {
        match self {
            Self::Text { text } => {
                let chars = text.chars().count();
                chars > 0 && chars <= MAX_TEXT_CHARS
            }
            Self::Emote { code } => !code.is_empty() && code.len() <= 64,
            Self::Image { url, width, height } => {
                url.starts_with("https://")
                    && url.len() <= 2_048
                    && (1..=4_096).contains(width)
                    && (1..=4_096).contains(height)
            }
        }
    }
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

    #[test]
    fn rejects_empty_and_oversized_text_messages() {
        assert!(
            !MessageBody::Text {
                text: String::new()
            }
            .is_valid()
        );
        assert!(
            !MessageBody::Text {
                text: "小".repeat(MAX_TEXT_CHARS + 1)
            }
            .is_valid()
        );
        assert!(
            MessageBody::Text {
                text: "你好".into()
            }
            .is_valid()
        );
    }

    #[test]
    fn validates_unambiguous_pairing_codes() {
        assert!(is_valid_pairing_code("7KQ3M9XZ"));
        assert!(!is_valid_pairing_code("7KQ3MI9Z"));
        assert!(!is_valid_pairing_code("short"));
    }

    #[test]
    fn validates_and_trims_display_names() {
        assert_eq!(
            normalize_display_name("  小鸡主人  ").as_deref(),
            Some("小鸡主人")
        );
        assert!(normalize_display_name("").is_none());
        assert!(normalize_display_name(&"x".repeat(MAX_DISPLAY_NAME_CHARS + 1)).is_none());
    }

    #[test]
    fn old_paired_events_default_to_remote_initiation() {
        let event: ServerEvent = serde_json::from_str(
            r#"{"type":"paired","partner_id":"owner-b","partner_display_name":"小麦"}"#,
        )
        .expect("deserialize legacy paired event");
        assert!(matches!(
            event,
            ServerEvent::Paired {
                partner_id,
                partner_display_name: Some(name),
                initiated_by_you: false,
            } if partner_id == "owner-b" && name == "小麦"
        ));
    }

    #[test]
    fn unpair_confirmation_events_round_trip_over_json() {
        let request = serde_json::to_string(&ClientEvent::RespondUnpair { accept: true })
            .expect("serialize unpair response");
        assert!(matches!(
            serde_json::from_str::<ClientEvent>(&request).expect("deserialize unpair response"),
            ClientEvent::RespondUnpair { accept: true }
        ));

        let pending = ServerEvent::UnpairRequested {
            requester_id: "owner-a".into(),
            requester_display_name: Some("小麦".into()),
        };
        let json = serde_json::to_string(&pending).expect("serialize unpair request");
        assert!(matches!(
            serde_json::from_str::<ServerEvent>(&json).expect("deserialize unpair request"),
            ServerEvent::UnpairRequested {
                requester_id,
                requester_display_name: Some(name),
            } if requester_id == "owner-a" && name == "小麦"
        ));
    }

    #[test]
    fn secure_channel_rejects_tampered_ciphertext() {
        let (channel, _) = SecureChannel::generate();
        let mut frame = channel.encrypt(&ClientEvent::Ping).expect("encrypt event");
        frame.ciphertext.push('A');
        assert!(channel.decrypt::<ClientEvent>(&frame).is_err());
    }

    #[test]
    fn rsa_protects_the_session_key() {
        let private_key = RsaPrivateKey::new(&mut OsRng, 2048).expect("generate key");
        let (_, session_key) = SecureChannel::generate();
        let encrypted = encrypt_session_key(&RsaPublicKey::from(&private_key), &session_key)
            .expect("encrypt session key");
        assert_eq!(
            decrypt_session_key(&private_key, &encrypted).expect("decrypt key"),
            session_key
        );
    }
}
