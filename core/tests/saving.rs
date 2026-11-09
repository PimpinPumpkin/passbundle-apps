//! Saving once, whatever the sync after it does (docs/core.md, "Saving"): a
//! new item the server took is saved, even when the sync after it fails, so
//! Save is never pressed again for a second one. On the vectors' sample
//! vault kept on this device, signed in to a stand-in for the server on this
//! computer that takes the save, then fails the sync.

use std::sync::{Arc, Mutex};

use passbundle_core::app::{App, Saved};
use passbundle_core::error::Error;
use passbundle_core::secrets::{MemorySecretStore, SecretStore};
use passbundle_core::store::{Settings, Store};
use passbundle_core::vault::ItemDraft;
use zeroize::Zeroizing;

/// What the stand-in answers a sync with.
#[derive(Clone, Copy)]
enum SyncAnswer {
    /// The server has a problem of its own.
    Unavailable,
    /// The connection drops before an answer.
    Dropped,
}

/// A stand-in for the server: a session for the refresh token, a save that
/// answers with the item as the server keeps it (what was sent, with an id),
/// and a sync that fails as `sync` says. The paths asked for, in order.
fn stand_in(sync: SyncAnswer) -> (String, Arc<Mutex<Vec<String>>>) {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let url = format!("http://{}", server.server_addr().to_ip().unwrap());
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = asked.clone();
    std::thread::spawn(move || {
        for mut request in server.incoming_requests() {
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
            if path == "/identity/connect/token" {
                let token = r#"{"access_token":"a.b.c","expires_in":3600,"refresh_token":"next","token_type":"Bearer"}"#;
                let _ = request.respond(json(200, token.into()));
            } else if path == "/api/ciphers" {
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                let mut saved: serde_json::Value = serde_json::from_str(&body).unwrap();
                saved["id"] = "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d".into();
                saved["object"] = "cipherDetails".into();
                saved["revisionDate"] = "2026-11-07T10:00:00.000Z".into();
                saved["edit"] = true.into();
                saved["viewPassword"] = true.into();
                let _ = request.respond(json(200, saved.to_string()));
            } else {
                match sync {
                    SyncAnswer::Unavailable => {
                        let _ = request.respond(json(503, String::new()));
                    }
                    // Dropped: no answer at all.
                    SyncAnswer::Dropped => drop(request),
                }
            }
        }
    });
    (url, asked)
}

struct Sample {
    dir: std::path::PathBuf,
    password: String,
    secrets: Arc<MemorySecretStore>,
}

impl Sample {
    /// The sample vault kept here, signed in to `server` (a refresh token
    /// kept), and locked.
    fn new(server: &str) -> Sample {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("vectors/core-v1.json")).unwrap();
        let pre = &v["account"]["pbkdf2"]["prelogin"];
        let dir =
            std::env::temp_dir().join(format!("pb-core-saving-{}", uuid::Uuid::new_v4().simple()));
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
        store.save_sync(&v["sync"].to_string()).unwrap();
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

fn draft() -> ItemDraft {
    ItemDraft {
        name: "Saved once".into(),
        username: Some("ana@example.test".into()),
        password: Some(Zeroizing::new("not-a-real-password-7".into())),
        ..ItemDraft::default()
    }
}

fn named(app: &App, name: &str) -> usize {
    app.vault()
        .unwrap()
        .items
        .iter()
        .filter(|i| i.name == name)
        .count()
}

#[test]
fn a_new_item_the_server_took_is_saved_when_the_sync_after_it_fails() {
    for answer in [SyncAnswer::Unavailable, SyncAnswer::Dropped] {
        let (url, asked) = stand_in(answer);
        let sample = Sample::new(&url);
        let mut app = sample.unlocked();
        let before = app.vault().unwrap().items.len();

        // Saved: not a failure, though the sync after it failed.
        let saved = app.save(None, &draft(), &[]).unwrap();
        assert_eq!(
            saved,
            Saved {
                id: "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d".into(),
                synced: false,
            }
        );
        assert_eq!(
            *asked.lock().unwrap(),
            [
                "POST /identity/connect/token",
                "POST /api/ciphers",
                "GET /api/sync?excludeDomains=true",
            ]
        );
        // It shows, opened from what the server saved, with its secrets.
        assert_eq!(app.vault().unwrap().items.len(), before + 1);
        assert_eq!(named(&app, "Saved once"), 1);
        assert_eq!(
            app.value(&saved.id, "password").unwrap().as_str(),
            "not-a-real-password-7"
        );
        // And it's kept here: locked and unlocked again, it's still there.
        app.lock();
        app.unlock(&sample.password).unwrap();
        assert_eq!(named(&app, "Saved once"), 1);
    }
}

#[test]
fn a_save_that_never_reached_the_server_says_so_and_keeps_nothing() {
    // Nothing listening there: this device is offline, as far as it knows.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let sample = Sample::new(&url);
    let mut app = sample.unlocked();
    let before = app.vault().unwrap().items.len();
    let refused = app.save(None, &draft(), &[]).unwrap_err();
    assert!(refused.offline(), "{refused:?}");
    assert!(matches!(refused, Error::Network { .. }));
    assert_eq!(app.vault().unwrap().items.len(), before);
    assert_eq!(named(&app, "Saved once"), 0);
}
