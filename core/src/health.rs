//! Password health, as the web vault's Health reports and the extension's
//! line under a login's password have it (passbundle-clients,
//! password-health-line.component.ts and the reports): how strong a
//! password is by zxcvbn's score (4 strong, 3 good, anything less weak),
//! passwords used for more than one login, and websites saved with plain
//! http://. All on this device; nothing is sent anywhere.

use std::collections::HashMap;

use zeroize::Zeroizing;

use crate::vault::{ItemKind, Vault};

/// zxcvbn's score, 0 to 4, with the login's own words (its username and
/// name) counting as easy to guess.
pub fn strength(password: &str, hints: &[&str]) -> u8 {
    if password.is_empty() {
        return 0;
    }
    let hints: Vec<&str> = hints.iter().copied().filter(|h| !h.is_empty()).collect();
    u8::from(zxcvbn::zxcvbn(password, &hints).score())
}

/// "strong", "good", or "weak", as the extension says it.
pub fn word(score: u8) -> &'static str {
    match score {
        4 => "strong",
        3 => "good",
        _ => "weak",
    }
}

/// A username's parts, as the web vault's weak passwords report counts them
/// against a password: split at "@" and anything not a letter or digit.
fn username_parts(username: &str) -> Vec<String> {
    username
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|p| p.len() >= 3)
        .map(str::to_owned)
        .collect()
}

/// What's wrong with one login.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issues {
    pub id: String,
    /// Its password is weak (a score under 3).
    pub weak: bool,
    /// How many other logins have the same password.
    pub reused: usize,
    /// One of its websites is plain http://.
    pub unsecured: bool,
}

/// A login's password and what counts against it, read while the vault is
/// open, to judge away from it (`report`).
pub struct Login {
    pub id: String,
    pub password: Zeroizing<String>,
    pub hints: Vec<String>,
    pub unsecured: bool,
}

/// The logins to judge: those with a password the person may see, in use
/// (not in the trash or archived).
pub fn logins(vault: &Vault) -> Vec<Login> {
    vault
        .items
        .iter()
        .filter(|i| i.kind == ItemKind::Login && i.in_use())
        .filter(|i| vault.grid(i).see_passwords)
        .filter_map(|i| {
            let password = i.password.as_ref().filter(|p| !p.is_empty())?;
            Some(Login {
                id: i.id.clone(),
                password: Zeroizing::new(password.as_str().to_owned()),
                hints: i
                    .username
                    .as_deref()
                    .map(username_parts)
                    .unwrap_or_default(),
                unsecured: i
                    .uris
                    .iter()
                    .any(|u| u.trim().to_ascii_lowercase().starts_with("http://")),
            })
        })
        .collect()
}

/// Every login with something to fix, in the order given.
pub fn report(logins: &[Login]) -> Vec<Issues> {
    let mut uses: HashMap<&str, usize> = HashMap::new();
    for l in logins {
        *uses.entry(l.password.as_str()).or_default() += 1;
    }
    logins
        .iter()
        .map(|l| {
            let hints: Vec<&str> = l.hints.iter().map(String::as_str).collect();
            Issues {
                id: l.id.clone(),
                weak: strength(&l.password, &hints) < 3,
                reused: uses[l.password.as_str()] - 1,
                unsecured: l.unsecured,
            }
        })
        .filter(|i| i.weak || i.reused > 0 || i.unsecured)
        .collect()
}

/// How healthy a person's own passwords are, in numbers only: what their
/// organizations' owners and admins see, never which logins (DESIGN.md in
/// the meta repo, "Members' own passwords"). The apps don't check breaches,
/// so they never say how many are exposed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct OwnCounts {
    /// The logins in Personal with a password, not in the trash.
    pub logins: u32,
    /// Of those, with a weak password.
    pub weak: u32,
    /// Of those, sharing a password with another of them.
    pub reused: u32,
}

/// The person's own logins' numbers: those in Personal (no organization's),
/// not in the trash, with a password, judged as Health judges them. `weak`
/// remembers each login's verdict by its id and revision, so only what
/// changed is judged again.
pub fn own_counts(vault: &Vault, weak: &mut HashMap<String, bool>) -> OwnCounts {
    let own: Vec<_> = vault
        .items
        .iter()
        .filter(|i| i.kind == ItemKind::Login && !i.deleted && i.organization_id.is_none())
        .filter_map(|i| Some((i, i.password.as_ref().filter(|p| !p.is_empty())?)))
        .collect();
    let mut uses: HashMap<&str, u32> = HashMap::new();
    for (_, password) in &own {
        *uses.entry(password.as_str()).or_default() += 1;
    }
    let mut counts = OwnCounts {
        logins: own.len() as u32,
        ..OwnCounts::default()
    };
    let mut judged = HashMap::new();
    for (item, password) in &own {
        let key = format!(
            "{}:{}",
            item.id,
            item.revision_date.as_deref().unwrap_or("")
        );
        let is_weak = *weak.entry(key.clone()).or_insert_with(|| {
            let hints = item
                .username
                .as_deref()
                .map(username_parts)
                .unwrap_or_default();
            let hints: Vec<&str> = hints.iter().map(String::as_str).collect();
            strength(password, &hints) < 3
        });
        judged.insert(key, is_weak);
        counts.weak += u32::from(is_weak);
        counts.reused += u32::from(uses[password.as_str()] > 1);
    }
    // What's gone or changed is forgotten.
    *weak = judged;
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn login(id: &str, password: &str, username: &str, uri: &str) -> Login {
        Login {
            id: id.into(),
            password: Zeroizing::new(password.into()),
            hints: username_parts(username),
            unsecured: uri.starts_with("http://"),
        }
    }

    #[test]
    fn weak_reused_and_unsecured_logins_are_found() {
        let found = report(&[
            login("a", "password1", "ana@example.test", "https://a.example"),
            login("b", "kq9#Vz!m2P@xL7&r-Wt4", "ana", "https://b.example"),
            login("c", "kq9#Vz!m2P@xL7&r-Wt4", "ana", "https://c.example"),
            login(
                "d",
                "an-0dd-long-phrase-Tulip-Seven",
                "ana",
                "http://d.example",
            ),
            login("e", "Yz7#Lw!q2Rb@Vn5&kd", "ana", "https://e.example"),
        ]);
        let by: HashMap<&str, &Issues> = found.iter().map(|i| (i.id.as_str(), i)).collect();
        assert!(by["a"].weak);
        assert_eq!((by["b"].reused, by["c"].reused), (1, 1));
        assert!(!by["b"].weak);
        assert!(by["d"].unsecured && !by["d"].weak);
        assert!(!by.contains_key("e"));
    }

    #[test]
    fn a_password_made_of_the_username_is_weak() {
        assert!(strength("anamaria1990", &["anamaria"]) < 3);
        assert_eq!(word(strength("kq9#Vz!m2P@xL7&r-Wt4", &[])), "strong");
        assert_eq!(word(0), "weak");
    }
}
