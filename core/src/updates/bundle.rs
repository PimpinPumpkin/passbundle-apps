//! The copy of PassBundle being updated: its folder, the release it is
//! (`release.json`, which each release's build writes into it), and whether
//! an update can take its place.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::access;
use super::manifest::{Version, APP};

/// What a release's build writes into the app's folder: which release this
/// is, and every path in the folder (`/` between names), the only ones an
/// update may move or remove (tools/release-folder.sh).
pub const RELEASE: &str = "release.json";

/// The most entries a release's folder has.
const MAX_ENTRIES: usize = 100_000;

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
    if bytes.len() > 4 << 20 {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

/// Every path in `dir`, `/` between names, folders too, never following a
/// link; None when there's a link (no release has one), more than a
/// release could have, or something unreadable.
pub fn listing(dir: &Path) -> Option<Vec<String>> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Option<()> {
        for entry in fs::read_dir(dir).ok()? {
            let entry = entry.ok()?;
            let name = entry.file_name().into_string().ok()?;
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let kind = entry.file_type().ok()?;
            if kind.is_symlink() || out.len() >= MAX_ENTRIES {
                return None;
            }
            out.push(path.clone());
            if kind.is_dir() {
                walk(&entry.path(), &path, out)?;
            }
        }
        Some(())
    }
    let mut out = Vec::new();
    walk(dir, "", &mut out)?;
    out.sort();
    Some(out)
}

fn key(path: &str) -> String {
    if cfg!(windows) {
        path.to_lowercase()
    } else {
        path.to_owned()
    }
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
/// words, or None when it can:
///
/// - it's a release for this platform;
/// - PassBundle isn't running as an administrator or root (`trust_admin`,
///   for tests with test hooks on CI's machines, which run elevated, lets it);
/// - it isn't in one of the OS's own places (Program Files, /opt, ...);
/// - its folder holds exactly what its release put there, every path
///   `release.json` lists and nothing else (so not the person's own files,
///   nor the folder PassBundle keeps the vault in, `data_dir`), with no
///   links;
/// - that folder, and the one it's in, are this person's alone (`access`):
///   no one else can change either;
/// - and this process can make files in both.
pub fn why_not(
    installed: &Installed,
    platform: &str,
    data_dir: &Path,
    trust_admin: bool,
) -> Option<String> {
    let Some(release) = installed.release_for(platform) else {
        return Some(
            "This copy of PassBundle was built on a computer, not by PassBundle's releases, so it can't update itself. Download the new version from GitHub instead."
                .into(),
        );
    };
    let dir = &installed.dir;
    let shown = dir.display();
    let installed_way = format!(
        "PassBundle is in a folder only an administrator can change ({shown}), so it can't update itself. Update it the way it was installed."
    );
    if access::elevated() && !trust_admin {
        return Some(
            "PassBundle is running as an administrator, so it doesn't replace itself. Update it the way it was installed, or open PassBundle as yourself to update it."
                .into(),
        );
    }
    if access::system_place(dir) {
        return Some(installed_way);
    }
    let own_folder = format!(
        "PassBundle's folder ({shown}) has other files in it, so it can't update itself. Put PassBundle in a folder of its own, such as {}, and open it from there.",
        suggested_place()
    );
    if same_or_inside(data_dir, dir) {
        return Some(own_folder);
    }
    let Some(found) = listing(dir) else {
        return Some(own_folder);
    };
    let listed: HashSet<String> = release.files.iter().map(|f| key(f)).collect();
    let unknown = found.iter().any(|path| {
        let name = path.rsplit('/').next().unwrap_or(path);
        !listed.contains(&key(path)) && !harmless(name)
    });
    if unknown {
        return Some(own_folder);
    }
    let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return Some(installed_way);
    };
    for folder in [dir.as_path(), parent] {
        if let Some(why) = access::not_theirs(folder, trust_admin) {
            return Some(format!(
                "PassBundle can't update itself where it is ({}: {why}). Put PassBundle in a folder of your own, such as {}, and open it from there, or update it the way it was installed.",
                folder.display(),
                suggested_place()
            ));
        }
    }
    if !writable(dir) || !writable(parent) {
        return Some(installed_way);
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
/// `platform`, with everything a release has, and exactly what its
/// `release.json` lists (so the next update can take its place too).
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
    let mut listed: Vec<String> = release.files.iter().map(|f| key(f)).collect();
    listed.sort();
    listed.dedup();
    let mut found: Vec<String> = listing(staged)
        .unwrap_or_default()
        .iter()
        .map(|f| key(f))
        .collect();
    found.sort();
    if found.is_empty() || found != listed {
        return Some(
            "The download doesn't hold what its release says, so it wasn't installed.".into(),
        );
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
        // As a person's own folders are, whatever the umask.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        }
        dir
    }

    pub fn release_json(version: &str, platform: &str, files: &[String]) -> String {
        serde_json::json!({
            "app": APP, "version": version, "platform": platform, "files": files,
        })
        .to_string()
    }

    /// A release's folder at 0.1.4, as release-folder.sh writes it: its
    /// release.json lists every path in it.
    fn install(base: &Path, extra: &[&str]) -> Installed {
        let dir = base.join("PassBundle");
        fs::create_dir_all(dir.join("data/flutter_assets")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::write(dir.join(program_name()), b"program").unwrap();
        fs::write(dir.join("data/flutter_assets/AssetManifest.bin"), b"assets").unwrap();
        fs::write(dir.join(RELEASE), b"").unwrap();
        let files = listing(&dir).unwrap();
        fs::write(
            dir.join(RELEASE),
            release_json("0.1.4", "linux-x64", &files),
        )
        .unwrap();
        for name in extra {
            fs::write(dir.join(name), b"").unwrap();
        }
        Installed::at(&dir.join(program_name()))
    }

    /// CI's Windows machines run tests elevated: there, as an administrator.
    fn why(installed: &Installed, data: &Path) -> Option<String> {
        why_not(installed, "linux-x64", data, cfg!(windows))
    }

    #[test]
    fn a_release_alone_in_its_own_folder_can_be_replaced() {
        let base = temp("ok");
        let installed = install(&base, &["desktop.ini", "data/Thumbs.db"]);
        assert_eq!(installed.build("linux-x64"), 4);
        assert_eq!(why(&installed, &base.join("data")), None);
        assert_eq!(work_dir(&installed.dir), base.join(".PassBundle-update"));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_copy_that_isnt_a_release_or_shares_its_folder_cant_be() {
        let base = temp("no");
        let installed = install(&base, &[]);
        // Another platform's release.json, or none: built on a computer.
        assert_eq!(installed.build("windows-x64"), 0);
        assert!(
            why_not(&installed, "windows-x64", &base.join("data"), false)
                .unwrap()
                .contains("built on a computer")
        );
        // Someone's own file, anywhere in it, or the vault in it.
        for mine in [
            "my notes.txt",
            "data/my notes.txt",
            "data/flutter_assets/mine",
        ] {
            fs::write(installed.dir.join(mine), b"").unwrap();
            assert!(
                why(&installed, &base.join("data"))
                    .unwrap()
                    .contains("other files"),
                "{mine}"
            );
            fs::remove_file(installed.dir.join(mine)).unwrap();
        }
        fs::create_dir(installed.dir.join("data/mine")).unwrap();
        assert!(why(&installed, &base.join("data"))
            .unwrap()
            .contains("other files"));
        fs::remove_dir(installed.dir.join("data/mine")).unwrap();
        assert!(why(&installed, &installed.dir)
            .unwrap()
            .contains("other files"));
        assert!(why(&installed, &installed.dir.join("data"))
            .unwrap()
            .contains("other files"));
        fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_the_folder_isnt_moved_with_it() {
        let base = temp("link");
        let installed = install(&base, &[]);
        std::os::unix::fs::symlink(&base, installed.dir.join("data/elsewhere")).unwrap();
        assert!(why(&installed, &base.join("data"))
            .unwrap()
            .contains("other files"));
        fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_this_person_cant_change_or_others_can_isnt_replaced() {
        use std::os::unix::fs::PermissionsExt;
        // Root can change anything, so this tells nothing there.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let base = temp("ro");
        let installed = install(&base, &[]);
        let data = Path::new("/nonexistent-data");
        fs::set_permissions(&base, fs::Permissions::from_mode(0o555)).unwrap();
        let read_only = why(&installed, data);
        fs::set_permissions(&base, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(read_only.unwrap().contains("only an administrator"));
        fs::set_permissions(&installed.dir, fs::Permissions::from_mode(0o555)).unwrap();
        let read_only = why(&installed, data);
        fs::set_permissions(&installed.dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(read_only.unwrap().contains("only an administrator"));
        // In a folder anyone can change (as /tmp), or that is itself.
        for (folder, mode) in [(&base, 0o1777), (&installed.dir, 0o777)] {
            fs::set_permissions(folder, fs::Permissions::from_mode(mode)).unwrap();
            let shared = why(&installed, data);
            fs::set_permissions(folder, fs::Permissions::from_mode(0o755)).unwrap();
            assert!(
                shared
                    .as_deref()
                    .unwrap_or("")
                    .contains("anyone can change it"),
                "{shared:?}"
            );
        }
        assert_eq!(why(&installed, data), None);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn an_update_must_be_the_release_it_says_holding_what_it_says() {
        let base = temp("staged");
        let staged = base.join("PassBundle");
        fs::create_dir_all(&staged).unwrap();
        for name in required() {
            let path = staged.join(&name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"x").unwrap();
        }
        let files = listing(&staged).unwrap();
        fs::write(
            staged.join(RELEASE),
            release_json("0.1.5", "linux-x64", &files),
        )
        .unwrap();
        assert_eq!(check_staged(&staged, "0.1.5", "linux-x64"), None);
        assert!(check_staged(&staged, "0.1.6", "linux-x64").is_some());
        assert!(check_staged(&staged, "0.1.5", "linux-arm64").is_some());
        // Something it doesn't list.
        fs::write(staged.join("extra"), b"x").unwrap();
        assert!(check_staged(&staged, "0.1.5", "linux-x64").is_some());
        fs::remove_file(staged.join("extra")).unwrap();
        fs::remove_file(staged.join(program_name())).unwrap();
        assert!(check_staged(&staged, "0.1.5", "linux-x64").is_some());
        fs::remove_dir_all(&base).unwrap();
    }
}
