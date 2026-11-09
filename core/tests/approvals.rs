//! Confirming it's the person, for approving a sign-in from another device
//! (docs/core.md, "Approving a sign-in"), on the vectors' sample vault kept on
//! this device: turning off "Confirm before approving" needs a confirmation
//! made just now, and uses it up; only the master password, or the vault's
//! own user key given back for this (a phone's or the agent's biometric
//! prompt), confirms; unlocking with that key again doesn't; and locking
//! forgets it. Approving itself is tested against a real server
//! (tests/server.rs).

use base64::{engine::general_purpose::STANDARD, Engine};
use passbundle_core::app::App;
use passbundle_core::crypto::SymmetricKey;
use passbundle_core::error::Error;
use passbundle_core::secrets::MemorySecretStore;
use passbundle_core::store::{Settings, Store};

struct Sample {
    dir: std::path::PathBuf,
    password: String,
    user_key: Vec<u8>,
}

impl Sample {
    fn new() -> Sample {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("vectors/core-v1.json")).unwrap();
        let pre = &v["account"]["pbkdf2"]["prelogin"];
        let dir = std::env::temp_dir().join(format!(
            "pb-core-approvals-{}",
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
        Sample {
            dir,
            password: v["account"]["password"].as_str().unwrap().into(),
            user_key: STANDARD
                .decode(v["keys"]["userKey"].as_str().unwrap())
                .unwrap(),
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

    fn key(&self) -> SymmetricKey {
        SymmetricKey::from_bytes(&self.user_key).unwrap()
    }
}

impl Drop for Sample {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn turning_off_confirming_needs_a_confirmation_made_just_now_and_uses_it_up() {
    let sample = Sample::new();
    let mut app = sample.unlocked();
    assert!(app.confirm_approvals());

    // Not without confirming, and unlocking just now isn't confirming.
    assert_eq!(app.set_confirm_approvals(false), Err(Error::ConfirmFirst));
    assert!(app.confirm_approvals());

    // A wrong master password confirms nothing.
    assert_eq!(
        app.confirm_with_master_password("not the password"),
        Err(Error::WrongPassword)
    );
    assert_eq!(app.set_confirm_approvals(false), Err(Error::ConfirmFirst));

    // The right one does, once.
    app.confirm_with_master_password(&sample.password).unwrap();
    app.set_confirm_approvals(false).unwrap();
    assert!(!app.confirm_approvals());
    // Turning it on needs nothing, and off again needs another.
    app.set_confirm_approvals(true).unwrap();
    assert_eq!(app.set_confirm_approvals(false), Err(Error::ConfirmFirst));

    // The vault's own key, given back for this, confirms; another key
    // doesn't (that unlock is out of date).
    assert!(matches!(
        app.confirm_approval_with_key(SymmetricKey::generate()),
        Err(Error::Refused(_))
    ));
    assert_eq!(app.set_confirm_approvals(false), Err(Error::ConfirmFirst));
    app.confirm_approval_with_key(sample.key()).unwrap();
    app.set_confirm_approvals(false).unwrap();
    app.set_confirm_approvals(true).unwrap();
}

#[test]
fn only_the_approval_prompt_confirms_and_locking_forgets_it() {
    let sample = Sample::new();
    let mut app = sample.unlocked();

    // Unlocking with the key (as a passkey's check that it's the person
    // does on a phone) isn't confirming an approval.
    app.unlock_with_key(sample.key()).unwrap();
    assert_eq!(app.set_confirm_approvals(false), Err(Error::ConfirmFirst));

    // A confirmation doesn't outlive locking.
    app.confirm_with_master_password(&sample.password).unwrap();
    app.lock();
    app.unlock(&sample.password).unwrap();
    assert_eq!(app.set_confirm_approvals(false), Err(Error::ConfirmFirst));

    // Locked, nothing confirms.
    app.lock();
    assert_eq!(
        app.confirm_approval_with_key(sample.key()),
        Err(Error::SignedOut)
    );
}
