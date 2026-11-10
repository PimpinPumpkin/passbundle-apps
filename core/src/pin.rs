//! Unlocking with a PIN (DESIGN.md in the meta repo, "Unlock with a PIN";
//! vectors in tests/vectors/pin-unlock-v1.json, a copy of the meta repo's).
//!
//! What's kept for an account that has a PIN is its user key, wrapped under a
//! key made from the PIN, and that record sealed to the device
//! (`SecretStore::seal_to_device`), in the account's folder (`pin.json`)
//! beside a count of wrong tries. The PIN itself is never kept, and nothing
//! here reaches the server. `App` (app/pin.rs) decides when a PIN may be
//! set, used, and removed; this file is the key, the wrap, and the file.
//!
//! - PIN key: Argon2id (version 0x13) of the PIN as typed, with a random
//!   16-byte salt, 32 bytes. v1 writes 64 MiB, 3 passes, 4 lanes, and opens
//!   nothing weaker (`usable`).
//! - Wrapping key: HKDF-Expand-SHA256 with the PIN key as the PRK, 64 bytes,
//!   its info the label, the server, the account's id there, and its id on
//!   this device, so a wrapped key opens for no other account, server, or
//!   version.
//! - Wrapped key: the user key as a type 2 string under the wrapping key,
//!   the construction every vault key uses (crypto.rs). A wrong PIN is a MAC
//!   that doesn't match.

use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD, Engine};
use hkdf::Hkdf;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::crypto::{EncString, SymmetricKey};
use crate::error::{Error, Result};
use crate::store::Store;

/// The fewest characters a PIN has, and the most.
pub const MIN_CHARACTERS: usize = 4;
pub const MAX_CHARACTERS: usize = 128;

/// This many wrong PINs in a row removes the PIN.
pub const MAX_WRONG_TRIES: u32 = 5;

/// What v1 writes, and the least it opens.
pub const MEMORY_MIB: u32 = 64;
pub const ITERATIONS: u32 = 3;
pub const PARALLELISM: u32 = 4;

/// The most the core takes anywhere (kdf.rs's bounds for a master password).
const MOST_MEMORY_MIB: u32 = 1024;
const MOST_ITERATIONS: u32 = 10;
const MOST_PARALLELISM: u32 = 16;

/// The wrapping key's label: the version is part of it.
const LABEL: &str = "keykeeper PIN unlock v1";
const VERSION: u32 = 1;

/// The account's file, in its own folder.
pub(crate) const FILE: &str = "pin.json";

/// A record whose Argon2id settings are under the floor (64 MiB, 3 passes,
/// 4 lanes) or over the most the core takes anywhere (1,024 MiB, 10, 16).
/// The app never shows this: such a PIN is removed (app/pin.rs).
const UNUSABLE: Error = Error::Damaged("a PIN's settings outside what PassBundle opens");

/// Whom a wrapped key is for. None of these is a secret; each goes into the
/// wrapping key.
#[derive(Clone, Copy, Debug)]
pub struct For<'a> {
    /// The server's address, as the account's settings keep it.
    pub server: &'a str,
    /// The account's id on that server (the sync's `profile.id`).
    pub user_id: &'a str,
    /// The account's own id on this device (accounts.rs).
    pub account: &'a str,
}

/// Whether `pin` may be a PIN: 4 to 128 characters, any characters. Nothing
/// is trimmed or changed, here or later.
pub fn check(pin: &str) -> Result<()> {
    match pin.chars().count() {
        n if n < MIN_CHARACTERS => Err(Error::Refused("a PIN needs at least 4 characters")),
        n if n > MAX_CHARACTERS => Err(Error::Refused("a PIN can have at most 128 characters")),
        _ => Ok(()),
    }
}

/// The record's Argon2id settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cost {
    memory_mib: u32,
    iterations: u32,
    parallelism: u32,
}

impl Cost {
    const FLOOR: Cost = Cost {
        memory_mib: MEMORY_MIB,
        iterations: ITERATIONS,
        parallelism: PARALLELISM,
    };

    /// At the floor or above it, and within what the core takes anywhere.
    /// Checked before a PIN is used, so a changed record can't make
    /// guessing cheaper or a try cost the device its memory.
    fn usable(&self) -> Result<()> {
        if !(MEMORY_MIB..=MOST_MEMORY_MIB).contains(&self.memory_mib)
            || !(ITERATIONS..=MOST_ITERATIONS).contains(&self.iterations)
            || !(PARALLELISM..=MOST_PARALLELISM).contains(&self.parallelism)
        {
            return Err(UNUSABLE);
        }
        Ok(())
    }
}

/// The 32-byte PIN key. Never kept.
fn pin_key(pin: &str, salt: &[u8], cost: Cost) -> Result<Zeroizing<[u8; 32]>> {
    cost.usable()?;
    let params = Params::new(
        cost.memory_mib * 1024,
        cost.iterations,
        cost.parallelism,
        Some(32),
    )
    .map_err(|_| UNUSABLE)?;
    let mut out = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(pin.as_bytes(), salt, &mut *out)
        .map_err(|_| UNUSABLE)?;
    Ok(out)
}

/// The label, then, each after one zero byte, the server, the account's id
/// there, and its id on this device. None of the three may hold a zero
/// byte, so no two sets of them make the same info.
fn info(label: &str, whom: &For) -> Result<Vec<u8>> {
    let mut info = label.as_bytes().to_vec();
    for part in [whom.server, whom.user_id, whom.account] {
        if part.contains('\0') {
            return Err(Error::Damaged("an account's id or server with a zero byte"));
        }
        info.push(0);
        info.extend_from_slice(part.as_bytes());
    }
    Ok(info)
}

/// The key the user key is wrapped under: HKDF-Expand-SHA256 with the PIN
/// key as the PRK, 64 bytes (32 for AES-256-CBC, then 32 for HMAC-SHA256).
fn wrapping_key(pin_key: &[u8; 32], label: &str, whom: &For) -> Result<SymmetricKey> {
    let hkdf = Hkdf::<Sha256>::from_prk(pin_key).expect("a 32-byte PRK is long enough");
    let mut okm = Zeroizing::new([0u8; 64]);
    hkdf.expand(&info(label, whom)?, &mut *okm)
        .expect("64 bytes is a valid HKDF-SHA256 length");
    SymmetricKey::from_bytes(&*okm)
}

/// What a PIN opens: the user key wrapped under it, with the salt and
/// settings its key was made with. It never reaches the disk as it is.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    version: u32,
    /// 16 random bytes, new at each setup, in base64.
    salt: String,
    memory_mib: u32,
    iterations: u32,
    parallelism: u32,
    /// The user key, as a type 2 string under the wrapping key.
    wrapped: String,
}

/// What opening a record with a PIN came to.
pub enum Opened {
    Key(SymmetricKey),
    /// The MAC didn't match: not the PIN (or not the account, server, or
    /// version) the record was made with.
    WrongPin,
}

impl Record {
    /// Wraps `user_key` under `pin` for one account, with a new salt and IV.
    pub fn make(pin: &str, user_key: &SymmetricKey, whom: &For) -> Result<Record> {
        let mut salt = [0u8; 16];
        let mut iv = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut salt);
        rand::thread_rng().fill_bytes(&mut iv);
        Record::make_with(pin, user_key, whom, salt, iv)
    }

    /// As `make`, with a given salt and IV: for the vectors.
    pub(crate) fn make_with(
        pin: &str,
        user_key: &SymmetricKey,
        whom: &For,
        salt: [u8; 16],
        iv: [u8; 16],
    ) -> Result<Record> {
        check(pin)?;
        let cost = Cost::FLOOR;
        let key = wrapping_key(&*pin_key(pin, &salt, cost)?, LABEL, whom)?;
        Ok(Record {
            version: VERSION,
            salt: STANDARD.encode(salt),
            memory_mib: cost.memory_mib,
            iterations: cost.iterations,
            parallelism: cost.parallelism,
            wrapped: key.encrypt_with_iv(&user_key.to_bytes(), iv).to_string(),
        })
    }

    /// The record as it's sealed to the device.
    pub fn to_json(&self) -> Zeroizing<String> {
        Zeroizing::new(serde_json::to_string(self).expect("a record is JSON"))
    }

    pub fn from_json(text: &[u8]) -> Result<Record> {
        serde_json::from_slice(text).map_err(|_| Error::Damaged("a PIN's record that isn't one"))
    }

    fn cost(&self) -> Cost {
        Cost {
            memory_mib: self.memory_mib,
            iterations: self.iterations,
            parallelism: self.parallelism,
        }
    }

    /// Whether this version opens the record at all: its version, and
    /// settings at the floor or above. Checked before a try is counted and
    /// before the PIN is used.
    pub fn usable(&self) -> Result<()> {
        if self.version != VERSION {
            return Err(Error::NewerApp);
        }
        self.cost().usable()
    }

    /// The user key, if `pin` is the PIN the record was made with, for this
    /// account. An error is a record that can't be opened with any PIN.
    pub fn open(&self, pin: &str, whom: &For) -> Result<Opened> {
        self.usable()?;
        let salt = STANDARD
            .decode(&self.salt)
            .map_err(|_| Error::Damaged("a salt that isn't base64"))?;
        let wrapped = EncString::parse(&self.wrapped)?;
        let key = wrapping_key(&*pin_key(pin, &salt, self.cost())?, LABEL, whom)?;
        match wrapped.decrypt_key(&key) {
            Ok(user_key) => Ok(Opened::Key(user_key)),
            // The only refusal opening a type 2 string has is its MAC.
            Err(Error::Refused(_)) => Ok(Opened::WrongPin),
            Err(e) => Err(e),
        }
    }
}

/// Never the wrapped key's or the salt's own words: a record is only ever
/// shown by what it is.
impl std::fmt::Debug for Record {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("pin::Record(..)")
    }
}

/// What a PIN was made for: SHA-256 of the account's protected user key as
/// the sync has it (`profile.key`), in base64. It changes with the master
/// password, the KDF settings, and the account's key.
pub fn made_for(protected_user_key: &str) -> String {
    STANDARD.encode(Sha256::digest(protected_user_key.as_bytes()))
}

/// The account's `pin.json`: the record sealed to the device, the count of
/// wrong tries in a row, and what the PIN was made for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Kept {
    version: u32,
    pub wrong_tries: u32,
    made_for: String,
    /// The record, sealed to the device, in base64.
    sealed: String,
}

impl Kept {
    pub fn new(protected_user_key: &str, sealed: &[u8]) -> Kept {
        Kept {
            version: VERSION,
            wrong_tries: 0,
            made_for: made_for(protected_user_key),
            sealed: STANDARD.encode(sealed),
        }
    }

    /// The account's kept PIN, if it has one this version reads.
    pub fn read(store: &Store) -> Option<Kept> {
        let kept: Kept = serde_json::from_str(&store.read(FILE)?).ok()?;
        (kept.version == VERSION).then_some(kept)
    }

    /// Whether the account has a PIN file at all, readable or not.
    pub fn is_kept(store: &Store) -> bool {
        store.read(FILE).is_some()
    }

    pub fn write(&self, store: &Store) -> Result<()> {
        store.write(
            FILE,
            &serde_json::to_string(self).expect("a kept PIN is JSON"),
        )
    }

    /// Whether the PIN was made for the account's keys as the sync has them
    /// now (`profile.key`).
    pub fn made_for(&self, protected_user_key: Option<&str>) -> bool {
        protected_user_key.is_some_and(|key| made_for(key) == self.made_for)
    }

    /// The record as the device sealed it.
    pub fn sealed(&self) -> Option<Vec<u8>> {
        STANDARD.decode(&self.sealed).ok()
    }
}

/// Removes the account's PIN, if it has one.
pub fn forget(store: &Store) {
    store.remove(FILE);
}

/// A master password unlock clears the count of wrong tries.
pub fn clear_tries(store: &Store) {
    if let Some(mut kept) = Kept::read(store) {
        if kept.wrong_tries != 0 {
            kept.wrong_tries = 0;
            let _ = kept.write(store);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn vectors() -> Value {
        serde_json::from_str(include_str!("../tests/vectors/pin-unlock-v1.json")).unwrap()
    }

    fn bytes(v: &Value) -> Vec<u8> {
        STANDARD.decode(v.as_str().unwrap()).unwrap()
    }

    fn whom(v: &Value) -> For<'_> {
        For {
            server: v["server"].as_str().unwrap(),
            user_id: v["userId"].as_str().unwrap(),
            account: v["account"].as_str().unwrap(),
        }
    }

    fn cost(v: &Value) -> Cost {
        Cost {
            memory_mib: v["memoryMib"].as_u64().unwrap() as u32,
            iterations: v["iterations"].as_u64().unwrap() as u32,
            parallelism: v["parallelism"].as_u64().unwrap() as u32,
        }
    }

    fn record_of(text: &str) -> Record {
        Record::from_json(text.as_bytes()).unwrap()
    }

    fn opens(record: &Record, pin: &str, whom: &For) -> Option<SymmetricKey> {
        match record.open(pin, whom).unwrap() {
            Opened::Key(key) => Some(key),
            Opened::WrongPin => None,
        }
    }

    #[test]
    fn the_floor_is_the_vectors() {
        let v = vectors();
        assert_eq!(cost(&v["floor"]), Cost::FLOOR);
        assert_eq!(v["wrap"]["label"], LABEL);
    }

    #[test]
    fn the_pin_key_the_wrapping_key_and_the_record_are_the_vectors_byte_for_byte() {
        let v = vectors();
        let w = &v["wrap"];
        let pin = w["pin"].as_str().unwrap();
        let key = pin_key(pin, &bytes(&w["salt"]), cost(w)).unwrap();
        assert_eq!(&key[..], &bytes(&w["pinKey"])[..]);
        assert_eq!(info(LABEL, &whom(w)).unwrap(), bytes(&w["info"]));
        let wrapping = wrapping_key(&key, LABEL, &whom(w)).unwrap();
        assert_eq!(&wrapping.to_bytes()[..], &bytes(&w["wrappingKey"])[..]);

        let user_key = SymmetricKey::from_bytes(&bytes(&w["userKey"])).unwrap();
        let record = Record::make_with(
            pin,
            &user_key,
            &whom(w),
            bytes(&w["salt"]).try_into().unwrap(),
            bytes(&w["iv"]).try_into().unwrap(),
        )
        .unwrap();
        assert_eq!(record.wrapped, w["wrapped"].as_str().unwrap());
        assert_eq!(record.to_json().as_str(), w["record"].as_str().unwrap());

        // And the vectors' own record opens to the user key.
        let theirs = record_of(w["record"].as_str().unwrap());
        assert!(opens(&theirs, pin, &whom(w)).unwrap() == user_key);
    }

    #[test]
    fn a_pin_is_used_as_typed() {
        let v = vectors();
        let w = &v["wrap"];
        let a = &v["asTyped"];
        let pin = a["pin"].as_str().unwrap();
        assert_eq!(pin.as_bytes(), &bytes(&a["pinUtf8"])[..]);
        let key = pin_key(pin, &bytes(&a["salt"]), cost(a)).unwrap();
        assert_eq!(&key[..], &bytes(&a["pinKey"])[..]);
        let user_key = SymmetricKey::from_bytes(&bytes(&w["userKey"])).unwrap();
        let record = Record::make_with(
            pin,
            &user_key,
            &whom(w),
            bytes(&a["salt"]).try_into().unwrap(),
            bytes(&a["iv"]).try_into().unwrap(),
        )
        .unwrap();
        assert_eq!(record.wrapped, a["wrapped"].as_str().unwrap());
        // Without its spaces, or with another capital, it's another PIN.
        assert!(opens(&record, pin.trim(), &whom(w)).is_none());
        assert!(opens(&record, &pin.to_lowercase(), &whom(w)).is_none());
        assert!(opens(&record, pin, &whom(w)).is_some());
    }

    #[test]
    fn a_wrong_pin_another_account_server_or_version_and_a_changed_record_are_refused() {
        let v = vectors();
        let w = &v["wrap"];
        let r = &v["refused"];
        let pin = w["pin"].as_str().unwrap();
        let record = record_of(w["record"].as_str().unwrap());
        let right = whom(w);

        assert!(opens(&record, r["wrongPin"].as_str().unwrap(), &right).is_none());
        let other_account = For {
            account: r["otherAccount"].as_str().unwrap(),
            ..right
        };
        let other_server = For {
            server: r["otherServer"].as_str().unwrap(),
            ..right
        };
        let other_user = For {
            user_id: r["otherUserId"].as_str().unwrap(),
            ..right
        };
        for other in [&other_account, &other_server, &other_user] {
            assert!(opens(&record, pin, other).is_none());
        }
        // The other version's label makes another key.
        let key = pin_key(pin, &bytes(&w["salt"]), cost(w)).unwrap();
        let other_label = wrapping_key(&key, r["otherLabel"].as_str().unwrap(), &right).unwrap();
        assert!(EncString::parse(w["wrapped"].as_str().unwrap())
            .unwrap()
            .decrypt_key(&other_label)
            .is_err());

        // The same user key wrapped, with the right PIN, for each of the
        // others, and the record changed after it was made.
        for name in [
            "madeForOtherAccount",
            "madeForOtherServer",
            "madeForOtherUserId",
            "madeForOtherLabel",
            "tamperedCiphertext",
            "tamperedMac",
        ] {
            let mut changed = record.clone();
            changed.wrapped = r[name].as_str().unwrap().to_owned();
            assert!(opens(&changed, pin, &right).is_none(), "{name}");
        }
        // Each made-for-another does open for the one it was made for.
        let mut theirs = record.clone();
        theirs.wrapped = r["madeForOtherAccount"].as_str().unwrap().to_owned();
        assert!(opens(&theirs, pin, &other_account).is_some());
    }

    #[test]
    fn settings_under_the_floor_or_over_the_most_are_refused_before_the_pin_is_used() {
        let v = vectors();
        let w = &v["wrap"];
        let r = &v["refused"];
        for parameters in r["parameters"].as_array().unwrap() {
            let cost = cost(parameters);
            assert!(cost.usable() == Err(UNUSABLE), "{cost:?}");
            assert!(pin_key("2580", &[0; 16], cost).is_err());
        }
        // A record whose wrap is right for its weak settings: only the
        // floor refuses it.
        let weak = record_of(r["weakRecord"].as_str().unwrap());
        assert_eq!(weak.usable(), Err(UNUSABLE));
        assert!(matches!(
            weak.open(w["pin"].as_str().unwrap(), &whom(w)),
            Err(e) if e == UNUSABLE
        ));
        // Another version's record isn't opened either.
        let mut newer = record_of(w["record"].as_str().unwrap());
        newer.version = 2;
        assert_eq!(newer.usable(), Err(Error::NewerApp));
    }

    #[test]
    fn what_a_pin_was_made_for_is_the_vectors() {
        let v = vectors();
        let m = &v["madeFor"];
        let protected = m["protectedUserKey"].as_str().unwrap();
        assert_eq!(made_for(protected), m["madeFor"].as_str().unwrap());
        let kept = Kept::new(protected, b"sealed");
        assert!(kept.made_for(Some(protected)));
        assert!(!kept.made_for(Some(&protected.replace("2.", "2. "))));
        assert!(!kept.made_for(None));
    }

    #[test]
    fn a_pin_has_4_to_128_characters_of_any_kind() {
        for pin in ["", "1", "123", "abc", "\u{e9}\u{e9}\u{e9}"] {
            assert!(check(pin).is_err(), "{pin:?}");
        }
        // Counted in characters, not bytes.
        for pin in [
            "1234",
            "abcd",
            "    ",
            "\u{e9}\u{e9}\u{e9}\u{e9}",
            "p@ss word!",
        ] {
            assert!(check(pin).is_ok(), "{pin:?}");
        }
        assert!(check(&"7".repeat(MAX_CHARACTERS)).is_ok());
        assert!(check(&"7".repeat(MAX_CHARACTERS + 1)).is_err());
        const { assert!(MAX_CHARACTERS >= 64) };
    }

    #[test]
    fn each_setup_has_its_own_salt_and_iv_and_a_zero_byte_is_refused() {
        let user_key = SymmetricKey::generate();
        let whom = For {
            server: "https://vault.example.test",
            user_id: "u",
            account: "a",
        };
        let one = Record::make("2580", &user_key, &whom).unwrap();
        let two = Record::make("2580", &user_key, &whom).unwrap();
        assert_ne!(one.salt, two.salt);
        assert_ne!(one.wrapped, two.wrapped);
        assert_eq!(STANDARD.decode(&one.salt).unwrap().len(), 16);
        assert_eq!(one.cost(), Cost::FLOOR);
        // "a\0b" and "c" must never read as "a" and "b\0c".
        let split = For {
            user_id: "u\0a",
            ..whom
        };
        assert!(Record::make("2580", &user_key, &split).is_err());
    }

    #[test]
    fn the_record_and_the_file_never_hold_the_pin_or_show_the_wrapped_key() {
        let user_key = SymmetricKey::generate();
        let whom = For {
            server: "https://vault.example.test",
            user_id: "u",
            account: "a",
        };
        let pin = "correct-horse-7391";
        let record = Record::make(pin, &user_key, &whom).unwrap();
        assert!(!record.to_json().contains(pin));
        assert_eq!(format!("{record:?}"), "pin::Record(..)");
        let kept = Kept::new("2.a|b|c", record.to_json().as_bytes());
        let file = serde_json::to_string(&kept).unwrap();
        assert!(!file.contains(pin));
    }
}
