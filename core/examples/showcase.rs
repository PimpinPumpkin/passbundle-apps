//! A fuller sample vault for the app's screenshots (app/test/screenshots_test.dart):
//! the test vectors' vault (tests/vectors/core-v1.json), the same account and
//! master password, with more in it, encrypted as the web vault would save
//! it: nested folders, shared folders with different permissions, items made
//! from templates, a website for its icon, and a bank account. Not committed:
//!
//!   cargo run --example showcase -- <folder>
//!
//! writes sync.json and settings.json there, as the app keeps them.

use base64::{engine::general_purpose::STANDARD, Engine};
use passbundle_core::crypto::{EncString, PrivateKey, SymmetricKey};
use serde_json::{json, Value};

const SETTINGS: &str = include_str!("../../app/test/sample/settings.json");

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("showcase <folder>");
        std::process::exit(2);
    });
    let v: Value = serde_json::from_str(include_str!("../tests/vectors/core-v1.json")).unwrap();
    let user = SymmetricKey::from_bytes(
        &STANDARD
            .decode(v["keys"]["userKey"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap();
    let private = PrivateKey::from_pkcs8_der(
        &STANDARD
            .decode(v["keys"]["privateKey"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap();
    let mut sync = v["sync"].clone();
    let org_id = sync["profile"]["organizations"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let org = SymmetricKey::from_bytes(
        &private
            .decrypt(
                &EncString::parse(sync["profile"]["organizations"][0]["key"].as_str().unwrap())
                    .unwrap(),
            )
            .unwrap(),
    )
    .unwrap();
    let mine = |t: &str| json!(user.encrypt_str(t).to_string());
    let theirs = |t: &str| json!(org.encrypt_str(t).to_string());

    // Folders: the vectors' one, called Home here, and Work with Clients
    // inside it, and Banking.
    let home = sync["folders"][0]["id"].as_str().unwrap().to_owned();
    sync["folders"][0]["name"] = mine("Home");
    for (id, name) in [
        ("f0000000-0000-4000-8000-000000000001", "Work"),
        ("f0000000-0000-4000-8000-000000000002", "Work/Clients"),
        ("f0000000-0000-4000-8000-000000000003", "Banking"),
    ] {
        sync["folders"].as_array_mut().unwrap().push(
            json!({ "id": id, "name": mine(name), "revisionDate": "2026-10-20T18:00:00.000Z" }),
        );
    }
    let (work, clients, banking) = (
        "f0000000-0000-4000-8000-000000000001",
        "f0000000-0000-4000-8000-000000000002",
        "f0000000-0000-4000-8000-000000000003",
    );

    // Shared folders: IT (the vectors'), IT/Admins inside it, and Everyone,
    // where Ana can see items but not their passwords.
    let it = sync["collections"][0]["id"].as_str().unwrap().to_owned();
    let all = json!({ "seePasswords": true, "create": true, "edit": true, "delete": true, "export": true, "share": true, "print": true, "viewHistory": true, "manage": true });
    sync["collections"][0]["keykeeper"] = all.clone();
    let admins = "c0000000-0000-4000-8000-000000000001";
    let everyone = "c0000000-0000-4000-8000-000000000002";
    for (id, name, grid) in [
        (admins, "IT/Admins", all.clone()),
        (
            everyone,
            "Everyone",
            json!({ "seePasswords": false, "create": false, "edit": false, "delete": false, "export": false, "share": false, "print": false, "viewHistory": true, "manage": false }),
        ),
    ] {
        sync["collections"].as_array_mut().unwrap().push(json!({
            "id": id, "organizationId": org_id, "name": theirs(name),
            "readOnly": grid["edit"] == false, "hidePasswords": grid["seePasswords"] == false,
            "manage": grid["manage"], "keykeeper": grid,
        }));
    }

    let marker = |id: &str, name: &str| format!(r#"{{"id":"{id}","name":"{name}","v":1}}"#);
    let field = |enc: &dyn Fn(&str) -> Value, name: &str, value: &str, kind: u8| json!({ "name": enc(name), "value": enc(value), "type": kind, "linkedId": null });
    let mut n = 0;
    let mut add = |mut cipher: Value| {
        n += 1;
        let base = json!({
            "id": format!("a0000000-0000-4000-8000-{n:012}"),
            "organizationId": null, "folderId": null, "key": null, "favorite": false,
            "reprompt": 0, "revisionDate": "2026-10-02T15:30:00.000Z",
            "creationDate": "2025-03-14T09:00:00.000Z", "deletedDate": null,
            "collectionIds": [], "notes": null, "fields": [], "attachments": null,
            "edit": true, "viewPassword": true, "permissions": { "delete": true, "restore": true },
        });
        for (k, v) in base.as_object().unwrap() {
            if cipher.get(k).is_none() {
                cipher[k] = v.clone();
            }
        }
        sync["ciphers"].as_array_mut().unwrap().push(cipher);
    };
    add(json!({
        "type": 2, "name": mine("Home Wi-Fi"), "folderId": home, "favorite": true,
        "secureNote": { "type": 0 },
        "fields": [
            field(&mine, "Network name", "Lopez Home", 0),
            field(&mine, "Password", "not-a-real-password-wifi", 1),
            field(&mine, "Security type", "WPA3", 0),
            field(&mine, "keykeeper.template", &marker("wifi", "Wi-Fi network"), 0),
        ],
    }));
    add(json!({
        "type": 1, "name": mine("Northwind Bank"), "folderId": banking,
        "login": { "username": mine("ana.lopez"), "password": mine("not-a-real-password-bank"), "uris": [{ "uri": mine("https://northwind.example/login"), "match": null }] },
    }));
    add(json!({
        "type": 6, "name": mine("Savings account"), "folderId": banking,
        "bankAccount": { "bankName": mine("Northwind Bank"), "nameOnAccount": mine("Ana López"), "accountType": mine("Savings"), "accountNumber": mine("000123456789"), "routingNumber": mine("011000015") },
    }));
    add(json!({
        "type": 2, "name": mine("Design tools license"), "folderId": clients,
        "secureNote": { "type": 0 },
        "fields": [
            field(&mine, "License key", "XXXX-0000-YYYY-1111", 1),
            field(&mine, "Licensed to", "Ana López", 0),
            field(&mine, "Version", "2026", 0),
            field(&mine, "Purchase date", "2026-01-15", 0),
            field(&mine, "keykeeper.template", &marker("software-license", "Software license"), 0),
        ],
    }));
    add(json!({
        "type": 2, "name": mine("Payments API"), "folderId": work,
        "secureNote": { "type": 0 },
        "fields": [
            field(&mine, "Key", "pk_test_not_a_real_key", 1),
            field(&mine, "Secret", "sk_test_not_a_real_secret", 1),
            field(&mine, "Service URL", "https://api.payments.example", 0),
            field(&mine, "keykeeper.template", &marker("api-key", "API key"), 0),
        ],
    }));
    add(json!({
        "type": 1, "name": theirs("Office server"), "organizationId": org_id, "collectionIds": [it],
        "login": { "username": theirs("deploy"), "password": theirs("not-a-real-password-server"), "uris": [] },
        "fields": [
            field(&theirs, "Hostname", "10.0.4.12", 0),
            field(&theirs, "Port", "22", 0),
            field(&theirs, "Operating system", "Debian 13", 0),
            field(&theirs, "keykeeper.template", &marker("server", "Server"), 0),
        ],
    }));
    add(json!({
        "type": 1, "name": theirs("contoso.example"), "organizationId": org_id, "collectionIds": [admins],
        "login": { "username": theirs("hostmaster"), "password": theirs("not-a-real-password-dns"), "uris": [] },
        "fields": [
            field(&theirs, "Domain name", "contoso.example", 0),
            field(&theirs, "Registrar", "Example Registrar", 0),
            field(&theirs, "Auto-renew", "true", 2),
            field(&theirs, "keykeeper.template", &marker("domain", "Domain"), 0),
        ],
    }));
    add(json!({
        "type": 1, "name": theirs("Fabrikam VPN"), "organizationId": org_id, "collectionIds": [everyone],
        "login": { "username": theirs("arivera"), "password": theirs("not-a-real-password-vpn"), "uris": [{ "uri": theirs("https://vpn.fabrikam.example"), "match": null }] },
        "edit": false, "viewPassword": false, "permissions": { "delete": false, "restore": false },
    }));

    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(format!("{out}/sync.json"), sync.to_string()).unwrap();
    std::fs::write(format!("{out}/settings.json"), SETTINGS).unwrap();
    println!("Wrote the showcase vault to {out}.");
}
