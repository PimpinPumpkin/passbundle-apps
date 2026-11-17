//! An organization's "Remove card item type" policy (upstream's type 15;
//! src/restricted_types.rs for the rule itself): on the vectors' sample
//! vault with a second organization, Fabrikam, and a card in each
//! organization beside the person's own, against a stand-in for the server
//! whose sync the test changes. The cards it removes leave the list, search,
//! items by id, reminders, and the export; no new card is made, none is
//! imported, and nothing is done to one it hides; and each sync brings the
//! policy as it now is, without opening the app again.

use std::sync::{Arc, Mutex};

use passbundle_core::app::{App, ImportRead};
use passbundle_core::crypto::{encrypt_for, SymmetricKey};
use passbundle_core::error::Error;
use passbundle_core::expiration::{self, Date};
use passbundle_core::export;
use passbundle_core::import::{Format, LeftOut};
use passbundle_core::templates::{self, Base};
use passbundle_core::vault::{Card, ItemDraft, ItemKind};
use serde_json::{json, Value};

#[path = "support/stand_in.rs"]
mod stand_in;
use stand_in::{core_vectors, Sample, GATE, MAIL, ORG};

/// The person's own card ("Everyday card", which expires in July 2029).
const CARD: &str = "718293a4-b5c6-4d74-8f01-162738495a6b";
const IDENTITY: &str = "8293a4b5-c6d7-4e85-9012-2738495a6b7c";
/// Contoso's card, and the second organization with its own.
const CONTOSO_CARD: &str = "a1b2c3d4-0000-4000-8000-000000000001";
const FABRIKAM: &str = "f0f1f2f3-f4f5-4f6f-8f9f-a0a1a2a3a4a5";
const FABRIKAM_CARD: &str = "a1b2c3d4-0000-4000-8000-000000000002";

fn b64(text: &str) -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .unwrap()
}

/// A card in an organization, encrypted with its key.
fn card(id: &str, org: &str, key: &SymmetricKey, name: &str) -> Value {
    let enc = |text: &str| key.encrypt_str(text).to_string();
    json!({
        "id": id, "organizationId": org, "folderId": null, "type": 3, "key": null,
        "favorite": false, "reprompt": 0, "deletedDate": null, "collectionIds": [],
        "revisionDate": "2026-11-16T12:00:00.000Z", "creationDate": "2026-11-16T12:00:00.000Z",
        "name": enc(name), "notes": null, "edit": true, "viewPassword": true,
        "card": {
            "cardholderName": enc("Ana Lopez"), "brand": enc("Visa"),
            "number": enc("4111111111111111"), "expMonth": enc("7"),
            "expYear": enc("2029"), "code": enc("123"),
        },
    })
}

/// The vectors' sample vault, as the server would send it, with Fabrikam
/// (the person a member there) and a card in Contoso and in Fabrikam.
fn sample_sync() -> Value {
    let v = core_vectors();
    let mut sync = v["sync"].clone();
    for c in sync["ciphers"].as_array_mut().unwrap() {
        c["edit"] = true.into();
        c["viewPassword"] = true.into();
    }
    let contoso =
        SymmetricKey::from_bytes(&b64(v["keys"]["organizationKey"].as_str().unwrap())).unwrap();
    let fabrikam = SymmetricKey::generate();
    let public = b64(v["keys"]["publicKey"].as_str().unwrap());
    sync["profile"]["organizations"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": FABRIKAM, "name": "Fabrikam", "type": 2, "status": 2, "enabled": true,
            "key": encrypt_for(&public, &fabrikam.to_bytes()).unwrap().to_string(),
        }));
    let ciphers = sync["ciphers"].as_array_mut().unwrap();
    ciphers.push(card(CONTOSO_CARD, ORG, &contoso, "Contoso travel card"));
    ciphers.push(card(
        FABRIKAM_CARD,
        FABRIKAM,
        &fabrikam,
        "Fabrikam fuel card",
    ));
    sync
}

/// Contoso's "Remove card item type" policy, on or off, with this data.
fn contoso_policy(enabled: bool, data: Value) -> Value {
    json!([{
        "id": "p-contoso", "organizationId": ORG, "type": 15, "enabled": enabled,
        "data": data, "object": "policy",
    }])
}

/// A stand-in for the server: a session for the kept refresh token, and
/// `sync` as it is when the app asks. What it was asked, by method and path.
fn stand_in(sync: Arc<Mutex<Value>>) -> (String, Arc<Mutex<Vec<String>>>) {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let url = format!("http://{}", server.server_addr().to_ip().unwrap());
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = asked.clone();
    std::thread::spawn(move || {
        for request in server.incoming_requests() {
            let path = request.url().to_owned();
            log.lock()
                .unwrap()
                .push(format!("{} {path}", request.method()));
            let answer = if path == "/identity/connect/token" {
                json!({"access_token":"a.b.c","expires_in":3600,"refresh_token":"next","token_type":"Bearer"})
            } else if path.starts_with("/api/sync") {
                sync.lock().unwrap().clone()
            } else {
                json!({})
            };
            let _ = request.respond(
                tiny_http::Response::from_string(answer.to_string()).with_header(
                    tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap(),
                ),
            );
        }
    });
    (url, asked)
}

fn names(app: &App, query: &str) -> Vec<String> {
    app.vault()
        .unwrap()
        .search(query)
        .into_iter()
        .map(|i| i.name.clone())
        .collect()
}

fn expiring(app: &App) -> Vec<String> {
    expiration::expiring(app.vault().unwrap(), Date::parse_iso("2029-07-10").unwrap())
        .into_iter()
        .map(|(i, _)| i.id.clone())
        .collect()
}

fn new_card() -> ItemDraft {
    ItemDraft {
        name: "New card".into(),
        card: Some(Card {
            brand: Some("Visa".into()),
            ..Card::default()
        }),
        ..ItemDraft::default()
    }
}

#[test]
fn removed_cards_leave_every_list_and_come_back_at_the_next_sync() {
    let base = sample_sync();
    let served = Arc::new(Mutex::new(base.clone()));
    let (url, asked) = stand_in(served.clone());
    let sample = Sample::new(&url, &[], |s| *s = base.clone());
    let mut app = sample.unlocked();

    // No policy: every card, and the person's own in the export.
    assert_eq!(
        names(&app, "card"),
        vec!["Contoso travel card", "Everyday card", "Fabrikam fuel card"]
    );
    assert!(expiring(&app).contains(&CARD.to_owned()));
    let before = export::json(app.vault().unwrap()).unwrap();
    assert_eq!(before.items, 6);
    let file = sample.path("export.json");
    std::fs::write(&file, before.text.as_str()).unwrap();
    assert!(!app.vault().unwrap().restricted.refuses_new(ItemKind::Card));

    // Contoso turns it on (no list: cards). It binds its owner too.
    served.lock().unwrap()["policies"] = contoso_policy(true, Value::Null);
    app.sync().unwrap();
    let vault = app.vault().unwrap();
    assert_eq!(names(&app, "card"), vec!["Fabrikam fuel card"]);
    assert!(vault.item(CARD).is_none() && vault.item(CONTOSO_CARD).is_none());
    assert!(vault.item(FABRIKAM_CARD).is_some() && vault.item(IDENTITY).is_some());
    assert!(vault.hides(CARD) && vault.hides(CONTOSO_CARD) && !vault.hides(FABRIKAM_CARD));
    assert!(!vault.damaged.iter().any(|d| d.id == CARD));
    // Every count: the list, reminders, and the export.
    assert_eq!(vault.search("").len(), 7);
    assert!(!expiring(&app).contains(&CARD.to_owned()));
    assert!(expiring(&app).contains(&FABRIKAM_CARD.to_owned()));
    let made = export::json(vault).unwrap();
    assert_eq!((made.items, made.left_out), (5, 0));
    let doc: Value = serde_json::from_str(&made.text).unwrap();
    assert!(doc["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|i| i["type"] != 3));
    // Templates make logins and notes, which it doesn't remove.
    assert!(templates::groups(vault, |_| true)
        .iter()
        .any(|g| g.name == "Built-in"));

    // No new card, and nothing done to one it hides, before the server is asked.
    let asked_before = asked.lock().unwrap().len();
    assert!(matches!(
        app.save(None, &new_card(), &[]),
        Err(Error::Refused(_))
    ));
    assert!(matches!(
        app.save(Some(CARD), &new_card(), &[]),
        Err(Error::Damaged("no such item"))
    ));
    // Nor does a login it can see become a card.
    assert!(matches!(
        app.save(Some(MAIL), &new_card(), &[]),
        Err(Error::Refused(_))
    ));
    assert!(matches!(
        app.trash(CARD),
        Err(Error::Damaged("no such item"))
    ));
    assert!(app.history(CARD).is_err());
    assert!(app.value(CARD, "number").is_err());
    assert_eq!(asked.lock().unwrap().len(), asked_before);

    // An import leaves the card out, and says why.
    assert_eq!(
        app.read_import(Format::Json, &file, None).unwrap(),
        ImportRead::Read
    );
    let parsed = app.pending_import().unwrap();
    assert_eq!(parsed.items.len(), 5);
    assert!(parsed.items.iter().all(|i| i.kind != 3));
    assert_eq!(parsed.left_out, vec![(LeftOut::Restricted, 1)]);
    assert_eq!(parsed.kinds().get(&4), Some(&1));
    app.cancel_import();

    // Turned off: the next sync brings them back.
    served.lock().unwrap()["policies"] = contoso_policy(false, Value::Null);
    app.sync().unwrap();
    assert_eq!(names(&app, "card").len(), 3);
    assert!(app.vault().unwrap().item(CARD).is_some());
    assert!(!app.vault().unwrap().restricted.refuses_new(ItemKind::Card));

    // Fabrikam's own policy now, and Contoso's listing notes: Contoso's
    // card stays (its policy doesn't list cards), Fabrikam's and the
    // person's own go, and so do the person's notes, and the templates
    // that make notes.
    {
        let mut sync = served.lock().unwrap();
        let mut policies = contoso_policy(true, json!([2]));
        policies.as_array_mut().unwrap().push(json!({
            "id": "p-fabrikam", "organizationId": FABRIKAM, "type": 15, "enabled": true,
            "data": null, "object": "policy",
        }));
        sync["policies"] = policies;
    }
    app.sync().unwrap();
    let vault = app.vault().unwrap();
    assert_eq!(names(&app, "card"), vec!["Contoso travel card"]);
    assert!(vault.item(GATE).is_none());
    assert!(vault.restricted.refuses_new(ItemKind::Card));
    assert!(vault.restricted.refuses_new(ItemKind::SecureNote));
    assert!(!vault.restricted.refuses_new(ItemKind::Login));
    let offered: Vec<Base> = templates::groups(vault, |_| true)
        .into_iter()
        .flat_map(|g| g.templates)
        .map(|t| t.base)
        .collect();
    assert!(!offered.is_empty() && offered.iter().all(|b| *b == Base::Login));
    // Contoso's card is edited as before: it's a card already.
    assert!(vault.item(CONTOSO_CARD).is_some());
}

/// Opening the vault kept here (a restart, or unlocking again) reads the
/// policy from the sync kept with it, without the network.
#[test]
fn the_policy_holds_from_the_moment_the_vault_unlocks() {
    let mut sync = sample_sync();
    sync["policies"] = contoso_policy(true, json!([3, 4]));
    let sample = Sample::new("https://vault.example.test", &[], |s| *s = sync.clone());
    let app = sample.unlocked();
    let vault = app.vault().unwrap();
    assert!(vault.item(CARD).is_none() && vault.item(IDENTITY).is_none());
    assert!(vault.item(FABRIKAM_CARD).is_some());
    assert!(vault.restricted.refuses_new(ItemKind::Identity));
    assert_eq!(names(&app, "card"), vec!["Fabrikam fuel card"]);
}
