//! Where a new passkey goes on a phone (docs/core.md, "Passkeys", "Where it
//! goes"), on the vectors' sample vault kept on this device, against a
//! stand-in for the server (tests/support/stand_in.rs): the logins offered
//! for it (in use, that the person may change, for the site and without a
//! passkey, or with this account's passkey for the site, which it
//! replaces), and making one into a login, in place of its old passkey or
//! beside its password, never into one the phones wouldn't offer. The
//! sample's Backup service login has a passkey for backup.example.test; its
//! Example Mail login is on mail.example.test, the same site by the default
//! match (example.test).

use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use p256::pkcs8::DecodePublicKey;
use passbundle_core::app::{App, NewPasskey, PasskeyMade, ALREADY_HAS_PASSKEY};
use passbundle_core::error::Error;
use passbundle_core::matching::PasskeySite;
use passbundle_core::passkey::{base64_either, credential_id};
use passbundle_core::vault::ItemDraft;
use serde_json::{json, Value};

#[path = "support/stand_in.rs"]
mod stand_in;
use stand_in::{sent, stand_in, Asked, Sample, BACKUP, GATE, MAIL, OFFICE_WIFI, ORG};

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
