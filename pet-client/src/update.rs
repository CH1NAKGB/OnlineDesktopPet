//! Release discovery. The HTTP request runs on a dedicated thread.

use std::{
    fs,
    io::{self, Read, Write},
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const DEFAULT_URL: &str =
    "https://api.github.com/repos/CH1NAKGB/OnlineDesktopPet/releases/latest";
const CHECK_INTERVAL_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub page_url: String,
    pub asset: Option<Asset>,
    pub checksum: Option<Asset>,
}

#[derive(Clone, Debug)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub digest: Option<String>,
}

#[derive(Deserialize)]
struct Manifest {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<ManifestAsset>,
}

#[derive(Deserialize)]
struct ManifestAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

fn parse_version(value: &str) -> Option<[u64; 3]> {
    let mut parts = value.trim().trim_start_matches('v').split('.');
    let version = [
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ];
    parts.next().is_none().then_some(version)
}

fn select_asset(assets: &[ManifestAsset], version: &str, platform: &str) -> Option<Asset> {
    let names: [String; 2] = match platform {
        "windows" => [
            format!("ChickLink-Setup-v{version}.exe"),
            format!("ChickLink-windows-x64-v{version}.zip"),
        ],
        "macos" => [
            format!("ChickLink-macos-universal-v{version}.zip"),
            format!("ChickLink-macos-universal-v{version}.dmg"),
        ],
        _ => return None,
    };
    names.iter().find_map(|name| {
        assets
            .iter()
            .find(|asset| &asset.name == name)
            .map(|asset| Asset {
                name: asset.name.clone(),
                url: asset.browser_download_url.clone(),
                digest: asset.digest.clone(),
            })
    })
}

fn parse_manifest(body: &str, platform: &str) -> Result<Option<Release>, String> {
    let manifest: Manifest =
        serde_json::from_str(body).map_err(|_| "更新清单格式无效".to_string())?;
    if manifest.draft || manifest.prerelease {
        return Ok(None);
    }
    let remote = parse_version(&manifest.tag_name).ok_or("更新版本号无效")?;
    let local = parse_version(env!("CARGO_PKG_VERSION")).ok_or("当前版本号无效")?;
    if remote <= local {
        return Ok(None);
    }
    let version = format!("{}.{}.{}", remote[0], remote[1], remote[2]);
    if !manifest.html_url.starts_with("https://")
        && !manifest.html_url.starts_with("http://localhost")
        && !manifest.html_url.starts_with("http://127.0.0.1:")
    {
        return Err("更新发布页地址无效".into());
    }
    Ok(Some(Release {
        asset: select_asset(&manifest.assets, &version, platform),
        checksum: manifest
            .assets
            .iter()
            .find(|asset| asset.name == format!("SHA256SUMS-v{version}.txt"))
            .map(|asset| Asset {
                name: asset.name.clone(),
                url: asset.browser_download_url.clone(),
                digest: asset.digest.clone(),
            }),
        page_url: manifest.html_url,
        version,
    }))
}

pub fn check(url: &str) -> Result<Option<Release>, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .build();
    let response = agent
        .get(url)
        .set(
            "User-Agent",
            &format!("ChickLink/{}", env!("CARGO_PKG_VERSION")),
        )
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|_| "检查更新失败".to_string())?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取更新清单".to_string())?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("更新清单过大".into());
    }
    let body = String::from_utf8(bytes).map_err(|_| "更新清单格式无效")?;
    parse_manifest(&body, std::env::consts::OS)
}

fn allowed_download_url(url: &str, local_test: bool) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return local_test
            && (url.starts_with("http://localhost:") || url.starts_with("http://127.0.0.1:"));
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    matches!(
        host,
        "api.github.com"
            | "github.com"
            | "objects.githubusercontent.com"
            | "release-assets.githubusercontent.com"
    )
}

fn fetch_asset(
    asset: &Asset,
    local_test: bool,
    limit: u64,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<Vec<u8>, String> {
    if !allowed_download_url(&asset.url, local_test) {
        return Err("更新附件地址不受信任".into());
    }
    let response = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(300))
        .build()
        .get(&asset.url)
        .set(
            "User-Agent",
            &format!("ChickLink/{}", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .map_err(|_| "下载更新失败")?;
    if !allowed_download_url(response.get_url(), local_test) {
        return Err("更新附件跳转到不受信任地址".into());
    }
    let total = response
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok());
    if total.is_some_and(|n| n > limit) {
        return Err("更新附件过大".into());
    }
    let mut reader = response.into_reader();
    let mut bytes = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|_| "下载更新失败")?;
        if n == 0 {
            break;
        }
        if bytes.len() as u64 + n as u64 > limit {
            return Err("更新附件过大".into());
        }
        bytes.extend_from_slice(&buf[..n]);
        progress(bytes.len() as u64, total);
    }
    Ok(bytes)
}

fn expected_hash(release: &Release, asset: &Asset, local_test: bool) -> Result<String, String> {
    if let Some(digest) = asset
        .digest
        .as_deref()
        .and_then(|s| s.strip_prefix("sha256:"))
    {
        if digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(digest.to_ascii_lowercase());
        }
        return Err("附件摘要无效".into());
    }
    let sums = release.checksum.as_ref().ok_or("缺少 SHA-256 校验文件")?;
    let bytes = fetch_asset(sums, local_test, 1024 * 1024, |_, _| {})?;
    let body = String::from_utf8(bytes).map_err(|_| "校验文件格式无效")?;
    body.lines()
        .find_map(|line| {
            let (hash, name) = line.split_once(char::is_whitespace)?;
            (name.trim().trim_start_matches('*') == asset.name
                && hash.len() == 64
                && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
        })
        .ok_or_else(|| "校验文件缺少更新附件".into())
}

pub fn download_verified(
    release: &Release,
    source_url: &str,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf, String> {
    let asset = release.asset.as_ref().ok_or("没有可下载的更新附件")?;
    let local_test =
        source_url.starts_with("http://localhost:") || source_url.starts_with("http://127.0.0.1:");
    let expected = expected_hash(release, asset, local_test)?;
    let dir = std::env::temp_dir().join(format!("chicklink-update-{:016x}", rand::random::<u64>()));
    fs::create_dir(&dir).map_err(|_| "无法创建更新临时目录")?;
    let path = dir.join(&asset.name);
    let result = (|| -> Result<(), String> {
        if !allowed_download_url(&asset.url, local_test) {
            return Err("更新附件地址不受信任".into());
        }
        let response = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(300))
            .build()
            .get(&asset.url)
            .set(
                "User-Agent",
                &format!("ChickLink/{}", env!("CARGO_PKG_VERSION")),
            )
            .call()
            .map_err(|_| "下载更新失败")?;
        if !allowed_download_url(response.get_url(), local_test) {
            return Err("更新附件跳转到不受信任地址".into());
        }
        let total = response
            .header("Content-Length")
            .and_then(|s| s.parse::<u64>().ok());
        let limit = 250 * 1024 * 1024;
        if total.is_some_and(|n| n > limit) {
            return Err("更新附件过大".into());
        }
        let mut reader = response.into_reader();
        let mut file = fs::File::create(&path).map_err(|_| "无法保存更新文件")?;
        let mut hasher = Sha256::new();
        let mut done = 0u64;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = reader.read(&mut buf).map_err(|_| "下载更新失败")?;
            if n == 0 {
                break;
            }
            done += n as u64;
            if done > limit {
                return Err("更新附件过大".into());
            }
            file.write_all(&buf[..n]).map_err(|_| "无法保存更新文件")?;
            hasher.update(&buf[..n]);
            progress(done, total);
        }
        if format!("{:x}", hasher.finalize()) != expected {
            return Err("更新文件 SHA-256 校验失败".into());
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&dir);
        return Err(error);
    }
    Ok(path)
}

fn settings_root() -> Option<std::path::PathBuf> {
    crate::config::app_data_root()
}

fn read_setting(name: &str) -> Option<String> {
    fs::read_to_string(settings_root()?.join(name))
        .ok()
        .map(|s| s.trim().to_owned())
}

fn write_setting(name: &str, value: &str) -> io::Result<()> {
    let root = settings_root().ok_or_else(|| io::Error::other("无法确定设置目录"))?;
    fs::create_dir_all(&root)?;
    fs::write(root.join(name), value)
}

pub fn enabled() -> bool {
    read_setting("update-check-enabled.txt").is_none_or(|value| value != "false")
}

pub fn set_enabled(value: bool) -> io::Result<()> {
    write_setting(
        "update-check-enabled.txt",
        if value { "true" } else { "false" },
    )
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn auto_check_due() -> bool {
    if !enabled() {
        return false;
    }
    due_since(
        read_setting("update-last-check.txt").and_then(|value| value.parse::<u64>().ok()),
        now_ms(),
    )
}

fn due_since(last: Option<u64>, now: u64) -> bool {
    last.is_none_or(|last| now.saturating_sub(last) >= CHECK_INTERVAL_MS)
}

pub fn mark_checked() {
    let _ = write_setting("update-last-check.txt", &now_ms().to_string());
}

pub fn remember_latest(version: &str) {
    let _ = write_setting("update-latest-version.txt", version);
}

pub fn should_notify(version: &str) -> bool {
    read_setting("update-notified-version.txt").as_deref() != Some(version)
}

pub fn mark_notified(version: &str) {
    let _ = write_setting("update-notified-version.txt", version);
}

pub fn open_page(url: &str) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        let wide: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
        let operation: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
        let code = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                wide.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            )
        } as isize;
        if code <= 32 {
            return Err(io::Error::other("无法打开浏览器"));
        }
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        std::process::Command::new(opener)
            .arg(url)
            .spawn()?
            .wait()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::TcpListener, thread};

    #[test]
    fn version_comparison_and_draft_filter() {
        assert!(parse_version("v0.4.0") > parse_version("0.3.4"));
        assert_eq!(parse_version("0.4.0-rc.1"), None);
        let body = r#"{"tag_name":"v0.3.4","html_url":"https://github.com/example","assets":[]}"#;
        assert!(parse_manifest(body, "macos").unwrap().is_none());
        let draft = body.replace("\"assets\"", "\"draft\":true,\"assets\"");
        assert!(parse_manifest(&draft, "macos").unwrap().is_none());
    }

    #[test]
    fn platform_asset_preference() {
        let assets = [
            ManifestAsset {
                name: "ChickLink-windows-x64-v0.4.0.zip".into(),
                browser_download_url: "https://example/zip".into(),
                digest: None,
            },
            ManifestAsset {
                name: "ChickLink-Setup-v0.4.0.exe".into(),
                browser_download_url: "https://example/setup".into(),
                digest: None,
            },
            ManifestAsset {
                name: "ChickLink-macos-universal-v0.4.0.dmg".into(),
                browser_download_url: "https://example/dmg".into(),
                digest: None,
            },
            ManifestAsset {
                name: "ChickLink-macos-universal-v0.4.0.zip".into(),
                browser_download_url: "https://example/maczip".into(),
                digest: None,
            },
        ];
        assert_eq!(
            select_asset(&assets, "0.4.0", "windows").unwrap().name,
            "ChickLink-Setup-v0.4.0.exe"
        );
        assert_eq!(
            select_asset(&assets, "0.4.0", "macos").unwrap().name,
            "ChickLink-macos-universal-v0.4.0.zip"
        );
    }

    #[test]
    fn throttle_and_trusted_hosts() {
        assert!(!due_since(Some(100), 100 + CHECK_INTERVAL_MS - 1));
        assert!(due_since(Some(100), 100 + CHECK_INTERVAL_MS));
        assert!(due_since(None, 100));
        assert!(allowed_download_url(
            "https://release-assets.githubusercontent.com/a",
            false
        ));
        assert!(!allowed_download_url(
            "https://github.com.evil.test/a",
            false
        ));
        assert!(!allowed_download_url("http://127.0.0.1:12/a", false));
    }

    #[test]
    fn local_manifest_download_and_checksum_rejection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let payload = b"fake application archive";
        let digest = format!("{:x}", Sha256::digest(payload));
        let manifest = format!(
            r#"{{"tag_name":"v9.9.9","html_url":"{base}/release","assets":[{{"name":"ChickLink-macos-universal-v9.9.9.zip","browser_download_url":"{base}/good","digest":"sha256:{digest}"}},{{"name":"ChickLink-windows-x64-v9.9.9.zip","browser_download_url":"{base}/good","digest":"sha256:{digest}"}}]}}"#
        );
        let server = thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 2048];
                let n = stream.read(&mut request).unwrap();
                let path = String::from_utf8_lossy(&request[..n]);
                assert!(path.contains("User-Agent: ChickLink/"));
                assert!(!path.contains("Authorization:") && !path.contains("access_token"));
                let body: &[u8] = if path.starts_with("GET /manifest ") {
                    manifest.as_bytes()
                } else {
                    payload
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(body).unwrap();
            }
        });
        let source = format!("{base}/manifest");
        let release = check(&source).unwrap().unwrap();
        let path = download_verified(&release, &source, |_, _| {}).unwrap();
        assert_eq!(fs::read(&path).unwrap(), payload);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
        let mut bad = release.clone();
        bad.asset.as_mut().unwrap().digest = Some(format!("sha256:{}", "0".repeat(64)));
        assert!(
            download_verified(&bad, &source, |_, _| {})
                .unwrap_err()
                .contains("校验失败")
        );
        server.join().unwrap();
    }
}
