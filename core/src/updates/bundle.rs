//! The copy of PassBundle being updated: its folder, the release it is
//! (`release.json`, which each release's build writes into it), and whether
//! an update can take its place.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::manifest::{Version, APP};

/// What a release's build writes into the app's folder: which release this
/// is, and the names in the folder, the only ones an update may move.
pub const RELEASE: &str = "release.json";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReleaseInfo {
    pub app: String,
    pub version: String,
    pub platform: String,
    pub files: Vec<String>,
}

/// The app's program, in its folder.
pub fn program_name() -> &'static str {
    if cfg!(windows) {
        "passbundle.exe"
    } else {
        "passbundle"
    }
}

/// What a release's folder must have: the program, the core, the updater,
/// and `release.json`.
pub fn required() -> Vec<String> {
    let core = if cfg!(windows) {
        "passbundle_core.dll"
    } else if cfg!(target_os = "macos") {
        "libpassbundle_core.dylib"
    } else {
        "lib/libpassbundle_core.so"
    };
    vec![
        program_name().to_owned(),
        core.to_owned(),
        super::swap::helper_name(),
        RELEASE.to_owned(),
    ]
}

/// Names the OS itself may add to a folder, which an update may move with it.
fn harmless(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "desktop.ini" | "thumbs.db" | ".directory" | ".ds_store"
    )
}

#[derive(Debug, Clone)]
pub struct Installed {
    /// The app's folder.
    pub dir: PathBuf,
    /// The release this copy is, or None for a copy built on a computer.
    pub release: Option<ReleaseInfo>,
}

impl Installed {
    /// The copy whose program is `program`.
    pub fn at(program: &Path) -> Installed {
        let dir = program.parent().map(Path::to_path_buf).unwrap_or_default();
        let release = read_release(&dir);
        Installed { dir, release }
    }

    /// This copy's release, if it's one for `platform`.
    pub fn release_for(&self, platform: &str) -> Option<&ReleaseInfo> {
        self.release
            .as_ref()
            .filter(|r| r.app == APP && r.platform == platform)
            .filter(|r| Version::parse(&r.version).is_some())
    }

    /// This copy's build number: 0 for a copy that isn't a release's.
    pub fn build(&self, platform: &str) -> u32 {
        self.release_for(platform)
            .and_then(|r| Version::parse(&r.version))
            .map(|v| v.build)
            .unwrap_or(0)
    }
}

pub fn read_release(dir: &Path) -> Option<ReleaseInfo> {
    let bytes = fs::read(dir.join(RELEASE)).ok()?;
    if bytes.len() > 64 << 10 {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

/// Where an update of the app in `install` is put together: next to it, on
/// the same disk, so putting it in place is a rename. `.PassBundle-update`
/// for `PassBundle`.
pub fn work_dir(install: &Path) -> PathBuf {
    let name = install
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "PassBundle".into());
    install.with_file_name(format!(".{name}-update"))
}

/// Where a copy that can't update itself could go, to say so.
pub fn suggested_place() -> &'static str {
    if cfg!(windows) {
        r"%LOCALAPPDATA%\Programs\PassBundle"
    } else {
        "~/.local/opt/PassBundle"
    }
}

/// Why an update can't take the place of `installed`, in the person's
/// words, or None when it can: it's a release for this platform, alone in
/// a folder of its own (nothing there but what its release put there, and
/// not where PassBundle keeps its vault, `data_dir`), which this person can
/// change, as they can the folder it's in.
pub fn why_not(installed: &Installed, platform: &str, data_dir: &Path) -> Option<String> {
    let Some(release) = installed.release_for(platform) else {
        return Some(
            "This copy of PassBundle was built on a computer, not by PassBundle's releases, so it can't update itself. Download the new version from GitHub instead."
                .into(),
        );
    };
    let dir = &installed.dir;
    let shown = dir.display();
    let own_folder = format!(
        "PassBundle's folder ({shown}) has other files in it, so it can't update itself. Put PassBundle in a folder of its own, such as {}, and open it from there.",
        suggested_place()
    );
    if same_or_inside(data_dir, dir) {
        return Some(own_folder);
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return Some(format!(
            "PassBundle can't read its folder ({shown}), so it can't update itself."
        ));
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let expected = release.files.iter().any(|f| {
            if cfg!(windows) {
                f.eq_ignore_ascii_case(&name)
            } else {
                *f == name
            }
        });
        if !expected && !harmless(&name) {
            return Some(own_folder);
        }
    }
    let admin = format!(
        "PassBundle is in a folder only an administrator can change ({shown}), so it can't update itself. Update it the way it was installed."
    );
    let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return Some(admin);
    };
    if !writable(dir) || !writable(parent) {
        return Some(admin);
    }
    None
}

fn same_or_inside(path: &Path, folder: &Path) -> bool {
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let folder = fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
    path.starts_with(&folder)
}

/// Whether this process can make (and remove) a file in `dir`: what an
/// update needs, whatever the folder's permissions say.
fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".passbundle-write-check-{}", std::process::id()));
    let made = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .is_ok();
    if made {
        let _ = fs::remove_file(&probe);
    }
    made
}

/// What's wrong with the unpacked update at `staged` (the release's own
/// `PassBundle` folder), or None: it's PassBundle at `version` for
/// `platform`, with everything a release has.
pub fn check_staged(staged: &Path, version: &str, platform: &str) -> Option<String> {
    let Some(release) = read_release(staged) else {
        return Some("The download isn't a PassBundle release, so it wasn't installed.".into());
    };
    if release.app != APP || release.platform != platform {
        return Some(
            "The download isn't PassBundle for this computer, so it wasn't installed.".into(),
        );
    }
    if release.version != version {
        return Some(
            "The download isn't the version its release says, so it wasn't installed.".into(),
        );
    }
    for name in required() {
        if !staged.join(&name).is_file() {
            return Some(format!(
                "The download is missing {name}, so it wasn't installed."
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pb-bundle-{name}-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub fn release_json(version: &str, platform: &str, files: &[&str]) -> String {
        serde_json::json!({
            "app": APP, "version": version, "platform": platform, "files": files,
        })
        .to_string()
    }

    fn install(base: &Path, extra: &[&str]) -> Installed {
        let dir = base.join("PassBundle");
        fs::create_dir_all(dir.join("data")).unwrap();
        fs::write(dir.join(program_name()), b"program").unwrap();
        fs::write(
            dir.join(RELEASE),
            release_json("0.1.4", "linux-x64", &[program_name(), "data", RELEASE]),
        )
        .unwrap();
        for name in extra {
            fs::write(dir.join(name), b"").unwrap();
        }
        Installed::at(&dir.join(program_name()))
    }

    #[test]
    fn a_release_alone_in_its_own_folder_can_be_replaced() {
        let base = temp("ok");
        let installed = install(&base, &["desktop.ini"]);
        assert_eq!(installed.build("linux-x64"), 4);
        assert_eq!(why_not(&installed, "linux-x64", &base.join("data")), None);
        assert_eq!(work_dir(&installed.dir), base.join(".PassBundle-update"));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_copy_that_isnt_a_release_or_shares_its_folder_cant_be() {
        let base = temp("no");
        let installed = install(&base, &[]);
        // Another platform's release.json, or none: built on a computer.
        assert_eq!(installed.build("windows-x64"), 0);
        assert!(why_not(&installed, "windows-x64", &base.join("data"))
            .unwrap()
            .contains("built on a computer"));
        // Someone's own file, or the vault, in the folder.
        fs::write(installed.dir.join("my notes.txt"), b"").unwrap();
        assert!(why_not(&installed, "linux-x64", &base.join("data"))
            .unwrap()
            .contains("other files"));
        fs::remove_file(installed.dir.join("my notes.txt")).unwrap();
        assert!(why_not(&installed, "linux-x64", &installed.dir)
            .unwrap()
            .contains("other files"));
        assert!(
            why_not(&installed, "linux-x64", &installed.dir.join("data"))
                .unwrap()
                .contains("other files")
        );
        fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_this_person_cant_change_isnt_replaced() {
        use std::os::unix::fs::PermissionsExt;
        // Root can change anything, so this tells nothing there.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let base = temp("ro");
        let installed = install(&base, &[]);
        fs::set_permissions(&base, fs::Permissions::from_mode(0o555)).unwrap();
        let why = why_not(&installed, "linux-x64", Path::new("/nonexistent-data"));
        fs::set_permissions(&base, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(why.unwrap().contains("only an administrator"));
        fs::set_permissions(&installed.dir, fs::Permissions::from_mode(0o555)).unwrap();
        let why = why_not(&installed, "linux-x64", Path::new("/nonexistent-data"));
        fs::set_permissions(&installed.dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(why.unwrap().contains("only an administrator"));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn an_update_must_be_the_release_it_says() {
        let base = temp("staged");
        let staged = base.join("PassBundle");
        fs::create_dir_all(&staged).unwrap();
        for name in required() {
            let path = staged.join(&name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"x").unwrap();
        }
        fs::write(
            staged.join(RELEASE),
            release_json("0.1.5", "linux-x64", &[]),
        )
        .unwrap();
        assert_eq!(check_staged(&staged, "0.1.5", "linux-x64"), None);
        assert!(check_staged(&staged, "0.1.6", "linux-x64").is_some());
        assert!(check_staged(&staged, "0.1.5", "linux-arm64").is_some());
        fs::remove_file(staged.join(program_name())).unwrap();
        assert!(check_staged(&staged, "0.1.5", "linux-x64").is_some());
        fs::remove_dir_all(&base).unwrap();
    }
}
