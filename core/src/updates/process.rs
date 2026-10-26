//! The processes an update waits for and stops: the app, its desktop agent,
//! and anything else running from the folder being replaced, by the OS's own
//! answer of which program each process runs.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A running process and the program it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Running {
    pub pid: u32,
    pub program: PathBuf,
}

/// Whether `program` is inside `folder`, as the OS names both (on Windows,
/// whatever the case).
pub fn is_inside(program: &Path, folder: &Path) -> bool {
    let program = comparable(program);
    let mut folder = comparable(folder);
    if !folder.ends_with(std::path::MAIN_SEPARATOR) {
        folder.push(std::path::MAIN_SEPARATOR);
    }
    program.starts_with(&folder)
}

fn comparable(path: &Path) -> String {
    let text = path.to_string_lossy();
    #[cfg(windows)]
    {
        let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
        text.replace('/', "\\").to_lowercase()
    }
    #[cfg(not(windows))]
    {
        // As the kernel names programs (/private/tmp on macOS, say).
        std::fs::canonicalize(path)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| text.into_owned())
    }
}

/// Every process running a program from inside `folder`, but this one.
pub fn under(folder: &Path) -> Vec<Running> {
    let me = std::process::id();
    all()
        .into_iter()
        .filter(|p| p.pid != me && is_inside(&p.program, folder))
        .collect()
}

/// Waits until `pid` has ended, at most `limit`; whether it has.
pub fn wait_for(pid: u32, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if !alive(pid) {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Ends each of `processes`, and waits for them, at most `limit`; whether
/// they've all ended.
pub fn stop(processes: &[Running], limit: Duration) -> bool {
    for p in processes {
        terminate(p.pid);
    }
    processes.iter().all(|p| wait_for(p.pid, limit))
}

#[cfg(target_os = "linux")]
fn all() -> Vec<Running> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
            let program = std::fs::read_link(entry.path().join("exe")).ok()?;
            Some(Running { pid, program })
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn all() -> Vec<Running> {
    use std::os::unix::ffi::OsStringExt;
    let mut pids = vec![0 as libc::pid_t; 8192];
    let count = unsafe {
        libc::proc_listallpids(
            pids.as_mut_ptr().cast(),
            (pids.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int,
        )
    };
    if count <= 0 {
        return Vec::new();
    }
    pids.truncate(count as usize);
    pids.into_iter()
        .filter(|&pid| pid > 0)
        .filter_map(|pid| {
            let mut path = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
            let written =
                unsafe { libc::proc_pidpath(pid, path.as_mut_ptr().cast(), path.len() as u32) };
            if written <= 0 {
                return None;
            }
            path.truncate(written as usize);
            Some(Running {
                pid: pid as u32,
                program: std::ffi::OsString::from_vec(path).into(),
            })
        })
        .collect()
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn all() -> Vec<Running> {
    Vec::new()
}

#[cfg(unix)]
pub fn alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if unsafe { libc::kill(pid, 0) } != 0 {
        // EPERM: someone else's, so still there.
        return std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
    }
    // A process that has ended but whose parent hasn't heard yet is gone.
    #[cfg(target_os = "linux")]
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        if let Some((_, after)) = stat.rsplit_once(')') {
            if after.trim_start().starts_with('Z') {
                return false;
            }
        }
    }
    true
}

#[cfg(unix)]
fn terminate(pid: u32) {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return;
    };
    unsafe { libc::kill(pid, libc::SIGTERM) };
    let deadline = Instant::now() + Duration::from_secs(3);
    while alive(pid as u32) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    if alive(pid as u32) {
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
}

#[cfg(windows)]
fn all() -> Vec<Running> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
    let mut pids = Vec::new();
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while more {
        pids.push(entry.th32ProcessID);
        more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    unsafe { CloseHandle(snapshot) };
    pids.into_iter()
        .filter(|&pid| pid != 0)
        .filter_map(|pid| {
            let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
            if process.is_null() {
                return None;
            }
            let mut path = vec![0u16; 32_768];
            let mut length = path.len() as u32;
            let ok =
                unsafe { QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length) };
            unsafe { CloseHandle(process) };
            if ok == 0 {
                return None;
            }
            path.truncate(length as usize);
            Some(Running {
                pid,
                program: std::ffi::OsString::from_wide(&path).into(),
            })
        })
        .collect()
}

#[cfg(windows)]
pub fn alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if process.is_null() {
        return false;
    }
    let running = unsafe { WaitForSingleObject(process, 0) } == WAIT_TIMEOUT;
    unsafe { CloseHandle(process) };
    running
}

#[cfg(windows)]
fn terminate(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    let process = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
    if process.is_null() {
        return;
    }
    unsafe { TerminateProcess(process, 1) };
    unsafe { CloseHandle(process) };
}

/// Starts `program` on its own: not ended with this process, nor with the
/// terminal or the session this one has, and with nothing of its input or
/// output.
pub fn spawn_detached(
    program: &Path,
    args: &[&std::ffi::OsStr],
    dir: &Path,
) -> std::io::Result<std::process::Child> {
    let mut command = std::process::Command::new(program);
    command
        .args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe, and nothing else runs here.
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        command.spawn()
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        // Out of any job this one is in (one that ends its processes with
        // it, say), where the job allows; else in it.
        command.creation_flags(
            DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB,
        );
        match command.spawn() {
            Ok(child) => Ok(child),
            Err(_) => {
                command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
                command.spawn()
            }
        }
    }
}

/// Starts the app at `program`, from its own folder, as if the person had.
pub fn start_app(program: &Path) -> std::io::Result<std::process::Child> {
    let dir = program.parent().unwrap_or(Path::new("."));
    let mut command = std::process::Command::new(program);
    command
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command.spawn()
}

/// Lets the process this one starts next come to the front (Windows keeps
/// a process in the background from taking it).
pub fn let_next_come_forward() {
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(
            windows_sys::Win32::UI::WindowsAndMessaging::ASFW_ANY,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_a_folder_is_by_whole_names() {
        let base = std::env::temp_dir().join(format!("pb-inside-{}", std::process::id()));
        let app = base.join("PassBundle");
        let other = base.join("PassBundle-old");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(app.join("passbundle"), b"").unwrap();
        std::fs::write(other.join("passbundle"), b"").unwrap();
        assert!(is_inside(&app.join("passbundle"), &app));
        assert!(!is_inside(&other.join("passbundle"), &app));
        assert!(!is_inside(&app, &app));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn this_test_is_running_from_its_own_folder() {
        let me = std::env::current_exe().unwrap();
        let folder = me.parent().unwrap();
        // `under` leaves this process out; `all` has it.
        assert!(all()
            .iter()
            .any(|p| p.pid == std::process::id() && is_inside(&p.program, folder)));
        assert!(under(folder).iter().all(|p| p.pid != std::process::id()));
        assert!(alive(std::process::id()));
    }
}
