//! "Change password" on a login with something to fix (DESIGN.md in the meta
//! repo, "Health (better than Watchtower)"): the site's own change-password
//! page, found the way the web vault and the browser extension find it
//! (passbundle-clients, default-change-login-password.service.ts). First
//! Apple's published list, on the device (`change_password/urls.tsv`); then,
//! for someone who shows websites' icons (who already tells the server which
//! sites they have logins for), the server's icon service
//! (`Server::change_password_uri`), which answers with the site's standard
//! address where the site supports it; else the website itself.

use std::collections::HashMap;
use std::sync::OnceLock;

/// Apple's list: each site's change-password page, by domain.
fn listed() -> &'static HashMap<&'static str, &'static str> {
    static LIST: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    LIST.get_or_init(|| {
        include_str!("change_password/urls.tsv")
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .collect()
    })
}

/// The page Apple's list has for a host: the host's own entry, or its
/// parent domain's ("www.example.com" takes "example.com"'s), as the web
/// vault looks it up. Never a whole top-level domain's.
pub fn listed_page(host: &str) -> Option<&'static str> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let labels: Vec<&str> = host.split('.').collect();
    (0..labels.len().saturating_sub(1))
        .find_map(|at| listed().get(labels[at..].join(".").as_str()).copied())
}

/// A login's websites that a change-password page can be for, in the
/// item's order: each as the browser opens it (https:// added to an address
/// written without a scheme, as the editor allows), with its host. Apps'
/// addresses (androidapp://, iosapp://) and anything else that isn't a
/// website are left out.
pub fn websites(uris: &[String]) -> Vec<(String, String)> {
    uris.iter()
        .filter_map(|uri| {
            let uri = uri.trim();
            let host = crate::icons::host_of(uri)?;
            let address = if uri.contains("://") {
                uri.to_owned()
            } else {
                format!("https://{uri}")
            };
            Some((address, host))
        })
        .collect()
}

/// Whether a page the server answered with may be opened for a website:
/// an http or https address on the website's own host, as the server's
/// lookup makes them (`https://<host>/.well-known/change-password`). Anything
/// else is ignored, so a server can't send the person somewhere else.
pub fn answer_fits(page: &str, host: &str) -> bool {
    let page = page.trim();
    let scheme_ok = page.split_once("://").is_some_and(|(scheme, _)| {
        matches!(scheme.to_ascii_lowercase().as_str(), "https" | "http")
    });
    scheme_ok && crate::icons::host_of(page).as_deref() == Some(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_has_every_entry_and_finds_a_site_by_its_parent_domain() {
        assert_eq!(listed().len(), 805);
        assert_eq!(
            listed_page("23andme.com"),
            Some("https://you.23andme.com/user/edit/password/")
        );
        // A host under a listed domain, in any case, with a dot at the end.
        assert_eq!(
            listed_page("WWW.23andMe.com."),
            Some("https://you.23andme.com/user/edit/password/")
        );
        assert_eq!(listed_page("example.test"), None);
        // Never by the top-level domain alone.
        assert_eq!(listed_page("com"), None);
        assert_eq!(listed_page(""), None);
    }

    #[test]
    fn only_websites_count_and_an_address_without_a_scheme_opens_over_https() {
        let uris = [
            "androidapp://com.example.app".to_owned(),
            "shop.example.test/sign-in".to_owned(),
            "http://old.example.test".to_owned(),
            "ftp://files.example.test".to_owned(),
            "not a website".to_owned(),
        ];
        assert_eq!(
            websites(&uris),
            vec![
                (
                    "https://shop.example.test/sign-in".to_owned(),
                    "shop.example.test".to_owned()
                ),
                (
                    "http://old.example.test".to_owned(),
                    "old.example.test".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn the_servers_answer_counts_only_for_the_websites_own_host() {
        let host = "shop.example.test";
        assert!(answer_fits(
            "https://shop.example.test/.well-known/change-password",
            host
        ));
        assert!(answer_fits(
            "http://shop.example.test/.well-known/change-password",
            host
        ));
        assert!(!answer_fits(
            "https://evil.example.test/.well-known/change-password",
            host
        ));
        assert!(!answer_fits(
            "https://shop.example.test.evil.example/change",
            host
        ));
        assert!(!answer_fits("javascript:alert(1)//shop.example.test", host));
        assert!(!answer_fits("shop.example.test/change", host));
        assert!(!answer_fits("", host));
    }
}
