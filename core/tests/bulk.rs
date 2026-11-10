//! One thing done to several items at once (docs/core.md, "Selecting
//! several items"): what's done, what's left and why, and what's said when
//! the server takes only part. On the vectors' sample vault, with an
//! organization's items the person may and may not change, signed in to a
//! stand-in for the server on this computer that keeps the vault and
//! changes it as the server's calls do.

use std::sync::{Arc, Mutex};

use passbundle_core::app::{App, Bulk, Outcome, Skip};
use passbundle_core::error::Error;
use passbundle_core::secrets::{MemorySecretStore, SecretStore};
use passbundle_core::store::{Settings, Store};
use passbundle_core::vault::ItemDraft;
use serde_json::{json, Value};

// The sample vault's items (the vectors' sync).
const MAIL: &str = "3d4e5f60-7182-4930-abc1-d2e3f4051627"; // own, in a folder, a favorite
const BACKUP: &str = "4e5f6071-8293-4a41-bcd2-e3f405162738"; // the organization's
const ASKS: &str = "5f607182-93a4-4b52-cde3-f40516273849"; // own, asks for the master password
const NOTE: &str = "60718293-a4b5-4c63-def4-05162738495a"; // own
const CARD: &str = "718293a4-b5c6-4d74-8f01-162738495a6b"; // own
                                                           // Added here: the organization's, in a shared folder the person only views.
const VIEWED: &str = "a0000000-0000-4000-8000-000000000001";
// And one whose passwords the person can't see, in the folder they change.
const HIDDEN: &str = "a0000000-0000-4000-8000-000000000002";

const FOLDER: &str = "1b2c3d4e-5f60-4718-89a0-b1c2d3e4f506";
/// The organization's shared folders: one the person changes items in, a
/// second like it, and one they only view.
const OPEN: &str = "2c3d4e5f-6071-4829-9ab0-c1d2e3f40516";
const SECOND: &str = "c0000000-0000-4000-8000-000000000002";
const VIEW_ONLY: &str = "c0000000-0000-4000-8000-000000000003";

/// The stand-in: the vault it keeps, what it was asked (method, path, and
/// the body sent), and the items it refuses to change.
#[derive(Clone)]
struct StandIn {
    url: String,
    asked: Arc<Mutex<Vec<(String, Value)>>>,
    refuses: Arc<Mutex<Vec<String>>>,
}

impl StandIn {
    /// What it was asked about items and the sync, in order.
    fn asked(&self) -> Vec<String> {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .map(|(call, _)| call.clone())
            .filter(|call| call.contains("/api/ciphers") || call.contains("/api/sync"))
            .collect()
    }

    /// The body of the one call it got at `path`.
    fn sent(&self, path: &str) -> Value {
        let asked = self.asked.lock().unwrap();
        let mut bodies = asked.iter().filter(|(call, _)| call.ends_with(path));
        let body = bodies.next().expect("asked").1.clone();
        assert!(bodies.next().is_none(), "asked once: {path}");
        body
    }

    fn refuse(&self, id: &str) {
        self.refuses.lock().unwrap().push(id.to_owned());
    }
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|id| (*id).to_owned()).collect()
}

fn ids_of(body: &Value) -> Vec<String> {
    sorted(
        body["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap().to_owned())
            .collect(),
    )
}

/// The sample vault, with the organization's items and shared folders
/// these tests are about.
fn vault() -> Value {
    let v: Value = serde_json::from_str(include_str!("vectors/core-v1.json")).unwrap();
    let mut sync = v["sync"].clone();
    let grid = |can: bool| {
        json!({
            "seePasswords": true, "create": can, "edit": can, "delete": can,
            "export": false, "share": can, "print": true, "viewHistory": true,
            "manage": false,
        })
    };
    let open = sync["collections"][0].clone();
    let mut folders = Vec::new();
    for (id, can) in [(OPEN, true), (SECOND, true), (VIEW_ONLY, false)] {
        let mut folder = open.clone();
        folder["id"] = id.into();
        folder["readOnly"] = (!can).into();
        folder["hidePasswords"] = false.into();
        folder["keykeeper"] = grid(can);
        folders.push(folder);
    }
    sync["collections"] = folders.into();
    let ciphers = sync["ciphers"].as_array_mut().unwrap();
    let backup = ciphers.iter().position(|c| c["id"] == BACKUP).unwrap();
    ciphers[backup]["edit"] = true.into();
    ciphers[backup]["viewPassword"] = true.into();
    ciphers[backup]["permissions"] = json!({ "delete": true, "restore": true });
    let mut viewed = ciphers[backup].clone();
    viewed["id"] = VIEWED.into();
    viewed["collectionIds"] = json!([VIEW_ONLY]);
    viewed["edit"] = false.into();
    viewed["permissions"] = json!({ "delete": false, "restore": false });
    let mut hidden = ciphers[backup].clone();
    hidden["id"] = HIDDEN.into();
    hidden["viewPassword"] = false.into();
    ciphers.push(viewed);
    ciphers.push(hidden);
    for c in ciphers.iter_mut() {
        if c["id"] == ASKS {
            c["reprompt"] = 1.into();
        }
    }
    sync
}

/// A stand-in for the server: a session for the refresh token, the vault
/// as a sync, and the calls that change several items or one, done to the
/// vault it keeps as the server does them. An item it's told to refuse
/// gets the server's words; the call for the trash stops there, with the
/// ones before it done, and the call for a folder moves the rest and says
/// not all were moved.
fn stand_in() -> StandIn {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let url = format!("http://{}", server.server_addr().to_ip().unwrap());
    let here = StandIn {
        url,
        asked: Arc::default(),
        refuses: Arc::default(),
    };
    let there = here.clone();
    let mut kept = vault();
    std::thread::spawn(move || {
        for mut request in server.incoming_requests() {
            let path = request.url().to_owned();
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body).unwrap();
            let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            there
                .asked
                .lock()
                .unwrap()
                .push((format!("{} {path}", request.method()), body.clone()));
            let refuses = there.refuses.lock().unwrap().clone();
            let refused = |id: &str| refuses.iter().any(|r| r == id);
            let ciphers = kept["ciphers"].as_array_mut().unwrap();
            let mut change = |id: &str, name: &str, value: Value| {
                for c in ciphers.iter_mut().filter(|c| c["id"] == id) {
                    c[name] = value.clone();
                }
            };
            let no = |words: &str| (400, json!({ "message": words }).to_string());
            let sent_ids: Vec<String> = body["ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|id| id.as_str().map(str::to_owned))
                .collect();
            let one = path
                .strip_prefix("/api/ciphers/")
                .and_then(|rest| rest.split_once('/'));
            let (status, answer) = if path == "/identity/connect/token" {
                let token = r#"{"access_token":"a.b.c","expires_in":3600,"refresh_token":"next","token_type":"Bearer"}"#;
                (200, token.to_owned())
            } else if path.starts_with("/api/sync") {
                (200, kept.to_string())
            } else if path == "/api/ciphers/delete" {
                let mut answer = (200, String::new());
                for id in &sent_ids {
                    if refused(id) {
                        answer = no("Cipher can't be deleted by user");
                        break;
                    }
                    change(id, "deletedDate", "2026-11-09T10:00:00.000Z".into());
                }
                answer
            } else if path == "/api/ciphers/move" {
                for id in sent_ids.iter().filter(|id| !refused(id)) {
                    change(id, "folderId", body["folderId"].clone());
                }
                match sent_ids.iter().any(|id| refused(id)) {
                    true => no("Not all ciphers are moved!"),
                    false => (200, String::new()),
                }
            } else if let Some((id, what)) = one {
                if refused(id) {
                    no("Cipher is not write accessible")
                } else if what == "partial" {
                    change(id, "favorite", body["favorite"].clone());
                    change(id, "folderId", body["folderId"].clone());
                    (200, "{}".to_owned())
                } else if what == "collections_v2" {
                    change(id, "collectionIds", body["collectionIds"].clone());
                    (200, "{}".to_owned())
                } else {
                    (404, String::new())
                }
            } else {
                // The rest a sync asks (settings, health): not this server's.
                (404, String::new())
            };
            let _ = request.respond(
                tiny_http::Response::from_string(answer)
                    .with_status_code(status)
                    .with_header(
                        tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap(),
                    ),
            );
        }
    });
    here
}

struct Sample {
    dir: std::path::PathBuf,
    password: String,
    secrets: Arc<MemorySecretStore>,
}

impl Sample {
    /// The vault kept here, signed in to `server` (a refresh token kept).
    fn new(server: &str) -> Sample {
        let sample = Sample::signed_out(server);
        let email = Store::new(&sample.dir).settings().unwrap().email;
        assert!(sample.secrets.keep(server, &email, "kept-refresh-token"));
        sample
    }

    /// The vault kept here, to read: no session.
    fn signed_out(server: &str) -> Sample {
        let v: Value = serde_json::from_str(include_str!("vectors/core-v1.json")).unwrap();
        let pre = &v["account"]["pbkdf2"]["prelogin"];
        let dir =
            std::env::temp_dir().join(format!("pb-core-bulk-{}", uuid::Uuid::new_v4().simple()));
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
        store.save_sync(&vault().to_string()).unwrap();
        Sample {
            dir,
            password: v["account"]["password"].as_str().unwrap().into(),
            secrets: Arc::new(MemorySecretStore::default()),
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

fn in_trash(app: &App, id: &str) -> bool {
    app.vault().unwrap().item(id).unwrap().deleted
}

fn favorite(app: &App, id: &str) -> bool {
    app.vault().unwrap().item(id).unwrap().favorite
}

fn folder(app: &App, id: &str) -> Option<String> {
    app.vault().unwrap().item(id).unwrap().folder_id.clone()
}

fn shared_folders(app: &App, id: &str) -> Vec<String> {
    app.vault()
        .unwrap()
        .item(id)
        .unwrap()
        .collection_ids
        .clone()
}

const SELECTION: [&str; 6] = [MAIL, BACKUP, ASKS, NOTE, VIEWED, "not-in-the-vault"];

#[test]
fn the_plan_says_which_items_are_left_and_why_without_sending_anything() {
    let server = stand_in();
    let sample = Sample::new(&server.url);
    let app = sample.unlocked();
    let plan = app.bulk_plan(&ids(&SELECTION), &Bulk::Trash).unwrap();
    assert_eq!(plan.go, ids(&[MAIL, BACKUP, NOTE]));
    assert_eq!(
        plan.skipped,
        [
            (ASKS.to_owned(), Skip::MasterPassword),
            (VIEWED.to_owned(), Skip::NoDelete),
            ("not-in-the-vault".to_owned(), Skip::Gone),
        ]
    );
    // An item named twice counts once.
    let twice = app.bulk_plan(&ids(&[NOTE, NOTE]), &Bulk::Trash).unwrap();
    assert_eq!(twice.go, ids(&[NOTE]));
    assert!(server.asked().is_empty(), "{:?}", server.asked());
    assert_eq!(
        plan.json(),
        json!({
            "ok": true,
            "go": [MAIL, BACKUP, NOTE],
            "skipped": [
                { "id": ASKS, "why": "masterPassword" },
                { "id": VIEWED, "why": "noDelete" },
                { "id": "not-in-the-vault", "why": "gone" },
            ],
        })
    );
}

#[test]
fn the_trash_takes_the_allowed_items_in_one_call_and_says_which_were_left() {
    let server = stand_in();
    let sample = Sample::new(&server.url);
    let mut app = sample.unlocked();
    let went = app.bulk(&ids(&SELECTION), &Bulk::Trash, None).unwrap();
    assert_eq!(
        went,
        Outcome {
            done: ids(&[MAIL, BACKUP, NOTE]),
            skipped: vec![
                (ASKS.to_owned(), Skip::MasterPassword),
                (VIEWED.to_owned(), Skip::NoDelete),
                ("not-in-the-vault".to_owned(), Skip::Gone),
            ],
            failed: vec![],
            error: None,
            synced: true,
        }
    );
    // One call for all of them, then one sync. The items left out were
    // never sent.
    assert_eq!(
        server.asked(),
        [
            "PUT /api/ciphers/delete",
            "GET /api/sync?excludeDomains=true"
        ]
    );
    assert_eq!(
        ids_of(&server.sent("/api/ciphers/delete")),
        sorted(ids(&[MAIL, BACKUP, NOTE]))
    );
    for id in [MAIL, BACKUP, NOTE] {
        assert!(in_trash(&app, id), "{id}");
    }
    for id in [ASKS, VIEWED, CARD] {
        assert!(!in_trash(&app, id), "{id}");
    }
    // Asked again, they're in the trash already: nothing to do, nothing sent.
    let again = app.bulk(&ids(&[MAIL, NOTE]), &Bulk::Trash, None).unwrap();
    assert_eq!(again.done, Vec::<String>::new());
    assert_eq!(
        again.skipped,
        [(MAIL.to_owned(), Skip::Gone), (NOTE.to_owned(), Skip::Gone)]
    );
    assert_eq!(server.asked().len(), 2);
}

#[test]
fn the_master_password_given_once_includes_the_items_that_ask_for_it() {
    let server = stand_in();
    let sample = Sample::new(&server.url);
    let mut app = sample.unlocked();
    // A wrong one does nothing at all.
    let wrong = app.bulk(
        &ids(&[ASKS, NOTE]),
        &Bulk::Trash,
        Some("not-the-master-password"),
    );
    assert_eq!(wrong.unwrap_err(), Error::WrongPassword);
    assert!(server.asked().is_empty(), "{:?}", server.asked());
    assert!(!in_trash(&app, NOTE));

    let went = app
        .bulk(
            &ids(&[ASKS, NOTE]),
            &Bulk::Favorite(true),
            Some(&sample.password),
        )
        .unwrap();
    assert_eq!(went.done, ids(&[ASKS, NOTE]));
    assert!(went.skipped.is_empty());
    assert!(favorite(&app, ASKS));
    // Given for the action only: the item's secrets still ask for it.
    let asks = app.vault().unwrap().item(ASKS).unwrap();
    assert!(!app.may_use(asks));
    assert_eq!(
        app.value(ASKS, "password").unwrap_err(),
        Error::MasterPasswordFirst
    );

    // Without it, the item that asks is left, whatever the action.
    for action in [
        Bulk::Favorite(false),
        Bulk::Folder(Some(FOLDER.into())),
        Bulk::Trash,
    ] {
        let left = app.bulk(&ids(&[ASKS]), &action, None).unwrap();
        assert_eq!(
            left.skipped,
            [(ASKS.to_owned(), Skip::MasterPassword)],
            "{action:?}"
        );
        assert!(left.done.is_empty());
    }
    // Given for that item on its own page (the single-item way), it counts.
    app.confirm_master_password(ASKS, &sample.password).unwrap();
    let plan = app.bulk_plan(&ids(&[ASKS]), &Bulk::Trash).unwrap();
    assert_eq!(plan.go, ids(&[ASKS]));
}

#[test]
fn favorites_go_item_by_item_and_one_the_server_refuses_leaves_the_rest_to_try() {
    let server = stand_in();
    let sample = Sample::new(&server.url);
    let mut app = sample.unlocked();
    server.refuse(NOTE);
    // The person's own mark: an item they only view takes it too, and one
    // that's a favorite already needs no call.
    let went = app
        .bulk(
            &ids(&[MAIL, NOTE, VIEWED, CARD]),
            &Bulk::Favorite(true),
            None,
        )
        .unwrap();
    assert_eq!(went.done, ids(&[MAIL, VIEWED, CARD]));
    assert_eq!(went.failed, ids(&[NOTE]));
    assert_eq!(
        went.error,
        Some(Error::Server("Cipher is not write accessible".into()))
    );
    assert!(went.synced);
    assert_eq!(
        server.asked(),
        [
            format!("PUT /api/ciphers/{NOTE}/partial"),
            format!("PUT /api/ciphers/{VIEWED}/partial"),
            format!("PUT /api/ciphers/{CARD}/partial"),
            "GET /api/sync?excludeDomains=true".to_owned(),
        ]
    );
    assert!(favorite(&app, VIEWED) && favorite(&app, CARD) && !favorite(&app, NOTE));
    // Each keeps the folder it was in.
    let sent = server.sent(&format!("/api/ciphers/{CARD}/partial"));
    assert_eq!(sent, json!({ "folderId": null, "favorite": true }));
    let json = went.json();
    assert_eq!(json["failed"], json!([NOTE]));
    assert_eq!(
        json["error"],
        "Your server said: Cipher is not write accessible."
    );
    assert_eq!(json["offline"], false);

    let off = app
        .bulk(&ids(&[MAIL]), &Bulk::Favorite(false), None)
        .unwrap();
    assert_eq!(off.done, ids(&[MAIL]));
    assert_eq!(
        server.sent(&format!("/api/ciphers/{MAIL}/partial")),
        json!({ "folderId": FOLDER, "favorite": false })
    );
    assert!(!favorite(&app, MAIL));
}

#[test]
fn a_folder_takes_the_persons_own_items_and_leaves_the_organizations() {
    let server = stand_in();
    let sample = Sample::new(&server.url);
    let mut app = sample.unlocked();
    let to = Bulk::Folder(Some(FOLDER.into()));
    let went = app
        .bulk(&ids(&[MAIL, BACKUP, NOTE, CARD, VIEWED]), &to, None)
        .unwrap();
    // The one in it already counts as done, with nothing sent for it.
    assert_eq!(went.done, ids(&[MAIL, NOTE, CARD]));
    assert_eq!(
        went.skipped,
        [
            (BACKUP.to_owned(), Skip::Shared),
            (VIEWED.to_owned(), Skip::Shared)
        ]
    );
    assert_eq!(
        server.asked(),
        ["PUT /api/ciphers/move", "GET /api/sync?excludeDomains=true"]
    );
    let sent = server.sent("/api/ciphers/move");
    assert_eq!(sent["folderId"], FOLDER);
    assert_eq!(ids_of(&sent), sorted(ids(&[NOTE, CARD])));
    assert_eq!(folder(&app, NOTE).as_deref(), Some(FOLDER));
    assert_eq!(folder(&app, BACKUP), None);

    // Out of it again, to no folder; the server leaves one where it is and
    // says not all were moved, and the outcome says which.
    server.asked.lock().unwrap().clear();
    server.refuse(CARD);
    let out = app
        .bulk(&ids(&[MAIL, NOTE, CARD]), &Bulk::Folder(None), None)
        .unwrap();
    assert_eq!(server.sent("/api/ciphers/move")["folderId"], Value::Null);
    assert_eq!(out.done, ids(&[MAIL, NOTE]));
    assert_eq!(out.failed, ids(&[CARD]));
    assert_eq!(
        out.error,
        Some(Error::Server("Not all ciphers are moved!".into()))
    );
    assert_eq!(folder(&app, MAIL), None);
    assert_eq!(folder(&app, CARD).as_deref(), Some(FOLDER));

    // A folder that isn't there (deleted on another device) is said, and
    // nothing is sent.
    server.asked.lock().unwrap().clear();
    let gone = app.bulk(&ids(&[NOTE]), &Bulk::Folder(Some("no-such".into())), None);
    assert_eq!(gone.unwrap_err(), Error::Damaged("no such folder"));
    assert!(server.asked().is_empty());
}

#[test]
fn a_move_between_shared_folders_is_checked_as_the_editors_is() {
    let server = stand_in();
    let sample = Sample::new(&server.url);
    let mut app = sample.unlocked();
    let to = Bulk::SharedFolder {
        to: SECOND.into(),
        from: Some(OPEN.into()),
    };
    let selection = ids(&[BACKUP, VIEWED, HIDDEN, NOTE]);
    let plan = app.bulk_plan(&selection, &to).unwrap();
    assert_eq!(plan.go, ids(&[BACKUP]));
    assert_eq!(
        plan.skipped,
        [
            // Only viewed: no share in the folder it would leave.
            (VIEWED.to_owned(), Skip::NoMove),
            (HIDDEN.to_owned(), Skip::HiddenPasswords),
            // The person's own: moving it to an organization is the web vault's.
            (NOTE.to_owned(), Skip::OtherOwner),
        ]
    );
    // The editor's save refuses the same items, for the same reasons: the
    // one check (`may_place`) serves both.
    let draft = ItemDraft {
        name: "Backup service".into(),
        ..ItemDraft::default()
    };
    assert_eq!(
        app.save(Some(VIEWED), &draft, &ids(&[SECOND])).unwrap_err(),
        Error::Refused("your organization doesn't let you change this item's shared folders")
    );
    assert_eq!(
        app.save(Some(HIDDEN), &draft, &ids(&[SECOND])).unwrap_err(),
        Error::Refused("you can't see this item's passwords, so edit it in the web vault")
    );
    assert!(server.asked().is_empty(), "{:?}", server.asked());

    let went = app.bulk(&selection, &to, None).unwrap();
    assert_eq!(went.done, ids(&[BACKUP]));
    assert_eq!(went.skipped, plan.skipped);
    assert_eq!(
        server.asked(),
        [
            format!("PUT /api/ciphers/{BACKUP}/collections_v2"),
            "GET /api/sync?excludeDomains=true".to_owned(),
        ]
    );
    assert_eq!(
        server.sent(&format!("/api/ciphers/{BACKUP}/collections_v2")),
        json!({ "collectionIds": [SECOND] })
    );
    assert_eq!(shared_folders(&app, BACKUP), [SECOND]);

    // Into a folder the person can't add items to: left, and nothing sent.
    // There already: done, and nothing sent.
    server.asked.lock().unwrap().clear();
    let closed = Bulk::SharedFolder {
        to: VIEW_ONLY.into(),
        from: None,
    };
    let left = app.bulk(&ids(&[BACKUP, VIEWED]), &closed, None).unwrap();
    assert_eq!(left.skipped, [(BACKUP.to_owned(), Skip::NoMove)]);
    assert_eq!(left.done, ids(&[VIEWED]));
    assert!(server.asked().is_empty(), "{:?}", server.asked());
    assert_eq!(shared_folders(&app, BACKUP), [SECOND]);
}

#[test]
fn a_trash_the_server_stops_partway_says_which_went_and_which_didnt() {
    let server = stand_in();
    let sample = Sample::new(&server.url);
    let mut app = sample.unlocked();
    server.refuse(NOTE);
    let went = app
        .bulk(&ids(&[MAIL, NOTE, CARD]), &Bulk::Trash, None)
        .unwrap();
    // The server took the first, refused the second, and never got to the
    // third: the vault it sent after says so.
    assert_eq!(went.done, ids(&[MAIL]));
    assert_eq!(went.failed, ids(&[NOTE, CARD]));
    assert_eq!(
        went.error,
        Some(Error::Server("Cipher can't be deleted by user".into()))
    );
    assert!(went.synced);
    assert!(in_trash(&app, MAIL) && !in_trash(&app, NOTE) && !in_trash(&app, CARD));
}

#[test]
fn offline_nothing_is_done_and_it_says_so() {
    // Nothing listening there: this device is offline, as far as it knows.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let sample = Sample::new(&url);
    let mut app = sample.unlocked();
    for action in [
        Bulk::Trash,
        Bulk::Favorite(true),
        Bulk::Folder(Some(FOLDER.into())),
    ] {
        let went = app
            .bulk(&ids(&[NOTE, CARD, VIEWED]), &action, None)
            .unwrap();
        assert!(went.done.is_empty(), "{action:?}");
        assert!(!went.synced);
        assert!(went.error.as_ref().is_some_and(Error::offline), "{went:?}");
        assert_eq!(went.json()["offline"], true);
        let failed = match action {
            Bulk::Favorite(_) => ids(&[NOTE, CARD, VIEWED]),
            _ => ids(&[NOTE, CARD]),
        };
        assert_eq!(went.failed, failed, "{action:?}");
    }
    assert!(!in_trash(&app, NOTE) && !favorite(&app, NOTE));
    // The plan is from the vault kept here, so it still answers.
    let plan = app.bulk_plan(&ids(&[NOTE, VIEWED]), &Bulk::Trash).unwrap();
    assert_eq!(plan.go, ids(&[NOTE]));
}

#[test]
fn signed_out_it_refuses_as_a_single_items_action_does() {
    let sample = Sample::signed_out("https://vault.example.test");
    let mut app = sample.unlocked();
    assert_eq!(app.trash(NOTE).unwrap_err(), Error::SignedOut);
    let refused = app.bulk(&ids(&[NOTE, CARD]), &Bulk::Trash, None);
    assert_eq!(refused.unwrap_err(), Error::SignedOut);
    // Locked, there's no vault to plan from either.
    app.lock();
    let locked = app.bulk_plan(&ids(&[NOTE]), &Bulk::Trash);
    assert_eq!(locked.unwrap_err(), Error::SignedOut);
    assert_eq!(
        app.bulk(&ids(&[NOTE]), &Bulk::Trash, None).unwrap_err(),
        Error::SignedOut
    );
}

#[test]
fn what_to_do_is_read_as_the_app_sends_it() {
    let read = |text: &str| Bulk::from_json(text);
    assert_eq!(
        read(r#"{"do":"favorite","favorite":true}"#),
        Ok(Bulk::Favorite(true))
    );
    assert_eq!(read(r#"{"do":"trash"}"#), Ok(Bulk::Trash));
    assert_eq!(
        read(r#"{"do":"folder","folderId":null}"#),
        Ok(Bulk::Folder(None))
    );
    assert_eq!(
        read(r#"{"do":"folder","folderId":"f"}"#),
        Ok(Bulk::Folder(Some("f".into())))
    );
    assert_eq!(
        read(r#"{"do":"sharedFolder","to":"t","from":null}"#),
        Ok(Bulk::SharedFolder {
            to: "t".into(),
            from: None
        })
    );
    // Anything else does nothing: never a guess at what was meant.
    for unread in [
        "",
        "{}",
        r#"{"do":"delete"}"#,
        r#"{"do":"favorite"}"#,
        r#"{"do":"sharedFolder"}"#,
    ] {
        assert!(read(unread).is_err(), "{unread}");
    }
}
