//! Where the refresh token is kept: the OS's secret store, Keychain (macOS,
//! iOS), Credential Manager (Windows), or Secret Service (Linux). Where
//! there's none, such as a Linux desktop without a keyring running, nothing is
//! kept and the person signs in again next time. Android's comes with the
//! Keystore (docs/phones.md); until then it keeps nothing. Tests use a store in memory,
//! so they never touch the real one.

use std::collections::HashMap;
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
pub struct OsSecretStore;

#[cfg(target_os = "android")]
impl SecretStore for OsSecretStore {
    fn keep(&self, _server: &str, _email: &str, _refresh_token: &str) -> bool {
        false
    }

    fn get(&self, _server: &str, _email: &str) -> Option<Zeroizing<String>> {
        None
    }

    fn forget(&self, _server: &str, _email: &str) {}
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
