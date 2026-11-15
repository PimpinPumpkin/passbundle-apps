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
//!
//! And sealing to the device (`seal_to_device`), for an account's PIN
//! (pin.rs; DESIGN.md in the meta repo, "Unlock with a PIN"): what the PIN
//! opens is kept only under a key that can't be copied off this device, the
//! Android Keystore's, the Mac's Secure Enclave's, the iPhone's Keychain's,
//! or Windows' data protection for the person. Where there's none (Linux,
//! a Mac without a Secure Enclave), nothing is sealed and no PIN is offered.

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

    /// Whether this device can seal something so that a copy of the disk
    /// doesn't open it (`seal_to_device`). Where it can't, no PIN is
    /// offered.
    fn seals_to_device(&self) -> bool {
        false
    }

    /// Seals `plain` for one account (its id on this device) under a key
    /// that stays on this device, without asking the person anything. None
    /// where the device can't.
    fn seal_to_device(&self, account: &str, plain: &[u8]) -> Option<Vec<u8>> {
        let _ = (account, plain);
        None
    }

    /// What `seal_to_device` sealed for this account; None if it doesn't
    /// open (the device's key is gone, it was sealed for another account or
    /// on another device, or it was changed).
    fn open_from_device(&self, account: &str, sealed: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
        let _ = (account, sealed);
        None
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

    fn seals_to_device(&self) -> bool {
        (**self).seals_to_device()
    }

    fn seal_to_device(&self, account: &str, plain: &[u8]) -> Option<Vec<u8>> {
        (**self).seal_to_device(account, plain)
    }

    fn open_from_device(&self, account: &str, sealed: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
        (**self).open_from_device(account, sealed)
    }
}

/// What's authenticated with anything sealed to the device, beside the
/// device's own key: this label, a zero byte, and the account's id on this
/// device. So one account's doesn't open as another's, and nothing else the
/// device's key protects passes for one. The Mac's and the iPhone's Swift
/// and Android's Kotlin build the same bytes.
pub const DEVICE_SEAL_LABEL: &str = "keykeeper PIN device seal v1";

fn device_seal_context(account: &str) -> Vec<u8> {
    let mut context = DEVICE_SEAL_LABEL.as_bytes().to_vec();
    context.push(0);
    context.extend_from_slice(account.as_bytes());
    context
}

/// The Mac's and the iPhone's seal is the app's own Swift (the Secure
/// Enclave, the Keychain), which the core calls through these once the app
/// has given them (`pb_set_device_seal`). Each reads `input_len` bytes at
/// `input` for the account named, writes its answer to `output`, which holds
/// `capacity` bytes, and returns how many it wrote: negative when it can't
/// (no key on this device, a seal that doesn't open, an answer that doesn't
/// fit). Neither ever shows a prompt.
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub type DeviceSealCall = unsafe extern "C" fn(
    account: *const std::ffi::c_char,
    input: *const u8,
    input_len: usize,
    output: *mut u8,
    capacity: usize,
) -> isize;

/// The app's seal and its opening, given once at start, and only where the
/// device has what they need (a Mac with a Secure Enclave, an iPhone).
/// Until then, and in the iPhone's AutoFill extension, which never gives
/// them, nothing is sealed and no PIN is offered.
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub fn set_device_seal(seal: DeviceSealCall, open: DeviceSealCall) {
    let _ = device::CALLS.set((seal, open));
}

/// The Mac and the iPhone: the app's own seal, where it gave one.
#[cfg(any(target_os = "macos", target_os = "ios"))]
mod device {
    use std::sync::OnceLock;

    use zeroize::Zeroizing;

    use super::DeviceSealCall;

    pub(super) static CALLS: OnceLock<(DeviceSealCall, DeviceSealCall)> = OnceLock::new();

    /// More than a sealed PIN record with the Mac's enclave key beside it
    /// (about 1 KiB).
    const MOST: usize = 16 * 1024;

    pub(super) fn call(
        call: DeviceSealCall,
        account: &str,
        input: &[u8],
    ) -> Option<Zeroizing<Vec<u8>>> {
        let account = std::ffi::CString::new(account).ok()?;
        let mut output = Zeroizing::new(vec![0u8; MOST]);
        // SAFETY: `account` is NUL-terminated, and it, `input`, and `output`
        // outlive the call; the app writes at most `capacity` bytes to
        // `output`.
        let written = unsafe {
            call(
                account.as_ptr(),
                input.as_ptr(),
                input.len(),
                output.as_mut_ptr(),
                output.len(),
            )
        };
        let written = usize::try_from(written).ok()?;
        (written <= output.len()).then(|| Zeroizing::new(output[..written].to_vec()))
    }

    pub(super) fn available() -> bool {
        CALLS.get().is_some()
    }

    pub(super) fn seal(account: &str, plain: &[u8]) -> Option<Vec<u8>> {
        call(CALLS.get()?.0, account, plain).map(|sealed| sealed.to_vec())
    }

    pub(super) fn open(account: &str, sealed: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
        call(CALLS.get()?.1, account, sealed)
    }
}

/// Windows: its data protection for the person (DPAPI), as the desktop
/// agent protects the Windows Hello key's file. Only their Windows account
/// opens it, and it never shows anything.
#[cfg(windows)]
mod device {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    use zeroize::{Zeroize, Zeroizing};

    use super::device_seal_context;

    fn blob(bytes: &[u8]) -> Option<CRYPT_INTEGER_BLOB> {
        Some(CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(bytes.len()).ok()?,
            pbData: bytes.as_ptr().cast_mut(),
        })
    }

    /// `context` is what DPAPI calls optional entropy: the same bytes must
    /// be given to open what was protected with them.
    fn call(context: &[u8], input: &[u8], protect: bool) -> Option<Zeroizing<Vec<u8>>> {
        let input = blob(input)?;
        let context = blob(context)?;
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: `input` and `context` point at slices that outlive the
        // call; Windows allocates `output` with LocalAlloc, which is
        // copied, wiped, and freed below.
        let ok = unsafe {
            if protect {
                CryptProtectData(
                    &input,
                    std::ptr::null(),
                    &context,
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            } else {
                CryptUnprotectData(
                    &input,
                    std::ptr::null_mut(),
                    &context,
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            }
        };
        if ok == 0 || output.pbData.is_null() {
            return None;
        }
        // SAFETY: Windows says `output` holds `cbData` bytes at `pbData`.
        let copy = unsafe {
            let bytes = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
            let copy = Zeroizing::new(bytes.to_vec());
            bytes.zeroize();
            LocalFree(output.pbData.cast());
            copy
        };
        Some(copy)
    }

    pub(super) fn available() -> bool {
        true
    }

    pub(super) fn seal(account: &str, plain: &[u8]) -> Option<Vec<u8>> {
        call(&device_seal_context(account), plain, true).map(|sealed| sealed.to_vec())
    }

    pub(super) fn open(account: &str, sealed: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
        call(&device_seal_context(account), sealed, false)
    }
}

/// Android: a key in the Android Keystore of its own (`passbundle-pin`),
/// apart from the one that seals the tokens.
#[cfg(target_os = "android")]
mod device {
    use zeroize::Zeroizing;

    use super::device_seal_context;

    pub(super) fn available() -> bool {
        true
    }

    pub(super) fn seal(account: &str, plain: &[u8]) -> Option<Vec<u8>> {
        crate::android::seal_pin(&device_seal_context(account), plain)
    }

    pub(super) fn open(account: &str, sealed: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
        crate::android::open_pin(&device_seal_context(account), sealed)
    }
}

/// Linux (and anything else): nothing that stays on the device to seal
/// with, so no PIN is offered (DESIGN.md, "Unlock with a PIN").
#[cfg(not(any(windows, target_os = "macos", target_os = "ios", target_os = "android")))]
mod device {
    use zeroize::Zeroizing;

    pub(super) fn available() -> bool {
        false
    }

    pub(super) fn seal(_account: &str, _plain: &[u8]) -> Option<Vec<u8>> {
        None
    }

    pub(super) fn open(_account: &str, _sealed: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
        None
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

    fn seals_to_device(&self) -> bool {
        device::available()
    }

    fn seal_to_device(&self, account: &str, plain: &[u8]) -> Option<Vec<u8>> {
        device::seal(account, plain)
    }

    fn open_from_device(&self, account: &str, sealed: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
        device::open(account, sealed)
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
/// password. It's the core's rule for the Mac, not the app's: whatever is
/// built on the core there (today, the app alone) is signed the same way.
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

    fn seals_to_device(&self) -> bool {
        device::available()
    }

    fn seal_to_device(&self, account: &str, plain: &[u8]) -> Option<Vec<u8>> {
        device::seal(account, plain)
    }

    fn open_from_device(&self, account: &str, sealed: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
        device::open(account, sealed)
    }
}

/// Kept in memory only, for tests.
pub struct MemorySecretStore {
    kept: Mutex<HashMap<(Secret, String), Zeroizing<String>>>,
    refresh_tokens: bool,
    /// Stands in for the device's own key: made new with each store, so
    /// another store (another device, or this one after its key is gone)
    /// opens nothing this one sealed. None for a device that can't seal.
    device_key: Option<SymmetricKey>,
}

impl Default for MemorySecretStore {
    fn default() -> Self {
        MemorySecretStore {
            kept: Mutex::default(),
            refresh_tokens: true,
            device_key: Some(SymmetricKey::generate()),
        }
    }
}

impl MemorySecretStore {
    /// As the Mac's store: device tokens only, so the app keeps refresh
    /// tokens sealed in the account's folder.
    pub fn without_refresh_tokens() -> Self {
        MemorySecretStore {
            refresh_tokens: false,
            ..MemorySecretStore::default()
        }
    }

    /// As a device with nothing to seal with (Linux, a Mac without a
    /// Secure Enclave): no PIN is offered.
    pub fn without_device_seal() -> Self {
        MemorySecretStore {
            device_key: None,
            ..MemorySecretStore::default()
        }
    }

    /// With this stand-in for the device's key, so a store made later with
    /// the same one (the app opened again on the same device) opens what
    /// this one sealed.
    pub fn with_device_key(key: SymmetricKey) -> Self {
        MemorySecretStore {
            device_key: Some(key),
            ..MemorySecretStore::default()
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

    fn seals_to_device(&self) -> bool {
        self.device_key.is_some()
    }

    /// The context, a zero byte, then what's sealed, under the stand-in
    /// key: the context is checked when it's opened, as a device's seal
    /// checks its authenticated data.
    fn seal_to_device(&self, account: &str, plain: &[u8]) -> Option<Vec<u8>> {
        let mut framed = Zeroizing::new(memory_seal_frame(account));
        framed.extend_from_slice(plain);
        Some(
            self.device_key
                .as_ref()?
                .encrypt(&framed)
                .to_string()
                .into_bytes(),
        )
    }

    fn open_from_device(&self, account: &str, sealed: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
        let framed = EncString::parse(std::str::from_utf8(sealed).ok()?)
            .ok()?
            .decrypt(self.device_key.as_ref()?)
            .ok()?;
        framed
            .strip_prefix(memory_seal_frame(account).as_slice())
            .map(|plain| Zeroizing::new(plain.to_vec()))
    }
}

/// What the memory store puts before what it seals: the context, ended with
/// a zero byte (no account's id holds one), so one account's context is
/// never the start of another's.
fn memory_seal_frame(account: &str) -> Vec<u8> {
    let mut frame = device_seal_context(account);
    frame.push(0);
    frame
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

    #[test]
    fn the_device_seals_context_is_the_label_and_the_account_as_the_vectors_say() {
        let v: Value =
            serde_json::from_str(include_str!("../tests/vectors/pin-unlock-v1.json")).unwrap();
        let seal = &v["macDeviceSeal"];
        assert_eq!(seal["info"], DEVICE_SEAL_LABEL);
        let account = seal["account"].as_str().unwrap();
        let mut expected = seal["info"].as_str().unwrap().as_bytes().to_vec();
        expected.push(0);
        expected.extend_from_slice(account.as_bytes());
        assert_eq!(device_seal_context(account), expected);
        // The one account's folder (no id) still has the label and the zero.
        assert_eq!(
            device_seal_context(""),
            [DEVICE_SEAL_LABEL.as_bytes(), &[0]].concat()
        );
    }

    #[test]
    fn what_the_memory_store_seals_opens_only_on_that_device_for_that_account() {
        let store = MemorySecretStore::default();
        assert!(store.seals_to_device());
        let plain = b"a PIN's record";
        let sealed = store.seal_to_device("account-a", plain).unwrap();
        assert_eq!(
            store
                .open_from_device("account-a", &sealed)
                .unwrap()
                .as_slice(),
            plain
        );
        // What's kept isn't what was sealed, and no two seals are alike.
        assert!(!sealed.windows(plain.len()).any(|w| w == plain));
        assert_ne!(store.seal_to_device("account-a", plain).unwrap(), sealed);

        // Another account, one whose id starts the same, one that starts
        // with this one's, and none.
        for other in ["account-b", "account-", "account-ab", ""] {
            assert!(store.open_from_device(other, &sealed).is_none(), "{other}");
        }
        // Another device.
        assert!(MemorySecretStore::default()
            .open_from_device("account-a", &sealed)
            .is_none());
        // Changed, cut short, or nothing at all.
        let mut changed = sealed.clone();
        let last = changed.len() - 2;
        changed[last] ^= 1;
        assert!(store.open_from_device("account-a", &changed).is_none());
        assert!(store
            .open_from_device("account-a", &sealed[..sealed.len() / 2])
            .is_none());
        assert!(store.open_from_device("account-a", b"").is_none());
        assert!(store.open_from_device("account-a", &[0xff, 0xfe]).is_none());

        // The same device, started again: a store made with the same key
        // opens it.
        let key = SymmetricKey::generate();
        let sealed = MemorySecretStore::with_device_key(key.clone())
            .seal_to_device("account-a", plain)
            .unwrap();
        assert_eq!(
            MemorySecretStore::with_device_key(key)
                .open_from_device("account-a", &sealed)
                .unwrap()
                .as_slice(),
            plain
        );
    }

    #[test]
    fn a_store_with_no_device_seal_seals_and_opens_nothing() {
        let sealed = MemorySecretStore::default()
            .seal_to_device("a", b"record")
            .unwrap();
        let store = MemorySecretStore::without_device_seal();
        assert!(!store.seals_to_device());
        assert!(store.seal_to_device("a", b"record").is_none());
        assert!(store.open_from_device("a", &sealed).is_none());
        // Everything else about it is as any store.
        assert!(store.keeps_refresh_tokens());

        // A store that doesn't say it seals (the trait's own answers).
        struct Plain;
        impl SecretStore for Plain {
            fn keep_secret(&self, _: Secret, _: &str, _: &str, _: &str) -> bool {
                false
            }
            fn get_secret(&self, _: Secret, _: &str, _: &str) -> Option<Zeroizing<String>> {
                None
            }
            fn forget_secret(&self, _: Secret, _: &str, _: &str) {}
        }
        assert!(!Plain.seals_to_device());
        assert!(Plain.seal_to_device("a", b"record").is_none());
        assert!(Plain.open_from_device("a", &sealed).is_none());
    }

    /// The app's seal as the core calls it: with the account's id and what
    /// to seal or open, taking only an answer that fits what it was given.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_apps_seal_gets_the_account_and_its_answer_has_to_fit() {
        use std::ffi::{c_char, CStr};

        /// Answers with the account's id, a colon, and what it was given.
        unsafe extern "C" fn echo(
            account: *const c_char,
            input: *const u8,
            input_len: usize,
            output: *mut u8,
            capacity: usize,
        ) -> isize {
            let mut answer = CStr::from_ptr(account).to_bytes().to_vec();
            answer.push(b':');
            answer.extend_from_slice(std::slice::from_raw_parts(input, input_len));
            if answer.len() > capacity {
                return -1;
            }
            std::ptr::copy_nonoverlapping(answer.as_ptr(), output, answer.len());
            answer.len() as isize
        }
        unsafe extern "C" fn refuses(
            _: *const c_char,
            _: *const u8,
            _: usize,
            _: *mut u8,
            _: usize,
        ) -> isize {
            -1
        }
        unsafe extern "C" fn says_more_than_fits(
            _: *const c_char,
            _: *const u8,
            _: usize,
            _: *mut u8,
            capacity: usize,
        ) -> isize {
            capacity as isize + 1
        }

        assert_eq!(
            device::call(echo, "account-a", b"record")
                .unwrap()
                .as_slice(),
            b"account-a:record"
        );
        assert!(device::call(refuses, "account-a", b"record").is_none());
        assert!(device::call(says_more_than_fits, "account-a", b"record").is_none());
        // More than the core gives room for is refused by the app's side.
        assert!(device::call(echo, "account-a", &vec![7u8; 17 * 1024]).is_none());
        // An id that can't be a C string is never handed over.
        assert!(device::call(echo, "account\0a", b"record").is_none());
    }

    /// This Mac's own store, before the app has given the core its seal
    /// (and in every test): no PIN is offered, and nothing is asked of the
    /// Secure Enclave or the keychain.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_macs_store_seals_nothing_until_the_app_gives_it_the_seal() {
        let os = OsSecretStore::new(Path::new("."));
        assert!(!os.seals_to_device());
        assert!(os.seal_to_device("a", b"record").is_none());
        assert!(os.open_from_device("a", b"sealed").is_none());
    }
}
