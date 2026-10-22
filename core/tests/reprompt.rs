//! Asking for the master password again (docs/core.md): on the vectors'
//! sample vault kept on this device, with one login set to ask, as the web
//! vault sets it. Its secrets don't leave the core until the master password
//! is given for it, and locking forgets that.

use passbundle_core::app::App;
use passbundle_core::error::Error;
use passbundle_core::secrets::MemorySecretStore;
use passbundle_core::store::{Settings, Store};
use passbundle_core::vault::ItemDraft;

struct Sample {
    dir: std::path::PathBuf,
    password: String,
    asks: String,
    other: String,
}

impl Sample {
    fn new() -> Sample {
        let mut v: serde_json::Value =
            serde_json::from_str(include_str!("vectors/core-v1.json")).unwrap();
        let pre = &v["account"]["pbkdf2"]["prelogin"];
        let dir = std::env::temp_dir().join(format!(
            "pb-core-reprompt-{}",
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
        let asks = v["ids"]["withKey"].as_str().unwrap().to_owned();
        let other = v["ids"]["personal"].as_str().unwrap().to_owned();
        for c in v["sync"]["ciphers"].as_array_mut().unwrap() {
            if c["id"] == asks.as_str() {
                c["reprompt"] = 1.into();
            }
        }
        store.save_sync(&v["sync"].to_string()).unwrap();
        Sample {
            dir,
            password: v["account"]["password"].as_str().unwrap().into(),
            asks,
            other,
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

#[test]
fn an_item_that_asks_keeps_its_secrets_until_the_master_password_is_given() {
    let sample = Sample::new();
    let mut app = sample.unlocked();
    let item = app.vault().unwrap().item(&sample.asks).unwrap();
    assert!(!app.may_use(item));
    // Its username isn't a secret; its password, code, notes, and fields are.
    assert_eq!(
        app.value(&sample.asks, "username").unwrap().as_str(),
        "backup@example.test"
    );
    for what in ["password", "totp", "notes", "field:0"] {
        assert_eq!(
            app.value(&sample.asks, what).err(),
            Some(Error::MasterPasswordFirst),
            "{what}"
        );
    }
    // Items that don't ask are as before.
    assert_eq!(
        app.value(&sample.other, "password").unwrap().as_str(),
        "not-a-real-password-1"
    );

    // A wrong password says so and lets nothing out.
    assert_eq!(
        app.confirm_master_password(&sample.asks, "not the password")
            .err(),
        Some(Error::WrongPassword)
    );
    assert!(app.value(&sample.asks, "password").is_err());

    app.confirm_master_password(&sample.asks, &sample.password)
        .unwrap();
    assert_eq!(
        app.value(&sample.asks, "password").unwrap().as_str(),
        "not-a-real-password-3"
    );
    assert_eq!(
        app.value(&sample.asks, "field:0").unwrap().as_str(),
        "k-123"
    );

    // Locking forgets it.
    app.lock();
    app.unlock(&sample.password).unwrap();
    assert_eq!(
        app.value(&sample.asks, "password").err(),
        Some(Error::MasterPasswordFirst)
    );
}

#[test]
fn it_lasts_until_another_item_asks_and_an_edit_waits_for_it() {
    let sample = Sample::new();
    let mut app = sample.unlocked();
    // Saving an edit before the master password is given is refused before
    // anything goes to the server: the editor's fields would be empty.
    let draft = ItemDraft::of(app.vault().unwrap().item(&sample.asks).unwrap());
    assert_eq!(
        app.save(Some(&sample.asks), &draft, &[]).err(),
        Some(Error::MasterPasswordFirst)
    );

    app.confirm_master_password(&sample.asks, &sample.password)
        .unwrap();
    assert!(app.value(&sample.asks, "password").is_ok());
    // Giving it for another item moves it there.
    app.confirm_master_password(&sample.other, &sample.password)
        .unwrap();
    assert_eq!(
        app.value(&sample.asks, "password").err(),
        Some(Error::MasterPasswordFirst)
    );
    // No such item: nothing to give it for.
    assert!(app
        .confirm_master_password("no-such-item", &sample.password)
        .is_err());
}
