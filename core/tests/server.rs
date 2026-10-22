//! The core against a real PassBundle server (tools/test-server.sh), with
//! real keys throughout: signing up an account the way an app would, signing
//! in (with a wrong password, and with two-step login), sync, saving, editing,
//! refusing an edit to a stale copy, trash, refreshing the session, and an
//! organization's items. Set PB_TEST_SERVER to the server's URL; without it,
//! these tests say so and pass.

use base64::{engine::general_purpose::STANDARD, Engine};
use passbundle_core::app::{App, State, Step};
use passbundle_core::crypto::{encrypt_for, PrivateKey, SymmetricKey};
use passbundle_core::kdf::{Kdf, MasterKey};
use passbundle_core::secrets::MemorySecretStore;
use passbundle_core::server::{Device, Server, Session, SignIn, TwoFactor};
use passbundle_core::store::Store;
use passbundle_core::totp::Totp;
use passbundle_core::vault::{Field, ItemDraft, ItemKind, Vault};
use passbundle_core::Error;
use serde_json::json;
use zeroize::Zeroizing;

const PASSWORD: &str = "correct horse battery staple 2";

fn server() -> Option<Server> {
    let Ok(url) = std::env::var("PB_TEST_SERVER") else {
        eprintln!("PB_TEST_SERVER isn't set; skipping the server tests (tools/test-server.sh starts one).");
        return None;
    };
    Some(Server::new(&url, Device::this_one(&Device::new_identifier())).unwrap())
}

fn new_email(prefix: &str) -> String {
    format!(
        "{prefix}-{}@core.example.test",
        &uuid::Uuid::new_v4().to_string()[..8]
    )
}

#[path = "support/attachment.rs"]
mod attachment;
#[path = "support/sign_up.rs"]
mod support;

fn sign_up(server: &Server, email: &str) {
    support::sign_up(server, email, PASSWORD);
}

fn sign_in(server: &Server, email: &str) -> Session {
    match server.sign_in(email, PASSWORD).unwrap() {
        SignIn::Done(session) => session,
        SignIn::NeedsTwoFactor(_) => panic!("unexpected two-step login"),
    }
}

fn open(server: &Server, session: &mut Session) -> Vault {
    let (sync, _) = server.sync(session).unwrap();
    Vault::open(&sync, session.user_key().clone()).unwrap()
}

fn secret(text: &str) -> Option<Zeroizing<String>> {
    Some(Zeroizing::new(text.to_owned()))
}

#[test]
fn signs_in_syncs_saves_edits_and_trashes() {
    let Some(server) = server() else { return };
    let email = new_email("items");
    sign_up(&server, &email);

    assert_eq!(
        server.sign_in(&email, "not the password").err(),
        Some(Error::WrongPassword)
    );
    // The email as typed: the core trims and lowercases it.
    let mut session = match server
        .sign_in(&format!("  {}", email.to_uppercase()), PASSWORD)
        .unwrap()
    {
        SignIn::Done(session) => session,
        SignIn::NeedsTwoFactor(_) => panic!("unexpected two-step login"),
    };
    assert!(open(&server, &mut session).items.is_empty());

    let vault = open(&server, &mut session);
    let draft = ItemDraft {
        name: "Example Mail".into(),
        username: Some("ana@example.test".into()),
        password: secret("not-a-real-password-1"),
        totp: secret("JBSWY3DPEHPK3PXP"),
        notes: secret("Recovery codes are in the safe."),
        uris: vec!["https://mail.example.test".into()],
        fields: vec![Field {
            name: "PIN".into(),
            value: Zeroizing::new("8642".into()),
            hidden: true,
        }],
        favorite: true,
        ..ItemDraft::default()
    };
    server
        .save_item(
            &mut session,
            None,
            vault.encrypt_item(&draft, None).unwrap(),
            &[],
        )
        .unwrap();

    let vault = open(&server, &mut session);
    assert_eq!(vault.items.len(), 1);
    let item = &vault.items[0];
    assert_eq!(item.name, "Example Mail");
    assert_eq!(item.username.as_deref(), Some("ana@example.test"));
    assert_eq!(
        item.password.as_deref().map(String::as_str),
        Some("not-a-real-password-1")
    );
    assert_eq!(
        item.totp.as_deref().map(String::as_str),
        Some("JBSWY3DPEHPK3PXP")
    );
    assert_eq!(
        item.notes.as_deref().map(String::as_str),
        Some("Recovery codes are in the safe.")
    );
    assert_eq!(item.uris, ["https://mail.example.test"]);
    assert!(item.favorite);
    assert_eq!(
        (
            item.fields[0].name.as_str(),
            item.fields[0].value.as_str(),
            item.fields[0].hidden
        ),
        ("PIN", "8642", true)
    );

    // An edit names the revision it changed, so a stale copy can't overwrite
    // it. The server allows two seconds of difference, so the edit comes later.
    std::thread::sleep(std::time::Duration::from_millis(2100));
    let mut edited = ItemDraft::of(item);
    edited.password = secret("not-a-real-password-2");
    server
        .save_item(
            &mut session,
            Some(&item.id),
            vault.encrypt_item(&edited, Some(item)).unwrap(),
            &[],
        )
        .unwrap();
    let mut stale = ItemDraft::of(item);
    stale.password = secret("from-a-stale-copy");
    let refused = server.save_item(
        &mut session,
        Some(&item.id),
        vault.encrypt_item(&stale, Some(item)).unwrap(),
        &[],
    );
    assert!(matches!(refused, Err(Error::Server(_))), "{refused:?}");

    let vault = open(&server, &mut session);
    assert_eq!(
        vault.items[0].password.as_deref().map(String::as_str),
        Some("not-a-real-password-2")
    );

    server.trash_item(&mut session, &vault.items[0].id).unwrap();
    // An expired access token is refreshed on the way.
    session.expires_at = 0;
    let vault = open(&server, &mut session);
    assert!(vault.items[0].deleted);
    assert!(vault.search("").is_empty());
}

#[test]
fn an_organizations_items_open_with_its_key() {
    let Some(server) = server() else { return };
    let email = new_email("org");
    sign_up(&server, &email);
    let mut session = sign_in(&server, &email);
    let vault = open(&server, &mut session);

    // A new organization, as an app makes one: its key sealed to the owner's
    // public key (RSA, type 4), its own key pair, and a first collection.
    let org_key = SymmetricKey::generate();
    let org_private = PrivateKey::generate().unwrap();
    let created = server
        .raw(
            &mut session,
            "POST",
            "/api/organizations",
            Some(json!({
                "name": "Contoso",
                "billingEmail": email,
                "collectionName": org_key.encrypt_str("IT").to_string(),
                "key": encrypt_for(&vault.public_key_der(), &org_key.to_bytes()).unwrap().to_string(),
                "keys": {
                    "publicKey": STANDARD.encode(org_private.public_key_der()),
                    "encryptedPrivateKey": org_key.encrypt(&org_private.to_pkcs8_der()).to_string(),
                },
                "planType": 0,
            })),
        )
        .unwrap();
    let org_id = created["id"].as_str().unwrap().to_owned();

    let vault = open(&server, &mut session);
    assert_eq!(vault.organizations[&org_id], "Contoso");
    let (collection_id, collection_name) = vault.collections.iter().next().unwrap();
    assert_eq!(collection_name, "IT");
    let draft = ItemDraft {
        name: "Office Wi-Fi".into(),
        password: secret("not-a-real-password-3"),
        organization_id: Some(org_id.clone()),
        ..ItemDraft::default()
    };
    server
        .save_item(
            &mut session,
            None,
            vault.encrypt_item(&draft, None).unwrap(),
            std::slice::from_ref(collection_id),
        )
        .unwrap();

    let vault = open(&server, &mut session);
    let item = &vault.items[0];
    assert_eq!(item.name, "Office Wi-Fi");
    assert_eq!(item.organization_id.as_deref(), Some(org_id.as_str()));
    assert_eq!(item.collection_ids, std::slice::from_ref(collection_id));
    assert_eq!(
        item.password.as_deref().map(String::as_str),
        Some("not-a-real-password-3")
    );
    assert!(vault.damaged.is_empty());
}

#[test]
fn two_step_login_with_an_authenticator_app() {
    let Some(server) = server() else { return };
    let email = new_email("two-step");
    sign_up(&server, &email);
    let mut session = sign_in(&server, &email);
    let secret = "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP";
    let totp = Totp::parse(secret).unwrap();
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    };
    let master = MasterKey::derive(
        PASSWORD,
        &email,
        &Kdf::Pbkdf2 {
            iterations: 600_000,
        },
    )
    .unwrap();
    server
        .raw(
            &mut session,
            "POST",
            "/api/two-factor/authenticator",
            Some(json!({ "key": secret, "token": totp.code_at(now()), "masterPasswordHash": master.login_hash(PASSWORD) })),
        )
        .unwrap();

    let SignIn::NeedsTwoFactor(pending) = server.sign_in(&email, PASSWORD).unwrap() else {
        panic!("two-step login should be asked for");
    };
    assert!(pending.providers.contains(&0), "{:?}", pending.providers);
    let wrong = TwoFactor {
        provider: 0,
        code: "000000".into(),
        remember: false,
    };
    assert_eq!(
        server.finish_sign_in(&pending, &wrong).err(),
        Some(Error::WrongCode)
    );
    // The same waiting sign-in takes the right code: no second KDF. A code from
    // the next window, so it isn't the one used to turn two-step login on.
    std::thread::sleep(std::time::Duration::from_secs(totp.seconds_left(now()) + 1));
    let right = TwoFactor {
        provider: 0,
        code: totp.code_at(now()),
        remember: false,
    };
    let mut session = server.finish_sign_in(&pending, &right).unwrap();
    assert!(open(&server, &mut session).items.is_empty());
}

#[test]
fn the_app_keeps_the_vault_locks_unlocks_offline_and_signs_out() {
    let Some(server) = server() else { return };
    let url = std::env::var("PB_TEST_SERVER").unwrap();
    let email = new_email("app");
    sign_up(&server, &email);
    let dir = std::env::temp_dir().join(format!("pb-app-{}", uuid::Uuid::new_v4()));

    let mut app = App::open(Store::new(&dir), Box::new(MemorySecretStore::default()));
    assert_eq!(
        app.state(),
        State::SignedOut {
            server: None,
            email: None
        }
    );
    assert_eq!(
        app.sign_in(&url, &email, "not the password").err(),
        Some(Error::WrongPassword)
    );
    assert_eq!(app.sign_in(&url, &email, PASSWORD).unwrap(), Step::Done);
    assert_eq!(
        app.state(),
        State::Unlocked {
            server: url.clone(),
            email: email.clone()
        }
    );
    let draft = ItemDraft {
        name: "Example Mail".into(),
        password: secret("not-a-real-password-1"),
        ..ItemDraft::default()
    };
    app.save(None, &draft, &[]).unwrap();
    assert_eq!(app.vault().unwrap().items.len(), 1);

    // Locked: nothing decrypted; the master password opens the kept vault.
    app.lock();
    assert_eq!(
        app.state(),
        State::Locked {
            server: url.clone(),
            email: email.clone()
        }
    );
    assert!(app.vault().is_none());
    assert_eq!(
        app.unlock("not the password").err(),
        Some(Error::WrongPassword)
    );
    app.unlock(PASSWORD).unwrap();
    assert_eq!(app.vault().unwrap().items[0].name, "Example Mail");
    // With the kept refresh token, it syncs.
    app.sync().unwrap();

    // After a restart without a kept refresh token (no secret store), the
    // vault still unlocks from this device; syncing says to sign in again.
    drop(app);
    let mut app = App::open(Store::new(&dir), Box::new(MemorySecretStore::default()));
    assert_eq!(
        app.state(),
        State::Locked {
            server: url.clone(),
            email: email.clone()
        }
    );
    app.unlock(PASSWORD).unwrap();
    assert_eq!(app.vault().unwrap().items.len(), 1);
    assert_eq!(app.sync().err(), Some(Error::SignedOut));
    assert!(app.vault().is_some());

    app.sign_out();
    assert_eq!(
        app.state(),
        State::SignedOut {
            server: Some(url),
            email: Some(email)
        }
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Attachments (docs/core.md, "Attachments"): a file uploaded the way
/// upstream's apps upload one (its own key, and its key and name under the
/// item's), then downloaded by the core, its MAC checked, and opened.
#[test]
fn an_attachment_downloads_and_opens() {
    use passbundle_core::crypto::decrypt_file;
    let Some(server) = server() else { return };
    let url = std::env::var("PB_TEST_SERVER").unwrap();
    let email = new_email("attachment");
    sign_up(&server, &email);
    let mut session = sign_in(&server, &email);
    let vault = open(&server, &mut session);
    let draft = ItemDraft {
        name: "Has a file".into(),
        username: Some("ana".into()),
        ..ItemDraft::default()
    };
    let saved = server
        .save_item(
            &mut session,
            None,
            vault.encrypt_item(&draft, None).unwrap(),
            &[],
        )
        .unwrap();
    let id = saved["id"].as_str().unwrap().to_owned();

    let contents = b"Recovery codes: 1234-5678".to_vec();
    let attachment_id =
        attachment::upload(&server, &url, &mut session, &id, "codes.txt", &contents);

    // The core lists it with the item, downloads it, and opens it.
    let vault = open(&server, &mut session);
    let item = vault.item(&id).unwrap();
    assert_eq!(item.attachments.len(), 1);
    let attachment = &item.attachments[0];
    assert_eq!(attachment.name, "codes.txt");
    let file = server
        .attachment(&mut session, &id, &attachment_id)
        .unwrap();
    // Its own format: the type byte, IV, MAC, and the ciphertext.
    assert_eq!(file[0], 2);
    assert_eq!(
        decrypt_file(file, attachment.key.as_ref().unwrap())
            .unwrap()
            .as_slice(),
        &contents[..]
    );
}

/// Folders: made, renamed, and deleted through the app, which syncs after
/// each; an item in a deleted folder stays, in no folder.
#[test]
fn folders_are_made_renamed_and_deleted() {
    let Some(server) = server() else { return };
    let url = std::env::var("PB_TEST_SERVER").unwrap();
    let email = new_email("folders");
    sign_up(&server, &email);
    let dir =
        std::env::temp_dir().join(format!("pb-core-folders-{}", uuid::Uuid::new_v4().simple()));
    let mut app = App::open(Store::new(&dir), Box::new(MemorySecretStore::default()));
    app.sign_in(&url, &email, PASSWORD).unwrap();

    assert_eq!(
        app.create_folder("   ").err(),
        Some(Error::Refused("a folder needs a name"))
    );
    let id = app.create_folder("  Work ").unwrap();
    assert_eq!(
        app.vault().unwrap().folders.get(&id).map(String::as_str),
        Some("Work")
    );
    app.save(
        None,
        &ItemDraft {
            name: "Filed".into(),
            folder_id: Some(id.clone()),
            ..ItemDraft::default()
        },
        &[],
    )
    .unwrap();

    app.rename_folder(&id, "Clients").unwrap();
    assert_eq!(
        app.vault().unwrap().folders.get(&id).map(String::as_str),
        Some("Clients")
    );
    assert!(app.rename_folder("no-such-folder", "X").is_err());

    app.delete_folder(&id).unwrap();
    let vault = app.vault().unwrap();
    assert!(vault.folders.is_empty());
    let item = vault.items.iter().find(|i| i.name == "Filed").unwrap();
    assert_eq!(item.folder_id, None);
    let _ = std::fs::remove_dir_all(dir);
}

/// Passkeys (docs/core.md, "Passkeys"): one made for a site is saved in a new
/// login before it's answered, and signing in with it gives a signature its
/// own public key verifies; one that counts its uses has its counter raised
/// and saved first; one goes into an existing login too; and a site's
/// credential the vault already has stops a new one.
#[test]
fn passkeys_are_made_and_sign_in() {
    use p256::ecdsa::signature::Verifier;
    use p256::ecdsa::{Signature, VerifyingKey};
    use p256::pkcs8::DecodePublicKey;
    use passbundle_core::app::NewPasskey;
    use sha2::{Digest, Sha256};

    let Some(server) = server() else { return };
    let url = std::env::var("PB_TEST_SERVER").unwrap();
    let email = new_email("passkeys");
    sign_up(&server, &email);
    let dir = std::env::temp_dir().join(format!(
        "pb-core-passkeys-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let mut app = App::open(Store::new(&dir), Box::new(MemorySecretStore::default()));
    app.sign_in(&url, &email, PASSWORD).unwrap();

    let hash: [u8; 32] = Sha256::digest(b"client data").into();
    let made = app
        .passkey_create(&NewPasskey {
            rp_id: "passkeys.example.test",
            rp_name: Some("Passkeys Example"),
            user_handle: &[7; 16],
            user_name: Some("ana@passkeys.example.test"),
            user_display_name: Some("Ana"),
            user_verified: true,
            discoverable: true,
            login: None,
            exclude: &[],
        })
        .unwrap();
    // The attested credential data names it, and the attestation object carries it.
    let data = &made.authenticator_data;
    assert_eq!(
        &data[..32],
        Sha256::digest(b"passkeys.example.test").as_slice()
    );
    assert_eq!(data[32], 0x5d);
    assert_eq!(&data[37..53], &passbundle_core::passkey::AAGUID);
    assert_eq!(&data[55..71], made.credential_id.as_slice());
    assert!(made
        .attestation_object
        .windows(data.len())
        .any(|w| w == data.as_slice()));
    let public = VerifyingKey::from_public_key_der(&made.public_key).unwrap();

    // Saved in a new login for the site, with the account's name.
    let item = app.vault().unwrap().item(&made.item_id).unwrap();
    // Named after the site's ID, not the name it gives itself.
    assert_eq!(item.name, "passkeys.example.test");
    assert_eq!(item.username.as_deref(), Some("ana@passkeys.example.test"));
    assert_eq!(item.uris, vec!["https://passkeys.example.test".to_owned()]);
    assert_eq!(app.passkeys_for("passkeys.example.test").len(), 1);
    assert!(app.passkeys_for("other.example.test").is_empty());

    // Signing in: the signature verifies with its public key.
    let signed_in = app
        .passkey_assert(&made.item_id, "passkeys.example.test", &hash, false)
        .unwrap();
    assert_eq!(signed_in.credential_id, made.credential_id);
    assert_eq!(signed_in.user_handle, vec![7; 16]);
    assert_eq!(signed_in.authenticator_data[32], 0x19);
    assert_eq!(&signed_in.authenticator_data[33..], &[0, 0, 0, 0]);
    let mut message = signed_in.authenticator_data.clone();
    message.extend_from_slice(&hash);
    let signature = Signature::from_der(&signed_in.signature).unwrap();
    assert!(public.verify(&message, &signature).is_ok());
    assert!(app
        .passkey_assert(&made.item_id, "other.example.test", &hash, false)
        .is_err());

    // One that counts its uses: raised and saved before it signs.
    {
        let vault = app.vault().unwrap();
        let item = vault.item(&made.item_id).unwrap();
        let passkey = item.passkey.as_ref().unwrap();
        let stored = passkey.with_counter(item.kept.fido2_credentials.as_ref().unwrap(), 5);
        let body = vault
            .encrypt_item_with(&ItemDraft::of(item), Some(item), Some(stored))
            .unwrap();
        // A second session of the same account, as another device would save it.
        let mut session = sign_in(&server, &email);
        server
            .save_item(&mut session, Some(&made.item_id), body, &[])
            .unwrap();
    }
    app.sync().unwrap();
    let counted = app
        .passkey_assert(&made.item_id, "passkeys.example.test", &hash, true)
        .unwrap();
    assert_eq!(&counted.authenticator_data[33..], &[0, 0, 0, 6]);
    assert_eq!(
        app.vault()
            .unwrap()
            .item(&made.item_id)
            .unwrap()
            .passkey
            .as_ref()
            .unwrap()
            .counter,
        6
    );

    // Into an existing login without a username: it gets the account's.
    app.save(
        None,
        &ItemDraft {
            name: "Shop".into(),
            uris: vec!["https://shop.example.test".into()],
            ..ItemDraft::default()
        },
        &[],
    )
    .unwrap();
    let shop = app
        .vault()
        .unwrap()
        .items
        .iter()
        .find(|i| i.name == "Shop")
        .unwrap()
        .id
        .clone();
    let into = app
        .passkey_create(&NewPasskey {
            rp_id: "shop.example.test",
            rp_name: Some("Shop"),
            user_handle: &[9; 8],
            user_name: Some("ana@shop.example.test"),
            user_display_name: None,
            user_verified: false,
            discoverable: true,
            login: Some(&shop),
            exclude: &[],
        })
        .unwrap();
    assert_eq!(into.item_id, shop);
    let item = app.vault().unwrap().item(&shop).unwrap();
    assert_eq!(item.username.as_deref(), Some("ana@shop.example.test"));
    assert_eq!(item.passkey.as_ref().unwrap().rp_id, "shop.example.test");

    // The site already has this one: nothing new is made.
    let count = app.vault().unwrap().items.len();
    assert!(app
        .passkey_create(&NewPasskey {
            rp_id: "shop.example.test",
            rp_name: Some("Shop"),
            user_handle: &[9; 8],
            user_name: Some("ana@shop.example.test"),
            user_display_name: None,
            user_verified: false,
            discoverable: true,
            login: None,
            exclude: std::slice::from_ref(&into.credential_id),
        })
        .is_err());
    assert_eq!(app.vault().unwrap().items.len(), count);
    let _ = std::fs::remove_dir_all(dir);
}

/// An SSH key on the server comes through the core's sync: the core says
/// which client version's format it reads (server::CLIENT_VERSION), and a
/// server holds SSH keys back from a client that doesn't.
#[test]
fn an_ssh_key_comes_through_sync() {
    let Some(server) = server() else { return };
    let email = new_email("ssh");
    sign_up(&server, &email);
    let mut session = sign_in(&server, &email);
    let key = session.user_key().clone();
    let enc = |text: &str| json!(key.encrypt_str(text).to_string());
    let (sync, _) = server.sync(&mut session).unwrap();
    // As upstream's apps save one (type 5).
    server
        .save_item(
            &mut session,
            None,
            json!({
                "type": 5,
                "encryptedFor": sync.profile.id,
                "name": enc("Deploy key"),
                "notes": null,
                "favorite": false,
                "reprompt": 0,
                "folderId": null,
                "organizationId": null,
                "fields": [],
                "sshKey": {
                    "privateKey": enc("-----BEGIN OPENSSH PRIVATE KEY-----\nnot-a-real-key\n-----END OPENSSH PRIVATE KEY-----\n"),
                    "publicKey": enc("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINotARealKeyNotARealKeyNotARealKey0000 deploy"),
                    "keyFingerprint": enc("SHA256:not-a-real-fingerprint"),
                },
            }),
            &[],
        )
        .unwrap();

    let vault = open(&server, &mut session);
    let item = vault
        .items
        .iter()
        .find(|i| i.name == "Deploy key")
        .expect("the SSH key came through sync");
    assert_eq!(item.kind, ItemKind::SshKey);
    let ssh = item.ssh_key.as_ref().unwrap();
    assert_eq!(
        ssh.fingerprint.as_deref(),
        Some("SHA256:not-a-real-fingerprint")
    );
    assert!(ssh
        .public_key
        .as_deref()
        .is_some_and(|k| k.starts_with("ssh-ed25519 ")));
}

/// Making an account in the app (docs/core.md, "Making an account"): it signs
/// in at once, the email as typed counts as the email, another device signs
/// in with the master password and opens the (empty) vault, and the server
/// refuses a second account for the same email. A short master password is
/// refused before anything is sent.
#[test]
fn an_account_is_made_in_the_app_and_opens_elsewhere() {
    let Some(server) = server() else { return };
    let url = std::env::var("PB_TEST_SERVER").unwrap();
    let email = new_email("made");
    let dir = std::env::temp_dir().join(format!("pb-core-made-{}", uuid::Uuid::new_v4().simple()));
    let mut app = App::open(Store::new(&dir), Box::new(MemorySecretStore::default()));

    assert_eq!(
        app.sign_up(&url, &email, None, "too short", None).err(),
        Some(Error::Refused(
            "a master password needs at least 12 characters"
        ))
    );
    let step = app
        .sign_up(
            &url,
            &format!("  {} ", email.to_uppercase()),
            Some("Made in the app"),
            PASSWORD,
            Some("the usual"),
        )
        .unwrap();
    assert!(matches!(step, Step::Done));
    assert!(app.vault().unwrap().items.is_empty());

    // Another device: the master password opens the same account.
    let mut session = sign_in(&server, &email);
    assert!(open(&server, &mut session).items.is_empty());

    // The same email again.
    let other_dir =
        std::env::temp_dir().join(format!("pb-core-made-{}", uuid::Uuid::new_v4().simple()));
    let mut other = App::open(
        Store::new(&other_dir),
        Box::new(MemorySecretStore::default()),
    );
    assert!(other.sign_up(&url, &email, None, PASSWORD, None).is_err());
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(other_dir);
}
