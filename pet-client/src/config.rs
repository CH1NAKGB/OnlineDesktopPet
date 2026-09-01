use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    pub relay_url: String,
    pub access_token: String,
    pub device_id: String,
    /// The other owner's development identity. Pairing will populate this after login exists.
    pub partner_id: Option<String>,
    /// Prefer a relay announced on the current LAN before using `relay_url`.
    pub discover_lan_relay: bool,
    /// The setting is opt-in and is shown in the UI before any activity is sent.
    pub share_active_app: bool,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            relay_url: "ws://127.0.0.1:8080/ws".into(),
            access_token: "dev:chick-user".into(),
            device_id: "local-desktop".into(),
            partner_id: std::env::var("CHICK_PARTNER_ID").ok(),
            discover_lan_relay: true,
            share_active_app: false,
        }
    }
}
