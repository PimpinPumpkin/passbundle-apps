//! Where the refresh token is kept: the OS's secret store, Keychain (macOS,
//! iOS), Credential Manager (Windows), or Secret Service (Linux). Where
//! there's none, such as a Linux desktop without a keyring running, nothing is
//! kept and the person signs in again next time. On Android it's sealed with
//! a key in the Keystore and kept in the app's own folder (docs/phones.md). Tests use a store in memory,
//! so they never touch the real one.
//!
//! Beside each refresh token, the account's device token: what the server
//! gave this device when it finished a sign-in (the new-device check's, or
//! two-step login's "Don't ask again"), sent back at the next sign-in with
//! the master password so this device isn't new to the account again
//! (DESIGN.md in the meta repo, "New-device check"). It's kept by the same
//! server and email, so the sign-in screen finds it before the account is
//! on this device, and signing out leaves it: signing out ends a session,
//! not a device.

use std::collections::HashMap;
use std::path::Path;
#[cfg(target_os = "android")]
use std::path::PathBuf;
use std::sync::Mutex;

use zeroize::Zeroizing;

/// What an entry holds: each account has one of each, named by the
/// account's server and email.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Secret {
    /// The session's refresh token.
    RefreshToken,
    /// This device's token for the account (`TwoFactorToken`), sent back
    /// at each sign-in with the master password.
    DeviceToken,
}

/// Each account's refresh token is its own entry, named by the account's
/// server and email, so several accounts on one device never share one
/// (DESIGN.md in the meta repo, "Several accounts in the apps"); its device
/// token is another, named the same way.
pub trait SecretStore: Send + Sync {
    /// Keeps a secret; returns whether it was kept.
    fn keep_secret(&self, secret: Secret, server: &str, email: &str, value: &str) -> bool;
    fn get_secret(&self, secret: Secret, server: &str, email: &str) -> Option<Zeroizing<String>>;
    fn forget_secret(&self, secret: Secret, server: &str, email: &str);

    /// Keeps the refresh token; returns whether it was kept.
    fn keep(&self, server: &str, email: &str, refresh_token: &str) -> bool {
        self.keep_secret(Secret::RefreshToken, server, email, refresh_token)
    }

    fn get(&self, server: &str, email: &str) -> Option<Zeroizing<String>> {
        self.get_secret(Secret::RefreshToken, server, email)
    }

    fn forget(&self, server: &str, email: &str) {
        self.forget_secret(Secret::RefreshToken, server, email)
    }
}

/// One store shared by every account's `App` (accounts.rs).
impl<T: SecretStore + ?Sized> SecretStore for std::sync::Arc<T> {
    fn keep_secret(&self, secret: Secret, server: &str, email: &str, value: &str) -> bool {
        (**self).keep_secret(secret, server, email, value)
    }

    fn get_secret(&self, secret: Secret, server: &str, email: &str) -> Option<Zeroizing<String>> {
        (**self).get_secret(secret, server, email)
    }

    fn forget_secret(&self, secret: Secret, server: &str, email: &str) {
        (**self).forget_secret(secret, server, email)
    }
}

/// The OS store's service for each kind of entry: the refresh tokens' as
/// before device tokens, and the device tokens' its own, so each account's
/// two entries have the same account name (`account`).
#[cfg(not(target_os = "android"))]
fn service(secret: Secret) -> &'static str {
    match secret {
        Secret::RefreshToken => "PassBundle",
        Secret::DeviceToken => "PassBundle device token",
    }
}

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
    /// One file per account and kind, named by a hash so the email isn't
    /// in the name: `<hash>.sealed` for the refresh token, as before device
    /// tokens, and `<hash>.device.sealed` for the device token.
    fn path(&self, secret: Secret, server: &str, email: &str) -> PathBuf {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(account(server, email).as_bytes());
        let name: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
        self.dir.join(match secret {
            Secret::RefreshToken => format!("{name}.sealed"),
            Secret::DeviceToken => format!("{name}.device.sealed"),
        })
    }
}

#[cfg(target_os = "android")]
impl SecretStore for OsSecretStore {
    fn keep_secret(&self, secret: Secret, server: &str, email: &str, value: &str) -> bool {
        let Some(sealed) = crate::android::seal(value.as_bytes()) else {
            return false;
        };
        std::fs::create_dir_all(&self.dir).is_ok()
            && std::fs::write(self.path(secret, server, email), sealed).is_ok()
    }

    fn get_secret(&self, secret: Secret, server: &str, email: &str) -> Option<Zeroizing<String>> {
        let sealed = std::fs::read(self.path(secret, server, email)).ok()?;
        let plain = crate::android::open(&sealed)?;
        std::str::from_utf8(&plain)
            .ok()
            .map(|token| Zeroizing::new(token.to_owned()))
    }

    fn forget_secret(&self, secret: Secret, server: &str, email: &str) {
        let _ = std::fs::remove_file(self.path(secret, server, email));
    }
}

#[cfg(not(target_os = "android"))]
impl OsSecretStore {
    fn entry(secret: Secret, server: &str, email: &str) -> Option<keyring::Entry> {
        keyring::Entry::new(service(secret), &account(server, email)).ok()
    }
}

#[cfg(not(target_os = "android"))]
impl SecretStore for OsSecretStore {
    fn keep_secret(&self, secret: Secret, server: &str, email: &str, value: &str) -> bool {
        Self::entry(secret, server, email).is_some_and(|e| e.set_password(value).is_ok())
    }

    fn get_secret(&self, secret: Secret, server: &str, email: &str) -> Option<Zeroizing<String>> {
        Self::entry(secret, server, email)?
            .get_password()
            .ok()
            .map(Zeroizing::new)
    }

    fn forget_secret(&self, secret: Secret, server: &str, email: &str) {
        if let Some(e) = Self::entry(secret, server, email) {
            let _ = e.delete_credential();
        }
    }
}

/// Kept in memory only, for tests.
#[derive(Default)]
pub struct MemorySecretStore(Mutex<HashMap<(Secret, String), Zeroizing<String>>>);

impl SecretStore for MemorySecretStore {
    fn keep_secret(&self, secret: Secret, server: &str, email: &str, value: &str) -> bool {
        self.0.lock().unwrap().insert(
            (secret, account(server, email)),
            Zeroizing::new(value.to_owned()),
        );
        true
    }

    fn get_secret(&self, secret: Secret, server: &str, email: &str) -> Option<Zeroizing<String>> {
        self.0
            .lock()
            .unwrap()
            .get(&(secret, account(server, email)))
            .cloned()
    }

    fn forget_secret(&self, secret: Secret, server: &str, email: &str) {
        self.0
            .lock()
            .unwrap()
            .remove(&(secret, account(server, email)));
    }
}
