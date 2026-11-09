//! Firefox, from Mozilla's own documentation and open-source code (its
//! profile service, its login manager, and NSS). The details are in
//! DESIGN.md, "Automatic import".
//!
//! - **Profiles:** `profiles.ini` lists each profile's folder and name. The
//!   one an `[Install…]` section names is the one that Firefox starts with
//!   (else the profile marked `Default=1`). A profile signed in to a Mozilla
//!   account has `signedInUser.json`, with the account's email.
//! - **Saved passwords:** `logins.json`, each login's username and password
//!   encrypted with the key in `key4.db` (nss.rs), which a Primary Password,
//!   when the person set one, protects. PassBundle asks for it each time, uses
//!   it once to open the key, and never keeps it.
//! - **Its own password saving:** `prefs.js`'s `signon.rememberSignons` (the
//!   "Ask to save passwords" setting) and `signon.autofillForms`.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use zeroize::Zeroizing;

use crate::nss::{DbError, KeyDb};
use crate::{Browser, Home, Login, Profile, Read, TurnOff, Why};

/// The profiles `profiles.ini` in `root` lists that are there.
pub(crate) fn profiles(root: &Path, home: &Home) -> Vec<Profile> {
    let Ok(text) = std::fs::read_to_string(root.join("profiles.ini")) else {
        return Vec::new();
    };
    let sections = ini(&text);
    let path_of = |section: &[(String, String)]| -> Option<PathBuf> {
        let path = value(section, "Path")?;
        let relative = value(section, "IsRelative").is_none_or(|r| r != "0");
        Some(if relative {
            path.split(['/', '\\'])
                .filter(|p| !p.is_empty())
                .fold(root.to_owned(), |dir, part| dir.join(part))
        } else {
            PathBuf::from(path)
        })
    };
    // The profiles each installed Firefox starts with.
    let installs: Vec<PathBuf> = sections
        .iter()
        .filter(|(name, _)| name.starts_with("Install"))
        .filter_map(|(_, s)| {
            value(s, "Default").map(|p| {
                p.split(['/', '\\'])
                    .filter(|p| !p.is_empty())
                    .fold(root.to_owned(), |dir, part| dir.join(part))
            })
        })
        .collect();
    let mut found = Vec::new();
    for (name, section) in &sections {
        if !name.starts_with("Profile") {
            continue;
        }
        let Some(dir) = path_of(section) else {
            continue;
        };
        // Only profiles in the person's own folders.
        if !dir.starts_with(&home.home) || !dir.is_dir() {
            continue;
        }
        let default = if installs.is_empty() {
            value(section, "Default").is_some_and(|d| d == "1")
        } else {
            installs.iter().any(|d| same_dir(d, &dir))
        };
        let label = value(section, "Name")
            .filter(|n| !n.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                dir.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
        found.push(Profile {
            id: format!("{}:{}", Browser::Firefox.key(), home.id_path(&dir)),
            browser: Browser::Firefox,
            name: label,
            email: signed_in_email(&dir),
            managed_by: None,
            default,
            platform: home.platform,
            browser_dir: dir.clone(),
            dir,
        });
    }
    found.sort_by(|a, b| b.default.cmp(&a.default).then(a.name.cmp(&b.name)));
    found
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

type Section = Vec<(String, String)>;

/// An INI file's sections, in order: each name and its keys and values.
fn ini(text: &str) -> Vec<(String, Section)> {
    let mut sections: Vec<(String, Section)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sections.push((name.trim().to_owned(), Vec::new()));
        } else if let (Some((key, value)), Some((_, section))) =
            (line.split_once('='), sections.last_mut())
        {
            section.push((key.trim().to_owned(), value.trim().to_owned()));
        }
    }
    sections
}

fn value<'a>(section: &'a [(String, String)], key: &str) -> Option<&'a str> {
    section
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.as_str())
}

/// The Mozilla account the profile is signed in to.
fn signed_in_email(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("signedInUser.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json.pointer("/accountData/email")
        .and_then(|e| e.as_str())
        .map(|e| e.trim().to_lowercase())
        .filter(|e| e.contains('@'))
}

#[derive(Deserialize)]
struct LoginsFile {
    #[serde(default)]
    logins: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    #[serde(default)]
    hostname: String,
    #[serde(default)]
    encrypted_username: String,
    #[serde(default)]
    encrypted_password: String,
    #[serde(default)]
    time_created: Option<i64>,
    #[serde(default)]
    time_password_changed: Option<i64>,
}

/// Reads a profile's saved logins.
pub(crate) fn read(profile: &Profile, primary_password: Option<&str>) -> Read {
    let text = match std::fs::read_to_string(profile.dir.join("logins.json")) {
        Ok(text) => Zeroizing::new(text),
        // Nothing saved yet.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Read::Logins {
                logins: Vec::new(),
                never_save: 0,
                empty: 0,
                unreadable: 0,
            }
        }
        Err(_) => return Read::Guided(Why::Damaged),
    };
    let Ok(file) = serde_json::from_str::<LoginsFile>(&text) else {
        return Read::Guided(Why::Damaged);
    };
    let db = match KeyDb::open(&profile.dir.join("key4.db")) {
        Ok(db) => db,
        // Firefox before version 58 kept its key in key3.db, which this
        // doesn't read.
        Err(_) if profile.dir.join("key3.db").is_file() => return Read::Guided(Why::Unknown),
        Err(DbError::Unknown) => return Read::Guided(Why::Unknown),
        Err(DbError::Unreadable) => {
            return Read::Guided(if crate::is_open(profile) {
                Why::Busy
            } else {
                Why::Damaged
            })
        }
    };
    let keys = if db.needs_password() {
        let Some(password) = primary_password.filter(|p| !p.is_empty()) else {
            return Read::NeedsPrimaryPassword;
        };
        match db.unlock(password.as_bytes()) {
            Some(keys) => keys,
            None => return Read::WrongPrimaryPassword,
        }
    } else {
        match db.unlock(b"") {
            Some(keys) => keys,
            None => return Read::Guided(Why::Damaged),
        }
    };
    let (mut logins, mut empty, mut unreadable) = (Vec::new(), 0, 0);
    for entry in file.logins {
        let lower = entry.hostname.to_ascii_lowercase();
        // Web sites only: not Firefox's own pages or proxies.
        if !(lower.starts_with("https://") || lower.starts_with("http://")) {
            continue;
        }
        let (Some(username), Some(password)) = (
            keys.decrypt(&entry.encrypted_username),
            keys.decrypt(&entry.encrypted_password),
        ) else {
            unreadable += 1;
            continue;
        };
        if password.is_empty() {
            empty += 1;
            continue;
        }
        logins.push(Login {
            url: entry.hostname,
            username: username.as_str().to_owned(),
            password,
            note: None,
            created_ms: entry.time_created.filter(|t| *t > 0),
            changed_ms: entry.time_password_changed.filter(|t| *t > 0),
        });
    }
    Read::Logins {
        logins,
        never_save: 0,
        empty,
        unreadable,
    }
}

const SAVING: &str = "signon.rememberSignons";
const FILLING: &str = "signon.autofillForms";

/// A preference's value as a `prefs.js` or `user.js` line sets it.
fn pref_value(text: &str, name: &str) -> Option<String> {
    let prefix = format!("\"{name}\"");
    text.lines().rev().find_map(|line| {
        let line = line.trim();
        let rest = line
            .strip_prefix("user_pref(")
            .or_else(|| line.strip_prefix("pref("))?
            .trim_start();
        let rest = rest.strip_prefix(&prefix)?.trim_start().strip_prefix(',')?;
        let value = rest.trim().strip_suffix(';')?.trim().strip_suffix(')')?;
        Some(value.trim().to_owned())
    })
}

pub(crate) fn saving_on(profile: &Profile) -> Option<bool> {
    let read = |name: &str| std::fs::read_to_string(profile.dir.join(name)).ok();
    let user = read("user.js").and_then(|t| pref_value(&t, SAVING));
    let prefs = read("prefs.js").and_then(|t| pref_value(&t, SAVING));
    Some(user.or(prefs).is_none_or(|v| v != "false"))
}

/// Turns off "Ask to save passwords" and filling saved logins on page load,
/// with Firefox closed, as its own settings do. What it saved stays.
pub(crate) fn turn_off(profile: &Profile) -> TurnOff {
    if crate::running::firefox(&profile.dir, profile.platform) {
        return TurnOff::CloseFirst;
    }
    // The person's own user.js sets it at every start: theirs to change.
    if let Ok(user) = std::fs::read_to_string(profile.dir.join("user.js")) {
        match pref_value(&user, SAVING).as_deref() {
            Some("false") => return TurnOff::AlreadyOff,
            Some(_) => return TurnOff::Failed,
            None => {}
        }
    }
    let path = profile.dir.join("prefs.js");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => return TurnOff::Failed,
    };
    if pref_value(&text, SAVING).as_deref() == Some("false")
        && pref_value(&text, FILLING).as_deref() == Some("false")
    {
        return TurnOff::AlreadyOff;
    }
    let mut lines: Vec<&str> = text
        .lines()
        .filter(|line| {
            let line = line.trim();
            !(line.starts_with("user_pref(")
                && (line.contains(&format!("\"{SAVING}\""))
                    || line.contains(&format!("\"{FILLING}\""))))
        })
        .collect();
    let saving = format!("user_pref(\"{SAVING}\", false);");
    let filling = format!("user_pref(\"{FILLING}\", false);");
    lines.push(&saving);
    lines.push(&filling);
    let mut out = lines.join("\n");
    out.push('\n');
    match crate::running::replace_file(&path, out.as_bytes()) {
        Ok(()) => TurnOff::Done,
        Err(_) => TurnOff::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_preferences_as_firefox_writes_them() {
        let prefs = "// Mozilla User Preferences\n\
                     user_pref(\"browser.startup.page\", 3);\n\
                     user_pref(\"signon.rememberSignons\", false);\n";
        assert_eq!(pref_value(prefs, SAVING).as_deref(), Some("false"));
        assert_eq!(pref_value(prefs, FILLING), None);
        assert_eq!(
            pref_value("user_pref(\"signon.rememberSignons\",true);", SAVING).as_deref(),
            Some("true")
        );
    }

    #[test]
    fn reads_profiles_ini_sections_in_order() {
        let sections =
            ini("[Install1]\nDefault=Profiles/a\n\n[Profile0]\nName=work\nPath=Profiles/a\n");
        assert_eq!(sections.len(), 2);
        assert_eq!(value(&sections[1].1, "name"), Some("work"));
    }
}
