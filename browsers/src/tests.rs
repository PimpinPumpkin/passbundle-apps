//! The browsers' own layouts and encryption, on made-up profiles with known
//! fake passwords in throwaway folders (fixtures.rs). Every platform's layout
//! is tested on every computer; Windows' real data protection only on
//! Windows. No test looks at a real profile.

use std::collections::BTreeMap;
use std::path::Path;

use crate::fixtures::{
    ChromiumProfile, Fake, FirefoxCipher, FirefoxProfile, Fixture, Sealed, TestKeys,
};
use crate::{
    find_profiles, read_logins, saving_on, turn_off_saving, Browser, KeyError, Platform, Profile,
    Read, TurnOff, Why,
};

const PLATFORMS: [Platform; 3] = [Platform::Windows, Platform::Mac, Platform::Linux];

fn work_logins() -> Vec<Fake> {
    vec![
        Fake::new(
            "https://mail.acme.test/login",
            "ana@acme.test",
            "correct-horse-1",
        )
        .with_note("The shared inbox too"),
        Fake::new("https://crm.acme.test/", "ana", "battery-staple-2"),
        Fake::never_save("https://never.example.test/"),
        Fake::new("https://empty.example.test/", "nobody", ""),
        Fake::new("androidapp://com.acme.timesheets", "ana", "app-password-3"),
    ]
}

/// Each site's username, password, and note.
type Sites = BTreeMap<String, (String, String, Option<String>)>;

/// What a read found: the sites, and the never-save, empty, and unreadable counts.
fn found(read: Read) -> (Sites, usize, usize, usize) {
    match read {
        Read::Logins {
            logins,
            never_save,
            empty,
            unreadable,
        } => (
            logins
                .into_iter()
                .map(|l| {
                    (
                        l.url,
                        (
                            l.username,
                            l.password.as_str().to_owned(),
                            l.note.map(|n| n.as_str().to_owned()),
                        ),
                    )
                })
                .collect(),
            never_save,
            empty,
            unreadable,
        ),
        Read::NeedsPrimaryPassword => panic!("asked for a Primary Password"),
        Read::WrongPrimaryPassword => panic!("a wrong Primary Password"),
        Read::Guided(why) => panic!("went to the guided import: {why:?}"),
    }
}

fn guided(read: Read) -> Why {
    match read {
        Read::Guided(why) => why,
        _ => panic!("expected the guided import"),
    }
}

fn by_id<'a>(profiles: &'a [Profile], id_end: &str) -> &'a Profile {
    profiles
        .iter()
        .find(|p| p.id.ends_with(id_end))
        .unwrap_or_else(|| {
            panic!(
                "no profile {id_end} in {:?}",
                profiles.iter().map(|p| &p.id).collect::<Vec<_>>()
            )
        })
}

/// Every file's bytes under a folder.
fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            let kind = std::fs::symlink_metadata(&path).unwrap().file_type();
            let bytes = if kind.is_symlink() {
                std::fs::read_link(&path)
                    .unwrap()
                    .to_string_lossy()
                    .as_bytes()
                    .to_vec()
            } else if kind.is_dir() {
                stack.push(path);
                continue;
            } else {
                std::fs::read(&path).unwrap()
            };
            files.insert(path.to_string_lossy().into_owned(), bytes);
        }
    }
    files
}

#[test]
fn finds_each_profile_with_who_its_signed_in_as_without_reading_a_password() {
    for platform in PLATFORMS {
        let fixture = Fixture::new(platform);
        fixture.chromium(ChromiumProfile {
            logins: work_logins(),
            ..ChromiumProfile::new(Browser::Chrome, "Default", "Person 1")
        });
        fixture.chromium(ChromiumProfile {
            email: Some("Ana@Acme.test"),
            hosted_domain: Some("acme.test"),
            logins: work_logins(),
            ..ChromiumProfile::new(Browser::Chrome, "Profile 1", "Work")
        });
        fixture.chromium(ChromiumProfile::new(Browser::Edge, "Default", "Profile 1"));
        fixture.firefox(FirefoxProfile {
            default: true,
            email: Some("ana.home@mail.test"),
            ..FirefoxProfile::new("abc123.default-release", "default-release")
        });
        fixture.firefox(FirefoxProfile::new("xyz789.default", "default"));

        let profiles = find_profiles(&fixture.home);
        let summary: Vec<(Browser, &str, Option<&str>, bool)> = profiles
            .iter()
            .map(|p| (p.browser, p.name.as_str(), p.email.as_deref(), p.default))
            .collect();
        assert_eq!(
            summary,
            [
                (Browser::Chrome, "Person 1", None, true),
                (Browser::Chrome, "Work", Some("ana@acme.test"), false),
                (Browser::Edge, "Profile 1", None, true),
                (
                    Browser::Firefox,
                    "default-release",
                    Some("ana.home@mail.test"),
                    true
                ),
                (Browser::Firefox, "default", None, false),
            ],
            "{platform:?}"
        );
        assert_eq!(
            by_id(&profiles, "Profile 1").managed_by.as_deref(),
            Some("acme.test")
        );
        // Ids say which browser and which folder, and are the same each time.
        assert!(profiles[1].id.starts_with("chrome:"), "{}", profiles[1].id);
        assert_eq!(find_profiles(&fixture.home), profiles);
    }
}

#[test]
fn a_home_with_no_browsers_has_no_profiles() {
    let fixture = Fixture::new(Platform::this());
    assert!(find_profiles(&fixture.home).is_empty());
}

#[test]
fn reads_chromium_passwords_as_each_platform_keeps_them() {
    for (platform, sealed) in [
        (Platform::Mac, Sealed::Usual),
        (Platform::Linux, Sealed::Usual),
        (Platform::Linux, Sealed::Keyring),
        (Platform::Windows, Sealed::Usual),
    ] {
        for browser in [Browser::Chrome, Browser::Edge, Browser::Brave] {
            let fixture = Fixture::new(platform);
            fixture.chromium(ChromiumProfile {
                logins: work_logins(),
                account_logins: vec![Fake::new(
                    "https://docs.acme.test/",
                    "ana@acme.test",
                    "kept-for-account-4",
                )],
                sealed,
                ..ChromiumProfile::new(browser, "Default", "Person 1")
            });
            let profiles = find_profiles(&fixture.home);
            let keys = TestKeys::default();
            let before = snapshot(&fixture.root);
            let (logins, never_save, empty, unreadable) =
                found(read_logins(&profiles[0], &keys, None));
            assert_eq!(
                logins.keys().collect::<Vec<_>>(),
                [
                    "androidapp://com.acme.timesheets",
                    "https://crm.acme.test/",
                    "https://docs.acme.test/",
                    "https://mail.acme.test/login",
                ],
                "{platform:?} {sealed:?} {browser:?}"
            );
            assert_eq!(
                logins["https://mail.acme.test/login"],
                (
                    "ana@acme.test".to_owned(),
                    "correct-horse-1".to_owned(),
                    Some("The shared inbox too".to_owned())
                )
            );
            assert_eq!(logins["https://docs.acme.test/"].1, "kept-for-account-4");
            assert_eq!((never_save, empty, unreadable), (1, 1, 0));
            // The key once, however many passwords; none on Linux without a keyring.
            let expected = if platform == Platform::Linux && sealed == Sealed::Usual {
                0
            } else {
                1
            };
            assert_eq!(keys.asked.get(), expected, "{platform:?} {sealed:?}");
            // Reading changes nothing in the profile.
            assert_eq!(snapshot(&fixture.root), before);
        }
    }
}

#[cfg(windows)]
#[test]
fn reads_chromium_passwords_with_windows_own_data_protection() {
    let fixture = Fixture::new(Platform::Windows);
    fixture.chromium(ChromiumProfile {
        logins: work_logins(),
        sealed: Sealed::RealWindows,
        ..ChromiumProfile::new(Browser::Edge, "Default", "Profile 1")
    });
    let profiles = find_profiles(&fixture.home);
    let (logins, ..) = found(read_logins(&profiles[0], &crate::SystemKeys, None));
    assert_eq!(logins["https://crm.acme.test/"].1, "battery-staple-2");
}

#[test]
fn chromes_app_bound_encryption_goes_to_the_guided_import_without_opening_anything() {
    let fixture = Fixture::new(Platform::Windows);
    fixture.chromium(ChromiumProfile {
        logins: work_logins(),
        sealed: Sealed::AppBound,
        ..ChromiumProfile::new(Browser::Chrome, "Default", "Person 1")
    });
    let profiles = find_profiles(&fixture.home);
    let keys = TestKeys::default();
    assert_eq!(
        guided(read_logins(&profiles[0], &keys, None)),
        Why::OnlyTheBrowser
    );
    // Not even the key that opens the rest was asked for.
    assert_eq!(keys.asked.get(), 0);
}

#[test]
fn a_keychain_that_says_no_or_a_locked_keyring_goes_to_the_guided_import() {
    for (platform, sealed, refusal, why) in [
        (
            Platform::Mac,
            Sealed::Usual,
            KeyError::Denied,
            Why::Declined,
        ),
        (Platform::Mac, Sealed::Usual, KeyError::NotFound, Why::NoKey),
        (
            Platform::Linux,
            Sealed::Keyring,
            KeyError::Locked,
            Why::KeyringLocked,
        ),
        (
            Platform::Linux,
            Sealed::Keyring,
            KeyError::Unavailable,
            Why::NoKey,
        ),
        (
            Platform::Windows,
            Sealed::Usual,
            KeyError::NotFound,
            Why::NoKey,
        ),
    ] {
        let fixture = Fixture::new(platform);
        fixture.chromium(ChromiumProfile {
            logins: work_logins(),
            sealed,
            ..ChromiumProfile::new(Browser::Chrome, "Default", "Person 1")
        });
        let profiles = find_profiles(&fixture.home);
        let keys = TestKeys::refusing(refusal);
        assert_eq!(
            guided(read_logins(&profiles[0], &keys, None)),
            why,
            "{platform:?}"
        );
        // Asked once, not once per password.
        assert_eq!(keys.asked.get(), 1);
    }
}

#[test]
fn reads_firefox_logins_with_each_cipher_and_database_scheme() {
    for cipher in [
        FirefoxCipher::TripleDes,
        FirefoxCipher::Aes,
        FirefoxCipher::OldDatabase,
    ] {
        for platform in PLATFORMS {
            let fixture = Fixture::new(platform);
            fixture.firefox(FirefoxProfile {
                default: true,
                logins: vec![
                    Fake::new("https://mail.acme.test", "ana@acme.test", "firefox-pass-1"),
                    Fake::new("https://wiki.acme.test", "", "only-a-password-2"),
                    Fake::new("https://empty.acme.test", "x", ""),
                    Fake::new("chrome://FirefoxAccounts", "ana@acme.test", "not-a-site"),
                ],
                cipher,
                ..FirefoxProfile::new("abc.default-release", "default-release")
            });
            let profiles = find_profiles(&fixture.home);
            let before = snapshot(&fixture.root);
            let (logins, never_save, empty, unreadable) =
                found(read_logins(&profiles[0], &TestKeys::default(), None));
            assert_eq!(
                logins,
                BTreeMap::from([
                    (
                        "https://mail.acme.test".to_owned(),
                        (
                            "ana@acme.test".to_owned(),
                            "firefox-pass-1".to_owned(),
                            None
                        )
                    ),
                    (
                        "https://wiki.acme.test".to_owned(),
                        (String::new(), "only-a-password-2".to_owned(), None)
                    ),
                ]),
                "{cipher:?} {platform:?}"
            );
            assert_eq!((never_save, empty, unreadable), (0, 1, 0));
            assert_eq!(snapshot(&fixture.root), before);
        }
    }
}

#[test]
fn firefox_asks_for_its_primary_password_and_refuses_a_wrong_one() {
    for cipher in [FirefoxCipher::TripleDes, FirefoxCipher::OldDatabase] {
        let fixture = Fixture::new(Platform::this());
        // A Primary Password this test made up.
        let primary = "test-primary-password-for-this-test";
        fixture.firefox(FirefoxProfile {
            default: true,
            logins: vec![Fake::new(
                "https://bank.example.test",
                "ana",
                "behind-the-primary-1",
            )],
            primary_password: Some(primary),
            cipher,
            ..FirefoxProfile::new("abc.default-release", "default-release")
        });
        let profiles = find_profiles(&fixture.home);
        let before = snapshot(&fixture.root);
        let keys = TestKeys::default();
        assert!(matches!(
            read_logins(&profiles[0], &keys, None),
            Read::NeedsPrimaryPassword
        ));
        assert!(matches!(
            read_logins(&profiles[0], &keys, Some("")),
            Read::NeedsPrimaryPassword
        ));
        assert!(matches!(
            read_logins(&profiles[0], &keys, Some("not-the-primary-password")),
            Read::WrongPrimaryPassword
        ));
        let (logins, ..) = found(read_logins(&profiles[0], &keys, Some(primary)));
        assert_eq!(
            logins["https://bank.example.test"].1,
            "behind-the-primary-1"
        );
        // Nothing was written anywhere: the Primary Password isn't kept.
        let after = snapshot(&fixture.root);
        assert_eq!(after, before);
        assert!(!after.values().any(|bytes| bytes
            .windows(primary.len())
            .any(|w| w == primary.as_bytes())));
    }
}

#[test]
fn a_primary_password_given_for_a_profile_without_one_is_ignored() {
    let fixture = Fixture::new(Platform::this());
    fixture.firefox(FirefoxProfile {
        default: true,
        logins: vec![Fake::new("https://a.example.test", "ana", "no-primary-1")],
        ..FirefoxProfile::new("abc.default-release", "default-release")
    });
    let profiles = find_profiles(&fixture.home);
    let (logins, ..) = found(read_logins(
        &profiles[0],
        &TestKeys::default(),
        Some("anything"),
    ));
    assert_eq!(logins.len(), 1);
}

#[test]
fn a_profile_with_nothing_saved_reads_as_none() {
    let fixture = Fixture::new(Platform::this());
    fixture.chromium(ChromiumProfile::new(Browser::Chrome, "Default", "Person 1"));
    let dir = fixture.firefox(FirefoxProfile::new(
        "abc.default-release",
        "default-release",
    ));
    std::fs::remove_file(dir.join("logins.json")).unwrap();
    for profile in find_profiles(&fixture.home) {
        let (logins, ..) = found(read_logins(&profile, &TestKeys::default(), None));
        assert!(logins.is_empty());
    }
}

#[test]
fn damaged_files_go_to_the_guided_import() {
    let fixture = Fixture::new(Platform::Linux);
    let chrome = fixture.chromium(ChromiumProfile {
        logins: work_logins(),
        ..ChromiumProfile::new(Browser::Chrome, "Default", "Person 1")
    });
    std::fs::write(chrome.join("Login Data"), b"not a database at all").unwrap();
    let firefox = fixture.firefox(FirefoxProfile {
        logins: work_logins(),
        ..FirefoxProfile::new("abc.default-release", "default-release")
    });
    std::fs::write(firefox.join("logins.json"), b"{ not json").unwrap();
    for profile in find_profiles(&fixture.home) {
        assert_eq!(
            guided(read_logins(&profile, &TestKeys::default(), None)),
            Why::Damaged
        );
    }
}

#[test]
fn turns_off_chromiums_password_saving_and_keeps_every_other_setting() {
    let fixture = Fixture::new(Platform::this());
    let dir = fixture.chromium(ChromiumProfile {
        logins: work_logins(),
        ..ChromiumProfile::new(Browser::Chrome, "Default", "Person 1")
    });
    let profiles = find_profiles(&fixture.home);
    let saved = std::fs::read(dir.join("Login Data")).unwrap();
    assert_eq!(saving_on(&profiles[0]), Some(true));
    assert_eq!(turn_off_saving(&profiles[0]), TurnOff::Done);
    let prefs: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("Preferences")).unwrap()).unwrap();
    assert_eq!(prefs["credentials_enable_service"], false);
    assert_eq!(prefs["credentials_enable_autosignin"], false);
    assert_eq!(prefs["profile"]["name"], "x");
    assert_eq!(saving_on(&profiles[0]), Some(false));
    assert_eq!(turn_off_saving(&profiles[0]), TurnOff::AlreadyOff);
    // Disabled, not purged: the saved passwords are as they were.
    assert_eq!(std::fs::read(dir.join("Login Data")).unwrap(), saved);
}

#[test]
fn turns_off_firefoxs_password_saving_and_keeps_every_other_setting() {
    let fixture = Fixture::new(Platform::this());
    let dir = fixture.firefox(FirefoxProfile {
        default: true,
        logins: vec![Fake::new("https://a.example.test", "ana", "kept-1")],
        ..FirefoxProfile::new("abc.default-release", "default-release")
    });
    let profiles = find_profiles(&fixture.home);
    let saved = std::fs::read(dir.join("logins.json")).unwrap();
    assert_eq!(saving_on(&profiles[0]), Some(true));
    assert_eq!(turn_off_saving(&profiles[0]), TurnOff::Done);
    let prefs = std::fs::read_to_string(dir.join("prefs.js")).unwrap();
    assert!(prefs.contains("user_pref(\"browser.startup.page\", 3);"));
    assert!(prefs.contains("user_pref(\"signon.rememberSignons\", false);"));
    assert!(prefs.contains("user_pref(\"signon.autofillForms\", false);"));
    assert_eq!(saving_on(&profiles[0]), Some(false));
    assert_eq!(turn_off_saving(&profiles[0]), TurnOff::AlreadyOff);
    assert_eq!(std::fs::read(dir.join("logins.json")).unwrap(), saved);
    // A user.js that turns saving on is the person's own: left alone.
    std::fs::write(
        dir.join("user.js"),
        "user_pref(\"signon.rememberSignons\", true);\n",
    )
    .unwrap();
    assert_eq!(saving_on(&profiles[0]), Some(true));
    assert_eq!(turn_off_saving(&profiles[0]), TurnOff::Failed);
}

#[cfg(unix)]
#[test]
fn an_open_browser_is_asked_to_close_first_and_nothing_is_changed() {
    let fixture = Fixture::new(Platform::Linux);
    let chrome = fixture.chromium(ChromiumProfile::new(Browser::Chrome, "Default", "Person 1"));
    let firefox = fixture.firefox(FirefoxProfile::new(
        "abc.default-release",
        "default-release",
    ));
    // Running: links to this test's own process, as the browsers make them.
    let me = std::process::id();
    std::os::unix::fs::symlink(
        format!("testhost-{me}"),
        chrome.parent().unwrap().join("SingletonLock"),
    )
    .unwrap();
    std::os::unix::fs::symlink(format!("127.0.0.1:+{me}"), firefox.join("lock")).unwrap();
    let before = snapshot(&fixture.root);
    for profile in find_profiles(&fixture.home) {
        assert!(crate::is_open(&profile), "{}", profile.id);
        assert_eq!(
            turn_off_saving(&profile),
            TurnOff::CloseFirst,
            "{}",
            profile.id
        );
    }
    assert_eq!(snapshot(&fixture.root), before);
    // Closed (a link left by a process that's gone): changed.
    for (link, target) in [
        (
            chrome.parent().unwrap().join("SingletonLock"),
            "testhost-999999999",
        ),
        (firefox.join("lock"), "127.0.0.1:+999999999"),
    ] {
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(target, &link).unwrap();
    }
    for profile in find_profiles(&fixture.home) {
        assert!(!crate::is_open(&profile), "{}", profile.id);
        assert_eq!(turn_off_saving(&profile), TurnOff::Done, "{}", profile.id);
    }
}

#[test]
fn safari_is_the_guided_import_on_a_mac() {
    assert_eq!(
        crate::guided_only(&Fixture::new(Platform::Mac).home),
        [Browser::Safari]
    );
    assert!(crate::guided_only(&Fixture::new(Platform::Windows).home).is_empty());
}

#[test]
fn a_list_that_points_outside_the_persons_own_folders_is_never_followed() {
    let fixture = Fixture::new(Platform::Linux);
    fixture.chromium(ChromiumProfile {
        logins: work_logins(),
        ..ChromiumProfile::new(Browser::Chrome, "Default", "Person 1")
    });
    // Someone else's profile, outside this home folder.
    let elsewhere = Fixture::new(Platform::Linux);
    let theirs = elsewhere.firefox(FirefoxProfile {
        default: true,
        logins: vec![Fake::new(
            "https://theirs.example.test",
            "bo",
            "not-yours-1",
        )],
        ..FirefoxProfile::new("theirs.default-release", "default-release")
    });
    // Local State naming a folder above the browser's, and profiles.ini
    // pointing at the other folder, plainly and through "..".
    let chrome = fixture.chromium_dir(Browser::Chrome);
    let state_path = chrome.join("Local State");
    let mut state: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&state_path).unwrap()).unwrap();
    state["profile"]["info_cache"][".."] = serde_json::json!({ "name": "Up" });
    std::fs::write(&state_path, state.to_string()).unwrap();
    std::fs::write(chrome.parent().unwrap().join("Preferences"), "{}").unwrap();
    let firefox_root = fixture.home.home.join(".mozilla").join("firefox");
    std::fs::create_dir_all(&firefox_root).unwrap();
    let up = std::iter::repeat_n("..", 8).collect::<Vec<_>>().join("/");
    std::fs::write(
        firefox_root.join("profiles.ini"),
        format!(
            "[Profile0]\nName=absolute\nIsRelative=0\nPath={}\n\n[Profile1]\nName=relative\nIsRelative=1\nPath={up}{}\n",
            theirs.display(),
            theirs.display()
        ),
    )
    .unwrap();
    let names: Vec<String> = find_profiles(&fixture.home)
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, ["Person 1"]);
}
