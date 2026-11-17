//! Exporting and importing files (meta repo DESIGN.md, "Export and import
//! in the apps"): the password-protected file against the meta repo's
//! vectors byte for byte, the `.json` and `.csv` exports of the vectors'
//! sample vault laid out as the web vault's, each read back, the browsers'
//! and 1Password's files from small made-up fixtures, our own fields kept
//! out of imports, what's sent to a stand-in for the server, and where an
//! import may go.

use passbundle_core::app::{App, Destination, ImportRead};
use passbundle_core::crypto::{EncString, SymmetricKey};
use passbundle_core::error::Error;
use passbundle_core::export::{self, Unopened};
use passbundle_core::import::{self, Format, LeftOut, Read};
use passbundle_core::kdf::Kdf;
use serde_json::{json, Value};

#[path = "support/stand_in.rs"]
mod stand_in;
use stand_in::{open, sent, stand_in, user_key, Sample, BACKUP, GATE, IT, MAIL, OFFICE_WIFI, ORG};

fn vectors() -> Value {
    serde_json::from_str(include_str!("vectors/vault-export-v1.json")).unwrap()
}

fn bytes16(b64: &str) -> [u8; 16] {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .unwrap()
        .try_into()
        .unwrap()
}

fn kdf_of(v: &Value) -> Kdf {
    match v["kdfType"].as_u64().unwrap() {
        0 => Kdf::Pbkdf2 {
            iterations: v["kdfIterations"].as_u64().unwrap() as u32,
        },
        _ => Kdf::Argon2id {
            iterations: v["kdfIterations"].as_u64().unwrap() as u32,
            memory_mib: v["kdfMemory"].as_u64().unwrap() as u32,
            parallelism: v["kdfParallelism"].as_u64().unwrap() as u32,
        },
    }
}

#[test]
fn the_password_protected_file_is_the_web_vaults_byte_for_byte() {
    let v = vectors();
    let password = v["password"].as_str().unwrap();
    let plain = v["plain"].as_str().unwrap();
    for name in ["pbkdf2", "argon2id"] {
        let vector = &v[name];
        let kdf = kdf_of(&vector["kdf"]);
        let salt = vector["salt"].as_str().unwrap();
        let key = export::file_key(password, salt, &kdf).unwrap();
        assert_eq!(
            key,
            SymmetricKey::from_bytes(&{
                use base64::Engine;
                base64::engine::general_purpose::STANDARD
                    .decode(vector["key"].as_str().unwrap())
                    .unwrap()
            })
            .unwrap(),
            "{name}: the key"
        );
        let made = export::protect_with(
            plain,
            password,
            &kdf,
            salt,
            vector["validation"].as_str().unwrap(),
            bytes16(vector["validationIv"].as_str().unwrap()),
            bytes16(vector["dataIv"].as_str().unwrap()),
        )
        .unwrap();
        assert_eq!(
            made.as_str(),
            vector["file"].as_str().unwrap(),
            "{name}: the file"
        );
        let file: Value = serde_json::from_str(vector["file"].as_str().unwrap()).unwrap();
        assert_eq!(
            export::open_protected(&file, password).unwrap().as_str(),
            plain,
            "{name}: opened"
        );
        assert_eq!(
            export::open_protected(&file, v["refused"]["wrongPassword"].as_str().unwrap()),
            Err(Unopened::WrongPassword)
        );
    }
}

#[test]
fn a_changed_or_overreaching_file_is_refused_before_anything_is_opened() {
    let v = vectors();
    let password = v["password"].as_str().unwrap();
    let refused = |name: &str| {
        let file: Value = serde_json::from_str(v["refused"][name].as_str().unwrap()).unwrap();
        export::open_protected(&file, password)
    };
    // A validation value that doesn't open is a password that isn't the file's.
    assert_eq!(refused("badValidationMac"), Err(Unopened::WrongPassword));
    assert_eq!(refused("badDataMac"), Err(Unopened::Damaged));
    assert_eq!(refused("badDataCiphertext"), Err(Unopened::Damaged));
    for name in [
        "tooFewIterations",
        "tooManyIterations",
        "tooMuchMemory",
        "unknownKdf",
    ] {
        assert_eq!(refused(name), Err(Unopened::Settings), "{name}");
    }
    assert_eq!(refused("noData"), Err(Unopened::Damaged));
}

fn read_back(text: &str, format: Format, app: &App) -> import::Parsed {
    match import::read(format, text.as_bytes(), None, app.vault().unwrap()).unwrap() {
        Read::Parsed(parsed) => parsed,
        _ => panic!("not read"),
    }
}

#[test]
fn the_json_export_holds_the_persons_own_items_as_the_web_vault_lays_them_out() {
    let (url, _) = stand_in(vec![]);
    let sample = Sample::new(&url, &[GATE], |_| {});
    let app = sample.unlocked();
    let made = export::json(app.vault().unwrap()).unwrap();
    // Every item of the person's own: not the organization's Office Wi-Fi.
    assert_eq!(made.items, 6);
    let doc: Value = serde_json::from_str(&made.text).unwrap();
    assert!(made
        .text
        .starts_with("{\n  \"encrypted\": false,\n  \"folders\": [\n    {\n      \"id\": "));
    assert_eq!(
        doc["folders"],
        json!([{ "id": "1b2c3d4e-5f60-4718-89a0-b1c2d3e4f506", "name": "Personal" }])
    );
    let items = doc["items"].as_array().unwrap();
    assert!(items
        .iter()
        .all(|i| i["id"] != OFFICE_WIFI && i["organizationId"].is_null()));
    assert!(items
        .iter()
        .all(|i| i.get("key").is_none() && i["collectionIds"].is_null()));
    let item = |id: &str| items.iter().find(|i| i["id"] == id).unwrap();
    let mail = item(MAIL);
    // Keys in the web vault's order, as the file has them.
    let text = made.text.as_str();
    let at = text.find(&format!("\"id\": \"{MAIL}\"")).unwrap();
    let start = text[..at].rfind("\n    {\n").unwrap();
    let end = start + text[start..].find("\n    }").unwrap();
    let keys: Vec<&str> = text[start..end]
        .lines()
        .filter_map(|l| l.strip_prefix("      \""))
        .filter_map(|l| l.split('"').next())
        .collect();
    assert_eq!(
        keys,
        [
            "passwordHistory",
            "revisionDate",
            "creationDate",
            "deletedDate",
            "archivedDate",
            "id",
            "organizationId",
            "folderId",
            "type",
            "reprompt",
            "name",
            "notes",
            "favorite",
            "fields",
            "login",
            "collectionIds",
        ]
    );
    assert_eq!(mail["login"]["username"], "ana.lopez@example.test");
    assert_eq!(mail["login"]["password"], "not-a-real-password-1");
    assert_eq!(
        mail["login"]["uris"],
        json!([{ "match": null, "uri": "https://mail.example.test/login" }])
    );
    assert_eq!(
        mail["fields"][1],
        json!({ "name": "Recovery PIN", "value": "8642", "type": 1, "linkedId": null })
    );
    assert_eq!(mail["favorite"], true);
    // Archived, with its date as JavaScript writes it.
    assert_eq!(item(GATE)["archivedDate"], "2026-11-01T10:00:00.123Z");
    assert_eq!(item(GATE)["secureNote"], json!({ "type": 0 }));
    // The passkey, its values opened, as upstream's export has them.
    let passkey = &item(BACKUP)["login"]["fido2Credentials"][0];
    assert_eq!(passkey["rpId"], "backup.example.test");
    assert_eq!(passkey["counter"], "0");
    assert_eq!(passkey["discoverable"], "true");
    assert_eq!(
        item("718293a4-b5c6-4d74-8f01-162738495a6b")["card"]["number"],
        "4111111111111111"
    );
    assert_eq!(
        item("8293a4b5-c6d7-4e85-9012-2738495a6b7c")["identity"]["ssn"],
        "123-45-6789"
    );
    assert!(
        item("93a4b5c6-d7e8-4f96-a123-38495a6b7c8d")["sshKey"]["privateKey"]
            .as_str()
            .unwrap()
            .starts_with("-----BEGIN OPENSSH PRIVATE KEY-----")
    );

    // And it reads back as it was, our template and expiry fields included.
    let parsed = read_back(&made.text, Format::Json, &app);
    assert_eq!(parsed.items.len(), 6);
    assert_eq!(parsed.folders, ["Personal"]);
    let mail = parsed
        .items
        .iter()
        .find(|i| i.name == "Example Mail")
        .unwrap();
    assert_eq!(mail.folder, Some(0));
    assert_eq!(
        mail.password.as_deref().map(String::as_str),
        Some("not-a-real-password-1")
    );
    assert!(mail.favorite);
    let gate = parsed.items.iter().find(|i| i.name == "Gate code").unwrap();
    assert_eq!(
        gate.archived_date.as_deref(),
        Some("2026-11-01T10:00:00.123Z")
    );
    let backup = parsed
        .items
        .iter()
        .find(|i| i.name == "Backup service")
        .unwrap();
    assert_eq!(backup.passkeys.len(), 1);
    assert_eq!(parsed.renamed, 0);
}

#[test]
fn the_csv_export_holds_logins_and_notes_in_upstreams_columns_and_reads_back() {
    let (url, _) = stand_in(vec![]);
    let sample = Sample::new(&url, &[GATE], |_| {});
    let app = sample.unlocked();
    let made = export::csv(app.vault().unwrap()).unwrap();
    assert_eq!(made.items, 3);
    // The card, the identity, and the SSH key aren't in a .csv.
    assert_eq!(made.left_out, 3);
    let mut lines = made.text.split("\r\n");
    assert_eq!(
        lines.next().unwrap(),
        "folder,favorite,type,name,notes,fields,reprompt,archivedDate,login_uri,login_username,login_password,login_totp"
    );
    let rows: Vec<&str> = lines.collect();
    assert!(rows.contains(&"Personal,1,login,Example Mail,Recovery codes are in the safe.,\"Account number: 12345\nRecovery PIN: 8642\",0,,https://mail.example.test/login,ana.lopez@example.test,not-a-real-password-1,otpauth://totp/Example:ana?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=Example"), "{rows:?}");
    assert!(
        rows.contains(&",,note,Gate code,4417#,,0,2026-11-01T10:00:00.123Z,,,,"),
        "{rows:?}"
    );

    let parsed = read_back(&made.text, Format::Csv, &app);
    assert_eq!(parsed.items.len(), 3);
    let mail = parsed
        .items
        .iter()
        .find(|i| i.name == "Example Mail")
        .unwrap();
    assert_eq!(mail.fields.len(), 2);
    assert_eq!(mail.fields[1].name.as_deref(), Some("Recovery PIN"));
    assert_eq!(mail.uris[0].0, "https://mail.example.test/login");
    let gate = parsed.items.iter().find(|i| i.name == "Gate code").unwrap();
    assert_eq!(gate.kind, 2);
    assert_eq!(
        gate.archived_date.as_deref(),
        Some("2026-11-01T10:00:00.123Z")
    );
}

#[test]
fn exporting_takes_the_master_password_each_time_and_writes_the_file() {
    let (url, _) = stand_in(vec![]);
    let sample = Sample::new(&url, &[], |_| {});
    let mut app = sample.unlocked();
    let path = sample.path("export.json");
    // Not without the master password.
    assert_eq!(
        app.export_to(export::Format::Json, &path, None).err(),
        Some(Error::Refused(
            "type your master password first, then export"
        ))
    );
    assert_eq!(
        app.confirm_export("not it").err(),
        Some(Error::WrongPassword)
    );
    app.confirm_export(&sample.password).unwrap();
    let made = app.export_to(export::Format::Json, &path, None).unwrap();
    assert_eq!(made.items, 6);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("not-a-real-password-1"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    // Once given, it's for one export.
    assert!(app.export_to(export::Format::Csv, &path, None).is_err());

    // Password-protected, with this account's KDF settings, and it opens
    // with the file's password.
    app.confirm_export(&sample.password).unwrap();
    let protected_path = sample.path("export-protected.json");
    app.export_to(
        export::Format::PasswordProtected,
        &protected_path,
        Some("made-up file password"),
    )
    .unwrap();
    let file: Value =
        serde_json::from_str(&std::fs::read_to_string(&protected_path).unwrap()).unwrap();
    assert_eq!(file["kdfType"], 0);
    assert_eq!(file["kdfIterations"], 600000);
    assert!(!file.to_string().contains("not-a-real-password-1"));
    let plain = export::open_protected(&file, "made-up file password").unwrap();
    assert!(plain.contains("not-a-real-password-1"));

    // Read back through the import, its password asked for first.
    assert_eq!(
        app.read_import(Format::Json, &protected_path, None)
            .unwrap(),
        ImportRead::NeedsPassword
    );
    assert_eq!(
        app.read_import(Format::Json, &protected_path, Some("not it"))
            .unwrap(),
        ImportRead::WrongPassword
    );
    assert_eq!(
        app.read_import(Format::Json, &protected_path, Some("made-up file password"))
            .unwrap(),
        ImportRead::Read
    );
    let pending = app.pending_import().unwrap();
    assert!(pending.protected);
    assert_eq!(pending.items.len(), 6);
    // Locking forgets it.
    app.lock();
    assert!(app.pending_import().is_none());
}

#[test]
fn an_organizations_policy_that_removes_personal_export_refuses_it() {
    let (url, _) = stand_in(vec![]);
    let sample = Sample::new(&url, &[], |sync| {
        // A member, not the owner, of an organization with the policy on.
        sync["profile"]["organizations"][0]["type"] = 2.into();
        sync["policies"] = json!([{ "organizationId": ORG, "type": 10, "enabled": true }]);
    });
    let mut app = sample.unlocked();
    assert!(app.export_forbidden_by().is_some());
    assert_eq!(
        app.confirm_export(&sample.password).err(),
        Some(Error::Refused(
            "an organization you're in doesn't let you export your own items"
        ))
    );
}

const CHROME: &str = "name,url,username,password,note\r\n\
mail.example.test,https://mail.example.test/login,ana.lopez@example.test,not-a-real-password-1,\r\n\
mail.example.test,https://mail.example.test/login,ana.lopez@example.test,a-newer-made-up-password,\r\n\
shop.example.test,https://shop.example.test/,ana,\"made-up, with a comma\",A note\r\n\
,android://aGFzaA==@com.example.app/,ana,made-up-app-password,\r\n\
nothing.example.test,https://nothing.example.test/,ana,,\r\n\
shop.example.test,https://shop.example.test/,ana,\"made-up, with a comma\",\r\n";

#[test]
fn a_browsers_export_is_sorted_as_the_guided_import_sorts_it() {
    let (url, _) = stand_in(vec![]);
    let sample = Sample::new(&url, &[], |_| {});
    let app = sample.unlocked();
    let parsed = read_back(CHROME, Format::Chromium("Google Chrome"), &app);
    let names: Vec<&str> = parsed.items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(
        names,
        ["mail.example.test", "shop.example.test", "com.example.app"]
    );
    // The one already in the vault and the repeat are left out, and the one
    // with no password skipped.
    assert!(
        parsed.left_out.contains(&(LeftOut::AlreadyThere, 2)),
        "{:?}",
        parsed.left_out
    );
    assert!(parsed.left_out.contains(&(LeftOut::NoPassword, 1)));
    // A new password for Example Mail's site and username is to compare.
    assert_eq!(parsed.to_check, [0]);
    assert_eq!(
        parsed.check_folder.as_deref(),
        Some("Check these: from Google Chrome")
    );
    assert_eq!(parsed.items[2].uris[0].0, "androidapp://com.example.app");
    assert_eq!(
        parsed.items[1].notes.as_deref().map(String::as_str),
        Some("A note")
    );

    // A file that isn't Chrome's, and an export with nothing in it.
    let refusal = |text: &str, format: Format| match import::read(
        format,
        text.as_bytes(),
        None,
        app.vault().unwrap(),
    ) {
        Err(Error::Refused(why)) => why,
        _ => panic!("read"),
    };
    assert!(refusal(
        "Title,URL,Username,Password\nA,https://a.test,a,b",
        Format::Chromium("Google Chrome")
    )
    .starts_with("that isn't Chrome's password export"));
    assert_eq!(
        refusal(
            "name,url,username,password,note\r\n",
            Format::Chromium("Google Chrome")
        ),
        "that export has no passwords in it"
    );
    assert!(refusal(CHROME, Format::Firefox).starts_with("that isn't Firefox's"));
    assert!(refusal("a,b\n\"never closed,x", Format::Csv).starts_with("that file has a quote"));
}

#[test]
fn firefox_safari_and_1password_files_come_in_as_their_importers_read_them() {
    let (url, _) = stand_in(vec![]);
    let sample = Sample::new(&url, &[], |_| {});
    let app = sample.unlocked();
    let firefox = "\"url\",\"username\",\"password\",\"httpRealm\",\"formActionOrigin\",\"guid\",\"timeCreated\",\"timeLastUsed\",\"timePasswordChanged\"\r\n\
\"https://www.wiki.example.test\",\"ana\",\"made-up-wiki-password\",,\"https://www.wiki.example.test\",\"{a}\",\"1\",\"1\",\"1\"\r\n\
\"chrome://FirefoxAccounts\",\"x\",\"y\",,,\"{b}\",\"1\",\"1\",\"1\"";
    let parsed = read_back(firefox, Format::Firefox, &app);
    assert_eq!(parsed.items.len(), 1);
    assert_eq!(parsed.items[0].name, "wiki.example.test");
    assert!(parsed.left_out.contains(&(LeftOut::FirefoxAccount, 1)));

    let safari = "Title,URL,Username,Password,Notes,OTPAuth\n\
forum.example.test (ana),https://forum.example.test/,ana,made-up-forum-password,Notes here,otpauth://totp/Forum:ana?secret=JBSWY3DPEHPK3PXP\n";
    let parsed = read_back(safari, Format::Safari, &app);
    assert_eq!(parsed.items[0].name, "forum.example.test (ana)");
    assert!(parsed.items[0]
        .totp
        .as_deref()
        .unwrap()
        .starts_with("otpauth://"));

    let one_password = "Title,Url,Username,Password,OTPAuth,Favorite,Archived,Tags,Notes,Member number\n\
Bank,https://bank.example.test,ana,made-up-bank-password,,true,false,\"Finance,Home\",Branch on Main St,12345\n\
Old forum,,old,made-up-old-password,,false,true,Finance,,\n\
Just a note,,,,,false,false,,Some text,\n";
    let parsed = read_back(one_password, Format::OnePasswordCsv, &app);
    assert_eq!(parsed.items.len(), 3);
    let bank = &parsed.items[0];
    assert!(bank.favorite);
    assert_eq!(bank.folder, Some(0));
    assert_eq!(parsed.folders, ["Finance"]);
    assert_eq!(
        bank.notes.as_deref().map(String::as_str),
        Some("Branch on Main St\nTags: Finance, Home")
    );
    assert_eq!(bank.fields[0].name.as_deref(), Some("Member number"));
    assert!(parsed.items[1].archived_date.is_some());
    assert_eq!(parsed.items[2].kind, 2);
}

#[test]
fn our_own_fields_never_come_in_from_another_apps_file() {
    let (url, _) = stand_in(vec![]);
    let sample = Sample::new(&url, &[], |_| {});
    let app = sample.unlocked();
    let one_password = "Title,Url,Username,Password,keykeeper.fill\n\
Bank,https://bank.example.test,ana,pw,\"{\"\"v\"\":1}\"\n";
    let parsed = read_back(one_password, Format::OnePasswordCsv, &app);
    assert_eq!(parsed.renamed, 1);
    assert_eq!(
        parsed.items[0].fields[0].name.as_deref(),
        Some("Imported keykeeper.fill")
    );
    let csv = "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n\
,,login,Bank,,\"keykeeper.expires: 2027-01-01\",0,https://bank.example.test,ana,pw,\n";
    let parsed = read_back(csv, Format::Csv, &app);
    assert_eq!(
        parsed.items[0].fields[0].name.as_deref(),
        Some("Imported keykeeper.expires")
    );
}

#[test]
fn an_import_into_personal_makes_its_folders_and_sends_only_whats_encrypted() {
    let (url, asked) = stand_in(vec![]);
    let sample = Sample::new(&url, &[], |_| {});
    let mut app = sample.unlocked();
    let path = sample.path("Chrome Passwords.csv");
    std::fs::write(&path, CHROME).unwrap();
    assert_eq!(
        app.read_import(Format::Chromium("Google Chrome"), &path, None)
            .unwrap(),
        ImportRead::Read
    );
    let saved = app.save_import(&Destination::Personal).unwrap();
    assert_eq!(saved.added, 3);
    assert!(saved.synced);
    assert!(app.pending_import().is_none());

    let key = user_key();
    let folders = sent(&asked, "POST /api/folders");
    assert_eq!(folders.len(), 1);
    assert_eq!(
        open(&key, &folders[0]["name"]),
        "Check these: from Google Chrome"
    );
    let imports = sent(&asked, "POST /api/ciphers/import");
    assert_eq!(imports.len(), 1);
    let body = imports[0].to_string();
    assert!(
        !body.contains("made-up") && !body.contains("example.test"),
        "{body}"
    );
    let ciphers = imports[0]["ciphers"].as_array().unwrap();
    assert_eq!(
        open(&key, &ciphers[0]["login"]["password"]),
        "a-newer-made-up-password"
    );
    assert_eq!(
        imports[0]["folderRelationships"],
        json!([{ "key": 0, "value": 0 }])
    );
    assert!(imports[0]["folders"][0]["id"].is_string());
    // In the vault now, the one to compare in its folder.
    let vault = app.vault().unwrap();
    let check = vault
        .folders
        .iter()
        .find(|(_, n)| n.starts_with("Check these"))
        .unwrap()
        .0;
    assert!(vault
        .items
        .iter()
        .any(|i| i.folder_id.as_deref() == Some(check.as_str()) && i.name == "mail.example.test"));
}

#[test]
fn an_import_into_a_shared_folder_goes_under_its_organizations_key() {
    let (url, asked) = stand_in(vec![]);
    let sample = Sample::new(&url, &[], |_| {});
    let mut app = sample.unlocked();
    assert_eq!(app.import_shared_folders().len(), 1);
    let path = sample.path("export.json");
    std::fs::write(&path, vectors()["plain"].as_str().unwrap()).unwrap();
    app.read_import(Format::Json, &path, None).unwrap();
    let saved = app
        .save_import(&Destination::SharedFolder(IT.into()))
        .unwrap();
    assert_eq!(saved.added, 3);
    let imports = sent(
        &asked,
        &format!("POST /api/ciphers/import-organization?organizationId={ORG}"),
    );
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0]["collections"][0]["id"], IT);
    assert_eq!(
        imports[0]["collectionRelationships"],
        json!([{ "key": 0, "value": 0 }, { "key": 1, "value": 0 }, { "key": 2, "value": 0 }])
    );
    let ciphers = imports[0]["ciphers"].as_array().unwrap();
    assert!(ciphers.iter().all(|c| c["organizationId"] == ORG));
    // Not under the user key: the organization's.
    assert!(EncString::parse(ciphers[0]["name"].as_str().unwrap())
        .unwrap()
        .decrypt_string(&user_key())
        .is_err());
    // The archived note stays archived, and the card asks for the master password.
    assert_eq!(ciphers[1]["archivedDate"], "2026-11-04T08:30:00.000Z");
    assert_eq!(ciphers[2]["reprompt"], 1);
    // A folder the person can't add to isn't one an import goes to.
    app.read_import(Format::Json, &path, None).unwrap();
    assert!(app
        .save_import(&Destination::SharedFolder("not-a-folder".into()))
        .is_err());
}

#[test]
fn where_an_organization_keeps_items_out_of_personal_an_import_goes_to_a_shared_folder() {
    let (url, asked) = stand_in(vec![]);
    let sample = Sample::new(&url, &[], |sync| {
        sync["profile"]["organizations"][0]["type"] = 2.into();
        sync["policies"] = json!([{ "organizationId": ORG, "type": 5, "enabled": true }]);
    });
    let mut app = sample.unlocked();
    assert!(app.keeps_items_out().is_some());
    let path = sample.path("export.json");
    std::fs::write(&path, vectors()["plain"].as_str().unwrap()).unwrap();
    app.read_import(Format::Json, &path, None).unwrap();
    assert!(matches!(
        app.save_import(&Destination::Personal),
        Err(Error::Refused(why)) if why.starts_with("your organization keeps items out of Personal")
    ));
    assert!(sent(&asked, "POST /api/ciphers/import").is_empty());
    assert_eq!(
        app.save_import(&Destination::SharedFolder(IT.into()))
            .unwrap()
            .added,
        3
    );
}

#[test]
fn a_save_the_server_refuses_partway_sends_only_the_rest_next_time() {
    let (url, _) = stand_in(vec![(
        "/api/ciphers/import",
        "This organization's plan has ended, so nothing new can be added.",
    )]);
    let sample = Sample::new(&url, &[], |_| {});
    let mut app = sample.unlocked();
    let path = sample.path("export.json");
    std::fs::write(&path, vectors()["plain"].as_str().unwrap()).unwrap();
    app.read_import(Format::Json, &path, None).unwrap();
    let refused = app.save_import(&Destination::Personal).err().unwrap();
    // The server's own sentence.
    assert_eq!(
        refused.to_string(),
        "Your server said: This organization's plan has ended, so nothing new can be added."
    );
    // Nothing went, so nothing is counted as saved, and it's still there to try again.
    assert_eq!(app.pending_import().unwrap().saved, 0);
}
