//! Fill rules on a website, for Android's autofill (src/fill_rules.rs for
//! the rules themselves, checked against the meta repo's vectors there; and
//! src/app/fill_plan.rs), on the vectors' sample vault kept on this device
//! with a bank's login of the person's own and one shared with them that
//! hides its passwords, each with fill rules: the account's "Never fill this
//! field on this site" list comes from the settings document the server
//! keeps, opened with the account's user key, and holds before the login's
//! own rule; one another key encrypted, or another account's document, holds
//! nothing; what the person may see of a shared login limits where its
//! password goes; and nothing is planned while the account is locked.

use passbundle_core::app::App;
use passbundle_core::crypto::SymmetricKey;
use passbundle_core::fill_rules::{Fill, Page, PageField, Take};
use passbundle_core::generator::Options;
use passbundle_core::store::Store;
use passbundle_core::synced_settings::{self, Known};
use serde_json::{json, Value};

#[path = "support/stand_in.rs"]
mod stand_in;
use stand_in::{core_vectors, Sample, IT, ORG};

const SERVER: &str = "https://vault.example.test";
/// The person's own login for the bank, and one shared with them in IT.
const BANK: &str = "b0b0b0b0-0000-4000-8000-0000000000b1";
const SHARED: &str = "b0b0b0b0-0000-4000-8000-0000000000b2";

fn key(name: &str) -> SymmetricKey {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(core_vectors()["keys"][name].as_str().unwrap())
        .unwrap();
    SymmetricKey::from_bytes(&bytes).unwrap()
}

/// A login for the bank with a customer ID, and rules: its password into
/// the text field with id custno and into the password field, and the
/// customer ID into the field with id member.
fn bank_login(id: &str, org: Option<&str>, key: &SymmetricKey, view_password: bool) -> Value {
    let enc = |text: &str| key.encrypt_str(text).to_string();
    let rule = |field: &str, fill: Value| {
        json!({ "v": 1, "host": "bank.example.test", "field": { "id": field }, "fill": fill })
            .to_string()
    };
    let rules = [
        rule("custno", json!({ "from": "password" })),
        rule("pass", json!({ "from": "password" })),
        rule("member", json!({ "from": "field", "name": "Customer ID" })),
    ];
    let mut fields = vec![
        json!({ "name": enc("Customer ID"), "value": enc("C-1234"), "type": 0, "linkedId": null }),
    ];
    for rule in &rules {
        fields.push(
            json!({ "name": enc("keykeeper.fill"), "value": enc(rule), "type": 1, "linkedId": null }),
        );
    }
    json!({
        "id": id, "organizationId": org, "folderId": null, "type": 1, "key": null,
        "favorite": false, "reprompt": 0, "deletedDate": null,
        "collectionIds": if org.is_some() { json!([IT]) } else { json!([]) },
        "revisionDate": "2026-11-19T12:00:00.000Z", "creationDate": "2026-11-19T12:00:00.000Z",
        "name": enc("Example Bank"), "notes": null, "edit": true, "viewPassword": view_password,
        "login": {
            "username": enc("ana@example.test"), "password": enc("Fake-password-1"), "totp": null,
            "uris": [{ "uri": enc("https://bank.example.test/"), "match": null }],
        },
        "fields": fields,
    })
}

fn sample() -> Sample {
    Sample::new(SERVER, &[], |sync| {
        let ciphers = sync["ciphers"].as_array_mut().unwrap();
        ciphers.push(bank_login(BANK, None, &key("userKey"), true));
        ciphers.push(bank_login(
            SHARED,
            Some(ORG),
            &key("organizationKey"),
            false,
        ));
    })
}

/// The account's settings document as the last sync kept it, with this
/// never-fill list encrypted with `key`, for `account`.
fn keep_settings(sample: &Sample, account: &str, list: &Value, key: &SymmetricKey) {
    let mut settings = serde_json::Map::new();
    settings.insert("type".into(), synced_settings::SETTINGS_TYPE.into());
    settings.insert("rev".into(), 1.into());
    settings.insert(
        "fill".into(),
        json!({ "neverFill": key.encrypt_str(&list.to_string()).to_string() }),
    );
    Store::new(sample.path(""))
        .save_synced_settings(&Known {
            account: account.into(),
            settings,
            generator: Options::default(),
        })
        .unwrap();
}

fn this_account() -> String {
    synced_settings::account(SERVER, core_vectors()["account"]["salt"].as_str().unwrap())
}

fn field(id: &str, kind: &str) -> PageField {
    PageField {
        tag: Some("input".into()),
        kind: Some(kind.into()),
        id: Some(id.into()),
        visible: true,
        ..Default::default()
    }
}

/// The bank's sign-in page, as Chrome describes it to Android.
fn bank_page() -> Page {
    Page {
        scheme: Some("https".into()),
        host: "bank.example.test".into(),
        fields: vec![
            field("user", "email"),
            field("pass", "password"),
            field("custno", "text"),
            field("member", "text"),
        ],
        forms: vec![],
    }
}

fn custno_never() -> Value {
    json!([{ "v": 1, "host": "bank.example.test", "field": { "id": "custno" }, "fill": { "from": "never" } }])
}

fn customer_id() -> Fill {
    Fill {
        field: 3,
        what: "field:0".into(),
        take: Take::Text,
    }
}

fn password_into(field: usize) -> Fill {
    Fill {
        field,
        what: "password".into(),
        take: Take::Text,
    }
}

fn unlocked(sample: &Sample) -> App {
    sample.unlocked()
}

#[test]
fn the_never_fill_list_comes_from_the_settings_document_and_holds_before_the_logins_rule() {
    let sample = sample();
    let app = unlocked(&sample);
    // No settings document yet: the login's own rules.
    assert!(app.site_never_fill().is_empty());
    let plan = app.fill_plan(BANK, &bank_page()).unwrap();
    assert_eq!(
        plan.fills,
        [password_into(1), password_into(2), customer_id()]
    );
    assert!(plan.never.is_empty());

    keep_settings(&sample, &this_account(), &custno_never(), &key("userKey"));
    let app = unlocked(&sample);
    assert_eq!(app.site_never_fill().len(), 1);
    let plan = app.fill_plan(BANK, &bank_page()).unwrap();
    assert_eq!(plan.fills, [password_into(1), customer_id()]);
    assert_eq!(plan.never, [2]);
    // For every item: the shared login too.
    assert_eq!(app.fill_plan(SHARED, &bank_page()).unwrap().never, [2]);
    // The list is kept as the server has it: encrypted, without the site in it.
    let kept = std::fs::read_to_string(sample.path("generator-synced.json")).unwrap();
    assert!(!kept.contains("bank.example.test") && !kept.contains("custno"));
}

#[test]
fn a_list_another_key_encrypted_or_another_accounts_holds_nothing() {
    let sample = sample();
    keep_settings(
        &sample,
        &this_account(),
        &custno_never(),
        &SymmetricKey::generate(),
    );
    let app = unlocked(&sample);
    assert!(app.site_never_fill().is_empty());
    assert_eq!(
        app.fill_plan(BANK, &bank_page()).unwrap().fills,
        [password_into(1), password_into(2), customer_id()]
    );

    let other = synced_settings::account(SERVER, "someone.else@example.test");
    keep_settings(&sample, &other, &custno_never(), &key("userKey"));
    let app = unlocked(&sample);
    assert!(app.site_never_fill().is_empty());
}

#[test]
fn a_shared_login_that_hides_its_password_puts_it_only_into_a_password_field() {
    let sample = sample();
    let app = unlocked(&sample);
    // Its customer ID fills where its rule says, and its password goes into
    // the password field, never the text field its other rule names.
    let plan = app.fill_plan(SHARED, &bank_page()).unwrap();
    assert_eq!(plan.fills, [password_into(1), customer_id()]);
    // Someone who can see it gets it where the rule says.
    let plan = app.fill_plan(BANK, &bank_page()).unwrap();
    assert_eq!(
        plan.fills,
        [password_into(1), password_into(2), customer_id()]
    );
}

#[test]
fn nothing_is_planned_while_locked_or_for_an_item_it_hasnt() {
    let sample = sample();
    let mut app = unlocked(&sample);
    assert!(app.fill_plan("no-such-item", &bank_page()).is_err());
    app.lock();
    assert!(app.fill_plan(BANK, &bank_page()).is_err());
    assert!(app.site_never_fill().is_empty());
}
