//! Which logins belong to a website or an app, for filling (docs/phones.md,
//! "Autofill"), by the rules the web vault and the extension use. Each of an
//! item's websites has a match setting (`match` in the vault format):
//!
//! - 0, or none set (the default): the same registrable domain, so
//!   `accounts.example.com` fills on `www.example.com`, but `a.github.io`
//!   doesn't fill on `b.github.io` (the Public Suffix List says where a
//!   registrable domain starts);
//! - 1: the same host;
//! - 2 (starts with), 3 (exact), and 4 (regular expression) need the page's
//!   whole address, and filling elsewhere than a browser has only its domain,
//!   so they aren't offered there;
//! - 5: never.
//!
//! An Android app matches only an item's own `androidapp://<package>` website,
//! never a lookalike name.
//!
//! Cards and identities aren't for a website, so they don't match by one:
//! Android's autofill offers every one in use on a card's or an address's
//! fields, whatever the app or the site (`fills_anywhere`).
//!
//! iOS suggests logins above the keyboard from a list the app gives it of
//! usernames and the domains they're for, never passwords (`suggestions`), by
//! the same rules; and passkeys by their site's ID (`passkey_suggestions`).
//!
//! Android also offers to save a card or an address someone typed into a
//! form, unless it's one they have already: a card by its number, an
//! identity by its first street line and its postal code (`card_with_number`,
//! `identity_at`). Before Android asks, what tells them apart goes to
//! Android's own check of what was typed: each card's last 4 digits, as
//! autofill's list shows them, and each identity's street and postal code
//! (`save_hints`).
//!
//! A new passkey goes in a login the phones offer for it (`passkey_target`,
//! `passkey_logins`): the login with the site's passkey for that account,
//! which it replaces, or one for the site by these rules that has none.

use std::collections::HashSet;
use std::net::IpAddr;

use crate::error::{Error, Result};
use crate::vault::{Item, ItemKind, Vault};

/// What's asking to be filled.
#[derive(Clone, Copy, Debug)]
pub enum Target<'a> {
    /// A page, by its host (what a browser or Android's autofill gives).
    Website(&'a str),
    /// An Android app, by its package name.
    App(&'a str),
}

/// Bitwarden's match settings for an item's website.
const DOMAIN: u8 = 0;
const HOST: u8 = 1;

/// A website's host and port, from however it was written: with or without a
/// scheme, a path, a port, or someone's username.
fn host_and_port(uri: &str) -> Option<(String, Option<u16>)> {
    let uri = uri.trim();
    let rest = match uri.find("://") {
        Some(at) => &uri[at + 3..],
        None => uri,
    };
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?;
    let (host, port) = if authority.parse::<IpAddr>().is_ok() {
        // An IPv6 address without brackets has colons but no port.
        (authority, None)
    } else if let Some(v6) = authority.strip_prefix('[') {
        let (host, after) = v6.split_once(']')?;
        (host, after.strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) => (host, Some(port)),
            _ => (authority, None),
        }
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let port = match port {
        Some(p) if !p.is_empty() => Some(p.parse().ok()?),
        _ => None,
    };
    Some((host, port))
}

/// The registrable domain (`example.co.uk` for `www.example.co.uk`); an IP
/// address, or a name with no dot (`localhost`), is its own.
fn registrable(host: &str) -> String {
    if host.parse::<IpAddr>().is_ok() || !host.contains('.') {
        return host.to_owned();
    }
    psl::domain_str(host).unwrap_or(host).to_owned()
}

/// Whether one of an item's websites, with its match setting, fits the target.
fn fits(uri: &str, match_type: Option<u8>, target: &Target) -> bool {
    match target {
        Target::App(package) => uri.trim().strip_prefix("androidapp://") == Some(*package),
        Target::Website(page) => {
            if uri.trim().starts_with("androidapp://") {
                return false;
            }
            let (Some((host, port)), Some((page, _))) = (host_and_port(uri), host_and_port(page))
            else {
                return false;
            };
            match match_type.unwrap_or(DOMAIN) {
                DOMAIN => registrable(&host) == registrable(&page),
                // A host with a port named fills only where the port is known too.
                HOST => port.is_none() && host == page,
                _ => false,
            }
        }
    }
}

/// Whether the item is a login for the target.
pub fn matches(item: &Item, target: &Target) -> bool {
    item.kind == ItemKind::Login
        && item.in_use()
        && item
            .uris
            .iter()
            .enumerate()
            .any(|(at, uri)| fits(uri, item.uri_matches.get(at).copied().flatten(), target))
}

/// Whether the item is a card or an identity in use, of that kind: autofill
/// offers those anywhere (docs/phones.md, "Autofill").
pub fn fills_anywhere(item: &Item, kind: ItemKind) -> bool {
    matches!(kind, ItemKind::Card | ItemKind::Identity) && item.kind == kind && item.in_use()
}

/// What autofill's list says under a card's or an identity's name, from its
/// plain parts: a card's brand, with its number's last 4 digits only for
/// someone who may see the number (`sees_secrets`, the item's grid); an
/// identity's name, else its email.
pub fn fill_subtitle(item: &Item, sees_secrets: bool) -> String {
    if let Some(card) = &item.card {
        return if sees_secrets {
            card.summary()
        } else {
            card.brand.clone().unwrap_or_default()
        };
    }
    if let Some(identity) = &item.identity {
        let name = identity.full_name();
        return if name.is_empty() {
            identity.email.clone().unwrap_or_default()
        } else {
            name
        };
    }
    String::new()
}

/// A login iOS may suggest above the keyboard (docs/phones.md, "Autofill"):
/// its username, and a domain its websites fill on. Never its password.
#[derive(Debug, PartialEq, Eq)]
pub struct Suggestion<'a> {
    pub id: &'a str,
    pub username: &'a str,
    pub domain: String,
}

impl Vault {
    /// The logins for the target, by name.
    pub fn matching(&self, target: &Target) -> Vec<&Item> {
        let mut found: Vec<&Item> = self.items.iter().filter(|i| matches(i, target)).collect();
        found.sort_by_key(|i| i.name.to_lowercase());
        found
    }

    /// The cards or the identities autofill offers, whatever the app or the
    /// site (`fills_anywhere`), by name.
    pub fn fillable_anywhere(&self, kind: ItemKind) -> Vec<&Item> {
        let mut found: Vec<&Item> = self
            .items
            .iter()
            .filter(|i| fills_anywhere(i, kind))
            .collect();
        found.sort_by_key(|i| i.name.to_lowercase());
        found
    }

    /// What iOS may suggest, for every login (`suggested`).
    pub fn suggestions(&self) -> Vec<Suggestion<'_>> {
        self.items.iter().flat_map(suggested).collect()
    }

    /// The passkeys iOS may suggest (`passkey_suggested`).
    pub fn passkey_suggestions(&self) -> Vec<PasskeySuggestion<'_>> {
        self.items.iter().filter_map(passkey_suggested).collect()
    }

    /// The card in use with this number (by its digits), if there's one,
    /// among those whose number the person may see: someone who can't see
    /// a shared card's number isn't told it's that one (`same_card`).
    pub fn card_with_number(&self, number: &str) -> Option<&Item> {
        let typed = card_digits(number);
        self.items
            .iter()
            .find(|i| same_card(i, &typed) && self.grid(i).see_passwords)
    }

    /// The identity in use at this address, if there's one (`same_address`).
    pub fn identity_at(&self, street: &str, postal_code: &str) -> Option<&Item> {
        let (street, postal_code) = (street_words(street), postal_key(postal_code));
        self.items
            .iter()
            .find(|i| same_address(i, &street, &postal_code))
    }

    /// What tells a new card or address from one the person has, for
    /// Android's own check before it offers to save (`save_hints`).
    pub fn save_hints(&self) -> SaveHints {
        save_hints(self.items.iter().map(|i| (i, self.grid(i).see_passwords)))
    }

    /// Whether a new passkey for `site` may go in a login, and if so,
    /// whether it replaces the login's passkey (docs/core.md, "Passkeys",
    /// "Where it goes"). The refusal says why not.
    ///
    /// - **A login the person may change:** in use (not in the trash or
    ///   archived), that they may edit and whose passwords they may see, as
    ///   the apps' editor needs. So never an organization's login in shared
    ///   folders they can only view, or that hide its passwords. A kind an
    ///   organization's policy removes isn't in the vault at all
    ///   (restricted_types.rs).
    /// - **Not one of their own** where an organization keeps items out of
    ///   Personal (`Vault::keeps_items_out`): the server refuses an edit to
    ///   it. Its owners and admins aren't bound, so theirs still take one.
    /// - **With the site's passkey for this account** (the same site ID and
    ///   user handle): the new one replaces it, as WebAuthn has an
    ///   authenticator do, whatever the login's websites.
    /// - **With no passkey:** one for the site by the fill list's rules: one
    ///   of its websites for the site's ID, by its match setting, or the
    ///   Android app's own `androidapp://` website when an app is asking.
    /// - **Never one with another passkey** (another account's, or one
    ///   PassBundle can't read), which the new one would take the place of.
    pub fn passkey_target(&self, item: &Item, site: &PasskeySite<'_>) -> Result<bool> {
        if item.kind != ItemKind::Login {
            return Err(Error::Refused("a passkey goes in a login"));
        }
        if item.deleted {
            return Err(Error::Refused("that login is in the trash"));
        }
        if item.archived {
            return Err(Error::Refused("that login is archived"));
        }
        if item.organization_id.is_none() && self.keeps_items_out.is_some() {
            return Err(Error::Refused(
                "your organization keeps items out of Personal, so that login can't be changed",
            ));
        }
        let grid = self.grid(item);
        if !grid.edit {
            return Err(Error::Refused(
                "your organization doesn't let you change that login",
            ));
        }
        if !grid.see_passwords {
            return Err(Error::Refused(
                "you can't see that login's passwords, so a passkey can't go in it",
            ));
        }
        if has_passkey_for(item, site) {
            return Ok(true);
        }
        if item.passkey.is_some() || holds_unread_passkey(item) {
            return Err(Error::Refused(
                "that login has a passkey for another account or site",
            ));
        }
        let for_site = matches(item, &Target::Website(site.rp_id))
            || site.app.is_some_and(|app| matches(item, &Target::App(app)));
        if !for_site {
            return Err(Error::Refused("that login isn't for this site"));
        }
        Ok(false)
    }

    /// The logins a new passkey for `site` may go in (`passkey_target`),
    /// each with whether it replaces the login's passkey: those first, then
    /// by name.
    pub fn passkey_logins(&self, site: &PasskeySite<'_>) -> Vec<(&Item, bool)> {
        let mut found: Vec<(&Item, bool)> = self
            .items
            .iter()
            .filter_map(|i| self.passkey_target(i, site).ok().map(|r| (i, r)))
            .collect();
        found.sort_by_key(|(i, replaces)| (!replaces, i.name.to_lowercase()));
        found
    }

    /// Whether the vault has, outside the trash, a passkey for the site
    /// (its ID) with one of these credential IDs (raw): one a site lists
    /// among the passkeys it has already (`excludeCredentials`).
    pub fn has_passkey_among(&self, rp_id: &str, ids: &[Vec<u8>]) -> bool {
        !ids.is_empty()
            && self.items.iter().filter(|i| !i.deleted).any(|i| {
                i.passkey.as_ref().is_some_and(|p| {
                    p.rp_id == rp_id
                        && crate::passkey::credential_id(&p.credential_id)
                            .is_ok_and(|id| ids.contains(&id))
                })
            })
    }
}

/// A new passkey's site and the account on it, for where it may go
/// (`Vault::passkey_target`).
#[derive(Clone, Copy, Debug)]
pub struct PasskeySite<'a> {
    /// The site's ID, which the passkey is bound to.
    pub rp_id: &'a str,
    /// The site's ID for the account (WebAuthn's user handle), raw.
    pub user_handle: &'a [u8],
    /// The Android app asking, by its package, once the site's Digital
    /// Asset Links name it (asset_links.rs): its own logins are for the
    /// site too, as autofill offers them in that app. None for a browser,
    /// and on iOS.
    pub app: Option<&'a str>,
}

/// Whether a login has the site's passkey for this account: the same site
/// ID and user handle, the passkey a new one for them replaces. Never for
/// an empty user handle, or one stored that isn't base64.
pub fn has_passkey_for(item: &Item, site: &PasskeySite<'_>) -> bool {
    !site.user_handle.is_empty()
        && item.passkey.as_ref().is_some_and(|p| {
            p.rp_id == site.rp_id
                && p.user_handle
                    .as_deref()
                    .and_then(crate::passkey::base64_either)
                    .is_some_and(|handle| handle == site.user_handle)
        })
}

/// Whether a login holds a passkey the core couldn't open (one whose values
/// don't decrypt, or of a shape it doesn't know): it's there, unseen, and
/// a new one would take its place.
fn holds_unread_passkey(item: &Item) -> bool {
    item.passkey.is_none()
        && item
            .kept
            .fido2_credentials
            .as_ref()
            .is_some_and(|stored| match stored {
                serde_json::Value::Null => false,
                serde_json::Value::Array(list) => !list.is_empty(),
                _ => true,
            })
}

/// Whether an item is a card in use with this number, as its digits
/// (`card_digits`); never for no number.
pub fn same_card(item: &Item, digits: &str) -> bool {
    !digits.is_empty()
        && fills_anywhere(item, ItemKind::Card)
        && item
            .card
            .as_ref()
            .and_then(|c| c.number.as_deref())
            .is_some_and(|n| card_digits(n) == digits)
}

/// Whether an item is an identity in use at this address: its first street
/// line's words (`street_words`) and its postal code (`postal_key`) are
/// these, neither of them empty.
pub fn same_address(item: &Item, street: &[String], postal_code: &str) -> bool {
    if street.is_empty() || postal_code.is_empty() || !fills_anywhere(item, ItemKind::Identity) {
        return false;
    }
    item.identity.as_ref().is_some_and(|d| {
        street_words(d.address1.as_deref().unwrap_or_default()) == street
            && postal_key(d.postal_code.as_deref().unwrap_or_default()) == postal_code
    })
}

/// What Android checks what was typed against before it offers to save a
/// card or an address (Kotlin's `Saving.kt` writes the check, and Android
/// runs it): never a whole number. Each card's last 4 digits, only for one
/// whose number the person may see, as autofill's list already shows them
/// (`fill_subtitle`); and each identity's first street line, as its words,
/// with its postal code, for one that has both. A number with the same
/// last 4 digits isn't offered then, even if it's another card; one that is
/// offered is checked in full once the person accepts (`card_with_number`).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SaveHints {
    pub last4: Vec<String>,
    pub addresses: Vec<(Vec<String>, String)>,
}

/// The hints from items in use, each with whether the person may see its
/// secrets, sorted and each once.
pub fn save_hints<'a>(items: impl IntoIterator<Item = (&'a Item, bool)>) -> SaveHints {
    let mut hints = SaveHints::default();
    for (item, sees_secrets) in items {
        if sees_secrets && fills_anywhere(item, ItemKind::Card) {
            if let Some(last4) = item.card.as_ref().and_then(card_last4) {
                hints.last4.push(last4);
            }
        }
        if fills_anywhere(item, ItemKind::Identity) {
            let Some(identity) = &item.identity else {
                continue;
            };
            let street = street_words(identity.address1.as_deref().unwrap_or_default());
            let postal_code = postal_key(identity.postal_code.as_deref().unwrap_or_default());
            if !street.is_empty() && !postal_code.is_empty() {
                hints.addresses.push((street, postal_code));
            }
        }
    }
    hints.last4.sort();
    hints.last4.dedup();
    hints.addresses.sort();
    hints.addresses.dedup();
    hints
}

/// A card's number as it's compared: its digits only ("4111 1111-1111
/// 1111" is "4111111111111111").
pub fn card_digits(number: &str) -> String {
    number.chars().filter(char::is_ascii_digit).collect()
}

/// A card's number's last 4 digits, when it has more than 4 (as
/// `Card::summary` shows them).
fn card_last4(card: &crate::vault::Card) -> Option<String> {
    let digits = card_digits(card.number.as_deref()?);
    (digits.len() > 4).then(|| digits[digits.len() - 4..].to_owned())
}

/// A street as it's compared: its words, lowercase, whatever is between
/// them ("12 Main St." and "12  main st" are the same).
pub fn street_words(street: &str) -> Vec<String> {
    street
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// A postal code as it's compared: its letters and digits, lowercase
/// ("SW1A 1AA" and "sw1a1aa" are the same).
pub fn postal_key(postal_code: &str) -> String {
    postal_code
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// A passkey iOS may suggest in its passkey sheet (docs/phones.md,
/// "Passkeys"): the site's ID, the account's name, and the IDs a site and
/// iOS know it by. Never its key.
#[derive(Debug, PartialEq, Eq)]
pub struct PasskeySuggestion<'a> {
    pub id: &'a str,
    pub rp_id: &'a str,
    /// The account's name on the site, or the site's ID without one (never
    /// the login's own name, which iOS would show with it).
    pub user_name: &'a str,
    pub credential_id: Vec<u8>,
    /// Empty when none is stored.
    pub user_handle: Vec<u8>,
}

/// What iOS may suggest of an item's passkey: one the core can sign with,
/// that a site can find by itself (discoverable), in a login in use (not in
/// the trash or archived).
pub fn passkey_suggested(item: &Item) -> Option<PasskeySuggestion<'_>> {
    let passkey = item.passkey.as_ref()?;
    if !item.in_use() || item.kind != ItemKind::Login || !passkey.usable || !passkey.discoverable {
        return None;
    }
    let credential_id = crate::passkey::credential_id(&passkey.credential_id).ok()?;
    let user_handle = match &passkey.user_handle {
        Some(handle) => crate::passkey::base64_either(handle)?,
        None => Vec::new(),
    };
    Some(PasskeySuggestion {
        id: &item.id,
        rp_id: &passkey.rp_id,
        user_name: passkey
            .user_name
            .as_deref()
            .filter(|u| !u.is_empty())
            .unwrap_or(&passkey.rp_id),
        credential_id,
        user_handle,
    })
}

/// What iOS may suggest for one item: a login with a username, once for each
/// domain its websites fill on (the registrable domain by default, the host
/// when matched by host). Websites that need a page's whole address, never
/// fill, or are an Android app's give none.
pub fn suggested(item: &Item) -> Vec<Suggestion<'_>> {
    let mut found = Vec::new();
    let username = match item.username.as_deref() {
        Some(u) if !u.is_empty() && item.kind == ItemKind::Login && item.in_use() => u,
        _ => return found,
    };
    let mut domains = HashSet::new();
    for (at, uri) in item.uris.iter().enumerate() {
        if uri.trim().starts_with("androidapp://") {
            continue;
        }
        let Some((host, port)) = host_and_port(uri) else {
            continue;
        };
        let domain = match item
            .uri_matches
            .get(at)
            .copied()
            .flatten()
            .unwrap_or(DOMAIN)
        {
            DOMAIN => registrable(&host),
            HOST if port.is_none() => host,
            _ => continue,
        };
        if domains.insert(domain.clone()) {
            found.push(Suggestion {
                id: &item.id,
                username,
                domain,
            });
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn login(uris: &[(&str, Option<u8>)]) -> Item {
        Item {
            card: None,
            identity: None,
            ssh_key: None,
            attachments: Vec::new(),
            passkey: None,
            kept: Default::default(),
            id: "id".into(),
            kind: ItemKind::Login,
            name: "Login".into(),
            folder_id: None,
            organization_id: None,
            collection_ids: vec![],
            favorite: false,
            deleted: false,
            archived: false,
            username: Some("ana@example.test".into()),
            uris: uris.iter().map(|(u, _)| (*u).to_owned()).collect(),
            uri_matches: uris.iter().map(|(_, m)| *m).collect(),
            notes: None,
            password: None,
            totp: None,
            fields: vec![],
            revision_date: None,
            creation_date: None,
            editable: true,
            view_password: true,
            deletable: true,
            details: Vec::new(),
            organization_use_totp: false,
        }
    }

    fn card(number: &str, brand: &str) -> Item {
        Item {
            kind: ItemKind::Card,
            name: "Visa".into(),
            username: None,
            card: Some(crate::vault::Card {
                cardholder_name: Some("Ana López".into()),
                brand: Some(brand.into()),
                exp_month: Some("7".into()),
                exp_year: Some("2029".into()),
                number: Some(zeroize::Zeroizing::new(number.into())),
                code: Some(zeroize::Zeroizing::new("123".into())),
            }),
            ..login(&[])
        }
    }

    fn identity() -> Item {
        Item {
            kind: ItemKind::Identity,
            name: "Home".into(),
            username: None,
            identity: Some(crate::vault::Identity {
                title: Some("Ms".into()),
                first_name: Some("Ana".into()),
                middle_name: Some("María".into()),
                last_name: Some("López".into()),
                address1: Some("1 Example Street".into()),
                address2: Some("Apt 2".into()),
                city: Some("Springfield".into()),
                state: Some("IL".into()),
                postal_code: Some("62701".into()),
                country: Some("US".into()),
                email: Some("ana@example.test".into()),
                ..Default::default()
            }),
            ..login(&[])
        }
    }

    fn site(uri: &str, m: Option<u8>, page: &str) -> bool {
        matches(&login(&[(uri, m)]), &Target::Website(page))
    }

    #[test]
    fn by_default_the_same_registrable_domain_fills() {
        assert!(site(
            "https://accounts.example.com/login",
            None,
            "www.example.com"
        ));
        assert!(site("https://example.com", Some(0), "example.com"));
        assert!(
            site("example.com/sign-in", None, "www.example.com"),
            "no scheme"
        );
        assert!(
            site("HTTPS://Example.COM", None, "login.example.com"),
            "any case"
        );
        assert!(
            site("https://ana@example.com:8443/", None, "example.com"),
            "a username and port"
        );
        assert!(
            site("https://example.com./", None, "example.com"),
            "a trailing dot"
        );
        assert!(!site("https://example.com", None, "example.org"));
        assert!(!site("https://example.com.evil.test", None, "example.com"));
    }

    #[test]
    fn the_public_suffix_list_says_where_a_domain_starts() {
        assert!(site(
            "https://shop.example.co.uk",
            None,
            "www.example.co.uk"
        ));
        assert!(!site("https://example.co.uk", None, "other.co.uk"));
        assert!(!site("https://ana.github.io", None, "ben.github.io"));
        assert!(site("https://ana.github.io/blog", None, "ana.github.io"));
    }

    #[test]
    fn addresses_and_names_without_a_dot_are_their_own() {
        assert!(site("http://192.168.1.10:8080/admin", None, "192.168.1.10"));
        assert!(!site("http://192.168.1.10", None, "192.168.1.11"));
        assert!(site("http://[::1]:3000", None, "::1"));
        assert!(site("http://localhost:5000", None, "localhost"));
        assert!(!site("http://intranet", None, "localhost"));
    }

    #[test]
    fn host_means_the_same_host() {
        assert!(site("https://a.example.com", Some(HOST), "a.example.com"));
        assert!(!site("https://a.example.com", Some(HOST), "b.example.com"));
        assert!(!site("https://example.com", Some(HOST), "www.example.com"));
        assert!(
            !site("https://a.example.com:8443", Some(HOST), "a.example.com"),
            "the port isn't known"
        );
    }

    #[test]
    fn rules_that_need_the_whole_address_or_never_fill_by_domain_arent_offered() {
        for m in [2, 3, 4, 5] {
            assert!(
                !site("https://example.com", Some(m), "example.com"),
                "match {m}"
            );
        }
    }

    #[test]
    fn an_app_matches_only_its_own_androidapp_link() {
        let item = login(&[
            ("androidapp://com.example.mail", None),
            ("https://mail.example.com", None),
        ]);
        assert!(matches(&item, &Target::App("com.example.mail")));
        assert!(!matches(&item, &Target::App("com.example.mail.evil")));
        assert!(!matches(&item, &Target::App("com.example")));
        assert!(matches(&item, &Target::Website("mail.example.com")));
        assert!(!matches(
            &login(&[("https://mail.example.com", None)]),
            &Target::App("com.example.mail")
        ));
        assert!(!site(
            "androidapp://com.example.mail",
            None,
            "com.example.mail"
        ));
    }

    #[test]
    fn only_logins_in_the_vault_fill() {
        let mut item = login(&[("https://example.com", None)]);
        item.deleted = true;
        assert!(!matches(&item, &Target::Website("example.com")));
        let mut archived = login(&[("https://example.com", None)]);
        archived.archived = true;
        assert!(!matches(&archived, &Target::Website("example.com")));
        let mut note = login(&[("https://example.com", None)]);
        note.kind = ItemKind::SecureNote;
        assert!(!matches(&note, &Target::Website("example.com")));
    }

    #[test]
    fn any_of_an_items_websites_can_match() {
        let item = login(&[
            ("https://example.org", None),
            ("https://example.com", Some(5)),
            ("https://login.example.net", None),
        ]);
        assert!(matches(&item, &Target::Website("www.example.net")));
        assert!(!matches(&item, &Target::Website("example.com")));
    }

    #[test]
    fn cards_and_identities_fill_anywhere_and_logins_never_do() {
        let visa = card("4111 1111 1111 1234", "Visa");
        assert!(fills_anywhere(&visa, ItemKind::Card));
        assert!(!fills_anywhere(&visa, ItemKind::Identity));
        assert!(fills_anywhere(&identity(), ItemKind::Identity));
        assert!(!fills_anywhere(&identity(), ItemKind::Card));
        // Never a login (those match by their websites) or another kind.
        let login = login(&[("https://example.com", None)]);
        assert!(!fills_anywhere(&login, ItemKind::Login));
        assert!(!fills_anywhere(&login, ItemKind::Card));
        // Only those in use: not in the trash, not archived.
        let deleted = Item {
            deleted: true,
            ..card("4111 1111 1111 1234", "Visa")
        };
        let archived = Item {
            archived: true,
            ..identity()
        };
        assert!(!fills_anywhere(&deleted, ItemKind::Card));
        assert!(!fills_anywhere(&archived, ItemKind::Identity));
        // A card never matches a website or an app as a login would.
        assert!(!matches(&visa, &Target::Website("example.com")));
    }

    #[test]
    fn a_cards_subtitle_shows_its_last_digits_only_to_someone_who_may_see_them() {
        let visa = card("4111 1111 1111 1234", "Visa");
        assert_eq!(fill_subtitle(&visa, true), "Visa, *1234");
        assert_eq!(fill_subtitle(&visa, false), "Visa");
        let no_brand = card("4111 1111 1111 1234", "");
        assert_eq!(fill_subtitle(&no_brand, false), "");
        // An identity by its name, else its email.
        assert_eq!(fill_subtitle(&identity(), false), "Ms Ana María López");
        let mut emailed = identity();
        if let Some(i) = emailed.identity.as_mut() {
            (i.title, i.first_name, i.middle_name, i.last_name) = (None, None, None, None);
        }
        assert_eq!(fill_subtitle(&emailed, true), "ana@example.test");
        assert_eq!(fill_subtitle(&login(&[]), true), "");
    }

    #[test]
    fn a_cards_and_an_identitys_parts_are_there_one_at_a_time() {
        let visa = card("4111 1111 1111 1234", "Visa");
        let part = |item: &Item, what: &str| item.detail(what).map(|v| v.as_str().to_owned());
        assert_eq!(part(&visa, "expMonth").as_deref(), Some("7"));
        assert_eq!(part(&visa, "expYear").as_deref(), Some("2029"));
        assert_eq!(
            part(&visa, "cardNumber").as_deref(),
            Some("4111 1111 1111 1234")
        );
        // A card has no identity's parts, and an identity no card's.
        assert_eq!(part(&visa, "city"), None);
        let home = identity();
        assert_eq!(part(&home, "expMonth"), None);
        for (what, want) in [
            ("title", "Ms"),
            ("firstName", "Ana"),
            ("middleName", "María"),
            ("lastName", "López"),
            ("address1", "1 Example Street"),
            ("address2", "Apt 2"),
            ("city", "Springfield"),
            ("state", "IL"),
            ("postalCode", "62701"),
            ("country", "US"),
            ("email", "ana@example.test"),
        ] {
            assert_eq!(part(&home, what).as_deref(), Some(want), "{what}");
            // Plain values, like the rest of an identity's name and address.
            assert!(!home.is_secret(what), "{what}");
        }
        // Empty ones are none.
        assert_eq!(part(&home, "address3"), None);
        assert_eq!(part(&home, "company"), None);
        // The month and year are plain; the number and code stay secrets.
        assert!(!visa.is_secret("expMonth") && !visa.is_secret("expYear"));
        assert!(visa.is_secret("cardNumber") && visa.is_secret("cardCode"));
    }

    #[test]
    fn a_card_already_saved_has_the_same_number_however_it_was_typed() {
        let visa = card("4111 1111 1111 1234", "Visa");
        for typed in [
            "4111111111111234",
            "4111 1111 1111 1234",
            "4111-1111-1111-1234",
        ] {
            assert!(same_card(&visa, &card_digits(typed)), "{typed}");
        }
        assert!(!same_card(&visa, &card_digits("4111 1111 1111 1235")));
        // A number is needed: an empty one is nobody's.
        assert!(!same_card(&visa, &card_digits("")));
        assert!(!same_card(&card("", "Visa"), &card_digits("")));
        // Only a card in use: not in the trash, not archived, not an identity.
        let deleted = Item {
            deleted: true,
            ..card("4111 1111 1111 1234", "Visa")
        };
        let archived = Item {
            archived: true,
            ..card("4111 1111 1111 1234", "Visa")
        };
        for other in [deleted, archived, identity()] {
            assert!(!same_card(&other, "4111111111111234"));
        }
    }

    #[test]
    fn an_address_already_saved_has_the_same_street_and_postal_code() {
        let home = identity();
        let at = |street: &str, postal_code: &str| {
            same_address(&home, &street_words(street), &postal_key(postal_code))
        };
        assert!(at("1 Example Street", "62701"));
        // Written another way: case, spaces, and what's between the words.
        assert!(at("1  example street.", " 62701 "));
        assert!(!at("2 Example Street", "62701"));
        assert!(!at("1 Example Street", "62702"));
        assert!(!at("1 Example", "62701"));
        // Both are needed.
        assert!(!at("1 Example Street", ""));
        assert!(!at("", "62701"));
        let mut no_postal_code = identity();
        if let Some(i) = no_postal_code.identity.as_mut() {
            i.postal_code = None;
        }
        assert!(!same_address(
            &no_postal_code,
            &street_words("1 Example Street"),
            ""
        ));
        // Only an identity in use.
        let deleted = Item {
            deleted: true,
            ..identity()
        };
        let street = street_words("1 Example Street");
        assert!(!same_address(&deleted, &street, "62701"));
        assert!(!same_address(
            &card("4111 1111 1111 1234", "Visa"),
            &street,
            "62701"
        ));
        // Postal codes with letters.
        assert_eq!(postal_key("SW1A 1AA"), postal_key("sw1a1aa"));
        assert_eq!(
            street_words("12, Rue de l'Église"),
            ["12", "rue", "de", "l", "église"]
        );
    }

    #[test]
    fn save_hints_are_cards_last_digits_the_person_may_see_and_streets_with_postal_codes() {
        let visa = card("4111 1111 1111 1234", "Visa");
        let shared = card("5555 5555 5555 4444", "Mastercard");
        let short = card("123", "Visa");
        let deleted = Item {
            deleted: true,
            ..card("4111 1111 1111 9999", "Visa")
        };
        let mut no_postal_code = identity();
        if let Some(i) = no_postal_code.identity.as_mut() {
            i.postal_code = None;
        }
        let hints = save_hints([
            (&visa, true),
            // A shared card whose number the person may not see: none of it.
            (&shared, false),
            (&short, true),
            (&deleted, true),
            (&visa, true),
            (&identity(), false),
            (&identity(), true),
            (&no_postal_code, true),
            (&login(&[("https://example.com", None)]), true),
        ]);
        assert_eq!(
            hints,
            SaveHints {
                last4: vec!["1234".into()],
                addresses: vec![(
                    vec!["1".into(), "example".into(), "street".into()],
                    "62701".into()
                )],
            }
        );
    }

    #[test]
    fn suggestions_name_each_login_once_per_domain_never_a_password() {
        let item = Item {
            id: "a".into(),
            ..login(&[
                ("https://accounts.example.com/login", None),
                ("https://www.example.com", Some(0)),
                ("https://mail.example.net", Some(1)),
                ("https://intranet.example.org:8443", Some(1)),
                ("https://example.biz/start", Some(2)),
                ("^https://.*$", Some(4)),
                ("https://never.example.info", Some(5)),
                ("androidapp://com.example.mail", None),
            ])
        };
        assert_eq!(
            suggested(&item),
            vec![
                Suggestion {
                    id: "a",
                    username: "ana@example.test",
                    domain: "example.com".into()
                },
                Suggestion {
                    id: "a",
                    username: "ana@example.test",
                    domain: "mail.example.net".into()
                },
            ]
        );
        let no_username = Item {
            username: None,
            ..login(&[("https://example.com", None)])
        };
        let deleted = Item {
            deleted: true,
            ..login(&[("https://example.com", None)])
        };
        let archived = Item {
            archived: true,
            ..login(&[("https://example.com", None)])
        };
        let note = Item {
            kind: ItemKind::SecureNote,
            ..login(&[("https://example.com", None)])
        };
        for item in [no_username, deleted, archived, note] {
            assert!(suggested(&item).is_empty());
        }
    }

    /// The vectors' sample vault (tests/vectors/core-v1.json), opened with
    /// its user key, each item one the person may edit and see, and
    /// Contoso's shared folder theirs to change, with its sync changed by
    /// `edit`.
    fn sample(edit: impl FnOnce(&mut serde_json::Value)) -> Vault {
        use base64::Engine;
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../tests/vectors/core-v1.json")).unwrap();
        let mut sync = v["sync"].clone();
        for c in sync["ciphers"].as_array_mut().unwrap() {
            c["edit"] = true.into();
            c["viewPassword"] = true.into();
        }
        sync["collections"][0]["keykeeper"] = serde_json::json!({
            "seePasswords": true, "create": true, "edit": true, "delete": true, "export": true,
            "share": true, "print": true, "viewHistory": true, "manage": false,
        });
        edit(&mut sync);
        let key = base64::engine::general_purpose::STANDARD
            .decode(v["keys"]["userKey"].as_str().unwrap())
            .unwrap();
        Vault::open(
            &crate::vault::Sync::from_json(&sync.to_string()).unwrap(),
            crate::crypto::SymmetricKey::from_bytes(&key).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn where_an_organization_keeps_items_out_of_personal_no_login_of_ones_own_takes_a_passkey() {
        // The sample's Backup service has this account's passkey for
        // backup.example.test, and Example Mail is for example.test: both
        // the person's own. Office Wi-Fi is Contoso's.
        const BACKUP: &str = "5f607182-93a4-4b52-cde3-f40516273849";
        const MAIL: &str = "3d4e5f60-7182-4930-abc1-d2e3f4051627";
        const OFFICE_WIFI: &str = "4e5f6071-8293-4a41-bcd2-e3f405162738";
        const CONTOSO: &str = "8f9a7c1e-3b2d-4e5f-9a8b-7c6d5e4f3a2b";
        let ana: Vec<u8> = (0xa0..=0xaf).collect();
        let backup = PasskeySite {
            rp_id: "backup.example.test",
            user_handle: &ana,
            app: None,
        };
        let offered = |vault: &Vault, site: &PasskeySite<'_>| -> Vec<(String, bool)> {
            vault
                .passkey_logins(site)
                .into_iter()
                .map(|(i, replaces)| (i.id.clone(), replaces))
                .collect()
        };
        let keeping_out = |role: u64, enabled: bool| {
            move |sync: &mut serde_json::Value| {
                sync["profile"]["organizations"][0]["type"] = role.into();
                sync["policies"] = serde_json::json!([
                    { "organizationId": CONTOSO, "type": 5, "enabled": enabled }
                ]);
            }
        };

        // A member it binds: neither of their own logins takes one, not
        // even the one whose passkey it would replace, since the server
        // refuses an edit to it.
        let mut member = sample(keeping_out(2, true));
        assert_eq!(member.keeps_items_out.as_deref(), Some("Contoso"));
        assert!(offered(&member, &backup).is_empty());
        for id in [BACKUP, MAIL] {
            assert_eq!(
                member.passkey_target(member.item(id).unwrap(), &backup),
                Err(Error::Refused(
                    "your organization keeps items out of Personal, so that login can't be changed"
                ))
            );
        }
        // Contoso's own login, with a website for the site, still does.
        let wifi = member
            .items
            .iter_mut()
            .find(|i| i.id == OFFICE_WIFI)
            .unwrap();
        wifi.uris = vec!["https://office.example.test".into()];
        wifi.uri_matches = vec![None];
        let office = PasskeySite {
            rp_id: "office.example.test",
            ..backup
        };
        assert_eq!(offered(&member, &office), [(OFFICE_WIFI.to_owned(), false)]);

        // Its owners and admins aren't bound, nor is anyone while it's off:
        // their own logins are offered as ever.
        for (role, enabled) in [(0, true), (1, true), (2, false)] {
            let vault = sample(keeping_out(role, enabled));
            assert_eq!(vault.keeps_items_out, None);
            assert_eq!(
                offered(&vault, &backup),
                [(BACKUP.to_owned(), true), (MAIL.to_owned(), false)]
            );
        }

        // A member whose organization key hasn't reached them yet
        // (passbundle-server's organizationsNew) is bound too.
        let joining = sample(|sync| {
            sync["profile"]["organizationsNew"] =
                serde_json::json!([{ "id": "fabrikam", "name": "Fabrikam", "type": 2 }]);
            sync["policies"] = serde_json::json!([
                { "organizationId": "fabrikam", "type": 5, "enabled": true }
            ]);
        });
        assert_eq!(joining.keeps_items_out.as_deref(), Some("Fabrikam"));
        assert!(offered(&joining, &backup).is_empty());
    }
}
