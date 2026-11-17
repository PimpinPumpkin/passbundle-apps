//! A stand-in for the server, for the core's tests of archive, export, and
//! import (tests/archived.rs, tests/transfer.rs): the vectors' sample vault
//! kept on this device and signed in to it, and the server's routes for
//! saving, archiving, history, folders, and imports, keeping what each
//! changes. Each test file uses what it needs of it.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use passbundle_core::app::App;
use passbundle_core::crypto::{EncString, SymmetricKey};
use passbundle_core::secrets::{MemorySecretStore, SecretStore};
use passbundle_core::store::{Settings, Store};
use serde_json::{json, Value};

pub const MAIL: &str = "3d4e5f60-7182-4930-abc1-d2e3f4051627";
pub const OFFICE_WIFI: &str = "4e5f6071-8293-4a41-bcd2-e3f405162738";
pub const BACKUP: &str = "5f607182-93a4-4b52-cde3-f40516273849";
pub const GATE: &str = "60718293-a4b5-4c63-def4-05162738495a";
pub const IT: &str = "2c3d4e5f-6071-4829-9ab0-c1d2e3f40516";
pub const ORG: &str = "8f9a7c1e-3b2d-4e5f-9a8b-7c6d5e4f3a2b";
pub const ARCHIVED_AT: &str = "2026-11-01T10:00:00.123456Z";

/// The vectors' sample vault kept on this device, signed in to `server`
/// with a refresh token kept, with `archived` archived, and the shared
/// folder giving the person `grid` (each capability's yes or no).
pub struct Sample {
    dir: std::path::PathBuf,
    pub password: String,
    secrets: Arc<MemorySecretStore>,
}

pub fn core_vectors() -> Value {
    serde_json::from_str(include_str!("../vectors/core-v1.json")).unwrap()
}

impl Sample {
    pub fn new(server: &str, archived: &[&str], edit_sync: impl FnOnce(&mut Value)) -> Sample {
        let v = core_vectors();
        let pre = &v["account"]["pbkdf2"]["prelogin"];
        let dir = std::env::temp_dir().join(format!(
            "pb-core-transfer-{}",
            uuid::Uuid::new_v4().simple()
        ));
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
        for c in sync["ciphers"].as_array_mut().unwrap() {
            if archived.contains(&c["id"].as_str().unwrap()) {
                c["archivedDate"] = ARCHIVED_AT.into();
            }
            c["edit"] = true.into();
            c["viewPassword"] = true.into();
        }
        sync["collections"][0]["keykeeper"] = json!({
            "seePasswords": true, "create": true, "edit": true, "delete": true, "export": true,
            "share": true, "print": true, "viewHistory": true, "manage": false,
        });
        edit_sync(&mut sync);
        store.save_sync(&sync.to_string()).unwrap();
        let secrets = Arc::new(MemorySecretStore::default());
        assert!(secrets.keep(server, &email, "kept-refresh-token"));
        Sample {
            dir,
            password: v["account"]["password"].as_str().unwrap().into(),
            secrets,
        }
    }

    pub fn unlocked(&self) -> App {
        let mut app = App::open(Store::new(&self.dir), Box::new(self.secrets.clone()));
        app.unlock(&self.password).unwrap();
        app
    }

    pub fn path(&self, name: &str) -> std::path::PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Sample {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// What the stand-in for the server was asked: each method and path, with
/// the body sent.
pub type Asked = Arc<Mutex<Vec<(String, Value)>>>;

/// A stand-in for the server: a session for the refresh token, and the
/// sample vault's sync, changed by what's saved, archived, and imported
/// (each new item given an id). `refuse` paths are answered with the
/// server's sentence.
pub fn stand_in(refuse: Vec<(&'static str, &'static str)>) -> (String, Asked) {
    stand_in_with(refuse, &[])
}

/// `stand_in`, with these items archived, as the server keeps them.
pub fn stand_in_with(
    refuse: Vec<(&'static str, &'static str)>,
    archived: &[&str],
) -> (String, Asked) {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let url = format!("http://{}", server.server_addr().to_ip().unwrap());
    let asked: Asked = Arc::new(Mutex::new(Vec::new()));
    let log = asked.clone();
    let mut sync = core_vectors()["sync"].clone();
    for c in sync["ciphers"].as_array_mut().unwrap() {
        if archived.contains(&c["id"].as_str().unwrap()) {
            c["archivedDate"] = ARCHIVED_AT.into();
        }
        c["edit"] = true.into();
        c["viewPassword"] = true.into();
    }
    sync["collections"][0]["keykeeper"] = json!({
        "seePasswords": true, "create": true, "edit": true, "delete": true, "export": true,
        "share": true, "print": true, "viewHistory": true, "manage": false,
    });
    std::thread::spawn(move || {
        let mut made = 0;
        for mut request in server.incoming_requests() {
            let path = request.url().to_owned();
            let method = request.method().to_string();
            let mut body = String::new();
            let _ = request.as_reader().read_to_string(&mut body);
            let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            log.lock()
                .unwrap()
                .push((format!("{method} {path}"), body.clone()));
            let json = |status: u16, body: String| {
                tiny_http::Response::from_string(body)
                    .with_status_code(status)
                    .with_header(
                        tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap(),
                    )
            };
            if let Some((_, words)) = refuse.iter().find(|(p, _)| path.starts_with(p)) {
                let _ = request.respond(json(400, json!({ "message": words }).to_string()));
                continue;
            }
            let ciphers = sync["ciphers"].as_array_mut().unwrap();
            let answer = if path == "/identity/connect/token" {
                json!({"access_token":"a.b.c","expires_in":3600,"refresh_token":"next","token_type":"Bearer"})
            } else if path.starts_with("/api/sync") {
                sync.clone()
            } else if let Some(id) = path
                .strip_prefix("/api/keykeeper/ciphers/")
                .and_then(|r| r.strip_suffix("/history"))
            {
                // One earlier version: the item as it is.
                let cipher = sync["ciphers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|c| c["id"] == id)
                    .unwrap()
                    .clone();
                json!({ "hidePasswords": false, "data": [{ "id": "version-1", "revisionDate": "2026-11-01T09:00:00.000000Z", "savedBy": null, "cipher": cipher }] })
            } else if method == "POST" && (path == "/api/ciphers" || path == "/api/ciphers/create")
            {
                // A new item, given an id: the person's own, or an
                // organization's in the shared folders named with it.
                let (mut c, collections) = match path.as_str() {
                    "/api/ciphers" => (body.clone(), json!([])),
                    _ => (body["cipher"].clone(), body["collectionIds"].clone()),
                };
                made += 1;
                c["id"] = format!("00000000-0000-4000-9000-{made:012}").into();
                c["collectionIds"] = collections;
                c["edit"] = true.into();
                c["viewPassword"] = true.into();
                ciphers.push(c.clone());
                c
            } else if let Some(rest) = path.strip_prefix("/api/ciphers/") {
                let (id, action) = rest.split_once('/').unwrap_or((rest, ""));
                match (method.as_str(), action) {
                    ("POST", _) if id.starts_with("import") => {
                        for (at, c) in body["ciphers"].as_array().unwrap().iter().enumerate() {
                            let mut c = c.clone();
                            made += 1;
                            c["id"] = format!("00000000-0000-4000-9000-{made:012}").into();
                            c["collectionIds"] = json!([]);
                            // In its folder, as the server puts it.
                            let folder = body["folderRelationships"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .find(|r| r["key"] == at)
                                .map(|r| {
                                    body["folders"][r["value"].as_u64().unwrap() as usize]["id"]
                                        .clone()
                                });
                            c["folderId"] = folder.unwrap_or(Value::Null);
                            ciphers.push(c);
                        }
                        Value::Null
                    }
                    ("PUT", "archive" | "unarchive") => {
                        let c = ciphers.iter_mut().find(|c| c["id"] == id).unwrap();
                        c["archivedDate"] = if action == "archive" {
                            "2026-11-16T09:00:00.000000Z".into()
                        } else {
                            Value::Null
                        };
                        c.clone()
                    }
                    ("PUT", "") => {
                        let c = ciphers.iter_mut().find(|c| c["id"] == id).unwrap();
                        let mut saved = body.clone();
                        saved["id"] = id.into();
                        saved["collectionIds"] = c["collectionIds"].clone();
                        saved["edit"] = true.into();
                        saved["viewPassword"] = true.into();
                        *c = saved.clone();
                        saved
                    }
                    _ => json!({}),
                }
            } else if path == "/api/folders" {
                made += 1;
                let id = format!("f0000000-0000-4000-9000-{made:012}");
                sync["folders"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({ "id": id, "name": body["name"] }));
                json!({ "id": id, "name": body["name"], "object": "folder" })
            } else {
                json!({})
            };
            let _ = request.respond(json(200, answer.to_string()));
        }
    });
    (url, asked)
}

pub fn sent(asked: &Asked, start: &str) -> Vec<Value> {
    asked
        .lock()
        .unwrap()
        .iter()
        .filter(|(p, _)| p.starts_with(start))
        .map(|(_, b)| b.clone())
        .collect()
}

pub fn open(key: &SymmetricKey, value: &Value) -> String {
    EncString::parse(value.as_str().unwrap())
        .unwrap()
        .decrypt_string(key)
        .unwrap()
        .as_str()
        .to_owned()
}

pub fn user_key() -> SymmetricKey {
    let v = core_vectors();
    use base64::Engine;
    SymmetricKey::from_bytes(
        &base64::engine::general_purpose::STANDARD
            .decode(v["keys"]["userKey"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap()
}
