//! Real Chrome and Firefox: what they saved, read back. CI seeds their
//! profiles in a throwaway home folder with made-up passwords
//! (tools/real-browsers/seed.py and fakes.json) and sets PB_REAL_BROWSERS to
//! that folder and PB_REAL_BROWSERS_FAKES to the fakes; without them this
//! says so and passes. It never looks at anyone's own profiles. Then it turns
//! their password saving off, which seed.py checks the browsers kept.

use std::collections::BTreeSet;
use std::path::Path;

use passbundle_browsers::{
    find_profiles, read_logins, turn_off_saving, Browser, Home, Platform, Read, SystemKeys, TurnOff,
};
use serde_json::Value;

/// Each login as (site, username, password, note).
type Seen = BTreeSet<(String, String, String, String)>;

fn host(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split('/').next().unwrap_or("").to_lowercase()
}

fn expected(logins: &Value) -> Seen {
    logins
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                host(l["url"].as_str().unwrap()),
                l["username"].as_str().unwrap().to_owned(),
                l["password"].as_str().unwrap().to_owned(),
                l["note"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect()
}

fn seen(read: Read) -> Seen {
    match read {
        Read::Logins {
            logins, unreadable, ..
        } => {
            assert_eq!(unreadable, 0, "some didn't open");
            logins
                .into_iter()
                .map(|l| {
                    (
                        host(&l.url),
                        l.username,
                        l.password.as_str().to_owned(),
                        l.note.map(|n| n.as_str().to_owned()).unwrap_or_default(),
                    )
                })
                .collect()
        }
        Read::NeedsPrimaryPassword => panic!("asked for a Primary Password"),
        Read::WrongPrimaryPassword => panic!("a wrong Primary Password"),
        Read::Guided(why) => panic!("went to the guided import: {why:?}"),
    }
}

#[test]
fn reads_what_real_chrome_and_firefox_saved_and_turns_their_saving_off() {
    let (Some(root), Some(fakes)) = (
        std::env::var_os("PB_REAL_BROWSERS"),
        std::env::var_os("PB_REAL_BROWSERS_FAKES"),
    ) else {
        eprintln!("PB_REAL_BROWSERS isn't set; skipping (CI seeds real browsers for it).");
        return;
    };
    let fakes: Value = serde_json::from_str(&std::fs::read_to_string(fakes).unwrap()).unwrap();
    let home = Home::inside(Path::new(&root), Platform::Linux);
    let profiles = find_profiles(&home);
    let names: Vec<(Browser, &str, bool)> = profiles
        .iter()
        .map(|p| (p.browser, p.name.as_str(), p.default))
        .collect();
    eprintln!("found {names:?}");
    let chrome = profiles
        .iter()
        .find(|p| p.browser == Browser::Chrome && p.default)
        .expect("Chrome's default profile");
    let plain = profiles
        .iter()
        .find(|p| p.browser == Browser::Firefox && p.name == "default-release")
        .expect("Firefox's default profile");
    assert!(plain.default);
    let primary = profiles
        .iter()
        .find(|p| p.browser == Browser::Firefox && p.name == "work")
        .expect("Firefox's profile with a Primary Password");

    // Chrome without a keyring: its basic store. Nothing to ask the OS.
    assert_eq!(
        seen(read_logins(chrome, &SystemKeys, None)),
        expected(&fakes["chrome"])
    );
    assert_eq!(
        seen(read_logins(plain, &SystemKeys, None)),
        expected(&fakes["firefox"])
    );
    let password = fakes["firefoxPrimary"]["password"].as_str().unwrap();
    assert!(matches!(
        read_logins(primary, &SystemKeys, None),
        Read::NeedsPrimaryPassword
    ));
    assert!(matches!(
        read_logins(primary, &SystemKeys, Some("not-the-primary-password")),
        Read::WrongPrimaryPassword
    ));
    assert_eq!(
        seen(read_logins(primary, &SystemKeys, Some(password))),
        expected(&fakes["firefoxPrimary"]["logins"])
    );

    for profile in [chrome, plain, primary] {
        assert_eq!(turn_off_saving(profile), TurnOff::Done, "{}", profile.id);
    }
}
