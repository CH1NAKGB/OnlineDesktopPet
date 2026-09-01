use rand::Rng;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    pub relay_url: String,
    pub access_token: String,
    pub device_id: String,
    /// The other owner's development identity. Pairing will populate this after login exists.
    pub partner_id: Option<String>,
    /// Generated locally and claimed at the relay. Ambiguous characters are omitted.
    pub pairing_code: String,
    /// Prefer a relay announced on the current LAN before using `relay_url`.
    pub discover_lan_relay: bool,
    /// The setting is opt-in and is shown in the UI before any activity is sent.
    pub share_active_app: bool,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            relay_url: std::env::var("CHICK_RELAY_URL")
                .unwrap_or_else(|_| "ws://127.0.0.1:8080/ws".into()),
            access_token: std::env::var("CHICK_ACCESS_TOKEN")
                .unwrap_or_else(|_| "dev:chick-user".into()),
            device_id: std::env::var("CHICK_DEVICE_ID").unwrap_or_else(|_| "local-desktop".into()),
            partner_id: std::env::var("CHICK_PARTNER_ID").ok(),
            pairing_code: std::env::var("CHICK_PAIRING_CODE")
                .unwrap_or_else(|_| load_or_create_pairing_code()),
            discover_lan_relay: true,
            share_active_app: false,
        }
    }
}

fn generate_pairing_code() -> String {
    const ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
    let mut rng = rand::rng();
    (0..shared_protocol::PAIRING_CODE_LEN)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect()
}

fn load_or_create_pairing_code() -> String {
    let Some(path) = pairing_code_path() else {
        return generate_pairing_code();
    };
    if let Ok(code) = fs::read_to_string(&path) {
        let code = code.trim().to_ascii_uppercase();
        if shared_protocol::is_valid_pairing_code(&code) {
            return code;
        }
    }
    let code = generate_pairing_code();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, &code);
    code
}

fn pairing_code_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let root = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let root = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".config"))
        });
    root.map(|root| root.join("ChickLink/pairing-code.txt"))
}
