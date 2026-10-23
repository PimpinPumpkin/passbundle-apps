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
            boolean: false,
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

/// The generator's settings follow the account (core/src/synced_settings.rs):
/// chosen on one device, they're the choices on another once it signs in;
/// the server has them only encrypted; and a change on the second comes back
/// to the first at its next sync.
#[test]
fn the_generators_settings_follow_the_account_to_another_device() {
    use passbundle_core::generator::{Kind, Options};

    let Some(server) = server() else { return };
    let url = std::env::var("PB_TEST_SERVER").unwrap();
    let email = new_email("generator");
    sign_up(&server, &email);
    let device = || {
        let dir = std::env::temp_dir().join(format!(
            "pb-core-generator-{}",
            uuid::Uuid::new_v4().simple()
        ));
        App::open(Store::new(&dir), Box::new(MemorySecretStore::default()))
    };

    let mut first = device();
    first.sign_in(&url, &email, PASSWORD).unwrap();
    let chosen = Options {
        kind: Kind::Passphrase,
        length: 33,
        symbols: false,
        words: 9,
        separator: ".".into(),
        ..Options::default()
    };
    first.generate(&chosen).unwrap();
    first.sync_settings().unwrap();

    let mut session = sign_in(&server, &email);
    let saved = server
        .raw(&mut session, "GET", "/api/keykeeper/settings", None)
        .unwrap();
    let data = saved["data"].as_str().unwrap();
    assert!(data.starts_with("2."), "{data}");
    assert!(!data.contains("passphrase"));

    let mut second = device();
    second.sign_in(&url, &email, PASSWORD).unwrap();
    second.sync().unwrap();
    let there = second.generator_options();
    assert_eq!(there.kind, Kind::Passphrase);
    assert_eq!(there.length, 33);
    assert!(!there.symbols);
    assert_eq!(there.words, 9);
    assert_eq!(there.separator, ".");

    second
        .generate(&Options {
            length: 50,
            ..there
        })
        .unwrap();
    second.sync_settings().unwrap();
    first.sync().unwrap();
    assert_eq!(first.generator_options().length, 50);
    assert_eq!(first.generator_options().words, 9);
}

/// A device of the app's own, on a folder of its own, signed in.
fn signed_in_app(url: &str, email: &str) -> App {
    let dir = std::env::temp_dir().join(format!("pb-core-app-{}", uuid::Uuid::new_v4().simple()));
    let mut app = App::open(Store::new(&dir), Box::new(MemorySecretStore::default()));
    assert_eq!(app.sign_in(url, email, PASSWORD).unwrap(), Step::Done);
    app
}

/// A new organization, as an app makes one (its key sealed to the owner's
/// public key, its own key pair, and a first shared folder, "IT"): its id
/// and key.
fn new_organization(server: &Server, session: &mut Session, email: &str) -> (String, SymmetricKey) {
    let vault = open(server, session);
    let org_key = SymmetricKey::generate();
    let org_private = PrivateKey::generate().unwrap();
    let created = server
        .raw(
            session,
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
    (created["id"].as_str().unwrap().to_owned(), org_key)
}

/// Templates (src/templates.rs): an item the web vault made from one (its
/// fields, then the field that records the template) keeps the template
/// and every field, in order, when the app edits it, a yes-or-no field
/// included; and one the app makes from a template is recorded as the web
/// vault records it.
#[test]
fn templated_items_keep_their_template_and_fields_through_the_apps_edits() {
    use passbundle_core::templates::{self, TEMPLATE_FIELD};

    let Some(server) = server() else { return };
    let url = std::env::var("PB_TEST_SERVER").unwrap();
    let email = new_email("templates");
    sign_up(&server, &email);
    let mut session = sign_in(&server, &email);
    let user_key = session.user_key().clone();
    let user_id = open(&server, &mut session).user_id;
    let enc = |text: &str| json!(user_key.encrypt_str(text).to_string());

    // As the web vault saves a Wi-Fi network (a note) and a domain (a login).
    let marker = |id: &str, name: &str| format!(r#"{{"id":"{id}","name":"{name}","v":1}}"#);
    let wifi = server
        .raw(
            &mut session,
            "POST",
            "/api/ciphers",
            Some(json!({
                "type": 2,
                "name": enc("Office Wi-Fi"),
                "notes": enc("The guest network is separate."),
                "secureNote": { "type": 0 },
                "fields": [
                    { "name": enc("Network name"), "value": enc("Contoso"), "type": 0, "linkedId": null },
                    { "name": enc("Password"), "value": enc("not-a-real-password-wifi"), "type": 1, "linkedId": null },
                    { "name": enc("Security type"), "value": enc("WPA2"), "type": 0, "linkedId": null },
                    { "name": enc(TEMPLATE_FIELD), "value": enc(&marker("wifi", "Wi-Fi network")), "type": 0, "linkedId": null },
                ],
                "favorite": false,
                "reprompt": 0,
                "encryptedFor": user_id,
            })),
        )
        .unwrap();
    let wifi_id = wifi["id"].as_str().unwrap().to_owned();
    let domain = server
        .raw(
            &mut session,
            "POST",
            "/api/ciphers",
            Some(json!({
                "type": 1,
                "name": enc("contoso.example"),
                "login": { "username": enc("hostmaster"), "password": enc("not-a-real-password-dns"), "uris": [] },
                "fields": [
                    { "name": enc("Domain name"), "value": enc("contoso.example"), "type": 0, "linkedId": null },
                    { "name": enc("Registrar"), "value": enc("Example Registrar"), "type": 0, "linkedId": null },
                    { "name": enc("Auto-renew"), "value": enc("true"), "type": 2, "linkedId": null },
                    { "name": enc(TEMPLATE_FIELD), "value": enc(&marker("domain", "Domain")), "type": 0, "linkedId": null },
                ],
                "favorite": false,
                "reprompt": 0,
                "encryptedFor": user_id,
            })),
        )
        .unwrap();
    let domain_id = domain["id"].as_str().unwrap().to_owned();

    let mut app = signed_in_app(&url, &email);
    let vault = app.vault().unwrap();
    let item = vault.item(&wifi_id).unwrap();
    assert_eq!(templates::category(item), "wifi");
    assert_eq!(
        templates::template_of(item).map(|(id, name, _)| (id, name)),
        Some(("wifi".to_owned(), "Wi-Fi network".to_owned()))
    );
    // The editor gets the template's fields, not the record of the template.
    let mut draft = ItemDraft::of(item);
    let names: Vec<&str> = draft.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["Network name", "Password", "Security type"]);
    draft.fields[2].value = Zeroizing::new("WPA3".into());
    draft.name = "Office Wi-Fi (5 GHz)".into();
    app.save(Some(&wifi_id), &draft, &[]).unwrap();

    let item = app.vault().unwrap().item(&domain_id).unwrap();
    assert_eq!(templates::category(item), "domain");
    let mut draft = ItemDraft::of(item);
    assert!(draft.fields[2].boolean);
    draft.fields[2].value = Zeroizing::new("false".into());
    app.save(Some(&domain_id), &draft, &[]).unwrap();

    // As the server keeps them: every field, in order, of the same kinds,
    // the template still recorded, last, as it was.
    let (sync, _) = server.sync(&mut session).unwrap();
    let saved = |id: &str| -> Vec<(String, String, u8)> {
        let cipher = sync.ciphers.iter().find(|c| c.id == id).unwrap();
        cipher
            .fields
            .iter()
            .flatten()
            .map(|f| {
                let open = |t: &Option<String>| {
                    t.as_deref()
                        .map(|t| {
                            passbundle_core::crypto::EncString::parse(t)
                                .unwrap()
                                .decrypt_string(&user_key)
                                .unwrap()
                                .to_string()
                        })
                        .unwrap_or_default()
                };
                (open(&f.name), open(&f.value), f.kind)
            })
            .collect()
    };
    let field = |n: &str, v: &str, k: u8| (n.to_owned(), v.to_owned(), k);
    assert_eq!(
        saved(&wifi_id),
        [
            field("Network name", "Contoso", 0),
            field("Password", "not-a-real-password-wifi", 1),
            field("Security type", "WPA3", 0),
            field(TEMPLATE_FIELD, &marker("wifi", "Wi-Fi network"), 0),
        ]
    );
    assert_eq!(
        saved(&domain_id),
        [
            field("Domain name", "contoso.example", 0),
            field("Registrar", "Example Registrar", 0),
            field("Auto-renew", "false", 2),
            field(TEMPLATE_FIELD, &marker("domain", "Domain"), 0),
        ]
    );
    let item = app.vault().unwrap().item(&wifi_id).unwrap();
    assert_eq!(item.name, "Office Wi-Fi (5 GHz)");
    assert_eq!(templates::category(item), "wifi");

    // A new server from the app's New item picker: recorded as the web vault records it.
    let draft = ItemDraft {
        name: "Build server".into(),
        username: Some("deploy".into()),
        password: secret("not-a-real-password-build"),
        fields: ["Hostname", "Port", "Operating system"]
            .iter()
            .zip(["10.0.4.12", "22", "Debian"])
            .map(|(name, value)| Field {
                name: (*name).into(),
                value: Zeroizing::new(value.into()),
                hidden: false,
                boolean: false,
            })
            .collect(),
        template: Some(("server".into(), "Server".into())),
        ..ItemDraft::default()
    };
    app.save(None, &draft, &[]).unwrap();
    let made = app
        .vault()
        .unwrap()
        .items
        .iter()
        .find(|i| i.name == "Build server")
        .unwrap();
    let id = made.id.clone();
    assert_eq!(templates::category(made), "server");
    let fields = {
        let (sync, _) = server.sync(&mut session).unwrap();
        let cipher = sync.ciphers.into_iter().find(|c| c.id == id).unwrap();
        cipher.fields.unwrap()
    };
    let last = fields.last().unwrap();
    let open = |t: &Option<String>| {
        passbundle_core::crypto::EncString::parse(t.as_deref().unwrap())
            .unwrap()
            .decrypt_string(&user_key)
            .unwrap()
            .to_string()
    };
    assert_eq!(open(&last.name), TEMPLATE_FIELD);
    assert_eq!(open(&last.value), marker("server", "Server"));
    assert_eq!(last.kind, 0);
    assert_eq!(fields.len(), 4);
}

/// Shared folders' permissions (docs/core.md, "Folders and permissions"):
/// a member whose shared folder hides passwords gets an organization's item
/// to see, but not its secrets, nor to edit or delete it; in one where they
/// can edit, they can; and moving an item into a folder they can't add to
/// is refused by the server, with the item left where it was.
#[test]
fn a_shared_folders_permissions_hold_in_the_app() {
    let Some(server) = server() else { return };
    let url = std::env::var("PB_TEST_SERVER").unwrap();
    let owner = new_email("owner");
    let member = new_email("member");
    sign_up(&server, &owner);
    sign_up(&server, &member);
    let mut session = sign_in(&server, &owner);
    let (org_id, org_key) = new_organization(&server, &mut session, &owner);
    let vault = open(&server, &mut session);
    let it = vault.collections.keys().next().unwrap().clone();
    let projects = server
        .raw(
            &mut session,
            "POST",
            &format!("/api/organizations/{org_id}/collections"),
            Some(json!({ "name": org_key.encrypt_str("Projects/Web").to_string(), "groups": [], "users": [] })),
        )
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // The member: invited with "can view, except passwords" in IT and "can
    // edit" in Projects/Web (accepted at once, as a server without email
    // does for an account that exists), then confirmed with the
    // organization's key sealed to their public key.
    server
        .raw(
            &mut session,
            "POST",
            &format!("/api/organizations/{org_id}/users/invite"),
            Some(json!({
                "emails": [member],
                "groups": [],
                "type": 2,
                "collections": [
                    { "id": it, "readOnly": true, "hidePasswords": true, "manage": false },
                    { "id": projects, "readOnly": false, "hidePasswords": false, "manage": false },
                ],
                "permissions": {},
            })),
        )
        .unwrap();
    let members = server
        .raw(
            &mut session,
            "GET",
            &format!("/api/organizations/{org_id}/users"),
            None,
        )
        .unwrap();
    let joined = members["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["email"] == member.as_str())
        .unwrap();
    let public = server
        .raw(
            &mut session,
            "GET",
            &format!(
                "/api/users/{}/public-key",
                joined["userId"].as_str().unwrap()
            ),
            None,
        )
        .unwrap();
    let public = STANDARD
        .decode(public["publicKey"].as_str().unwrap())
        .unwrap();
    server
        .raw(
            &mut session,
            "POST",
            &format!(
                "/api/organizations/{org_id}/users/{}/confirm",
                joined["id"].as_str().unwrap()
            ),
            Some(json!({ "key": encrypt_for(&public, &org_key.to_bytes()).unwrap().to_string() })),
        )
        .unwrap();

    // The owner's items, one in each shared folder, and a template in IT
    // whose hidden field has a default.
    let vault = open(&server, &mut session);
    let field = |name: &str, value: &str, hidden: bool| Field {
        name: name.into(),
        value: Zeroizing::new(value.into()),
        hidden,
        boolean: false,
    };
    let template = ItemDraft {
        note: true,
        name: "Router".into(),
        organization_id: Some(org_id.clone()),
        fields: vec![
            field("Model", "not-a-real-model", false),
            field("Admin PIN", "not-a-real-pin", true),
            field(
                passbundle_core::templates::DEFINITION_FIELD,
                r#"{"id":"office-router","base":"note","v":1}"#,
                false,
            ),
        ],
        ..ItemDraft::default()
    };
    server
        .save_item(
            &mut session,
            None,
            vault.encrypt_item(&template, None).unwrap(),
            std::slice::from_ref(&it),
        )
        .unwrap();
    for (name, password, folder) in [
        ("Office Wi-Fi", "not-a-real-password-it", &it),
        ("Project wiki", "not-a-real-password-wiki", &projects),
    ] {
        let draft = ItemDraft {
            name: name.into(),
            username: Some("not-a-real-user".into()),
            password: secret(password),
            organization_id: Some(org_id.clone()),
            ..ItemDraft::default()
        };
        server
            .save_item(
                &mut session,
                None,
                vault.encrypt_item(&draft, None).unwrap(),
                std::slice::from_ref(folder),
            )
            .unwrap();
    }

    let mut app = signed_in_app(&url, &member);
    let vault = app.vault().unwrap();
    let find = |name: &str| vault.items.iter().find(|i| i.name == name).unwrap();
    let wifi = find("Office Wi-Fi");
    let wiki = find("Project wiki");
    let (wifi_id, wiki_id) = (wifi.id.clone(), wiki.id.clone());
    let hidden = vault.grid(wifi);
    assert!(!hidden.see_passwords && !hidden.edit && !hidden.delete);
    assert!(hidden.view_history);
    let editable = vault.grid(wiki);
    assert!(editable.see_passwords && editable.edit && editable.delete);
    assert!(!vault.collection_grids[&it].create);
    assert!(vault.collection_grids[&projects].create);
    // "Projects/Web" has no "Projects" to sit in: it stays whole, at the top.
    let collections: Vec<(String, String)> = vault
        .collections
        .iter()
        .map(|(id, name)| (id.clone(), name.clone()))
        .collect();
    let nested = passbundle_core::places::nest(&collections);
    assert!(nested.iter().all(|n| n.depth == 0));

    // Its secrets aren't copied or filled, its plain values are, and it
    // isn't edited.
    for what in ["password", "totp"] {
        assert!(matches!(
            app.value_to_copy(&wifi_id, what),
            Err(Error::Refused(_))
        ));
    }
    assert_eq!(
        app.value_to_copy(&wifi_id, "username").unwrap().as_str(),
        "not-a-real-user"
    );
    // The template's hidden default isn't handed to them; its plain one is.
    let groups = passbundle_core::templates::groups(app.vault().unwrap());
    let router = groups
        .iter()
        .flat_map(|g| &g.templates)
        .find(|t| t.id == "office-router")
        .unwrap();
    let default = |label: &str| {
        router
            .fields
            .iter()
            .find(|f| f.label == label)
            .unwrap()
            .value
            .clone()
    };
    assert_eq!(default("Model").as_deref(), Some("not-a-real-model"));
    assert_eq!(default("Admin PIN"), None);
    let draft = ItemDraft::of(app.vault().unwrap().item(&wifi_id).unwrap());
    assert!(app
        .save(Some(&wifi_id), &draft, std::slice::from_ref(&it))
        .is_err());

    // Moving the wiki into IT, where the member can't add items, is refused
    // before anything is saved, as the server would refuse it; edited where
    // it is, it's saved.
    let mut draft = ItemDraft::of(app.vault().unwrap().item(&wiki_id).unwrap());
    draft.name = "Project wiki (team)".into();
    let moved = app.save(Some(&wiki_id), &draft, std::slice::from_ref(&it));
    assert!(matches!(moved, Err(Error::Refused(_))), "{moved:?}");
    assert_eq!(
        app.vault().unwrap().item(&wiki_id).unwrap().name,
        "Project wiki"
    );
    app.save(Some(&wiki_id), &draft, std::slice::from_ref(&projects))
        .unwrap();
    let wiki = app.vault().unwrap().item(&wiki_id).unwrap();
    assert_eq!(wiki.name, "Project wiki (team)");
    assert_eq!(wiki.collection_ids, std::slice::from_ref(&projects));

    // The owner moves it, and it goes.
    let mut owner_app = signed_in_app(&url, &owner);
    let draft = ItemDraft::of(owner_app.vault().unwrap().item(&wiki_id).unwrap());
    owner_app
        .save(Some(&wiki_id), &draft, &[it.clone(), projects.clone()])
        .unwrap();
    let mut held = owner_app
        .vault()
        .unwrap()
        .item(&wiki_id)
        .unwrap()
        .collection_ids
        .clone();
    held.sort();
    let mut both = vec![it.clone(), projects.clone()];
    both.sort();
    assert_eq!(held, both);
}
