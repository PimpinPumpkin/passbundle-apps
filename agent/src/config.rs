//! Settings an organization sets by policy. On Windows the installer writes
//! them to `HKLM\SOFTWARE\Policies\<Name>\Agent`; on macOS an MDM profile sets
//! the `com.<domain>.agent` preference domain (for passbundle.com,
//! `com.passbundle.agent`). Nothing is read from places a standard user can
//! write, so a user or malware running as them can't point the agent elsewhere.

use crate::brand;

#[derive(Debug, PartialEq, Eq)]
pub struct Config {
    /// The web vault, opened from the tray menu.
    pub server_url: String,
    /// Off when the organization updates the agent itself (through its RMM).
    pub updates: bool,
}

/// What a policy can say. Missing values take the defaults.
#[derive(Debug, Default)]
pub struct Policy {
    pub server_url: Option<String>,
    pub updates: Option<bool>,
}

impl Config {
    pub fn from_policy(policy: Policy) -> Self {
        let server_url = policy
            .server_url
            .map(|url| url.trim().trim_end_matches('/').to_owned())
            .filter(|url| valid_server_url(url))
            .unwrap_or_else(|| brand::WEB_VAULT_URL.to_owned());
        Self {
            server_url,
            updates: policy.updates.unwrap_or(true),
        }
    }

    pub fn load() -> Self {
        Self::from_policy(read_policy())
    }
}

/// HTTPS only, like the apps: they refuse plain-HTTP servers.
fn valid_server_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    !host.is_empty()
        && !host.contains('@')
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// The macOS preference domain: the brand's domain reversed, plus "agent".
#[cfg(target_os = "macos")]
pub fn preference_domain() -> String {
    reverse_domain(brand::DOMAIN) + ".agent"
}

/// `passbundle.com` as `com.passbundle`, for names that follow that convention.
pub fn reverse_domain(domain: &str) -> String {
    let mut labels: Vec<&str> = domain.split('.').collect();
    labels.reverse();
    labels.join(".")
}

#[cfg(windows)]
fn read_policy() -> Policy {
    let path = format!(r"SOFTWARE\Policies\{}\Agent", brand::NAME);
    let Ok(key) = windows_registry::LOCAL_MACHINE.open(&path) else {
        return Policy::default();
    };
    Policy {
        server_url: key.get_string("ServerUrl").ok(),
        updates: key.get_u32("Updates").ok().map(|v| v != 0),
    }
}

#[cfg(target_os = "macos")]
fn read_policy() -> Policy {
    // Managed preferences, written by the MDM profile; root-owned.
    let path = format!("/Library/Managed Preferences/{}.plist", preference_domain());
    let Ok(plist::Value::Dictionary(values)) = plist::Value::from_file(path) else {
        return Policy::default();
    };
    Policy {
        server_url: values
            .get("ServerUrl")
            .and_then(|v| v.as_string())
            .map(str::to_owned),
        updates: values.get("Updates").and_then(|v| v.as_boolean()),
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn read_policy() -> Policy {
    Policy::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_policy_the_agent_uses_the_brand_server_and_updates_itself() {
        assert_eq!(
            Config::from_policy(Policy::default()),
            Config {
                server_url: brand::WEB_VAULT_URL.to_owned(),
                updates: true,
            }
        );
    }

    #[test]
    fn a_policy_sets_the_server_and_can_turn_updates_off() {
        let config = Config::from_policy(Policy {
            server_url: Some(" https://vault.example.test/ ".into()),
            updates: Some(false),
        });
        assert_eq!(config.server_url, "https://vault.example.test");
        assert!(!config.updates);
    }

    #[test]
    fn a_server_url_that_isnt_plain_https_is_ignored() {
        for bad in [
            "http://vault.example.test",
            "https://",
            "https://user@vault.example.test",
            "https://vault.example.test/a b",
            "javascript:alert(1)",
            "vault.example.test",
        ] {
            let config = Config::from_policy(Policy {
                server_url: Some(bad.into()),
                updates: None,
            });
            assert_eq!(config.server_url, brand::WEB_VAULT_URL, "{bad:?}");
        }
    }

    #[test]
    fn domains_reverse_label_by_label() {
        assert_eq!(reverse_domain("example.com"), "com.example");
        assert_eq!(reverse_domain("vault.example.co.uk"), "uk.co.example.vault");
    }
}
