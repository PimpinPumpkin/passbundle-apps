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
use passbundle_core::vault::{Field, ItemDraft, Vault};
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

/// Signs up as an app would: the master key and login hash from the password,
/// a new user key sealed with the stretched master key, and a new RSA key pair
/// with its private key under the user key.
fn sign_up(server: &Server, email: &str) {
    let master = MasterKey::derive(
        PASSWORD,
        email,
        &Kdf::Pbkdf2 {
            iterations: 600_000,
        },
    )
    .unwrap();
    let user_key = SymmetricKey::generate();
    let private = PrivateKey::generate().unwrap();
    let token = server
        .raw_post(
            "/identity/accounts/register/send-verification-email",
            json!({ "email": email, "name": "Core test", "receiveMarketingEmails": false }),
        )
        .unwrap();
    server
        .raw_post(
            "/identity/accounts/register/finish",
            json!({
                "email": email,
                "emailVerificationToken": token,
                "masterPasswordHash": master.login_hash(PASSWORD),
                "masterPasswordHint": null,
                "userSymmetricKey": master.stretch().encrypt(&user_key.to_bytes()).to_string(),
                "userAsymmetricKeys": {
                    "publicKey": STANDARD.encode(private.public_key_der()),
                    "encryptedPrivateKey": user_key.encrypt(&private.to_pkcs8_der()).to_string(),
                },
                "kdf": 0,
                "kdfIterations": 600000,
            }),
        )
        .unwrap();
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
