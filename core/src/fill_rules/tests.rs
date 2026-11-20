use serde_json::{json, Value};
use zeroize::Zeroizing;

use super::*;
use crate::vault::{Card, Field, Identity, Kept};

/// An item as the vectors describe it (`fill-rules-v1.json`).
fn item_of(v: &Value) -> Item {
    let kind = match v["type"].as_str().unwrap() {
        "login" => ItemKind::Login,
        "card" => ItemKind::Card,
        "identity" => ItemKind::Identity,
        other => panic!("no such kind {other}"),
    };
    let text = |o: &Value, key: &str| o[key].as_str().map(str::to_owned);
    let secret = |o: &Value, key: &str| text(o, key).map(Zeroizing::new);
    let mut field_kinds = Vec::new();
    let fields = v["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let kind = f["type"].as_u64().unwrap() as u8;
            field_kinds.push((kind, None));
            // A rule as it's kept: its JSON text.
            let value = match &f["rule"] {
                Value::Null => f["value"].as_str().unwrap_or("").to_owned(),
                rule => rule.to_string(),
            };
            Field {
                name: f["name"].as_str().unwrap().to_owned(),
                value: Zeroizing::new(value),
                hidden: kind == 1,
                boolean: kind == 2,
            }
        })
        .collect();
    let c = &v["card"];
    let card = (kind == ItemKind::Card).then(|| Card {
        cardholder_name: text(c, "cardholderName"),
        brand: text(c, "brand"),
        exp_month: text(c, "expMonth"),
        exp_year: text(c, "expYear"),
        number: secret(c, "number"),
        code: secret(c, "code"),
    });
    let i = &v["identity"];
    let identity = (kind == ItemKind::Identity).then(|| Identity {
        title: text(i, "title"),
        first_name: text(i, "firstName"),
        middle_name: text(i, "middleName"),
        last_name: text(i, "lastName"),
        address1: text(i, "address1"),
        address2: text(i, "address2"),
        address3: text(i, "address3"),
        city: text(i, "city"),
        state: text(i, "state"),
        postal_code: text(i, "postalCode"),
        country: text(i, "country"),
        company: text(i, "company"),
        email: text(i, "email"),
        phone: text(i, "phone"),
        ..Default::default()
    });
    let login = kind == ItemKind::Login;
    Item {
        id: "item".into(),
        kind,
        name: "Item".into(),
        folder_id: None,
        organization_id: None,
        collection_ids: vec![],
        favorite: false,
        deleted: false,
        archived: false,
        username: text(v, "username").filter(|_| login),
        uris: vec![],
        uri_matches: vec![],
        notes: None,
        password: secret(v, "password").filter(|_| login),
        totp: secret(v, "totp").filter(|_| login),
        fields,
        revision_date: None,
        creation_date: None,
        editable: true,
        view_password: v["seesSecrets"].as_bool().unwrap(),
        deletable: true,
        card,
        identity,
        ssh_key: None,
        attachments: vec![],
        passkey: None,
        details: vec![],
        kept: Kept {
            field_kinds,
            ..Default::default()
        },
        organization_use_totp: false,
    }
}

fn may_of(v: &Value) -> May {
    May {
        see_secrets: v["seesSecrets"].as_bool().unwrap(),
        codes: v["codes"].as_bool().unwrap(),
    }
}

/// The rule's source a value's name is, as the vectors write it.
fn source_of(item: &Item, what: &str) -> Value {
    match what {
        "username" | "password" | "totp" => json!({ "from": what }),
        "cardNumber" => json!({ "from": "card", "value": "number" }),
        "cardCode" => json!({ "from": "card", "value": "code" }),
        what if what.starts_with("field:") => {
            let at: usize = what["field:".len()..].parse().unwrap();
            json!({ "from": "field", "name": item.fields[at].name })
        }
        other if item.kind == ItemKind::Card => json!({ "from": "card", "value": other }),
        other => json!({ "from": "identity", "value": other }),
    }
}

fn vectors() -> Value {
    serde_json::from_str(include_str!("../../tests/vectors/fill-rules-v1.json")).unwrap()
}

#[test]
fn the_vectors_fill_where_they_say_and_nowhere_else() {
    let v = vectors();
    let cases = v["cases"].as_array().unwrap();
    assert!(cases.len() >= 40);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let page: Page = serde_json::from_value(case["page"].clone()).unwrap();
        let item = item_of(&case["item"]);
        let never_fill = never_fill_list(&case["neverFill"].to_string());
        let plan = plan(&page, &item, &never_fill, may_of(&case["item"]));
        let fills: Vec<Value> = plan
            .fills
            .iter()
            .map(|f| json!({ "field": f.field, "fill": source_of(&item, &f.what) }))
            .collect();
        assert_eq!(Value::from(fills), case["fills"], "{name}");
        assert_eq!(json!(plan.never), case["never"], "{name}");
    }
}

#[test]
fn the_vectors_cover_each_limit() {
    let v = vectors();
    let names: Vec<&str> = v["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    for wanted in [
        "the id decides",
        "only on the host it was made on",
        "never on an http copy of the site",
        "site-wide never-fill holds for every item, and before the item's own rule",
        "a password only into a password field",
        "its number and code only into a password field",
        "never a field people can't see",
        "never a field the generic matching never fills",
        "never a field in a newsletter form",
        "a card's value only on the page's own site",
        "an identity's values where its rules say, for anyone",
        "so never on Android",
        "doesn't say the page's scheme",
    ] {
        assert!(names.iter().any(|n| n.contains(wanted)), "{wanted}");
    }
}

fn field(id: &str) -> PageField {
    PageField {
        tag: Some("input".into()),
        kind: Some("text".into()),
        id: Some(id.into()),
        visible: true,
        ..Default::default()
    }
}

fn page(fields: Vec<PageField>) -> Page {
    Page {
        scheme: Some("https".into()),
        host: "shop.example.test".into(),
        fields,
        forms: vec![],
    }
}

fn card_with(rules: &[(&str, &str)]) -> Item {
    let mut fields = vec![json!({ "name": "Note", "value": "hi", "type": 0 })];
    for (id, value) in rules {
        fields.push(json!({
            "name": "keykeeper.fill", "type": 1,
            "rule": { "v": 1, "host": "shop.example.test", "field": { "id": id }, "fill": { "from": "card", "value": value } },
        }));
    }
    item_of(&json!({
        "type": "card", "seesSecrets": true, "codes": true, "fields": fields,
        "card": { "cardholderName": "Ana Lopez", "number": "4111 1111 1111 1111", "expMonth": "7", "expYear": "2029", "code": "123" },
    }))
}

const MAY: May = May {
    see_secrets: true,
    codes: true,
};

#[test]
fn each_value_says_how_its_field_takes_it() {
    let item = card_with(&[
        ("n", "number"),
        ("m", "expMonth"),
        ("y", "expYear"),
        ("yy", "expYear"),
        ("ml", "expMonth"),
        ("yl", "expYear"),
        ("h", "cardholderName"),
        ("hl", "cardholderName"),
    ]);
    let list = |id: &str| PageField {
        tag: Some("select".into()),
        kind: Some("select-one".into()),
        list: true,
        ..field(id)
    };
    let page = page(vec![
        field("n"),
        field("m"),
        field("y"),
        PageField {
            max_length: Some(2),
            ..field("yy")
        },
        list("ml"),
        list("yl"),
        field("h"),
        list("hl"),
    ]);
    let plan = plan(&page, &item, &[], MAY);
    let got: Vec<(usize, &str, Take)> = plan
        .fills
        .iter()
        .map(|f| (f.field, f.what.as_str(), f.take))
        .collect();
    assert_eq!(
        got,
        [
            // Digits only, as Android's card fill writes a number.
            (0, "cardNumber", Take::Digits),
            (1, "expMonth", Take::Month),
            (2, "expYear", Take::Year),
            // Two digits where the field takes only two characters.
            (3, "expYear", Take::ShortYear),
            (4, "expMonth", Take::MonthOption),
            (5, "expYear", Take::YearOption),
            (6, "cardholderName", Take::Text),
            (7, "cardholderName", Take::Option),
        ]
    );
}

#[test]
fn an_identitys_state_and_country_lists_take_them_by_code_or_name() {
    let rule = |id: &str, value: &str| {
        json!({
            "name": "keykeeper.fill", "type": 1,
            "rule": { "v": 1, "host": "shop.example.test", "field": { "id": id }, "fill": { "from": "identity", "value": value } },
        })
    };
    let item = item_of(&json!({
        "type": "identity", "seesSecrets": false, "codes": true,
        "fields": [rule("s", "state"), rule("c", "country"), rule("n", "fullName")],
        "identity": { "title": "Ms", "firstName": "Ana", "lastName": "Lopez", "state": "IL", "country": "US" },
    }));
    let list = |id: &str| PageField {
        list: true,
        ..field(id)
    };
    let plan = plan(
        &page(vec![list("s"), list("c"), field("n")]),
        &item,
        &[],
        MAY,
    );
    let got: Vec<(&str, Take)> = plan
        .fills
        .iter()
        .map(|f| (f.what.as_str(), f.take))
        .collect();
    assert_eq!(
        got,
        [
            ("state", Take::StateOption),
            ("country", Take::CountryOption),
            // Upstream's full name: with the title.
            ("fullName", Take::Text),
        ]
    );
    assert_eq!(
        item.detail("fullName").as_deref().map(String::as_str),
        Some("Ms Ana Lopez")
    );
}

#[test]
fn a_cards_values_only_from_a_card_and_an_identitys_only_from_an_identity_whatever_else_it_carries()
{
    let mut item = card_with(&[("n", "number")]);
    let identity = item_of(&json!({
        "type": "identity", "seesSecrets": true, "codes": true, "fields": [],
        "identity": { "city": "Springfield" },
    }));
    item.identity = identity.identity;
    item.fields.push(Field {
        name: RULE_FIELD.into(),
        value: Zeroizing::new(
            json!({ "v": 1, "host": "shop.example.test", "field": { "id": "c" }, "fill": { "from": "identity", "value": "city" } })
                .to_string(),
        ),
        hidden: true,
        boolean: false,
    });
    let page = page(vec![field("n"), field("c")]);
    assert_eq!(
        plan(&page, &item, &[], MAY).fills.len(),
        1,
        "the card's number only"
    );
    item.kind = ItemKind::Login;
    assert!(plan(&page, &item, &[], MAY).fills.is_empty());
    item.kind = ItemKind::Identity;
    item.card = None;
    let fills = plan(&page, &item, &[], MAY).fills;
    assert_eq!(
        fills.iter().map(|f| f.what.as_str()).collect::<Vec<_>>(),
        ["city"],
        "the identity's city only"
    );
    // A login that carries an identity's values gets none of them from a rule.
    item.kind = ItemKind::Login;
    assert!(plan(&page, &item, &[], MAY).fills.is_empty());
}

#[test]
fn a_custom_field_is_named_by_its_place_on_the_item_and_the_first_with_its_name_fills() {
    let item = item_of(&json!({
        "type": "login", "seesSecrets": true, "codes": true, "username": "ana", "password": "p",
        "fields": [
            { "name": "keykeeper.expires", "value": "2027-01-31", "type": 0 },
            { "name": "Code", "value": "A-1", "type": 0 },
            { "name": "Code", "value": "B-2", "type": 0 },
            { "name": "keykeeper.fill", "type": 1, "rule": { "v": 1, "host": "shop.example.test", "field": { "id": "c" }, "fill": { "from": "field", "name": "Code" } } },
        ],
    }));
    let plan = plan(&page(vec![field("c")]), &item, &[], MAY);
    assert_eq!(
        plan.fills,
        [Fill {
            field: 0,
            what: "field:1".into(),
            take: Take::Text
        }]
    );
}

#[test]
fn a_linked_custom_field_with_its_name_counts_as_a_secret() {
    let item = item_of(&json!({
        "type": "login", "seesSecrets": false, "codes": true, "username": "ana", "password": "p",
        "fields": [
            { "name": "Code", "value": "A-1", "type": 0 },
            { "name": "Code", "type": 3 },
            { "name": "keykeeper.fill", "type": 1, "rule": { "v": 1, "host": "shop.example.test", "field": { "id": "c" }, "fill": { "from": "field", "name": "Code" } } },
        ],
    }));
    // A text field with that name is held back with the linked one, as a secret.
    assert!(plan(&page(vec![field("c")]), &item, &[], MAY_NOT_SEE)
        .fills
        .is_empty());
    let password = PageField {
        kind: Some("password".into()),
        ..field("c")
    };
    assert_eq!(
        plan(&page(vec![password]), &item, &[], MAY_NOT_SEE)
            .fills
            .len(),
        1
    );
}

const MAY_NOT_SEE: May = May {
    see_secrets: false,
    codes: true,
};

#[test]
fn rules_are_read_as_the_extension_reads_them() {
    let read = |v: Value| Rule::read(&v);
    let rule = |field: Value, fill: Value| json!({ "v": 1, "host": "a.test", "field": field, "fill": fill });
    assert_eq!(
        read(rule(
            json!({ "id": "x", "name": "n" }),
            json!({ "from": "password" })
        )),
        Some(Rule {
            host: Some("a.test".into()),
            scheme: None,
            field: Fingerprint::Id("x".into()),
            fill: Source::Password,
        })
    );
    // What decides is the first that's there, as JavaScript counts it.
    for (field, decides) in [
        (
            json!({ "id": "", "name": "n" }),
            Some(Fingerprint::Name("n".into())),
        ),
        (
            json!({ "id": 0, "name": "n" }),
            Some(Fingerprint::Name("n".into())),
        ),
        (
            json!({ "id": null, "label": "l" }),
            Some(Fingerprint::Label("l".into())),
        ),
        // One that's there but isn't text matches no field.
        (json!({ "id": 5, "name": "n" }), None),
        (json!({ "name": ["n"] }), None),
        (json!({}), None),
    ] {
        assert_eq!(
            read(rule(field.clone(), json!({ "from": "never" }))).map(|r| r.field),
            decides,
            "{field}"
        );
    }
    for value in CARD_VALUES {
        assert_eq!(
            read(rule(
                json!({ "id": "x" }),
                json!({ "from": "card", "value": value })
            ))
            .map(|r| r.fill),
            Some(Source::Card(value))
        );
    }
    for value in IDENTITY_VALUES {
        assert_eq!(
            read(rule(
                json!({ "id": "x" }),
                json!({ "from": "identity", "value": value })
            ))
            .map(|r| r.fill),
            Some(Source::Identity(value))
        );
    }
    for bad in [
        json!({ "v": 2, "field": { "id": "x" }, "fill": { "from": "password" } }),
        json!({ "v": 1, "field": "x", "fill": { "from": "password" } }),
        json!({ "v": 1, "field": { "id": "x" }, "fill": "password" }),
        json!({ "v": 1, "host": 7, "field": { "id": "x" }, "fill": { "from": "password" } }),
        json!({ "v": 1, "scheme": "ftp", "field": { "id": "x" }, "fill": { "from": "password" } }),
        json!({ "v": 1, "field": { "id": "x" }, "fill": { "from": "card", "value": "pin" } }),
        json!({ "v": 1, "field": { "id": "x" }, "fill": { "from": "identity", "value": "ssn" } }),
        json!({ "v": 1, "field": { "id": "x" }, "fill": { "from": "field" } }),
        json!({ "v": 1, "field": { "id": "x" }, "fill": { "from": "everything" } }),
    ] {
        assert_eq!(read(bad.clone()), None, "{bad}");
    }
    assert_eq!(Rule::parse("not json"), None);
}

#[test]
fn the_site_wide_list_holds_only_never_rules_with_a_site() {
    assert!(never_fill_list("not json").is_empty());
    assert!(never_fill_list(r#"{"v":1}"#).is_empty());
    let list = never_fill_list(
        &json!([
            { "v": 1, "host": "a.test", "field": { "id": "x" }, "fill": { "from": "never" } },
            { "v": 1, "field": { "id": "x" }, "fill": { "from": "never" } },
            { "v": 1, "host": "", "field": { "id": "x" }, "fill": { "from": "never" } },
            { "v": 1, "host": "a.test", "field": { "id": "x" }, "fill": { "from": "password" } },
            "junk",
        ])
        .to_string(),
    );
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].host.as_deref(), Some("a.test"));
}

#[test]
fn the_pages_host_is_read_in_any_case_and_without_its_last_dot() {
    let item = card_with(&[("n", "number")]);
    let mut page = page(vec![field("n")]);
    page.host = "Shop.Example.TEST.".into();
    assert_eq!(plan(&page, &item, &[], MAY).fills.len(), 1);
    page.scheme = Some("HTTPS".into());
    assert_eq!(plan(&page, &item, &[], MAY).fills.len(), 1);
    page.scheme = Some("ftp".into());
    assert!(plan(&page, &item, &[], MAY).fills.is_empty());
}

#[test]
fn the_plan_goes_to_the_app_as_json_with_names_of_values_only() {
    let item = card_with(&[("n", "number"), ("m", "expMonth")]);
    let never = never_fill_list(
        &json!([{ "v": 1, "host": "shop.example.test", "field": { "id": "m" }, "fill": { "from": "never" } }])
            .to_string(),
    );
    let plan = plan(&page(vec![field("n"), field("m")]), &item, &never, MAY);
    let text = serde_json::to_string(&plan).unwrap();
    assert_eq!(
        text,
        r#"{"fills":[{"field":0,"what":"cardNumber","take":"digits"}],"never":[1]}"#
    );
    assert!(!text.contains("4111"));
}

#[test]
fn a_page_comes_from_the_app_as_json() {
    // As the app writes it (FillRulesTest.kt, in app/android/app/src/test).
    let page: Page = serde_json::from_str(
        r#"{"scheme":"https","host":"a.test","fields":[{"tag":"input","type":"text","id":"i","name":"n","label":"L","ariaLabel":"A","placeholder":"P","autocomplete":"email","visible":true,"list":false,"maxLength":4,"form":0}],"forms":["f"]}"#,
    )
    .unwrap();
    let f = &page.fields[0];
    assert_eq!(
        (
            f.kind.as_deref(),
            f.aria_label.as_deref(),
            f.max_length,
            f.form
        ),
        (Some("text"), Some("A"), Some(4), Some(0))
    );
    assert_eq!(f.shown_label(), "l");
    // What it leaves out is none, and a field it doesn't say is visible isn't.
    let page: Page = serde_json::from_str(r#"{"host":"a.test","fields":[{}]}"#).unwrap();
    assert_eq!(page.scheme, None);
    assert!(!page.fields[0].visible);
    // And what the app sends for what a page leaves out, its words as they are.
    let page: Page = serde_json::from_str(
        r#"{"scheme":null,"host":"a.test","fields":[{"tag":null,"type":null,"id":null,"name":null,"label":"Say \"hi\"\\\u000a\u0009there\u2028","ariaLabel":null,"placeholder":null,"autocomplete":null,"visible":false,"list":true,"maxLength":null,"form":null}],"forms":[null]}"#,
    )
    .unwrap();
    assert_eq!(page.forms, [None]);
    let f = &page.fields[0];
    assert_eq!(f.label.as_deref(), Some("Say \"hi\"\\\n\tthere\u{2028}"));
    assert_eq!((f.max_length, f.form, f.list), (None, None, true));
    assert_eq!(f.shown_label(), "say \"hi\"\\ there");
}
