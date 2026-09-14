use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, io::Write, path::PathBuf};

#[derive(Clone)]
pub struct Store(Option<PathBuf>);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u32,
    partners: HashMap<String, String>,
}

fn validate(partners: &HashMap<String, String>) -> anyhow::Result<()> {
    for (owner, peer) in partners {
        anyhow::ensure!(
            !owner.is_empty() && owner != peer && partners.get(peer) == Some(owner),
            "invalid non-reciprocal pairing state"
        );
    }
    Ok(())
}

impl Store {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self(path)
    }

    pub fn load(&self) -> anyhow::Result<HashMap<String, String>> {
        let Some(path) = &self.0 else {
            return Ok(HashMap::new());
        };
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
            Err(error) => return Err(error).context("read pairing state"),
        };
        let snapshot: Snapshot = serde_json::from_slice(&bytes).context("parse pairing state")?;
        anyhow::ensure!(snapshot.version == 1, "unsupported pairing state version");
        validate(&snapshot.partners)?;
        Ok(snapshot.partners)
    }

    // Caller holds the partners write lock until save succeeds, then publishes the new map.
    pub async fn save(&self, partners: &HashMap<String, String>) -> anyhow::Result<()> {
        let Some(path) = self.0.clone() else {
            return Ok(());
        };
        validate(partners)?;
        let bytes = serde_json::to_vec(&Snapshot {
            version: 1,
            partners: partners.clone(),
        })?;
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("."));
            fs::create_dir_all(parent)?;
            let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&temporary)
                .context("create temporary pairing state")?;
            let result = (|| -> anyhow::Result<()> {
                file.write_all(&bytes)?;
                file.sync_all()?;
                fs::rename(&temporary, &path)?;
                #[cfg(unix)]
                if let Err(error) =
                    fs::File::open(parent).and_then(|directory| directory.sync_all())
                {
                    // Rename has committed: keep memory consistent with the published file.
                    tracing::error!(%error, "pairing state committed but directory sync failed");
                }
                Ok(())
            })();
            if result.is_err() {
                let _ = fs::remove_file(&temporary);
            }
            result
        })
        .await??;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory() -> PathBuf {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "chicklink-state-{}-{stamp}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[tokio::test]
    async fn pairing_and_unpairing_survive_reload() {
        let dir = directory();
        let path = dir.join("state.json");
        let store = Store::new(Some(path.clone()));
        assert!(store.load().unwrap().is_empty());
        let partners = HashMap::from([("a".into(), "b".into()), ("b".into(), "a".into())]);
        store.save(&partners).await.unwrap();
        assert_eq!(Store::new(Some(path)).load().unwrap(), partners);
        store.save(&HashMap::new()).await.unwrap();
        assert!(store.load().unwrap().is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn malformed_state_and_failed_write_do_not_reset_saved_pairing() {
        let dir = directory();
        let path = dir.join("state.json");
        let store = Store::new(Some(path.clone()));
        let partners = HashMap::from([("a".into(), "b".into()), ("b".into(), "a".into())]);
        store.save(&partners).await.unwrap();
        assert!(
            store
                .save(&HashMap::from([("a".into(), "a".into())]))
                .await
                .is_err()
        );
        fs::write(
            path.with_extension(format!("{}.tmp", std::process::id())),
            "occupied",
        )
        .unwrap();
        assert!(store.save(&HashMap::new()).await.is_err());
        assert_eq!(store.load().unwrap(), partners);
        fs::write(&path, "broken").unwrap();
        assert!(store.load().is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
