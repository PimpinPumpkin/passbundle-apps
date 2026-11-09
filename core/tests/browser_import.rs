//! Importing from the browsers on a computer (meta repo DESIGN.md,
//! "Automatic import"; docs/core.md, "Importing from browsers"), on
//! made-up profiles with known fake passwords in throwaway folders (the
//! browsers crate's fixtures), into the vectors' sample vault, signed in to
//! a stand-in for the server on this computer that keeps what it's sent.
//! Never a real browser profile.
//!
//! The profile rules, a personal profile left alone, consent declined (no
//! profile read), a browser that only its own Export brings in, Firefox's
//! Primary Password (one these tests make up), sorting against the vault,
//! and every value encrypted with the person's keys before it's sent.
#![cfg(not(any(target_os = "android", target_os = "ios")))]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::STANDARD, Engine};
use passbundle_browsers::fixtures::{
    read_under, ChromiumProfile, Fake, FirefoxProfile, Fixture, Sealed, TestKeys,
};
use passbundle_browsers::{Browser, KeyError, Platform};
use passbundle_core::app::App;
use passbundle_core::browser_import::{Outcome, Rule, TurnOff, Why};
use passbundle_core::crypto::{EncString, SymmetricKey};
use passbundle_core::error::Error;
use passbundle_core::secrets::{MemorySecretStore, SecretStore};
use passbundle_core::store::{Settings, Store};
use serde_json::{json, Value};
use zeroize::Zeroizing;

fn vectors() -> Value {
    serde_json::from_str(include_str!("vectors/core-v1.json")).unwrap()
}

/// What the stand-in was sent: each request's method, path, and body.
type Sent = Arc<Mutex<Vec<(String, String, String)>>>;

/// A stand-in for the server: a session for the refresh token, folders and
/// imports kept in its sync (as the server keeps them), and the sync, which
/// starts as the vectors' sample vault with `policies`.
fn stand_in(policies: Value) -> (String, Sent) {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let url = format!("http://{}", server.server_addr().to_ip().unwrap());
    let sent: Sent = Arc::default();
    let log = sent.clone();
    let mut sync = vectors()["sync"].clone();
    sync["policies"] = policies;
    std::thread::spawn(move || {
        let mut made = 0;
        for mut request in server.incoming_requests() {
            let path = request.url().to_owned();
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body).unwrap();
            log.lock()
                .unwrap()
                .push((request.method().to_string(), path.clone(), body.clone()));
            let json = |status: u16, body: String| {
                tiny_http::Response::from_string(body)
                    .with_status_code(status)
                    .with_header(
                        tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap(),
                    )
            };
            let answer = if path == "/identity/connect/token" {
                r#"{"access_token":"a.b.c","expires_in":3600,"refresh_token":"next","token_type":"Bearer"}"#.to_owned()
            } else if path == "/api/folders" {
                made += 1;
                let name: Value = serde_json::from_str(&body).unwrap();
                let folder = json!({
                    "id": format!("f0000000-0000-4000-8000-{made:012}"),
                    "name": name["name"],
                    "object": "folder",
                    "revisionDate": "2026-11-09T10:00:00.000Z",
                });
                sync["folders"].as_array_mut().unwrap().push(folder.clone());
                folder.to_string()
            } else if path == "/api/ciphers/import" {
                let import: Value = serde_json::from_str(&body).unwrap();
                let folder_of: HashMap<u64, u64> = import["folderRelationships"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| (r["key"].as_u64().unwrap(), r["value"].as_u64().unwrap()))
                    .collect();
                for (i, cipher) in import["ciphers"].as_array().unwrap().iter().enumerate() {
                    made += 1;
                    let mut kept = cipher.clone();
                    kept["id"] = format!("c0000000-0000-4000-8000-{made:012}").into();
                    kept["object"] = "cipherDetails".into();
                    kept["edit"] = true.into();
                    kept["viewPassword"] = true.into();
                    kept["revisionDate"] = "2026-11-09T10:00:00.000Z".into();
                    kept["folderId"] = folder_of
                        .get(&(i as u64))
                        .map(|f| import["folders"][*f as usize]["id"].clone())
                        .unwrap_or(Value::Null);
                    sync["ciphers"].as_array_mut().unwrap().push(kept);
                }
                String::new()
            } else if path.starts_with("/api/sync") {
                sync.to_string()
            } else {
                let _ = request.respond(json(404, String::new()));
                continue;
            };
            let _ = request.respond(json(200, answer));
        }
    });
    (url, sent)
}

struct Sample {
    dir: PathBuf,
    password: String,
    secrets: Arc<MemorySecretStore>,
}

impl Sample {
    /// The sample vault kept here, signed in to `server`, with `policies`.
    fn new(server: &str, policies: Value) -> Sample {
        let v = vectors();
        let pre = &v["account"]["pbkdf2"]["prelogin"];
        let dir =
            std::env::temp_dir().join(format!("pb-core-import-{}", uuid::Uuid::new_v4().simple()));
        let email = v["account"]["salt"].as_str().unwrap().to_owned();
        let store = Store::new(&dir);
        store
            .save_settings(&Settings {
                server: server.into(),
                email: email.clone(),
                kdf: 0,
                kdf_iterations: pre["kdfIterations"].as_u64().unwrap() as u32,
                kdf_memory: None,
                kdf_parallelism: None,
                device_identifier: "d".into(),
            })
            .unwrap();
        let mut sync = v["sync"].clone();
        sync["policies"] = policies;
        store.save_sync(&sync.to_string()).unwrap();
        let secrets = Arc::new(MemorySecretStore::default());
        assert!(secrets.keep(server, &email, "kept-refresh-token"));
        Sample {
            dir,
            password: v["account"]["password"].as_str().unwrap().into(),
            secrets,
        }
    }

    fn unlocked(&self) -> App {
        let mut app = App::open(Store::new(&self.dir), Box::new(self.secrets.clone()));
        app.unlock(&self.password).unwrap();
        app
    }
}

impl Drop for Sample {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The sample vault's own email, which these profiles are signed in with.
const EMAIL: &str = "ana.lopez@example.test";

fn user_key() -> SymmetricKey {
    SymmetricKey::from_bytes(
        &STANDARD
            .decode(vectors()["keys"]["userKey"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap()
}

/// A computer with Chrome's default profile (signed in to nothing), its Work
/// profile (signed in with the account's email), and its Home profile
/// (signed in to another account), and Firefox's default profile.
fn computer(platform: Platform) -> Fixture {
    let fixture = Fixture::new(platform);
    fixture.chromium(ChromiumProfile {
        logins: vec![Fake::new(
            "https://intranet.example.test/",
            "ana",
            "default-profile-pass-1",
        )],
        ..ChromiumProfile::new(Browser::Chrome, "Default", "Person 1")
    });
    fixture.chromium(ChromiumProfile {
        email: Some(EMAIL),
        logins: vec![
            // The sample vault's "Example Mail": already there.
            Fake::new(
                "https://mail.example.test/inbox",
                EMAIL,
                "not-a-real-password-1",
            ),
            // Its "Backup service", with another password: to check.
            Fake::new(
                "https://backup.example.test/",
                "backup@example.test",
                "a-newer-backup-pass-2",
            ),
            Fake::new("https://crm.example.test/", "ana", "crm-pass-3")
                .with_note("Shared with sales"),
            Fake::never_save("https://never.example.test/"),
            Fake::new("https://empty.example.test/", "ana", ""),
        ],
        ..ChromiumProfile::new(Browser::Chrome, "Profile 1", "Work")
    });
    fixture.chromium(ChromiumProfile {
        email: Some("ana.home@mail.example.test"),
        logins: vec![Fake::new(
            "https://shop.example.test/",
            "ana.home",
            "personal-shop-pass-4",
        )],
        ..ChromiumProfile::new(Browser::Chrome, "Profile 2", "Home")
    });
    fixture.firefox(FirefoxProfile {
        default: true,
        logins: vec![
            // The same as Chrome's Work profile: imported once.
            Fake::new("https://crm.example.test", "ana", "crm-pass-3"),
            Fake::new("https://wiki.example.test", "ana", "wiki-pass-5"),
        ],
        ..FirefoxProfile::new("abc.default-release", "default-release")
    });
    fixture
}

/// The ids of the profiles the rules chose.
fn chosen_by_rules(app: &App, fixture: &Fixture) -> Vec<String> {
    app.browser_profiles(&fixture.home)
        .unwrap()
        .profiles
        .into_iter()
        .filter(|p| p.choice.chosen)
        .map(|p| p.profile.id)
        .collect()
}

/// Every import request's body, as sent.
fn imports(sent: &Sent) -> Vec<Value> {
    sent.lock()
        .unwrap()
        .iter()
        .filter(|(_, path, _)| path == "/api/ciphers/import")
        .map(|(_, _, body)| serde_json::from_str(body).unwrap())
        .collect()
}

/// Everything sent to the server, as one text.
fn all_sent(sent: &Sent) -> String {
    sent.lock()
        .unwrap()
        .iter()
        .map(|(_, _, body)| body.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

fn open(value: &Value, key: &SymmetricKey) -> String {
    EncString::parse(
        value
            .as_str()
            .unwrap_or_else(|| panic!("not a string: {value}")),
    )
    .unwrap()
    .decrypt_string(key)
    .unwrap()
    .as_str()
    .to_owned()
}

fn no_passwords() -> HashMap<String, Zeroizing<String>> {
    HashMap::new()
}

#[test]
fn the_rules_choose_each_browsers_profile_and_never_a_personal_one() {
    let (url, sent) = stand_in(json!([]));
    let sample = Sample::new(&url, json!([]));
    let app = sample.unlocked();
    for platform in [Platform::Windows, Platform::Mac, Platform::Linux] {
        let fixture = computer(platform);
        let found = app.browser_profiles(&fixture.home).unwrap();
        let listed: Vec<(&str, bool, Option<Rule>, bool)> = found
            .profiles
            .iter()
            .map(|l| {
                (
                    l.profile.name.as_str(),
                    l.choice.chosen,
                    l.choice.rule,
                    l.choice.personal,
                )
            })
            .collect();
        assert_eq!(
            listed,
            [
                ("Person 1", false, None, false),
                ("Home", false, None, true),
                ("Work", true, Some(Rule::YourEmail), false),
                ("default-release", true, Some(Rule::BrowserDefault), false),
            ],
            "{platform:?}"
        );
        assert!(!found.ask);
        // Finding them read no passwords and sent nothing.
        assert!(read_under(&fixture.root).is_empty());
    }
    assert!(sent.lock().unwrap().is_empty());
}

#[test]
fn an_organizations_domain_chooses_a_work_profile_signed_in_with_another_email() {
    let (url, _) = stand_in(json!([]));
    let sample = Sample::new(&url, json!([]));
    // The sample's organization has set its email domain.
    let store = Store::new(&sample.dir);
    let mut sync: Value = serde_json::from_str(&store.sync().unwrap()).unwrap();
    sync["profile"]["organizations"][0]["keykeeperEmailDomains"] = json!(["contoso.example.test"]);
    store.save_sync(&sync.to_string()).unwrap();
    let app = sample.unlocked();
    let fixture = Fixture::new(Platform::this());
    fixture.chromium(ChromiumProfile::new(Browser::Edge, "Default", "Profile 1"));
    fixture.chromium(ChromiumProfile {
        email: Some("ana@contoso.example.test"),
        ..ChromiumProfile::new(Browser::Edge, "Profile 1", "Contoso")
    });
    let found = app.browser_profiles(&fixture.home).unwrap();
    let chosen: Vec<(&str, Option<Rule>)> = found
        .profiles
        .iter()
        .filter(|l| l.choice.chosen)
        .map(|l| (l.profile.name.as_str(), l.choice.rule))
        .collect();
    assert_eq!(chosen, [("Contoso", Some(Rule::WorkDomain))]);
}

#[test]
fn with_no_profile_chosen_by_a_rule_the_person_is_asked() {
    let (url, _) = stand_in(json!([]));
    let sample = Sample::new(&url, json!([]));
    let app = sample.unlocked();
    let fixture = Fixture::new(Platform::this());
    fixture.chromium(ChromiumProfile {
        email: Some("ana.home@mail.example.test"),
        ..ChromiumProfile::new(Browser::Chrome, "Default", "Home")
    });
    fixture.chromium(ChromiumProfile::new(
        Browser::Chrome,
        "Profile 1",
        "Projects",
    ));
    let found = app.browser_profiles(&fixture.home).unwrap();
    assert!(found.ask);
    assert!(found.profiles.iter().all(|l| !l.choice.chosen));
}

#[test]
fn imports_the_chosen_profiles_encrypted_with_the_persons_keys_before_anything_is_sent() {
    for platform in [Platform::Windows, Platform::Mac, Platform::Linux] {
        let (url, sent) = stand_in(json!([]));
        let sample = Sample::new(&url, json!([]));
        let mut app = sample.unlocked();
        let fixture = computer(platform);
        let chosen = chosen_by_rules(&app, &fixture);
        let keys = TestKeys::default();
        let imported = app
            .import_from_browsers(&fixture.home, &keys, &chosen, &no_passwords())
            .unwrap();

        // Chrome's Work profile and Firefox's default, sorted against the vault.
        assert_eq!(
            imported
                .profiles
                .iter()
                .map(|(_, p, o)| (p.as_ref().unwrap().name.as_str(), o))
                .collect::<Vec<_>>(),
            [
                ("Work", &Outcome::Read { logins: 3 }),
                ("default-release", &Outcome::Read { logins: 2 }),
            ],
            "{platform:?}"
        );
        assert_eq!(imported.added, 3, "crm, wiki, and backup to check");
        assert_eq!(imported.to_check, 1);
        assert_eq!(imported.already_there, 2, "Example Mail, and crm twice");
        assert_eq!(
            (imported.empty, imported.never_save, imported.unreadable),
            (1, 1, 0)
        );
        assert_eq!(imported.check_folders, ["Check these: from Google Chrome"]);
        assert!(imported.synced);

        // One request with the three, each value encrypted with the user key.
        let key = user_key();
        let requests = imports(&sent);
        assert_eq!(requests.len(), 1);
        let ciphers = requests[0]["ciphers"].as_array().unwrap();
        let mut opened: Vec<(String, String, String, String)> = ciphers
            .iter()
            .map(|c| {
                assert_eq!(c["type"], 1);
                assert!(c["organizationId"].is_null());
                (
                    open(&c["name"], &key),
                    open(&c["login"]["username"], &key),
                    open(&c["login"]["password"], &key),
                    open(&c["login"]["uris"][0]["uri"], &key),
                )
            })
            .collect();
        opened.sort();
        assert_eq!(
            opened,
            [
                (
                    "backup.example.test".into(),
                    "backup@example.test".into(),
                    "a-newer-backup-pass-2".into(),
                    "https://backup.example.test/".into()
                ),
                (
                    "crm.example.test".into(),
                    "ana".into(),
                    "crm-pass-3".into(),
                    "https://crm.example.test/".into()
                ),
                (
                    "wiki.example.test".into(),
                    "ana".into(),
                    "wiki-pass-5".into(),
                    "https://wiki.example.test".into()
                ),
            ]
        );
        let crm = ciphers
            .iter()
            .find(|c| open(&c["login"]["password"], &key) == "crm-pass-3")
            .unwrap();
        assert_eq!(open(&crm["notes"], &key), "Shared with sales");
        // The one to check is in Chrome's "Check these" folder, made first,
        // its name encrypted too.
        let folders = sent
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, p, _)| m == "POST" && p == "/api/folders")
            .map(|(_, _, b)| serde_json::from_str::<Value>(b).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(folders.len(), 1);
        assert_eq!(
            open(&folders[0]["name"], &key),
            "Check these: from Google Chrome"
        );
        assert_eq!(
            requests[0]["folderRelationships"].as_array().unwrap().len(),
            1
        );
        let to_check = requests[0]["folderRelationships"][0]["key"]
            .as_u64()
            .unwrap() as usize;
        assert_eq!(
            open(&ciphers[to_check]["login"]["password"], &key),
            "a-newer-backup-pass-2"
        );

        // Nothing the profiles hold reached the server as it is.
        let everything = all_sent(&sent);
        for plain in [
            "a-newer-backup-pass-2",
            "crm-pass-3",
            "wiki-pass-5",
            "not-a-real-password-1",
            "backup@example.test",
            "crm.example.test",
            "wiki.example.test",
            "Shared with sales",
            "Check these",
            "personal-shop-pass-4",
            "default-profile-pass-1",
        ] {
            assert!(
                !everything.contains(plain),
                "{plain} was sent as it is ({platform:?})"
            );
        }

        // The vault opens them, after the sync, and in the folder.
        let vault = app.vault().unwrap();
        let crm = vault
            .items
            .iter()
            .find(|i| i.name == "crm.example.test")
            .unwrap();
        assert_eq!(
            app.value(&crm.id, "password").unwrap().as_str(),
            "crm-pass-3"
        );
        let backup = vault
            .items
            .iter()
            .find(|i| i.name == "backup.example.test" && i.folder_id.is_some())
            .unwrap();
        assert_eq!(
            vault.folders[backup.folder_id.as_ref().unwrap()],
            "Check these: from Google Chrome"
        );
    }
}

#[test]
fn a_personal_profile_and_every_profile_not_chosen_are_never_read() {
    let (url, sent) = stand_in(json!([]));
    let sample = Sample::new(&url, json!([]));
    let mut app = sample.unlocked();
    let fixture = computer(Platform::this());
    let found = app.browser_profiles(&fixture.home).unwrap();
    let work = found
        .profiles
        .iter()
        .find(|l| l.profile.name == "Work")
        .unwrap()
        .profile
        .clone();
    let keys = TestKeys::default();
    // The person unchecked Firefox's: only Work. An id that isn't a
    // profile's goes nowhere.
    let chosen = vec![work.id.clone(), "chrome:../../elsewhere".into()];
    let imported = app
        .import_from_browsers(&fixture.home, &keys, &chosen, &no_passwords())
        .unwrap();
    assert_eq!(imported.profiles[1].2, Outcome::Gone);
    let read = read_under(&fixture.root);
    assert_eq!(read.len(), 1);
    assert!(read[0].ends_with("Profile 1"), "{read:?}");
    let everything = all_sent(&sent);
    assert!(!everything.contains("personal-shop-pass-4"));
    assert_eq!(imported.added, 2);
}

#[test]
fn declining_reads_nothing_and_sends_nothing() {
    let (url, sent) = stand_in(json!([]));
    let sample = Sample::new(&url, json!([]));
    let mut app = sample.unlocked();
    let fixture = computer(Platform::this());
    let _ = app.browser_profiles(&fixture.home).unwrap();
    // "Not now" imports nothing; nor does Import with nothing chosen.
    let keys = TestKeys::default();
    let imported = app
        .import_from_browsers(&fixture.home, &keys, &[], &no_passwords())
        .unwrap();
    assert_eq!(imported.added, 0);
    assert!(read_under(&fixture.root).is_empty());
    assert_eq!(keys.asked.get(), 0);
    assert!(imports(&sent).is_empty());
    assert!(!app.browser_import_offered());
}

#[test]
fn a_browser_that_keeps_its_passwords_to_itself_goes_to_its_own_export() {
    let (url, sent) = stand_in(json!([]));
    let sample = Sample::new(&url, json!([]));
    let mut app = sample.unlocked();
    let fixture = Fixture::new(Platform::Windows);
    fixture.chromium(ChromiumProfile {
        email: Some(EMAIL),
        logins: vec![Fake::new(
            "https://protected.example.test/",
            "ana",
            "app-bound-pass-6",
        )],
        sealed: Sealed::AppBound,
        ..ChromiumProfile::new(Browser::Chrome, "Default", "Person 1")
    });
    fixture.chromium(ChromiumProfile {
        logins: vec![Fake::new(
            "https://edge.example.test/",
            "ana",
            "edge-pass-7",
        )],
        ..ChromiumProfile::new(Browser::Edge, "Default", "Profile 1")
    });
    let chosen = chosen_by_rules(&app, &fixture);
    assert_eq!(chosen.len(), 2);
    let keys = TestKeys::default();
    let imported = app
        .import_from_browsers(&fixture.home, &keys, &chosen, &no_passwords())
        .unwrap();
    assert_eq!(imported.profiles[0].2, Outcome::Guided(Why::OnlyTheBrowser));
    assert_eq!(imported.profiles[1].2, Outcome::Read { logins: 1 });
    assert_eq!(imported.added, 1);
    assert!(!all_sent(&sent).contains("app-bound-pass-6"));
    let words = passbundle_core::browser_import::guided_words(Browser::Chrome, Why::OnlyTheBrowser);
    assert_eq!(
        words,
        "Chrome protects its passwords on this computer so that only Chrome can read them, and PassBundle doesn't get around that. Export them from Chrome instead."
    );

    // A Mac whose keychain prompt was answered no.
    let mac = Fixture::new(Platform::Mac);
    mac.chromium(ChromiumProfile {
        email: Some(EMAIL),
        logins: vec![Fake::new(
            "https://a.example.test/",
            "ana",
            "keychain-pass-8",
        )],
        ..ChromiumProfile::new(Browser::Brave, "Default", "Person 1")
    });
    let chosen = chosen_by_rules(&app, &mac);
    let imported = app
        .import_from_browsers(
            &mac.home,
            &TestKeys::refusing(KeyError::Denied),
            &chosen,
            &no_passwords(),
        )
        .unwrap();
    assert_eq!(imported.profiles[0].2, Outcome::Guided(Why::Declined));
    assert_eq!(imported.added, 0);
}

#[test]
fn firefoxs_primary_password_is_asked_for_used_once_and_never_kept() {
    let (url, sent) = stand_in(json!([]));
    let sample = Sample::new(&url, json!([]));
    let mut app = sample.unlocked();
    let fixture = Fixture::new(Platform::this());
    // A Primary Password this test made up.
    let primary = "made-up-primary-password-9";
    fixture.firefox(FirefoxProfile {
        default: true,
        logins: vec![Fake::new(
            "https://bank.example.test",
            "ana",
            "behind-the-primary-10",
        )],
        primary_password: Some(primary),
        ..FirefoxProfile::new("abc.default-release", "default-release")
    });
    let chosen = chosen_by_rules(&app, &fixture);
    let keys = TestKeys::default();
    let first = app
        .import_from_browsers(&fixture.home, &keys, &chosen, &no_passwords())
        .unwrap();
    assert_eq!(first.profiles[0].2, Outcome::NeedsPrimaryPassword);
    assert_eq!(first.added, 0);
    let wrong = HashMap::from([(chosen[0].clone(), Zeroizing::new("not-it".to_owned()))]);
    let second = app
        .import_from_browsers(&fixture.home, &keys, &chosen, &wrong)
        .unwrap();
    assert_eq!(second.profiles[0].2, Outcome::WrongPrimaryPassword);
    assert!(imports(&sent)
        .iter()
        .all(|r| r["ciphers"].as_array().unwrap().is_empty()));
    let right = HashMap::from([(chosen[0].clone(), Zeroizing::new(primary.to_owned()))]);
    let third = app
        .import_from_browsers(&fixture.home, &keys, &chosen, &right)
        .unwrap();
    assert_eq!(third.profiles[0].2, Outcome::Read { logins: 1 });
    assert_eq!(third.added, 1);
    // Never sent, and never kept in the app's folder.
    assert!(!all_sent(&sent).contains(primary));
    assert!(!folder_contains(&sample.dir, primary));
}

fn folder_contains(dir: &Path, text: &str) -> bool {
    std::fs::read_dir(dir).unwrap().flatten().any(|entry| {
        let path = entry.path();
        if path.is_dir() {
            folder_contains(&path, text)
        } else {
            std::fs::read(&path)
                .map(|b| b.windows(text.len()).any(|w| w == text.as_bytes()))
                .unwrap_or(false)
        }
    })
}

#[test]
fn importing_again_brings_in_nothing_twice() {
    let (url, sent) = stand_in(json!([]));
    let sample = Sample::new(&url, json!([]));
    let mut app = sample.unlocked();
    let fixture = computer(Platform::this());
    let chosen = chosen_by_rules(&app, &fixture);
    let keys = TestKeys::default();
    let first = app
        .import_from_browsers(&fixture.home, &keys, &chosen, &no_passwords())
        .unwrap();
    assert_eq!(first.added, 3);
    let again = app
        .import_from_browsers(&fixture.home, &keys, &chosen, &no_passwords())
        .unwrap();
    assert_eq!(again.added, 0);
    assert_eq!(again.already_there, 5);
    // A password changed in Chrome since goes in the "Check these" folder
    // the vault has now, not a new one.
    fixture.chromium(ChromiumProfile {
        email: Some(EMAIL),
        logins: vec![Fake::new(
            "https://crm.example.test/",
            "ana",
            "crm-pass-changed-11",
        )],
        ..ChromiumProfile::new(Browser::Chrome, "Profile 1", "Work")
    });
    let changed = app
        .import_from_browsers(&fixture.home, &keys, &chosen, &no_passwords())
        .unwrap();
    assert_eq!((changed.added, changed.to_check), (1, 1));
    let made: Vec<String> = sent
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, p, _)| p == "/api/folders")
        .map(|(_, _, b)| b.clone())
        .collect();
    assert_eq!(made.len(), 1);
    let last = imports(&sent).pop().unwrap();
    assert_eq!(
        last["folders"][0]["id"],
        "f0000000-0000-4000-8000-000000000001"
    );
    assert_eq!(last["folderRelationships"][0]["value"], 0);
}

#[test]
fn many_logins_go_in_batches() {
    let (url, sent) = stand_in(json!([]));
    let sample = Sample::new(&url, json!([]));
    let mut app = sample.unlocked();
    let fixture = Fixture::new(Platform::Linux);
    fixture.chromium(ChromiumProfile {
        email: Some(EMAIL),
        logins: (0..600)
            .map(|i| {
                Fake::new(
                    &format!("https://site{i}.example.test/"),
                    "ana",
                    &format!("pass-{i}"),
                )
            })
            .collect(),
        ..ChromiumProfile::new(Browser::Chromium, "Default", "Person 1")
    });
    let chosen = chosen_by_rules(&app, &fixture);
    let imported = app
        .import_from_browsers(
            &fixture.home,
            &TestKeys::default(),
            &chosen,
            &no_passwords(),
        )
        .unwrap();
    assert_eq!(imported.added, 600);
    let sizes: Vec<usize> = imports(&sent)
        .iter()
        .map(|r| r["ciphers"].as_array().unwrap().len())
        .collect();
    assert_eq!(sizes, [250, 250, 100]);
    assert_eq!(app.vault().unwrap().items.len(), 7 + 600);
}

#[test]
fn an_organization_that_keeps_items_out_of_personal_stops_it_before_anything_is_read() {
    let policy = json!([{
        "id": "p1",
        "organizationId": "8f9a7c1e-3b2d-4e5f-9a8b-7c6d5e4f3a2b",
        "type": 5,
        "enabled": true,
        "data": null,
    }]);
    let (url, sent) = stand_in(policy.clone());
    let sample = Sample::new(&url, policy);
    // A member, whom it binds (owners and admins it doesn't).
    let store = Store::new(&sample.dir);
    let mut sync: Value = serde_json::from_str(&store.sync().unwrap()).unwrap();
    sync["profile"]["organizations"][0]["type"] = json!(2);
    store.save_sync(&sync.to_string()).unwrap();
    let mut app = sample.unlocked();
    let fixture = computer(Platform::this());
    let found = app.browser_profiles(&fixture.home).unwrap();
    assert_eq!(
        found.organizations.keeps_items_out.as_deref(),
        Some("Contoso")
    );
    let chosen = chosen_by_rules(&app, &fixture);
    let refused = app
        .import_from_browsers(
            &fixture.home,
            &TestKeys::default(),
            &chosen,
            &no_passwords(),
        )
        .unwrap_err();
    assert!(matches!(refused, Error::Refused(_)));
    assert!(
        refused
            .to_string()
            .starts_with("Your organization keeps items out of Personal"),
        "{refused}"
    );
    assert!(read_under(&fixture.root).is_empty());
    assert!(imports(&sent).is_empty());
}

#[test]
fn afterwards_the_profiles_own_password_saving_is_turned_off() {
    let (url, _) = stand_in(json!([]));
    let sample = Sample::new(&url, json!([]));
    let app = sample.unlocked();
    let fixture = computer(Platform::this());
    let chosen = chosen_by_rules(&app, &fixture);
    let results = app.turn_off_browser_saving(&fixture.home, &chosen).unwrap();
    assert_eq!(
        results.iter().map(|(_, _, r)| *r).collect::<Vec<_>>(),
        [TurnOff::Done, TurnOff::Done]
    );
    let found = app.browser_profiles(&fixture.home).unwrap();
    let saving: Vec<(&str, Option<bool>)> = found
        .profiles
        .iter()
        .map(|l| (l.profile.name.as_str(), l.saving_on))
        .collect();
    // Only the chosen ones; the personal profile is left as it was.
    assert_eq!(
        saving,
        [
            ("Person 1", Some(true)),
            ("Home", Some(true)),
            ("Work", Some(false)),
            ("default-release", Some(false)),
        ]
    );
}

#[test]
fn a_locked_or_signed_out_app_reads_nothing() {
    let (url, _) = stand_in(json!([]));
    let sample = Sample::new(&url, json!([]));
    let mut app = sample.unlocked();
    app.lock();
    let fixture = computer(Platform::this());
    assert!(matches!(
        app.browser_profiles(&fixture.home),
        Err(Error::SignedOut)
    ));
    assert!(matches!(
        app.import_from_browsers(
            &fixture.home,
            &TestKeys::default(),
            &["x".into()],
            &no_passwords()
        ),
        Err(Error::SignedOut)
    ));
    assert!(read_under(&fixture.root).is_empty());
}
