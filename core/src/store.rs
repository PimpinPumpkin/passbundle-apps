//! What the app keeps on this device (docs/core.md, "What's kept where"):
//! the server, email, KDF settings, and this install's device identifier, the
//! person's choices for locking, and the last sync as the server sent it (its
//! secrets still encrypted). The
//! refresh token goes to the OS's secret store instead (`secrets`). Nothing
//! decrypted is ever written.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::autolock::Preferences;
use crate::error::{Error, Result};
use crate::generator;
use crate::kdf::Prelogin;
use crate::synced_settings;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub server: String,
    pub email: String,
    pub kdf: u8,
    pub kdf_iterations: u32,
    pub kdf_memory: Option<u32>,
    pub kdf_parallelism: Option<u32>,
    pub device_identifier: String,
}

impl Settings {
    pub fn prelogin(&self) -> Prelogin {
        Prelogin {
            kdf: self.kdf,
            kdf_iterations: self.kdf_iterations,
            kdf_memory: self.kdf_memory,
            kdf_parallelism: self.kdf_parallelism,
        }
    }
}

pub struct Store {
    dir: PathBuf,
}

/// How the app looks, kept on this device: whether logins show their
/// websites' icons from the server (upstream's "Show website icons", on
/// until turned off).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Appearance {
    pub show_icons: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance { show_icons: true }
    }
}

/// Approving a sign-in from another device, kept on this device
/// (docs/core.md, "Approving a sign-in"): whether approving asks for
/// biometrics or the master password first (on until turned off).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Approvals {
    pub confirm: bool,
}

impl Default for Approvals {
    fn default() -> Self {
        Approvals { confirm: true }
    }
}

impl Store {
    /// The app's own folder: the OS's local app data folder (AppData\Local on
    /// Windows, Application Support on macOS, ~/.local/share on Linux), in
    /// "PassBundle". Test builds (the test-hooks feature) take PB_DATA_DIR
    /// instead when it's set.
    pub fn default_dir() -> PathBuf {
        #[cfg(feature = "test-hooks")]
        if let Some(dir) = std::env::var_os("PB_DATA_DIR") {
            return PathBuf::from(dir);
        }
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("PassBundle")
    }

    pub fn new(dir: impl Into<PathBuf>) -> Store {
        Store { dir: dir.into() }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// Writes a file whole or not at all, readable only by this user from the
    /// moment it's made, in a folder only they can open.
    fn write(&self, name: &str, text: &str) -> Result<()> {
        private_dir(&self.dir)
            .map_err(|e| Error::Malformed(format!("can't make {}: {e}", self.dir.display())))?;
        let target = self.path(name);
        let partial = self.path(&format!("{name}.partial"));
        write_new(&partial, text.as_bytes())
            .map_err(|e| Error::Malformed(format!("can't write {}: {e}", partial.display())))?;
        fs::rename(&partial, &target)
            .map_err(|e| Error::Malformed(format!("can't write {}: {e}", target.display())))
    }

    fn read(&self, name: &str) -> Option<String> {
        fs::read_to_string(self.path(name)).ok()
    }

    pub fn settings(&self) -> Option<Settings> {
        serde_json::from_str(&self.read("settings.json")?).ok()
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        self.write(
            "settings.json",
            &serde_json::to_string_pretty(settings).expect("settings always serialize"),
        )
    }

    /// When to lock by itself; the defaults until the person chooses.
    pub fn preferences(&self) -> Preferences {
        self.read("preferences.json")
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save_preferences(&self, preferences: &Preferences) -> Result<()> {
        self.write(
            "preferences.json",
            &serde_json::to_string_pretty(preferences).expect("preferences always serialize"),
        )
    }

    /// The generator's last choices; the defaults until the person makes one.
    pub fn generator_options(&self) -> generator::Options {
        self.read("generator.json")
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save_generator_options(&self, options: &generator::Options) -> Result<()> {
        self.write(
            "generator.json",
            &serde_json::to_string_pretty(options).expect("options always serialize"),
        )
    }

    /// What the server has of the generator's settings, as far as this
    /// device knows (synced_settings).
    pub fn synced_settings(&self) -> Option<synced_settings::Known> {
        serde_json::from_str(&self.read("generator-synced.json")?).ok()
    }

    pub fn save_synced_settings(&self, known: &synced_settings::Known) -> Result<()> {
        self.write(
            "generator-synced.json",
            &serde_json::to_string_pretty(known).expect("settings always serialize"),
        )
    }

    /// The last sync, as the server sent it.
    pub fn sync(&self) -> Option<String> {
        self.read("sync.json")
    }

    pub fn save_sync(&self, text: &str) -> Result<()> {
        self.write("sync.json", text)
    }

    /// Signing out forgets the vault; the server and email stay, to sign in again.
    pub fn forget_vault(&self) {
        let _ = fs::remove_file(self.path("sync.json"));
    }

    /// How the app looks on this device (`Appearance`); the defaults until
    /// the person chooses.
    pub fn appearance(&self) -> Appearance {
        self.read("appearance.json")
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save_appearance(&self, appearance: &Appearance) -> Result<()> {
        self.write(
            "appearance.json",
            &serde_json::to_string_pretty(appearance).expect("appearance always serializes"),
        )
    }

    /// Approving sign-ins on this device (`Approvals`); the defaults until
    /// the person chooses.
    pub fn approvals(&self) -> Approvals {
        self.read("approvals.json")
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save_approvals(&self, approvals: &Approvals) -> Result<()> {
        self.write(
            "approvals.json",
            &serde_json::to_string_pretty(approvals).expect("approvals always serialize"),
        )
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// Makes the app's folder, only this person's to open (mode 0700 on Linux
/// and macOS; the folders above it as the OS makes them). A folder of theirs
/// that's open to others, as earlier builds made it (0755, which others can
/// open where the home folder is 755, as on many Linux computers), is closed
/// to them too. On Windows the person's own AppData folder is already private
/// to them.
#[cfg(unix)]
fn private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    if let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    match fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    // Opened, so what's checked is what's changed.
    let folder = fs::File::open(dir)?;
    let meta = folder.metadata()?;
    // SAFETY: geteuid has no preconditions and can't fail.
    let mine = meta.uid() == unsafe { libc::geteuid() };
    if meta.is_dir() && mine && meta.mode() & 0o7777 != 0o700 {
        folder.set_permissions(fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn private_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)
}

/// Writes `bytes` to a new file at `path`, made readable only by this user
/// (mode 0600 on Linux and macOS, never wider for a moment). One left by a
/// write that stopped part way is removed first, so nothing that was there,
/// a link to elsewhere included, is ever written through. A file that can't
/// be written whole is removed.
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = None;
    for _ in 0..2 {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        match options.open(path) {
            Ok(opened) => {
                file = Some(opened);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    let mut file = file.ok_or_else(|| std::io::Error::from(std::io::ErrorKind::AlreadyExists))?;
    let written = file.write_all(bytes);
    drop(file);
    if written.is_err() {
        let _ = fs::remove_file(path);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_settings_choices_and_the_sync_and_forgets_the_vault_on_sign_out() {
        let dir = std::env::temp_dir().join(format!("pb-store-{}", uuid::Uuid::new_v4()));
        let store = Store::new(&dir);
        assert!(store.settings().is_none());
        let settings = Settings {
            server: "https://vault.example.test".into(),
            email: "ana@example.test".into(),
            kdf: 0,
            kdf_iterations: 600_000,
            kdf_memory: None,
            kdf_parallelism: None,
            device_identifier: "d".into(),
        };
        store.save_settings(&settings).unwrap();
        store.save_sync("{\"profile\":{}}").unwrap();
        assert_eq!(store.settings(), Some(settings));
        assert_eq!(store.sync().as_deref(), Some("{\"profile\":{}}"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.join("sync.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert_eq!(store.preferences(), Preferences::default());
        let chosen = Preferences {
            lock_after_minutes: None,
            lock_on_sleep: false,
        };
        store.save_preferences(&chosen).unwrap();
        assert_eq!(store.generator_options(), generator::Options::default());
        let generating = generator::Options {
            kind: generator::Kind::Passphrase,
            words: 4,
            ..generator::Options::default()
        };
        store.save_generator_options(&generating).unwrap();
        store.forget_vault();
        assert!(store.sync().is_none());
        assert!(store.settings().is_some());
        // The choices are this device's, kept for whoever signs in next.
        assert_eq!(store.preferences(), chosen);
        assert_eq!(store.generator_options(), generating);
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
    }

    #[cfg(unix)]
    #[test]
    fn makes_its_folder_and_files_only_the_persons_from_the_start() {
        let parent = std::env::temp_dir().join(format!("pb-store-{}", uuid::Uuid::new_v4()));
        let dir = parent.join("PassBundle");
        let store = Store::new(&dir);
        store.save_sync("{}").unwrap();
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("sync.json")), 0o600);
        // The new file is 0600 as it's made, before anything is in it: there's
        // no change of mode after.
        let partial = dir.join("made.partial");
        write_new(&partial, b"").unwrap();
        assert_eq!(mode(&partial), 0o600);
        let _ = fs::remove_dir_all(parent);
    }

    #[cfg(unix)]
    #[test]
    fn closes_a_folder_an_earlier_build_left_open() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("pb-store-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        let store = Store::new(&dir);
        store
            .save_settings(&Settings {
                server: "https://vault.example.test".into(),
                email: "ana@example.test".into(),
                kdf: 0,
                kdf_iterations: 600_000,
                kdf_memory: None,
                kdf_parallelism: None,
                device_identifier: "d".into(),
            })
            .unwrap();
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("settings.json")), 0o600);
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn never_writes_through_what_a_stopped_write_left() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("pb-store-{}", uuid::Uuid::new_v4()));
        let store = Store::new(&dir);
        store.save_sync("{\"old\":1}").unwrap();
        // A partial file left readable by others, and one that's a link to a
        // file elsewhere: neither is written into.
        let elsewhere = std::env::temp_dir().join(format!("pb-elsewhere-{}", uuid::Uuid::new_v4()));
        fs::write(&elsewhere, "theirs").unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.join("sync.json.partial")).unwrap();
        store.save_sync("{\"new\":2}").unwrap();
        assert_eq!(fs::read_to_string(&elsewhere).unwrap(), "theirs");
        assert_eq!(store.sync().as_deref(), Some("{\"new\":2}"));
        assert_eq!(mode(&dir.join("sync.json")), 0o600);
        fs::write(dir.join("generator.json.partial"), "left").unwrap();
        fs::set_permissions(
            dir.join("generator.json.partial"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        store
            .save_generator_options(&generator::Options::default())
            .unwrap();
        assert_eq!(mode(&dir.join("generator.json")), 0o600);
        assert!(!dir.join("generator.json.partial").exists());
        let _ = fs::remove_file(elsewhere);
        let _ = fs::remove_dir_all(dir);
    }
}
