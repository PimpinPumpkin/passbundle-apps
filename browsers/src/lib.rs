//! The browsers on this computer, for importing a person's saved passwords
//! into PassBundle (meta repo DESIGN.md, "Browser import", "Automatic
//! import"): their profiles, with who each is signed in as; the passwords
//! saved in the profiles the person chooses; and turning a profile's own
//! password saving off afterwards ("disable it, don't purge it").
//!
//! It runs on the person's own computer, as them, and reads only their own
//! profiles, only the ones they chose, each time they say so. It reads what
//! each browser's own files let that same person read, and nothing more: no
//! other program's memory, no browser's elevation or decryption service, no
//! administrator rights, no other person's folder, and no OS protection
//! turned off. Where a browser keeps its passwords so that only it can read
//! them (Chrome's app-bound encryption on Windows), this says so
//! (`Why::OnlyTheBrowser`), and the person imports with the browser's own
//! Export instead. The mechanics, per browser, are in DESIGN.md.
//!
//! Finding profiles reads no passwords: only each profile's name, the account
//! it's signed in to, and whether it's the browser's default.

mod choose;
mod chromium;
mod der;
mod firefox;
mod home;
mod keys;
mod nss;
mod running;
mod sqlite;

#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;
#[cfg(test)]
mod tests;

use std::path::PathBuf;

use zeroize::Zeroizing;

pub use choose::{choose, Choice, Rule};
pub use home::Home;
pub use keys::{KeyError, Keys, SystemKeys};

/// A browser PassBundle knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Browser {
    Chrome,
    Edge,
    Brave,
    Vivaldi,
    Opera,
    Chromium,
    Firefox,
    /// On a Mac: always the guided import (the Passwords app's Export).
    Safari,
}

impl Browser {
    /// The Chromium browsers, in the order they're listed.
    pub const CHROMIUM: [Browser; 6] = [
        Browser::Chrome,
        Browser::Edge,
        Browser::Brave,
        Browser::Vivaldi,
        Browser::Opera,
        Browser::Chromium,
    ];

    /// Its name, as the guided import names it.
    pub fn name(self) -> &'static str {
        match self {
            Browser::Chrome => "Google Chrome",
            Browser::Edge => "Microsoft Edge",
            Browser::Brave => "Brave",
            Browser::Vivaldi => "Vivaldi",
            Browser::Opera => "Opera",
            Browser::Chromium => "Chromium",
            Browser::Firefox => "Firefox",
            Browser::Safari => "Safari",
        }
    }

    /// Its short name, for sentences ("Close Chrome first").
    pub fn short(self) -> &'static str {
        match self {
            Browser::Chrome => "Chrome",
            Browser::Edge => "Edge",
            other => other.name(),
        }
    }

    /// Its name in a profile's id.
    pub fn key(self) -> &'static str {
        match self {
            Browser::Chrome => "chrome",
            Browser::Edge => "edge",
            Browser::Brave => "brave",
            Browser::Vivaldi => "vivaldi",
            Browser::Opera => "opera",
            Browser::Chromium => "chromium",
            Browser::Firefox => "firefox",
            Browser::Safari => "safari",
        }
    }
}

/// Which OS a profile's files are laid out and protected for. Always this
/// computer's, except in tests, which make profiles for each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Mac,
    Linux,
}

impl Platform {
    pub fn this() -> Platform {
        if cfg!(windows) {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::Mac
        } else {
            Platform::Linux
        }
    }
}

/// One browser profile on this computer, as found: nothing secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    /// Stable while the profile stays where it is: the browser and the
    /// profile's folder. Importing finds profiles again and takes only the
    /// ones whose ids it's given, so an id can't point anywhere else.
    pub id: String,
    pub browser: Browser,
    /// The profile's name in the browser ("Work", "Person 1", "default-release").
    pub name: String,
    /// The account the profile is signed in to (a Google, Microsoft, or
    /// Mozilla account), lowercase; None when signed in to none.
    pub email: Option<String>,
    /// The domain that manages the signed-in account, where the browser says
    /// (Chrome's Google Workspace accounts).
    pub managed_by: Option<String>,
    /// The browser's own default profile: Chromium's "Default" folder, or the
    /// one Firefox starts with.
    pub default: bool,
    pub(crate) platform: Platform,
    /// The profile's own folder.
    pub(crate) dir: PathBuf,
    /// Chromium: the browser's folder that holds its profiles and Local
    /// State. Firefox: the same as `dir`.
    pub(crate) browser_dir: PathBuf,
}

/// A saved password, as read from a profile. Lives only in memory, and is
/// wiped when dropped.
pub struct Login {
    /// The site: its address, or `androidapp://<package>` for an Android app
    /// a Chromium browser saved one for.
    pub url: String,
    pub username: String,
    pub password: Zeroizing<String>,
    /// A note saved with it (Chrome's password notes), if any.
    pub note: Option<Zeroizing<String>>,
    /// When it was saved, and when its password last changed, in
    /// milliseconds since 1970, where the browser kept them.
    pub created_ms: Option<i64>,
    pub changed_ms: Option<i64>,
}

impl Login {
    /// The newer of when it was saved and when its password changed.
    pub fn when_ms(&self) -> i64 {
        self.changed_ms.max(self.created_ms).unwrap_or(0)
    }
}

/// What reading one profile came to.
pub enum Read {
    /// Its saved passwords. `never_save` counts the sites the person told
    /// the browser never to save for, which aren't passwords; `empty` the
    /// saved sign-ins with no password; `unreadable` the ones that didn't
    /// open (damaged, or in a form this doesn't know), left out.
    Logins {
        logins: Vec<Login>,
        never_save: usize,
        empty: usize,
        unreadable: usize,
    },
    /// Firefox: the profile has a Primary Password, and none was given.
    NeedsPrimaryPassword,
    /// Firefox: the Primary Password given isn't this profile's.
    WrongPrimaryPassword,
    /// It can't be read from outside the browser: import it with the
    /// browser's own Export instead (the guided import), and why.
    Guided(Why),
}

/// Why a profile goes to the guided import instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// The browser keeps its passwords so that only it can read them
    /// (Chrome's app-bound encryption on Windows). PassBundle doesn't try
    /// to get around that.
    OnlyTheBrowser,
    /// The person said no when the computer asked (macOS's keychain).
    Declined,
    /// The browser's key isn't in the person's keychain or keyring, or the
    /// keyring can't be reached (KWallet, or no keyring running).
    NoKey,
    /// The keyring that holds the browser's key is locked.
    KeyringLocked,
    /// The browser had the file open in a way that kept it from being read:
    /// close the browser and try again.
    Busy,
    /// A format this version doesn't read (a very old or very new browser).
    Unknown,
    /// The profile's files are damaged.
    Damaged,
}

/// What turning off a profile's own password saving came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnOff {
    /// Turned off now. Its saved passwords stay where they were.
    Done,
    /// It was already off.
    AlreadyOff,
    /// The browser is open, and would put it back when it closes: close it
    /// and try again.
    CloseFirst,
    /// It couldn't be changed (a file that couldn't be written).
    Failed,
}

/// Every browser profile on this computer, for this person: the Chromium
/// browsers', then Firefox's. Reads no passwords.
pub fn find_profiles(home: &Home) -> Vec<Profile> {
    let mut found = Vec::new();
    for browser in Browser::CHROMIUM {
        for dir in home.chromium_dirs(browser) {
            found.extend(chromium::profiles(browser, &dir, home));
        }
    }
    for root in home.firefox_dirs() {
        found.extend(firefox::profiles(&root, home));
    }
    let mut seen = std::collections::HashSet::new();
    found.retain(|p| seen.insert(p.id.clone()));
    found
}

/// The browsers here that only the guided import brings passwords in from:
/// Safari, on a Mac. Its passwords are the Passwords app's, which only
/// Apple's apps read; its own Export makes the file.
pub fn guided_only(home: &Home) -> Vec<Browser> {
    match home.platform {
        Platform::Mac => vec![Browser::Safari],
        _ => Vec::new(),
    }
}

/// Reads one profile's saved passwords: the person chose it, just now. A
/// Firefox profile with a Primary Password needs it (`primary_password`),
/// used once and never kept.
pub fn read_logins(profile: &Profile, keys: &dyn Keys, primary_password: Option<&str>) -> Read {
    #[cfg(any(test, feature = "fixtures"))]
    fixtures::note_read(&profile.dir);
    match profile.browser {
        Browser::Firefox => firefox::read(profile, primary_password),
        Browser::Safari => Read::Guided(Why::OnlyTheBrowser),
        _ => chromium::read(profile, keys),
    }
}

/// Turns off the profile's own offer to save passwords (and Chromium's
/// automatic sign-in, Firefox's filling on page load), leaving what it saved
/// where it is. Only while the browser is closed: it writes its settings back
/// when it closes, over a change made while it's open.
pub fn turn_off_saving(profile: &Profile) -> TurnOff {
    match profile.browser {
        Browser::Firefox => firefox::turn_off(profile),
        Browser::Safari => TurnOff::Failed,
        _ => chromium::turn_off(profile),
    }
}

/// Whether the profile's own password saving is on, as its settings say
/// (None where they can't be read).
pub fn saving_on(profile: &Profile) -> Option<bool> {
    match profile.browser {
        Browser::Firefox => firefox::saving_on(profile),
        Browser::Safari => None,
        _ => chromium::saving_on(profile),
    }
}

/// Whether the browser has this profile open now.
pub fn is_open(profile: &Profile) -> bool {
    match profile.browser {
        Browser::Firefox => running::firefox(&profile.dir, profile.platform),
        Browser::Safari => false,
        _ => running::chromium(&profile.browser_dir, profile.platform),
    }
}
