//! Where a new item may go (`App::new_item_places`), the one rule the
//! editor, Android's autofill saving into it, and a new passkey's login
//! keep to: Personal, unless an organization's "Remove individual vault"
//! (upstream's policy 5) binds the person, and then only a shared folder
//! they may add items to, starting in one where they may also edit items
//! and see passwords. Its owners and admins aren't bound. Saving keeps to
//! it before anything is sent: nothing new in Personal, and no change to
//! the person's own items, as the server refuses both. On the vectors'
//! sample vault, signed in to a stand-in for the server.

use passbundle_core::app::{App, NewItemPlaces};
use passbundle_core::crypto::SymmetricKey;
use passbundle_core::error::Error;
use passbundle_core::vault::ItemDraft;
use serde_json::{json, Value};

#[path = "support/stand_in.rs"]
mod stand_in;
use stand_in::{stand_in, Sample, IT, MAIL, OFFICE_WIFI, ORG};

/// Contoso's second shared folder for the sample, Accounting (before IT
/// by name), its name under Contoso's key.
const ACCOUNTING: &str = "7c8d9e0f-1a2b-4c3d-8e4f-5a6b7c8d9e0f";

const NOT_IN_PERSONAL: &str = "your organization keeps items out of Personal, so PassBundle can't save it there. Choose a shared folder you can add to, or ask an admin for one";
const OWN_ITEM_KEPT_OUT: &str = "your organization keeps items out of Personal, so this item can't be changed. You can move it to a shared folder in the web vault";

/// What a shared folder lets the person do: add and edit items or only
/// see them, and see their passwords or not.
fn grid(add: bool, see_passwords: bool) -> Value {
    json!({
        "seePasswords": see_passwords, "create": add, "edit": add, "delete": add,
        "export": true, "share": add && see_passwords, "print": see_passwords,
        "viewHistory": true, "manage": false,
    })
}

/// Contoso's Accounting, giving the person `grid`.
fn accounting(grid: Value) -> Value {
    use base64::Engine;
    let v = stand_in::core_vectors();
    let key = SymmetricKey::from_bytes(
        &base64::engine::general_purpose::STANDARD
            .decode(v["keys"]["organizationKey"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap();
    json!({
        "id": ACCOUNTING, "organizationId": ORG, "readOnly": false,
        "name": key.encrypt_str("Accounting").to_string(), "keykeeper": grid,
    })
}

/// The sample, the person's role in Contoso `role` (upstream's numbers: 0
/// owner, 1 admin, 2 user), with Contoso's "Remove individual vault" on or
/// off, IT giving them `it`, and Accounting beside it when given.
fn sample(url: &str, role: u64, policy: bool, it: Value, also: Option<Value>) -> Sample {
    Sample::new(url, &[], move |sync| {
        sync["profile"]["organizations"][0]["type"] = role.into();
        sync["policies"] = json!([{ "organizationId": ORG, "type": 5, "enabled": policy }]);
        sync["collections"][0]["keykeeper"] = it;
        if let Some(grid) = also {
            sync["collections"]
                .as_array_mut()
                .unwrap()
                .push(accounting(grid));
        }
    })
}

fn places(app: &App) -> NewItemPlaces {
    app.new_item_places()
}

fn shared(ids: &[&str]) -> Vec<(String, String)> {
    ids.iter()
        .map(|f| (ORG.to_owned(), (*f).to_owned()))
        .collect()
}

#[test]
fn where_an_organization_keeps_items_out_of_personal_a_new_item_goes_only_in_a_shared_folder() {
    let (url, _) = stand_in(vec![]);

    // Not bound: Personal, as ever, and nobody's start is a shared folder.
    let free = sample(&url, 2, false, grid(true, true), None).unlocked();
    let anywhere = places(&free);
    assert_eq!(anywhere, NewItemPlaces::default());
    assert!(anywhere.personal());
    assert!(!anywhere.nowhere());

    // A member it binds: not Personal, only IT, and a new item starts there.
    let member = sample(&url, 2, true, grid(true, true), None).unlocked();
    let bound = places(&member);
    assert!(!bound.personal());
    assert!(!bound.nowhere());
    assert_eq!(bound.keeps_items_out.as_deref(), Some("Contoso"));
    assert_eq!(bound.shared, shared(&[IT]));
    assert_eq!(bound.start, Some((ORG.to_owned(), IT.to_owned())));

    // Its owners and admins aren't bound: Personal for them.
    for role in [0, 1] {
        let app = sample(&url, role, true, grid(true, true), None).unlocked();
        assert_eq!(places(&app), NewItemPlaces::default(), "role {role}");
    }
}

#[test]
fn a_shared_folder_the_person_can_only_see_takes_no_new_item() {
    let (url, _) = stand_in(vec![]);

    // IT they may only see: nowhere at all.
    let viewer = sample(&url, 2, true, grid(false, true), None).unlocked();
    let none = places(&viewer);
    assert!(!none.personal());
    assert!(none.nowhere());
    assert!(none.shared.is_empty());
    assert_eq!(none.start, None);

    // Beside Accounting, which they may add to: only Accounting.
    let one = sample(&url, 2, true, grid(false, true), Some(grid(true, true))).unlocked();
    let only = places(&one);
    assert_eq!(only.shared, shared(&[ACCOUNTING]));
    assert_eq!(only.start, Some((ORG.to_owned(), ACCOUNTING.to_owned())));

    // Both take new items, Accounting first by name but hiding passwords:
    // both are offered, in the import's order, and a new one starts in IT.
    let two = sample(&url, 2, true, grid(true, true), Some(grid(true, false))).unlocked();
    let both = places(&two);
    assert_eq!(both.shared, shared(&[ACCOUNTING, IT]));
    assert_eq!(both.start, Some((ORG.to_owned(), IT.to_owned())));
}

#[test]
fn saving_keeps_a_bound_persons_new_items_out_of_personal_and_their_own_unchanged() {
    let (url, asked) = stand_in(vec![]);
    let saves = |path: &str| -> Vec<Value> {
        asked
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p == path)
            .map(|(_, body)| body.clone())
            .collect()
    };
    let new = |organization: Option<&str>| ItemDraft {
        name: "Router".into(),
        uris: vec!["https://router.example.test".into()],
        organization_id: organization.map(str::to_owned),
        ..ItemDraft::default()
    };
    // Each save syncs with the stand-in, which keeps no policy: a sample
    // of its own for each.
    let bound = || sample(&url, 2, true, grid(true, true), None);
    let first = bound();
    let mut member = first.unlocked();

    // Their own Example Mail is kept out; Contoso's Office Wi-Fi isn't.
    let vault = member.vault().unwrap();
    assert_eq!(vault.keeps_out(vault.item(MAIL).unwrap()), Some("Contoso"));
    assert_eq!(vault.keeps_out(vault.item(OFFICE_WIFI).unwrap()), None);

    // A new item in Personal, and a change to Example Mail: both refused,
    // in plain words, before anything is sent.
    assert_eq!(
        member.save(None, &new(None), &[]).err(),
        Some(Error::Refused(NOT_IN_PERSONAL))
    );
    let mail = ItemDraft::of(member.vault().unwrap().item(MAIL).unwrap());
    assert_eq!(
        member.save(Some(MAIL), &mail, &[]).err(),
        Some(Error::Refused(OWN_ITEM_KEPT_OUT))
    );
    assert!(asked
        .lock()
        .unwrap()
        .iter()
        .all(|(p, _)| !p.starts_with("POST /api/ciphers") && !p.starts_with("PUT ")));

    // In IT, a new item is Contoso's, sent with IT.
    member
        .save(None, &new(Some(ORG)), &[IT.to_owned()])
        .unwrap();
    let created = saves("POST /api/ciphers/create");
    assert_eq!(created.len(), 1);
    assert_eq!(created[0]["collectionIds"], json!([IT]));
    assert_eq!(created[0]["cipher"]["organizationId"], ORG);

    // Contoso's own items change as ever.
    let second = bound();
    let mut member = second.unlocked();
    assert!(member.vault().unwrap().keeps_items_out.is_some());
    let wifi = ItemDraft::of(member.vault().unwrap().item(OFFICE_WIFI).unwrap());
    member.save(Some(OFFICE_WIFI), &wifi, &[]).unwrap();
    assert_eq!(saves(&format!("PUT /api/ciphers/{OFFICE_WIFI}")).len(), 1);

    // An owner isn't bound: theirs goes in Personal, and their own items
    // change.
    let owners = sample(&url, 0, true, grid(true, true), None);
    let mut owner = owners.unlocked();
    let vault = owner.vault().unwrap();
    assert_eq!(vault.keeps_out(vault.item(MAIL).unwrap()), None);
    owner.save(None, &new(None), &[]).unwrap();
    let personal = saves("POST /api/ciphers");
    assert_eq!(personal.len(), 1);
    assert!(personal[0]["organizationId"].is_null());
    let mut owner = owners.unlocked();
    let mail = ItemDraft::of(owner.vault().unwrap().item(MAIL).unwrap());
    owner.save(Some(MAIL), &mail, &[]).unwrap();
    assert_eq!(saves(&format!("PUT /api/ciphers/{MAIL}")).len(), 1);
}
