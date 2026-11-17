//! 1Password's `.1pux` and LastPass's CSV (meta repo DESIGN.md, "Archive,
//! export, and import in the apps"), read as our fork's web vault reads them
//! (`OnePassword1PuxImporter`, `LastPassCsvImporter`, and their specs), from
//! small made-up fixtures: a `.1pux` is built here, a zip in memory, from
//! JSON, one item of each category and each oddity the specs test; LastPass
//! files are a few lines of CSV. Then one of each saved, encrypted, against
//! a stand-in for the server.

use std::io::Write as _;

use passbundle_core::app::{App, Destination, ImportRead};
use passbundle_core::error::Error;
use passbundle_core::import::{self, Format, Item, LeftOut, Parsed, Read};
use serde_json::{json, Value};

#[path = "support/stand_in.rs"]
mod stand_in;
use stand_in::{open, sent, stand_in, user_key, Sample};

// ---------------------------------------------------------------------------
// Reading.

fn read(app: &App, format: Format, file: &[u8]) -> Parsed {
    match import::read(format, file, None, app.vault().unwrap()).unwrap() {
        Read::Parsed(parsed) => parsed,
        _ => panic!("not read"),
    }
}

fn refusal(app: &App, format: Format, file: &[u8]) -> &'static str {
    match import::read(format, file, None, app.vault().unwrap()) {
        Err(Error::Refused(why)) => why,
        other => panic!("read: {:?}", other.is_ok()),
    }
}

fn app() -> (Sample, App) {
    let (url, _) = stand_in(vec![]);
    let sample = Sample::new(&url, &[], |_| {});
    let app = sample.unlocked();
    (sample, app)
}

fn text(v: &Option<zeroize::Zeroizing<String>>) -> Option<&str> {
    v.as_deref().map(String::as_str)
}

/// Each field's name and value, in order.
fn fields(item: &Item) -> Vec<(&str, &str)> {
    item.fields
        .iter()
        .map(|f| {
            (
                f.name.as_deref().unwrap_or(""),
                text(&f.value).unwrap_or(""),
            )
        })
        .collect()
}

fn field_kind(item: &Item, name: &str) -> u8 {
    item.fields
        .iter()
        .find(|f| f.name.as_deref() == Some(name))
        .unwrap()
        .kind
}

// ---------------------------------------------------------------------------
// 1Password's .1pux, made here.

/// A section field, as 1Password writes one.
fn field(title: &str, id: &str, value: Value) -> Value {
    json!({
        "title": title, "id": id, "value": value, "guarded": false, "multiline": false,
        "dontGenerate": false,
        "inputTraits": { "keyboard": "default", "correction": "default", "capitalization": "default" },
    })
}

fn section(title: &str, fields: Vec<Value>) -> Value {
    json!({ "title": title, "name": format!("Section_{}", title.len()), "fields": fields })
}

/// An item of a category, with its title, its details, and anything else
/// (overview, favIndex, state) merged over.
fn item(category: &str, title: &str, details: Value, extra: Value) -> Value {
    let mut item = json!({
        "uuid": format!("made-up-{category}-{}", title.len()),
        "favIndex": 0, "createdAt": 1_700_000_000, "updatedAt": 1_700_000_100,
        "state": "active", "categoryUuid": category,
        "details": details,
        "overview": { "subtitle": "", "title": title, "url": "", "ps": 0, "pbe": 0.0, "pgrng": false },
    });
    if let Value::Object(extra) = extra {
        for (k, v) in extra {
            if k == "overview" {
                for (ok, ov) in v.as_object().unwrap() {
                    item["overview"][ok] = ov.clone();
                }
            } else {
                item[k] = v;
            }
        }
    }
    item
}

/// A vault by its name, with its items.
type Vault<'a> = (&'a str, Vec<Value>);

/// One account's export, its vaults by name.
fn export(vaults: Vec<Vault>) -> Value {
    accounts(vec![("Test Person", vaults)])
}

/// Accounts by name, with their vaults.
fn accounts(accounts: Vec<(&str, Vec<Vault>)>) -> Value {
    json!({
        "accounts": accounts.into_iter().map(|(name, vaults)| json!({
            "attrs": { "accountName": name, "name": name, "avatar": "", "email": "test@example.test",
                       "uuid": "MADEUPACCOUNT", "domain": "https://example.test/" },
            "vaults": vaults.into_iter().map(|(vault, items)| json!({
                "attrs": { "uuid": format!("vault-{vault}"), "desc": "", "avatar": "", "name": vault, "type": "U" },
                "items": items,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

/// A `.1pux`: export.attributes, export.data, and files/ with `files` in it.
fn onepux(data: &Value, files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("export.attributes", options).unwrap();
    zip.write_all(
        br#"{"version":3,"description":"1Password Unencrypted Export","createdAt":1700000000}"#,
    )
    .unwrap();
    zip.start_file("export.data", options).unwrap();
    zip.write_all(data.to_string().as_bytes()).unwrap();
    zip.add_directory("files/", options).unwrap();
    for (name, body) in files {
        zip.start_file(format!("files/{name}"), options).unwrap();
        zip.write_all(body).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn one(app: &App, category: &str, title: &str, details: Value) -> Item {
    let data = export(vec![(
        "Personal",
        vec![item(category, title, details, json!({}))],
    )]);
    let mut parsed = read(app, Format::OnePassword1pux, &onepux(&data, &[]));
    assert_eq!(parsed.items.len(), 1);
    parsed.items.remove(0)
}

#[test]
fn a_login_with_its_websites_code_fields_history_and_folder() {
    let (_sample, app) = app();
    let details = json!({
        "loginFields": [
            { "value": "ana@example.test", "id": "", "name": "email", "fieldType": "E", "designation": "username" },
            { "value": "made-up-login-password", "id": "", "name": "password", "fieldType": "P", "designation": "password" },
            { "value": "", "id": "terms", "name": "terms", "fieldType": "C" },
            { "value": "✓", "id": "remember", "name": "remember", "fieldType": "C" },
            { "value": "made-up-pin", "id": "pin", "name": "PIN", "fieldType": "P" },
        ],
        "notesPlain": "The login's own notes.\r\nSecond line.\n",
        "sections": [section("Saved on login.example.test", vec![
            field("Account number", "acct", json!({ "string": "12345" })),
            field("one-time password", "TOTP_madeup", json!({ "totp": "otpauth://totp/Example:ana?secret=JBSWY3DPEHPK3PXP" })),
            field("Backup codes", "codes", json!({ "string": "11111\n22222\n33333" })),
            field("", "untitled", json!({ "string": "named after its section" })),
            field("Empty", "empty", json!({ "string": "" })),
        ])],
        "passwordHistory": [
            { "value": "made-up-older", "time": 1_600_000_000 },
            { "value": "made-up-newest", "time": 1_650_000_000_000_i64 },
            { "value": " ", "time": 1_610_000_000 },
            { "value": "made-up-oldest", "time": 1_500_000_000 },
        ],
    });
    let extra = json!({
        "favIndex": 2,
        "overview": {
            "urls": [
                { "label": "website", "url": "https://login.example.test/" },
                { "label": "", "url": "second.example.test" },
                { "label": "", "url": "  " },
            ],
            "tags": ["  example  tag", "second tag"],
        },
    });
    let data = export(vec![(
        "Personal",
        vec![item("001", "Example Login", details, extra)],
    )]);
    let parsed = read(&app, Format::OnePassword1pux, &onepux(&data, &[]));
    let login = &parsed.items[0];
    assert_eq!(login.kind, 1);
    assert_eq!(login.name, "Example Login");
    // The second favorite is a favorite too.
    assert!(login.favorite);
    assert_eq!(login.username.as_deref(), Some("ana@example.test"));
    assert_eq!(text(&login.password), Some("made-up-login-password"));
    assert_eq!(
        text(&login.totp),
        Some("otpauth://totp/Example:ana?secret=JBSWY3DPEHPK3PXP")
    );
    let uris: Vec<&str> = login.uris.iter().map(|(u, _)| u.as_str()).collect();
    assert_eq!(
        uris,
        ["https://login.example.test/", "http://second.example.test"]
    );
    // A checkbox is yes or no, a password a hidden field, a field with no
    // title its section's name, and an empty one nothing.
    assert_eq!(
        fields(login),
        [
            ("terms", "false"),
            ("remember", "true"),
            ("PIN", "made-up-pin"),
            ("Account number", "12345"),
            ("Saved on login.example.test", "named after its section"),
        ]
    );
    assert_eq!(field_kind(login, "terms"), 2);
    assert_eq!(field_kind(login, "PIN"), 1);
    assert_eq!(field_kind(login, "Account number"), 0);
    // A value on more than one line goes to the notes, after the item's own.
    assert_eq!(
        text(&login.notes),
        Some("The login's own notes.\nSecond line.\n\nBackup codes: 11111\n22222\n33333")
    );
    // Newest first, at most as many as there were, a time in seconds or in
    // milliseconds, and the blank one left out.
    let history: Vec<(&str, Option<&str>)> = login
        .history
        .iter()
        .map(|(p, when)| (p.as_str(), when.as_deref()))
        .collect();
    assert_eq!(
        history,
        [
            ("made-up-newest", Some("2022-04-15T05:20:00.000Z")),
            ("made-up-older", Some("2020-09-13T12:26:40.000Z")),
            ("made-up-oldest", Some("2017-07-14T02:40:00.000Z")),
        ]
    );
    // The first tag is the folder, with each word's first letter a capital.
    assert_eq!(parsed.folders, ["Example  Tag"]);
    assert_eq!(login.folder, Some(0));
    assert!(login.archived_date.is_none());
    assert_eq!(login.reprompt, 0);
}

#[test]
fn an_archived_item_and_a_guarded_field() {
    let (_sample, app) = app();
    let mut guarded = field("PIN", "pin", json!({ "concealed": "1234" }));
    guarded["guarded"] = true.into();
    let data = export(vec![(
        "Personal",
        vec![
            item(
                "003",
                "Locker",
                json!({ "notesPlain": "Gym.", "sections": [section("", vec![guarded])] }),
                json!({ "state": "archived", "favIndex": 1 }),
            ),
            item("003", "", json!({ "notesPlain": "  " }), json!({})),
        ],
    )]);
    let parsed = read(&app, Format::OnePassword1pux, &onepux(&data, &[]));
    let note = &parsed.items[0];
    assert_eq!(note.kind, 2);
    assert!(note.archived_date.is_some());
    assert!(note.favorite);
    assert_eq!(fields(note), [("PIN", "1234")]);
    assert_eq!(field_kind(note, "PIN"), 1);
    // A guarded field asks for the master password for the whole item.
    assert_eq!(note.reprompt, 1);
    // No name is "--", and blank notes are none.
    assert_eq!(parsed.items[1].name, "--");
    assert_eq!(parsed.items[1].notes, None);
}

#[test]
fn a_card_and_a_bank_account_are_cards() {
    let (_sample, app) = app();
    let card = one(
        &app,
        "002",
        "Test Discover",
        json!({
            "notesPlain": "Not a real card.",
            "sections": [
                section("", vec![
                    field("cardholder name", "cardholder", json!({ "string": "Test Person" })),
                    field("type", "type", json!({ "cardType": "discover" })),
                    field("number", "ccnum", json!({ "creditCardNumber": "6011111111111117" })),
                    field("verification number", "cvv", json!({ "concealed": "1312" })),
                    field("expiry date", "expiry", json!({ "monthYear": 209_912 })),
                    field("valid from", "validFrom", json!({ "monthYear": 200_101 })),
                ]),
                section("Contact Information", vec![
                    field("issuing bank", "bank", json!({ "string": "Example Bank" })),
                    field("", "phoneLocal", json!({ "phone": "555-0100" })),
                ]),
            ],
        }),
    );
    assert_eq!(card.kind, 3);
    let c = card.card.as_ref().unwrap();
    assert_eq!(c.cardholder_name.as_deref(), Some("Test Person"));
    assert_eq!(text(&c.number), Some("6011111111111117"));
    assert_eq!(c.brand.as_deref(), Some("Discover"));
    assert_eq!(text(&c.code), Some("1312"));
    assert_eq!(c.exp_month.as_deref(), Some("12"));
    assert_eq!(c.exp_year.as_deref(), Some("2099"));
    assert_eq!(
        fields(&card),
        [
            ("valid from", "200101"),
            ("issuing bank", "Example Bank"),
            ("Contact Information", "555-0100"),
        ]
    );
    // A month with a zero in front loses it.
    let early = one(
        &app,
        "002",
        "Test Visa",
        json!({ "sections": [section("", vec![
            field("number", "ccnum", json!({ "creditCardNumber": "4111111111111111" })),
            field("expiry date", "expiry", json!({ "monthYear": 203_007 })),
        ])] }),
    );
    let c = early.card.as_ref().unwrap();
    assert_eq!(c.brand.as_deref(), Some("Visa"));
    assert_eq!(c.exp_month.as_deref(), Some("7"));

    let bank = one(
        &app,
        "101",
        "Bank Account",
        json!({
            "notesPlain": "My made-up account.",
            "sections": [section("", vec![
                field("bank name", "bankName", json!({ "string": "Example Credit Union" })),
                field("name on account", "owner", json!({ "string": "Test Person" })),
                field("routing number", "routingNo", json!({ "string": "000000000" })),
                field("account number", "accountNo", json!({ "string": "1234567890" })),
                field("PIN", "telephonePin", json!({ "concealed": "5555" })),
            ])],
        }),
    );
    assert_eq!(bank.kind, 3);
    assert_eq!(
        bank.card.as_ref().unwrap().cardholder_name.as_deref(),
        Some("Test Person")
    );
    assert_eq!(
        fields(&bank),
        [
            ("bank name", "Example Credit Union"),
            ("routing number", "000000000"),
            ("account number", "1234567890"),
            ("PIN", "5555"),
        ]
    );
}

#[test]
fn an_identity_with_its_address_email_and_dates() {
    let (_sample, app) = app();
    let identity = one(
        &app,
        "004",
        "Test Person",
        json!({ "sections": [
            section("Identification", vec![
                field("first name", "firstname", json!({ "string": "Test" })),
                field("initial", "initial", json!({ "string": "Q" })),
                field("last name", "lastname", json!({ "string": "Person" })),
                field("sex", "sex", json!({ "menu": "female" })),
                field("birth date", "birthdate", json!({ "date": 347_198_460 })),
                field("occupation", "occupation", json!({ "string": "Tester" })),
                field("company", "company", json!({ "string": "Example Corp" })),
            ]),
            section("Address", vec![
                field("address", "address", json!({ "address": {
                    "street": "1 Example Street", "city": "Exampleville", "country": "us",
                    "zip": "00000", "state": "Example State",
                } })),
                field("default phone", "defphone", json!({ "phone": "555-0100" })),
            ]),
            section("Internet Details", vec![
                field("email", "email", json!({ "email": { "email_address": "test@example.test", "provider": "Example Mail" } })),
                field("2nd email", "email2", json!({ "email": { "email_address": "second@example.test", "provider": null } })),
                field("username", "username", json!({ "string": "tperson" })),
            ]),
        ] }),
    );
    assert_eq!(identity.kind, 4);
    let i = identity.identity.as_ref().unwrap();
    assert_eq!(i.first_name.as_deref(), Some("Test"));
    assert_eq!(i.middle_name.as_deref(), Some("Q"));
    assert_eq!(i.last_name.as_deref(), Some("Person"));
    assert_eq!(i.company.as_deref(), Some("Example Corp"));
    assert_eq!(i.address1.as_deref(), Some("1 Example Street"));
    assert_eq!(i.city.as_deref(), Some("Exampleville"));
    // The country in capitals.
    assert_eq!(i.country.as_deref(), Some("US"));
    assert_eq!(i.postal_code.as_deref(), Some("00000"));
    assert_eq!(i.state.as_deref(), Some("Example State"));
    assert_eq!(i.phone.as_deref(), Some("555-0100"));
    assert_eq!(i.email.as_deref(), Some("test@example.test"));
    assert_eq!(i.username.as_deref(), Some("tperson"));
    // A date as the web vault writes one; the first email's provider, then a
    // second email, are fields.
    assert_eq!(
        fields(&identity),
        [
            ("sex", "female"),
            ("birth date", "Thu, 01 Jan 1981 12:01:00 GMT"),
            ("occupation", "Tester"),
            ("provider", "Example Mail"),
            ("2nd email", "second@example.test"),
        ]
    );
}

#[test]
fn identities_from_licenses_memberships_passports_rewards_and_ssns() {
    let (_sample, app) = app();
    let license = one(
        &app,
        "103",
        "Driver's license",
        json!({ "sections": [section("", vec![
            field("full name", "fullname", json!({ "string": "Test Middle Person Jr" })),
            field("address", "address", json!({ "string": "1 Example Street" })),
            field("date of birth", "birthdate", json!({ "date": 252_504_060 })),
            field("country", "country", json!({ "string": "United States" })),
            field("state", "state", json!({ "string": "Example State" })),
            field("number", "number", json!({ "string": "D1234567" })),
            field("expiry date", "expiry_date", json!({ "monthYear": 203_012 })),
        ])] }),
    );
    assert_eq!(license.kind, 4);
    let i = license.identity.as_ref().unwrap();
    assert_eq!(
        (
            i.first_name.as_deref(),
            i.middle_name.as_deref(),
            i.last_name.as_deref()
        ),
        (Some("Test"), Some("Middle"), Some("Person Jr"))
    );
    assert_eq!(i.address1.as_deref(), Some("1 Example Street"));
    assert_eq!(i.country.as_deref(), Some("United States"));
    assert_eq!(i.state.as_deref(), Some("Example State"));
    assert_eq!(text(&i.license_number), Some("D1234567"));
    assert_eq!(
        fields(&license),
        [
            ("date of birth", "Sun, 01 Jan 1978 12:01:00 GMT"),
            ("expiry date", "203012")
        ]
    );

    let outdoor = one(
        &app,
        "104",
        "Fishing license",
        json!({ "sections": [section("", vec![
            field("full name", "name", json!({ "string": "Test Person" })),
            field("country", "country", json!({ "string": "Example Land" })),
            field("state", "state", json!({ "string": "Example State" })),
            field("approved wildlife", "game", json!({ "string": "Trout" })),
        ])] }),
    );
    let i = outdoor.identity.as_ref().unwrap();
    assert_eq!(
        (
            i.first_name.as_deref(),
            i.middle_name.as_deref(),
            i.last_name.as_deref()
        ),
        (Some("Test"), None, Some("Person"))
    );
    assert_eq!(i.state.as_deref(), Some("Example State"));
    assert_eq!(fields(&outdoor), [("approved wildlife", "Trout")]);

    let membership = one(
        &app,
        "105",
        "Library card",
        json!({ "sections": [section("", vec![
            field("member name", "member_name", json!({ "string": "Test Person" })),
            field("group", "org_name", json!({ "string": "Example Library" })),
            field("telephone", "phone", json!({ "phone": "555-0101" })),
            field("member ID", "membership_no", json!({ "string": "L-42" })),
        ])] }),
    );
    let i = membership.identity.as_ref().unwrap();
    assert_eq!(i.company.as_deref(), Some("Example Library"));
    assert_eq!(i.phone.as_deref(), Some("555-0101"));
    assert_eq!(fields(&membership), [("member ID", "L-42")]);

    let passport = one(
        &app,
        "106",
        "Passport",
        json!({ "sections": [section("", vec![
            field("type", "type", json!({ "string": "Made-up passport" })),
            field("full name", "fullname", json!({ "string": "Test Person" })),
            field("issuing country", "issuing_country", json!({ "string": "Example Land" })),
            field("number", "number", json!({ "string": "P1234567" })),
        ])] }),
    );
    let i = passport.identity.as_ref().unwrap();
    assert_eq!(i.country.as_deref(), Some("Example Land"));
    assert_eq!(text(&i.passport_number), Some("P1234567"));
    assert_eq!(fields(&passport), [("type", "Made-up passport")]);

    let rewards = one(
        &app,
        "107",
        "Rewards",
        json!({ "sections": [section("", vec![
            field("company name", "company_name", json!({ "string": "Example Store" })),
            field("member name", "member_name", json!({ "string": "Test Person" })),
            field("member ID", "membership_no", json!({ "string": "R-7" })),
        ])] }),
    );
    let i = rewards.identity.as_ref().unwrap();
    assert_eq!(i.company.as_deref(), Some("Example Store"));
    assert_eq!(i.first_name.as_deref(), Some("Test"));
    assert_eq!(fields(&rewards), [("member ID", "R-7")]);

    let ssn = one(
        &app,
        "108",
        "SSN",
        json!({ "sections": [section("", vec![
            field("name", "name", json!({ "string": "Test Person" })),
            field("number", "number", json!({ "concealed": "000-00-0000" })),
        ])] }),
    );
    let i = ssn.identity.as_ref().unwrap();
    assert_eq!(text(&i.ssn), Some("000-00-0000"));
    assert_eq!(i.last_name.as_deref(), Some("Person"));
    assert!(ssn.fields.is_empty());
}

#[test]
fn passwords_databases_servers_and_api_credentials_are_logins() {
    let (_sample, app) = app();
    let password = item(
        "005",
        "Made-up password",
        json!({ "password": "made-up-legacy-password", "notesPlain": "Legacy." }),
        json!({ "overview": { "url": "https://legacy.example.test", "urls": [{ "label": "", "url": "https://legacy.example.test" }] } }),
    );
    let data = export(vec![("Personal", vec![password])]);
    let parsed = read(&app, Format::OnePassword1pux, &onepux(&data, &[]));
    let password = &parsed.items[0];
    assert_eq!(password.kind, 1);
    assert_eq!(text(&password.password), Some("made-up-legacy-password"));
    assert_eq!(password.uris[0].0, "https://legacy.example.test");

    let database = one(
        &app,
        "102",
        "Database",
        json!({ "sections": [section("", vec![
            field("type", "database_type", json!({ "menu": "postgresql" })),
            field("server", "hostname", json!({ "string": "db.example.test" })),
            field("port", "port", json!({ "string": "5432" })),
            field("username", "username", json!({ "string": "dbuser" })),
            field("password", "password", json!({ "concealed": "made-up-db-password" })),
        ])] }),
    );
    assert_eq!(database.kind, 1);
    assert_eq!(database.username.as_deref(), Some("dbuser"));
    assert_eq!(text(&database.password), Some("made-up-db-password"));
    assert_eq!(
        fields(&database),
        [
            ("type", "postgresql"),
            ("server", "db.example.test"),
            ("port", "5432")
        ]
    );

    let server = one(
        &app,
        "110",
        "Server",
        json!({ "sections": [
            section("", vec![
                field("URL", "url", json!({ "string": "https://server.example.test" })),
                field("username", "username", json!({ "string": "admin" })),
                field("password", "password", json!({ "concealed": "made-up-server-password" })),
            ]),
            section("Admin Console", vec![
                field("admin console URL", "admin_console_url", json!({ "string": "https://server.example.test/admin" })),
                field("console password", "admin_console_password", json!({ "concealed": "made-up-console-password" })),
            ]),
        ] }),
    );
    assert_eq!(server.kind, 1);
    assert_eq!(server.uris[0].0, "https://server.example.test");
    assert_eq!(server.username.as_deref(), Some("admin"));
    assert_eq!(
        fields(&server),
        [
            ("admin console URL", "https://server.example.test/admin"),
            ("console password", "made-up-console-password"),
        ]
    );
    assert_eq!(field_kind(&server, "console password"), 1);

    let api = one(
        &app,
        "112",
        "API credential",
        json!({ "sections": [section("", vec![
            field("username", "username", json!({ "string": "api@example.test" })),
            field("credential", "credential", json!({ "concealed": "made-up-api-credential" })),
            field("type", "type", json!({ "menu": "jwt" })),
            field("hostname", "hostname", json!({ "string": "api.example.test" })),
            field("expires", "expires", json!({ "date": 1_932_552_060 })),
        ])] }),
    );
    assert_eq!(api.kind, 1);
    assert_eq!(text(&api.password), Some("made-up-api-credential"));
    assert_eq!(api.uris[0].0, "http://api.example.test");
    assert_eq!(
        fields(&api),
        [
            ("type", "jwt"),
            ("expires", "Sat, 29 Mar 2031 12:01:00 GMT")
        ]
    );
}

#[test]
fn notes_from_routers_licenses_email_accounts_medical_records_and_documents() {
    let (_sample, app) = app();
    // A router has nothing to sign in with: a note, its passwords fields.
    let router = one(
        &app,
        "109",
        "Router",
        json!({ "notesPlain": "Made-up router.", "sections": [section("", vec![
            field("base station name", "name", json!({ "string": "Example Router" })),
            field("base station password", "password", json!({ "concealed": "made-up-router-password" })),
            field("network name", "network_name", json!({ "string": "ExampleNet" })),
        ])] }),
    );
    assert_eq!(router.kind, 2);
    assert_eq!(
        fields(&router),
        [
            ("base station name", "Example Router"),
            ("base station password", "made-up-router-password"),
            ("network name", "ExampleNet"),
        ]
    );
    for category in ["100", "111", "113"] {
        let note = one(
            &app,
            category,
            "Note",
            json!({ "notesPlain": "Made up.", "sections": [section("", vec![
                field("port number", "pop_port", json!({ "string": "587" })),
                field("port number", "smtp_port", json!({ "string": "589" })),
            ])] }),
        );
        assert_eq!(note.kind, 2, "{category}");
        // Two fields of one name stay two.
        assert_eq!(
            fields(&note),
            [("port number", "587"), ("port number", "589")]
        );
    }
    // A document's file isn't imported; the item is a note.
    let document = one(
        &app,
        "006",
        "Made-up scan",
        json!({ "notesPlain": "The scan.", "documentAttributes": {
            "fileName": "scan.pdf", "documentId": "madeupdoc", "decryptedSize": 12,
        } }),
    );
    assert_eq!(document.kind, 2);
    assert_eq!(text(&document.notes), Some("The scan."));
}

/// The RFC 8032 test key (7.1, TEST 1), as 1Password keeps a key: PKCS#8.
fn made_up_ssh_key() -> String {
    use base64::Engine;
    let mut der = Vec::new();
    let hex = "302e020100300506032b6570042204209d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
    for i in (0..hex.len()).step_by(2) {
        der.push(u8::from_str_radix(&hex[i..i + 2], 16).unwrap());
    }
    let body = base64::engine::general_purpose::STANDARD.encode(der);
    format!("-----BEGIN PRIVATE KEY-----\n{body}\n-----END PRIVATE KEY-----\n")
}

fn ssh_item(title: &str, private_key: &str, tags: Value) -> Value {
    item(
        "114",
        title,
        json!({ "notesPlain": "SSH key note.", "sections": [section("", vec![
            field("private key", "private_key", json!({ "sshKey": {
                "privateKey": "not used",
                "metadata": {
                    "privateKey": private_key,
                    "publicKey": "ssh-ed25519 not used", "fingerprint": "not used", "keyType": "ed25519",
                },
            } })),
        ])] }),
        json!({ "overview": { "tags": tags } }),
    )
}

#[test]
fn an_ssh_key_comes_in_as_opensshs_and_one_that_cant_be_read_is_left_out() {
    let (_sample, app) = app();
    let data = export(vec![(
        "Personal",
        vec![
            ssh_item("Made-up SSH key", &made_up_ssh_key(), json!([])),
            ssh_item(
                "Unreadable key",
                "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n",
                json!(["Work"]),
            ),
            item(
                "001",
                "After it",
                json!({ "loginFields": [
                { "value": "ana", "id": "", "name": "username", "fieldType": "T", "designation": "username" },
            ] }),
                json!({}),
            ),
            // No details at all: the web vault skips it too.
            json!({ "uuid": "broken", "categoryUuid": "001", "favIndex": 0, "state": "active",
                    "overview": { "title": "Broken" } }),
        ],
    )]);
    let parsed = read(&app, Format::OnePassword1pux, &onepux(&data, &[]));
    let names: Vec<&str> = parsed.items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["Made-up SSH key", "After it"]);
    let key = parsed.items[0].ssh_key.as_ref().unwrap();
    assert_eq!(parsed.items[0].kind, 5);
    assert!(text(&key.private_key)
        .unwrap()
        .starts_with("-----BEGIN OPENSSH PRIVATE KEY-----\n"));
    assert_eq!(
        key.public_key.as_deref(),
        Some("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINdamAGCsQq31Uv+08lkBzoO4XLz2qYjJa8CGmj3B1Ea")
    );
    assert_eq!(
        key.fingerprint.as_deref(),
        Some("SHA256:bbXpuKG6zhzdmnxq256TlqzFBzRl2f6OOg722cYNbU8")
    );
    assert_eq!(text(&parsed.items[0].notes), Some("SSH key note."));
    // The key left out takes its folder with it.
    assert!(parsed.folders.is_empty());
    assert_eq!(parsed.items[1].folder, None);
    assert!(parsed.left_out.contains(&(LeftOut::SshKeyUnreadable, 1)));
    assert!(parsed.left_out.contains(&(LeftOut::Unreadable, 1)));
}

#[test]
fn every_vault_is_a_folder_when_theres_more_than_one_with_its_tags_inside() {
    let (_sample, app) = app();
    let tagged = |title: &str, tags: Value| {
        item(
            "003",
            title,
            json!({ "notesPlain": "x" }),
            json!({ "overview": { "tags": tags } }),
        )
    };
    let data = export(vec![
        (
            "Personal",
            vec![tagged("A", json!(["finance"])), tagged("B", json!([]))],
        ),
        (
            "Work",
            vec![
                tagged("C", json!(["Work/Servers"])),
                tagged("D", json!(["work"])),
            ],
        ),
    ]);
    let parsed = read(&app, Format::OnePassword1pux, &onepux(&data, &[]));
    let folder = |i: usize| parsed.items[i].folder.map(|f| parsed.folders[f].as_str());
    assert_eq!(folder(0), Some("Personal/Finance"));
    assert_eq!(folder(1), Some("Personal"));
    assert_eq!(folder(2), Some("Work/Servers"));
    assert_eq!(folder(3), Some("Work"));

    // With one vault, the folders are the tags, as the web vault has them.
    let data = export(vec![(
        "Personal",
        vec![tagged("A", json!(["finance"])), tagged("B", json!([]))],
    )]);
    let parsed = read(&app, Format::OnePassword1pux, &onepux(&data, &[]));
    assert_eq!(parsed.folders, ["Finance"]);
    assert_eq!(parsed.items[1].folder, None);

    // Every account, each by its name, when there's more than one.
    let data = accounts(vec![
        (
            "Test Person",
            vec![("Private", vec![tagged("A", json!([]))])],
        ),
        (
            "Example Team",
            vec![("Private", vec![tagged("B", json!(["servers"]))])],
        ),
    ]);
    let parsed = read(&app, Format::OnePassword1pux, &onepux(&data, &[]));
    assert_eq!(parsed.items.len(), 2);
    let folder = |i: usize| parsed.items[i].folder.map(|f| parsed.folders[f].as_str());
    assert_eq!(folder(0), Some("Test Person/Private"));
    assert_eq!(folder(1), Some("Example Team/Private/Servers"));
}

#[test]
fn attachments_arent_imported_and_the_summary_says_how_many() {
    let (_sample, app) = app();
    let data = export(vec![(
        "Personal",
        vec![item(
            "003",
            "Note",
            json!({ "notesPlain": "With two files." }),
            json!({}),
        )],
    )]);
    let file = onepux(
        &data,
        &[
            ("madeup1__scan.pdf", b"%PDF made up"),
            ("madeup2__photo.jpg", b"made up"),
        ],
    );
    let parsed = read(&app, Format::OnePassword1pux, &file);
    assert_eq!(parsed.items.len(), 1);
    assert_eq!(parsed.left_out, [(LeftOut::Attachment, 2)]);

    // export.data by itself (the web vault takes it too): the files it names.
    let data = export(vec![(
        "Personal",
        vec![item(
            "006",
            "Scan",
            json!({
                "documentAttributes": { "fileName": "scan.pdf", "documentId": "d1", "decryptedSize": 3 },
                "sections": [section("", vec![field("receipt", "f", json!({ "file": {
                    "fileName": "receipt.pdf", "documentId": "d2", "decryptedSize": 3,
                } }))])],
            }),
            json!({}),
        )],
    )]);
    let parsed = read(&app, Format::OnePassword1pux, data.to_string().as_bytes());
    assert_eq!(parsed.items.len(), 1);
    assert_eq!(parsed.left_out, [(LeftOut::Attachment, 2)]);
}

/// A zip with one entry stored as it is, whose sizes say `said`: what a
/// zip bomb says before it's unpacked.
fn zip_saying(name: &str, body: &[u8], said: u32) -> Vec<u8> {
    let crc = {
        let mut crc = 0xffff_ffffu32;
        for b in body {
            crc ^= u32::from(*b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    };
    let mut out = Vec::new();
    let header = |out: &mut Vec<u8>, central: bool| {
        out.extend(if central { 0x0201_4b50u32 } else { 0x0403_4b50 }.to_le_bytes());
        if central {
            out.extend(20u16.to_le_bytes());
        }
        out.extend(20u16.to_le_bytes()); // version needed
        out.extend(0u16.to_le_bytes()); // flags
        out.extend(0u16.to_le_bytes()); // stored
        out.extend(0u32.to_le_bytes()); // time and date
        out.extend(crc.to_le_bytes());
        out.extend((body.len() as u32).to_le_bytes());
        out.extend(said.to_le_bytes());
        out.extend((name.len() as u16).to_le_bytes());
        out.extend(0u16.to_le_bytes()); // extra
        if central {
            out.extend([0u8; 10]); // comment, disk, attributes
            out.extend(0u32.to_le_bytes()); // the local header's offset
        }
        out.extend(name.as_bytes());
    };
    header(&mut out, false);
    out.extend(body);
    let central_at = out.len();
    header(&mut out, true);
    let central_len = out.len() - central_at;
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0u8; 4]);
    out.extend(1u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend((central_len as u32).to_le_bytes());
    out.extend((central_at as u32).to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

#[test]
fn a_1pux_too_big_or_not_one_is_refused_before_its_unpacked() {
    let (_sample, app) = app();
    let data = export(vec![("Personal", vec![])]).to_string();
    // The sizes the zip states are what's checked, before anything's read.
    assert_eq!(
        refusal(
            &app,
            Format::OnePassword1pux,
            &zip_saying("files/big.bin", b"x", u32::MAX - 1)
        ),
        "that .1pux unpacks to more than PassBundle imports (1 GB), so PassBundle didn't open it"
    );
    assert_eq!(
        refusal(
            &app,
            Format::OnePassword1pux,
            &zip_saying("export.data", data.as_bytes(), 200 << 20)
        ),
        "that .1pux holds more items than PassBundle imports (100 MB of them)"
    );
    // The same, honest about its size, reads.
    let honest = zip_saying("export.data", data.as_bytes(), data.len() as u32);
    assert!(read(&app, Format::OnePassword1pux, &honest)
        .items
        .is_empty());
    assert!(refusal(
        &app,
        Format::OnePassword1pux,
        &zip_saying("other.data", b"{}", 2)
    )
    .starts_with("that .1pux has no items in it"));
    let mut damaged = onepux(&json!({ "accounts": [] }), &[]);
    damaged.truncate(damaged.len() - 30);
    assert!(refusal(&app, Format::OnePassword1pux, &damaged).starts_with("that .1pux is damaged"));
    for not_one in [&b"not json"[..], b"{\"accounts\": []}", b"{\"items\": []}"] {
        assert!(refusal(&app, Format::OnePassword1pux, not_one)
            .starts_with("that isn't a 1Password export PassBundle can read"));
    }
    // A .1pux chosen as 1Password's CSV says which to choose.
    assert_eq!(
        refusal(&app, Format::OnePasswordCsv, &honest),
        "that's 1Password's .1pux export, not its CSV. Choose 1Password (.1pux) as its format"
    );
}

#[test]
fn our_own_fields_never_come_in_from_a_1pux() {
    let (_sample, app) = app();
    let login = one(
        &app,
        "001",
        "Planted",
        json!({
            "loginFields": [{ "value": "ana", "id": "", "name": "u", "fieldType": "T", "designation": "username" }],
            "sections": [section("", vec![field("keykeeper.fill", "x", json!({ "concealed": "{\"v\":1}" }))])],
        }),
    );
    assert_eq!(fields(&login), [("Imported keykeeper.fill", "{\"v\":1}")]);
}

// ---------------------------------------------------------------------------
// LastPass's CSV.

const LASTPASS: &str = "url,username,password,totp,extra,name,grouping,fav\n";

fn lastpass(app: &App, rows: &str) -> Parsed {
    read(
        app,
        Format::LastPassCsv,
        format!("{LASTPASS}{rows}").as_bytes(),
    )
}

fn card_of(item: &Item) -> &passbundle_core::vault::Card {
    assert_eq!(item.kind, 3);
    item.card.as_ref().unwrap()
}

#[test]
fn a_lastpass_login_with_its_code_folder_and_favorite() {
    let (_sample, app) = app();
    let parsed = lastpass(
        &app,
        "        http://login.example.test,ana,made-up-lastpass-password,JBSWY3DPEHPK3PXP,The login's notes.,Example Login,Shared-Work\\Servers,1\n\
login2.example.test,,,,,Second,(none),0\n\
https://third.example.test,bo,made-up-third,,,Third,Shared-Work\\Servers,0\n\
https://fourth.example.test,cy,made-up-fourth,,,,Shared-Work,0\n",
    );
    assert_eq!(parsed.items.len(), 4);
    let login = &parsed.items[0];
    assert_eq!(login.kind, 1);
    assert_eq!(login.name, "Example Login");
    assert_eq!(login.uris[0].0, "http://login.example.test");
    assert_eq!(login.username.as_deref(), Some("ana"));
    assert_eq!(text(&login.password), Some("made-up-lastpass-password"));
    assert_eq!(text(&login.totp), Some("JBSWY3DPEHPK3PXP"));
    assert_eq!(text(&login.notes), Some("The login's notes."));
    assert!(login.favorite);
    // The grouping is the folder, as LastPass names it: no "(none)", and
    // no folder made for the one a folder is inside.
    assert_eq!(parsed.folders, ["Shared-Work/Servers", "Shared-Work"]);
    assert_eq!(login.folder, Some(0));
    assert_eq!(parsed.items[1].folder, None);
    assert_eq!(parsed.items[1].uris[0].0, "http://login2.example.test");
    assert!(!parsed.items[1].favorite);
    assert_eq!(parsed.items[2].folder, Some(0));
    assert_eq!(parsed.items[3].folder, Some(1));
    assert_eq!(parsed.items[3].name, "--");
}

#[test]
fn lastpass_cards_in_secure_notes() {
    let (_sample, app) = app();
    let parsed = lastpass(
        &app,
        "http://sn,,,,\"NoteType:Credit Card\nName on Card:Test Person\nType:\nNumber:1234567812345678\nSecurity Code:123\nStart Date:October,2017\nExpiration Date:June,2020\nNotes:some text\n\",Expiration,,0\n\
http://sn,,,,\"NoteType:Credit Card\nName on Card:\nType:\nNumber:\nSecurity Code:\nStart Date:,\nExpiration Date:,\nNotes:\",Blank,,0\n\
http://sn,,,,\"NoteType:Credit Card\nName on Card:Test Person\nType:Visa\nNumber:1234567887654321\nSecurity Code:321\nStart Date:,\nExpiration Date:January,\nNotes:\",No year,,0\n\
http://sn,,,,\"NoteType:Credit Card\nName on Card:Test Person\nType:Mastercard\nNumber:8765432112345678\nSecurity Code:987\nStart Date:,\nExpiration Date:,2020\nNotes:\",No month,,0\n\
http://sn,,,,\"NoteType:Credit Card\nLanguage:en-US\nName on Card:Test Person\nType:Visa\nNumber:4111111111111111\nSecurity Code:123\nStart Date:,\nExpiration Date:December,2030\nNotes:Not a real card.\",Brand from number,Finance,1\n\
http://sn,,,,\"NoteType:Credit Card\nLanguage:en-US\nName on Card:Test Person\nType:Debit\nNumber:5555555555554444\nSecurity Code:321\nStart Date:March,2024\nExpiration Date:January,2029\nNotes:\",Type kept,,0\n",
    );
    let c = card_of(&parsed.items[0]);
    assert_eq!(c.cardholder_name.as_deref(), Some("Test Person"));
    assert_eq!(text(&c.number), Some("1234567812345678"));
    assert_eq!(text(&c.code), Some("123"));
    assert_eq!(c.exp_month.as_deref(), Some("6"));
    assert_eq!(c.exp_year.as_deref(), Some("2020"));
    assert_eq!(c.brand, None);
    assert_eq!(fields(&parsed.items[0]), [("Start Date", "October,2017")]);
    // Every line after Notes is the notes, the empty last one too.
    assert_eq!(text(&parsed.items[0].notes), Some("some text\n"));

    let blank = card_of(&parsed.items[1]);
    assert_eq!(
        (blank.exp_month.as_deref(), blank.number.is_none()),
        (None, true)
    );
    assert!(parsed.items[1].fields.is_empty());

    // The type is the brand when the number doesn't say.
    let no_year = card_of(&parsed.items[2]);
    assert_eq!(no_year.exp_month.as_deref(), Some("1"));
    assert_eq!(no_year.exp_year, None);
    assert_eq!(no_year.brand.as_deref(), Some("Visa"));
    assert!(parsed.items[2].fields.is_empty());
    let no_month = card_of(&parsed.items[3]);
    assert_eq!(no_month.exp_month, None);
    assert_eq!(no_month.exp_year.as_deref(), Some("2020"));
    assert_eq!(no_month.brand.as_deref(), Some("Mastercard"));

    let visa = card_of(&parsed.items[4]);
    assert_eq!(visa.brand.as_deref(), Some("Visa"));
    assert_eq!(visa.exp_month.as_deref(), Some("12"));
    assert_eq!(visa.exp_year.as_deref(), Some("2030"));
    assert!(parsed.items[4].fields.is_empty());
    assert_eq!(text(&parsed.items[4].notes), Some("Not a real card."));
    assert!(parsed.items[4].favorite);
    assert_eq!(parsed.folders, ["Finance"]);

    // A type that isn't the brand, and a real start date, are fields.
    assert_eq!(
        card_of(&parsed.items[5]).brand.as_deref(),
        Some("Mastercard")
    );
    assert_eq!(
        fields(&parsed.items[5]),
        [("Type", "Debit"), ("Start Date", "March,2024")]
    );
}

#[test]
fn lastpass_addresses_and_the_kinds_that_stay_notes() {
    let (_sample, app) = app();
    let long = "x".repeat(250);
    let parsed = lastpass(
        &app,
        &format!(
            "http://sn,,,,\"NoteType:Address\nLanguage:en-US\nTitle:mx\nFirst Name:Test\nLast Name:Person\nAddress 1:1 Example Street\nCity / Town:Exampleville\nZip / Postal Code:00000\nEmail Address:test@example.test\nPhone:{{\"\"num\"\":\"\"5550100\"\",\"\"ext\"\":\"\"12\"\",\"\"cc3l\"\":\"\"USA\"\"}}\nMobile Phone:{{\"\"num\"\":\"\"5550199\"\",\"\"ext\"\":\"\"\"\",\"\"cc3l\"\":\"\"USA\"\"}}\nFax:\nTimezone:{long}\nNotes:First line\nSecond line\",Test Person,,0\n\
http://sn,,,,\"NoteType:Address\nFirst Name:Zoë\nPhone:+1 555 0100\nNotes:\",Zoë,,0\n\
http://sn,,,,\"NoteType:Bank Account\nLanguage:\nBank Name:Example Bank\nAccount Number:1234567890\nNotes:\",Bank,Finance,0\n\
http://sn,,,,\"NoteType:Passport\nNumber:P1234567\nNotes:\",Passport,,0\n\
http://sn,,,,\"A plain note.\nWith two lines.\",Plain,,0\n"
        ),
    );
    let person = &parsed.items[0];
    assert_eq!(person.kind, 4);
    let i = person.identity.as_ref().unwrap();
    assert_eq!(i.title.as_deref(), Some("mx"));
    assert_eq!(i.first_name.as_deref(), Some("Test"));
    assert_eq!(i.last_name.as_deref(), Some("Person"));
    assert_eq!(i.address1.as_deref(), Some("1 Example Street"));
    assert_eq!(i.city.as_deref(), Some("Exampleville"));
    assert_eq!(i.postal_code.as_deref(), Some("00000"));
    assert_eq!(i.email.as_deref(), Some("test@example.test"));
    // LastPass's phone is the number, with its extension; another is a field.
    assert_eq!(i.phone.as_deref(), Some("5550100 ext. 12"));
    assert_eq!(fields(person), [("Mobile Phone", "5550199")]);
    // A long value goes to the notes, before Notes and what follows it.
    assert_eq!(
        text(&person.notes),
        Some(format!("Timezone: {long}\n\nFirst line\nSecond line").as_str())
    );
    let plain_phone = parsed.items[1].identity.as_ref().unwrap();
    assert_eq!(plain_phone.phone.as_deref(), Some("+1 555 0100"));
    assert!(parsed.items[1].fields.is_empty());

    // Bank accounts, passports, and the rest stay notes, as they are.
    for (at, starts) in [
        (
            2,
            "NoteType:Bank Account\nLanguage:\nBank Name:Example Bank",
        ),
        (3, "NoteType:Passport\n"),
    ] {
        assert_eq!(parsed.items[at].kind, 2);
        assert!(text(&parsed.items[at].notes).unwrap().starts_with(starts));
        assert!(parsed.items[at].fields.is_empty());
    }
    assert_eq!(parsed.items[2].folder, Some(0));
    assert_eq!(
        text(&parsed.items[4].notes),
        Some("A plain note.\nWith two lines.")
    );
}

#[test]
fn lastpass_form_fills_are_identities_with_their_cards_or_cards() {
    let (_sample, app) = app();
    let csv = "profilename,profilelanguage,title,firstname,middlename,lastname,username,company,ssn,address1,address2,address3,city,state,zip,country,email,phone,ccname,ccnum,cccsc,ccexp,notes,grouping\n\
Me,en-US,dr,Test,Q,Person,tperson,Example Corp,000-00-0000,1 Example Street,,,Exampleville,ES,00000,US,test@example.test,555-0100,Test Person,4111111111111111,123,2030-06,My form fill.,Personal\n\
Card only,en-US,,,,,,,,,,,,,,,,,Test Person,5555555555554444,321,2029-11,,\n";
    let parsed = read(&app, Format::LastPassCsv, csv.as_bytes());
    let kinds: Vec<u8> = parsed.items.iter().map(|i| i.kind).collect();
    assert_eq!(kinds, [3, 4, 3]);
    let card = card_of(&parsed.items[0]);
    assert_eq!(parsed.items[0].name, "Me");
    assert_eq!(card.brand.as_deref(), Some("Visa"));
    assert_eq!(card.exp_month.as_deref(), Some("6"));
    assert_eq!(card.exp_year.as_deref(), Some("2030"));
    assert_eq!(parsed.items[0].notes, None);
    let person = parsed.items[1].identity.as_ref().unwrap();
    assert_eq!(person.title.as_deref(), Some("Dr"));
    assert_eq!(person.middle_name.as_deref(), Some("Q"));
    assert_eq!(text(&person.ssn), Some("000-00-0000"));
    assert_eq!(person.postal_code.as_deref(), Some("00000"));
    assert_eq!(text(&parsed.items[1].notes), Some("My form fill."));
    // Both of the form fill's are in its folder.
    assert_eq!(parsed.items[0].folder, Some(0));
    assert_eq!(parsed.items[1].folder, Some(0));
    let only = card_of(&parsed.items[2]);
    assert_eq!(only.brand.as_deref(), Some("Mastercard"));
    assert_eq!(only.exp_month.as_deref(), Some("11"));
    assert_eq!(parsed.items[2].name, "Card only");
}

#[test]
fn a_file_that_isnt_lastpasss_is_refused() {
    let (_sample, app) = app();
    assert!(refusal(
        &app,
        Format::LastPassCsv,
        b"name,url,username,password,note\nA,https://a.test,a,b,\n"
    )
    .starts_with("that isn't LastPass's CSV export"));
    // Its column names alone: nothing comes in.
    assert!(read(&app, Format::LastPassCsv, LASTPASS.as_bytes())
        .items
        .is_empty());
    assert!(refusal(
        &app,
        Format::LastPassCsv,
        b"url,name,grouping\n\"never closed,x"
    )
    .starts_with("that file has a quote"));
}

// ---------------------------------------------------------------------------
// Saving.

#[test]
fn a_1pux_and_a_lastpass_export_come_in_encrypted_with_their_folders() {
    let (url, asked) = stand_in(vec![]);
    let sample = Sample::new(&url, &[], |_| {});
    let mut app = sample.unlocked();
    let data = export(vec![
        (
            "Private",
            vec![item(
                "001",
                "Made-up 1Password login",
                json!({ "loginFields": [
                    { "value": "ana", "id": "", "name": "username", "fieldType": "T", "designation": "username" },
                    { "value": "made-up-1pux-password", "id": "", "name": "password", "fieldType": "P", "designation": "password" },
                ] }),
                json!({ "overview": { "tags": ["travel"], "urls": [{ "label": "", "url": "https://travel.example.test/" }] } }),
            )],
        ),
        (
            "Family",
            vec![ssh_item("Made-up SSH key", &made_up_ssh_key(), json!([]))],
        ),
    ]);
    let path = sample.path("TESTACCOUNT.1pux");
    std::fs::write(&path, onepux(&data, &[("madeup__scan.pdf", b"made up")])).unwrap();
    assert_eq!(
        app.read_import(Format::OnePassword1pux, &path, None)
            .unwrap(),
        ImportRead::Read
    );
    let pending = app.pending_import().unwrap();
    assert_eq!(pending.left_out, [(LeftOut::Attachment, 1)]);
    assert_eq!(pending.folders, ["Private/Travel", "Private", "Family"]);
    let saved = app.save_import(&Destination::Personal).unwrap();
    assert_eq!(saved.added, 2);

    let key = user_key();
    let folders: Vec<String> = sent(&asked, "POST /api/folders")
        .iter()
        .map(|f| open(&key, &f["name"]))
        .collect();
    // Only the folders an item goes in.
    assert_eq!(folders, ["Private/Travel", "Family"]);
    let imports = sent(&asked, "POST /api/ciphers/import");
    let body = imports[0].to_string();
    assert!(
        !body.contains("made-up") && !body.contains("example.test") && !body.contains("OPENSSH"),
        "{body}"
    );
    let ciphers = imports[0]["ciphers"].as_array().unwrap();
    assert_eq!(
        open(&key, &ciphers[0]["login"]["password"]),
        "made-up-1pux-password"
    );
    assert_eq!(ciphers[1]["type"], 5);
    assert!(open(&key, &ciphers[1]["sshKey"]["privateKey"])
        .starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
    assert_eq!(
        open(&key, &ciphers[1]["sshKey"]["keyFingerprint"]),
        "SHA256:bbXpuKG6zhzdmnxq256TlqzFBzRl2f6OOg722cYNbU8"
    );

    let path = sample.path("lastpass_export.csv");
    std::fs::write(
        &path,
        format!(
            "{LASTPASS}https://shop.example.test,ana,made-up-shop-password,,,Example Shop,Shopping,0\n\
http://sn,,,,\"NoteType:Credit Card\nName on Card:Test Person\nNumber:4111111111111111\nSecurity Code:123\nExpiration Date:June,2030\nNotes:\",Made-up card,Shopping,0\n"
        ),
    )
    .unwrap();
    app.read_import(Format::LastPassCsv, &path, None).unwrap();
    assert_eq!(app.save_import(&Destination::Personal).unwrap().added, 2);
    let imports = sent(&asked, "POST /api/ciphers/import");
    let last = imports.last().unwrap();
    assert!(!last.to_string().contains("made-up"));
    let ciphers = last["ciphers"].as_array().unwrap();
    assert_eq!(ciphers[1]["type"], 3);
    assert_eq!(
        open(&key, &ciphers[1]["card"]["number"]),
        "4111111111111111"
    );
    assert_eq!(open(&key, &ciphers[1]["card"]["brand"]), "Visa");
    assert_eq!(
        last["folderRelationships"],
        json!([{ "key": 0, "value": 0 }, { "key": 1, "value": 0 }])
    );
}
