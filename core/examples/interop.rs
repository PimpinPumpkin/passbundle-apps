//! The core and the web vault reading each other's items, for the meta repo's
//! end-to-end suite (e2e/tests/core-interop.spec.ts): signs in to a server
//! that uses a throwaway certificate, prints every item it opens as JSON, and
//! then, given a draft, saves it (to the first collection of its organization,
//! if it has one).
//!
//! Passkeys too, through the app's own calls (`App`), as the phones use them
//! (e2e/tests/extension-passkeys.spec.ts): instead of a draft, a file with
//! `{"action": "makePasskey", rpId, rpName, userName, userHandle}` makes one
//! in a new login, and `{"action": "signWithPasskey", rpId, clientDataHash}`
//! signs in with the site's passkey (IDs and hashes in base64url).
//!
//!   cargo run --release --features test-trust --example interop -- \
//!     <server> <certificate.pem> <email> <password> [<draft.json>]
//!
//! Test data only: it prints the secrets it reads.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use passbundle_core::app::{App, NewPasskey};
use passbundle_core::secrets::MemorySecretStore;
use passbundle_core::server::{Device, Server, SignIn};
use passbundle_core::store::Store;
use passbundle_core::vault::{Field, ItemDraft, Vault};
use serde_json::{json, Value};
use zeroize::Zeroizing;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [server, cert, email, password, rest @ ..] = args.as_slice() else {
        eprintln!("interop <server> <certificate.pem> <email> <password> [<draft.json>]");
        std::process::exit(2);
    };
    let pem = std::fs::read(cert).expect("the certificate");
    let server_url = server.as_str();
    let server = Server::new_trusting(server, Device::this_one(&Device::new_identifier()), &pem)
        .expect("the server");
    let SignIn::Done(mut session) = server.sign_in(email, password).expect("signing in") else {
        panic!("two-step login isn't part of this test");
    };
    let (sync, _) = server.sync(&mut session).expect("sync");
    let vault = Vault::open(&sync, session.user_key().clone()).expect("the vault");
    let items: Vec<Value> = vault
        .items
        .iter()
        .map(|i| {
            json!({
                "name": i.name,
                "username": i.username,
                "password": i.password.as_deref(),
                "notes": i.notes.as_deref(),
                "uris": i.uris,
                "organizationId": i.organization_id,
                "fields": i.fields.iter().map(|f| json!({ "name": f.name, "value": f.value.as_str(), "hidden": f.hidden })).collect::<Vec<_>>(),
                "passkey": i.passkey.as_ref().map(|p| json!({
                    "rpId": p.rp_id,
                    "userName": p.user_name,
                    "credentialId": passbundle_core::passkey::credential_id(&p.credential_id).ok().map(|id| URL_SAFE_NO_PAD.encode(id)),
                    "counter": p.counter,
                    "discoverable": p.discoverable,
                    "usable": p.usable,
                })),
            })
        })
        .collect();

    let mut saved = false;
    if let Some(path) = rest.first() {
        let d: Value = serde_json::from_str(&std::fs::read_to_string(path).expect("the draft"))
            .expect("draft JSON");
        if let Some(action) = d["action"].as_str() {
            let answer = passkey(action, &d, server_url, &pem, email, password);
            println!("{}", json!({ "items": items, "passkey": answer }));
            return;
        }
        let text = |k: &str| d[k].as_str().map(str::to_owned);
        let draft = ItemDraft {
            name: text("name").unwrap_or_default(),
            username: text("username"),
            password: text("password").map(Zeroizing::new),
            notes: text("notes").map(Zeroizing::new),
            uris: d["uris"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|u| u.as_str().map(str::to_owned))
                .collect(),
            fields: d["fields"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|f| Field {
                    name: f["name"].as_str().unwrap_or_default().to_owned(),
                    value: Zeroizing::new(f["value"].as_str().unwrap_or_default().to_owned()),
                    hidden: f["hidden"].as_bool().unwrap_or(false),
                })
                .collect(),
            organization_id: text("organizationId"),
            ..ItemDraft::default()
        };
        let collections: Vec<String> = draft
            .organization_id
            .iter()
            .flat_map(|org| {
                vault
                    .collection_orgs
                    .iter()
                    .filter(move |(_, o)| *o == org)
                    .map(|(c, _)| c.clone())
            })
            .take(1)
            .collect();
        let body = vault.encrypt_item(&draft, None).expect("encrypting");
        server
            .save_item(&mut session, None, body, &collections)
            .expect("saving");
        saved = true;
    }
    println!("{}", json!({ "items": items, "saved": saved }));
}

/// A passkey action, through the app's own calls, signed in afresh: making
/// one in a new login, or signing in with the site's.
fn passkey(
    action: &str,
    d: &Value,
    server: &str,
    pem: &[u8],
    email: &str,
    password: &str,
) -> Value {
    let bytes = |key: &str| {
        URL_SAFE_NO_PAD
            .decode(d[key].as_str().unwrap_or_default().trim_end_matches('='))
            .unwrap_or_else(|_| panic!("{key} isn't base64url"))
    };
    let text = |key: &str| d[key].as_str().map(str::to_owned);
    let rp_id = text("rpId").expect("rpId");
    passbundle_core::server::trust_only_for_tests(pem);
    let dir = std::env::temp_dir().join(format!("pb-interop-{}", uuid::Uuid::new_v4().simple()));
    let mut app = App::open(Store::new(&dir), Box::new(MemorySecretStore::default()));
    app.sign_in(server, email, password).expect("signing in");
    let answer = match action {
        "makePasskey" => {
            let (rp_name, user_name) = (text("rpName"), text("userName"));
            let made = app
                .passkey_create(&NewPasskey {
                    rp_id: &rp_id,
                    rp_name: rp_name.as_deref(),
                    user_handle: &bytes("userHandle"),
                    user_name: user_name.as_deref(),
                    user_display_name: None,
                    user_verified: true,
                    discoverable: true,
                    login: None,
                    exclude: &[],
                })
                .expect("making a passkey");
            json!({
                "itemId": made.item_id,
                "credentialId": URL_SAFE_NO_PAD.encode(&made.credential_id),
                "publicKey": URL_SAFE_NO_PAD.encode(&made.public_key),
            })
        }
        "signWithPasskey" => {
            let hash: [u8; 32] = bytes("clientDataHash").try_into().expect("a 32-byte hash");
            let id = app
                .passkeys_for(&rp_id)
                .first()
                .map(|item| item.id.clone())
                .expect("a passkey for the site");
            let signed = app
                .passkey_assert(&id, &rp_id, &hash, true)
                .expect("signing in with the passkey");
            json!({
                "authenticatorData": URL_SAFE_NO_PAD.encode(&signed.authenticator_data),
                "signature": URL_SAFE_NO_PAD.encode(&signed.signature),
                "credentialId": URL_SAFE_NO_PAD.encode(&signed.credential_id),
                "userHandle": URL_SAFE_NO_PAD.encode(&signed.user_handle),
            })
        }
        other => panic!("no such action: {other}"),
    };
    let _ = std::fs::remove_dir_all(dir);
    answer
}
