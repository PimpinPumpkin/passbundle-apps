//! Whose the folders an update touches are: the app's folder and the one it's
//! in must be this person's alone, and the work folder next to them theirs
//! alone too, made by the updater for itself (DESIGN.md, "Desktop updates").
//! Otherwise someone else on the computer could plant a file the app reads,
//! a link it writes through, or a program where the app's folder was a
//! moment ago, which would then run as this person.

use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

/// Why `dir` isn't a folder only this person can change, or None when it
/// is: a real folder (not a link to one), theirs, and not writable by anyone
/// else. On Linux and macOS, writable by their own group only when that's
/// their private group (named as they are, with no one else in it). On
/// Windows, writable by them, SYSTEM, Administrators, and TrustedInstaller
/// only; with `admins_own`, a folder Administrators own counts as theirs
/// (what an elevated process makes, as on CI's machines).
pub fn not_theirs(dir: &Path, admins_own: bool) -> Option<&'static str> {
    let meta = match fs::symlink_metadata(dir) {
        Ok(meta) => meta,
        Err(_) => return Some("it can't be read"),
    };
    if !meta.is_dir() {
        return Some("it isn't a folder");
    }
    imp::not_theirs(dir, &meta, admins_own)
}

/// Whether `dir` is a work folder this updater made: a real folder, this
/// person's, and only theirs to read or change (on Linux and macOS, mode
/// 0700).
pub fn is_private(dir: &Path) -> bool {
    match fs::symlink_metadata(dir) {
        Ok(meta) if meta.is_dir() => imp::is_private(dir, &meta),
        _ => false,
    }
}

/// Makes `dir` (which mustn't exist yet), only this person's: on Linux and
/// macOS mode 0700; on Windows, owned by them, with an access list that
/// lets in only them and SYSTEM and inherits nothing.
pub fn make_private_dir(dir: &Path) -> io::Result<()> {
    imp::make_private_dir(dir)?;
    if is_private(dir) {
        Ok(())
    } else {
        Err(io::Error::other("the work folder isn't private"))
    }
}

/// Writes `bytes` to `path` in a private work folder, whole, never through a
/// link that's there.
pub fn write_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)
}

/// Reads at most `max` bytes of `path` in a private work folder, never
/// through a link; None when it isn't there, is a link, or is longer.
pub fn read_file(path: &Path, max: u64) -> Option<Vec<u8>> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= max).then_some(bytes)
}

/// Whether this process runs as root, or on Windows elevated (as an
/// administrator): then the app only says there's an update, as for a
/// system-wide install, and never replaces itself.
pub fn elevated() -> bool {
    imp::elevated()
}

/// Whether `dir` is in one of the OS's own places for programs and their
/// data (Program Files, ProgramData, Windows; /opt, /usr, /snap, and the
/// like), where a package or an administrator put it: then the app only
/// says there's an update.
pub fn system_place(dir: &Path) -> bool {
    imp::system_place(dir)
}

#[cfg(unix)]
mod imp {
    use std::ffi::CStr;
    use std::fs;
    use std::io;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    use std::path::Path;

    pub fn not_theirs(_dir: &Path, meta: &fs::Metadata, _admins_own: bool) -> Option<&'static str> {
        if meta.uid() != unsafe { libc::geteuid() } {
            return Some("someone else owns it");
        }
        let mode = meta.mode();
        if mode & 0o002 != 0 {
            return Some("anyone can change it");
        }
        if mode & 0o020 != 0 && !private_group(meta.gid()) {
            return Some("its group can change it");
        }
        None
    }

    pub fn is_private(_dir: &Path, meta: &fs::Metadata) -> bool {
        meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o777 == 0o700
    }

    pub fn make_private_dir(dir: &Path) -> io::Result<()> {
        fs::DirBuilder::new().mode(0o700).create(dir)
    }

    pub fn elevated() -> bool {
        unsafe { libc::geteuid() == 0 }
    }

    /// The person's own group: their primary group, named as they are, with
    /// no one else in it (user private groups, where `umask` is 002).
    fn private_group(gid: libc::gid_t) -> bool {
        let uid = unsafe { libc::geteuid() };
        let mut passwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut passwd_buffer = vec![0 as libc::c_char; 16 << 10];
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        let ok = unsafe {
            libc::getpwuid_r(
                uid,
                &mut passwd,
                passwd_buffer.as_mut_ptr(),
                passwd_buffer.len(),
                &mut found,
            )
        };
        if ok != 0 || found.is_null() || passwd.pw_gid != gid || passwd.pw_name.is_null() {
            return false;
        }
        let name = unsafe { CStr::from_ptr(passwd.pw_name) };
        let mut group: libc::group = unsafe { std::mem::zeroed() };
        let mut group_buffer = vec![0 as libc::c_char; 64 << 10];
        let mut found: *mut libc::group = std::ptr::null_mut();
        let ok = unsafe {
            libc::getgrgid_r(
                gid,
                &mut group,
                group_buffer.as_mut_ptr(),
                group_buffer.len(),
                &mut found,
            )
        };
        if ok != 0 || found.is_null() || group.gr_name.is_null() {
            return false;
        }
        if unsafe { CStr::from_ptr(group.gr_name) } != name {
            return false;
        }
        let mut member = group.gr_mem;
        while !member.is_null() && !unsafe { *member }.is_null() {
            if unsafe { CStr::from_ptr(*member) } != name {
                return false;
            }
            member = unsafe { member.add(1) };
        }
        true
    }

    #[cfg(target_os = "linux")]
    pub fn system_place(dir: &Path) -> bool {
        const PLACES: &[&str] = &[
            "/opt", "/usr", "/snap", "/var", "/etc", "/bin", "/sbin", "/lib", "/lib32", "/lib64",
            "/boot", "/srv", "/app", "/nix", "/gnu",
        ];
        let dir = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        PLACES.iter().any(|place| dir.starts_with(place))
    }

    /// Not on macOS and the like, where the app updates itself its own way.
    #[cfg(not(target_os = "linux"))]
    pub fn system_place(_dir: &Path) -> bool {
        false
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::OsStr;
    use std::fs;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    use std::path::Path;
    use std::ptr;

    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, ERROR_SUCCESS, HANDLE};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        GetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        GetAce, GetTokenInformation, TokenElevation, TokenUser, ACCESS_ALLOWED_ACE, ACL,
        DACL_SECURITY_INFORMATION, INHERIT_ONLY_ACE, OWNER_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::{CreateDirectoryW, FILE_ATTRIBUTE_REPARSE_POINT};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    const SYSTEM: &str = "S-1-5-18";
    const ADMINISTRATORS: &str = "S-1-5-32-544";
    const OWNER_RIGHTS: &str = "S-1-3-4";
    const TRUSTED_INSTALLER: &str =
        "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";
    /// An access-allowed entry (ACCESS_ALLOWED_ACE_TYPE).
    const ALLOWED: u8 = 0;
    /// What lets someone put something in a folder, or take it away, or
    /// change who may: adding files or folders, deleting what's in it or it,
    /// changing its access list or owner, and the generic rights that
    /// include those.
    const CHANGES: u32 = 0x0002 // FILE_ADD_FILE
        | 0x0004 // FILE_ADD_SUBDIRECTORY
        | 0x0040 // FILE_DELETE_CHILD
        | 0x0001_0000 // DELETE
        | 0x0004_0000 // WRITE_DAC
        | 0x0008_0000 // WRITE_OWNER
        | 0x1000_0000 // GENERIC_ALL
        | 0x4000_0000; // GENERIC_WRITE

    fn wide(path: &OsStr) -> Vec<u16> {
        path.encode_wide().chain(Some(0)).collect()
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

    /// A query of this process's token: its user, or whether it's elevated.
    fn token_information(class: i32) -> Option<Vec<u8>> {
        let mut token: HANDLE = ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return None;
        }
        let mut buffer = vec![0u8; 512];
        let mut needed = 0u32;
        let ok = unsafe {
            GetTokenInformation(
                token,
                class,
                buffer.as_mut_ptr() as _,
                buffer.len() as u32,
                &mut needed,
            )
        };
        unsafe { CloseHandle(token) };
        (ok != 0).then_some(buffer)
    }

    fn user_sid() -> Option<String> {
        let buffer = token_information(TokenUser)?;
        let user = unsafe { &*(buffer.as_ptr() as *const TOKEN_USER) };
        sid_string(user.User.Sid)
    }

    pub fn elevated() -> bool {
        token_information(TokenElevation)
            .map(|buffer| unsafe { &*(buffer.as_ptr() as *const TOKEN_ELEVATION) }.TokenIsElevated != 0)
            .unwrap_or(true)
    }

    /// A folder's owner, and whether anyone but the trusted may change it.
    fn owner_and_others(dir: &Path, me: &str) -> Option<(String, bool)> {
        let name = wide(dir.as_os_str());
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
            return None;
        }
        let answer = (|| {
            let owner = sid_string(owner)?;
            // No access list at all lets everyone do anything.
            if dacl.is_null() {
                return Some((owner, true));
            }
            let count = unsafe { (*dacl).AceCount };
            let mut others = false;
            for i in 0..count as u32 {
                let mut ace: *mut core::ffi::c_void = ptr::null_mut();
                if unsafe { GetAce(dacl, i, &mut ace) } == 0 {
                    return None;
                }
                let allowed = unsafe { &*(ace as *const ACCESS_ALLOWED_ACE) };
                // Entries only for what's made inside it don't apply to it.
                if allowed.Header.AceType != ALLOWED
                    || u32::from(allowed.Header.AceFlags) & INHERIT_ONLY_ACE != 0
                    || allowed.Mask & CHANGES == 0
                {
                    continue;
                }
                let sid = sid_string(ptr::addr_of!(allowed.SidStart) as PSID)?;
                if !matches!(
                    sid.as_str(),
                    SYSTEM | ADMINISTRATORS | OWNER_RIGHTS | TRUSTED_INSTALLER
                ) && sid != me
                {
                    others = true;
                }
            }
            Some((owner, others))
        })();
        unsafe { LocalFree(descriptor as _) };
        answer
    }

    fn is_link(meta: &fs::Metadata) -> bool {
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    pub fn not_theirs(dir: &Path, meta: &fs::Metadata, admins_own: bool) -> Option<&'static str> {
        if is_link(meta) {
            return Some("it's a link to somewhere else");
        }
        let Some(me) = user_sid() else {
            return Some("who's running PassBundle can't be told");
        };
        let Some((owner, others)) = owner_and_others(dir, &me) else {
            return Some("its access list can't be read");
        };
        if owner != me && !(admins_own && owner == ADMINISTRATORS) {
            return Some("someone else owns it");
        }
        if others {
            return Some("others can change it");
        }
        None
    }

    pub fn is_private(dir: &Path, meta: &fs::Metadata) -> bool {
        if is_link(meta) {
            return false;
        }
        let Some(me) = user_sid() else {
            return false;
        };
        matches!(owner_and_others(dir, &me), Some((owner, false)) if owner == me)
    }

    pub fn make_private_dir(dir: &Path) -> io::Result<()> {
        let me =
            user_sid().ok_or_else(|| io::Error::other("who's running PassBundle can't be told"))?;
        // Owned by this person; a protected access list (nothing inherited)
        // with full control for them and SYSTEM, for it and what's in it.
        let sddl = wide(OsStr::new(&format!(
            "O:{me}D:P(A;OICI;FA;;;{me})(A;OICI;FA;;;SY)"
        )));
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
        let made = unsafe { CreateDirectoryW(name.as_ptr(), &attributes) };
        let error = io::Error::last_os_error();
        unsafe { LocalFree(descriptor as _) };
        if made == 0 {
            return Err(error);
        }
        Ok(())
    }

    pub fn system_place(dir: &Path) -> bool {
        let dir = comparable(&dir.to_string_lossy());
        [
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramW6432",
            "ProgramData",
            "SystemRoot",
            "windir",
        ]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .filter(|place| !place.trim().is_empty())
        .any(|place| {
            let place = comparable(&place);
            dir == place || dir.starts_with(&format!("{}\\", place.trim_end_matches('\\')))
        })
    }

    fn comparable(path: &str) -> String {
        let path = path.strip_prefix(r"\\?\").unwrap_or(path);
        path.replace('/', "\\").to_lowercase()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pb-access-{name}-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_folder_anyone_can_change_isnt_theirs() {
        let dir = temp("modes");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(not_theirs(&dir, false), None);
        // Writable by anyone, as /tmp is, sticky or not.
        for mode in [0o777, 0o1777, 0o757] {
            fs::set_permissions(&dir, fs::Permissions::from_mode(mode)).unwrap();
            assert_eq!(
                not_theirs(&dir, false),
                Some("anyone can change it"),
                "{mode:o}"
            );
        }
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        // A link to a folder is never taken for one.
        let link = dir.with_extension("link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        assert_eq!(not_theirs(&link, false), Some("it isn't a folder"));
        assert!(!is_private(&link));
        fs::remove_file(&link).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_work_folder_is_private_only_as_this_makes_it() {
        let base = temp("private");
        let work = base.join("work");
        make_private_dir(&work).unwrap();
        assert!(is_private(&work));
        assert_eq!(
            fs::metadata(&work).unwrap().permissions().mode() & 0o777,
            0o700
        );
        // It must not be there already, made by someone.
        assert!(make_private_dir(&work).is_err());
        fs::set_permissions(&work, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!is_private(&work));
        fs::set_permissions(&work, fs::Permissions::from_mode(0o700)).unwrap();
        // Files in it are written and read, never through a link.
        write_file(&work.join("started"), b"42").unwrap();
        assert_eq!(read_file(&work.join("started"), 10).unwrap(), b"42");
        assert_eq!(read_file(&work.join("started"), 1), None);
        let elsewhere = base.join("elsewhere");
        fs::write(&elsewhere, b"theirs").unwrap();
        std::os::unix::fs::symlink(&elsewhere, work.join("link")).unwrap();
        assert!(write_file(&work.join("link"), b"mine").is_err());
        assert_eq!(read_file(&work.join("link"), 100), None);
        assert_eq!(fs::read(&elsewhere).unwrap(), b"theirs");
        fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_oss_own_places_are_only_told_of_updates() {
        for place in [
            "/opt/passbundle",
            "/usr/lib/passbundle",
            "/snap/passbundle/1",
        ] {
            assert!(system_place(Path::new(place)), "{place}");
        }
        assert!(!system_place(&std::env::temp_dir()));
    }
}
