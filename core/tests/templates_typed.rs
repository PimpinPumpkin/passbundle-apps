//! Typed fields from templates (src/templates.rs; DESIGN.md, "Item
//! templates", "Where a field's type is kept" and "A date field that sets
//! Expires"): a field's kind read from a template's definition or an item's
//! marker, an unknown kind as text, the expiration date field and which
//! date wins, a one-time code field's key, the kinds written back as the
//! editor saves, a new template's definition, and "Save as template". On the
//! vectors' sample vault, changed as the web vault would leave it.

use base64::Engine;
use passbundle_core::crypto::{EncString, SymmetricKey};
use passbundle_core::expiration;
use passbundle_core::kdf::Prelogin;
use passbundle_core::templates::{self, Base, FieldKind, Typing};
use passbundle_core::vault::{Field, ItemDraft, Sync, Vault};
use serde_json::{json, Value};
use zeroize::Zeroizing;

/// The vectors' note, in Personal with no key of its own.
const NOTE: &str = "60718293-a4b5-4c63-def4-05162738495a";

fn vectors() -> Value {
    serde_json::from_str(include_str!("vectors/core-v1.json")).unwrap()
}

fn user_key(v: &Value) -> SymmetricKey {
    SymmetricKey::from_bytes(
        &base64::engine::general_purpose::STANDARD
            .decode(v["keys"]["userKey"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap()
}

/// The sample with the note's fields as given: (name, value, upstream type).
fn vault_with(fields: &[(&str, &str, u8)]) -> (Vault, SymmetricKey) {
    let mut v = vectors();
    let key = user_key(&v);
    let enc = |text: &str| Value::from(key.encrypt_str(text).to_string());
    let note = v["sync"]["ciphers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["id"] == NOTE)
        .unwrap();
    note["fields"] = fields
        .iter()
        .map(|(name, value, kind)| {
            json!({ "name": enc(name), "value": enc(value), "type": kind, "linkedId": null })
        })
        .collect();
    let prelogin: Prelogin =
        serde_json::from_value(v["account"]["pbkdf2"]["prelogin"].clone()).unwrap();
    let password = v["account"]["password"].as_str().unwrap().to_owned();
    let sync = Sync::from_json(&v["sync"].to_string()).unwrap();
    (Vault::unlock(&sync, &prelogin, &password).unwrap(), key)
}

fn opened(key: &SymmetricKey, text: &Value) -> String {
    EncString::parse(text.as_str().unwrap())
        .unwrap()
        .decrypt_string(key)
        .unwrap()
        .as_str()
        .to_owned()
}

/// A saved body's fields, opened: (name, value).
fn fields_of(body: &Value, key: &SymmetricKey) -> Vec<(String, String)> {
    body["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (opened(key, &f["name"]), opened(key, &f["value"])))
        .collect()
}

fn ours(fields: &[(String, String)], name: &str) -> Value {
    serde_json::from_str(&fields.iter().find(|(n, _)| n == name).unwrap().1).unwrap()
}

const MARKER: &str = r#"{"id":"lic","name":"License","v":1,"types":{"Renews":"date","Console code":"totp","Site":"url","Later":"fingerprint","Hidden date":"date"},"expires":"Renews"}"#;

/// An item from a custom template: its marker says each field's kind.
fn from_template(renews: &str, expires: &str) -> (Vault, SymmetricKey) {
    vault_with(&[
        ("Renews", renews, 0),
        ("Console code", "JBSWY3DPEHPK3PXP", 1),
        ("Site", "https://vendor.example.test", 0),
        ("Later", "x", 0),
        ("Hidden date", "2027-01-01", 1),
        ("keykeeper.template", MARKER, 0),
        ("keykeeper.expires", expires, 0),
    ])
}

#[test]
fn an_items_fields_have_the_kinds_its_template_gave_them_and_an_unknown_one_is_text() {
    let (vault, _) = from_template("2027-06-30", "2027-06-30");
    let item = vault.item(NOTE).unwrap();

    assert_eq!(
        templates::field_kinds(item),
        vec![
            Some(FieldKind::Date),
            Some(FieldKind::Totp),
            Some(FieldKind::Url),
            // A kind this app doesn't know is text.
            Some(FieldKind::Text),
            // A date must be a text field: a hidden one stays hidden.
            Some(FieldKind::Hidden),
            // Our own data has none.
            None,
            None,
        ]
    );
    let typing = templates::typing_of(item);
    assert_eq!(typing.expires.as_deref(), Some("Renews"));
    assert!(typing.is_expiration("Renews", 0));
    assert!(!typing.is_expiration("Hidden date", 1));

    // A one-time code field's key, for its current code; nothing for another field.
    assert_eq!(templates::field_code_key(item, 1), Some("JBSWY3DPEHPK3PXP"));
    assert_eq!(templates::field_code_key(item, 0), None);
    assert_eq!(templates::field_code_key(item, 4), None);
}

#[test]
fn the_expiration_date_field_is_the_items_date_and_wins_while_it_holds_one() {
    let date = |iso: &str| expiration::Date::parse_iso(iso);

    let (vault, _) = from_template("2027-06-30", "2027-06-30");
    assert_eq!(
        expiration::stored(vault.item(NOTE).unwrap()),
        date("2027-06-30")
    );

    // Another app changed Expires and not the field: the field wins.
    let (vault, _) = from_template("2027-06-30", "2027-03-31");
    assert_eq!(
        expiration::stored(vault.item(NOTE).unwrap()),
        date("2027-06-30")
    );

    // Without a date in it, Expires stands.
    for field in ["", "next spring", "2027-02-30"] {
        let (vault, _) = from_template(field, "2027-03-31");
        assert_eq!(
            expiration::stored(vault.item(NOTE).unwrap()),
            date("2027-03-31"),
            "{field:?}"
        );
    }

    // A template's own date is a default for new items, not its expiration.
    let (vault, _) = vault_with(&[
        ("Renews", "2027-06-30", 0),
        (
            "keykeeper.template-definition",
            r#"{"id":"lic","base":"note","v":1,"types":{"Renews":"date"},"expires":"Renews"}"#,
            0,
        ),
    ]);
    assert_eq!(expiration::stored(vault.item(NOTE).unwrap()), None);
}

#[test]
fn saving_writes_the_editors_kinds_into_the_marker_and_keeps_the_rest_of_it() {
    let (vault, key) = from_template("2027-06-30", "2027-06-30");
    let item = vault.item(NOTE).unwrap();

    // "Site" renamed "Vendor site", keeping its kind.
    let mut draft = ItemDraft::of(item);
    for field in &mut draft.fields {
        if field.name == "Site" {
            field.name = "Vendor site".into();
        }
    }
    let mut types = std::collections::BTreeMap::new();
    types.insert("Renews".to_owned(), FieldKind::Date);
    types.insert("Console code".to_owned(), FieldKind::Totp);
    types.insert("Vendor site".to_owned(), FieldKind::Url);
    draft.typing = Some(Typing {
        types,
        expires: Some("Renews".into()),
    });
    let fields = fields_of(&vault.encrypt_item(&draft, Some(item)).unwrap(), &key);
    assert_eq!(
        ours(&fields, "keykeeper.template"),
        json!({
            "id": "lic", "name": "License", "v": 1,
            "types": { "Renews": "date", "Console code": "totp", "Vendor site": "url" },
            "expires": "Renews",
        })
    );

    // Without kinds from the editor, the marker stays as it was.
    let draft = ItemDraft::of(item);
    let fields = fields_of(&vault.encrypt_item(&draft, Some(item)).unwrap(), &key);
    assert_eq!(
        fields
            .iter()
            .find(|(n, _)| n == "keykeeper.template")
            .unwrap()
            .1,
        MARKER
    );
}

#[test]
fn a_new_template_gets_a_definition_with_a_fresh_id_and_its_kinds() {
    let (vault, key) = vault_with(&[]);
    let typing = Typing::of_fields(&[templates::TemplateField {
        label: "Renews".into(),
        kind: FieldKind::Date,
        value: None,
        expires: true,
    }]);
    let draft = ItemDraft {
        note: true,
        name: "Software license".into(),
        fields: vec![Field {
            name: "Renews".into(),
            value: Zeroizing::new(String::new()),
            hidden: false,
            boolean: false,
        }],
        typing: Some(typing),
        new_template: Some(Base::Login),
        ..ItemDraft::default()
    };
    let first = fields_of(&vault.encrypt_item(&draft, None).unwrap(), &key);
    let second = fields_of(&vault.encrypt_item(&draft, None).unwrap(), &key);
    let definition = ours(&first, "keykeeper.template-definition");
    assert_eq!(definition["base"], "login");
    assert_eq!(definition["v"], 1);
    assert_eq!(definition["types"], json!({ "Renews": "date" }));
    assert_eq!(definition["expires"], "Renews");
    assert!(definition.get("from").is_none(), "never a copy from an MSP");
    assert_ne!(
        definition["id"],
        ours(&second, "keykeeper.template-definition")["id"],
        "each a fresh id"
    );
    assert!(!first.iter().any(|(n, _)| n == "keykeeper.template"));

    // Only a note becomes one.
    let login = ItemDraft {
        note: false,
        ..draft
    };
    let fields = fields_of(&vault.encrypt_item(&login, None).unwrap(), &key);
    assert!(!fields.iter().any(|(n, _)| n.starts_with("keykeeper.")));
}

#[test]
fn a_new_item_from_a_template_records_its_kinds_in_the_marker() {
    let (vault, key) = vault_with(&[]);
    let api_key = templates::built_in()
        .into_iter()
        .find(|t| t.id == "api-key")
        .unwrap();
    let draft = ItemDraft {
        note: true,
        name: "Payments API".into(),
        template: Some(("api-key".into(), "API key".into())),
        typing: Some(Typing::of_fields(&api_key.fields)),
        ..ItemDraft::default()
    };
    let fields = fields_of(&vault.encrypt_item(&draft, None).unwrap(), &key);
    assert_eq!(
        ours(&fields, "keykeeper.template"),
        json!({ "id": "api-key", "name": "API key", "v": 1, "types": { "Service URL": "url" } })
    );
}

#[test]
fn a_templates_fields_have_their_kinds_and_save_as_template_takes_labels_and_kinds_only() {
    // A template note.
    let (vault, _) = vault_with(&[
        ("Renews", "2027-06-30", 0),
        ("Console code", "JBSWY3DPEHPK3PXP", 1),
        (
            "keykeeper.template-definition",
            r#"{"id":"lic","base":"note","v":1,"types":{"Renews":"date","Console code":"totp"},"expires":"Renews"}"#,
            0,
        ),
    ]);
    let template = templates::custom(vault.item(NOTE).unwrap()).unwrap();
    let kinds: Vec<_> = template
        .fields
        .iter()
        .map(|f| (f.label.as_str(), f.kind, f.expires))
        .collect();
    assert_eq!(
        kinds,
        [
            ("Renews", FieldKind::Date, true),
            ("Console code", FieldKind::Totp, false),
        ]
    );
    // A template isn't made into another.
    assert!(templates::from_item(vault.item(NOTE).unwrap()).is_none());
    // Serialized for the apps by its kinds' names.
    let json = serde_json::to_value(&template.fields).unwrap();
    assert_eq!(json[0]["kind"], "date");
    assert_eq!(json[0]["expires"], true);
    assert_eq!(json[1]["kind"], "totp");
    assert!(json[1].get("expires").is_none());

    // An item from one: its fields by label and kind, never their values.
    let (vault, _) = from_template("2027-06-30", "2027-06-30");
    let (base, fields) = templates::from_item(vault.item(NOTE).unwrap()).unwrap();
    assert_eq!(base, Base::Note);
    let kinds: Vec<_> = fields
        .iter()
        .map(|f| (f.label.as_str(), f.kind, f.expires, f.value.clone()))
        .collect();
    assert_eq!(
        kinds,
        [
            ("Renews", FieldKind::Date, true, None),
            ("Console code", FieldKind::Totp, false, None),
            ("Site", FieldKind::Url, false, None),
            ("Later", FieldKind::Text, false, None),
            ("Hidden date", FieldKind::Hidden, false, None),
        ]
    );
}

#[test]
fn our_own_export_brings_back_kinds_only_when_well_formed() {
    use passbundle_core::import::{guard_reserved_fields, Field as Imported, Item};
    let field = |name: &str, value: &str| Imported {
        name: Some(name.to_owned()),
        value: Some(Zeroizing::new(value.to_owned())),
        kind: 0,
        linked_id: None,
    };
    let mut items = vec![Item {
        kind: 2,
        fields: vec![
            field("keykeeper.template", MARKER),
            field(
                "keykeeper.template-definition",
                r#"{"id":"t","base":"note","v":1,"from":"MSP","types":{"A":"date","B":"script"},"expires":"A"}"#,
            ),
        ],
        ..Item::default()
    }];
    guard_reserved_fields(&mut items, true);
    let value = |i: usize| items[0].fields[i].value.as_deref().unwrap().to_string();
    // Rewritten from what's well formed, in the web vault's order.
    assert_eq!(
        value(0),
        r#"{"id":"lic","name":"License","v":1,"types":{"Console code":"totp","Hidden date":"date","Renews":"date","Site":"url"},"expires":"Renews"}"#
    );
    assert_eq!(
        value(1),
        r#"{"id":"t","base":"note","v":1,"types":{"A":"date"},"expires":"A"}"#
    );
}

#[test]
fn saving_keeps_expires_with_the_expiration_date_field() {
    let date = |iso: &str| expiration::Date::parse_iso(iso);
    let text = |name: &str, value: &str| Field {
        name: name.into(),
        value: Zeroizing::new(value.into()),
        hidden: false,
        boolean: false,
    };
    let expires = |fields: &[(String, String)]| {
        fields
            .iter()
            .find(|(n, _)| n == "keykeeper.expires")
            .map(|(_, v)| v.clone())
    };

    // A new item from a template, its date filled: Expires is that date.
    let (vault, key) = vault_with(&[]);
    let mut types = std::collections::BTreeMap::new();
    types.insert("Renews".to_owned(), FieldKind::Date);
    let typing = Typing {
        types,
        expires: Some("Renews".into()),
    };
    let new = ItemDraft {
        note: true,
        name: "Office license".into(),
        fields: vec![text("Renews", "2027-06-30")],
        template: Some(("lic".into(), "License".into())),
        typing: Some(typing.clone()),
        ..ItemDraft::default()
    };
    let fields = fields_of(&vault.encrypt_item(&new, None).unwrap(), &key);
    assert_eq!(expires(&fields).as_deref(), Some("2027-06-30"));

    // An edit whose Expires says one date and the field another: the field.
    let (vault, key) = from_template("2027-06-30", "2027-06-30");
    let item = vault.item(NOTE).unwrap();
    let mut draft = ItemDraft::of(item);
    for field in &mut draft.fields {
        if field.name == "Renews" {
            field.value = Zeroizing::new("2027-09-30".into());
        }
    }
    draft.expiration = Some(passbundle_core::vault::Expiration {
        date: date("2027-12-31"),
        remind: expiration::Remind::default(),
    });
    let fields = fields_of(&vault.encrypt_item(&draft, Some(item)).unwrap(), &key);
    assert_eq!(expires(&fields).as_deref(), Some("2027-09-30"));
    assert_eq!(
        fields
            .iter()
            .filter(|(n, _)| n == "keykeeper.expires")
            .count(),
        1
    );

    // A field without a date leaves Expires as the editor set it.
    let mut draft = ItemDraft::of(item);
    for field in &mut draft.fields {
        if field.name == "Renews" {
            field.value = Zeroizing::new(String::new());
        }
    }
    let fields = fields_of(&vault.encrypt_item(&draft, Some(item)).unwrap(), &key);
    assert_eq!(expires(&fields).as_deref(), Some("2027-06-30"));

    // A new template's date is a default for its items, never its own.
    let (vault, key) = vault_with(&[]);
    let template = ItemDraft {
        template: None,
        new_template: Some(Base::Note),
        ..new
    };
    let fields = fields_of(&vault.encrypt_item(&template, None).unwrap(), &key);
    assert_eq!(expires(&fields), None);
}
