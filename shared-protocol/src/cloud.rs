//! Account and encrypted cloud-storage wire types.

use serde::{Deserialize, Serialize};

/// Contains credentials: never log requests, including their `Debug` output.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CloudRequest {
    Register {
        username: String,
        password: String,
        device_id: String,
        legacy_token: Option<String>,
    },
    Login {
        username: String,
        password: String,
        device_id: String,
    },
    Recover {
        username: String,
        recovery_code: String,
        new_password: String,
        device_id: String,
    },
    Resume {
        token: String,
        device_id: String,
    },
    Logout,
    Sync {
        after: u64,
    },
    SendMessage {
        client_id: String,
        recipient_id: String,
        body: CloudBody,
    },
    PutSticker {
        name: String,
        image: ImageData,
    },
    ListStickers,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CloudResponse {
    Authenticated {
        user_id: String,
        username: String,
        token: String,
        recovery_code: Option<String>,
    },
    LoggedOut,
    History {
        items: Vec<CloudMessage>,
        next_cursor: u64,
        has_more: bool,
    },
    Message {
        item: CloudMessage,
    },
    SendRejected {
        client_id: String,
        message: String,
        retryable: bool,
    },
    Stickers {
        items: Vec<CloudSticker>,
    },
    StickerSaved {
        item: CloudSticker,
    },
    Error {
        message: String,
    },
}

/// The recovery code is returned only on registration or password recovery.
/// Like requests, authentication results must not be logged.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthResult {
    pub user_id: String,
    pub username: String,
    pub token: String,
    pub recovery_code: Option<String>,
}

impl From<AuthResult> for CloudResponse {
    fn from(auth: AuthResult) -> Self {
        Self::Authenticated {
            user_id: auth.user_id,
            username: auth.username,
            token: auth.token,
            recovery_code: auth.recovery_code,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageData {
    pub mime: String,
    pub data_base64: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CloudBody {
    Text { text: String },
    Image { image: ImageData },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CloudMessage {
    pub seq: u64,
    pub client_id: String,
    pub sender_id: String,
    pub recipient_id: String,
    pub body: CloudBody,
    pub unix_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CloudSticker {
    pub id: String,
    pub name: String,
    pub image: ImageData,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_and_authentication_wire_shapes() {
        let request: CloudRequest = serde_json::from_str(
            r#"{"type":"register","username":"alice","password":"long password","device_id":"phone"}"#,
        )
        .unwrap();
        assert!(matches!(
            request,
            CloudRequest::Register {
                legacy_token: None,
                ..
            }
        ));
        let response = CloudResponse::from(AuthResult {
            user_id: "owner".into(),
            username: "alice".into(),
            token: "secret".into(),
            recovery_code: None,
        });
        let json = serde_json::to_value(response).unwrap();
        assert_eq!(json["type"], "authenticated");
        assert!(json["recovery_code"].is_null());
        let body = CloudBody::Text {
            text: "hello".into(),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["kind"], "text");
        assert_eq!(serde_json::from_value::<CloudBody>(json).unwrap(), body);
    }
}
