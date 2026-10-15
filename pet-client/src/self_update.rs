//! Install a verified release after the user explicitly chooses to update.

#[cfg(target_os = "macos")]
use std::{fs, path::PathBuf, process::Stdio};
use std::{path::Path, process::Command};

#[cfg(target_os = "macos")]
const HELPER: &str = r#"#!/bin/sh
set -u
old=$1
new=$2
pid=$3
backup="$old.bak"
marker=$4
opener=$5
copier=$6
remover=$7
candidate="$old.installing.$$"
committed=0
replacement_started=0
marker_written=0
log() { printf 'Chick Link update: %s\n' "$*" >&2; }
finish() {
  status=$1
  trap - EXIT INT TERM
  if [ "$committed" -eq 1 ]; then exit 0; fi
  log "install stopped (status $status); restoring previous application"
  "$remover" -rf -- "$candidate" || log "could not remove incomplete candidate: $candidate"
  if [ "$marker_written" -eq 1 ]; then
    rm -f -- "$marker" || log "could not remove update marker: $marker"
  fi
  if [ -e "$backup" ]; then
    if [ -e "$old" ] && [ "$replacement_started" -eq 1 ]; then
      if ! "$remover" -rf -- "$old" || [ -e "$old" ]; then
        # Never move a backup into an existing directory: mv would nest it.
        quarantine="$old.failed.$$"
        if ! mv "$old" "$quarantine"; then
          log "CRITICAL: cannot clear damaged app; intact backup remains at $backup"
        fi
      fi
    fi
    if [ ! -e "$old" ]; then
      mv "$backup" "$old" || log "CRITICAL: could not restore backup $backup"
    fi
  fi
  if [ -e "$old" ]; then
    "$opener" "$old" || log "could not reopen application: $old"
  else
    log "CRITICAL: no application at $old; inspect backup $backup"
  fi
  exit "$status"
}
trap 'finish $?' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
i=0
while kill -0 "$pid" 2>/dev/null; do
  [ "$i" -lt 60 ] || { log "old process did not exit within 30 seconds"; exit 1; }
  sleep 0.5
  i=$((i + 1))
done
[ -e "$old" ] || { log "installed application is missing"; exit 1; }
[ ! -e "$candidate" ] || { log "candidate path already exists"; exit 1; }
# Copy beside the live app. A signal or disk error cannot leave a partial $old.
"$copier" "$new" "$candidate" || { log "candidate copy failed"; exit 1; }
if [ -e "$backup" ]; then
  # A previous update is still awaiting the ten-second verification window.
  # Keep its single known-good backup; replace only the unverified current app.
  replacement_started=1
  "$remover" -rf -- "$old" && [ ! -e "$old" ] || { log "could not clear current app"; exit 1; }
else
  mv "$old" "$backup" || { log "could not create backup"; exit 1; }
  replacement_started=1
fi
mv "$candidate" "$old" || { log "could not activate candidate"; exit 1; }
printf '%s' "$old" > "$marker" || { log "could not write update marker"; exit 1; }
marker_written=1
"$opener" "$old" || { log "could not launch updated app"; exit 1; }
committed=1
exit 0
"#;

#[cfg(target_os = "macos")]
fn current_bundle() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|_| "无法定位当前应用")?;
    let macos = exe.parent().ok_or("无法定位当前应用")?;
    if macos.file_name().and_then(|s| s.to_str()) != Some("MacOS")
        || macos
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            != Some("Contents")
    {
        return Err("当前程序不是应用包，请打开发布页安装".into());
    }
    let bundle = macos
        .parent()
        .and_then(Path::parent)
        .ok_or("无法定位应用包")?;
    if bundle.extension().and_then(|s| s.to_str()) != Some("app") || bundle.starts_with("/Volumes/")
    {
        return Err("当前应用位置不能自更新，请打开发布页安装".into());
    }
    let parent = bundle.parent().ok_or("无法定位应用目录")?;
    let probe = parent.join(format!(".chicklink-write-{:x}", rand::random::<u64>()));
    fs::File::create(&probe).map_err(|_| "应用目录不可写，请打开发布页安装")?;
    let _ = fs::remove_file(probe);
    Ok(bundle.to_path_buf())
}

#[cfg(target_os = "macos")]
pub fn preflight() -> Result<(), String> {
    let bundle = current_bundle()?;
    if PathBuf::from(format!("{}.bak", bundle.display())).exists() {
        return Err("发现尚未清理的更新备份，请打开发布页安装".into());
    }
    Ok(())
}
#[cfg(not(target_os = "macos"))]
pub fn preflight() -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn install(archive: &Path) -> Result<(), String> {
    let bundle = current_bundle()?;
    let dir = archive.parent().ok_or("更新临时目录无效")?;
    let unpacked = dir.join("unpacked");
    fs::create_dir(&unpacked).map_err(|_| "无法解压更新")?;
    let status = Command::new("/usr/bin/ditto")
        .args(["-x", "-k"])
        .arg(archive)
        .arg(&unpacked)
        .status()
        .map_err(|_| "无法解压更新")?;
    if !status.success() {
        return Err("更新包解压失败".into());
    }
    let next = unpacked.join("Chick Link.app");
    if !next.join("Contents/MacOS").is_dir() {
        return Err("更新包缺少应用程序".into());
    }
    let status = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(&next)
        .status()
        .map_err(|_| "无法验证应用签名")?;
    if !status.success() {
        return Err("更新包签名校验失败".into());
    }
    let root = crate::config::app_data_root().ok_or("无法确定应用数据目录")?;
    fs::create_dir_all(&root).map_err(|_| "无法准备更新日志")?;
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("update.log"))
        .map_err(|_| "无法准备更新日志")?;
    let err = log.try_clone().map_err(|_| "无法准备更新日志")?;
    let helper = dir.join("install.sh");
    fs::write(&helper, HELPER).map_err(|_| "无法准备安装程序")?;
    use std::os::unix::process::CommandExt;
    let mut command = Command::new("/bin/sh");
    // A terminal or parent process-group signal must not interrupt the copy.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    command
        .arg(&helper)
        .arg(&bundle)
        .arg(&next)
        .arg(std::process::id().to_string())
        .arg(root.join("update-pending.txt"))
        .arg("/usr/bin/open")
        .arg("/usr/bin/ditto")
        .arg("/bin/rm")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err))
        .spawn()
        .map_err(|_| "无法启动安装程序")?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn clean_successful_backup() {
    if let Ok(bundle) = current_bundle() {
        if let Some(root) = crate::config::app_data_root() {
            let marker = root.join("update-pending.txt");
            if fs::read_to_string(&marker).ok().as_deref() == bundle.to_str() {
                let backup = PathBuf::from(format!("{}.bak", bundle.display()));
                if backup.is_dir() && fs::remove_dir_all(backup).is_ok() {
                    let _ = fs::remove_file(marker);
                }
            }
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn failed_replace_restores_old_bundle() {
        let root =
            std::env::temp_dir().join(format!("chicklink-rollback-{:x}", rand::random::<u64>()));
        let old = root.join("Chick Link.app");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("marker"), "old").unwrap();
        let script = root.join("install.sh");
        fs::write(&script, HELPER).unwrap();
        let output = Command::new("/bin/sh")
            .arg(&script)
            .arg(&old)
            .arg(root.join("missing.app"))
            .arg("9999999")
            .arg(root.join("marker.txt"))
            .arg("/usr/bin/true")
            .arg("/usr/bin/ditto")
            .arg("/bin/rm")
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(fs::read_to_string(old.join("marker")).unwrap(), "old");
        assert!(!root.join("Chick Link.app.bak").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn successful_replace_keeps_backup_and_records_marker() {
        let root =
            std::env::temp_dir().join(format!("chicklink-replace-{:x}", rand::random::<u64>()));
        let old = root.join("Chick Link.app");
        let next = root.join("next.app");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&next).unwrap();
        fs::write(old.join("marker"), "old").unwrap();
        fs::write(next.join("marker"), "new").unwrap();
        let script = root.join("install.sh");
        let marker = root.join("pending.txt");
        fs::write(&script, HELPER).unwrap();
        let output = Command::new("/bin/sh")
            .arg(&script)
            .arg(&old)
            .arg(&next)
            .arg("9999999")
            .arg(&marker)
            .arg("/usr/bin/true")
            .arg("/usr/bin/ditto")
            .arg("/bin/rm")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(fs::read_to_string(old.join("marker")).unwrap(), "new");
        assert_eq!(
            fs::read_to_string(root.join("Chick Link.app.bak/marker")).unwrap(),
            "old"
        );
        assert_eq!(fs::read_to_string(marker).unwrap(), old.to_string_lossy());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interrupted_copy_restores_runnable_old_app() {
        use std::{thread, time::Duration};
        let root =
            std::env::temp_dir().join(format!("chicklink-interrupt-{:x}", rand::random::<u64>()));
        let old = root.join("Chick Link.app");
        let next = root.join("next.app");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&next).unwrap();
        fs::write(old.join("marker"), "old").unwrap();
        let copier = root.join("slow-copy.sh");
        fs::write(
            &copier,
            "#!/bin/sh\nmkdir -p \"$2\"\nprintf partial > \"$2/marker\"\nsleep 1\n",
        )
        .unwrap();
        fs::set_permissions(&copier, fs::Permissions::from_mode(0o700)).unwrap();
        let script = root.join("install.sh");
        fs::write(&script, HELPER).unwrap();
        let mut child = Command::new("/bin/sh")
            .arg(&script)
            .arg(&old)
            .arg(&next)
            .arg("9999999")
            .arg(root.join("pending.txt"))
            .arg("/usr/bin/true")
            .arg(&copier)
            .arg("/bin/rm")
            .spawn()
            .unwrap();
        let mut saw_partial = false;
        for _ in 0..100 {
            if root.read_dir().unwrap().any(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains("installing")
            }) {
                saw_partial = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(saw_partial);
        unsafe {
            libc::kill(child.id() as i32, libc::SIGTERM);
        }
        assert!(!child.wait().unwrap().success());
        assert_eq!(fs::read_to_string(old.join("marker")).unwrap(), "old");
        assert!(!root.join("Chick Link.app.bak").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_removal_never_nests_backup_in_damaged_app() {
        let root =
            std::env::temp_dir().join(format!("chicklink-rm-fail-{:x}", rand::random::<u64>()));
        let old = root.join("Chick Link.app");
        let backup = root.join("Chick Link.app.bak");
        let next = root.join("next.app");
        for path in [&old, &backup, &next] {
            fs::create_dir_all(path).unwrap();
        }
        fs::write(old.join("marker"), "unverified").unwrap();
        fs::write(backup.join("marker"), "known-good").unwrap();
        fs::write(next.join("marker"), "new").unwrap();
        let pending = root.join("pending.txt");
        fs::write(&pending, old.to_string_lossy().as_bytes()).unwrap();
        let remover = root.join("fail-rm.sh");
        fs::write(
            &remover,
            "#!/bin/sh\ncase \"$3\" in *.app) exit 1;; esac\nexec /bin/rm \"$@\"\n",
        )
        .unwrap();
        fs::set_permissions(&remover, fs::Permissions::from_mode(0o700)).unwrap();
        let script = root.join("install.sh");
        fs::write(&script, HELPER).unwrap();
        let output = Command::new("/bin/sh")
            .arg(&script)
            .arg(&old)
            .arg(&next)
            .arg("9999999")
            .arg(&pending)
            .arg("/usr/bin/true")
            .arg("/usr/bin/ditto")
            .arg(&remover)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(
            fs::read_to_string(old.join("marker")).unwrap(),
            "known-good"
        );
        assert!(!old.join("Chick Link.app.bak").exists());
        assert!(!backup.exists());
        assert_eq!(fs::read_to_string(pending).unwrap(), old.to_string_lossy());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(target_os = "windows")]
pub fn install(installer: &Path) -> Result<(), String> {
    if installer.extension().and_then(|s| s.to_str()) != Some("exe") {
        return Err("当前附件不是 Windows 安装器".into());
    }
    Command::new(installer)
        .spawn()
        .map_err(|_| "无法启动安装器".into())
        .map(|_| ())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn install(_archive: &Path) -> Result<(), String> {
    Err("当前平台不支持应用内更新".into())
}
