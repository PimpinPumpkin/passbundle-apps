//! Made-up browser profiles with known fake passwords, in a throwaway folder,
//! laid out and encrypted as each browser does it: for this crate's tests,
//! the core's, and the app's. Never a real profile, and never in a release
//! (the core turns this on only with its test-hooks feature).
//!
//! The keys are made up too (`TestKeys`): a Mac's keychain secret and a
//! Linux keyring's are fixed words, and Windows' data protection is a
//! stand-in unless a test asks for the real one, which on Windows protects a
//! throwaway key for the person running the test.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::der::{int, seq, tlv, OCTET_STRING, OID};
use crate::keys::{KeyError, Keys};
use crate::nss::{AES256_CBC, DES_EDE3_CBC, HMAC_SHA256, PBES2, PBE_SHA1_3DES, PBKDF2};
use crate::{Browser, Home, Platform};

/// A saved password to put in a made-up profile.
#[derive(Clone, Debug)]
pub struct Fake {
    pub url: String,
    pub username: String,
    pub password: String,
    pub note: Option<String>,
    /// "Never save" for this site (Chromium): no password.
    pub never_save: bool,
    /// When its password last changed, in milliseconds since 1970.
    pub changed_ms: i64,
}

impl Fake {
    pub fn new(url: &str, username: &str, password: &str) -> Fake {
        Fake {
            url: url.into(),
            username: username.into(),
            password: password.into(),
            note: None,
            never_save: false,
            changed_ms: 1_767_225_600_000,
        }
    }

    pub fn changed(mut self, ms: i64) -> Fake {
        self.changed_ms = ms;
        self
    }

    pub fn with_note(mut self, note: &str) -> Fake {
        self.note = Some(note.into());
        self
    }

    pub fn never_save(url: &str) -> Fake {
        Fake {
            never_save: true,
            ..Fake::new(url, "", "")
        }
    }
}

/// How a made-up Chromium profile's passwords are encrypted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sealed {
    /// As the platform's browser does by default: macOS's keychain (v10),
    /// Linux with no keyring (v10), Windows' Local State key (v10).
    Usual,
    /// Linux with a keyring (v11).
    Keyring,
    /// Chrome's app-bound encryption on Windows (v20): only Chrome opens it.
    AppBound,
    /// Windows, with the key protected by Windows' real data protection for
    /// the person running the test (on Windows only).
    RealWindows,
}

/// Firefox's cipher for saved logins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirefoxCipher {
    /// Triple DES, with the key database's newer scheme (PBES2, AES-256).
    TripleDes,
    /// AES-256, as newer Firefox releases encrypt, and with a Primary
    /// Password, the key database's longer salt as they write it.
    Aes,
    /// Triple DES, with the key database's older scheme (SHA-1 and Triple DES).
    OldDatabase,
}

/// A Chromium profile to make.
pub struct ChromiumProfile<'a> {
    pub browser: Browser,
    /// Its folder in the browser's folder ("Default", "Profile 1").
    pub folder: &'a str,
    pub name: &'a str,
    pub email: Option<&'a str>,
    pub hosted_domain: Option<&'a str>,
    pub logins: Vec<Fake>,
    pub sealed: Sealed,
    /// Also some in `Login Data For Account`.
    pub account_logins: Vec<Fake>,
}

impl<'a> ChromiumProfile<'a> {
    pub fn new(browser: Browser, folder: &'a str, name: &'a str) -> Self {
        ChromiumProfile {
            browser,
            folder,
            name,
            email: None,
            hosted_domain: None,
            logins: Vec::new(),
            sealed: Sealed::Usual,
            account_logins: Vec::new(),
        }
    }
}

/// A Firefox profile to make.
pub struct FirefoxProfile<'a> {
    /// Its folder, under `Profiles/`.
    pub folder: &'a str,
    pub name: &'a str,
    pub email: Option<&'a str>,
    /// The one Firefox starts with.
    pub default: bool,
    pub logins: Vec<Fake>,
    pub primary_password: Option<&'a str>,
    pub cipher: FirefoxCipher,
}

impl<'a> FirefoxProfile<'a> {
    pub fn new(folder: &'a str, name: &'a str) -> Self {
        FirefoxProfile {
            folder,
            name,
            email: None,
            default: false,
            logins: Vec::new(),
            primary_password: None,
            cipher: FirefoxCipher::TripleDes,
        }
    }
}

static MADE: AtomicU32 = AtomicU32::new(0);

/// Every profile folder whose passwords were read, in this process: tests
/// check a profile nobody chose never is.
static READ: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn note_read(dir: &Path) {
    READ.lock()
        .unwrap_or_else(|p| p.into_inner())
        .push(dir.to_owned());
}

/// The profile folders under `root` whose passwords were read so far.
pub fn read_under(root: &Path) -> Vec<PathBuf> {
    READ.lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .filter(|d| d.starts_with(root))
        .cloned()
        .collect()
}

/// A throwaway home folder with made-up browsers in it. Removed when dropped.
pub struct Fixture {
    pub root: PathBuf,
    pub home: Home,
    keep: bool,
}

impl Fixture {
    /// A new, empty one, laid out as `platform` lays out a home folder.
    pub fn new(platform: Platform) -> Fixture {
        let n = MADE.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let root =
            std::env::temp_dir().join(format!("pb-browsers-{}-{n}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&root).expect("a throwaway folder");
        let home = Home::inside(&root, platform);
        std::fs::create_dir_all(&home.home).expect("a throwaway home folder");
        Fixture {
            root,
            home,
            keep: false,
        }
    }

    /// One that stays when dropped: for the app's tests, which hand its
    /// folder to the core.
    pub fn kept(platform: Platform, root: &Path) -> Fixture {
        std::fs::create_dir_all(root).expect("the test's folder");
        let home = Home::inside(root, platform);
        std::fs::create_dir_all(&home.home).expect("a throwaway home folder");
        Fixture {
            root: root.to_owned(),
            home,
            keep: true,
        }
    }

    fn platform(&self) -> Platform {
        self.home.platform
    }

    /// The browser's folder, made.
    pub fn chromium_dir(&self, browser: Browser) -> PathBuf {
        let made = self.home.clone();
        // Make the folder so `chromium_dirs` finds it, then ask where it is.
        let probe = match (made.platform, browser) {
            (Platform::Windows, Browser::Chrome) => made.app_data.join("Google/Chrome/User Data"),
            (Platform::Windows, Browser::Edge) => made.app_data.join("Microsoft/Edge/User Data"),
            (Platform::Windows, Browser::Brave) => {
                made.app_data.join("BraveSoftware/Brave-Browser/User Data")
            }
            (Platform::Windows, Browser::Vivaldi) => made.app_data.join("Vivaldi/User Data"),
            (Platform::Windows, Browser::Chromium) => made.app_data.join("Chromium/User Data"),
            (Platform::Windows, Browser::Opera) => made.roaming.join("Opera Software/Opera Stable"),
            (Platform::Mac, Browser::Chrome) => made.app_data.join("Google/Chrome"),
            (Platform::Mac, Browser::Edge) => made.app_data.join("Microsoft Edge"),
            (Platform::Mac, Browser::Brave) => made.app_data.join("BraveSoftware/Brave-Browser"),
            (Platform::Mac, Browser::Vivaldi) => made.app_data.join("Vivaldi"),
            (Platform::Mac, Browser::Chromium) => made.app_data.join("Chromium"),
            (Platform::Mac, Browser::Opera) => made.app_data.join("com.operasoftware.Opera"),
            (Platform::Linux, Browser::Chrome) => made.app_data.join("google-chrome"),
            (Platform::Linux, Browser::Edge) => made.app_data.join("microsoft-edge"),
            (Platform::Linux, Browser::Brave) => made.app_data.join("BraveSoftware/Brave-Browser"),
            (Platform::Linux, Browser::Vivaldi) => made.app_data.join("vivaldi"),
            (Platform::Linux, Browser::Chromium) => made.app_data.join("chromium"),
            (Platform::Linux, Browser::Opera) => made.app_data.join("opera"),
            _ => panic!("not a Chromium browser"),
        };
        let dir: PathBuf = probe.components().collect();
        std::fs::create_dir_all(&dir).expect("the browser's folder");
        let found = self.home.chromium_dirs(browser);
        assert_eq!(
            found,
            std::slice::from_ref(&dir),
            "the browser's folder is where it looks"
        );
        dir
    }

    /// Makes a Chromium profile; answers its folder.
    pub fn chromium(&self, profile: ChromiumProfile<'_>) -> PathBuf {
        let browser_dir = self.chromium_dir(profile.browser);
        let profile_dir = browser_dir.join(profile.folder);
        std::fs::create_dir_all(&profile_dir).expect("the profile's folder");
        let state_path = browser_dir.join("Local State");
        let mut state: Value = std::fs::read_to_string(&state_path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| json!({}));
        state["profile"]["info_cache"][profile.folder] = json!({
            "name": profile.name,
            "user_name": profile.email.unwrap_or(""),
            "gaia_name": "",
            "hosted_domain": profile.hosted_domain.unwrap_or("NO_HOSTED_DOMAIN"),
            "is_using_default_name": false,
        });
        let key = windows_key(profile.browser);
        if self.platform() == Platform::Windows && state["os_crypt"]["encrypted_key"].is_null() {
            let protected = match profile.sealed {
                Sealed::RealWindows => real_protect(&key),
                _ => [TEST_DPAPI, &key[..]].concat(),
            };
            state["os_crypt"]["encrypted_key"] =
                Value::String(STANDARD.encode([b"DPAPI".as_slice(), &protected].concat()));
        }
        std::fs::write(&state_path, state.to_string()).expect("Local State");
        std::fs::write(
            profile_dir.join("Preferences"),
            "{\"profile\":{\"name\":\"x\"}}",
        )
        .expect("Preferences");
        let seal = |value: &str| -> Vec<u8> { self.seal(profile.browser, profile.sealed, value) };
        write_login_data(&profile_dir.join("Login Data"), &profile.logins, &seal);
        if !profile.account_logins.is_empty() {
            write_login_data(
                &profile_dir.join("Login Data For Account"),
                &profile.account_logins,
                &seal,
            );
        }
        profile_dir
    }

    fn seal(&self, browser: Browser, sealed: Sealed, value: &str) -> Vec<u8> {
        if value.is_empty() {
            return Vec::new();
        }
        match (self.platform(), sealed) {
            (_, Sealed::AppBound) => [b"v20".as_slice(), &[7u8; 60]].concat(),
            (Platform::Windows, _) => {
                let key = windows_key(browser);
                let nonce = &bytes(12, value)[..];
                let sealed = Aes256Gcm::new(key.as_slice().into())
                    .encrypt(Nonce::from_slice(nonce), value.as_bytes())
                    .expect("AES-GCM seals");
                [b"v10".as_slice(), nonce, &sealed].concat()
            }
            (Platform::Mac, _) => {
                let key = crate::chromium::cbc_key(&TestKeys::secret(browser), 1003);
                [b"v10".as_slice(), &cbc_128(&key, value.as_bytes())].concat()
            }
            (Platform::Linux, Sealed::Keyring) => {
                let key = crate::chromium::cbc_key(&TestKeys::secret(browser), 1);
                [b"v11".as_slice(), &cbc_128(&key, value.as_bytes())].concat()
            }
            (Platform::Linux, _) => {
                let key = crate::chromium::cbc_key(b"peanuts", 1);
                [b"v10".as_slice(), &cbc_128(&key, value.as_bytes())].concat()
            }
        }
    }

    /// Makes a Firefox profile, and lists it in `profiles.ini`; answers its folder.
    pub fn firefox(&self, profile: FirefoxProfile<'_>) -> PathBuf {
        let root = match self.platform() {
            Platform::Windows => self.home.roaming.join("Mozilla").join("Firefox"),
            Platform::Mac => self.home.app_data.join("Firefox"),
            Platform::Linux => self.home.home.join(".mozilla").join("firefox"),
        };
        let dir = root.join("Profiles").join(profile.folder);
        std::fs::create_dir_all(&dir).expect("the profile's folder");
        // profiles.ini: this profile, and it as the install's default.
        let ini_path = root.join("profiles.ini");
        let mut ini = std::fs::read_to_string(&ini_path).unwrap_or_default();
        let n = ini.matches("[Profile").count();
        ini.push_str(&format!(
            "[Profile{n}]\nName={}\nIsRelative=1\nPath=Profiles/{}\n\n",
            profile.name, profile.folder
        ));
        if profile.default {
            ini.push_str(&format!(
                "[Install{n}F96D1932A9F858E]\nDefault=Profiles/{}\nLocked=1\n\n",
                profile.folder
            ));
        }
        std::fs::write(&ini_path, ini).expect("profiles.ini");
        if let Some(email) = profile.email {
            std::fs::write(
                dir.join("signedInUser.json"),
                json!({ "version": 1, "accountData": { "email": email, "verified": true } })
                    .to_string(),
            )
            .expect("signedInUser.json");
        }
        std::fs::write(
            dir.join("prefs.js"),
            "// Mozilla User Preferences\n\nuser_pref(\"browser.startup.page\", 3);\n",
        )
        .expect("prefs.js");
        write_firefox(&dir, &profile);
        dir
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

/// What the stand-in for Windows' data protection puts before a key.
const TEST_DPAPI: &[u8] = b"TEST-DPAPI:";

#[cfg(windows)]
fn real_protect(key: &[u8]) -> Vec<u8> {
    crate::keys::os::protect(key)
}

#[cfg(not(windows))]
fn real_protect(_: &[u8]) -> Vec<u8> {
    panic!("Windows' data protection is only on Windows")
}

/// A made-up browser's Windows key: fixed per browser.
fn windows_key(browser: Browser) -> [u8; 32] {
    bytes(32, &format!("windows key for {}", browser.key()))
        .try_into()
        .expect("32 bytes")
}

/// Bytes that are the same every time for the same label.
fn bytes(n: usize, label: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut counter = 0u32;
    while out.len() < n {
        let mut hash = Sha256::new();
        hash.update(label.as_bytes());
        hash.update(counter.to_be_bytes());
        out.extend_from_slice(&hash.finalize());
        counter += 1;
    }
    out.truncate(n);
    out
}

fn cbc_128(key: &[u8; 16], plain: &[u8]) -> Vec<u8> {
    cbc::Encryptor::<aes::Aes128>::new(key.into(), &[b' '; 16].into())
        .encrypt_padded_vec_mut::<Pkcs7>(plain)
}

/// Milliseconds since 1970 as Chromium's microseconds since 1601.
fn webkit(ms: i64) -> i64 {
    (ms + 11_644_473_600_000) * 1000
}

fn write_login_data(path: &Path, logins: &[Fake], seal: &dyn Fn(&str) -> Vec<u8>) {
    let _ = std::fs::remove_file(path);
    let conn = Connection::open(path).expect("a Login Data file");
    conn.execute_batch(
        "CREATE TABLE meta(key LONGVARCHAR NOT NULL UNIQUE PRIMARY KEY, value LONGVARCHAR);
         INSERT INTO meta VALUES ('version', '43'), ('last_compatible_version', '40');
         CREATE TABLE logins (origin_url VARCHAR NOT NULL, action_url VARCHAR,
           username_element VARCHAR, username_value VARCHAR, password_element VARCHAR,
           password_value BLOB, submit_element VARCHAR, signon_realm VARCHAR NOT NULL,
           date_created INTEGER NOT NULL, blacklisted_by_user INTEGER NOT NULL,
           scheme INTEGER NOT NULL, password_type INTEGER, times_used INTEGER,
           form_data BLOB, display_name VARCHAR, icon_url VARCHAR, federation_url VARCHAR,
           skip_zero_click INTEGER, generation_upload_status INTEGER,
           possible_username_pairs BLOB, id INTEGER PRIMARY KEY AUTOINCREMENT,
           date_last_used INTEGER NOT NULL DEFAULT 0, moving_blocked_for BLOB,
           date_password_modified INTEGER NOT NULL DEFAULT 0,
           UNIQUE (origin_url, username_element, username_value, password_element, signon_realm));
         CREATE TABLE password_notes (id INTEGER PRIMARY KEY AUTOINCREMENT,
           parent_id INTEGER NOT NULL REFERENCES logins ON UPDATE CASCADE ON DELETE CASCADE
             DEFERRABLE INITIALLY DEFERRED,
           key VARCHAR NOT NULL, value BLOB, date_created INTEGER NOT NULL,
           confidential INTEGER, UNIQUE (parent_id, key));",
    )
    .expect("Chromium's tables");
    for login in logins {
        let realm = match login.url.strip_prefix("androidapp://") {
            Some(package) => format!("android://hash123==@{package}/"),
            None => {
                let origin = login
                    .url
                    .splitn(4, '/')
                    .take(3)
                    .collect::<Vec<_>>()
                    .join("/");
                format!("{origin}/")
            }
        };
        let origin = if login.url.starts_with("androidapp://") {
            String::new()
        } else {
            login.url.clone()
        };
        conn.execute(
            "INSERT INTO logins (origin_url, action_url, username_element, username_value,
               password_element, password_value, submit_element, signon_realm, date_created,
               blacklisted_by_user, scheme, date_password_modified)
             VALUES (?1, '', 'username', ?2, 'password', ?3, '', ?4, ?5, ?6, 0, ?7)",
            params![
                origin,
                login.username,
                seal(&login.password),
                realm,
                webkit(login.changed_ms - 86_400_000),
                login.never_save as i64,
                webkit(login.changed_ms),
            ],
        )
        .expect("a login");
        if let Some(note) = &login.note {
            let id = conn.last_insert_rowid();
            // As Chrome writes it: the sealed bytes as a text value, in a
            // column it declares a blob.
            conn.execute(
                "INSERT INTO password_notes (parent_id, key, value, date_created, confidential)
                 VALUES (?1, '', CAST(?2 AS TEXT), ?3, 0)",
                params![id, seal(note), webkit(login.changed_ms)],
            )
            .expect("a note");
        }
    }
}

/// The CKA_ID NSS gives the key Firefox encrypts logins with.
const SDR_ID: [u8; 16] = [0xf8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];

fn write_firefox(dir: &Path, profile: &FirefoxProfile<'_>) {
    let password = profile.primary_password.unwrap_or("");
    // As Firefox writes it today: a longer salt once a Primary Password is
    // set. Triple DES stands for the Firefox releases before that.
    let salt_len = match (profile.cipher, password.is_empty()) {
        (FirefoxCipher::Aes, false) => 48,
        _ => 20,
    };
    let salt = bytes(salt_len, &format!("global salt {}", profile.folder));
    let (sdr_key, cipher_oid, iv_len) = match profile.cipher {
        FirefoxCipher::Aes => (bytes(32, "sdr key aes"), AES256_CBC, 16),
        _ => (bytes(24, "sdr key 3des"), DES_EDE3_CBC, 8),
    };
    let old = profile.cipher == FirefoxCipher::OldDatabase;
    let pbe = |plain: &[u8], label: &str| {
        if old {
            pbe_3des(plain, &salt, password.as_bytes(), label)
        } else {
            pbe_aes(
                plain,
                &salt,
                password.as_bytes(),
                label,
                if password.is_empty() { 1 } else { 10_000 },
            )
        }
    };
    let key_db = dir.join("key4.db");
    let _ = std::fs::remove_file(&key_db);
    let conn = Connection::open(&key_db).expect("key4.db");
    conn.execute_batch(
        "CREATE TABLE metaData (id PRIMARY KEY UNIQUE ON CONFLICT REPLACE, item1, item2);
         CREATE TABLE nssPrivate (id PRIMARY KEY UNIQUE ON CONFLICT ABORT, a0, a1, a2, a3,
           a10, a11, a12, a80, a81, a82, a83, a84, a85, a86, a87, a88, a89, a8a, a8b, a90, a100,
           a101, a102, a103, a104, a105, a106, a107, a108, a109, a10a, a10b, a10c, a110, a111,
           a120, a121, a122, a123, a124, a125, a126, a127, a128, a129, a130, a131, a132, a133,
           a134, a160, a161, a162, a163, a164, a165, a166, a170, a200, a201, a202, a210, a300,
           a301, a302, a400, a401, a402, a403, a404, a405, a406, a480, a481, a482, a500, a501,
           a502, a503, a40000211, a40000212, a80000001, ace534351, ace534352, ace534353,
           ace534354, ace534355, ace534356, ace534357, ace534358, ace534364, ace534365,
           ace534366, ace534367, ace534368, ace534369, ace534373, ace534374, ace536351,
           ace536352, ace536353, ace536354, ace536355, ace536356, ace536357, ace536358,
           ace536359, ace53635a, ace53635b, ace53635c, ace53635d, ace53635e, ace53635f,
           ace536360, ace5363b4, ace5363b5, ad5a0db00);",
    )
    .expect("key4.db's tables");
    conn.execute(
        "INSERT INTO metaData (id, item1, item2) VALUES ('password', ?1, ?2)",
        params![salt, pbe(b"password-check", "check")],
    )
    .expect("the password check");
    conn.execute(
        "INSERT INTO nssPrivate (id, a11, a102) VALUES (1, ?1, ?2)",
        params![pbe(&sdr_key, "key"), SDR_ID.to_vec()],
    )
    .expect("the key");
    drop(conn);
    let encrypt = |value: &str, label: &str| -> String {
        let iv = bytes(iv_len, label);
        let ciphertext = if cipher_oid == AES256_CBC {
            cbc::Encryptor::<aes::Aes256>::new_from_slices(&sdr_key, &iv)
                .expect("a key")
                .encrypt_padded_vec_mut::<Pkcs7>(value.as_bytes())
        } else {
            cbc::Encryptor::<des::TdesEde3>::new_from_slices(&sdr_key, &iv)
                .expect("a key")
                .encrypt_padded_vec_mut::<Pkcs7>(value.as_bytes())
        };
        STANDARD.encode(seq(&[
            tlv(OCTET_STRING, &SDR_ID),
            seq(&[tlv(OID, cipher_oid), tlv(OCTET_STRING, &iv)]),
            tlv(OCTET_STRING, &ciphertext),
        ]))
    };
    let logins: Vec<Value> = profile
        .logins
        .iter()
        .enumerate()
        .map(|(i, l)| {
            json!({
                "id": i + 1,
                "hostname": l.url,
                "httpRealm": null,
                "formSubmitURL": l.url,
                "usernameField": "username",
                "passwordField": "password",
                "encryptedUsername": encrypt(&l.username, &format!("user {i}")),
                "encryptedPassword": encrypt(&l.password, &format!("password {i}")),
                "guid": format!("{{0000000{i}-0000-0000-0000-000000000000}}"),
                "encType": 1,
                "timeCreated": l.changed_ms - 86_400_000,
                "timeLastUsed": l.changed_ms,
                "timePasswordChanged": l.changed_ms,
                "timesUsed": 1,
            })
        })
        .collect();
    std::fs::write(
        dir.join("logins.json"),
        json!({ "nextId": logins.len() + 1, "logins": logins, "version": 3 }).to_string(),
    )
    .expect("logins.json");
}

/// The salt and the password as NSS hashes them: with SHA-384 under its
/// 48-byte salt, with SHA-1 under its 20-byte one.
fn salted(global_salt: &[u8], password: &[u8]) -> Vec<u8> {
    if global_salt.len() == 48 {
        let mut hash = sha2::Sha384::new();
        hash.update(global_salt);
        hash.update(password);
        hash.finalize().to_vec()
    } else {
        let mut hash = sha1::Sha1::new();
        sha1::Digest::update(&mut hash, global_salt);
        sha1::Digest::update(&mut hash, password);
        sha1::Digest::finalize(hash).to_vec()
    }
}

/// NSS's newer scheme: PBES2, PBKDF2 with HMAC-SHA256, AES-256-CBC.
fn pbe_aes(plain: &[u8], global_salt: &[u8], password: &[u8], label: &str, rounds: u32) -> Vec<u8> {
    let kdf_salt = bytes(32, &format!("kdf salt {label}"));
    let iv14 = bytes(14, &format!("iv {label}"));
    let hashed = salted(global_salt, password);
    let mut key = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(&hashed, &kdf_salt, rounds, &mut key);
    let iv: Vec<u8> = [&[OCTET_STRING, 14][..], &iv14].concat();
    let ciphertext = cbc::Encryptor::<aes::Aes256>::new_from_slices(&key, &iv)
        .expect("a key")
        .encrypt_padded_vec_mut::<Pkcs7>(plain);
    seq(&[
        seq(&[
            tlv(OID, PBES2),
            seq(&[
                seq(&[
                    tlv(OID, PBKDF2),
                    seq(&[
                        tlv(OCTET_STRING, &kdf_salt),
                        int(rounds),
                        int(32),
                        seq(&[tlv(OID, HMAC_SHA256)]),
                    ]),
                ]),
                seq(&[tlv(OID, AES256_CBC), tlv(OCTET_STRING, &iv14)]),
            ]),
        ]),
        tlv(OCTET_STRING, &ciphertext),
    ])
}

/// NSS's older scheme: PKCS #12's SHA-1 and Triple DES.
fn pbe_3des(plain: &[u8], global_salt: &[u8], password: &[u8], label: &str) -> Vec<u8> {
    let entry_salt = bytes(20, &format!("entry salt {label}"));
    let hashed = salted(global_salt, password);
    let (key, iv) = crate::nss::triple_des_key(&hashed, &entry_salt);
    let ciphertext = cbc::Encryptor::<des::TdesEde3>::new_from_slices(&key, &iv)
        .expect("a key")
        .encrypt_padded_vec_mut::<Pkcs7>(plain);
    seq(&[
        seq(&[
            tlv(OID, PBE_SHA1_3DES),
            seq(&[tlv(OCTET_STRING, &entry_salt), int(1)]),
        ]),
        tlv(OCTET_STRING, &ciphertext),
    ])
}

/// The keys the made-up profiles are encrypted with. A test can have the
/// keychain say no, or the keyring be locked.
#[derive(Default)]
pub struct TestKeys {
    pub refuse: Option<KeyError>,
    /// How many times a key was asked for.
    pub asked: std::cell::Cell<u32>,
}

impl TestKeys {
    /// A made-up browser's keychain or keyring secret: fixed per browser.
    pub fn secret(browser: Browser) -> Vec<u8> {
        format!("test safe storage for {}", browser.key()).into_bytes()
    }

    pub fn refusing(error: KeyError) -> TestKeys {
        TestKeys {
            refuse: Some(error),
            ..TestKeys::default()
        }
    }
}

impl Keys for TestKeys {
    fn safe_storage(&self, browser: Browser) -> Result<Zeroizing<Vec<u8>>, KeyError> {
        self.asked.set(self.asked.get() + 1);
        match self.refuse {
            Some(e) => Err(e),
            None => Ok(Zeroizing::new(TestKeys::secret(browser))),
        }
    }

    fn unprotect(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, KeyError> {
        self.asked.set(self.asked.get() + 1);
        if let Some(e) = self.refuse {
            return Err(e);
        }
        match blob.strip_prefix(TEST_DPAPI) {
            Some(key) => Ok(Zeroizing::new(key.to_vec())),
            // A key the real data protection protected (Sealed::RealWindows).
            None => crate::SystemKeys.unprotect(blob),
        }
    }
}
