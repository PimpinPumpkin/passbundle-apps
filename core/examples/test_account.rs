//! A throwaway account on a test server, with two logins in its vault (the
//! first with an attachment), for
//! tests that sign in from elsewhere (the phone app's test,
//! tools/android-app-test.sh). Signs up as an app would, then saves the item
//! through the core.
//!
//!   cargo run --example test_account -- <server> <email> <password>

#[path = "../tests/support/attachment.rs"]
mod attachment;
#[path = "../tests/support/sign_up.rs"]
mod support;

use passbundle_core::app::App;
use passbundle_core::secrets::MemorySecretStore;
use passbundle_core::server::{Device, Server};
use passbundle_core::store::Store;
use passbundle_core::vault::ItemDraft;
use zeroize::Zeroizing;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [server_url, email, password] = args.as_slice() else {
        eprintln!("test_account <server> <email> <password>");
        std::process::exit(2);
    };
    let server =
        Server::new(server_url, Device::this_one(&Device::new_identifier())).expect("the server");
    support::sign_up(&server, email, password);

    let dir =
        std::env::temp_dir().join(format!("pb-test-account-{}", uuid::Uuid::new_v4().simple()));
    let mut app = App::open(Store::new(&dir), Box::new(MemorySecretStore::default()));
    app.sign_in(server_url, email, password)
        .expect("signing in");
    app.save(
        None,
        &ItemDraft {
            name: "Saved before the phone signed in".into(),
            username: Some("phone@example.test".into()),
            password: Some(Zeroizing::new("not-a-real-password-phone".into())),
            ..Default::default()
        },
        &[],
    )
    .expect("saving the item");
    // For the autofill test's sign-in form (tools/android-autofill-test.sh):
    // a login for its app, by the app's own androidapp:// link.
    app.save(
        None,
        &ItemDraft {
            name: "Autofill test app".into(),
            username: Some("autofill@example.test".into()),
            password: Some(Zeroizing::new("not-a-real-password-autofill".into())),
            uris: vec!["androidapp://com.passbundle.autofilltest".into()],
            ..Default::default()
        },
        &[],
    )
    .expect("saving the autofill item");
    // And a website's login, which the test app's screen that says it's that
    // website must not get until it's trusted as a browser.
    app.save(
        None,
        &ItemDraft {
            name: "Bank website".into(),
            username: Some("bank@example.test".into()),
            password: Some(Zeroizing::new("not-a-real-password-bank".into())),
            uris: vec!["https://bank.example.test".into()],
            ..Default::default()
        },
        &[],
    )
    .expect("saving the website item");
    // The websites the test app names as its own: one whose asset links name
    // the app back, whose login the app gets; one whose name another key,
    // whose login it doesn't. Each its own registrable domain: a site's
    // logins come by the usual matching rules, so one sharing the bank's
    // would bring the bank's login too, as it would in a browser there.
    for (name, username, site) in [
        (
            "Linked website",
            "linked@example.test",
            "https://linked.test",
        ),
        (
            "Unlinked website",
            "unlinked@example.test",
            "https://unlinked.test",
        ),
    ] {
        app.save(
            None,
            &ItemDraft {
                name: name.into(),
                username: Some(username.into()),
                password: Some(Zeroizing::new("not-a-real-password-linked".into())),
                uris: vec![site.into()],
                ..Default::default()
            },
            &[],
        )
        .expect("saving a linked website's item");
    }
    // An attachment on the first login, for the phone's test of saving one.
    let mut session = match server.sign_in(email, password).expect("signing in") {
        passbundle_core::server::SignIn::Done(session) => session,
        _ => panic!("unexpected two-step login"),
    };
    let (sync, _) = server.sync(&mut session).expect("syncing");
    let vault =
        passbundle_core::vault::Vault::open(&sync, session.user_key().clone()).expect("opening");
    let first = vault
        .items
        .iter()
        .find(|i| i.name == "Saved before the phone signed in")
        .expect("the first login");
    attachment::upload(
        &server,
        server_url,
        &mut session,
        &first.id,
        "notes.txt",
        b"Saved from a phone test.\n",
    );
    let _ = std::fs::remove_dir_all(dir);
    println!("{email}");
}
