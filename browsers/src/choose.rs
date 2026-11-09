//! Which profiles are chosen before the person changes anything (DESIGN.md,
//! "Which browser profiles"). In each browser, the first rule that picks
//! anything decides:
//!
//! 1. the profiles signed in with the person's PassBundle email (the one
//!    they were invited with);
//! 2. the profiles signed in with an email at one of their organizations'
//!    domains, where an admin set some;
//! 3. the browser's default profile, unless it's signed in to another
//!    account;
//! 4. otherwise none: the person is asked, once.
//!
//! A profile signed in to another account (one that's neither the person's
//! email nor at an organization's domain) is personal, and no rule picks it.
//! The person sees every profile, which is which, and why each is chosen or
//! not, and can change any of it before anything is read.

use crate::{Browser, Profile};

/// Whether a rule picks a profile.
type Picks<'a> = dyn Fn(&Profile) -> bool + 'a;

/// Why a profile is chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// Signed in with the person's PassBundle email.
    YourEmail,
    /// Signed in with an email at an organization's domain.
    WorkDomain,
    /// The browser's default profile, signed in to no other account.
    BrowserDefault,
}

/// A profile, chosen or not, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub chosen: bool,
    pub rule: Option<Rule>,
    /// Signed in to another account: personal, never chosen by a rule.
    pub personal: bool,
}

/// The choices for `profiles`, in their order. `email` is the person's
/// PassBundle email; `domains` their organizations' email domains.
pub fn choose(profiles: &[Profile], email: &str, domains: &[String]) -> Vec<Choice> {
    let email = email.trim().to_lowercase();
    let domains: Vec<String> = domains
        .iter()
        .map(|d| d.trim().trim_start_matches('@').to_lowercase())
        .filter(|d| !d.is_empty())
        .collect();
    let at_work = |e: &str| {
        e.rsplit_once('@')
            .is_some_and(|(_, d)| domains.iter().any(|w| w == d))
    };
    let yours = |p: &Profile| p.email.as_deref() == Some(email.as_str()) && !email.is_empty();
    let work = |p: &Profile| p.email.as_deref().is_some_and(at_work);
    let personal = |p: &Profile| p.email.is_some() && !yours(p) && !work(p);

    let mut choices: Vec<Choice> = profiles
        .iter()
        .map(|p| Choice {
            chosen: false,
            rule: None,
            personal: personal(p),
        })
        .collect();
    let mut browsers: Vec<Browser> = profiles.iter().map(|p| p.browser).collect();
    browsers.sort();
    browsers.dedup();
    for browser in browsers {
        let in_browser: Vec<usize> = (0..profiles.len())
            .filter(|&i| profiles[i].browser == browser)
            .collect();
        let rules: [(Rule, &Picks); 3] = [
            (Rule::YourEmail, &yours),
            (Rule::WorkDomain, &work),
            (Rule::BrowserDefault, &|p: &Profile| {
                p.default && !personal(p)
            }),
        ];
        for (rule, picks) in rules {
            let picked: Vec<usize> = in_browser
                .iter()
                .copied()
                .filter(|&i| picks(&profiles[i]))
                .collect();
            if picked.is_empty() {
                continue;
            }
            for i in picked {
                choices[i].chosen = true;
                choices[i].rule = Some(rule);
            }
            break;
        }
    }
    choices
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::Platform;

    fn profile(browser: Browser, name: &str, email: Option<&str>, default: bool) -> Profile {
        Profile {
            id: format!("{}:{name}", browser.key()),
            browser,
            name: name.into(),
            email: email.map(str::to_owned),
            managed_by: None,
            default,
            platform: Platform::Linux,
            dir: PathBuf::from(name),
            browser_dir: PathBuf::from(name),
        }
    }

    fn chosen(profiles: &[Profile], email: &str, domains: &[&str]) -> Vec<(String, Option<Rule>)> {
        let domains: Vec<String> = domains.iter().map(|d| d.to_string()).collect();
        profiles
            .iter()
            .zip(choose(profiles, email, &domains))
            .filter(|(_, c)| c.chosen)
            .map(|(p, c)| (p.id.clone(), c.rule))
            .collect()
    }

    #[test]
    fn the_profile_signed_in_with_the_invited_email_comes_first() {
        let profiles = [
            profile(Browser::Chrome, "Default", None, true),
            profile(Browser::Chrome, "Work", Some("ana@acme.test"), false),
            profile(Browser::Chrome, "Colleague", Some("bo@acme.test"), false),
        ];
        // Rule 1 wins over the domain and the default.
        assert_eq!(
            chosen(&profiles, "Ana@Acme.test", &["acme.test"]),
            [("chrome:Work".to_owned(), Some(Rule::YourEmail))]
        );
    }

    #[test]
    fn then_a_profile_at_an_organizations_domain_if_the_admin_set_one() {
        let profiles = [
            profile(Browser::Edge, "Default", None, true),
            profile(Browser::Edge, "Work", Some("ana@acme.test"), false),
        ];
        assert_eq!(
            chosen(&profiles, "ana.personal@mail.test", &["@ACME.test"]),
            [("edge:Work".to_owned(), Some(Rule::WorkDomain))]
        );
        // With no domain set, the default instead; the profile signed in to
        // another account is personal.
        assert_eq!(
            chosen(&profiles, "ana.personal@mail.test", &[]),
            [("edge:Default".to_owned(), Some(Rule::BrowserDefault))]
        );
        // Domains are exact: a subdomain isn't the organization's.
        assert_eq!(
            chosen(&profiles, "x@mail.test", &["sub.acme.test"]),
            [("edge:Default".to_owned(), Some(Rule::BrowserDefault))]
        );
    }

    #[test]
    fn then_the_browsers_default_profile() {
        let profiles = [
            profile(Browser::Firefox, "default-release", None, true),
            profile(Browser::Firefox, "other", None, false),
        ];
        assert_eq!(
            chosen(&profiles, "ana@acme.test", &[]),
            [(
                "firefox:default-release".to_owned(),
                Some(Rule::BrowserDefault)
            )]
        );
    }

    #[test]
    fn otherwise_nothing_is_chosen_and_the_person_is_asked() {
        let profiles = [
            profile(Browser::Chrome, "Default", Some("ana.home@mail.test"), true),
            profile(Browser::Chrome, "Projects", None, false),
        ];
        assert!(chosen(&profiles, "ana@acme.test", &["acme.test"]).is_empty());
    }

    #[test]
    fn a_personal_profile_is_never_chosen_even_as_the_default() {
        let profiles = [
            profile(Browser::Chrome, "Default", Some("ana.home@mail.test"), true),
            profile(Browser::Firefox, "default", None, true),
        ];
        let domains = vec!["acme.test".to_owned()];
        let choices = choose(&profiles, "ana@acme.test", &domains);
        assert_eq!(
            choices[0],
            Choice {
                chosen: false,
                rule: None,
                personal: true
            }
        );
        assert!(choices[1].chosen);
        assert!(!choices[1].personal);
    }

    #[test]
    fn each_browser_has_its_own_rules() {
        let profiles = [
            profile(Browser::Chrome, "Default", None, true),
            profile(Browser::Chrome, "Work", Some("ana@acme.test"), false),
            profile(Browser::Firefox, "default-release", None, true),
        ];
        assert_eq!(
            chosen(&profiles, "ana@acme.test", &[]),
            [
                ("chrome:Work".to_owned(), Some(Rule::YourEmail)),
                (
                    "firefox:default-release".to_owned(),
                    Some(Rule::BrowserDefault)
                ),
            ]
        );
    }
}
