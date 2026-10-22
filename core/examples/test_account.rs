//! A throwaway account on a test server, with two logins in its vault, for
//! tests that sign in from elsewhere (the phone app's test,
//! tools/android-app-test.sh). Signs up as an app would, then saves the item
//! through the core.
//!
//!   cargo run --example test_account -- <server> <email> <password>

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
            // A website, for what iOS may suggest above the keyboard.
            uris: vec!["https://phone.example.test/sign-in".into()],
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
    let _ = std::fs::remove_dir_all(dir);
    println!("{email}");
}
