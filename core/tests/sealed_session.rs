//! Where the OS's store doesn't keep refresh tokens (the Mac's: the login
//! keychain asks for the Mac's password again for each build signed without
//! an Apple developer team), the app keeps each account's sealed in its
//! folder (the meta repo's docs/core.md, "The Mac without an Apple developer
//! account"): on the vectors' sample vault, unlocking opens it, the sync
//! after uses it and seals the new one the server gives, and it never
//! reaches the store. One that doesn't open is no session, signing out
//! forgets it, and a new one that can't be written doesn't fail the sync.

use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::STANDARD, Engine};
use passbundle_core::app::App;
use passbundle_core::crypto::SymmetricKey;
use passbundle_core::error::Error;
use passbundle_core::secrets::{
    open_refresh_token, seal_refresh_token, MemorySecretStore, SecretStore,
};
use passbundle_core::store::{Settings, Store};

fn vectors() -> serde_json::Value {
    serde_json::from_str(include_str!("vectors/core-v1.json")).unwrap()
}

fn user_key() -> SymmetricKey {
    let v = vectors();
    SymmetricKey::from_bytes(
        &STANDARD
            .decode(v["keys"]["userKey"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap()
}

/// A stand-in server: the refresh grant answers with a new refresh token,
/// and the sync with the vectors' vault. The refresh tokens it was sent.
fn stand_in() -> (String, Arc<Mutex<Vec<String>>>) {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let url = format!("http://{}", server.server_addr().to_ip().unwrap());
    let sent = Arc::new(Mutex::new(Vec::new()));
    let log = sent.clone();
    let sync = vectors()["sync"].to_string();
    std::thread::spawn(move || {
        for mut request in server.incoming_requests() {
            let json = |status: u16, body: String| {
                tiny_http::Response::from_string(body)
                    .with_status_code(status)
                    .with_header(
                        tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap(),
                    )
            };
            let path = request.url().to_owned();
            if path == "/identity/connect/token" {
                let mut form = String::new();
                let _ = request.as_reader().read_to_string(&mut form);
                let token = form
                    .split('&')
                    .find_map(|pair| pair.strip_prefix("refresh_token="))
                    .unwrap_or_default()
                    .to_owned();
                log.lock().unwrap().push(token);
                let answer = r#"{"access_token":"access","refresh_token":"refresh-2","expires_in":3600,"token_type":"Bearer"}"#;
                let _ = request.respond(json(200, answer.into()));
            } else if path.starts_with("/api/sync") {
                let _ = request.respond(json(200, sync.clone()));
            } else {
                let _ = request.respond(json(404, "{}".into()));
            }
        }
    });
    (url, sent)
}

struct Sample {
    dir: std::path::PathBuf,
    password: String,
    secrets: Arc<MemorySecretStore>,
}

impl Sample {
    fn new(server: &str) -> Sample {
        let v = vectors();
        let pre = &v["account"]["pbkdf2"]["prelogin"];
        let dir = std::env::temp_dir().join(format!(
            "pb-core-sealed-session-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = Store::new(&dir);
        store
            .save_settings(&Settings {
                server: server.into(),
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
            secrets: Arc::new(MemorySecretStore::without_refresh_tokens()),
        }
    }

    fn store(&self) -> Store {
        Store::new(&self.dir)
    }

    fn open(&self) -> App {
        App::open(self.store(), Box::new(self.secrets.clone()))
    }
}

impl Drop for Sample {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn unlocking_opens_the_sealed_token_and_the_sync_seals_the_new_one() {
    let (url, sent) = stand_in();
    let sample = Sample::new(&url);
    sample
        .store()
        .save_sealed_refresh_token(&seal_refresh_token(&user_key(), "refresh-1"))
        .unwrap();

    let mut app = sample.open();
    app.unlock(&sample.password).unwrap();
    app.sync().unwrap();
    assert_eq!(*sent.lock().unwrap(), vec!["refresh-1".to_owned()]);

    // The server's new one, sealed in the account's folder, and never in the store.
    let sealed = sample.store().sealed_refresh_token().unwrap();
    assert!(!sealed.contains("refresh-2"));
    assert_eq!(
        open_refresh_token(&user_key(), &sealed).as_deref(),
        Some(&"refresh-2".to_owned())
    );
    let email = vectors()["account"]["salt"].as_str().unwrap().to_owned();
    assert!(sample.secrets.get(&url, &email).is_none());

    // Locked and unlocked again, the new one is the session.
    app.lock();
    app.unlock(&sample.password).unwrap();
    app.sync().unwrap();
    assert_eq!(sent.lock().unwrap().last().unwrap(), "refresh-2");

    // Signing out forgets it.
    app.sign_out();
    assert!(sample.store().sealed_refresh_token().is_none());
}

#[test]
fn a_sealed_token_that_doesnt_open_is_no_session() {
    let (url, sent) = stand_in();
    let sample = Sample::new(&url);
    // Sealed under another account's key.
    sample
        .store()
        .save_sealed_refresh_token(&seal_refresh_token(&SymmetricKey::generate(), "refresh-1"))
        .unwrap();
    let mut app = sample.open();
    app.unlock(&sample.password).unwrap();
    assert_eq!(app.sync(), Err(Error::SignedOut));
    assert!(sent.lock().unwrap().is_empty());
}

#[test]
fn a_new_token_that_cant_be_written_doesnt_fail_the_sync() {
    let (url, sent) = stand_in();
    let sample = Sample::new(&url);
    sample
        .store()
        .save_sealed_refresh_token(&seal_refresh_token(&user_key(), "refresh-1"))
        .unwrap();
    let mut app = sample.open();
    app.unlock(&sample.password).unwrap();

    // Something in the way of the file: a folder that isn't empty.
    let file = sample.dir.join("session.sealed");
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    std::fs::write(file.join("in-the-way"), "").unwrap();

    // The sync is done all the same, and the session goes on (the core
    // says on standard error that the new token wasn't kept).
    app.sync().unwrap();
    app.sync().unwrap();
    assert_eq!(*sent.lock().unwrap(), vec!["refresh-1".to_owned()]);

    // At worst, the next unlock has no session, and asks to sign in again.
    app.lock();
    app.unlock(&sample.password).unwrap();
    assert_eq!(app.sync(), Err(Error::SignedOut));
}
