//! Where the refresh token is kept: the OS's secret store, Keychain (iOS),
//! Credential Manager (Windows), or Secret Service (Linux). Where there's
//! none, such as a Linux desktop without a keyring running, nothing is kept
//! and the person signs in again next time. On Android it's sealed with a
//! key in the Keystore and kept in the app's own folder (docs/phones.md).
//! On the Mac it's sealed under a key of its own derived from the account's
//! user key, in the account's folder (`seal_refresh_token`), and never in the
//! login keychain: without an Apple developer team, the login keychain asks
//! for the Mac's password again for every build (the meta repo's
//! docs/core.md, "The Mac without an Apple developer account"). Tests use a
//! store in memory, so they never touch the real one.
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

use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::crypto::{EncString, SymmetricKey};

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

    /// Whether this store keeps refresh tokens. When it doesn't (the Mac's),
    /// the app keeps each account's sealed in the account's folder instead
    /// (`seal_refresh_token`), and the store's refresh token calls do
    /// nothing.
    fn keeps_refresh_tokens(&self) -> bool {
        true
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

    fn keeps_refresh_tokens(&self) -> bool {
        (**self).keeps_refresh_tokens()
    }
}

/// What the key a refresh token is sealed under is derived with: HKDF-Expand
/// of the user key (docs/vectors/mac-local-v1.json in the meta repo).
const REFRESH_TOKEN_INFO: &[u8] = b"keykeeper refresh token v1";

/// The key a refresh token is sealed under: HKDF-Expand-SHA256 of the
/// account's user key (all 64 bytes as the PRK), 64 bytes. Its own key, so
/// a sealed token is never a string the vault's key opens, and nothing else
/// under the user key (an item's field, say) passes for one.
fn refresh_token_key(user_key: &SymmetricKey) -> SymmetricKey {
    let prk = user_key.to_bytes();
    let hkdf = Hkdf::<Sha256>::from_prk(&prk).expect("a 64-byte PRK is long enough");
    let mut okm = Zeroizing::new([0u8; 64]);
    hkdf.expand(REFRESH_TOKEN_INFO, &mut *okm)
        .expect("64 bytes is a valid HKDF-SHA256 length");
    SymmetricKey::from_bytes(&*okm).expect("64 bytes")
}

/// A refresh token sealed for keeping in the account's folder, where the
/// store doesn't keep refresh tokens (the Mac's): a type 2 string under the
/// token's own key, derived from the account's user key. So it opens only
/// while the vault is open, which is the only time the app uses it.
pub fn seal_refresh_token(user_key: &SymmetricKey, token: &str) -> String {
    refresh_token_key(user_key).encrypt_str(token).to_string()
}

/// As `seal_refresh_token`, with a given IV: for the vectors.
pub fn seal_refresh_token_with_iv(user_key: &SymmetricKey, token: &str, iv: [u8; 16]) -> String {
    refresh_token_key(user_key)
        .encrypt_with_iv(token.as_bytes(), iv)
        .to_string()
}

/// A refresh token `seal_refresh_token` sealed, or None if it doesn't open
/// with this user key (another account's, the account's keys changed, or it
/// was changed).
pub fn open_refresh_token(user_key: &SymmetricKey, sealed: &str) -> Option<Zeroizing<String>> {
    EncString::parse(sealed.trim())
        .ok()?
        .decrypt_string(&refresh_token_key(user_key))
        .ok()
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

/// Whether the OS's store here keeps this kind of secret. The Mac's keeps
/// only device tokens: the app reads and writes an account's refresh token
/// at every unlock and sync, and the login keychain asks for the Mac's
/// password again for each build signed without an Apple developer team
/// (its partition list names such a build by its code directory hash). So
/// the Mac keeps refresh tokens sealed in each account's folder instead
/// (`seal_refresh_token`), and never touches the keychain for one. The
/// device token is read and written only when signing in with the master
/// password.
#[cfg(not(target_os = "android"))]
fn kept_here(secret: Secret) -> bool {
    !(cfg!(target_os = "macos") && secret == Secret::RefreshToken)
}

#[cfg(not(target_os = "android"))]
impl SecretStore for OsSecretStore {
    fn keep_secret(&self, secret: Secret, server: &str, email: &str, value: &str) -> bool {
        kept_here(secret)
            && Self::entry(secret, server, email).is_some_and(|e| e.set_password(value).is_ok())
    }

    fn get_secret(&self, secret: Secret, server: &str, email: &str) -> Option<Zeroizing<String>> {
        if !kept_here(secret) {
            return None;
        }
        Self::entry(secret, server, email)?
            .get_password()
            .ok()
            .map(Zeroizing::new)
    }

    fn forget_secret(&self, secret: Secret, server: &str, email: &str) {
        if !kept_here(secret) {
            return;
        }
        if let Some(e) = Self::entry(secret, server, email) {
            let _ = e.delete_credential();
        }
    }

    fn keeps_refresh_tokens(&self) -> bool {
        kept_here(Secret::RefreshToken)
    }
}

/// Kept in memory only, for tests.
pub struct MemorySecretStore {
    kept: Mutex<HashMap<(Secret, String), Zeroizing<String>>>,
    refresh_tokens: bool,
}

impl Default for MemorySecretStore {
    fn default() -> Self {
        MemorySecretStore {
            kept: Mutex::default(),
            refresh_tokens: true,
        }
    }
}

impl MemorySecretStore {
    /// As the Mac's store: device tokens only, so the app keeps refresh
    /// tokens sealed in the account's folder.
    pub fn without_refresh_tokens() -> Self {
        MemorySecretStore {
            kept: Mutex::default(),
            refresh_tokens: false,
        }
    }

    fn keeps(&self, secret: Secret) -> bool {
        self.refresh_tokens || secret != Secret::RefreshToken
    }
}

impl SecretStore for MemorySecretStore {
    fn keep_secret(&self, secret: Secret, server: &str, email: &str, value: &str) -> bool {
        if !self.keeps(secret) {
            return false;
        }
        self.kept.lock().unwrap().insert(
            (secret, account(server, email)),
            Zeroizing::new(value.to_owned()),
        );
        true
    }

    fn get_secret(&self, secret: Secret, server: &str, email: &str) -> Option<Zeroizing<String>> {
        if !self.keeps(secret) {
            return None;
        }
        self.kept
            .lock()
            .unwrap()
            .get(&(secret, account(server, email)))
            .cloned()
    }

    fn forget_secret(&self, secret: Secret, server: &str, email: &str) {
        self.kept
            .lock()
            .unwrap()
            .remove(&(secret, account(server, email)));
    }

    fn keeps_refresh_tokens(&self) -> bool {
        self.refresh_tokens
    }
}

#[cfg(test)]
mod tests {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use serde_json::Value;

    use super::*;

    /// The meta repo's docs/vectors/mac-local-v1.json (copied into
    /// tests/vectors), which plain Node made.
    fn vectors() -> Value {
        let v: Value =
            serde_json::from_str(include_str!("../tests/vectors/mac-local-v1.json")).unwrap();
        v["refreshToken"].clone()
    }

    fn key(v: &Value, name: &str) -> SymmetricKey {
        SymmetricKey::from_bytes(&STANDARD.decode(v[name].as_str().unwrap()).unwrap()).unwrap()
    }

    #[test]
    fn a_refresh_token_is_sealed_under_its_own_key_as_the_vectors_say() {
        let v = vectors();
        let user_key = key(&v, "userKey");
        assert_eq!(refresh_token_key(&user_key), key(&v, "key"));
        let iv: [u8; 16] = STANDARD
            .decode(v["iv"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let token = v["token"].as_str().unwrap();
        assert_eq!(
            seal_refresh_token_with_iv(&user_key, token, iv),
            v["sealed"].as_str().unwrap()
        );
        assert_eq!(
            open_refresh_token(&user_key, v["sealed"].as_str().unwrap()).as_deref(),
            Some(&token.to_owned())
        );
        // A random IV each time, and it opens.
        let sealed = seal_refresh_token(&user_key, token);
        assert_ne!(sealed, seal_refresh_token(&user_key, token));
        assert_eq!(
            open_refresh_token(&user_key, &sealed).as_deref(),
            Some(&token.to_owned())
        );
    }

    #[test]
    fn a_sealed_token_that_was_changed_or_is_under_another_key_is_refused() {
        let v = vectors();
        let user_key = key(&v, "userKey");
        assert!(open_refresh_token(&user_key, v["tampered"].as_str().unwrap()).is_none());
        // Under the user key itself, as an item's field is: not a token.
        assert!(open_refresh_token(&user_key, v["underUserKey"].as_str().unwrap()).is_none());
        // Another account's user key.
        assert!(
            open_refresh_token(&SymmetricKey::generate(), v["sealed"].as_str().unwrap()).is_none()
        );
        assert!(open_refresh_token(&user_key, "not a sealed token").is_none());
    }

    #[test]
    fn a_store_without_refresh_tokens_keeps_only_device_tokens() {
        let store = MemorySecretStore::without_refresh_tokens();
        assert!(!store.keeps_refresh_tokens());
        assert!(!store.keep("https://vault.example.test", "ana@example.test", "refresh"));
        assert!(store
            .get("https://vault.example.test", "ana@example.test")
            .is_none());
        assert!(store.keep_secret(
            Secret::DeviceToken,
            "https://vault.example.test",
            "ana@example.test",
            "device"
        ));
        assert!(store
            .get_secret(
                Secret::DeviceToken,
                "https://vault.example.test",
                "ana@example.test"
            )
            .is_some());
        assert!(MemorySecretStore::default().keeps_refresh_tokens());
        // The Mac's own store never keeps a refresh token in the keychain.
        #[cfg(target_os = "macos")]
        {
            let os = OsSecretStore::new(Path::new("."));
            assert!(!os.keeps_refresh_tokens());
            assert!(!kept_here(Secret::RefreshToken));
            assert!(kept_here(Secret::DeviceToken));
        }
    }
}
