//! The core against docs/vectors/core-v1.json (copied into tests/vectors),
//! which plain Node made and upstream's own crypto also opens. Every value
//! must derive or open as the vectors say, and every refusal must refuse.

use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::Value;

use crate::crypto::{EncString, PrivateKey, SymmetricKey};
use crate::error::Error;
use crate::kdf::{Kdf, MasterKey, Prelogin};
use crate::totp::Totp;
use crate::vault::{ItemKind, Sync, Vault};

fn vectors() -> Value {
    serde_json::from_str(include_str!("../tests/vectors/core-v1.json")).unwrap()
}

fn b64(v: &Value) -> Vec<u8> {
    STANDARD.decode(v.as_str().unwrap()).unwrap()
}

fn key(v: &Value) -> SymmetricKey {
    SymmetricKey::from_bytes(&b64(v)).unwrap()
}

fn prelogin(v: &Value) -> Prelogin {
    serde_json::from_value(v.clone()).unwrap()
}

#[test]
fn derives_the_master_key_login_hash_and_stretched_key_with_each_kdf() {
    let v = vectors();
    let account = &v["account"];
    for name in ["pbkdf2", "argon2id"] {
        let a = &account[name];
        let kdf = Kdf::from_prelogin(&prelogin(&a["prelogin"])).unwrap();
        // As typed: the core trims and lowercases the email itself.
        let password = account["password"].as_str().unwrap();
        let master =
            MasterKey::derive(password, account["emailAsTyped"].as_str().unwrap(), &kdf).unwrap();
        assert_eq!(
            master.bytes().as_slice(),
            b64(&a["masterKey"]),
            "{name} master key"
        );
        assert_eq!(
            master.login_hash(password),
            a["loginHash"].as_str().unwrap(),
            "{name} login hash"
        );
        assert_eq!(
            master.stretch(),
            key(&a["stretchedKey"]),
            "{name} stretched key"
        );
        let user = EncString::parse(a["protectedUserKey"].as_str().unwrap())
            .unwrap()
            .decrypt_key(&master.stretch())
            .unwrap();
        assert_eq!(user, key(&v["keys"]["userKey"]), "{name} user key");
    }
}

#[test]
fn refuses_kdf_settings_outside_the_bounds_before_deriving() {
    let v = vectors();
    for weak in v["weakKdf"].as_array().unwrap() {
        let result = Kdf::from_prelogin(&prelogin(&weak["prelogin"]));
        assert!(
            matches!(result, Err(Error::WeakKdf(_)) | Err(Error::NewerApp)),
            "should refuse {}: {result:?}",
            weak["why"]
        );
    }
}

#[test]
fn opens_the_private_key_and_organization_keys_in_both_rsa_formats() {
    let v = vectors();
    let keys = &v["keys"];
    let user = key(&keys["userKey"]);
    let der = EncString::parse(keys["encryptedPrivateKey"].as_str().unwrap())
        .unwrap()
        .decrypt(&user)
        .unwrap();
    assert_eq!(*der, b64(&keys["privateKey"]));
    let private = PrivateKey::from_pkcs8_der(&der).unwrap();
    assert_eq!(private.public_key_der(), b64(&keys["publicKey"]));
    for wrapped in ["encryptedOrganizationKey", "encryptedOrganizationKeySha256"] {
        let opened = private
            .decrypt(&EncString::parse(keys[wrapped].as_str().unwrap()).unwrap())
            .unwrap();
        assert_eq!(*opened, b64(&keys["organizationKey"]), "{wrapped}");
    }
    let item = EncString::parse(keys["encryptedItemKey"].as_str().unwrap())
        .unwrap()
        .decrypt_key(&user)
        .unwrap();
    assert_eq!(item, key(&keys["itemKey"]));
}

/// Checks an opened vault against the vectors' "opened" section.
fn assert_opens_as_the_vectors_say(vault: &Vault, v: &Value) {
    let opened = &v["opened"];
    assert!(
        vault.damaged.is_empty(),
        "nothing damaged: {:?}",
        vault.damaged.iter().map(|d| &d.error).collect::<Vec<_>>()
    );
    for (id, name) in opened["folders"].as_object().unwrap() {
        assert_eq!(vault.folders[id], name.as_str().unwrap());
    }
    for (id, name) in opened["collections"].as_object().unwrap() {
        assert_eq!(vault.collections[id], name.as_str().unwrap());
    }
    let items = opened["items"].as_object().unwrap();
    assert_eq!(vault.items.len(), items.len());
    for (id, want) in items {
        let item = vault.item(id).unwrap_or_else(|| panic!("item {id}"));
        let text =
            |s: &Option<zeroize::Zeroizing<String>>| s.as_ref().map(|z| z.as_str().to_owned());
        assert_eq!(item.name, want["name"].as_str().unwrap());
        assert_eq!(text(&item.notes).as_deref(), want["notes"].as_str());
        assert_eq!(item.folder_id.as_deref(), want["folderId"].as_str());
        assert_eq!(
            item.organization_id.as_deref(),
            want["organizationId"].as_str()
        );
        assert_eq!(item.favorite, want["favorite"].as_bool().unwrap());
        match want["type"].as_u64().unwrap() {
            1 => {
                assert_eq!(item.kind, ItemKind::Login);
                assert_eq!(item.username.as_deref(), want["username"].as_str());
                assert_eq!(text(&item.password).as_deref(), want["password"].as_str());
                assert_eq!(text(&item.totp).as_deref(), want["totp"].as_str());
                let uris: Vec<&str> = want["uris"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|u| u.as_str().unwrap())
                    .collect();
                assert_eq!(item.uris, uris);
                let fields = want["fields"].as_array().unwrap();
                assert_eq!(item.fields.len(), fields.len());
                for (field, f) in item.fields.iter().zip(fields) {
                    assert_eq!(field.name, f["name"].as_str().unwrap());
                    assert_eq!(field.value.as_str(), f["value"].as_str().unwrap());
                    assert_eq!(field.hidden, f["type"].as_u64() == Some(1));
                }
            }
            2 => assert_eq!(item.kind, ItemKind::SecureNote),
            3 => {
                assert_eq!(item.kind, ItemKind::Card);
                let card = item.card.as_ref().expect("a card's details");
                let want = &want["card"];
                assert_eq!(
                    card.cardholder_name.as_deref(),
                    want["cardholderName"].as_str()
                );
                assert_eq!(card.brand.as_deref(), want["brand"].as_str());
                assert_eq!(card.exp_month.as_deref(), want["expMonth"].as_str());
                assert_eq!(card.exp_year.as_deref(), want["expYear"].as_str());
                assert_eq!(text(&card.number).as_deref(), want["number"].as_str());
                assert_eq!(text(&card.code).as_deref(), want["code"].as_str());
            }
            4 => {
                assert_eq!(item.kind, ItemKind::Identity);
                let id = item.identity.as_ref().expect("an identity's details");
                let want = &want["identity"];
                for (seen, key) in [
                    (&id.title, "title"),
                    (&id.first_name, "firstName"),
                    (&id.middle_name, "middleName"),
                    (&id.last_name, "lastName"),
                    (&id.address1, "address1"),
                    (&id.address2, "address2"),
                    (&id.address3, "address3"),
                    (&id.city, "city"),
                    (&id.state, "state"),
                    (&id.postal_code, "postalCode"),
                    (&id.country, "country"),
                    (&id.company, "company"),
                    (&id.email, "email"),
                    (&id.phone, "phone"),
                    (&id.username, "username"),
                ] {
                    assert_eq!(seen.as_deref(), want[key].as_str(), "{key}");
                }
                assert_eq!(text(&id.ssn).as_deref(), want["ssn"].as_str());
                assert_eq!(
                    text(&id.passport_number).as_deref(),
                    want["passportNumber"].as_str()
                );
                assert_eq!(
                    text(&id.license_number).as_deref(),
                    want["licenseNumber"].as_str()
                );
            }
            5 => {
                assert_eq!(item.kind, ItemKind::SshKey);
                let key = item.ssh_key.as_ref().expect("an SSH key's details");
                let want = &want["sshKey"];
                assert_eq!(key.public_key.as_deref(), want["publicKey"].as_str());
                assert_eq!(key.fingerprint.as_deref(), want["keyFingerprint"].as_str());
                assert_eq!(
                    text(&key.private_key).as_deref(),
                    want["privateKey"].as_str()
                );
            }
            other => panic!("unexpected item type {other}"),
        }
    }
}

/// Attachments: each item's files' names and sizes, and each file opens with
/// its own key; a changed file is refused.
#[test]
fn attachments_open_and_a_changed_file_is_refused() {
    use crate::crypto::decrypt_file;
    let v = vectors();
    let sync = Sync::from_json(&v["sync"].to_string()).unwrap();
    let vault = Vault::unlock(
        &sync,
        &prelogin(&v["account"]["pbkdf2"]["prelogin"]),
        v["account"]["password"].as_str().unwrap(),
    )
    .unwrap();
    let mut seen = 0;
    for (id, want) in v["opened"]["items"].as_object().unwrap() {
        let item = vault.item(id).unwrap();
        let wanted = want["attachments"].as_array().cloned().unwrap_or_default();
        assert_eq!(item.attachments.len(), wanted.len(), "{id}");
        for (a, w) in item.attachments.iter().zip(&wanted) {
            assert_eq!(a.id, w["id"].as_str().unwrap());
            assert_eq!(a.name, w["fileName"].as_str().unwrap());
            assert_eq!(a.size, w["size"].as_u64());
            let file = &v["attachmentFiles"][&a.id];
            let opened = decrypt_file(b64(&file["encrypted"]), a.key.as_ref().unwrap()).unwrap();
            assert_eq!(opened.as_slice(), b64(&file["plain"]).as_slice());
            seen += 1;
        }
    }
    assert_eq!(seen, 2);
    for r in v["attachmentRefused"].as_array().unwrap() {
        let id = r["id"].as_str().unwrap();
        let key = &vault
            .items
            .iter()
            .flat_map(|i| &i.attachments)
            .find(|a| a.id == id)
            .unwrap()
            .key
            .clone()
            .unwrap();
        assert!(
            decrypt_file(b64(&r["encrypted"]), key).is_err(),
            "{}",
            r["why"]
        );
    }
    // Not a file at all, or another format.
    let key = &vault
        .items
        .iter()
        .flat_map(|i| &i.attachments)
        .next()
        .unwrap()
        .key
        .clone()
        .unwrap();
    assert!(decrypt_file(vec![], key).is_err());
    assert!(decrypt_file(vec![2; 40], key).is_err());
    assert!(decrypt_file(vec![0; 80], key).is_err());
}

/// An attachment whose key doesn't open is listed as one that can't be
/// opened, and the rest of its item still opens.
#[test]
fn an_attachment_that_doesnt_open_leaves_its_item() {
    let v = vectors();
    let mut changed = v["sync"].clone();
    let item = changed["ciphers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| {
            c["attachments"]
                .as_array()
                .is_some_and(|a| a.iter().any(|a| a["key"].is_string()))
        })
        .unwrap();
    let id = item["id"].as_str().unwrap().to_owned();
    let attachment = item["attachments"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|a| a["key"].is_string())
        .unwrap();
    let attachment_id = attachment["id"].as_str().unwrap().to_owned();
    // Its key with another MAC: its integrity check fails.
    let wrapped = attachment["key"].as_str().unwrap();
    let (rest, _mac) = wrapped.rsplit_once('|').unwrap();
    attachment["key"] = Value::String(format!("{rest}|{}", "A".repeat(43) + "="));
    let sync = Sync::from_json(&changed.to_string()).unwrap();
    let vault = Vault::unlock(
        &sync,
        &prelogin(&v["account"]["pbkdf2"]["prelogin"]),
        v["account"]["password"].as_str().unwrap(),
    )
    .unwrap();
    let item = vault.item(&id).expect("the item still opens");
    assert_eq!(
        item.name,
        v["opened"]["items"][&id]["name"].as_str().unwrap()
    );
    let attachment = item
        .attachments
        .iter()
        .find(|a| a.id == attachment_id)
        .unwrap();
    assert!(attachment.key.is_none());
    assert_eq!(attachment.name, crate::vault::CANT_OPEN_ATTACHMENT);
}

#[test]
fn unlocks_the_sample_sync_with_the_master_password() {
    let v = vectors();
    let sync = Sync::from_json(&v["sync"].to_string()).unwrap();
    let pre = prelogin(&v["account"]["pbkdf2"]["prelogin"]);
    let vault = Vault::unlock(&sync, &pre, v["account"]["password"].as_str().unwrap()).unwrap();
    assert_eq!(vault.public_key_der(), b64(&v["keys"]["publicKey"]));
    assert_eq!(
        vault.organizations[v["ids"]["organization"].as_str().unwrap()],
        "Contoso"
    );
    assert_opens_as_the_vectors_say(&vault, &v);
}

#[test]
fn unlocks_an_argon2id_account_too() {
    let v = vectors();
    let mut sync_json = v["sync"].clone();
    sync_json["profile"]["key"] = v["account"]["argon2id"]["protectedUserKey"].clone();
    let sync = Sync::from_json(&sync_json.to_string()).unwrap();
    let pre = prelogin(&v["account"]["argon2id"]["prelogin"]);
    let vault = Vault::unlock(&sync, &pre, v["account"]["password"].as_str().unwrap()).unwrap();
    assert_opens_as_the_vectors_say(&vault, &v);
}

#[test]
fn a_wrong_password_is_wrong_and_weak_settings_are_refused_before_deriving() {
    let v = vectors();
    let sync = Sync::from_json(&v["sync"].to_string()).unwrap();
    let pre = prelogin(&v["account"]["pbkdf2"]["prelogin"]);
    assert_eq!(
        Vault::unlock(&sync, &pre, "not the password").err(),
        Some(Error::WrongPassword)
    );
    let weak = prelogin(&v["weakKdf"][0]["prelogin"]);
    assert!(matches!(
        Vault::unlock(&sync, &weak, v["account"]["password"].as_str().unwrap()).err(),
        Some(Error::WeakKdf(_))
    ));
}

#[test]
fn an_item_that_doesnt_open_is_damaged_and_the_rest_still_open() {
    let v = vectors();
    let mut sync_json = v["sync"].clone();
    let bad = v["refused"][0]["encString"].clone();
    sync_json["ciphers"][0]["name"] = bad;
    let sync = Sync::from_json(&sync_json.to_string()).unwrap();
    let vault = Vault::unlock(
        &sync,
        &prelogin(&v["account"]["pbkdf2"]["prelogin"]),
        v["account"]["password"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(vault.damaged.len(), 1);
    assert_eq!(vault.damaged[0].id, v["ids"]["personal"].as_str().unwrap());
    assert_eq!(vault.items.len(), 6);
}

#[test]
fn refuses_every_refused_string() {
    let v = vectors();
    for r in v["refused"].as_array().unwrap() {
        let k = key(&v["keys"][r["key"].as_str().unwrap()]);
        let result =
            EncString::parse(r["encString"].as_str().unwrap()).and_then(|e| e.decrypt_string(&k));
        assert!(result.is_err(), "should refuse {}", r["why"]);
    }
}

#[test]
fn makes_one_time_codes_and_refuses_secrets_it_cant_use() {
    let v = vectors();
    for t in v["totp"].as_array().unwrap() {
        let totp = Totp::parse(t["secret"].as_str().unwrap()).unwrap();
        assert_eq!(
            totp.code_at(t["at"].as_u64().unwrap()),
            t["code"].as_str().unwrap(),
            "{}",
            t["secret"]
        );
    }
    for r in v["totpRefused"].as_array().unwrap() {
        assert!(
            Totp::parse(r["secret"].as_str().unwrap()).is_err(),
            "should refuse {}",
            r["why"]
        );
    }
}

#[test]
fn search_matches_every_word_in_names_usernames_websites_and_details() {
    let v = vectors();
    let sync = Sync::from_json(&v["sync"].to_string()).unwrap();
    let vault = Vault::unlock(
        &sync,
        &prelogin(&v["account"]["pbkdf2"]["prelogin"]),
        v["account"]["password"].as_str().unwrap(),
    )
    .unwrap();
    let names = |q: &str| {
        vault
            .search(q)
            .iter()
            .map(|i| i.name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(names("mail"), ["Example Mail"]);
    assert_eq!(names("BACKUP example"), ["Backup service"]);
    assert_eq!(names("office wi-fi"), ["Office Wi-Fi"]);
    assert_eq!(names("").len(), 7);
    assert!(names("nothing like this").is_empty());
    // A card by its holder and brand, an identity by its name and email;
    // never by a secret.
    assert_eq!(names("visa"), ["Everyday card"]);
    assert_eq!(names("ana lopez visa"), ["Everyday card"]);
    assert_eq!(
        names("ana lopez"),
        ["Ana at home", "Everyday card", "Example Mail"]
    );
    assert_eq!(names("maría"), ["Ana at home"]);
    assert_eq!(
        names("ana.lopez@example.test"),
        ["Ana at home", "Example Mail"]
    );
    assert!(names("4111").is_empty() && names("123-45").is_empty());
}

/// Passkeys: a login's passkey opens (its private key only when asked), and
/// signing in with it gives the vectors' authenticator data and signature to
/// the byte. The vectors' signatures come from their own RFC 6979 signer, so
/// two implementations agree, and Node's crypto verifies them.
#[test]
fn a_passkey_opens_and_signs_as_the_vectors_say() {
    use crate::passkey::{authenticator_data, credential_id, flags};
    let v = vectors();
    let sync = Sync::from_json(&v["sync"].to_string()).unwrap();
    let vault = Vault::unlock(
        &sync,
        &prelogin(&v["account"]["pbkdf2"]["prelogin"]),
        v["account"]["password"].as_str().unwrap(),
    )
    .unwrap();
    let mut seen = 0;
    for (id, want) in v["opened"]["items"].as_object().unwrap() {
        let item = vault.item(id).unwrap();
        let Some(want) = want.get("passkey") else {
            assert!(item.passkey.is_none(), "{id}");
            continue;
        };
        let passkey = item.passkey.as_ref().unwrap();
        assert_eq!(
            passkey.credential_id,
            want["credentialId"].as_str().unwrap()
        );
        assert_eq!(passkey.rp_id, want["rpId"].as_str().unwrap());
        assert_eq!(passkey.rp_name.as_deref(), want["rpName"].as_str());
        assert_eq!(passkey.user_handle.as_deref(), want["userHandle"].as_str());
        assert_eq!(passkey.user_name.as_deref(), want["userName"].as_str());
        assert_eq!(
            passkey.user_display_name.as_deref(),
            want["userDisplayName"].as_str()
        );
        assert_eq!(
            u64::from(passkey.counter),
            want["counter"].as_u64().unwrap()
        );
        assert_eq!(
            passkey.discoverable,
            want["discoverable"].as_bool().unwrap()
        );
        assert_eq!(
            passkey.creation_date.as_deref(),
            want["creationDate"].as_str()
        );
        assert!(passkey.usable);
        let (x, y) = passkey.key().unwrap().public();
        assert_eq!(x.to_vec(), b64(&v["passkeyPublic"]["x"]));
        assert_eq!(y.to_vec(), b64(&v["passkeyPublic"]["y"]));
        seen += 1;
    }
    assert_eq!(seen, 1);

    for a in v["passkeyAssertions"].as_array().unwrap() {
        let item = vault.item(a["itemId"].as_str().unwrap()).unwrap();
        let passkey = item.passkey.as_ref().unwrap();
        let mut set = flags::USER_PRESENT | flags::BACKUP_ELIGIBLE | flags::BACKED_UP;
        if a["userVerified"].as_bool().unwrap() {
            set |= flags::USER_VERIFIED;
        }
        let data = authenticator_data(a["rpId"].as_str().unwrap(), set, passkey.counter, None);
        assert_eq!(data, b64(&a["authenticatorData"]));
        let hash: [u8; 32] = b64(&a["clientDataHash"]).try_into().unwrap();
        assert_eq!(
            passkey.key().unwrap().sign(&data, &hash),
            b64(&a["signature"])
        );
        assert_eq!(
            credential_id(&passkey.credential_id).unwrap(),
            b64(&a["credentialId"])
        );
    }
}

/// Making an account (docs/core.md, "Making an account"): from the vectors'
/// password, keys, and IVs, the core makes exactly the account the vectors
/// have, which upstream's crypto opens (docs/vectors/check-core-vectors.mjs),
/// with each KDF.
#[test]
fn a_new_account_is_made_as_the_vectors_say() {
    use crate::signup::NewAccount;
    let v = vectors();
    let account = &v["account"];
    let user_key = key(&v["keys"]["userKey"]);
    let private = PrivateKey::from_pkcs8_der(&b64(&v["keys"]["privateKey"])).unwrap();
    let iv = |start: u8| -> [u8; 16] { std::array::from_fn(|i| start + i as u8) };
    let password = account["password"].as_str().unwrap();
    let email = account["emailAsTyped"].as_str().unwrap();

    let made = NewAccount::with(
        email,
        None,
        password,
        None,
        &Kdf::from_prelogin(&prelogin(&account["pbkdf2"]["prelogin"])).unwrap(),
        &user_key,
        &private,
        Some((iv(0x80), iv(0xa0))),
    )
    .unwrap();
    assert_eq!(made.email, account["salt"].as_str().unwrap());
    assert_eq!(
        made.login_hash,
        account["pbkdf2"]["loginHash"].as_str().unwrap()
    );
    assert_eq!(
        made.user_key.to_string(),
        account["pbkdf2"]["protectedUserKey"].as_str().unwrap()
    );
    assert_eq!(
        made.private_key.to_string(),
        v["keys"]["encryptedPrivateKey"].as_str().unwrap()
    );
    assert_eq!(
        STANDARD.encode(&made.public_key),
        v["keys"]["publicKey"].as_str().unwrap()
    );

    let made = NewAccount::with(
        email,
        None,
        password,
        None,
        &Kdf::from_prelogin(&prelogin(&account["argon2id"]["prelogin"])).unwrap(),
        &user_key,
        &private,
        Some((iv(0x90), iv(0xa0))),
    )
    .unwrap();
    assert_eq!(
        made.login_hash,
        account["argon2id"]["loginHash"].as_str().unwrap()
    );
    assert_eq!(
        made.user_key.to_string(),
        account["argon2id"]["protectedUserKey"].as_str().unwrap()
    );
    let body = made.finish_body("token");
    assert_eq!(
        (
            body["kdf"].as_u64(),
            body["kdfIterations"].as_u64(),
            body["kdfMemory"].as_u64(),
            body["kdfParallelism"].as_u64()
        ),
        (Some(1), Some(3), Some(64), Some(4))
    );
}
