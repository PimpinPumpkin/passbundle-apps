//! Importing the passwords saved in the browsers on this computer, on a
//! computer only (meta repo DESIGN.md, "Browser import", "Automatic import";
//! docs/core.md, "Importing from browsers"). The browsers crate
//! (../browsers) finds the profiles and reads the ones the person chose;
//! this decides which are chosen to begin with, sorts what they hold against
//! the vault, and hands each new login to the vault to encrypt with the
//! person's keys, as any new item is, before anything goes to the server.
//!
//! - **Nothing is read until the person says so,** each time: finding
//!   profiles reads only their names and who they're signed in as; reading
//!   takes the ids the person confirmed, finds the profiles again, and reads
//!   only those. An id that isn't one of them is never read.
//! - **A Firefox Primary Password** comes in for one read, as the master
//!   password does, and is wiped after it; it's never kept or sent.
//! - **Sorting**, as the guided import does (passbundle-clients,
//!   browser-import.ts): a login the vault already has (same site,
//!   username, and password), or that two profiles both have, is left out;
//!   one without a password is skipped; and one whose password differs from
//!   a login the person has for the same site and username is imported into
//!   "Check these: from <browser>", to compare. The newest of the logins that
//!   differ is the one imported plainly. Items whose passwords the person
//!   can't see aren't compared at all.

use std::collections::{HashMap, HashSet};

use passbundle_browsers::{self as browsers, Browser, Choice, Home, Keys, Login, Profile, Read};
use serde_json::Value;
use zeroize::Zeroizing;

use crate::vault::{ItemDraft, ItemKind, Vault};

pub use passbundle_browsers::{Rule, TurnOff, Why};

/// What the kept sync says about the person's organizations.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Organizations {
    /// Their email domains, where an admin set some
    /// (`keykeeperEmailDomains` on each organization, passbundle-server).
    pub domains: Vec<String>,
    /// The name of an organization that keeps items out of Personal
    /// (upstream's personal ownership policy), which binds this person.
    pub keeps_items_out: Option<String>,
}

/// Upstream's "Remove individual vault" (organization data ownership) policy.
const PERSONAL_OWNERSHIP: u64 = 5;

impl Organizations {
    pub fn from_sync(text: Option<&str>) -> Organizations {
        let Some(sync) = text.and_then(|t| serde_json::from_str::<Value>(t).ok()) else {
            return Organizations::default();
        };
        // Confirmed memberships, and (`organizationsNew`, passbundle-server)
        // ones whose org key hasn't reached the person yet: someone who just
        // joined with the install command is one of those.
        let organizations: Vec<Value> = ["/profile/organizations", "/profile/organizationsNew"]
            .iter()
            .filter_map(|p| sync.pointer(p).and_then(Value::as_array))
            .flatten()
            .cloned()
            .collect();
        let mut domains: Vec<String> = organizations
            .iter()
            .filter_map(|o| o["keykeeperEmailDomains"].as_array())
            .flatten()
            .filter_map(Value::as_str)
            .map(|d| d.trim().trim_start_matches('@').to_lowercase())
            .filter(|d| d.contains('.'))
            .collect();
        domains.sort();
        domains.dedup();
        // Owners and admins aren't bound by it, as upstream has it.
        let keeps_items_out = sync["policies"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| p["type"].as_u64() == Some(PERSONAL_OWNERSHIP) && p["enabled"] == true)
            .find_map(|p| {
                let org = organizations
                    .iter()
                    .find(|o| o["id"] == p["organizationId"])?;
                let exempt = matches!(org["type"].as_u64(), Some(0 | 1));
                (!exempt).then(|| {
                    org["name"]
                        .as_str()
                        .unwrap_or("Your organization")
                        .to_owned()
                })
            });
        Organizations {
            domains,
            keeps_items_out,
        }
    }
}

/// A profile as the import screen lists it.
pub struct Listed {
    pub profile: Profile,
    pub choice: Choice,
    /// The browser has it open now (its password saving is turned off only
    /// once it's closed).
    pub open: bool,
    /// Its own password saving, as its settings say.
    pub saving_on: Option<bool>,
}

/// What's on this computer to import from.
pub struct Found {
    pub profiles: Vec<Listed>,
    /// Browsers here whose passwords only their own Export brings in (Safari).
    pub guided: Vec<Browser>,
    /// No rule chose a profile: the person is asked which, if any.
    pub ask: bool,
    pub organizations: Organizations,
}

/// The browsers' profiles, chosen by the rules for this person. Reads no
/// passwords.
pub fn find(home: &Home, email: &str, sync: Option<&str>) -> Found {
    let organizations = Organizations::from_sync(sync);
    let profiles = browsers::find_profiles(home);
    let choices = browsers::choose(&profiles, email, &organizations.domains);
    let ask = !choices.iter().any(|c| c.chosen);
    Found {
        profiles: profiles
            .into_iter()
            .zip(choices)
            .map(|(profile, choice)| Listed {
                open: browsers::is_open(&profile),
                saving_on: browsers::saving_on(&profile),
                profile,
                choice,
            })
            .collect(),
        guided: browsers::guided_only(home),
        ask,
        organizations,
    }
}

/// How reading one chosen profile went.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Read: how many passwords it had.
    Read {
        logins: usize,
    },
    NeedsPrimaryPassword,
    WrongPrimaryPassword,
    /// Its browser's own Export brings it in instead, and why.
    Guided(Why),
    /// It isn't here any more.
    Gone,
}

/// What reading the chosen profiles found.
pub struct Gathered {
    /// Each profile read, or that couldn't be, in the order chosen.
    pub profiles: Vec<(String, Option<Profile>, Outcome)>,
    /// Their passwords, with the browser each came from.
    pub logins: Vec<(Browser, Login)>,
    pub never_save: usize,
    pub empty: usize,
    pub unreadable: usize,
}

/// Reads the profiles the person chose (`chosen`, by id), and no others:
/// the profiles are found again, and only those with these ids are read.
/// `primary_passwords` has the Firefox Primary Passwords the person typed,
/// by profile id.
pub fn gather(
    home: &Home,
    keys: &dyn Keys,
    chosen: &[String],
    primary_passwords: &HashMap<String, Zeroizing<String>>,
) -> Gathered {
    let profiles = browsers::find_profiles(home);
    let mut gathered = Gathered {
        profiles: Vec::new(),
        logins: Vec::new(),
        never_save: 0,
        empty: 0,
        unreadable: 0,
    };
    let mut seen = HashSet::new();
    for id in chosen {
        if !seen.insert(id.as_str()) {
            continue;
        }
        let Some(profile) = profiles.iter().find(|p| &p.id == id) else {
            gathered.profiles.push((id.clone(), None, Outcome::Gone));
            continue;
        };
        #[cfg(feature = "test-hooks")]
        test_hooks::READ
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(id.clone());
        let primary = primary_passwords.get(id).map(|p| p.as_str());
        let outcome = match browsers::read_logins(profile, keys, primary) {
            Read::Logins {
                logins,
                never_save,
                empty,
                unreadable,
            } => {
                gathered.never_save += never_save;
                gathered.empty += empty;
                gathered.unreadable += unreadable;
                let count = logins.len();
                gathered
                    .logins
                    .extend(logins.into_iter().map(|l| (profile.browser, l)));
                Outcome::Read { logins: count }
            }
            Read::NeedsPrimaryPassword => Outcome::NeedsPrimaryPassword,
            Read::WrongPrimaryPassword => Outcome::WrongPrimaryPassword,
            Read::Guided(why) => Outcome::Guided(why),
        };
        gathered
            .profiles
            .push((id.clone(), Some(profile.clone()), outcome));
    }
    gathered
}

/// A new login to import.
pub struct New {
    pub browser: Browser,
    pub draft: ItemDraft,
    /// Its password differs from a login the person has for the same site
    /// and username: it goes in its browser's "Check these" folder.
    pub to_check: bool,
}

/// What sorting came to.
pub struct Plan {
    pub new: Vec<New>,
    /// Already in the vault, or twice among the profiles read.
    pub already_there: usize,
}

impl Plan {
    pub fn to_check(&self) -> usize {
        self.new.iter().filter(|n| n.to_check).count()
    }

    /// The browsers whose "Check these" folder is needed.
    pub fn check_browsers(&self) -> Vec<Browser> {
        let mut browsers: Vec<Browser> = self
            .new
            .iter()
            .filter(|n| n.to_check)
            .map(|n| n.browser)
            .collect();
        browsers.sort();
        browsers.dedup();
        browsers
    }
}

/// The folder a browser's logins to compare go in, as the guided import names it.
pub fn check_folder(browser: Browser) -> String {
    format!("Check these: from {}", browser.name())
}

/// The site a login is for, as the guided import compares them: its
/// address's host (an Android app's whole address), lowercase.
fn site_of(url: &str) -> String {
    let lower = url.trim().to_lowercase();
    if lower.starts_with("androidapp://") {
        return lower.trim_end_matches('/').to_owned();
    }
    let rest = lower.split_once("://").map_or(lower.as_str(), |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    host.to_owned()
}

/// An imported login's name: its site, without "www.".
fn name_of(url: &str) -> String {
    let site = site_of(url);
    if let Some(app) = site.strip_prefix("androidapp://") {
        return app.to_owned();
    }
    let host = site.rsplit_once(':').map_or(site.as_str(), |(h, port)| {
        if port.chars().all(|c| c.is_ascii_digit()) {
            h
        } else {
            site.as_str()
        }
    });
    host.strip_prefix("www.").unwrap_or(host).to_owned()
}

fn key_of(site: &str, username: &str) -> String {
    format!("{site}\u{0}{}", username.trim().to_lowercase())
}

/// Sorts what was read against the vault.
pub fn plan(vault: &Vault, mut logins: Vec<(Browser, Login)>) -> Plan {
    let mut known: HashMap<String, Vec<Zeroizing<String>>> = HashMap::new();
    for item in &vault.items {
        // Not items whose passwords the person can't see: "already there"
        // or "a different password" would confirm or rule out a guess.
        if item.kind != ItemKind::Login || item.deleted || !vault.grid(item).see_passwords {
            continue;
        }
        let site = item
            .uris
            .first()
            .map(|u| site_of(u))
            .unwrap_or_else(|| item.name.trim().to_lowercase());
        let password = item
            .password
            .as_ref()
            .map(|p| Zeroizing::new(p.as_str().to_owned()))
            .unwrap_or_default();
        known
            .entry(key_of(&site, item.username.as_deref().unwrap_or("")))
            .or_default()
            .push(password);
    }
    // The newest first, so it's the one imported plainly.
    logins.sort_by_key(|(_, l)| std::cmp::Reverse(l.when_ms()));
    let mut plan = Plan {
        new: Vec::new(),
        already_there: 0,
    };
    for (browser, login) in logins {
        let key = key_of(&site_of(&login.url), &login.username);
        let passwords = known.entry(key).or_default();
        if passwords
            .iter()
            .any(|p| p.as_str() == login.password.as_str())
        {
            plan.already_there += 1;
            continue;
        }
        let to_check = !passwords.is_empty();
        passwords.push(Zeroizing::new(login.password.as_str().to_owned()));
        plan.new.push(New {
            browser,
            to_check,
            draft: ItemDraft {
                name: name_of(&login.url),
                username: Some(login.username).filter(|u| !u.is_empty()),
                password: Some(login.password),
                notes: login.note,
                uris: vec![login.url],
                ..ItemDraft::default()
            },
        });
    }
    plan
}

/// What an import came to, for the summary. Nothing secret.
#[derive(Debug)]
pub struct Imported {
    /// Each chosen profile, and how reading it went.
    pub profiles: Vec<(String, Option<Profile>, Outcome)>,
    /// New logins saved, of which `to_check` went to a "Check these" folder.
    pub added: usize,
    pub to_check: usize,
    pub already_there: usize,
    pub empty: usize,
    pub never_save: usize,
    pub unreadable: usize,
    /// The "Check these" folders used.
    pub check_folders: Vec<String>,
    /// Whether the sync after saving came through (the vault shows them
    /// either way, at the next sync if not now).
    pub synced: bool,
}

/// Why a profile goes to the guided import, in words, with what to do.
pub fn guided_words(browser: Browser, why: Why) -> String {
    let short = browser.short();
    match why {
        Why::OnlyTheBrowser if browser == Browser::Safari => {
            "Safari keeps its passwords in the Passwords app, which only Apple's apps can read. Export them from the Passwords app instead.".to_owned()
        }
        Why::OnlyTheBrowser => format!(
            "{short} protects its passwords on this computer so that only {short} can read them, and PassBundle doesn't get around that. Export them from {short} instead."
        ),
        Why::Declined => format!(
            "Your Mac asked whether PassBundle could use {short}'s key, and the answer was no. Try again and choose Allow, or export them from {short} instead."
        ),
        Why::NoKey => format!(
            "PassBundle couldn't find {short}'s key on this computer. Export them from {short} instead."
        ),
        Why::KeyringLocked => format!(
            "Your keyring is locked. Unlock it and try again, or export them from {short} instead."
        ),
        Why::Busy => format!(
            "{short} was using its files. Close {short} and try again, or export them from {short} instead."
        ),
        Why::Unknown => format!(
            "This version of {short} keeps its passwords in a way PassBundle can't read yet. Export them from {short} instead."
        ),
        Why::Damaged => format!(
            "{short}'s saved passwords couldn't be read. Export them from {short} instead."
        ),
    }
}

/// How to turn off a browser's password saving by hand, as the guided
/// import says it.
pub fn turn_off_by_hand(browser: Browser) -> &'static str {
    match browser {
        Browser::Chrome => {
            "In Chrome's Password Manager settings, turn off Offer to save passwords."
        }
        Browser::Edge => "In Edge's Passwords settings, turn off Offer to save passwords.",
        Browser::Brave => "In Brave's Password Manager settings, turn off Offer to save passwords.",
        Browser::Vivaldi => "In Vivaldi's settings, turn off saving passwords.",
        Browser::Opera => "In Opera's Passwords settings, turn off Offer to save passwords.",
        Browser::Chromium => {
            "In Chromium's Password Manager settings, turn off Offer to save passwords."
        }
        Browser::Firefox => {
            "In Firefox's Settings, under Privacy & Security, turn off Ask to save passwords."
        }
        Browser::Safari => {
            "In Safari's Settings, under AutoFill, turn off User names and passwords."
        }
    }
}

/// At most this many items go to the server in one request.
pub const BATCH: usize = 250;

/// Turns off the chosen profiles' own password saving (and nothing else),
/// finding them again by id, as `gather` does.
pub fn turn_off(home: &Home, chosen: &[String]) -> Vec<(String, Option<Profile>, TurnOff)> {
    let profiles = browsers::find_profiles(home);
    let mut seen = HashSet::new();
    chosen
        .iter()
        .filter(|id| seen.insert(id.as_str()))
        .map(|id| match profiles.iter().find(|p| &p.id == id) {
            Some(profile) => (
                id.clone(),
                Some(profile.clone()),
                browsers::turn_off_saving(profile),
            ),
            None => (id.clone(), None, TurnOff::Failed),
        })
        .collect()
}

/// For tests with test hooks (never a release): the browsers on a made-up
/// home folder, and their made-up keys.
#[cfg(feature = "test-hooks")]
pub mod test_hooks {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use passbundle_browsers::fixtures::{ChromiumProfile, Fake, FirefoxProfile, Fixture, Sealed};
    use passbundle_browsers::{Browser, Home, Platform};

    /// The made-up home folder the app's tests point the core at, and its
    /// layout.
    pub static HOME: Mutex<Option<(PathBuf, Platform)>> = Mutex::new(None);

    /// Every profile read since the tests last asked, by id.
    pub static READ: Mutex<Vec<String>> = Mutex::new(Vec::new());

    pub fn home() -> Option<Home> {
        let given = HOME.lock().unwrap_or_else(|p| p.into_inner());
        given
            .as_ref()
            .map(|(root, platform)| Home::inside(root, *platform))
    }

    /// The Primary Password of the sample's Firefox profile, made up here.
    pub const PRIMARY_PASSWORD: &str = "made-up primary password for tests";

    /// Makes the app tests' sample browsers in `root`, laid out for
    /// `platform`, for the person signed in as `email`:
    /// - Chrome's "Person 1" (its default, signed in to nothing), and "Work",
    ///   signed in with `email`, with two logins and a site never to save;
    /// - Chrome's "Home", signed in to another account, with one login;
    /// - Firefox's default profile, with a Primary Password and one login;
    /// - on Windows, Edge, whose passwords only Edge can open.
    pub fn make_sample(root: &Path, platform: Platform, email: &str) {
        let fixture = Fixture::kept(platform, root);
        fixture.chromium(ChromiumProfile {
            logins: vec![Fake::new(
                "https://intranet.example.test/",
                "ana",
                "person-one-password-1",
            )],
            ..ChromiumProfile::new(Browser::Chrome, "Default", "Person 1")
        });
        fixture.chromium(ChromiumProfile {
            email: Some(email),
            logins: vec![
                Fake::new(
                    "https://mail.example.test/login",
                    email,
                    "work-mail-password-2",
                ),
                Fake::new("https://crm.example.test/", "ana", "work-crm-password-3"),
                Fake::never_save("https://never.example.test/"),
            ],
            ..ChromiumProfile::new(Browser::Chrome, "Profile 1", "Work")
        });
        fixture.chromium(ChromiumProfile {
            email: Some("ana.home@mail.example.test"),
            logins: vec![Fake::new(
                "https://shop.example.test/",
                "ana.home",
                "personal-password-4",
            )],
            ..ChromiumProfile::new(Browser::Chrome, "Profile 2", "Home")
        });
        fixture.firefox(FirefoxProfile {
            default: true,
            logins: vec![Fake::new(
                "https://wiki.example.test",
                "ana",
                "firefox-password-5",
            )],
            primary_password: Some(PRIMARY_PASSWORD),
            ..FirefoxProfile::new("sample.default-release", "default-release")
        });
        if platform == Platform::Windows {
            fixture.chromium(ChromiumProfile {
                logins: vec![Fake::new(
                    "https://edge.example.test/",
                    "ana",
                    "edge-password-6",
                )],
                sealed: Sealed::AppBound,
                ..ChromiumProfile::new(Browser::Edge, "Default", "Profile 1")
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sites_and_names_as_the_guided_import_compares_them() {
        assert_eq!(
            site_of("https://Mail.Example.test:8443/login?x=1"),
            "mail.example.test:8443"
        );
        assert_eq!(
            site_of("https://user@www.example.test/"),
            "www.example.test"
        );
        assert_eq!(
            site_of("androidapp://com.example.app"),
            "androidapp://com.example.app"
        );
        assert_eq!(name_of("https://www.example.test/login"), "example.test");
        assert_eq!(
            name_of("https://mail.example.test:8443/"),
            "mail.example.test"
        );
        assert_eq!(name_of("androidapp://com.example.app"), "com.example.app");
    }

    #[test]
    fn organizations_domains_and_a_policy_that_keeps_items_out_of_personal() {
        let sync = serde_json::json!({
            "profile": {
                "organizations": [
                    { "id": "o1", "name": "Acme Dental", "type": 2, "keykeeperEmailDomains": ["acmedental.test", "@Acme.test"] },
                    { "id": "o2", "name": "Owned", "type": 0 },
                ],
                // Joined, waiting for the org key.
                "organizationsNew": [
                    { "id": "o3", "name": "Branch", "type": 2, "keykeeperEmailDomains": ["branch.test", "acme.test"] },
                ],
            },
            "policies": [
                { "organizationId": "o2", "type": 5, "enabled": true },
            ],
        })
        .to_string();
        let orgs = Organizations::from_sync(Some(&sync));
        assert_eq!(
            orgs.domains,
            ["acme.test", "acmedental.test", "branch.test"]
        );
        // Its owner isn't bound.
        assert_eq!(orgs.keeps_items_out, None);
        let bound = sync.replace("\"o2\",\"type\":5", "\"o1\",\"type\":5");
        assert_eq!(
            Organizations::from_sync(Some(&bound))
                .keeps_items_out
                .as_deref(),
            Some("Acme Dental")
        );
        assert_eq!(Organizations::from_sync(None), Organizations::default());
    }
}
