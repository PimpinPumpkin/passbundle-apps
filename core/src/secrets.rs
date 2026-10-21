//! Where the refresh token is kept: the OS's secret store, Keychain (macOS,
//! iOS), Credential Manager (Windows), or Secret Service (Linux). Where
//! there's none, such as a Linux desktop without a keyring running, nothing is
//! kept and the person signs in again next time. On Android it's sealed with
//! a key in the Keystore and kept in the app's own folder (docs/phones.md). Tests use a store in memory,
//! so they never touch the real one.

use std::collections::HashMap;
use std::path::Path;
#[cfg(target_os = "android")]
use std::path::PathBuf;
use std::sync::Mutex;

use zeroize::Zeroizing;

pub trait SecretStore: Send {
    /// Keeps the refresh token; returns whether it was kept.
    fn keep(&self, server: &str, email: &str, refresh_token: &str) -> bool;
    fn get(&self, server: &str, email: &str) -> Option<Zeroizing<String>>;
    fn forget(&self, server: &str, email: &str);
}

#[cfg(not(target_os = "android"))]
const SERVICE: &str = "PassBundle";

fn account(server: &str, email: &str) -> String {
    format!("{email} on {server}")
}

/// The OS's own secret store.
pub struct OsSecretStore {
    /// Where Android keeps the sealed tokens: the app's own folder.
    #[cfg(target_os = "android")]
    dir: PathBuf,
}

impl OsSecretStore {
    /// The OS's store, for the app whose data folder is `dir` (Android keeps
    /// the sealed tokens there; the others' stores are the OS's own).
    pub fn new(dir: &Path) -> OsSecretStore {
        #[cfg(not(target_os = "android"))]
        let _ = dir;
        OsSecretStore {
            #[cfg(target_os = "android")]
            dir: dir.join("tokens"),
        }
    }
}

#[cfg(target_os = "android")]
impl OsSecretStore {
    /// One file per account, named by a hash so the email isn't in the name.
    fn path(&self, server: &str, email: &str) -> PathBuf {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(account(server, email).as_bytes());
        let name: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
        self.dir.join(format!("{name}.sealed"))
    }
}

#[cfg(target_os = "android")]
impl SecretStore for OsSecretStore {
    fn keep(&self, server: &str, email: &str, refresh_token: &str) -> bool {
        let Some(sealed) = crate::android::seal(refresh_token.as_bytes()) else {
            return false;
        };
        std::fs::create_dir_all(&self.dir).is_ok()
            && std::fs::write(self.path(server, email), sealed).is_ok()
    }

    fn get(&self, server: &str, email: &str) -> Option<Zeroizing<String>> {
        let sealed = std::fs::read(self.path(server, email)).ok()?;
        let plain = crate::android::open(&sealed)?;
        std::str::from_utf8(&plain)
            .ok()
            .map(|token| Zeroizing::new(token.to_owned()))
    }

    fn forget(&self, server: &str, email: &str) {
        let _ = std::fs::remove_file(self.path(server, email));
    }
}

#[cfg(not(target_os = "android"))]
impl OsSecretStore {
    fn entry(server: &str, email: &str) -> Option<keyring::Entry> {
        keyring::Entry::new(SERVICE, &account(server, email)).ok()
    }
}

#[cfg(not(target_os = "android"))]
impl SecretStore for OsSecretStore {
    fn keep(&self, server: &str, email: &str, refresh_token: &str) -> bool {
        Self::entry(server, email).is_some_and(|e| e.set_password(refresh_token).is_ok())
    }

    fn get(&self, server: &str, email: &str) -> Option<Zeroizing<String>> {
        Self::entry(server, email)?
            .get_password()
            .ok()
            .map(Zeroizing::new)
    }

    fn forget(&self, server: &str, email: &str) {
        if let Some(e) = Self::entry(server, email) {
            let _ = e.delete_credential();
        }
    }
}

/// Kept in memory only, for tests.
#[derive(Default)]
pub struct MemorySecretStore(Mutex<HashMap<String, Zeroizing<String>>>);

impl SecretStore for MemorySecretStore {
    fn keep(&self, server: &str, email: &str, refresh_token: &str) -> bool {
        self.0.lock().unwrap().insert(
            account(server, email),
            Zeroizing::new(refresh_token.to_owned()),
        );
        true
    }

    fn get(&self, server: &str, email: &str) -> Option<Zeroizing<String>> {
        self.0.lock().unwrap().get(&account(server, email)).cloned()
    }

    fn forget(&self, server: &str, email: &str) {
        self.0.lock().unwrap().remove(&account(server, email));
    }
}
