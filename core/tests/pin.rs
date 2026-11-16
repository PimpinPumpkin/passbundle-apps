//! Unlocking with a PIN (DESIGN.md in the meta repo, "Unlock with a PIN"):
//! on the vectors' sample vault kept on this device, with a stand-in for the
//! device's own key (`MemorySecretStore`), never the OS's. Setting a PIN,
//! unlocking with it, the count of wrong tries, everything that removes it,
//! one account's never opening another's, and what a PIN never stands in
//! for. The key and the wrap are checked against the vectors in src/pin.rs.

use std::sync::{Arc, Mutex};

use passbundle_core::app::{App, PinStatus};
use passbundle_core::crypto::SymmetricKey;
use passbundle_core::error::Error;
use passbundle_core::pin::{For, Kept, Record};
use passbundle_core::secrets::{MemorySecretStore, SecretStore};
use passbundle_core::store::{Settings, Store};
use serde_json::{json, Value};

const SERVER: &str = "https://vault.example.test";
const PIN: &str = "2580";
const WRONG: &str = "2581";

struct Sample {
    dir: std::path::PathBuf,
    password: String,
    /// Stands in for this device's store: shared by every `App` opened on
    /// the folder, as the OS's is by every start of the app.
    secrets: Arc<MemorySecretStore>,
    /// The sample's sync, as it's kept.
    sync: Value,
    /// A login that asks for the master password again.
    asks: String,
}

fn settings(v: &Value, server: &str) -> Settings {
    Settings {
        server: server.into(),
        email: v["account"]["salt"].as_str().unwrap().into(),
        kdf: 0,
        kdf_iterations: v["account"]["pbkdf2"]["prelogin"]["kdfIterations"]
            .as_u64()
            .unwrap() as u32,
        kdf_memory: None,
        kdf_parallelism: None,
        device_identifier: "d".into(),
    }
}

/// The sample account's user key, as the vectors give it: what the device's
/// biometric store would hand back.
fn user_key() -> SymmetricKey {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let v: Value = serde_json::from_str(include_str!("vectors/core-v1.json")).unwrap();
    let bytes = STANDARD
        .decode(v["keys"]["userKey"].as_str().unwrap())
        .unwrap();
    SymmetricKey::from_bytes(&bytes).unwrap()
}

fn vectors() -> Value {
    let mut v: Value = serde_json::from_str(include_str!("vectors/core-v1.json")).unwrap();
    let asks = v["ids"]["withKey"].clone();
    for c in v["sync"]["ciphers"].as_array_mut().unwrap() {
        if c["id"] == asks {
            c["reprompt"] = 1.into();
        }
    }
    v
}

impl Sample {
    fn new() -> Sample {
        Sample::at(SERVER)
    }

    /// The sample vault kept here, signed in to `server` (a refresh token
    /// kept), and locked.
    fn at(server: &str) -> Sample {
        let v = vectors();
        let dir =
            std::env::temp_dir().join(format!("pb-core-pin-{}", uuid::Uuid::new_v4().simple()));
        let store = Store::new(&dir);
        let settings = settings(&v, server);
        store.save_settings(&settings).unwrap();
        store.save_sync(&v["sync"].to_string()).unwrap();
        let secrets = Arc::new(MemorySecretStore::default());
        assert!(secrets.keep(server, &settings.email, "kept-refresh-token"));
        Sample {
            dir,
            password: v["account"]["password"].as_str().unwrap().into(),
            secrets,
            sync: v["sync"].clone(),
            asks: v["ids"]["withKey"].as_str().unwrap().into(),
        }
    }

    fn store(&self) -> Store {
        Store::new(&self.dir)
    }

    /// The app as it starts: locked, on this device.
    fn started(&self) -> App {
        App::open(self.store(), Box::new(self.secrets.clone()))
    }

    fn unlocked(&self) -> App {
        let mut app = self.started();
        app.unlock(&self.password).unwrap();
        app
    }

    /// Unlocked with the master password, with `PIN` set.
    fn with_pin(&self) -> App {
        let app = self.unlocked();
        app.set_pin(&self.password, PIN).unwrap();
        assert_eq!(app.pin_status(), PinStatus::On);
        app
    }

    /// The app started again with `PIN` set: locked.
    fn locked_with_pin(&self) -> App {
        drop(self.with_pin());
        self.started()
    }

    fn file(&self) -> Option<Value> {
        let text = std::fs::read_to_string(self.dir.join("pin.json")).ok()?;
        Some(serde_json::from_str(&text).unwrap())
    }

    fn tries(&self) -> u64 {
        self.file().expect("a PIN is kept")["wrongTries"]
            .as_u64()
            .unwrap()
    }

    /// Changes the kept `pin.json` as something with the person's files could.
    fn change_file(&self, change: impl FnOnce(&mut Value)) {
        let mut file = self.file().unwrap();
        change(&mut file);
        std::fs::write(self.dir.join("pin.json"), file.to_string()).unwrap();
    }

    /// Changes the vault kept here, as a sync by another start of the app
    /// would have.
    fn change_kept_sync(&self, change: impl FnOnce(&mut Value)) {
        let mut sync = self.sync.clone();
        change(&mut sync);
        self.store().save_sync(&sync.to_string()).unwrap();
    }
}

impl Drop for Sample {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// An organization the person is a member of, not an owner, with "Remove
/// Unlock with PIN" on.
fn forbids_pins(sync: &mut Value) {
    sync["policies"] = json!([{
        "id": "pin-policy",
        "organizationId": "another-organization",
        "type": 14,
        "enabled": true,
        "data": null,
        "object": "policy",
    }]);
}

/// The account's protected user key as the server has it after a new master
/// password, new KDF settings, or a rotated key: another string.
fn new_protected_key(sync: &mut Value) {
    sync["profile"]["key"] = SymmetricKey::generate()
        .encrypt(&[7u8; 64])
        .to_string()
        .into();
}

fn wrong(left: u32) -> Option<Error> {
    Some(Error::WrongPin { left })
}

#[test]
fn a_pin_set_with_the_master_password_unlocks_the_kept_vault_after_a_restart() {
    let sample = Sample::new();
    let app = sample.unlocked();
    assert_eq!(app.pin_status(), PinStatus::Off);
    assert!(sample.file().is_none());
    app.set_pin(&sample.password, PIN).unwrap();
    assert_eq!(app.pin_status(), PinStatus::On);
    // Setting one isn't unlocking with one.
    assert!(!app.opened_with_pin());
    let items = app.vault().unwrap().items.len();
    drop(app);

    // The app starts again: the PIN is still there, and opens the vault.
    let mut app = sample.started();
    assert_eq!(app.pin_status(), PinStatus::On);
    assert!(app.vault().is_none());
    app.unlock_with_pin(PIN).unwrap();
    assert!(app.opened_with_pin());
    assert_eq!(app.vault().unwrap().items.len(), items);
    assert_eq!(sample.tries(), 0);

    // Locking forgets that a PIN opened it; the master password still works.
    app.lock();
    assert!(!app.opened_with_pin());
    app.unlock(&sample.password).unwrap();
    assert!(!app.opened_with_pin());
    assert_eq!(app.pin_status(), PinStatus::On);
}

#[test]
fn setting_a_pin_needs_the_master_password_an_open_vault_and_4_to_128_characters() {
    let sample = Sample::new();
    // Locked: nothing to wrap.
    assert_eq!(
        sample.started().set_pin(&sample.password, PIN).err(),
        Some(Error::SignedOut)
    );
    let app = sample.unlocked();
    // A wrong master password, the PIN itself given as the master password,
    // and none.
    for not_it in ["not the password", PIN, ""] {
        assert_eq!(
            app.set_pin(not_it, PIN).err(),
            Some(Error::WrongPassword),
            "{not_it:?}"
        );
    }
    // Too short, and too long, are refused before anything is derived.
    for (pin, why) in [
        ("", "A PIN needs at least 4 characters."),
        ("123", "A PIN needs at least 4 characters."),
        (&"1".repeat(129), "A PIN can have at most 128 characters."),
    ] {
        let refused = app.set_pin(&sample.password, pin).unwrap_err();
        assert_eq!(refused.to_string(), why);
    }
    assert!(sample.file().is_none());
    assert_eq!(app.pin_status(), PinStatus::Off);

    // Any characters, counted as characters, and as long as 128.
    for pin in ["1234", "çé🙂!", "a b c d", &"9".repeat(128)] {
        app.set_pin(&sample.password, pin).unwrap();
        let mut again = sample.started();
        again.unlock_with_pin(pin).unwrap();
    }
}

#[test]
fn a_vault_a_pin_opened_sets_a_new_pin_only_with_the_master_password() {
    let sample = Sample::new();
    let mut app = sample.locked_with_pin();
    app.unlock_with_pin(PIN).unwrap();
    // The PIN doesn't pass for the master password.
    assert_eq!(app.set_pin(PIN, "9999").err(), Some(Error::WrongPassword));
    assert_eq!(app.set_pin("", "9999").err(), Some(Error::WrongPassword));
    app.lock();
    assert_eq!(app.unlock_with_pin("9999").err(), wrong(4));

    // With it, the new PIN replaces the old one, and the count starts over.
    app.unlock_with_pin(PIN).unwrap();
    app.lock();
    assert_eq!(app.unlock_with_pin(WRONG).err(), wrong(4));
    app.unlock_with_pin(PIN).unwrap();
    app.lock();
    assert_eq!(app.unlock_with_pin(WRONG).err(), wrong(4));
    assert_eq!(sample.tries(), 1);
    app.unlock(&sample.password).unwrap();
    app.set_pin(&sample.password, "9999").unwrap();
    assert_eq!(sample.tries(), 0);
    app.lock();
    assert_eq!(app.unlock_with_pin(PIN).err(), wrong(4));
    app.unlock_with_pin("9999").unwrap();
}

#[test]
fn a_wrong_pin_says_how_many_tries_are_left_and_the_fifth_turns_the_pin_off() {
    let sample = Sample::new();
    let mut app = sample.locked_with_pin();
    for left in [4, 3, 2, 1] {
        assert_eq!(app.unlock_with_pin(WRONG).err(), wrong(left));
        assert_eq!(sample.tries(), u64::from(5 - left));
        assert!(app.vault().is_none());
        assert_eq!(app.pin_status(), PinStatus::On);
    }
    let fifth = app.unlock_with_pin(WRONG).unwrap_err();
    assert_eq!(fifth, Error::PinTurnedOff);
    assert_eq!(
        fifth.to_string(),
        "Unlocking with a PIN was turned off after 5 wrong tries. Use your master password."
    );
    // The PIN is gone: the right one opens nothing now.
    assert!(sample.file().is_none());
    assert_eq!(app.pin_status(), PinStatus::Off);
    assert_eq!(app.unlock_with_pin(PIN).err(), Some(Error::NoPin));
    assert!(app.vault().is_none());
    // The master password does, and a PIN can be set again.
    app.unlock(&sample.password).unwrap();
    app.set_pin(&sample.password, PIN).unwrap();
    assert_eq!(sample.tries(), 0);
}

#[test]
fn something_that_cant_be_a_pin_isnt_a_try() {
    let sample = Sample::new();
    let mut app = sample.locked_with_pin();
    for not_a_pin in ["", "1", "123", &"1".repeat(129)] {
        assert!(matches!(
            app.unlock_with_pin(not_a_pin),
            Err(Error::Refused(_))
        ));
    }
    assert_eq!(sample.tries(), 0);
    assert_eq!(app.pin_status(), PinStatus::On);
}

#[test]
fn the_count_is_kept_across_a_restart() {
    let sample = Sample::new();
    let mut app = sample.locked_with_pin();
    assert_eq!(app.unlock_with_pin(WRONG).err(), wrong(4));
    assert_eq!(app.unlock_with_pin(WRONG).err(), wrong(3));
    drop(app);

    // The app starts again: the two wrong tries still count.
    let mut app = sample.started();
    assert_eq!(sample.tries(), 2);
    assert_eq!(app.unlock_with_pin(WRONG).err(), wrong(2));
    drop(app);
    let mut app = sample.started();
    assert_eq!(app.unlock_with_pin(WRONG).err(), wrong(1));
    drop(app);
    let mut app = sample.started();
    assert_eq!(app.unlock_with_pin(WRONG).err(), Some(Error::PinTurnedOff));
    assert!(sample.file().is_none());
}

#[test]
fn only_a_right_pin_or_the_master_password_clears_the_count() {
    let sample = Sample::new();
    let mut app = sample.locked_with_pin();

    // A right PIN.
    assert_eq!(app.unlock_with_pin(WRONG).err(), wrong(4));
    assert_eq!(app.unlock_with_pin(WRONG).err(), wrong(3));
    app.unlock_with_pin(PIN).unwrap();
    assert_eq!(sample.tries(), 0);
    app.lock();

    // The master password.
    for left in [4, 3, 2] {
        assert_eq!(app.unlock_with_pin(WRONG).err(), wrong(left));
    }
    assert_eq!(sample.tries(), 3);
    // A wrong master password doesn't.
    assert_eq!(
        app.unlock("not the password").err(),
        Some(Error::WrongPassword)
    );
    assert_eq!(sample.tries(), 3);
    app.unlock(&sample.password).unwrap();
    assert_eq!(sample.tries(), 0);
    app.lock();

    // Biometric unlock (the user key the OS kept) doesn't: the count stays,
    // through locking and starting again.
    for left in [4, 3, 2, 1] {
        assert_eq!(app.unlock_with_pin(WRONG).err(), wrong(left));
    }
    app.unlock_with_key(user_key()).unwrap();
    assert!(!app.opened_with_pin());
    assert_eq!(sample.tries(), 4);
    app.lock();
    drop(app);
    let mut app = sample.started();
    app.unlock_with_key(user_key()).unwrap();
    app.lock();
    assert_eq!(app.unlock_with_pin(WRONG).err(), Some(Error::PinTurnedOff));
}

#[test]
fn the_count_is_read_before_anything_is_derived() {
    let sample = Sample::new();
    let mut app = sample.locked_with_pin();
    // Five wrong tries on record, as a try that never finished would leave
    // them: the right PIN is refused, and the PIN goes.
    sample.change_file(|file| file["wrongTries"] = 5.into());
    assert_eq!(app.unlock_with_pin(PIN).err(), Some(Error::PinTurnedOff));
    assert!(app.vault().is_none());
    assert!(sample.file().is_none());

    // And before the device is asked to open the seal: with a seal that's
    // nothing at all, the answer is still that it was turned off.
    let mut app = sample.locked_with_pin();
    sample.change_file(|file| {
        file["wrongTries"] = 99.into();
        file["sealed"] = "AAAA".into();
    });
    assert_eq!(app.unlock_with_pin(PIN).err(), Some(Error::PinTurnedOff));
    assert!(sample.file().is_none());
}

#[test]
fn a_try_whose_count_cant_be_written_first_is_refused() {
    let sample = Sample::new();
    let mut app = sample.locked_with_pin();
    // The file can't be replaced: where its new copy is written first,
    // there's a folder.
    let in_the_way = sample.dir.join("pin.json.partial");
    std::fs::create_dir(&in_the_way).unwrap();
    std::fs::write(in_the_way.join("kept"), "").unwrap();
    // The right PIN, even: nothing is derived for a try that isn't counted.
    assert!(app.unlock_with_pin(PIN).is_err());
    assert!(app.vault().is_none());
    assert_eq!(sample.tries(), 0);

    std::fs::remove_dir_all(&in_the_way).unwrap();
    app.unlock_with_pin(PIN).unwrap();
}

#[test]
fn turning_it_off_removes_the_pin() {
    let sample = Sample::new();
    let mut app = sample.with_pin();
    app.remove_pin();
    assert!(sample.file().is_none());
    assert_eq!(app.pin_status(), PinStatus::Off);
    app.lock();
    assert_eq!(app.unlock_with_pin(PIN).err(), Some(Error::NoPin));
    assert_eq!(
        Error::NoPin.to_string(),
        "Unlocking with a PIN is off on this device. Use your master password."
    );
    assert!(app.vault().is_none());
}

#[test]
fn signing_out_removes_the_pin() {
    let sample = Sample::new();
    let mut app = sample.with_pin();
    app.sign_out();
    assert!(sample.file().is_none());
    assert_eq!(app.unlock_with_pin(PIN).err(), Some(Error::SignedOut));

    // Signed out while locked, too.
    let sample = Sample::new();
    let mut app = sample.locked_with_pin();
    app.sign_out();
    assert!(sample.file().is_none());
}

/// What the stand-in for the server answers.
#[derive(Clone)]
enum Answers {
    /// The session was ended there: a new master password, new KDF
    /// settings, a rotated key, or "Deauthorize sessions".
    SessionEnded,
    /// A session, and this sync.
    Sync(String),
}

/// A stand-in for the server on this computer, and the paths asked for.
fn stand_in(answers: Arc<Mutex<Answers>>) -> (String, Arc<Mutex<Vec<String>>>) {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let url = format!("http://{}", server.server_addr().to_ip().unwrap());
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = asked.clone();
    std::thread::spawn(move || {
        for request in server.incoming_requests() {
            let path = request.url().to_owned();
            log.lock()
                .unwrap()
                .push(format!("{} {path}", request.method()));
            let json = |status: u16, body: String| {
                tiny_http::Response::from_string(body)
                    .with_status_code(status)
                    .with_header(
                        tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap(),
                    )
            };
            let answers = answers.lock().unwrap().clone();
            let _ = match (path.as_str(), answers) {
                ("/identity/connect/token", Answers::SessionEnded) => {
                    request.respond(json(400, r#"{"error":"invalid_grant"}"#.into()))
                }
                ("/identity/connect/token", Answers::Sync(_)) => request.respond(json(
                    200,
                    r#"{"access_token":"a.b.c","expires_in":3600,"refresh_token":"next","token_type":"Bearer"}"#.into(),
                )),
                (path, Answers::Sync(sync)) if path.starts_with("/api/sync") => {
                    request.respond(json(200, sync))
                }
                _ => request.respond(json(404, String::new())),
            };
        }
    });
    (url, asked)
}

#[test]
fn the_server_ending_the_session_removes_the_pin() {
    let answers = Arc::new(Mutex::new(Answers::SessionEnded));
    let (url, _) = stand_in(answers);
    let sample = Sample::at(&url);
    let mut app = sample.locked_with_pin();
    app.unlock_with_pin(PIN).unwrap();
    assert_eq!(app.sync().err(), Some(Error::SignedOut));
    assert!(sample.file().is_none());
    assert_eq!(app.pin_status(), PinStatus::Off);
    // The kept vault stays open to read, as before.
    assert!(app.vault().is_some());
    app.lock();
    assert_eq!(app.unlock_with_pin(PIN).err(), Some(Error::NoPin));
    // The old master password still opens what's kept here.
    app.unlock(&sample.password).unwrap();
}

#[test]
fn a_sync_keeps_the_pin_unless_the_keys_changed_or_a_policy_forbids_it() {
    let v = vectors();
    let answers = Arc::new(Mutex::new(Answers::Sync(v["sync"].to_string())));
    let (url, _) = stand_in(answers.clone());

    // Nothing about the keys changed: the PIN stays.
    let sample = Sample::at(&url);
    let mut app = sample.with_pin();
    app.sync().unwrap();
    assert_eq!(app.pin_status(), PinStatus::On);
    assert!(sample.file().is_some());

    // The account's protected user key is another one now (the master
    // password, the KDF settings, or the key changed): the PIN goes.
    let mut changed = v["sync"].clone();
    new_protected_key(&mut changed);
    *answers.lock().unwrap() = Answers::Sync(changed.to_string());
    app.sync().unwrap();
    assert!(sample.file().is_none());
    assert_eq!(app.pin_status(), PinStatus::Off);

    // An organization turned "Remove Unlock with PIN" on: the PIN goes, and
    // none can be set.
    *answers.lock().unwrap() = Answers::Sync(v["sync"].to_string());
    let sample = Sample::at(&url);
    let mut app = sample.with_pin();
    let mut forbidden = v["sync"].clone();
    forbids_pins(&mut forbidden);
    *answers.lock().unwrap() = Answers::Sync(forbidden.to_string());
    app.sync().unwrap();
    assert!(sample.file().is_none());
    assert_eq!(app.pin_status(), PinStatus::Policy);
    let refused = app.set_pin(&sample.password, PIN).unwrap_err();
    assert_eq!(
        refused.to_string(),
        "Your organization doesn't allow unlocking with a PIN."
    );
    assert!(sample.file().is_none());
}

#[test]
fn a_policy_in_the_kept_vault_removes_the_pin_at_the_next_unlock() {
    // With the PIN.
    let sample = Sample::new();
    let mut app = sample.locked_with_pin();
    sample.change_kept_sync(forbids_pins);
    assert_eq!(app.pin_status(), PinStatus::Policy);
    let refused = app.unlock_with_pin(PIN).unwrap_err();
    assert_eq!(
        refused.to_string(),
        "Your organization doesn't allow unlocking with a PIN."
    );
    assert!(app.vault().is_none());
    assert!(sample.file().is_none());

    // With the master password.
    let sample = Sample::new();
    let mut app = sample.locked_with_pin();
    sample.change_kept_sync(forbids_pins);
    app.unlock(&sample.password).unwrap();
    assert!(sample.file().is_none());
    assert_eq!(app.pin_status(), PinStatus::Policy);

    // With biometrics.
    let sample = Sample::new();
    drop(sample.with_pin());
    sample.change_kept_sync(forbids_pins);
    let mut app = sample.started();
    app.unlock_with_key(user_key()).unwrap();
    assert!(sample.file().is_none());

    // The organization's owners are exempt, as upstream exempts them.
    let sample = Sample::new();
    let mut app = sample.locked_with_pin();
    let owned = sample.sync["profile"]["organizations"][0]["id"].clone();
    sample.change_kept_sync(|sync| {
        forbids_pins(sync);
        sync["policies"][0]["organizationId"] = owned;
    });
    assert_eq!(app.pin_status(), PinStatus::On);
    app.unlock_with_pin(PIN).unwrap();
}

#[test]
fn a_pin_made_for_other_keys_is_removed_at_the_next_try() {
    let sample = Sample::new();
    let mut app = sample.locked_with_pin();
    // The kept vault's protected user key changed since the PIN was set.
    sample.change_kept_sync(new_protected_key);
    assert_eq!(app.unlock_with_pin(PIN).err(), Some(Error::NoPin));
    assert!(app.vault().is_none());
    assert!(sample.file().is_none());
}

/// Keeps a PIN for a user key that isn't the sample vault's, as a PIN set
/// before the account's key was rotated would be.
fn keep_pin_for(sample: &Sample, user_key: &SymmetricKey) {
    let whom = For {
        server: SERVER,
        user_id: sample.sync["profile"]["id"].as_str().unwrap(),
        account: "",
    };
    let record = Record::make(PIN, user_key, &whom).unwrap();
    let sealed = sample
        .secrets
        .seal_to_device("", record.to_json().as_bytes())
        .unwrap();
    Kept::new(sample.sync["profile"]["key"].as_str().unwrap(), &sealed)
        .write(&sample.store())
        .unwrap();
}

#[test]
fn a_pin_whose_key_no_longer_opens_the_vault_is_removed() {
    let sample = Sample::new();
    keep_pin_for(&sample, &SymmetricKey::generate());
    let mut app = sample.started();
    assert_eq!(app.pin_status(), PinStatus::On);
    assert_eq!(app.unlock_with_pin(PIN).err(), Some(Error::NoPin));
    assert!(app.vault().is_none());
    assert!(sample.file().is_none());

    // The same, kept by hand for the vault's own key, opens: the difference
    // above is the key.
    keep_pin_for(&sample, &user_key());
    let mut app = sample.started();
    app.unlock_with_pin(PIN).unwrap();
}

#[test]
fn a_seal_this_device_no_longer_opens_removes_the_pin_and_isnt_a_wrong_try() {
    // Another device (or this one after its key is gone): a copy of the
    // account's folder, the PIN's file included.
    let sample = Sample::new();
    drop(sample.with_pin());
    let mut elsewhere = App::open(sample.store(), Box::new(MemorySecretStore::default()));
    assert_eq!(elsewhere.unlock_with_pin(PIN).err(), Some(Error::NoPin));
    assert!(elsewhere.vault().is_none());
    assert!(sample.file().is_none());

    // A seal that was changed, and one that isn't base64.
    for seal in ["AAAA", "not base64"] {
        let sample = Sample::new();
        let mut app = sample.locked_with_pin();
        sample.change_file(|file| file["sealed"] = seal.into());
        assert_eq!(app.unlock_with_pin(PIN).err(), Some(Error::NoPin));
        assert!(sample.file().is_none());
    }
}

#[test]
fn a_file_this_version_cant_read_is_no_pin() {
    for change in [
        (|file: &mut Value| file["version"] = 2.into()) as fn(&mut Value),
        |file| *file = json!("not a PIN's file"),
        |file| {
            file.as_object_mut().unwrap().remove("wrongTries");
        },
    ] {
        let sample = Sample::new();
        let mut app = sample.locked_with_pin();
        sample.change_file(change);
        assert_eq!(app.pin_status(), PinStatus::Off);
        assert_eq!(app.unlock_with_pin(PIN).err(), Some(Error::NoPin));
        assert!(sample.file().is_none());
        app.unlock(&sample.password).unwrap();
    }
}

#[test]
fn a_record_with_settings_under_the_floor_is_removed_before_the_pin_is_used() {
    let sample = Sample::new();
    // A record made as cheap to guess as the file's format allows, sealed by
    // this device itself and kept as the account's.
    let whom = For {
        server: SERVER,
        user_id: sample.sync["profile"]["id"].as_str().unwrap(),
        account: "",
    };
    let record = Record::make(PIN, &user_key(), &whom).unwrap();
    let mut weak: Value = serde_json::from_str(&record.to_json()).unwrap();
    weak["memoryMib"] = 8.into();
    let sealed = sample
        .secrets
        .seal_to_device("", weak.to_string().as_bytes())
        .unwrap();
    Kept::new(sample.sync["profile"]["key"].as_str().unwrap(), &sealed)
        .write(&sample.store())
        .unwrap();
    let mut app = sample.started();
    assert_eq!(app.unlock_with_pin(PIN).err(), Some(Error::NoPin));
    assert!(sample.file().is_none());
}

/// Two accounts on one device, as accounts.rs keeps them: each in its own
/// folder under `accounts/`, with the one store for the device.
struct Two {
    dir: std::path::PathBuf,
    secrets: Arc<MemorySecretStore>,
    password: String,
}

const FIRST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa0";
const SECOND: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa1";

impl Two {
    /// The same sample vault twice (so the same user key, the worst case),
    /// on two servers.
    fn new() -> Two {
        let v = vectors();
        let dir =
            std::env::temp_dir().join(format!("pb-core-pin-two-{}", uuid::Uuid::new_v4().simple()));
        for (id, server) in [(FIRST, SERVER), (SECOND, "https://vault.home.example.test")] {
            let store = Store::new(dir.join("accounts").join(id));
            store.save_settings(&settings(&v, server)).unwrap();
            store.save_sync(&v["sync"].to_string()).unwrap();
        }
        Two {
            dir,
            secrets: Arc::new(MemorySecretStore::default()),
            password: v["account"]["password"].as_str().unwrap().into(),
        }
    }

    fn folder(&self, id: &str) -> std::path::PathBuf {
        self.dir.join("accounts").join(id)
    }

    fn started(&self, id: &str) -> App {
        App::open_in(
            id,
            Store::new(self.folder(id)),
            Store::new(&self.dir),
            Box::new(self.secrets.clone()),
        )
    }

    fn file(&self, id: &str) -> Option<Value> {
        let text = std::fs::read_to_string(self.folder(id).join("pin.json")).ok()?;
        Some(serde_json::from_str(&text).unwrap())
    }
}

impl Drop for Two {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn one_accounts_pin_never_opens_or_touches_anothers() {
    let two = Two::new();
    let mut first = two.started(FIRST);
    let mut second = two.started(SECOND);
    first.unlock(&two.password).unwrap();
    first.set_pin(&two.password, PIN).unwrap();
    first.lock();

    // The other account has no PIN, and the first one's doesn't open it.
    assert_eq!(second.pin_status(), PinStatus::Off);
    assert_eq!(second.unlock_with_pin(PIN).err(), Some(Error::NoPin));
    assert!(two.file(SECOND).is_none());

    // The first account's file, copied into the other's folder: this
    // device's seal was made for the first account, and doesn't open as the
    // other's. The copy goes; the first account's own is as it was.
    let kept = two.file(FIRST).unwrap();
    std::fs::copy(
        two.folder(FIRST).join("pin.json"),
        two.folder(SECOND).join("pin.json"),
    )
    .unwrap();
    assert_eq!(second.unlock_with_pin(PIN).err(), Some(Error::NoPin));
    assert!(second.vault().is_none());
    assert!(two.file(SECOND).is_none());
    assert_eq!(two.file(FIRST).unwrap(), kept);

    // Even sealed again by this device for the other account, the first
    // account's record doesn't open there: the wrapping key is made for the
    // account and its server, so the same PIN is a wrong one.
    let sealed = Kept::read(&Store::new(two.folder(FIRST)))
        .unwrap()
        .sealed()
        .unwrap();
    let record = two.secrets.open_from_device(FIRST, &sealed).unwrap();
    assert!(two.secrets.open_from_device(SECOND, &sealed).is_none());
    let resealed = two.secrets.seal_to_device(SECOND, &record).unwrap();
    let protected = vectors()["sync"]["profile"]["key"]
        .as_str()
        .unwrap()
        .to_owned();
    Kept::new(&protected, &resealed)
        .write(&Store::new(two.folder(SECOND)))
        .unwrap();
    assert_eq!(second.unlock_with_pin(PIN).err(), wrong(4));
    assert!(second.vault().is_none());

    // Wrong tries, and the PIN going, are each account's own.
    assert_eq!(two.file(FIRST).unwrap()["wrongTries"], 0);
    for left in [3, 2, 1] {
        assert_eq!(second.unlock_with_pin(PIN).err(), wrong(left));
    }
    assert_eq!(second.unlock_with_pin(PIN).err(), Some(Error::PinTurnedOff));
    assert!(two.file(SECOND).is_none());
    assert_eq!(two.file(FIRST).unwrap(), kept);
    first.unlock_with_pin(PIN).unwrap();

    // Each sets its own; signing one out leaves the other's.
    second.unlock(&two.password).unwrap();
    second.set_pin(&two.password, "4321").unwrap();
    second.lock();
    assert_eq!(first.pin_status(), PinStatus::On);
    first.lock();
    assert_eq!(first.unlock_with_pin("4321").err(), wrong(4));
    assert_eq!(second.unlock_with_pin(PIN).err(), wrong(4));
    first.unlock_with_pin(PIN).unwrap();
    second.unlock_with_pin("4321").unwrap();
    first.sign_out();
    assert!(two.file(FIRST).is_none());
    assert!(two.file(SECOND).is_some());
    second.remove_pin();
    assert!(two.file(SECOND).is_none());
}

#[test]
fn a_device_that_cant_seal_is_offered_no_pin() {
    let sample = Sample::new();
    // A PIN's file from a device that could (a copied folder).
    drop(sample.with_pin());
    let kept = sample.file().unwrap();

    let mut app = App::open(
        sample.store(),
        Box::new(MemorySecretStore::without_device_seal()),
    );
    assert_eq!(app.pin_status(), PinStatus::Unsupported);
    assert_eq!(app.unlock_with_pin(PIN).err(), Some(Error::NoPin));
    app.unlock(&sample.password).unwrap();
    assert_eq!(app.pin_status(), PinStatus::Unsupported);
    let refused = app.set_pin(&sample.password, PIN).unwrap_err();
    assert_eq!(refused.to_string(), "This device can't keep a PIN safely.");
    // Nothing was written, weaker or otherwise.
    assert_eq!(sample.file().unwrap(), kept);

    // An account that's nowhere on this device has none either.
    assert_eq!(App::nobody().pin_status(), PinStatus::Unsupported);
    assert_eq!(
        App::nobody().unlock_with_pin(PIN).err(),
        Some(Error::SignedOut)
    );
}

#[test]
fn a_pin_never_stands_in_for_the_master_password() {
    let sample = Sample::new();
    let mut app = sample.locked_with_pin();
    app.unlock_with_pin(PIN).unwrap();

    // An item that asks for the master password again still asks, and the
    // PIN isn't it.
    let item = app.vault().unwrap().item(&sample.asks).unwrap();
    assert!(!app.may_use(item));
    assert_eq!(
        app.value(&sample.asks, "password").err(),
        Some(Error::MasterPasswordFirst)
    );
    assert_eq!(
        app.confirm_master_password(&sample.asks, PIN).err(),
        Some(Error::WrongPassword)
    );
    assert_eq!(
        app.value(&sample.asks, "password").err(),
        Some(Error::MasterPasswordFirst)
    );
    // Nor where the master password proves it's the person (a passkey's
    // check, approving a sign-in, turning off asking before approvals).
    assert_eq!(
        app.check_master_password(PIN).err(),
        Some(Error::WrongPassword)
    );
    assert_eq!(
        app.confirm_with_master_password(PIN).err(),
        Some(Error::WrongPassword)
    );
    assert!(app.confirm_approvals());
    assert_eq!(
        app.set_confirm_approvals(false).err(),
        Some(Error::ConfirmFirst)
    );
    assert!(app.confirm_approvals());

    // The master password itself still does all of it, in the same vault.
    app.confirm_master_password(&sample.asks, &sample.password)
        .unwrap();
    assert!(app.value(&sample.asks, "password").is_ok());
    app.confirm_with_master_password(&sample.password).unwrap();
    app.set_confirm_approvals(false).unwrap();
}

#[test]
fn biometric_unlock_cant_be_turned_on_in_a_vault_a_pin_opened() {
    let sample = Sample::new();
    let mut app = sample.locked_with_pin();
    app.unlock_with_pin(PIN).unwrap();
    assert!(app.opened_with_pin());
    let refused = app.enroll_biometric().unwrap_err();
    assert_eq!(
        refused.to_string(),
        "Unlock with your master password to turn this on."
    );
    // Setting a PIN again, master password and all, doesn't change what
    // opened the vault.
    app.set_pin(&sample.password, PIN).unwrap();
    assert!(app.opened_with_pin());
    assert!(app.enroll_biometric().is_err_and(|e| e == refused));

    // Unlocked with the master password, it's the agent's to answer again
    // (tests/agent.rs, with the agent's stand-in for the device's prompt).
    app.lock();
    app.unlock(&sample.password).unwrap();
    assert!(!app.opened_with_pin());
}

#[test]
fn the_pin_is_in_no_file_no_error_and_no_debug_output() {
    const SECRET_PIN: &str = "7391-zebra-PIN";
    let sample = Sample::new();
    let app = sample.unlocked();
    app.set_pin(&sample.password, SECRET_PIN).unwrap();
    let user_key = user_key().to_bytes().to_vec();
    drop(app);
    let mut app = sample.started();
    let mut said = Vec::new();
    for pin in ["7391-zebra-PIM", "739", SECRET_PIN] {
        if let Err(e) = app.unlock_with_pin(pin) {
            said.push(format!("{e} {e:?}"));
        }
        app.lock();
    }
    let file = std::fs::read_to_string(sample.dir.join("pin.json")).unwrap();
    said.push(format!(
        "{:?} {:?}",
        Kept::read(&sample.store()).unwrap(),
        Record::make(
            SECRET_PIN,
            &SymmetricKey::generate(),
            &For {
                server: SERVER,
                user_id: "u",
                account: "",
            }
        )
        .unwrap()
    ));
    for text in said.iter().chain([&file]) {
        for part in ["7391", "zebra", "739"] {
            assert!(!text.contains(part), "{text}");
        }
    }
    // Nothing in the account's folder holds the PIN, or the user key as it
    // is or in base64.
    use base64::{engine::general_purpose::STANDARD, Engine};
    let encoded = STANDARD.encode(&user_key);
    let mut files = 0;
    for entry in std::fs::read_dir(&sample.dir).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
        assert!(!has(SECRET_PIN.as_bytes()));
        assert!(!has(&user_key));
        assert!(!has(encoded.as_bytes()));
        files += 1;
    }
    assert!(files >= 3, "settings.json, sync.json, pin.json");
    // The file is the person's alone.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(sample.dir.join("pin.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn the_server_hears_nothing_of_a_pin() {
    let v = vectors();
    let answers = Arc::new(Mutex::new(Answers::Sync(v["sync"].to_string())));
    let (url, asked) = stand_in(answers);
    let sample = Sample::at(&url);
    let mut app = sample.with_pin();
    app.lock();
    assert_eq!(app.unlock_with_pin(WRONG).err(), wrong(4));
    app.unlock_with_pin(PIN).unwrap();
    let _ = app.pin_status();
    app.remove_pin();
    app.set_pin(&sample.password, PIN).unwrap();
    app.lock();
    for _ in 0..5 {
        let _ = app.unlock_with_pin(WRONG);
    }
    assert!(sample.file().is_none());
    // Not one request, for any of it.
    assert_eq!(*asked.lock().unwrap(), Vec::<String>::new());

    // A vault a PIN opened takes up the session as one the master password
    // opened does: the same two requests, and nothing about a PIN in them.
    app.unlock(&sample.password).unwrap();
    app.set_pin(&sample.password, PIN).unwrap();
    app.lock();
    app.unlock_with_pin(PIN).unwrap();
    app.sync().unwrap();
    let asked = asked.lock().unwrap().clone();
    assert_eq!(
        asked[..2],
        [
            "POST /identity/connect/token",
            "GET /api/sync?excludeDomains=true"
        ]
    );
    assert!(asked
        .iter()
        .all(|path| !path.to_lowercase().contains("pin")));
}
