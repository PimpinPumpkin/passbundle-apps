//! What the update task asks of the computer: whether anyone but
//! administrators can change a folder or a file, a work folder only they
//! can change, which programs run from a folder, and whether a new agent
//! starts. Only Windows has an update task; the tests stand in for it on
//! every platform.

use std::io;
use std::path::{Path, PathBuf};

pub trait Machine {
    /// Why someone other than SYSTEM, Administrators, and TrustedInstaller
    /// could change `path` (a folder or a file), or None when no one else
    /// can. A link is never taken for what it points to.
    fn not_admin_only(&self, path: &Path) -> Option<String>;

    /// Makes `dir`, which mustn't be there yet: only SYSTEM and
    /// Administrators can change it or anything made in it, and everyone can
    /// read and run what's in it, since it's moved into the install folder.
    fn make_admin_dir(&self, dir: &Path) -> io::Result<()>;

    /// The programs running from inside `dir`, other than this process; Err
    /// when that can't be told.
    fn running_in(&self, dir: &Path) -> Result<Vec<PathBuf>, String>;

    /// Whether the agent at `program` starts: it answers `--version`.
    fn starts(&self, program: &Path) -> bool;
}

#[cfg(windows)]
pub use windows::Windows;

#[cfg(windows)]
mod windows {
    use std::ffi::OsStr;
    use std::fs;
    use std::io::{self, Read};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::fs::MetadataExt;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::ptr;
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{
        CloseHandle, LocalFree, ERROR_SUCCESS, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        GetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        GetAce, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, DACL_SECURITY_INFORMATION, INHERIT_ONLY_ACE,
        OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES,
    };
    use windows_sys::Win32::Storage::FileSystem::{CreateDirectoryW, FILE_ATTRIBUTE_REPARSE_POINT};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::LibraryLoader::{
        SetDefaultDllDirectories, LOAD_LIBRARY_SEARCH_SYSTEM32,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    use super::Machine;
    use crate::brand;

    const SYSTEM: &str = "S-1-5-18";
    const ADMINISTRATORS: &str = "S-1-5-32-544";
    const TRUSTED_INSTALLER: &str =
        "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";
    /// Whoever owns it, which must be one of the three above.
    const OWNER_RIGHTS: &str = "S-1-3-4";
    const ALLOWED: u8 = 0; // ACCESS_ALLOWED_ACE_TYPE
    const DENIED: u8 = 1; // ACCESS_DENIED_ACE_TYPE
    /// What lets someone change a file or put something in a folder, or take
    /// it away, or change who may: writing or appending to it (adding a file
    /// or a folder to a folder), deleting what's in it or it, changing its
    /// access list or owner, and the generic rights that include those.
    const CHANGES: u32 = 0x0002 // FILE_WRITE_DATA, FILE_ADD_FILE
        | 0x0004 // FILE_APPEND_DATA, FILE_ADD_SUBDIRECTORY
        | 0x0040 // FILE_DELETE_CHILD
        | 0x0001_0000 // DELETE
        | 0x0004_0000 // WRITE_DAC
        | 0x0008_0000 // WRITE_OWNER
        | 0x1000_0000 // GENERIC_ALL
        | 0x4000_0000; // GENERIC_WRITE
    /// The work folder: protected (nothing inherited from Program Files),
    /// full control for SYSTEM and Administrators, read and run for Users,
    /// for it and everything made in it.
    const WORK_DIR_SDDL: &str = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)";
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// The Windows the update task runs on, as SYSTEM.
    pub struct Windows;

    impl Windows {
        /// Every library loaded from now on comes from System32 only: never
        /// from the folder the task starts in or a folder on PATH, which an
        /// administrator may have made writable for others.
        pub fn harden() {
            unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) };
        }
    }

    fn wide(text: &OsStr) -> Vec<u16> {
        text.encode_wide().chain(Some(0)).collect()
    }

    fn sid_string(sid: PSID) -> Option<String> {
        let mut string: *mut u16 = ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(sid, &mut string) } == 0 {
            return None;
        }
        let length = (0..)
            .take_while(|&i| unsafe { *string.add(i) } != 0)
            .count();
        let text = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(string, length) });
        unsafe { LocalFree(string as _) };
        Some(text)
    }

    fn trusted(sid: &str) -> bool {
        matches!(sid, SYSTEM | ADMINISTRATORS | TRUSTED_INSTALLER)
    }

    /// A SID as an administrator reads it in a log.
    fn who(sid: &str) -> String {
        match sid {
            "S-1-1-0" => "Everyone".into(),
            "S-1-5-11" => "Authenticated Users".into(),
            "S-1-5-32-545" => "Users".into(),
            "S-1-5-4" => "INTERACTIVE".into(),
            "S-1-3-0" => "CREATOR OWNER".into(),
            other => other.into(),
        }
    }

    impl Machine for Windows {
        fn not_admin_only(&self, path: &Path) -> Option<String> {
            let meta = match fs::symlink_metadata(path) {
                Ok(meta) => meta,
                Err(_) => return Some("it can't be read".into()),
            };
            if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Some("it's a link to somewhere else".into());
            }
            let name = wide(path.as_os_str());
            let mut owner: PSID = ptr::null_mut();
            let mut dacl: *mut ACL = ptr::null_mut();
            let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
            let status = unsafe {
                GetNamedSecurityInfoW(
                    name.as_ptr(),
                    SE_FILE_OBJECT,
                    OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                    &mut owner,
                    ptr::null_mut(),
                    &mut dacl,
                    ptr::null_mut(),
                    &mut descriptor,
                )
            };
            if status != ERROR_SUCCESS {
                return Some("its access list can't be read".into());
            }
            let why = (|| {
                match sid_string(owner) {
                    Some(sid) if trusted(&sid) => {}
                    Some(sid) => return Some(format!("{} owns it", who(&sid))),
                    None => return Some("its owner can't be read".into()),
                }
                // No access list at all lets everyone do anything.
                if dacl.is_null() {
                    return Some("anyone can change it".into());
                }
                let count = unsafe { (*dacl).AceCount };
                for i in 0..u32::from(count) {
                    let mut ace: *mut core::ffi::c_void = ptr::null_mut();
                    if unsafe { GetAce(dacl, i, &mut ace) } == 0 {
                        return Some("its access list can't be read".into());
                    }
                    let header = unsafe { &*(ace as *const ACE_HEADER) };
                    if header.AceType == DENIED {
                        continue;
                    }
                    // Callback, object, and compound entries could allow
                    // anything: none of PassBundle's folders has one.
                    if header.AceType != ALLOWED {
                        return Some(
                            "its access list has entries the update task doesn't read".into(),
                        );
                    }
                    let allowed = unsafe { &*(ace as *const ACCESS_ALLOWED_ACE) };
                    // Entries only for what's made inside it don't apply to it.
                    if u32::from(header.AceFlags) & INHERIT_ONLY_ACE != 0
                        || allowed.Mask & CHANGES == 0
                    {
                        continue;
                    }
                    let Some(sid) = sid_string(ptr::addr_of!(allowed.SidStart) as PSID) else {
                        return Some("its access list can't be read".into());
                    };
                    if !trusted(&sid) && sid != OWNER_RIGHTS {
                        return Some(format!("{} can change it", who(&sid)));
                    }
                }
                None
            })();
            unsafe { LocalFree(descriptor as _) };
            why
        }

        fn make_admin_dir(&self, dir: &Path) -> io::Result<()> {
            let sddl = wide(OsStr::new(WORK_DIR_SDDL));
            let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
            let ok = unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    SDDL_REVISION_1,
                    &mut descriptor,
                    ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            let attributes = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            };
            let name = wide(dir.as_os_str());
            // Fails if anything is there already.
            let made = unsafe { CreateDirectoryW(name.as_ptr(), &attributes) };
            let error = io::Error::last_os_error();
            unsafe { LocalFree(descriptor as _) };
            if made == 0 {
                return Err(error);
            }
            Ok(())
        }

        fn running_in(&self, dir: &Path) -> Result<Vec<PathBuf>, String> {
            let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
            if snapshot == INVALID_HANDLE_VALUE {
                return Err("the running programs can't be listed".into());
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
            let me = std::process::id();
            let inside = comparable(dir) + "\\";
            Ok(pids
                .into_iter()
                .filter(|&pid| pid != 0 && pid != me)
                .filter_map(|pid| {
                    // SYSTEM may ask this of every signed-in person's programs.
                    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
                    if process.is_null() {
                        return None;
                    }
                    let mut path = vec![0u16; 32_768];
                    let mut length = path.len() as u32;
                    let ok = unsafe {
                        QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length)
                    };
                    unsafe { CloseHandle(process) };
                    if ok == 0 {
                        return None;
                    }
                    path.truncate(length as usize);
                    Some(PathBuf::from(std::ffi::OsString::from_wide(&path)))
                })
                .filter(|program| comparable(program).starts_with(&inside))
                .collect())
        }

        fn starts(&self, program: &Path) -> bool {
            let Some(dir) = program.parent() else {
                return false;
            };
            // By its whole path, from its own folder, with nothing else run.
            let Ok(mut child) = Command::new(program)
                .arg("--version")
                .current_dir(dir)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
            else {
                return false;
            };
            let deadline = Instant::now() + Duration::from_secs(30);
            let status = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break status,
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(100))
                    }
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return false;
                    }
                }
            };
            let mut said = String::new();
            if let Some(mut out) = child.stdout.take() {
                let _ = out.by_ref().take(4096).read_to_string(&mut said);
            }
            status.success() && said.starts_with(&format!("{} agent ", brand::NAME))
        }
    }

    fn comparable(path: &Path) -> String {
        let text = path.to_string_lossy();
        let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
        text.replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn elevated() -> bool {
            // What an elevated process makes, Administrators own.
            let probe = std::env::temp_dir().join(format!(
                "pb-agent-elevated-{}-{}",
                std::process::id(),
                rand::random::<u32>()
            ));
            let made = Windows.make_admin_dir(&probe).is_ok();
            let admins = made && Windows.not_admin_only(&probe).is_none();
            let _ = fs::remove_dir(&probe);
            admins
        }

        #[test]
        fn program_files_is_administrators_alone_and_a_profile_isnt() {
            let program_files = PathBuf::from(std::env::var("ProgramFiles").unwrap());
            assert_eq!(Windows.not_admin_only(&program_files), None);
            let windows = PathBuf::from(std::env::var("SystemRoot").unwrap());
            assert_eq!(Windows.not_admin_only(&windows.join("System32")), None);
            // The person's own folders are theirs, or others' too.
            let temp = std::env::temp_dir();
            assert!(Windows.not_admin_only(&temp).is_some());
            assert!(Windows.not_admin_only(&temp.join("not-there")).is_some());
        }

        #[test]
        fn the_work_folder_is_administrators_alone_with_what_is_made_in_it() {
            if !elevated() {
                eprintln!("skipped: not elevated, as CI's machines are");
                return;
            }
            let base = std::env::temp_dir().join(format!(
                "pb-agent-admin-{}-{}",
                std::process::id(),
                rand::random::<u32>()
            ));
            Windows.make_admin_dir(&base).unwrap();
            assert_eq!(Windows.not_admin_only(&base), None);
            // It must not be there already.
            assert!(Windows.make_admin_dir(&base).is_err());
            fs::create_dir(base.join("PassBundle")).unwrap();
            fs::write(base.join("PassBundle/passbundle.exe"), b"x").unwrap();
            assert_eq!(Windows.not_admin_only(&base.join("PassBundle")), None);
            assert_eq!(
                Windows.not_admin_only(&base.join("PassBundle/passbundle.exe")),
                None
            );
            fs::remove_dir_all(&base).unwrap();
        }

        /// Not a test of its own: what the next one runs a copy of this
        /// program as, to have something running from a folder.
        #[test]
        #[ignore]
        fn stays_running() {
            std::thread::sleep(Duration::from_secs(60));
        }

        #[test]
        fn a_program_running_from_a_folder_is_found_and_this_one_never() {
            let me = std::env::current_exe().unwrap();
            let base = std::env::temp_dir().join(format!(
                "pb-agent-running-{}-{}",
                std::process::id(),
                rand::random::<u32>()
            ));
            fs::create_dir_all(&base).unwrap();
            let copy = base.join("passbundle.exe");
            fs::copy(&me, &copy).unwrap();
            let mut child = Command::new(&copy)
                .args([
                    "--exact",
                    "machine::windows::tests::stays_running",
                    "--ignored",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut running = Vec::new();
            while running.is_empty() && Instant::now() < deadline {
                running = Windows.running_in(&base).unwrap();
                std::thread::sleep(Duration::from_millis(100));
            }
            let _ = child.kill();
            let _ = child.wait();
            assert_eq!(
                running.iter().map(|p| comparable(p)).collect::<Vec<_>>(),
                vec![comparable(&copy)]
            );
            // Never this process, though it runs from its own folder.
            let mine = Windows.running_in(me.parent().unwrap()).unwrap();
            assert!(mine.iter().all(|p| comparable(p) != comparable(&me)));
            let _ = fs::remove_dir_all(&base);
        }
    }
}
