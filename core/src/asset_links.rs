//! Digital Asset Links (docs/phones.md, "Passkeys"): whether a site lets an
//! Android app use its passkeys. An app that isn't a browser on Google's
//! privileged list gets a site's passkeys only if the site's
//! `https://<site>/.well-known/assetlinks.json` names that app's package and
//! signing certificate for `delegate_permission/common.get_login_creds`. The
//! core fetches it itself: HTTPS only, no redirects (so it's the site's own
//! answer), at most 64 KB, in 10 seconds. Answers are kept in memory for a day.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;
use ureq::tls::{RootCerts, TlsConfig};

use crate::error::{Error, Result};

const RELATION: &str = "delegate_permission/common.get_login_creds";
const MOST: u64 = 64 * 1024;
const KEPT_FOR: Duration = Duration::from_secs(24 * 60 * 60);

/// A site, an app's package, and its certificate's SHA-256.
type Asked = (String, String, [u8; 32]);

/// Answers already fetched, and when.
static KEPT: Mutex<Option<HashMap<Asked, (bool, Instant)>>> = Mutex::new(None);

/// Whether a site's statements (an assetlinks.json) let this app, signed with
/// this certificate (its SHA-256), use the site's sign-in credentials.
pub fn allows(statements: &str, package: &str, certificate: &[u8; 32]) -> bool {
    let Ok(Value::Array(statements)) = serde_json::from_str::<Value>(statements) else {
        return false;
    };
    statements.iter().any(|s| {
        let relations = s["relation"].as_array();
        let target = &s["target"];
        relations.is_some_and(|r| r.iter().any(|r| r.as_str() == Some(RELATION)))
            && target["namespace"].as_str() == Some("android_app")
            && target["package_name"].as_str() == Some(package)
            && target["sha256_cert_fingerprints"]
                .as_array()
                .is_some_and(|f| {
                    f.iter()
                        .filter_map(Value::as_str)
                        .any(|f| fingerprint(f) == Some(*certificate))
                })
    })
}

/// A fingerprint as asset links write it ("AB:CD:..."), as bytes.
fn fingerprint(text: &str) -> Option<[u8; 32]> {
    // Hex digits and colons only, checked before slicing by byte: anything
    // else (a site's file is anyone's text) is no fingerprint.
    if !text.bytes().all(|b| b.is_ascii_hexdigit() || b == b':') {
        return None;
    }
    let hex: String = text.chars().filter(|c| *c != ':').collect();
    if hex.len() != 64 {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(bytes)
}

/// A site's ID that can be fetched as a host: letters, digits, hyphens, and
/// dots, nothing that could change the address.
fn host(rp_id: &str) -> Result<&str> {
    let labels: Vec<&str> = rp_id.split('.').collect();
    // A domain: lower case, two labels or more, and not an address.
    let ok = !rp_id.is_empty()
        && rp_id.len() <= 253
        && labels.len() >= 2
        && !labels
            .last()
            .is_some_and(|tld| tld.bytes().all(|b| b.is_ascii_digit()))
        && !rp_id.bytes().any(|b| b.is_ascii_uppercase())
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        });
    if ok {
        Ok(rp_id)
    } else {
        Err(Error::Refused("a site's ID that isn't a host name"))
    }
}

/// The site's statements, fetched from the site itself.
pub fn fetch(rp_id: &str) -> Result<String> {
    let url = format!("https://{}/.well-known/assetlinks.json", host(rp_id)?);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_global(Some(Duration::from_secs(10)))
        .user_agent(format!("PassBundle/{}", crate::version()))
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into();
    let mut response = agent
        .get(&url)
        .call()
        .map_err(|e| Error::Network(e.to_string()))?;
    if response.status().as_u16() != 200 {
        return Err(Error::Server(format!(
            "the site's asset links couldn't be read ({})",
            response.status().as_u16()
        )));
    }
    response
        .body_mut()
        .with_config()
        .limit(MOST)
        .read_to_string()
        .map_err(|e| Error::Network(e.to_string()))
}

/// Whether the site lets this app use its passkeys, from what was fetched in
/// the last day or by fetching now. `statements` stands in for fetching (the
/// tests, and debug builds pointing a test site at a file), and isn't kept.
pub fn check(
    rp_id: &str,
    package: &str,
    certificate: &[u8; 32],
    statements: Option<&str>,
) -> Result<bool> {
    if let Some(given) = statements {
        return Ok(allows(given, package, certificate));
    }
    let key = (host(rp_id)?.to_owned(), package.to_owned(), *certificate);
    {
        let kept = KEPT.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((answer, at)) = kept.as_ref().and_then(|k| k.get(&key)) {
            if at.elapsed() < KEPT_FOR {
                return Ok(*answer);
            }
        }
    }
    let answer = allows(&fetch(rp_id)?, package, certificate);
    KEPT.lock()
        .unwrap_or_else(|p| p.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(key, (answer, Instant::now()));
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CERT: [u8; 32] = [0xab; 32];

    fn statements(relation: &str, package: &str, fingerprint: &str) -> String {
        serde_json::json!([
            {"relation": ["delegate_permission/common.handle_all_urls"],
             "target": {"namespace": "web", "site": "https://example.test"}},
            {"relation": ["delegate_permission/common.handle_all_urls", relation],
             "target": {"namespace": "android_app", "package_name": package,
                        "sha256_cert_fingerprints": ["00:11", fingerprint]}}
        ])
        .to_string()
    }

    fn colons(bytes: &[u8]) -> String {
        bytes
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":")
    }

    #[test]
    fn a_site_lets_only_the_app_and_key_it_names_use_its_credentials() {
        let named = statements(RELATION, "com.example.app", &colons(&CERT));
        assert!(allows(&named, "com.example.app", &CERT));
        // Lower case and no colons read the same.
        let plain = statements(
            RELATION,
            "com.example.app",
            &colons(&CERT).replace(':', "").to_lowercase(),
        );
        assert!(allows(&plain, "com.example.app", &CERT));
        // Another package, another key, or only links: no.
        assert!(!allows(&named, "com.example.other", &CERT));
        assert!(!allows(&named, "com.example.app", &[0xcd; 32]));
        let links_only = statements(
            "delegate_permission/common.handle_all_urls",
            "com.example.app",
            &colons(&CERT),
        );
        assert!(!allows(&links_only, "com.example.app", &CERT));
        // Not statements at all.
        assert!(!allows("<html>", "com.example.app", &CERT));
        assert!(!allows("{}", "com.example.app", &CERT));
    }

    #[test]
    fn only_host_names_are_fetched() {
        assert!(host("example.test").is_ok());
        assert!(host("a-b.example.test").is_ok());
        for bad in [
            "",
            "example.test/evil",
            "evil.test#",
            "user@evil.test",
            "evil.test:8443",
            "-a.test",
            "a..test",
            "localhost",
            "10.0.0.1",
            "Example.test",
        ] {
            assert!(host(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn statements_given_are_read_but_not_kept() {
        let named = statements(RELATION, "com.example.kept", &colons(&CERT));
        assert!(check("kept.example.test", "com.example.kept", &CERT, Some(&named)).unwrap());
        // Not kept: asking without them fetches (which fails for this name).
        assert!(check("kept.example.test", "com.example.kept", &CERT, None).is_err());
    }

    #[test]
    fn a_fingerprint_that_isnt_hex_is_none_not_a_crash() {
        let odd = format!("{}\u{e9}A", "A".repeat(61));
        assert!(fingerprint(&odd).is_none());
        assert!(fingerprint(&format!("+{}", "A".repeat(63))).is_none());
        let named = statements(RELATION, "com.example.app", &odd);
        assert!(!allows(&named, "com.example.app", &CERT));
    }
}
