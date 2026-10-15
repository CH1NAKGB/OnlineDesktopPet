//! A version-independent guard, acquired before any client settings are loaded.

use std::io;

#[cfg(not(target_os = "windows"))]
use std::{
    fs::{self, File, OpenOptions},
    os::fd::AsRawFd,
    path::Path,
};

#[cfg(target_os = "windows")]
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
    System::Threading::CreateMutexW,
};

pub struct InstanceGuard {
    #[cfg(target_os = "windows")]
    handle: HANDLE,
    #[cfg(not(target_os = "windows"))]
    _file: File,
}

pub fn acquire() -> io::Result<Option<InstanceGuard>> {
    #[cfg(target_os = "windows")]
    {
        acquire_named("Local\\ChickLink.SingleInstance")
    }
    #[cfg(not(target_os = "windows"))]
    {
        let root =
            crate::config::app_data_root().ok_or_else(|| io::Error::other("无法确定设置目录"))?;
        acquire_at(&root.join("instance.lock"))
    }
}

#[cfg(target_os = "windows")]
fn acquire_named(name: &str) -> io::Result<Option<InstanceGuard>> {
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    // SAFETY: The name is NUL-terminated and valid for this call. The handle is
    // kept alive by InstanceGuard and closed on drop.
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // GetLastError must immediately follow CreateMutexW: a successful call can
    // still report that another process created this mutex first.
    let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if existed {
        unsafe { CloseHandle(handle) };
        Ok(None)
    } else {
        Ok(Some(InstanceGuard { handle }))
    }
}

#[cfg(not(target_os = "windows"))]
fn acquire_at(path: &Path) -> io::Result<Option<InstanceGuard>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)?;
    // SAFETY: fd belongs to the open file retained in InstanceGuard.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(Some(InstanceGuard { _file: file }));
    }
    let error = io::Error::last_os_error();
    if matches!(error.raw_os_error(), Some(libc::EWOULDBLOCK)) {
        Ok(None)
    } else {
        Err(error)
    }
}

#[cfg(target_os = "windows")]
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_waits_until_guard_is_dropped() {
        #[cfg(target_os = "windows")]
        let name = format!("Local\\ChickLink.Test.{}", std::process::id());
        #[cfg(not(target_os = "windows"))]
        let path = std::env::temp_dir().join(format!(
            "chicklink-instance-test-{}.lock",
            std::process::id()
        ));

        #[cfg(target_os = "windows")]
        let first = acquire_named(&name).unwrap().unwrap();
        #[cfg(not(target_os = "windows"))]
        let first = acquire_at(&path).unwrap().unwrap();

        #[cfg(target_os = "windows")]
        assert!(acquire_named(&name).unwrap().is_none());
        #[cfg(not(target_os = "windows"))]
        assert!(acquire_at(&path).unwrap().is_none());

        drop(first);

        #[cfg(target_os = "windows")]
        assert!(acquire_named(&name).unwrap().is_some());
        #[cfg(not(target_os = "windows"))]
        {
            assert!(acquire_at(&path).unwrap().is_some());
            fs::remove_file(path).unwrap();
        }
    }
}
