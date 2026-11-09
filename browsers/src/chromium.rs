//! Chrome, Edge, Brave, Vivaldi, Opera, and Chromium: one layout, from
//! Chromium's own open-source code (its profile manager, its password
//! store, and `os_crypt`). The details are in DESIGN.md, "Automatic import".
//!
//! - **Profiles:** the browser's folder holds `Local State` (JSON), whose
//!   `profile.info_cache` names each profile's folder, its name, and the
//!   account it's signed in to (`user_name`). The "Default" folder is the
//!   browser's first profile.
//! - **Saved passwords:** each profile's `Login Data` (and `Login Data For
//!   Account`, those kept for the signed-in account) is SQLite: table
//!   `logins`, with `password_value` encrypted by the browser.
//! - **Encrypted how:** a value starts with its version. On macOS, `v10`:
//!   AES-128-CBC under a key made with PBKDF2 (HMAC-SHA1, salt "saltysalt",
//!   1,003 rounds) from the browser's secret in the keychain. On Linux the
//!   same with one round, from the keyring's secret for `v11`, or from the
//!   fixed word Chromium uses without a keyring for `v10`. On Windows,
//!   `v10`: AES-256-GCM under a key in `Local State` (`os_crypt.encrypted_key`)
//!   that Windows protects for the person (DPAPI). `v20` is Chrome's
//!   app-bound encryption, which only Chrome can open: the profile goes to
//!   the guided import, and nothing in it is opened.

use std::collections::HashMap;
use std::path::Path;

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::types::ValueRef;
use serde_json::Value;
use zeroize::Zeroizing;

use crate::keys::{KeyError, Keys};
use crate::{Browser, Home, Login, Platform, Profile, Read, TurnOff, Why};

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

/// Folders in a browser's folder that are never a person's profile.
const NOT_PROFILES: [&str; 3] = ["System Profile", "Guest Profile", "Crashpad"];

/// The profiles in one Chromium browser's folder.
pub(crate) fn profiles(browser: Browser, dir: &Path, home: &Home) -> Vec<Profile> {
    let local_state = read_json(&dir.join("Local State"));
    let info = local_state
        .as_ref()
        .and_then(|s| s.pointer("/profile/info_cache"))
        .and_then(Value::as_object);
    let mut found = Vec::new();
    match info {
        Some(info) => {
            for (folder, entry) in info {
                // A folder's own name, never a path ("..", "a/b"), so a list
                // can't point anywhere but inside the browser's folder.
                let plain = !folder.is_empty()
                    && folder != "."
                    && folder != ".."
                    && !folder.contains(['/', '\\', ':']);
                if NOT_PROFILES.contains(&folder.as_str()) || !plain {
                    continue;
                }
                let profile_dir = dir.join(folder);
                if !is_profile(&profile_dir) || !home.owns(&profile_dir) {
                    continue;
                }
                if entry["is_ephemeral"] == true {
                    continue;
                }
                let name = entry["name"]
                    .as_str()
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or(folder)
                    .to_owned();
                found.push(profile(
                    browser,
                    home,
                    dir,
                    &profile_dir,
                    folder,
                    name,
                    entry,
                ));
            }
        }
        None => {
            // No list of profiles: the one in "Default", or, as Opera keeps
            // it, the browser's folder itself.
            for (profile_dir, folder) in [(dir.join("Default"), "Default"), (dir.to_owned(), "")] {
                if is_profile(&profile_dir) {
                    let name = if folder.is_empty() {
                        browser.short().to_owned()
                    } else {
                        folder.to_owned()
                    };
                    found.push(profile(
                        browser,
                        home,
                        dir,
                        &profile_dir,
                        if folder.is_empty() { "Default" } else { folder },
                        name,
                        &Value::Null,
                    ));
                    break;
                }
            }
        }
    }
    // The default first, then by name.
    found.sort_by(|a, b| b.default.cmp(&a.default).then(a.name.cmp(&b.name)));
    found
}

fn profile(
    browser: Browser,
    home: &Home,
    browser_dir: &Path,
    profile_dir: &Path,
    folder: &str,
    name: String,
    entry: &Value,
) -> Profile {
    let email = entry["user_name"]
        .as_str()
        .map(|e| e.trim().to_lowercase())
        .filter(|e| e.contains('@'));
    let managed_by = entry["hosted_domain"]
        .as_str()
        .map(|d| d.trim().to_lowercase())
        .filter(|d| !d.is_empty() && d != "no_hosted_domain" && d.contains('.'));
    Profile {
        id: format!("{}:{}", browser.key(), home.id_path(profile_dir)),
        browser,
        name,
        email,
        managed_by,
        default: folder == "Default",
        platform: home.platform,
        dir: profile_dir.to_owned(),
        browser_dir: browser_dir.to_owned(),
    }
}

fn is_profile(dir: &Path) -> bool {
    ["Login Data", "Login Data For Account", "Preferences"]
        .iter()
        .any(|f| dir.join(f).is_file())
}

fn read_json(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// A row of `logins`, as stored.
struct Row {
    id: i64,
    origin: String,
    realm: String,
    username: String,
    value: Vec<u8>,
    never_save: bool,
    created: i64,
    changed: i64,
}

/// Reads a profile's saved passwords.
pub(crate) fn read(profile: &Profile, keys: &dyn Keys) -> Read {
    let mut rows = Vec::new();
    let mut notes: Vec<HashMap<i64, Vec<u8>>> = Vec::new();
    for name in ["Login Data", "Login Data For Account"] {
        let path = profile.dir.join(name);
        if !path.is_file() {
            continue;
        }
        match read_store(&path) {
            Ok((store_rows, store_notes)) => {
                let index = notes.len();
                notes.push(store_notes);
                rows.extend(store_rows.into_iter().map(|r| (index, r)));
            }
            Err(_) => {
                return Read::Guided(if crate::is_open(profile) {
                    Why::Busy
                } else {
                    Why::Damaged
                })
            }
        }
    }
    // Chrome's app-bound encryption: only Chrome opens these. Nothing in the
    // profile is opened, not even with the key that would open the rest.
    if rows.iter().any(|(_, r)| r.value.starts_with(b"v20")) {
        return Read::Guided(Why::OnlyTheBrowser);
    }
    let mut keyring = Keyring::new(profile, keys);
    let (mut logins, mut never_save, mut empty, mut unreadable) = (Vec::new(), 0, 0, 0);
    for (store, row) in rows {
        if row.never_save {
            never_save += 1;
            continue;
        }
        let Some(url) = site(&row.origin, &row.realm) else {
            continue;
        };
        if row.value.is_empty() {
            empty += 1;
            continue;
        }
        let password = match keyring.open(&row.value) {
            Ok(p) => p,
            Err(Open::Key(why)) => return Read::Guided(why),
            Err(Open::Unreadable) => {
                unreadable += 1;
                continue;
            }
        };
        if password.is_empty() {
            empty += 1;
            continue;
        }
        let note = notes[store]
            .get(&row.id)
            .and_then(|value| keyring.open(value).ok())
            .filter(|n| !n.trim().is_empty());
        logins.push(Login {
            url,
            username: row.username,
            password,
            note,
            created_ms: webkit_ms(row.created),
            changed_ms: webkit_ms(row.changed),
        });
    }
    Read::Logins {
        logins,
        never_save,
        empty,
        unreadable,
    }
}

type Store = (Vec<Row>, HashMap<i64, Vec<u8>>);

fn read_store(path: &Path) -> rusqlite::Result<Store> {
    let conn = crate::sqlite::open(path)?;
    let columns = crate::sqlite::columns(&conn, "logins")?;
    let has = |c: &str| columns.iter().any(|n| n == c);
    let or_zero = |c: &'static str| if has(c) { c } else { "0" };
    let sql = format!(
        "SELECT rowid, origin_url, signon_realm, username_value, password_value, \
         blacklisted_by_user, {}, {} FROM logins",
        or_zero("date_created"),
        or_zero("date_password_modified"),
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement
        .query_map([], |r| {
            Ok(Row {
                id: r.get(0)?,
                origin: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                realm: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                username: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                value: stored_bytes(r.get_ref(4)?).unwrap_or_default(),
                never_save: r.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0,
                created: r.get::<_, Option<i64>>(6)?.unwrap_or(0),
                changed: r.get::<_, Option<i64>>(7)?.unwrap_or(0),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // Notes saved with passwords (newer versions): the one shown in the
    // browser is the one with no name. A store without them has none.
    // Chrome keeps a note's sealed bytes as a text value, not a blob.
    let mut notes = HashMap::new();
    if crate::sqlite::columns(&conn, "password_notes").is_ok_and(|c| !c.is_empty()) {
        if let Ok(mut statement) = conn
            .prepare("SELECT parent_id, value FROM password_notes WHERE key = '' OR key IS NULL")
        {
            let found = statement.query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, stored_bytes(r.get_ref(1)?)))
            });
            if let Ok(found) = found {
                for (parent, value) in found.flatten() {
                    if let Some(value) = value.filter(|v| !v.is_empty()) {
                        notes.insert(parent, value);
                    }
                }
            }
        }
    }
    Ok((rows, notes))
}

/// A sealed value's bytes as the store keeps them. Chrome writes a
/// password's as a blob and a note's as a text value, which isn't text to
/// read: the bytes are the same sealed bytes either way.
fn stored_bytes(value: ValueRef<'_>) -> Option<Vec<u8>> {
    match value {
        ValueRef::Blob(bytes) | ValueRef::Text(bytes) => Some(bytes.to_vec()),
        _ => None,
    }
}

/// The site a saved password is for: its page's address (http or https),
/// or an Android app's (`android://<hash>@<package>/` as `androidapp://<package>`).
/// Anything else (a browser's own pages, proxies) isn't a site.
fn site(origin: &str, realm: &str) -> Option<String> {
    if let Some(rest) = realm.strip_prefix("android://") {
        let package = rest.split_once('@').map_or(rest, |(_, p)| p);
        let package = package.trim_end_matches('/');
        if !package.is_empty() {
            return Some(format!("androidapp://{package}"));
        }
        return None;
    }
    let url = if origin.is_empty() { realm } else { origin };
    let lower = url.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://")).then(|| url.to_owned())
}

/// Chromium's times (microseconds since 1601) as milliseconds since 1970.
fn webkit_ms(micros: i64) -> Option<i64> {
    (micros > 0).then(|| micros / 1000 - 11_644_473_600_000)
}

enum Open {
    /// The key couldn't be had: the profile goes to the guided import.
    Key(Why),
    /// This one value didn't open.
    Unreadable,
}

/// The profile's keys, fetched once, when a value first needs one.
struct Keyring<'a> {
    profile: &'a Profile,
    keys: &'a dyn Keys,
    /// macOS, and Linux's keyring (`v11`): AES-128.
    keychain: Option<Result<Zeroizing<[u8; 16]>, Why>>,
    /// Windows: AES-256-GCM, from Local State.
    windows: Option<Result<Zeroizing<[u8; 32]>, Why>>,
}

/// What Chromium on Linux encrypts with when there's no keyring (`v10`).
const NO_KEYRING: &[u8] = b"peanuts";
const SALT: &[u8] = b"saltysalt";

pub(crate) fn cbc_key(secret: &[u8], rounds: u32) -> Zeroizing<[u8; 16]> {
    let mut key = Zeroizing::new([0u8; 16]);
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(secret, SALT, rounds, key.as_mut());
    key
}

fn key_why(e: KeyError) -> Why {
    match e {
        KeyError::Denied => Why::Declined,
        KeyError::Locked => Why::KeyringLocked,
        KeyError::NotFound | KeyError::Unavailable => Why::NoKey,
    }
}

impl<'a> Keyring<'a> {
    fn new(profile: &'a Profile, keys: &'a dyn Keys) -> Self {
        Keyring {
            profile,
            keys,
            keychain: None,
            windows: None,
        }
    }

    fn open(&mut self, value: &[u8]) -> Result<Zeroizing<String>, Open> {
        let (version, rest) = value.split_at(value.len().min(3));
        let bytes = match (self.profile.platform, version) {
            (Platform::Windows, b"v10" | b"v11") => {
                let key = self.windows_key()?;
                open_gcm(&key, rest).ok_or(Open::Unreadable)?
            }
            (Platform::Windows, _) => {
                // Older versions protected each value with DPAPI itself.
                self.keys.unprotect(value).map_err(|_| Open::Unreadable)?
            }
            (Platform::Mac, b"v10") => open_cbc(&*self.keychain_key(1003)?, rest)?,
            (Platform::Linux, b"v10") => open_cbc(&cbc_key(NO_KEYRING, 1), rest)?,
            (Platform::Linux, b"v11") => open_cbc(&*self.keychain_key(1)?, rest)?,
            _ => return Err(Open::Unreadable),
        };
        let text = std::str::from_utf8(&bytes).map_err(|_| Open::Unreadable)?;
        Ok(Zeroizing::new(text.to_owned()))
    }

    fn keychain_key(&mut self, rounds: u32) -> Result<Zeroizing<[u8; 16]>, Open> {
        let (keys, browser) = (self.keys, self.profile.browser);
        let fetched = self.keychain.get_or_insert_with(|| {
            keys.safe_storage(browser)
                .map(|secret| cbc_key(&secret, rounds))
                .map_err(key_why)
        });
        fetched.clone().map_err(Open::Key)
    }

    fn windows_key(&mut self) -> Result<Zeroizing<[u8; 32]>, Open> {
        let (keys, dir) = (self.keys, self.profile.browser_dir.clone());
        let fetched = self.windows.get_or_insert_with(|| windows_key(keys, &dir));
        fetched.clone().map_err(Open::Key)
    }
}

/// The key Local State keeps for the profiles' passwords, which Windows
/// protects for the person ("DPAPI" and the protected key, in base64).
fn windows_key(keys: &dyn Keys, browser_dir: &Path) -> Result<Zeroizing<[u8; 32]>, Why> {
    let state = read_json(&browser_dir.join("Local State")).ok_or(Why::NoKey)?;
    let encoded = state
        .pointer("/os_crypt/encrypted_key")
        .and_then(Value::as_str)
        .ok_or(Why::NoKey)?;
    let blob = Zeroizing::new(STANDARD.decode(encoded).map_err(|_| Why::Damaged)?);
    let protected = blob.strip_prefix(b"DPAPI").ok_or(Why::Unknown)?;
    let opened = keys.unprotect(protected).map_err(key_why)?;
    let key: [u8; 32] = opened.as_slice().try_into().map_err(|_| Why::Damaged)?;
    Ok(Zeroizing::new(key))
}

fn open_cbc(key: &[u8; 16], ciphertext: &[u8]) -> Result<Zeroizing<Vec<u8>>, Open> {
    let iv = [b' '; 16];
    let opened = Aes128CbcDec::new(key.into(), &iv.into())
        .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
        .map_err(|_| Open::Unreadable)?;
    Ok(Zeroizing::new(opened))
}

/// A 12-byte nonce, then the ciphertext and its 16-byte tag.
fn open_gcm(key: &[u8; 32], sealed: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
    if sealed.len() < 12 + 16 {
        return None;
    }
    let (nonce, ciphertext) = sealed.split_at(12);
    let cipher = Aes256Gcm::new(key.into());
    cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .ok()
        .map(Zeroizing::new)
}

/// The profile's `Preferences`: the switches PassBundle turns off.
const SAVING: &str = "credentials_enable_service";
const AUTO_SIGN_IN: &str = "credentials_enable_autosignin";

pub(crate) fn saving_on(profile: &Profile) -> Option<bool> {
    let prefs = read_json(&profile.dir.join("Preferences"))?;
    Some(prefs[SAVING].as_bool().unwrap_or(true))
}

/// Turns off the profile's "Offer to save passwords" and automatic sign-in,
/// as its own Settings switch does, with the browser closed. What it saved
/// stays.
pub(crate) fn turn_off(profile: &Profile) -> TurnOff {
    if crate::running::chromium(&profile.browser_dir, profile.platform) {
        return TurnOff::CloseFirst;
    }
    let path = profile.dir.join("Preferences");
    let mut prefs = match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(map)) => Value::Object(map),
            _ => return TurnOff::Failed,
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Object(Default::default()),
        Err(_) => return TurnOff::Failed,
    };
    if prefs[SAVING] == false && prefs[AUTO_SIGN_IN] == false {
        return TurnOff::AlreadyOff;
    }
    prefs[SAVING] = Value::Bool(false);
    prefs[AUTO_SIGN_IN] = Value::Bool(false);
    match crate::running::replace_file(&path, prefs.to_string().as_bytes()) {
        Ok(()) => TurnOff::Done,
        Err(_) => TurnOff::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sealed_value_is_its_bytes_whether_the_store_keeps_it_as_text_or_a_blob() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        // Not text: a zero byte, and bytes that aren't UTF-8.
        let sealed: &[u8] = b"v10\xea\xef\x92\x00\xb2\xff";
        for sql in ["SELECT ?1", "SELECT CAST(?1 AS TEXT)"] {
            let read = conn
                .query_row(sql, [sealed], |r| Ok(stored_bytes(r.get_ref(0)?)))
                .unwrap();
            assert_eq!(read.as_deref(), Some(sealed), "{sql}");
        }
        let nothing = conn
            .query_row("SELECT NULL", [], |r| Ok(stored_bytes(r.get_ref(0)?)))
            .unwrap();
        assert_eq!(nothing, None);
    }

    #[test]
    fn sites_are_web_pages_and_android_apps_only() {
        assert_eq!(
            site("https://example.test/login", "https://example.test/").as_deref(),
            Some("https://example.test/login")
        );
        assert_eq!(
            site("", "android://AbC1-hash==@com.example.app/").as_deref(),
            Some("androidapp://com.example.app")
        );
        assert_eq!(site("chrome://settings", "chrome://settings"), None);
        assert_eq!(
            site("ftp://files.example.test", "ftp://files.example.test"),
            None
        );
    }

    #[test]
    fn chromium_times_count_from_1601() {
        // 2026-01-01T00:00:00Z.
        assert_eq!(webkit_ms(13_411_699_200_000_000), Some(1_767_225_600_000));
        assert_eq!(webkit_ms(0), None);
    }
}
