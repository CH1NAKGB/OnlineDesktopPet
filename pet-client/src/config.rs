use rand::Rng;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectionSettings {
    relay_url: String,
    access_token: String,
    #[serde(default)]
    discover_lan_relay: bool,
    #[serde(default)]
    server_rsa_public_key_path: Option<PathBuf>,
    #[serde(default)]
    tls_ca_cert_path: Option<PathBuf>,
}

impl ClientConfig {
    pub fn load() -> anyhow::Result<Self> {
        let explicit = std::env::var_os("CHICK_CONFIG_PATH").map(PathBuf::from);
        let portable = std::env::current_exe()?
            .parent()
            .map(|path| path.join("chicklink-connection.json"));
        let path = explicit
            .clone()
            .or_else(|| portable.filter(|path| path.is_file()))
            .or_else(|| {
                app_data_root()
                    .map(|root| root.join("connection.json"))
                    .filter(|path| path.is_file())
            });
        let mut config = Self::default();
        if let Some(path) = path {
            let data = fs::read_to_string(&path)
                .map_err(|_| anyhow::anyhow!("无法读取连接配置：{}", path.display()))?;
            // Do not include the original parser error: it may contain a secret token value.
            let settings: ConnectionSettings = serde_json::from_str(&data).map_err(|_| {
                anyhow::anyhow!("连接配置 JSON 无效，请检查字段和格式：{}", path.display())
            })?;
            validate_connection(&settings)?;
            if std::env::var_os("CHICK_RELAY_URL").is_none() {
                config.relay_url = settings.relay_url;
            }
            if std::env::var_os("CHICK_ACCESS_TOKEN").is_none() {
                config.access_token = settings.access_token;
            }
            config.discover_lan_relay = settings.discover_lan_relay;
            let base = path.parent().unwrap_or(std::path::Path::new("."));
            if std::env::var_os("CHICK_SERVER_RSA_PUBLIC_KEY_PATH").is_none() {
                config.server_rsa_public_key_path = settings
                    .server_rsa_public_key_path
                    .map(|path| base.join(path));
            }
            if std::env::var_os("CHICK_TLS_CA_CERT_PATH").is_none() {
                config.tls_ca_cert_path = settings.tls_ca_cert_path.map(|path| base.join(path));
            }
        }
        if let Ok(value) = std::env::var("CHICK_DISCOVER_LAN") {
            config.discover_lan_relay = match value.as_str() {
                "1" | "true" => true,
                "0" | "false" => false,
                _ => anyhow::bail!("CHICK_DISCOVER_LAN 只能为 true/false 或 1/0"),
            };
        }
        for path in [&config.server_rsa_public_key_path, &config.tls_ca_cert_path]
            .into_iter()
            .flatten()
        {
            anyhow::ensure!(
                path.is_file(),
                "无法找到连接所需的公钥或证书：{}",
                path.display()
            );
        }
        Ok(config)
    }
}

fn validate_connection(settings: &ConnectionSettings) -> anyhow::Result<()> {
    let url = url::Url::parse(&settings.relay_url).map_err(|_| anyhow::anyhow!("中继地址无效"))?;
    anyhow::ensure!(
        matches!(url.scheme(), "ws" | "wss")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none(),
        "中继地址必须为 ws:// 或 wss://，且不能包含登录凭据"
    );
    anyhow::ensure!(!settings.access_token.trim().is_empty(), "个人令牌不能为空");
    Ok(())
}

#[cfg(test)]
mod connection_tests {
    use super::*;
    #[test]
    fn portable_settings_validate_address_and_default_to_explicit_server() {
        let mut settings: ConnectionSettings = serde_json::from_str(
            r#"{"relay_url":"wss://pet.narendra.cn:18443/ws","access_token":"test"}"#,
        )
        .unwrap();
        assert!(!settings.discover_lan_relay);
        assert!(validate_connection(&settings).is_ok());
        settings.relay_url = "https://pet.narendra.cn".into();
        assert!(validate_connection(&settings).is_err());
        settings.relay_url = "ws://user:secret@localhost/ws".into();
        assert!(validate_connection(&settings).is_err());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    pub relay_url: String,
    pub access_token: String,
    pub device_id: String,
    /// The other owner's development identity. Pairing will populate this after login exists.
    pub partner_id: Option<String>,
    /// Generated locally and claimed at the relay. Ambiguous characters are omitted.
    pub pairing_code: String,
    /// User-facing nickname. Internal routing continues to use the authenticated user id.
    pub display_name: String,
    /// Prefer a relay announced on the current LAN before using `relay_url`.
    pub discover_lan_relay: bool,
    /// Optional PEM certificate or CA bundle trusted for `wss://` without a public CA.
    pub tls_ca_cert_path: Option<PathBuf>,
    /// PEM RSA public key pinned for encrypted `ws://` relay connections.
    pub server_rsa_public_key_path: Option<PathBuf>,
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
            display_name: std::env::var("CHICK_DISPLAY_NAME")
                .ok()
                .and_then(|name| shared_protocol::normalize_display_name(&name))
                .or_else(load_display_name)
                .unwrap_or_else(|| "小鸡主人".into()),
            discover_lan_relay: true,
            tls_ca_cert_path: std::env::var_os("CHICK_TLS_CA_CERT_PATH").map(PathBuf::from),
            server_rsa_public_key_path: std::env::var_os("CHICK_SERVER_RSA_PUBLIC_KEY_PATH")
                .map(PathBuf::from),
            share_active_app: false,
        }
    }
}

pub fn save_display_name(name: &str) -> Option<String> {
    let name = shared_protocol::normalize_display_name(name)?;
    let path = app_data_root()?.join("display-name.txt");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok()?;
    }
    fs::write(path, &name).ok()?;
    Some(name)
}

pub fn sounds_enabled() -> bool {
    app_data_root()
        .and_then(|root| fs::read_to_string(root.join("sounds-enabled.txt")).ok())
        .is_none_or(|value| value.trim() != "false")
}

pub fn save_sounds_enabled(enabled: bool) -> std::io::Result<()> {
    let root = app_data_root().ok_or_else(|| std::io::Error::other("无法确定设置目录"))?;
    fs::create_dir_all(&root)?;
    fs::write(
        root.join("sounds-enabled.txt"),
        if enabled { "true" } else { "false" },
    )
}

fn load_display_name() -> Option<String> {
    let name = fs::read_to_string(app_data_root()?.join("display-name.txt")).ok()?;
    shared_protocol::normalize_display_name(&name)
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
    Some(app_data_root()?.join("pairing-code.txt"))
}

fn app_data_root() -> Option<PathBuf> {
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
    root.map(|root| root.join("ChickLink"))
}
