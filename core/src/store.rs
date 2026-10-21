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
use crate::kdf::Prelogin;

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

impl Store {
    /// The app's own folder: PB_DATA_DIR if set (tests), else the OS's local
    /// app data folder (AppData\Local on Windows, Application Support on macOS,
    /// ~/.local/share on Linux), in "PassBundle".
    pub fn default_dir() -> PathBuf {
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

    /// Writes a file whole or not at all, readable only by this user.
    fn write(&self, name: &str, text: &str) -> Result<()> {
        fs::create_dir_all(&self.dir)
            .map_err(|e| Error::Malformed(format!("can't make {}: {e}", self.dir.display())))?;
        let target = self.path(name);
        let partial = self.path(&format!("{name}.partial"));
        fs::write(&partial, text)
            .map_err(|e| Error::Malformed(format!("can't write {}: {e}", partial.display())))?;
        restrict(&partial);
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

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

#[cfg(unix)]
fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict(_path: &Path) {
    // The user's own AppData folder is already private to them.
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
        store.forget_vault();
        assert!(store.sync().is_none());
        assert!(store.settings().is_some());
        // The choices are this device's, kept for whoever signs in next.
        assert_eq!(store.preferences(), chosen);
        let _ = fs::remove_dir_all(dir);
    }
}
