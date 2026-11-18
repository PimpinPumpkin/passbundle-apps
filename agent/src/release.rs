//! A Windows release's folder: the agent, its native-messaging host, and the
//! app, with `release.json`, which the release's build writes (the app's
//! `tools/release-folder.sh`): which release it is, and every path in the
//! folder, folders too, with `/` between names. The update task reads the
//! installed version from it, and an update must hold exactly what its own
//! lists. The app's updater reads the same file (the core's
//! `src/updates/bundle.rs`).

use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::version::Version;

pub const RELEASE: &str = "release.json";
/// The app's ID, as releases name it.
pub const APP: &str = "com.passbundle.app";
/// The agent's program, which the update task runs.
pub const AGENT: &str = "passbundle-agent.exe";
/// What every release has: the agent, its host, the app, and the app's core.
pub const REQUIRED: &[&str] = &[
    AGENT,
    "passbundle-agent-host.exe",
    "passbundle.exe",
    "passbundle_core.dll",
    RELEASE,
];

/// The most entries a release's folder has.
const MAX_ENTRIES: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReleaseInfo {
    pub app: String,
    pub version: String,
    pub platform: String,
    pub files: Vec<String>,
}

pub fn read(dir: &Path) -> Option<ReleaseInfo> {
    let bytes = fs::read(dir.join(RELEASE)).ok()?;
    if bytes.len() > 4 << 20 {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

/// Every path in `dir`, `/` between names, folders too, never following a
/// link; None when there's a link (no release has one), more than a release
/// could have, or something unreadable.
pub fn listing(dir: &Path) -> Option<Vec<String>> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Option<()> {
        for entry in fs::read_dir(dir).ok()? {
            let entry = entry.ok()?;
            let name = entry.file_name().into_string().ok()?;
            let path = if prefix.is_empty() {
                name
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

/// Windows' names: the same whatever the case.
fn key(path: &str) -> String {
    path.to_lowercase()
}

/// The version of the release installed in `dir`, for `platform`; or why
/// the update task leaves it alone.
pub fn installed(dir: &Path, platform: &str) -> Result<Version, String> {
    let info = read(dir)
        .filter(|r| r.app == APP && r.platform == platform)
        .ok_or_else(|| {
            format!(
                "{} wasn't installed from one of PassBundle's releases (it has no {RELEASE} for {platform}), so the update task leaves it alone. Install a release with the install script.",
                dir.display()
            )
        })?;
    Version::parse(&info.version).ok_or_else(|| {
        format!(
            "{} doesn't say which version it is, so the update task leaves it alone.",
            dir.display()
        )
    })
}

/// Why the unpacked update at `staged` (the release's own `PassBundle`
/// folder) can't go in, or Ok: it's PassBundle at `version` for `platform`,
/// with the agent, its host, the app, and its core, and exactly what its
/// `release.json` lists, none of it a name of `kept` (what the install
/// script writes into the folder itself, which an update leaves alone).
pub fn check(
    staged: &Path,
    version: Version,
    platform: &str,
    kept: &[String],
) -> Result<(), String> {
    let info = read(staged).ok_or("the download isn't a PassBundle release")?;
    if info.app != APP || info.platform != platform {
        return Err("the download isn't PassBundle for this computer".into());
    }
    if Version::parse(&info.version) != Some(version) {
        return Err("the download isn't the version its manifest says".into());
    }
    for name in REQUIRED {
        if !staged.join(name).is_file() {
            return Err(format!("the download is missing {name}"));
        }
    }
    let mut listed: Vec<String> = info.files.iter().map(|f| key(f)).collect();
    listed.sort();
    listed.dedup();
    let mut found: Vec<String> = listing(staged)
        .unwrap_or_default()
        .iter()
        .map(|f| key(f))
        .collect();
    found.sort();
    if found.is_empty() || found != listed {
        return Err("the download doesn't hold exactly what its release.json lists".into());
    }
    if let Some(name) = kept.iter().find(|k| listed.contains(&key(k))) {
        return Err(format!(
            "the download has {name}, which the install script writes"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pb-agent-release-{name}-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A release's folder at `version`, as release-folder.sh writes one.
    fn folder(dir: &Path, version: &str, platform: &str) {
        for name in REQUIRED
            .iter()
            .chain(&["data/flutter_assets/AssetManifest.bin"])
        {
            let path = dir.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, format!("{name} {version}")).unwrap();
        }
        let files = listing(dir).unwrap();
        let info = serde_json::json!({
            "app": APP, "version": version, "platform": platform, "files": files,
        });
        fs::write(dir.join(RELEASE), info.to_string()).unwrap();
    }

    const KEPT: &[String] = &[];

    #[test]
    fn an_update_is_the_release_it_says_holding_exactly_what_it_lists() {
        let base = temp("check");
        let staged = base.join("PassBundle");
        folder(&staged, "0.1.5", "windows-x64");
        let v = Version::new(0, 1, 5);
        assert_eq!(check(&staged, v, "windows-x64", KEPT), Ok(()));
        assert_eq!(installed(&staged, "windows-x64"), Ok(v));
        // Another version, or platform.
        assert!(check(&staged, Version::new(0, 1, 6), "windows-x64", KEPT).is_err());
        assert!(check(&staged, v, "linux-x64", KEPT).is_err());
        assert!(installed(&staged, "linux-x64").is_err());
        // Something it doesn't list.
        fs::write(staged.join("extra.dll"), b"x").unwrap();
        assert!(check(&staged, v, "windows-x64", KEPT)
            .unwrap_err()
            .contains("exactly what"));
        fs::remove_file(staged.join("extra.dll")).unwrap();
        // Something it lists, but doesn't have.
        fs::remove_file(staged.join("data/flutter_assets/AssetManifest.bin")).unwrap();
        assert!(check(&staged, v, "windows-x64", KEPT)
            .unwrap_err()
            .contains("exactly what"));
        // A program every release has.
        fs::remove_file(staged.join("passbundle-agent-host.exe")).unwrap();
        assert!(check(&staged, v, "windows-x64", KEPT)
            .unwrap_err()
            .contains("missing passbundle-agent-host.exe"));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn an_update_never_brings_what_the_install_script_writes() {
        let base = temp("kept");
        let staged = base.join("PassBundle");
        fs::create_dir_all(&staged).unwrap();
        fs::write(staged.join("com.passbundle.agent.json"), b"{}").unwrap();
        folder(&staged, "0.1.5", "windows-x64");
        let kept = vec!["com.passbundle.agent.json".to_owned()];
        assert!(check(&staged, Version::new(0, 1, 5), "windows-x64", &kept)
            .unwrap_err()
            .contains("install script"));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_folder_without_a_release_json_isnt_updated() {
        let base = temp("none");
        assert!(installed(&base, "windows-x64")
            .unwrap_err()
            .contains("wasn't installed from one of PassBundle's releases"));
        fs::remove_dir_all(&base).unwrap();
    }
}
