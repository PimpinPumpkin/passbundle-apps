//! Editing an item keeps what the app doesn't show: how each of its websites
//! is matched for filling, which the web vault sets (src/matching.rs). On the
//! vectors' sample vault, with one website's match set as the web vault would.

use passbundle_core::kdf::Prelogin;
use passbundle_core::matching::{matches, Target};
use passbundle_core::vault::{ItemDraft, Sync, Vault};

#[test]
fn editing_keeps_how_each_website_is_matched() {
    let mut v: serde_json::Value =
        serde_json::from_str(include_str!("vectors/core-v1.json")).unwrap();
    let pre = v["account"]["pbkdf2"]["prelogin"].clone();
    let password = v["account"]["password"].as_str().unwrap().to_owned();
    let id = "3d4e5f60-7182-4930-abc1-d2e3f4051627";
    let cipher = v["sync"]["ciphers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["id"] == id)
        .unwrap();
    // "Host": the same host only.
    cipher["login"]["uris"][0]["match"] = 1.into();
    let sync = Sync::from_json(&v["sync"].to_string()).unwrap();
    let prelogin: Prelogin = serde_json::from_value(pre).unwrap();
    let vault = Vault::unlock(&sync, &prelogin, &password).unwrap();
    let item = vault.item(id).unwrap();
    assert_eq!(item.uri_matches, vec![Some(1)]);

    // Edited, with the website kept and another added.
    let mut draft = ItemDraft::of(item);
    draft.name = "Renamed".into();
    draft.uris.push("https://new.example.test".into());
    let body = vault.encrypt_item(&draft, Some(item)).unwrap();
    let uris = body["login"]["uris"].as_array().unwrap();
    assert_eq!(uris[0]["match"], 1, "the kept website keeps its match");
    assert!(uris[1]["match"].is_null(), "a new one has the default");

    // And the setting counts when filling: its host (mail.example.test), not its whole domain.
    assert_eq!(item.uris[0], "https://mail.example.test/login");
    assert!(matches(item, &Target::Website("mail.example.test")));
    assert!(!matches(item, &Target::Website("other.example.test")));
}
