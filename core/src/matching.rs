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
        && !item.deleted
        && item
            .uris
            .iter()
            .enumerate()
            .any(|(at, uri)| fits(uri, item.uri_matches.get(at).copied().flatten(), target))
}

impl Vault {
    /// The logins for the target, by name.
    pub fn matching(&self, target: &Target) -> Vec<&Item> {
        let mut found: Vec<&Item> = self.items.iter().filter(|i| matches(i, target)).collect();
        found.sort_by_key(|i| i.name.to_lowercase());
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn login(uris: &[(&str, Option<u8>)]) -> Item {
        Item {
            card: None,
            identity: None,
            ssh_key: None,
            kept: Default::default(),
            id: "id".into(),
            kind: ItemKind::Login,
            name: "Login".into(),
            folder_id: None,
            organization_id: None,
            collection_ids: vec![],
            favorite: false,
            deleted: false,
            username: Some("ana@example.test".into()),
            uris: uris.iter().map(|(u, _)| (*u).to_owned()).collect(),
            uri_matches: uris.iter().map(|(_, m)| *m).collect(),
            notes: None,
            password: None,
            totp: None,
            fields: vec![],
            revision_date: None,
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
}
