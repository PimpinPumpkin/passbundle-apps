//! Where a new passkey goes on a phone (docs/core.md, "Passkeys", "Where it
//! goes"), on the vectors' sample vault kept on this device, against a
//! stand-in for the server (tests/support/stand_in.rs): the logins offered
//! for it (in use, that the person may change, for the site and without a
//! passkey, or with this account's passkey for the site, which it
//! replaces), and making one into a login, in place of its old passkey or
//! beside its password, never into one the phones wouldn't offer; a new
//! login goes in a shared folder where an organization keeps items out of
//! Personal, and isn't made, or offered, with no shared folder to add to
//! (the phones hear why before they'd offer one). A passkey that counts,
//! in a login of the person's own that an organization keeps out of
//! Personal, is refused before it signs, since its raised counter can't be
//! saved there. The
//! sample's Backup service login has a passkey for backup.example.test; its
//! Example Mail login is on mail.example.test, the same site by the default
//! match (example.test).

use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use p256::pkcs8::DecodePublicKey;
use passbundle_core::app::{App, NewPasskey, PasskeyMade, ALREADY_HAS_PASSKEY};
use passbundle_core::crypto::{EncString, SymmetricKey};
use passbundle_core::error::Error;
use passbundle_core::matching::PasskeySite;
use passbundle_core::passkey::{base64_either, credential_id};
use passbundle_core::vault::ItemDraft;
use serde_json::{json, Value};

#[path = "support/stand_in.rs"]
mod stand_in;
use stand_in::{sent, stand_in, user_key, Asked, Sample, BACKUP, GATE, IT, MAIL, OFFICE_WIFI, ORG};

const SITE: &str = "backup.example.test";
/// The sample's passkey, as Backup service stores it.
const OLD_CREDENTIAL: &str = "0c2f6a8e-4d1b-4f3a-9e7c-5b8d2a1f6e40";

/// The account on backup.example.test that the sample's passkey is for:
/// its user handle, `oKGio6SlpqeoqaqrrK2urw` as stored.
fn ana() -> Vec<u8> {
    (0xa0..=0xaf).collect()
}

/// Another account on the same site.
const BEA: [u8; 16] = [1; 16];

fn site<'a>(rp_id: &'a str, user_handle: &'a [u8]) -> PasskeySite<'a> {
    PasskeySite {
        rp_id,
        user_handle,
        app: None,
    }
}

/// The logins offered for a new passkey, by id, each with whether it
/// replaces the login's passkey.
fn offered(app: &App, site: &PasskeySite<'_>) -> Vec<(String, bool)> {
    app.vault()
        .unwrap()
        .passkey_logins(site)
        .into_iter()
        .map(|(i, replaces)| (i.id.clone(), replaces))
        .collect()
}

fn ids(list: &[(&str, bool)]) -> Vec<(String, bool)> {
    list.iter().map(|(id, r)| ((*id).to_owned(), *r)).collect()
}

fn new_passkey<'a>(
    rp_id: &'a str,
    user_handle: &'a [u8],
    login: Option<&'a str>,
    exclude: &'a [Vec<u8>],
) -> NewPasskey<'a> {
    NewPasskey {
        rp_id,
        rp_name: None,
        user_handle,
        user_name: Some("ana@backup.example.test"),
        user_display_name: None,
        user_verified: true,
        discoverable: true,
        login,
        exclude,
        app: None,
    }
}

/// How many times the stand-in was asked to save an item.
fn saves(asked: &Asked) -> usize {
    sent(asked, "PUT /api/ciphers").len() + sent(asked, "POST /api/ciphers").len()
}

/// The item's passkey, as stored.
fn credential(app: &App, id: &str) -> String {
    app.vault()
        .unwrap()
        .item(id)
        .unwrap()
        .passkey
        .as_ref()
        .unwrap()
        .credential_id
        .clone()
}

/// Signs in with a login's passkey, and checks the signature with the
/// public key the site was given when it was made.
fn signs_with(app: &mut App, id: &str, rp_id: &str, made: &PasskeyMade) {
    let hash = [7u8; 32];
    let signed = app.passkey_assert(id, rp_id, &hash, true).unwrap();
    assert_eq!(signed.credential_id, made.credential_id);
    let mut message = signed.authenticator_data.clone();
    message.extend_from_slice(&hash);
    let public = VerifyingKey::from_public_key_der(&made.public_key).unwrap();
    let signature = Signature::from_der(&signed.signature).unwrap();
    assert!(public.verify(&message, &signature).is_ok());
}

/// An item's websites as the sync has them, encrypted with its key as the
/// app would save them (`login.uris`), to keep on the device in their place.
fn websites(url: &str, id: &str, uris: &[&str]) -> Value {
    let sample = Sample::new(url, &[], |_| {});
    let app = sample.unlocked();
    let vault = app.vault().unwrap();
    let item = vault.item(id).unwrap();
    let mut draft = ItemDraft::of(item);
    draft.uris = uris.iter().map(|u| (*u).to_owned()).collect();
    vault.encrypt_item(&draft, Some(item)).unwrap()["login"]["uris"].clone()
}

/// The sample with an item's websites changed (`websites`), and the sync
/// changed further by `edit`.
fn with_websites(url: &str, id: &str, uris: Value, edit: impl FnOnce(&mut Value)) -> Sample {
    Sample::new(url, &[], move |sync| {
        for c in sync["ciphers"].as_array_mut().unwrap() {
            if c["id"] == id {
                c["login"]["uris"] = uris.clone();
            }
        }
        edit(sync);
    })
}

#[test]
fn a_new_passkey_is_offered_the_login_it_replaces_first_then_the_sites_logins() {
    let (url, _) = stand_in(vec![]);
    let sample = Sample::new(&url, &[], |_| {});
    let app = sample.unlocked();
    let ana = ana();
    let vault = app.vault().unwrap();
    // The sample's passkey is for this account, by its user handle.
    let stored = vault.item(BACKUP).unwrap().passkey.as_ref().unwrap();
    assert_eq!(
        base64_either(stored.user_handle.as_deref().unwrap()),
        Some(ana.clone())
    );

    // The same site and account: Backup service, whose passkey the new one
    // replaces, first; then Example Mail, for example.test, with no passkey.
    assert_eq!(
        offered(&app, &site(SITE, &ana)),
        ids(&[(BACKUP, true), (MAIL, false)])
    );
    assert_eq!(
        vault.passkey_target(vault.item(BACKUP).unwrap(), &site(SITE, &ana)),
        Ok(true)
    );
    assert_eq!(
        vault.passkey_target(vault.item(MAIL).unwrap(), &site(SITE, &ana)),
        Ok(false)
    );
    // Another account on the site: Backup service's passkey is the other
    // account's, which it would lose, so only Example Mail is offered.
    assert_eq!(offered(&app, &site(SITE, &BEA)), ids(&[(MAIL, false)]));
    assert_eq!(
        vault.passkey_target(vault.item(BACKUP).unwrap(), &site(SITE, &BEA)),
        Err(Error::Refused(
            "that login has a passkey for another account or site"
        ))
    );
    // The same account on another of example.test's sites: a passkey is
    // bound to its site's ID, so Backup service's isn't this one's.
    assert_eq!(
        offered(&app, &site("mail.example.test", &ana)),
        ids(&[(MAIL, false)])
    );
    // A site none of the logins is for: nothing, so a new login.
    assert!(offered(&app, &site("elsewhere.test", &ana)).is_empty());
    assert_eq!(
        vault.passkey_target(vault.item(MAIL).unwrap(), &site("elsewhere.test", &ana)),
        Err(Error::Refused("that login isn't for this site"))
    );
    // Office Wi-Fi has no website; Gate code isn't a login.
    assert_eq!(
        vault.passkey_target(vault.item(OFFICE_WIFI).unwrap(), &site(SITE, &ana)),
        Err(Error::Refused("that login isn't for this site"))
    );
    assert_eq!(
        vault.passkey_target(vault.item(GATE).unwrap(), &site(SITE, &ana)),
        Err(Error::Refused("a passkey goes in a login"))
    );
    // The same account is by the user handle's bytes; an empty one is no
    // account's.
    assert!(passbundle_core::matching::has_passkey_for(
        vault.item(BACKUP).unwrap(),
        &site(SITE, &ana)
    ));
    assert!(!passbundle_core::matching::has_passkey_for(
        vault.item(BACKUP).unwrap(),
        &site(SITE, &[])
    ));
}

#[test]
fn replacing_the_accounts_old_passkey_keeps_its_login_and_password() {
    let (url, asked) = stand_in(vec![]);
    let sample = Sample::new(&url, &[], |_| {});
    let mut app = sample.unlocked();
    let ana = ana();
    let old = app.passkey_assert(BACKUP, SITE, &[7u8; 32], true).unwrap();
    assert_eq!(old.credential_id, credential_id(OLD_CREDENTIAL).unwrap());

    let made = app
        .passkey_create(&new_passkey(SITE, &ana, Some(BACKUP), &[]))
        .unwrap();
    assert_eq!(made.item_id, BACKUP);
    assert_ne!(made.credential_id, old.credential_id);
    // Saved once, as an edit of Backup service, before the site hears of it.
    assert_eq!(saves(&asked), 1);
    assert_eq!(sent(&asked, &format!("PUT /api/ciphers/{BACKUP}")).len(), 1);

    // The login as it was, with the new passkey in place of the old one.
    let vault = app.vault().unwrap();
    let item = vault.item(BACKUP).unwrap();
    assert_eq!(item.name, "Backup service");
    assert_eq!(item.username.as_deref(), Some("backup@example.test"));
    assert_eq!(
        item.password.as_deref().map(String::as_str),
        Some("not-a-real-password-3")
    );
    assert_eq!(item.uris, vec!["https://backup.example.test".to_owned()]);
    let passkey = item.passkey.as_ref().unwrap();
    assert_eq!(
        credential_id(&passkey.credential_id).unwrap(),
        made.credential_id
    );
    assert_eq!(passkey.rp_id, SITE);
    assert_eq!(
        base64_either(passkey.user_handle.as_deref().unwrap()),
        Some(ana.clone())
    );
    // One passkey for the site and the account, and it's the new one: the
    // old one is gone, so it can't sign in again.
    assert_eq!(app.passkeys_for(SITE).len(), 1);
    signs_with(&mut app, BACKUP, SITE, &made);
    // Making one again, it's the login to replace now.
    assert_eq!(
        offered(&app, &site(SITE, &ana)),
        ids(&[(BACKUP, true), (MAIL, false)])
    );
}

#[test]
fn a_passkey_goes_into_a_login_for_the_site_beside_its_password() {
    let (url, asked) = stand_in(vec![]);
    let sample = Sample::new(&url, &[], |_| {});
    let mut app = sample.unlocked();
    let handle = [5u8; 16];
    let made = app
        .passkey_create(&NewPasskey {
            user_name: Some("ana@mail.example.test"),
            ..new_passkey("mail.example.test", &handle, Some(MAIL), &[])
        })
        .unwrap();
    assert_eq!(made.item_id, MAIL);
    assert_eq!(sent(&asked, &format!("PUT /api/ciphers/{MAIL}")).len(), 1);
    let vault = app.vault().unwrap();
    let item = vault.item(MAIL).unwrap();
    // Its name, username, password, code, and fields as they were: the
    // username is filled in only where there's none.
    assert_eq!(item.name, "Example Mail");
    assert_eq!(item.username.as_deref(), Some("ana.lopez@example.test"));
    assert_eq!(
        item.password.as_deref().map(String::as_str),
        Some("not-a-real-password-1")
    );
    assert!(item.totp.is_some());
    assert_eq!(item.fields.len(), 2);
    let passkey = item.passkey.as_ref().unwrap();
    assert_eq!(passkey.rp_id, "mail.example.test");
    assert_eq!(passkey.user_name.as_deref(), Some("ana@mail.example.test"));
    // Backup service's passkey is as it was.
    assert_eq!(credential(&app, BACKUP), OLD_CREDENTIAL);
    signs_with(&mut app, MAIL, "mail.example.test", &made);
}

#[test]
fn a_login_the_phones_dont_offer_never_takes_a_passkey() {
    let (url, asked) = stand_in(vec![]);
    let sample = Sample::new(&url, &[], |_| {});
    let mut app = sample.unlocked();
    let mut refused = |new: NewPasskey<'_>| app.passkey_create(&new).err();
    // Another account's passkey for the site would be lost.
    assert_eq!(
        refused(new_passkey(SITE, &BEA, Some(BACKUP), &[])),
        Some(Error::Refused(
            "that login has a passkey for another account or site"
        ))
    );
    // A login for another site, and an item that isn't a login.
    assert_eq!(
        refused(new_passkey("elsewhere.test", &BEA, Some(MAIL), &[])),
        Some(Error::Refused("that login isn't for this site"))
    );
    assert_eq!(
        refused(new_passkey(SITE, &BEA, Some(GATE), &[])),
        Some(Error::Refused("a passkey goes in a login"))
    );
    assert_eq!(
        refused(new_passkey(SITE, &BEA, Some("no-such-item"), &[])),
        Some(Error::Damaged("no such item"))
    );
    assert_eq!(saves(&asked), 0);
    assert_eq!(credential(&app, BACKUP), OLD_CREDENTIAL);
}

#[test]
fn an_archived_login_or_one_in_the_trash_is_never_offered() {
    let (url, asked) = stand_in(vec![]);
    let sample = Sample::new(&url, &[BACKUP], |sync| {
        for c in sync["ciphers"].as_array_mut().unwrap() {
            if c["id"] == MAIL {
                c["deletedDate"] = "2026-11-02T10:00:00.000Z".into();
            }
        }
    });
    let mut app = sample.unlocked();
    let ana = ana();
    assert!(offered(&app, &site(SITE, &ana)).is_empty());
    let vault = app.vault().unwrap();
    assert_eq!(
        vault.passkey_target(vault.item(BACKUP).unwrap(), &site(SITE, &ana)),
        Err(Error::Refused("that login is archived"))
    );
    assert_eq!(
        vault.passkey_target(vault.item(MAIL).unwrap(), &site(SITE, &ana)),
        Err(Error::Refused("that login is in the trash"))
    );
    assert_eq!(
        app.passkey_create(&new_passkey(SITE, &ana, Some(BACKUP), &[]))
            .err(),
        Some(Error::Refused("that login is archived"))
    );
    assert_eq!(saves(&asked), 0);
}

#[test]
fn a_shared_folder_the_person_can_only_view_or_that_hides_passwords_never_takes_one() {
    let (url, asked) = stand_in(vec![]);
    let office = websites(&url, OFFICE_WIFI, &["https://office.example.test"]);
    let grid = |edit: bool, see_passwords: bool| {
        json!({
            "seePasswords": see_passwords, "create": edit, "edit": edit, "delete": edit,
            "export": true, "share": edit, "print": see_passwords, "viewHistory": true, "manage": false,
        })
    };
    let handle = [3u8; 16];
    let new = site("office.example.test", &handle);
    let in_folder = |grid: Value| {
        with_websites(&url, OFFICE_WIFI, office.clone(), move |sync| {
            sync["collections"][0]["keykeeper"] = grid;
        })
    };

    // Where the person may change it, Contoso's Office Wi-Fi is offered
    // beside their own login for example.test.
    let sample = in_folder(grid(true, true));
    let app = sample.unlocked();
    assert_eq!(
        offered(&app, &new),
        ids(&[(MAIL, false), (OFFICE_WIFI, false)])
    );

    // A shared folder they can only view, and one that hides passwords
    // (which the apps' editor doesn't edit either): never offered, and
    // refused if asked for.
    for (folder, why) in [
        (
            grid(false, true),
            "your organization doesn't let you change that login",
        ),
        (
            grid(true, false),
            "you can't see that login's passwords, so a passkey can't go in it",
        ),
    ] {
        let sample = in_folder(folder);
        let mut app = sample.unlocked();
        assert_eq!(offered(&app, &new), ids(&[(MAIL, false)]));
        assert_eq!(
            app.passkey_create(&new_passkey(
                "office.example.test",
                &handle,
                Some(OFFICE_WIFI),
                &[]
            ))
            .err(),
            Some(Error::Refused(why))
        );
    }
    assert_eq!(saves(&asked), 0);
}

#[test]
fn where_a_policy_removes_logins_none_is_offered_and_no_new_one_is_made() {
    let (url, asked) = stand_in(vec![]);
    let policy = |data: Value| {
        move |sync: &mut Value| {
            sync["policies"] = json!([{
                "id": "a1b2c3d4-0000-4000-8000-000000000015",
                "organizationId": ORG,
                "type": 15,
                "enabled": true,
                "data": data,
            }]);
        }
    };
    let sample = Sample::new(&url, &[], policy(json!([1])));
    let mut app = sample.unlocked();
    let ana = ana();
    // Contoso removes logins, its own included: none is offered.
    assert!(offered(&app, &site(SITE, &ana)).is_empty());
    assert!(offered(&app, &site("office.example.test", &ana)).is_empty());
    // Asked for one, it isn't there; nor is a new login made.
    assert_eq!(
        app.passkey_create(&new_passkey(SITE, &ana, Some(BACKUP), &[]))
            .err(),
        Some(Error::Damaged("no such item"))
    );
    assert_eq!(
        app.passkey_create(&new_passkey(SITE, &ana, None, &[]))
            .err(),
        Some(Error::Refused(
            "an organization you're in doesn't allow that kind of item"
        ))
    );
    assert_eq!(saves(&asked), 0);

    // Cards only (the policy's default): logins are offered as ever.
    let cards = Sample::new(&url, &[], policy(Value::Null));
    assert_eq!(
        offered(&cards.unlocked(), &site(SITE, &ana)),
        ids(&[(BACKUP, true), (MAIL, false)])
    );
}

/// The sample with Contoso keeping items out of Personal (upstream's
/// "Remove individual vault", policy 5), for a member it binds (`member`;
/// its owners and admins aren't bound, and the sample's person owns it),
/// whose shared folder IT takes new items or not (`create`).
fn keeping_items_out(url: &str, member: bool, create: bool) -> Sample {
    Sample::new(url, &[], move |sync| {
        if member {
            sync["profile"]["organizations"][0]["type"] = 2.into();
        }
        sync["policies"] = json!([{ "organizationId": ORG, "type": 5, "enabled": true }]);
        sync["collections"][0]["keykeeper"]["create"] = create.into();
    })
}

#[test]
fn where_an_organization_keeps_items_out_of_personal_a_new_login_goes_in_a_shared_folder() {
    let (url, asked) = stand_in(vec![]);
    let handle = [4u8; 16];
    let sample = keeping_items_out(&url, true, true);
    let mut app = sample.unlocked();
    let made = app
        .passkey_create(&new_passkey("new.example.test", &handle, None, &[]))
        .unwrap();
    // One save: an organization's item, in Contoso's shared folder IT, as
    // an import goes there, encrypted with Contoso's key, not the person's.
    assert_eq!(saves(&asked), 1);
    let created = sent(&asked, "POST /api/ciphers/create");
    assert_eq!(created.len(), 1);
    assert_eq!(created[0]["collectionIds"], json!([IT]));
    let cipher = &created[0]["cipher"];
    assert_eq!(cipher["organizationId"], ORG);
    assert!(cipher["folderId"].is_null());
    assert!(EncString::parse(cipher["name"].as_str().unwrap())
        .unwrap()
        .decrypt_string(&user_key())
        .is_err());
    // The login as the server keeps it: Contoso's, in IT, named after the
    // site, with the account's name; and its passkey signs.
    let vault = app.vault().unwrap();
    let item = vault.item(&made.item_id).unwrap();
    assert_eq!(item.organization_id.as_deref(), Some(ORG));
    assert_eq!(item.collection_ids, [IT]);
    assert_eq!(item.name, "new.example.test");
    assert_eq!(item.username.as_deref(), Some("ana@backup.example.test"));
    assert_eq!(item.passkey.as_ref().unwrap().rp_id, "new.example.test");
    signs_with(&mut app, &made.item_id, "new.example.test", &made);
}

/// Contoso's second shared folder for the sample, Accounting (before IT
/// by name), its name under Contoso's key, giving the person `grid`.
const ACCOUNTING: &str = "7c8d9e0f-1a2b-4c3d-8e4f-5a6b7c8d9e0f";

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

/// What a shared folder lets the person do: add and edit items, and see
/// their passwords or not.
fn folder_grid(create: bool, see_passwords: bool) -> Value {
    json!({
        "seePasswords": see_passwords, "create": create, "edit": create, "delete": create,
        "export": true, "share": create, "print": see_passwords, "viewHistory": true, "manage": false,
    })
}

#[test]
fn a_new_login_goes_in_a_shared_folder_that_shows_passwords_before_one_that_hides_them() {
    let (url, asked) = stand_in(vec![]);
    let handle = [4u8; 16];
    // A member Contoso keeps out of Personal, with Accounting (first by
    // name, hiding passwords) and IT (`it`).
    let with_folders = |it: Value| {
        Sample::new(&url, &[], move |sync| {
            sync["profile"]["organizations"][0]["type"] = 2.into();
            sync["policies"] = json!([{ "organizationId": ORG, "type": 5, "enabled": true }]);
            sync["collections"][0]["keykeeper"] = it;
            sync["collections"]
                .as_array_mut()
                .unwrap()
                .push(accounting(folder_grid(true, false)));
        })
    };
    let sample = with_folders(folder_grid(true, true));
    let mut app = sample.unlocked();
    let order: Vec<String> = app
        .import_shared_folders()
        .into_iter()
        .map(|(_, name, ..)| name)
        .collect();
    assert_eq!(order, ["Accounting", "IT"]);
    // IT, where a later passkey for the account can take this one's place.
    app.passkey_create(&new_passkey("new.example.test", &handle, None, &[]))
        .unwrap();
    let created = sent(&asked, "POST /api/ciphers/create");
    assert_eq!(created[0]["collectionIds"], json!([IT]));

    // Where IT doesn't take new items, Accounting, the only folder left,
    // does: the passkey signs in from there, though it hides passwords.
    let sample = with_folders(folder_grid(false, true));
    let mut app = sample.unlocked();
    let made = app
        .passkey_create(&new_passkey("new.example.test", &handle, None, &[]))
        .unwrap();
    let created = sent(&asked, "POST /api/ciphers/create");
    assert_eq!(created.len(), 2);
    assert_eq!(created[1]["collectionIds"], json!([ACCOUNTING]));
    let mut kept = created[1]["cipher"].clone();
    kept["id"] = made.item_id.clone().into();
    kept["collectionIds"] = json!([ACCOUNTING]);
    kept["edit"] = true.into();
    kept["viewPassword"] = false.into();
    let hidden = Sample::new(&url, &[], move |sync| {
        sync["profile"]["organizations"][0]["type"] = 2.into();
        sync["collections"]
            .as_array_mut()
            .unwrap()
            .push(accounting(folder_grid(true, false)));
        sync["ciphers"].as_array_mut().unwrap().push(kept);
    });
    let mut app = hidden.unlocked();
    let vault = app.vault().unwrap();
    let item = vault.item(&made.item_id).unwrap();
    assert!(!vault.grid(item).see_passwords);
    // A later passkey for the account can't go in it.
    assert!(vault
        .passkey_target(item, &site("new.example.test", &handle))
        .is_err());
    signs_with(&mut app, &made.item_id, "new.example.test", &made);
}

#[test]
fn where_an_organization_keeps_items_out_of_personal_its_members_own_logins_take_no_passkey() {
    let (url, asked) = stand_in(vec![]);
    let ana = ana();
    let sample = keeping_items_out(&url, true, true);
    let mut app = sample.unlocked();
    // Backup service has this account's passkey and Example Mail is for
    // the site, but both are the member's own: the server would refuse
    // either edit, so neither is offered and both are refused unsent.
    assert!(offered(&app, &site(SITE, &ana)).is_empty());
    for login in [BACKUP, MAIL] {
        assert_eq!(
            app.passkey_create(&new_passkey(SITE, &ana, Some(login), &[]))
                .err(),
            Some(Error::Refused(
                "your organization keeps items out of Personal, so that login can't be changed"
            ))
        );
    }
    assert_eq!(saves(&asked), 0);
    assert_eq!(credential(&app, BACKUP), OLD_CREDENTIAL);
    // Its owners aren't bound: Backup service is the one to replace, as ever.
    let owners = keeping_items_out(&url, false, true);
    assert_eq!(
        offered(&owners.unlocked(), &site(SITE, &ana)),
        ids(&[(BACKUP, true), (MAIL, false)])
    );
}

#[test]
fn with_no_shared_folder_to_add_to_no_new_login_is_made() {
    let (url, asked) = stand_in(vec![]);
    let handle = [4u8; 16];
    // IT doesn't take new items from this member: nowhere a new login
    // could go.
    let sample = keeping_items_out(&url, true, false);
    let mut app = sample.unlocked();
    assert_eq!(
        app.passkey_create(&new_passkey("new.example.test", &handle, None, &[]))
            .err(),
        Some(Error::Refused(
            "your organization keeps items out of Personal, and you have no shared folder you can add to. Ask an admin for one, then try again"
        ))
    );
    assert_eq!(saves(&asked), 0);
    assert_eq!(app.passkeys_for("new.example.test").len(), 0);

    // An owner or admin isn't bound by it: theirs goes in Personal, as
    // anyone's does where no organization keeps items out of it.
    let sample = keeping_items_out(&url, false, false);
    let mut app = sample.unlocked();
    let made = app
        .passkey_create(&new_passkey("new.example.test", &handle, None, &[]))
        .unwrap();
    let created = sent(&asked, "POST /api/ciphers");
    assert_eq!(created.len(), 1);
    assert!(created[0]["organizationId"].is_null());
    let item = app.vault().unwrap().item(&made.item_id).unwrap();
    assert_eq!(item.organization_id, None);
    assert!(item.collection_ids.is_empty());
}

/// The sample with Contoso keeping items out of Personal, the person's role
/// in it `role` (upstream's numbers: 0 owner, 1 admin, 2 user), its shared
/// folder IT taking new items from them or not (`create`; they may still
/// edit its items and see their passwords), and the sync changed further
/// by `edit`.
fn bound_as(url: &str, role: u64, create: bool, edit: impl FnOnce(&mut Value)) -> Sample {
    Sample::new(url, &[], move |sync| {
        sync["profile"]["organizations"][0]["type"] = role.into();
        sync["policies"] = json!([{ "organizationId": ORG, "type": 5, "enabled": true }]);
        sync["collections"][0]["keykeeper"]["create"] = create.into();
        edit(sync);
    })
}

/// What the phones say in place of a new login where Contoso keeps the
/// person's items out of Personal and they have no shared folder to add
/// to: the words New says it in (places.dart in the app, `nowhereWords`).
const NOWHERE: &str = "Contoso keeps items out of Personal, and you have no shared folder you can add to. Ask an admin for one, then try again.";

#[test]
fn the_phones_hear_whether_a_new_login_can_take_a_passkey_before_they_offer_one() {
    let (url, asked) = stand_in(vec![]);
    let handle = [4u8; 16];

    // A member Contoso binds, with IT to add to: a new login goes there,
    // so it's offered.
    let with_folder = bound_as(&url, 2, true, |_| {});
    let app = with_folder.unlocked();
    assert_eq!(
        app.new_passkey_login(),
        Ok(Some((ORG.to_owned(), IT.to_owned())))
    );
    assert_eq!(app.no_new_passkey_login(), None);

    // Without one, a new login isn't offered: the phones say why, naming
    // Contoso. Asked for anyway, nothing is made or sent.
    let without = bound_as(&url, 2, false, |_| {});
    let mut app = without.unlocked();
    assert_eq!(app.no_new_passkey_login().as_deref(), Some(NOWHERE));
    // Nor is their own Example Mail, for example.test, offered.
    assert!(offered(&app, &site("new.example.test", &handle)).is_empty());
    assert_eq!(
        app.new_passkey_login(),
        Err(Error::Refused(
            "your organization keeps items out of Personal, and you have no shared folder you can add to. Ask an admin for one, then try again"
        ))
    );
    assert!(app
        .passkey_create(&new_passkey("new.example.test", &handle, None, &[]))
        .is_err());
    assert_eq!(saves(&asked), 0);

    // Its owners and admins aren't bound, folder or not, nor is anyone
    // where no organization keeps items out of Personal: a new login, in
    // Personal.
    for role in [0, 1] {
        let sample = bound_as(&url, role, false, |_| {});
        let app = sample.unlocked();
        assert_eq!(app.new_passkey_login(), Ok(None), "role {role}");
        assert_eq!(app.no_new_passkey_login(), None, "role {role}");
    }
    let free = Sample::new(&url, &[], |_| {});
    let app = free.unlocked();
    assert_eq!(app.no_new_passkey_login(), None);
    assert_eq!(
        offered(&app, &site("new.example.test", &handle)),
        ids(&[(MAIL, false)])
    );

    // A policy that removes logins makes none either, and says so.
    let removed = Sample::new(&url, &[], |sync| {
        sync["policies"] = json!([{
            "organizationId": ORG, "type": 15, "enabled": true, "data": [1],
        }]);
    });
    assert_eq!(
        removed.unlocked().no_new_passkey_login().as_deref(),
        Some("An organization you're in doesn't allow that kind of item.")
    );
}

#[test]
fn with_no_new_login_to_offer_a_shared_login_the_person_may_change_still_takes_the_passkey() {
    let (url, asked) = stand_in(vec![]);
    let handle = [3u8; 16];
    let office = websites(&url, OFFICE_WIFI, &["https://office.example.test"]);
    // A member Contoso binds, who may change IT's items but add none:
    // Contoso's Office Wi-Fi, for the site, is offered, and no new login.
    let sample = bound_as(&url, 2, false, move |sync| {
        for c in sync["ciphers"].as_array_mut().unwrap() {
            if c["id"] == OFFICE_WIFI {
                c["login"]["uris"] = office.clone();
            }
        }
    });
    let mut app = sample.unlocked();
    let new = site("office.example.test", &handle);
    assert_eq!(offered(&app, &new), ids(&[(OFFICE_WIFI, false)]));
    assert_eq!(app.no_new_passkey_login().as_deref(), Some(NOWHERE));
    // Picked, it goes in beside the login's password, in IT.
    let made = app
        .passkey_create(&new_passkey(
            "office.example.test",
            &handle,
            Some(OFFICE_WIFI),
            &[],
        ))
        .unwrap();
    assert_eq!(made.item_id, OFFICE_WIFI);
    assert_eq!(saves(&asked), 1);
    assert_eq!(
        sent(&asked, &format!("PUT /api/ciphers/{OFFICE_WIFI}")).len(),
        1
    );
    let item = app.vault().unwrap().item(OFFICE_WIFI).unwrap();
    assert_eq!(item.organization_id.as_deref(), Some(ORG));
    assert_eq!(item.passkey.as_ref().unwrap().rp_id, "office.example.test");
    signs_with(&mut app, OFFICE_WIFI, "office.example.test", &made);
}

/// Backup service's passkey as stored, counting its sign-ins from 5, as
/// one imported from an authenticator that counts them: encrypted with its
/// login's key, to keep on the device in place of the sample's (which
/// stays at 0).
fn counting_from_5(url: &str) -> Value {
    let sample = Sample::new(url, &[], |_| {});
    let app = sample.unlocked();
    let item = app.vault().unwrap().item(BACKUP).unwrap();
    let passkey = item.passkey.as_ref().unwrap();
    assert_eq!(passkey.counter, 0);
    passkey.with_counter(item.kept.fido2_credentials.as_ref().unwrap(), 5)
}

/// The counter of an item's passkey, as the vault has it now.
fn counter(app: &App, id: &str) -> u32 {
    app.vault()
        .unwrap()
        .item(id)
        .unwrap()
        .passkey
        .as_ref()
        .unwrap()
        .counter
}

const COUNTING_KEPT_OUT: &str = "your organization keeps items out of Personal, so this login can't be changed. Its passkey counts its sign-ins, and signing in would change the login. You can move it to a shared folder in the web vault, then try again";

#[test]
fn a_passkey_that_counts_in_a_login_kept_out_of_personal_is_refused_before_it_signs() {
    let (url, asked) = stand_in(vec![]);
    let hash = [7u8; 32];
    let counting = counting_from_5(&url);
    let counts = |sync: &mut Value| {
        for c in sync["ciphers"].as_array_mut().unwrap() {
            if c["id"] == BACKUP {
                c["login"]["fido2Credentials"] = counting.clone();
            }
        }
    };

    // A member Contoso binds: Backup service is their own, and its passkey
    // counts, so signing in would save the login with the raised counter,
    // which the server refuses. It's still listed for the site, and signing
    // in with it is refused in words that say what to do, with nothing
    // signed or sent and the counter where it was.
    let member = bound_as(&url, 2, true, counts);
    let mut app = member.unlocked();
    let vault = app.vault().unwrap();
    assert_eq!(
        vault.keeps_out(vault.item(BACKUP).unwrap()),
        Some("Contoso")
    );
    assert_eq!(counter(&app, BACKUP), 5);
    assert_eq!(app.passkeys_for(SITE).len(), 1);
    assert_eq!(
        app.passkey_assert(BACKUP, SITE, &hash, true).err(),
        Some(Error::Refused(COUNTING_KEPT_OUT))
    );
    assert_eq!(saves(&asked), 0);
    assert_eq!(counter(&app, BACKUP), 5);

    // One that doesn't count (the sample's own, at 0) has nothing to save:
    // it signs in from the kept-out login as ever, its counter still 0.
    let plain = bound_as(&url, 2, true, |_| {});
    let mut app = plain.unlocked();
    let signed = app.passkey_assert(BACKUP, SITE, &hash, true).unwrap();
    assert_eq!(&signed.authenticator_data[33..], &[0, 0, 0, 0]);
    assert_eq!(saves(&asked), 0);

    // Its owners and admins aren't bound: the raised counter is saved in
    // the login first, then it signs with it.
    for role in [0, 1] {
        let sample = bound_as(&url, role, true, counts);
        let mut app = sample.unlocked();
        let before = saves(&asked);
        let signed = app.passkey_assert(BACKUP, SITE, &hash, true).unwrap();
        assert_eq!(
            &signed.authenticator_data[33..],
            &[0, 0, 0, 6],
            "role {role}"
        );
        assert_eq!(saves(&asked), before + 1, "role {role}");
        assert_eq!(counter(&app, BACKUP), 6, "role {role}");
    }
}

#[test]
fn a_site_that_has_the_passkey_already_is_told_so_and_nothing_is_replaced() {
    let (url, asked) = stand_in(vec![]);
    let sample = Sample::new(&url, &[], |_| {});
    let mut app = sample.unlocked();
    let ana = ana();
    let old = credential_id(OLD_CREDENTIAL).unwrap();
    let vault = app.vault().unwrap();
    assert!(vault.has_passkey_among(SITE, std::slice::from_ref(&old)));
    // Only for its own site, by one of its IDs.
    assert!(!vault.has_passkey_among("mail.example.test", std::slice::from_ref(&old)));
    assert!(!vault.has_passkey_among(SITE, &[BEA.to_vec()]));
    assert!(!vault.has_passkey_among(SITE, &[]));
    // Even into the login that has it: the site is told it has it already
    // (WebAuthn's InvalidStateError), and nothing changes.
    assert_eq!(
        app.passkey_create(&new_passkey(
            SITE,
            &ana,
            Some(BACKUP),
            std::slice::from_ref(&old)
        ))
        .err(),
        Some(Error::Refused(ALREADY_HAS_PASSKEY))
    );
    assert_eq!(saves(&asked), 0);
    assert_eq!(credential(&app, BACKUP), OLD_CREDENTIAL);

    // In the trash, it doesn't count.
    let trashed = Sample::new(&url, &[], |sync| {
        for c in sync["ciphers"].as_array_mut().unwrap() {
            if c["id"] == BACKUP {
                c["deletedDate"] = "2026-11-02T10:00:00.000Z".into();
            }
        }
    });
    let app = trashed.unlocked();
    assert!(!app
        .vault()
        .unwrap()
        .has_passkey_among(SITE, std::slice::from_ref(&old)));
}

#[test]
fn an_android_apps_own_logins_are_offered_for_the_site_that_names_it() {
    let (url, _) = stand_in(vec![]);
    let mail_app = websites(&url, MAIL, &["androidapp://com.example.mail"]);
    let sample = with_websites(&url, MAIL, mail_app, |_| {});
    let app = sample.unlocked();
    let ana = ana();
    // Example Mail is now the app's own login, not example.test's.
    assert_eq!(offered(&app, &site(SITE, &ana)), ids(&[(BACKUP, true)]));
    // Asked by that app (the site's asset links name it): its login too.
    let asking = PasskeySite {
        app: Some("com.example.mail"),
        ..site(SITE, &ana)
    };
    assert_eq!(
        offered(&app, &asking),
        ids(&[(BACKUP, true), (MAIL, false)])
    );
    // Not by another app, nor one whose name only starts the same.
    for other in ["com.example.other", "com.example"] {
        let asking = PasskeySite {
            app: Some(other),
            ..site(SITE, &ana)
        };
        assert_eq!(offered(&app, &asking), ids(&[(BACKUP, true)]));
    }
}
