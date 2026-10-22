//! Editing an item keeps what the app doesn't show (docs/core.md, "Editing
//! keeps what the app doesn't show"): how each of its websites is matched for
//! filling (src/matching.rs), its own key, whether it asks for the master
//! password, its passkeys, its password history, and the fields the editor
//! doesn't edit; and it refuses edits it can't make safely. On the vectors'
//! sample vault, changed as the web vault would leave it.

use passbundle_core::crypto::{EncString, SymmetricKey};
use passbundle_core::kdf::Prelogin;
use passbundle_core::matching::{matches, Target};
use passbundle_core::vault::{Card, ItemDraft, Sync, Vault};
use zeroize::Zeroizing;

#[test]
fn editing_keeps_how_each_website_is_matched() {
    let mut v: serde_json::Value =
        serde_json::from_str(include_str!("vectors/core-v1.json")).unwrap();
    let pre = v["account"]["pbkdf2"]["prelogin"].clone();
    let password = v["account"]["password"].as_str().unwrap().to_owned();
    let id = "3d4e5f60-7182-4930-abc1-d2e3f4051627";
    let cipher = v["sync"]["ciphers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["id"] == id)
        .unwrap();
    // "Host": the same host only.
    cipher["login"]["uris"][0]["match"] = 1.into();
    let sync = Sync::from_json(&v["sync"].to_string()).unwrap();
    let prelogin: Prelogin = serde_json::from_value(pre).unwrap();
    let vault = Vault::unlock(&sync, &prelogin, &password).unwrap();
    let item = vault.item(id).unwrap();
    assert_eq!(item.uri_matches, vec![Some(1)]);

    // Edited, with the website kept and another added.
    let mut draft = ItemDraft::of(item);
    draft.name = "Renamed".into();
    draft.uris.push("https://new.example.test".into());
    let body = vault.encrypt_item(&draft, Some(item)).unwrap();
    let uris = body["login"]["uris"].as_array().unwrap();
    assert_eq!(uris[0]["match"], 1, "the kept website keeps its match");
    assert!(uris[1]["match"].is_null(), "a new one has the default");

    // And the setting counts when filling: its host (mail.example.test), not its whole domain.
    assert_eq!(item.uris[0], "https://mail.example.test/login");
    assert!(matches(item, &Target::Website("mail.example.test")));
    assert!(!matches(item, &Target::Website("other.example.test")));
}

/// The vectors' sample vault, as JSON to change before opening.
fn sample() -> (serde_json::Value, Prelogin, String) {
    let v: serde_json::Value = serde_json::from_str(include_str!("vectors/core-v1.json")).unwrap();
    let prelogin = serde_json::from_value(v["account"]["pbkdf2"]["prelogin"].clone()).unwrap();
    let password = v["account"]["password"].as_str().unwrap().to_owned();
    (v, prelogin, password)
}

fn open(v: &serde_json::Value, prelogin: &Prelogin, password: &str) -> Vault {
    Vault::unlock(
        &Sync::from_json(&v["sync"].to_string()).unwrap(),
        prelogin,
        password,
    )
    .unwrap()
}

fn cipher<'a>(v: &'a mut serde_json::Value, id: &str) -> &'a mut serde_json::Value {
    v["sync"]["ciphers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["id"] == id)
        .unwrap()
}

fn opened(key: &SymmetricKey, text: &serde_json::Value) -> String {
    EncString::parse(text.as_str().unwrap())
        .unwrap()
        .decrypt_string(key)
        .unwrap()
        .as_str()
        .to_owned()
}

#[test]
fn editing_keeps_the_item_key_reprompt_passkeys_other_fields_and_history() {
    let (mut v, prelogin, password) = sample();
    let id = v["ids"]["withKey"].as_str().unwrap().to_owned();
    // The item as the web vault would leave it: asking for the master
    // password, with a passkey, an earlier password, and a yes-or-no and a
    // linked field, all under the item's own key.
    let item_key = open(&v, &prelogin, &password)
        .item(&id)
        .unwrap()
        .kept
        .item_key
        .clone()
        .expect("the vectors' item has its own key");
    let enc = |text: &str| serde_json::Value::from(item_key.encrypt_str(text).to_string());
    let passkeys = serde_json::json!([{ "credentialId": enc("credential"), "keyType": enc("public-key"), "rpId": enc("backup.example.test") }]);
    let item_key_as_sent = {
        let c = cipher(&mut v, &id);
        c["reprompt"] = 1.into();
        c["passwordHistory"] = serde_json::json!([{ "password": enc("an-older-password"), "lastUsedDate": "2026-01-02T03:04:05.000Z" }]);
        c["login"]["fido2Credentials"] = passkeys.clone();
        c["login"]["autofillOnPageLoad"] = true.into();
        c["login"]["passwordRevisionDate"] = "2026-01-02T03:04:05.000Z".into();
        let fields = c["fields"].as_array_mut().unwrap();
        fields.push(serde_json::json!({ "name": enc("Remember me"), "value": enc("true"), "type": 2, "linkedId": null }));
        fields.push(serde_json::json!({ "name": enc("Login name"), "value": null, "type": 3, "linkedId": 100 }));
        c["key"].clone()
    };
    let vault = open(&v, &prelogin, &password);
    let item = vault.item(&id).unwrap();
    assert_eq!(item.kept.reprompt, 1);

    // The editor edits the text and hidden fields only. A new password, and
    // the hidden "API key" field removed.
    let mut draft = ItemDraft::of(item);
    assert_eq!(draft.fields.len(), 1);
    draft.password = Some("a-new-password".to_owned().into());
    draft.fields.clear();
    let body = vault.encrypt_item(&draft, Some(item)).unwrap();
    assert_eq!(body["reprompt"], 1, "still asks for the master password");
    assert_eq!(body["key"], item_key_as_sent, "keeps its own key");
    assert_eq!(
        body["login"]["fido2Credentials"], passkeys,
        "keeps its passkey"
    );
    assert_eq!(body["login"]["autofillOnPageLoad"], true);
    let changed_at = body["login"]["passwordRevisionDate"].as_str().unwrap();
    assert!(changed_at > "2026-01-02" && changed_at.ends_with('Z'));
    let kinds: Vec<_> = body["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["type"].clone())
        .collect();
    assert_eq!(kinds, vec![serde_json::json!(2), serde_json::json!(3)]);
    assert_eq!(body["fields"][1]["linkedId"], 100);

    // What the server keeps opens again, under the same key.
    let mut back = body.clone();
    back["id"] = id.clone().into();
    *cipher(&mut v, &id) = back;
    let reopened = open(&v, &prelogin, &password);
    let item = reopened.item(&id).unwrap();
    assert_eq!(
        item.password.as_deref().map(String::as_str),
        Some("a-new-password")
    );
    assert_eq!(item.fields.len(), 2);
    assert_eq!(
        (item.fields[0].name.as_str(), item.fields[0].value.as_str()),
        ("Remember me", "true")
    );
    assert_eq!(item.kept.field_kinds[1], (3, Some(100.into())));
    // Newest first: the removed hidden field, the old password, the earlier one.
    let history: Vec<String> = item
        .kept
        .password_history
        .iter()
        .map(|h| opened(&item_key, &h["password"]))
        .collect();
    assert_eq!(
        history,
        [
            "API key: k-123",
            "not-a-real-password-3",
            "an-older-password"
        ]
    );
}

#[test]
fn the_history_keeps_the_five_newest() {
    let (mut v, prelogin, password) = sample();
    let id = v["ids"]["withKey"].as_str().unwrap().to_owned();
    let item_key = open(&v, &prelogin, &password)
        .item(&id)
        .unwrap()
        .kept
        .item_key
        .clone()
        .unwrap();
    let earlier: Vec<_> = (1..=5)
        .map(|n| serde_json::json!({ "password": item_key.encrypt_str(&format!("earlier-{n}")).to_string(), "lastUsedDate": "2026-01-01T00:00:00.000Z" }))
        .collect();
    cipher(&mut v, &id)["passwordHistory"] = earlier.into();
    let vault = open(&v, &prelogin, &password);
    let item = vault.item(&id).unwrap();
    let mut draft = ItemDraft::of(item);
    draft.password = Some("a-new-password".to_owned().into());
    let body = vault.encrypt_item(&draft, Some(item)).unwrap();
    let history: Vec<String> = body["passwordHistory"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| opened(&item_key, &h["password"]))
        .collect();
    assert_eq!(
        history,
        [
            "not-a-real-password-3",
            "earlier-1",
            "earlier-2",
            "earlier-3",
            "earlier-4"
        ]
    );

    // Nothing changed: nothing new in it, and the password's date stays.
    let unchanged = vault
        .encrypt_item(&ItemDraft::of(item), Some(item))
        .unwrap();
    assert_eq!(unchanged["passwordHistory"].as_array().unwrap().len(), 5);
    assert!(unchanged["login"]["passwordRevisionDate"].is_null());
}

#[test]
fn refuses_edits_the_app_cant_make_safely() {
    let (mut v, prelogin, password) = sample();
    let note = v["ids"]["note"].as_str().unwrap().to_owned();
    let personal = v["ids"]["personal"].as_str().unwrap().to_owned();
    let organization = v["ids"]["organization"].as_str().unwrap().to_owned();
    // An SSH key: the web vault edits it, so the app never turns it into a login.
    let ssh_key = {
        let mut c = cipher(&mut v, &note).clone();
        c["id"] = "7a8b9c0d-1e2f-4a3b-8c4d-5e6f7a8b9c0d".into();
        c["type"] = 5.into();
        c["secureNote"] = serde_json::Value::Null;
        c["sshKey"] =
            serde_json::json!({ "privateKey": null, "publicKey": null, "keyFingerprint": null });
        v["sync"]["ciphers"].as_array_mut().unwrap().push(c);
        "7a8b9c0d-1e2f-4a3b-8c4d-5e6f7a8b9c0d"
    };
    let vault = open(&v, &prelogin, &password);
    let item = vault.item(ssh_key).unwrap();
    assert!(vault
        .encrypt_item(&ItemDraft::of(item), Some(item))
        .is_err());

    // A card stays a card.
    let card = vault.item(v["ids"]["card"].as_str().unwrap()).unwrap();
    let mut draft = ItemDraft::of(card);
    draft.card = None;
    assert!(vault.encrypt_item(&draft, Some(card)).is_err());

    // A note stays a note.
    let item = vault.item(&note).unwrap();
    let mut draft = ItemDraft::of(item);
    draft.note = false;
    assert!(vault.encrypt_item(&draft, Some(item)).is_err());

    // Moving an item to an organization is the web vault's, which re-encrypts it.
    let item = vault.item(&personal).unwrap();
    let mut draft = ItemDraft::of(item);
    draft.organization_id = Some(organization);
    assert!(vault.encrypt_item(&draft, Some(item)).is_err());
    assert!(vault.encrypt_item(&ItemDraft::of(item), Some(item)).is_ok());
}

#[test]
fn our_own_fields_stay_out_of_the_editor_and_through_an_edit() {
    let (mut v, prelogin, password) = sample();
    let id = v["ids"]["withKey"].as_str().unwrap().to_owned();
    let item_key = open(&v, &prelogin, &password)
        .item(&id)
        .unwrap()
        .kept
        .item_key
        .clone()
        .unwrap();
    let enc = |text: &str| serde_json::Value::from(item_key.encrypt_str(text).to_string());
    let rule = r#"{"v":1,"host":"backup.example.test","field":{"id":"custno"},"fill":{"from":"username"}}"#;
    let fields = cipher(&mut v, &id)["fields"].as_array_mut().unwrap();
    fields.push(serde_json::json!({ "name": enc("keykeeper.fill"), "value": enc(rule), "type": 1, "linkedId": null }));
    fields.push(serde_json::json!({ "name": enc("keykeeper.expires"), "value": enc("2026-11-01"), "type": 0, "linkedId": null }));
    let vault = open(&v, &prelogin, &password);
    let item = vault.item(&id).unwrap();
    assert!(item.fields[1].reserved() && item.fields[2].reserved());

    // The editor has only the person's own "API key" field, and removing it
    // puts only it in the history.
    let mut draft = ItemDraft::of(item);
    assert_eq!(draft.fields.len(), 1);
    draft.fields.clear();
    let body = vault.encrypt_item(&draft, Some(item)).unwrap();
    let kept: Vec<(String, String)> = body["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                opened(&item_key, &f["name"]),
                opened(&item_key, &f["value"]),
            )
        })
        .collect();
    assert_eq!(
        kept,
        [
            ("keykeeper.fill".to_owned(), rule.to_owned()),
            ("keykeeper.expires".to_owned(), "2026-11-01".to_owned()),
        ]
    );
    assert_eq!(body["fields"][0]["type"], 1);
    let history: Vec<String> = body["passwordHistory"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| opened(&item_key, &h["password"]))
        .collect();
    assert_eq!(history, ["API key: k-123"]);
}

#[test]
fn websites_carry_their_checksums_and_kept_fields_keep_their_places() {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let (mut v, prelogin, password) = sample();
    let id = v["ids"]["withKey"].as_str().unwrap().to_owned();
    let item_key = open(&v, &prelogin, &password)
        .item(&id)
        .unwrap()
        .kept
        .item_key
        .clone()
        .unwrap();
    let enc = |text: &str| serde_json::Value::from(item_key.encrypt_str(text).to_string());
    // A yes-or-no field first, before the "API key" field.
    cipher(&mut v, &id)["fields"]
        .as_array_mut()
        .unwrap()
        .insert(0, serde_json::json!({ "name": enc("Remember me"), "value": enc("true"), "type": 2, "linkedId": null }));
    let vault = open(&v, &prelogin, &password);
    let item = vault.item(&id).unwrap();
    let mut draft = ItemDraft::of(item);
    draft.uris.push("https://new.example.test/".into());
    let body = vault.encrypt_item(&draft, Some(item)).unwrap();

    // Upstream's apps show an item with its own key only websites whose
    // checksum, base64 of the address's SHA-256, matches.
    for uri in body["login"]["uris"].as_array().unwrap() {
        let address = opened(&item_key, &uri["uri"]);
        let checksum =
            base64::engine::general_purpose::STANDARD.encode(Sha256::digest(address.as_bytes()));
        assert_eq!(
            opened(&item_key, &uri["uriChecksum"]),
            checksum,
            "{address}"
        );
    }
    let names: Vec<String> = body["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| opened(&item_key, &f["name"]))
        .collect();
    assert_eq!(names, ["Remember me", "API key"]);
}

fn user_key(v: &serde_json::Value) -> SymmetricKey {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(v["keys"]["userKey"].as_str().unwrap())
        .unwrap();
    SymmetricKey::from_bytes(&bytes).unwrap()
}

#[test]
fn cards_and_identities_are_edited_and_made() {
    let (v, prelogin, password) = sample();
    let key = user_key(&v);
    let vault = open(&v, &prelogin, &password);

    // A new number and code on the vectors' card; it stays a card.
    let card = vault.item(v["ids"]["card"].as_str().unwrap()).unwrap();
    let mut draft = ItemDraft::of(card);
    let details = draft.card.as_mut().unwrap();
    details.number = Some(Zeroizing::new("5555555555554444".into()));
    details.code = Some(Zeroizing::new("321".into()));
    details.brand = Some("Mastercard".into());
    let body = vault.encrypt_item(&draft, Some(card)).unwrap();
    assert_eq!(body["type"], 3);
    assert!(body["login"].is_null() && body["passwordHistory"].is_null());
    let c = &body["card"];
    assert_eq!(opened(&key, &c["number"]), "5555555555554444");
    assert_eq!(opened(&key, &c["code"]), "321");
    assert_eq!(opened(&key, &c["brand"]), "Mastercard");
    assert_eq!(opened(&key, &c["cardholderName"]), "Ana Lopez");
    assert_eq!(
        (opened(&key, &c["expMonth"]), opened(&key, &c["expYear"])),
        ("7".to_owned(), "2029".to_owned())
    );

    // A new email on the vectors' identity; the rest as it was.
    let identity = vault.item(v["ids"]["identity"].as_str().unwrap()).unwrap();
    let mut draft = ItemDraft::of(identity);
    draft.identity.as_mut().unwrap().email = Some("ana@example.test".into());
    let body = vault.encrypt_item(&draft, Some(identity)).unwrap();
    assert_eq!(body["type"], 4);
    let i = &body["identity"];
    assert_eq!(opened(&key, &i["email"]), "ana@example.test");
    assert_eq!(opened(&key, &i["ssn"]), "123-45-6789");
    assert_eq!(opened(&key, &i["middleName"]), "María");
    assert!(i["address3"].is_null());

    // A new card, under the user key.
    let new = ItemDraft {
        name: "Work card".into(),
        card: Some(Card {
            cardholder_name: Some("Ana Lopez".into()),
            number: Some(Zeroizing::new("4000056655665556".into())),
            ..Card::default()
        }),
        ..ItemDraft::default()
    };
    let body = vault.encrypt_item(&new, None).unwrap();
    assert_eq!(body["type"], 3);
    assert_eq!(opened(&key, &body["card"]["number"]), "4000056655665556");
    assert!(body["card"]["code"].is_null());
}
