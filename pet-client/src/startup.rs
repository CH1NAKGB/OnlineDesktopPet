#[cfg(target_os = "macos")]
mod platform {
    use std::{fs, io::ErrorKind, path::PathBuf};

    const LABEL: &str = "com.chicklink.desktop-pet";

    pub fn is_enabled() -> bool {
        let Ok(path) = launch_agent_path() else {
            return false;
        };
        let Ok(contents) = fs::read_to_string(path) else {
            return false;
        };
        let Ok(executable) = std::env::current_exe() else {
            return false;
        };
        contents.contains(&xml_escape(&executable.to_string_lossy()))
    }

    pub fn set_enabled(enabled: bool) -> anyhow::Result<()> {
        let path = launch_agent_path()?;
        if !enabled {
            match fs::remove_file(path) {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error.into()),
            }
        }

        let executable = std::env::current_exe()?;
        let executable = xml_escape(&executable.to_string_lossy());
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{executable}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>LimitLoadToSessionType</key>
    <string>Aqua</string>
</dict>
</plist>
"#,
        );
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, plist)?;
        Ok(())
    }

    fn launch_agent_path() -> anyhow::Result<PathBuf> {
        let home =
            std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("无法确定当前用户目录"))?;
        Ok(PathBuf::from(home)
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist")))
    }

    fn xml_escape(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    #[cfg(test)]
    mod tests {
        use super::xml_escape;

        #[test]
        fn escapes_launch_agent_program_paths() {
            assert_eq!(
                xml_escape(r#"/Applications/Chick & Link/"pet""#),
                "/Applications/Chick &amp; Link/&quot;pet&quot;"
            );
        }
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::ffi::c_void;

    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{
            HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
            RegSetKeyValueW,
        },
    };

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE_NAME: &str = "ChickLink";

    pub fn is_enabled() -> bool {
        let subkey = wide(RUN_KEY);
        let value_name = wide(VALUE_NAME);
        let mut byte_len = 0;
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                value_name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut byte_len,
            )
        };
        if result != ERROR_SUCCESS || byte_len == 0 {
            return false;
        }
        let mut data = vec![0_u16; byte_len.div_ceil(size_of::<u16>() as u32) as usize];
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                value_name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                data.as_mut_ptr().cast::<c_void>(),
                &mut byte_len,
            )
        };
        if result != ERROR_SUCCESS {
            return false;
        }
        let Some(executable) = std::env::current_exe().ok() else {
            return false;
        };
        let actual = String::from_utf16_lossy(&data)
            .trim_end_matches('\0')
            .to_owned();
        actual == format!(r#""{}""#, executable.display())
    }

    pub fn set_enabled(enabled: bool) -> anyhow::Result<()> {
        let subkey = wide(RUN_KEY);
        let value_name = wide(VALUE_NAME);
        let result = if enabled {
            let executable = std::env::current_exe()?;
            let command = wide(&format!(r#""{}""#, executable.display()));
            unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    subkey.as_ptr(),
                    value_name.as_ptr(),
                    REG_SZ,
                    command.as_ptr().cast::<c_void>(),
                    (command.len() * size_of::<u16>()) as u32,
                )
            }
        } else {
            unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, subkey.as_ptr(), value_name.as_ptr()) }
        };
        if result == ERROR_SUCCESS || (!enabled && result == ERROR_FILE_NOT_FOUND) {
            Ok(())
        } else {
            anyhow::bail!("Windows 启动项操作失败，错误码 {result}")
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain([0]).collect()
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    pub fn is_enabled() -> bool {
        false
    }

    pub fn set_enabled(_enabled: bool) -> anyhow::Result<()> {
        anyhow::bail!("当前平台暂不支持开机自启")
    }
}

pub use platform::{is_enabled, set_enabled};
