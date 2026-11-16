//! Who may change an item's attachments (docs/core.md, "Attachments"), on
//! the vectors' sample vault kept on this device: the checks the app makes
//! before anything goes to a server. With no server here, a change that
//! passes them stops at the first call to one.

use passbundle_core::app::App;
use passbundle_core::error::Error;
use passbundle_core::secrets::MemorySecretStore;
use passbundle_core::store::{Settings, Store};

struct Sample {
    dir: std::path::PathBuf,
    password: String,
    ids: serde_json::Value,
}

impl Sample {
    /// The sample vault, with one login set to ask for the master password
    /// again, a note in the trash, and whether the organization's item may
    /// be edited (`shared_edit`), each as the server says it.
    fn new(shared_edit: bool) -> Sample {
        let mut v: serde_json::Value =
            serde_json::from_str(include_str!("vectors/core-v1.json")).unwrap();
        let pre = &v["account"]["pbkdf2"]["prelogin"];
        let dir = std::env::temp_dir().join(format!(
            "pb-core-attaching-{}",
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
        let ids = v["ids"].clone();
        for c in v["sync"]["ciphers"].as_array_mut().unwrap() {
            if c["id"] == ids["withKey"] {
                c["reprompt"] = 1.into();
            }
            if c["id"] == ids["note"] {
                c["deletedDate"] = "2026-11-01T10:00:00.000000Z".into();
            }
            if c["id"] == ids["shared"] {
                c["edit"] = shared_edit.into();
                c["viewPassword"] = true.into();
            }
        }
        store.save_sync(&v["sync"].to_string()).unwrap();
        Sample {
            dir,
            password: v["account"]["password"].as_str().unwrap().into(),
            ids,
        }
    }

    fn id(&self, name: &str) -> &str {
        self.ids[name].as_str().unwrap()
    }

    fn unlocked(&self) -> App {
        let mut app = App::open(
            Store::new(&self.dir),
            Box::new(MemorySecretStore::default()),
        );
        app.unlock(&self.password).unwrap();
        app
    }

    /// The id of an item's first attachment.
    fn attachment(&self, app: &App, item: &str) -> String {
        app.vault()
            .unwrap()
            .item(self.id(item))
            .unwrap()
            .attachments[0]
            .id
            .clone()
    }
}

impl Drop for Sample {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const VIEW_ONLY: Error =
    Error::Refused("you can view this item but not change it, so you can't change its attachments");
const IN_TRASH: Error =
    Error::Refused("this item is in the trash. Restore it to change its attachments");

#[test]
fn an_item_the_person_cant_edit_takes_no_file_and_loses_none() {
    let sample = Sample::new(false);
    let mut app = sample.unlocked();
    let shared = sample.id("shared");
    assert_eq!(
        app.attachment_to_add(shared, "codes.txt").err(),
        Some(VIEW_ONLY)
    );
    assert_eq!(
        app.delete_attachment(shared, "anything").err(),
        Some(VIEW_ONLY)
    );
    // Start to finish, the file isn't even read: one that isn't there is
    // refused for the item, not for itself.
    let nowhere = sample.dir.join("not-a-file.txt");
    assert_eq!(app.attach(shared, &nowhere, "").err(), Some(VIEW_ONLY));

    // One they may edit passes, and so do their own.
    let sample = Sample::new(true);
    let app = sample.unlocked();
    assert!(app
        .attachment_to_add(sample.id("shared"), "codes.txt")
        .is_ok());
    assert!(app
        .attachment_to_add(sample.id("personal"), "codes.txt")
        .is_ok());
}

#[test]
fn an_item_in_the_trash_takes_no_file() {
    let sample = Sample::new(true);
    let mut app = sample.unlocked();
    let note = sample.id("note");
    assert_eq!(
        app.attachment_to_add(note, "codes.txt").err(),
        Some(IN_TRASH)
    );
    assert_eq!(
        app.delete_attachment(note, "anything").err(),
        Some(IN_TRASH)
    );
}

#[test]
fn an_item_that_asks_for_the_master_password_waits_for_it() {
    let sample = Sample::new(true);
    let mut app = sample.unlocked();
    let asks = sample.id("withKey");
    let attachment = sample.attachment(&app, "withKey");
    assert_eq!(
        app.attachment_to_add(asks, "codes.txt").err(),
        Some(Error::MasterPasswordFirst)
    );
    assert_eq!(
        app.delete_attachment(asks, &attachment).err(),
        Some(Error::MasterPasswordFirst)
    );

    app.confirm_master_password(asks, &sample.password).unwrap();
    let adding = app.attachment_to_add(asks, "codes.txt").unwrap();
    // Nothing is signed in here, so the server is never told: the checks
    // passed, and that's where it stops.
    assert_eq!(
        app.start_attachment(&adding, 65).err(),
        Some(Error::SignedOut)
    );
    assert_eq!(
        app.delete_attachment(asks, &attachment).err(),
        Some(Error::SignedOut)
    );

    // Locking forgets the master password was given, and the vault.
    app.lock();
    assert_eq!(
        app.start_attachment(&adding, 65).err(),
        Some(Error::SignedOut)
    );
    app.unlock(&sample.password).unwrap();
    assert_eq!(
        app.start_attachment(&adding, 65).err(),
        Some(Error::MasterPasswordFirst)
    );
}

#[test]
fn a_file_needs_a_name_and_an_item_and_an_attachment_must_be_the_items() {
    let sample = Sample::new(true);
    let mut app = sample.unlocked();
    let personal = sample.id("personal");
    assert_eq!(
        app.attachment_to_add(personal, " ").err(),
        Some(Error::Refused("a file needs a name to be attached"))
    );
    assert!(app.attachment_to_add(personal, &"n".repeat(255)).is_ok());
    assert!(matches!(
        app.attachment_to_add(personal, &"n".repeat(256)),
        Err(Error::Refused(why)) if why.contains("255 characters")
    ));
    assert_eq!(
        app.attachment_to_add("not-an-item", "codes.txt").err(),
        Some(Error::Damaged("no such item"))
    );
    // Another item's attachment isn't this item's to delete.
    let other = sample.attachment(&app, "withKey");
    assert_eq!(
        app.delete_attachment(personal, &other).err(),
        Some(Error::Damaged("no such attachment"))
    );

    // A locked vault takes nothing.
    app.lock();
    assert_eq!(
        app.attachment_to_add(personal, "codes.txt").err(),
        Some(Error::SignedOut)
    );
}
