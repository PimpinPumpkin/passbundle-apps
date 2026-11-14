//! Archived items (archived in the web vault, which sends `archivedDate`):
//! kept in the vault as the server sent them, but out of the way, as the web
//! vault and the extension have them. Not listed or found by search, not
//! judged by Health, not reminded about, and not offered for filling or as
//! passkeys. On the vectors' sample vault kept on this device, with items
//! archived as the web vault archives them.

use std::collections::BTreeMap;

use passbundle_core::app::App;
use passbundle_core::error::Error;
use passbundle_core::expiration::{self, Date};
use passbundle_core::matching::Target;
use passbundle_core::secrets::MemorySecretStore;
use passbundle_core::store::{Settings, Store};

const MAIL: &str = "3d4e5f60-7182-4930-abc1-d2e3f4051627";
/// A login with a website, a username, and a passkey for backup.example.test.
const BACKUP: &str = "5f607182-93a4-4b52-cde3-f40516273849";
const GATE: &str = "60718293-a4b5-4c63-def4-05162738495a";
/// A card that expires in July 2029.
const CARD: &str = "718293a4-b5c6-4d74-8f01-162738495a6b";

const ARCHIVED_AT: &str = "2026-11-01T10:00:00.000Z";

struct Sample {
    dir: std::path::PathBuf,
    password: String,
}

impl Sample {
    /// The sample vault kept on this device, with `archived` archived.
    fn new(archived: &[&str]) -> Sample {
        let mut v: serde_json::Value =
            serde_json::from_str(include_str!("vectors/core-v1.json")).unwrap();
        let pre = &v["account"]["pbkdf2"]["prelogin"];
        let dir = std::env::temp_dir().join(format!(
            "pb-core-archived-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = Store::new(&dir);
        store
            .save_settings(&Settings {
                server: "https://vault.example.test".into(),
                email: v["account"]["salt"].as_str().unwrap().into(),
                kdf: 0,
                kdf_iterations: pre["kdfIterations"].as_u64().unwrap() as u32,
                kdf_memory: None,
                kdf_parallelism: None,
                device_identifier: "d".into(),
            })
            .unwrap();
        for c in v["sync"]["ciphers"].as_array_mut().unwrap() {
            if archived.contains(&c["id"].as_str().unwrap()) {
                c["archivedDate"] = ARCHIVED_AT.into();
            }
        }
        store.save_sync(&v["sync"].to_string()).unwrap();
        Sample {
            dir,
            password: v["account"]["password"].as_str().unwrap().into(),
        }
    }

    fn unlocked(&self) -> App {
        let mut app = App::open(
            Store::new(&self.dir),
            Box::new(MemorySecretStore::default()),
        );
        app.unlock(&self.password).unwrap();
        app
    }
}

impl Drop for Sample {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn ids<'a>(items: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut ids: Vec<String> = items.into_iter().map(str::to_owned).collect();
    ids.sort();
    ids
}

#[test]
fn archived_items_stay_in_the_vault_but_arent_listed_or_found() {
    let sample = Sample::new(&[BACKUP, GATE]);
    let app = sample.unlocked();
    let vault = app.vault().unwrap();
    // Kept, as the server sent them: the core still has every item.
    assert_eq!(vault.items.len(), 7);
    let backup = vault.item(BACKUP).unwrap();
    assert!(backup.archived && !backup.deleted && !backup.in_use());
    assert!(vault.item(MAIL).unwrap().in_use());
    // The lists, their counts, and search: every item but those two.
    let listed = ids(vault.search("").into_iter().map(|i| i.id.as_str()));
    assert_eq!(listed.len(), 5);
    assert!(!listed.contains(&BACKUP.to_owned()));
    assert!(!listed.contains(&GATE.to_owned()));
    assert!(listed.contains(&MAIL.to_owned()));
    assert!(vault.search("backup").is_empty());
    assert!(vault.search("gate").is_empty());
    assert_eq!(vault.search("example mail").len(), 1);

    // Not archived, they're listed and found as ever.
    let before = Sample::new(&[]);
    let app = before.unlocked();
    let vault = app.vault().unwrap();
    assert_eq!(vault.search("").len(), 7);
    assert_eq!(vault.search("backup").len(), 1);
    assert_eq!(vault.search("gate").len(), 1);
}

#[test]
fn an_archived_login_isnt_offered_for_filling_or_as_a_passkey() {
    let site = Target::Website("backup.example.test");
    let archived = Sample::new(&[BACKUP]);
    let before = Sample::new(&[]);
    let (mut app, mut was) = (archived.unlocked(), before.unlocked());

    // Autofill: Android's service and iOS's AutoFill extension.
    let filled = |app: &App| -> Vec<String> {
        ids(app
            .vault()
            .unwrap()
            .matching(&site)
            .into_iter()
            .map(|i| i.id.as_str()))
    };
    // Both logins are on example.test, so each fills on the other's site.
    assert_eq!(filled(&was), ids([MAIL, BACKUP]));
    assert_eq!(filled(&app), vec![MAIL.to_owned()]);

    // What iOS suggests above the keyboard, and in its passkey sheet.
    let suggested = |app: &App| -> Vec<String> {
        ids(app.vault().unwrap().suggestions().into_iter().map(|s| s.id))
    };
    assert!(suggested(&was).contains(&BACKUP.to_owned()));
    assert!(!suggested(&app).contains(&BACKUP.to_owned()));
    assert!(suggested(&app).contains(&MAIL.to_owned()));
    assert_eq!(was.vault().unwrap().passkey_suggestions().len(), 1);
    assert!(app.vault().unwrap().passkey_suggestions().is_empty());

    // A phone's list of passkeys for the site, and signing with it.
    assert_eq!(was.passkeys_for("backup.example.test").len(), 1);
    assert!(app.passkeys_for("backup.example.test").is_empty());
    let hash = [7u8; 32];
    assert!(was
        .passkey_assert(BACKUP, "backup.example.test", &hash, false)
        .is_ok());
    assert_eq!(
        app.passkey_assert(BACKUP, "backup.example.test", &hash, false)
            .err(),
        Some(Error::Refused("that login is archived"))
    );
}

#[test]
fn health_and_expiration_reminders_leave_archived_items_out() {
    let archived = Sample::new(&[BACKUP, CARD]);
    let before = Sample::new(&[]);
    let (app, was) = (archived.unlocked(), before.unlocked());

    // Health judges the logins in use only.
    let judged = |app: &App| -> Vec<String> {
        ids(passbundle_core::health::logins(app.vault().unwrap())
            .iter()
            .map(|l| l.id.as_str()))
    };
    assert!(judged(&was).contains(&BACKUP.to_owned()));
    assert!(!judged(&app).contains(&BACKUP.to_owned()));
    assert!(judged(&app).contains(&MAIL.to_owned()));

    // The card expires in July 2029: "Expiring soon", Health, and the
    // reminders (the tray's count too) have it only while it's in use.
    let today = Date::new(2029, 7, 20).unwrap();
    let expiring = |app: &App| -> Vec<String> {
        ids(expiration::expiring(app.vault().unwrap(), today)
            .into_iter()
            .map(|(i, _)| i.id.as_str()))
    };
    assert_eq!(expiring(&was), vec![CARD.to_owned()]);
    assert!(expiring(&app).is_empty());
    let check = |app: &App| expiration::check(app.vault().unwrap(), &mut BTreeMap::new(), today);
    assert_eq!(check(&was).standing.total(), 1);
    assert_eq!(check(&was).due.total(), 1);
    assert_eq!(check(&app).standing.total(), 0);
    assert_eq!(check(&app).due.total(), 0);
}
