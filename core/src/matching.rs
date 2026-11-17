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

use std::collections::HashSet;
use std::net::IpAddr;

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
}
