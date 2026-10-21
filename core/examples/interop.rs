//! The core and the web vault reading each other's items, for the meta repo's
//! end-to-end suite (e2e/tests/core-interop.spec.ts): signs in to a server
//! that uses a throwaway certificate, prints every item it opens as JSON, and
//! then, given a draft, saves it (to the first collection of its organization,
//! if it has one).
//!
//!   cargo run --release --features test-trust --example interop -- \
//!     <server> <certificate.pem> <email> <password> [<draft.json>]
//!
//! Test data only: it prints the secrets it reads.

use passbundle_core::server::{Device, Server, SignIn};
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
            })
        })
        .collect();

    let mut saved = false;
    if let Some(path) = rest.first() {
        let d: Value = serde_json::from_str(&std::fs::read_to_string(path).expect("the draft"))
            .expect("draft JSON");
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
