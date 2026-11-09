//! Whether a browser has a profile open, by the lock it keeps beside it, and
//! writing a settings file back whole. A browser writes its settings back
//! when it closes, so PassBundle changes them only while it's closed.
//!
//! - **Chromium:** on macOS and Linux, `SingletonLock` in the browser's
//!   folder, a link to "<computer>-<process id>" while it runs; on Windows,
//!   `lockfile` there, which the running browser holds open, unshared.
//! - **Firefox:** on macOS and Linux, `.parentlock` in the profile, which
//!   the running browser holds a lock on (and on Linux a `lock` link to
//!   "<address>:+<process id>"); on Windows, `parent.lock`, held open unshared.

use std::path::Path;

use crate::Platform;

pub(crate) fn chromium(browser_dir: &Path, platform: Platform) -> bool {
    match platform {
        Platform::Windows => held_open(&browser_dir.join("lockfile")),
        _ => linked_process_alive(&browser_dir.join("SingletonLock"), '-'),
    }
}

pub(crate) fn firefox(profile_dir: &Path, platform: Platform) -> bool {
    match platform {
        Platform::Windows => held_open(&profile_dir.join("parent.lock")),
        _ => {
            locked(&profile_dir.join(".parentlock"))
                || linked_process_alive(&profile_dir.join("lock"), '+')
        }
    }
}

/// A link whose target ends in a process id, after the last `sep`, whose
/// process is running. A link to another computer's process (a shared home
/// folder) counts as open, since this computer can't tell.
fn linked_process_alive(link: &Path, sep: char) -> bool {
    let Ok(target) = std::fs::read_link(link) else {
        return false;
    };
    let target = target.to_string_lossy().into_owned();
    let Some(pid) = target
        .rsplit_once(sep)
        .and_then(|(_, pid)| pid.parse::<i32>().ok())
    else {
        return true;
    };
    process_alive(pid)
}

#[cfg(unix)]
fn process_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // SAFETY: signal 0 only asks whether the process exists.
    let answer = unsafe { libc::kill(pid, 0) };
    answer == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn process_alive(_: i32) -> bool {
    true
}

/// Whether another process holds a lock on the file (fcntl).
#[cfg(unix)]
fn locked(path: &Path) -> bool {
    use std::os::fd::AsRawFd;
    let Ok(file) = std::fs::OpenOptions::new().read(true).open(path) else {
        return false;
    };
    // SAFETY: an all-zero flock is valid; F_GETLK only fills it in.
    let mut lock: libc::flock = unsafe { std::mem::zeroed() };
    lock.l_type = libc::F_WRLCK as libc::c_short;
    lock.l_whence = libc::SEEK_SET as libc::c_short;
    // SAFETY: `file` is open for the call, and `lock` is a valid flock.
    let asked = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETLK, &mut lock) };
    asked == 0 && lock.l_type != libc::F_UNLCK as libc::c_short
}

#[cfg(not(unix))]
fn locked(_: &Path) -> bool {
    false
}

/// Whether the file is there and another program holds it open unshared.
#[cfg(windows)]
fn held_open(path: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    if !path.exists() {
        return false;
    }
    // Share nothing: a running browser has it open, so this is refused.
    match std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
    {
        Ok(_) => false,
        Err(e) => e.raw_os_error() == Some(32), // ERROR_SHARING_VIOLATION
    }
}

#[cfg(not(windows))]
fn held_open(_: &Path) -> bool {
    false
}

/// Writes a file whole: a new file beside it, then renamed over it, keeping
/// its permissions, so a stopped write never leaves half a file.
pub(crate) fn replace_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = path.with_file_name(format!(".{name}.passbundle"));
    let _ = std::fs::remove_file(&temporary);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let written = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .and_then(|()| match std::fs::metadata(path) {
            Ok(meta) => std::fs::set_permissions(&temporary, meta.permissions()),
            Err(_) => Ok(()),
        });
    drop(file);
    if let Err(e) = written.and_then(|()| std::fs::rename(&temporary, path)) {
        let _ = std::fs::remove_file(&temporary);
        return Err(e);
    }
    Ok(())
}
