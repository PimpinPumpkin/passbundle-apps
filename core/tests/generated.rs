//! The generator's Copy (a phone's Generator tab, the computer's Tools): the
//! last password or passphrase it made is what copying `("", "generated")`
//! gives, through the same path as an item's secrets (so phones clear it
//! after 30 seconds), only while the vault is open, and locking forgets it.
//! On the vectors' sample vault kept on this device.

use passbundle_core::app::App;
use passbundle_core::generator::Options;
use passbundle_core::secrets::MemorySecretStore;
use passbundle_core::store::{Settings, Store};

#[test]
fn the_generators_copy_is_its_last_password_until_the_vault_locks() {
    let v: serde_json::Value = serde_json::from_str(include_str!("vectors/core-v1.json")).unwrap();
    let pre = &v["account"]["pbkdf2"]["prelogin"];
    let dir = std::env::temp_dir().join(format!(
        "pb-core-generated-{}",
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
    store.save_sync(&v["sync"].to_string()).unwrap();
    let mut app = App::open(Store::new(&dir), Box::new(MemorySecretStore::default()));

    // Locked: nothing made, and nothing to copy.
    assert!(app.value("", "generated").is_err());
    app.unlock(v["account"]["password"].as_str().unwrap())
        .unwrap();
    assert!(app.value("", "generated").is_err());
    let (first, _) = app.generate(&Options::default()).unwrap();
    let (made, _) = app.generate(&Options::default()).unwrap();
    assert_ne!(first.as_str(), made.as_str());
    assert_eq!(app.value("", "generated").unwrap().as_str(), made.as_str());
    // Not an item's: another "what" for no item is none.
    assert!(app.value("", "password").is_err());

    app.lock();
    assert!(app.value("", "generated").is_err());
    let _ = std::fs::remove_dir_all(dir);
}
