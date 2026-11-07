//! The browser's half of signing in with a security key (W3C, "Web
//! Authentication" Level 2, `navigator.credentials.get()`), done by the core
//! for its own server: the server's request options are read and checked,
//! the client data is made for the server's own origin, and the key's
//! answer is checked and put in the shape the server reads.
//!
//! The origin is always the server the app signs in to, never anything the
//! server says, and the site ID it asks for must be that origin's host or a
//! domain above it that isn't a public suffix, as a browser requires. So a
//! server can only ever get a signature for itself: a hacked or phishing
//! server can't have the app sign another site's challenge.

use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::ctap::{KeyAnswer, KeyRequest};
use crate::error::{Error, Result};
use crate::passkey::base64_either;

/// A sign-in's request for a security key, checked.
#[derive(Clone, Debug)]
pub struct SecurityKeyRequest {
    /// The server's origin: `https://host[:port]`.
    pub origin: String,
    pub rp_id: String,
    /// The AppID a key added with upstream's older U2F dialog signs for,
    /// when the server names one on its own origin.
    pub app_id: Option<String>,
    pub allow: Vec<Vec<u8>>,
    pub timeout: Duration,
    /// User verification: `discouraged`, `preferred`, or `required`.
    pub user_verification: String,
    /// The client data, exactly as signed.
    pub client_data_json: Vec<u8>,
}

fn malformed(what: &str) -> Error {
    Error::Malformed(format!("a security key request with {what}"))
}

/// A URL's origin (`scheme://host[:port]`, the port left out when it's the
/// scheme's own) and host, for https, or http to this computer.
pub fn origin_of(url: &str) -> Option<(String, String)> {
    let (scheme, rest) = url.trim().split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (host, after) = v6.split_once(']')?;
        (format!("[{host}]"), after.strip_prefix(':'))
    } else {
        match authority.split_once(':') {
            Some((host, port)) => (host.to_owned(), Some(port)),
            None => (authority.to_owned(), None),
        }
    };
    let host = host.to_ascii_lowercase();
    let default = match scheme.as_str() {
        "https" => "443",
        "http" if host == "localhost" || host == "127.0.0.1" || host == "[::1]" => "80",
        _ => return None,
    };
    let port = port.filter(|p| !p.is_empty() && *p != default);
    if port.is_some_and(|p| !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let origin = match port {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    };
    Some((origin, host))
}

fn is_address(host: &str) -> bool {
    host.starts_with('[') || host.parse::<std::net::Ipv4Addr>().is_ok()
}

/// Whether a site ID may be used from a host: the host itself, or a domain
/// above it that isn't a public suffix (WebAuthn, "Relying Party
/// Identifier"). Never an address: WebAuthn takes only domains.
pub fn rp_id_allowed(rp_id: &str, host: &str) -> bool {
    if rp_id.is_empty() || is_address(host) || is_address(rp_id) {
        return false;
    }
    if rp_id == host {
        return true;
    }
    let below = host
        .strip_suffix(rp_id)
        .is_some_and(|start| start.ends_with('.'));
    let public = psl::suffix_str(rp_id) == Some(rp_id);
    below && !public
}

/// The client data (WebAuthn, "Client Data Used in WebAuthn Signatures"),
/// with its members in the order the specification serializes them.
pub fn client_data(challenge: &[u8], origin: &str) -> Vec<u8> {
    let challenge = URL_SAFE_NO_PAD.encode(challenge);
    format!(
        r#"{{"type":"webauthn.get","challenge":{},"origin":{},"crossOrigin":false}}"#,
        Value::String(challenge),
        Value::String(origin.to_owned()),
    )
    .into_bytes()
}

impl SecurityKeyRequest {
    /// The server's request options (its two-step login answer, provider
    /// 7), for a sign-in to the server at `server_url`.
    pub fn new(options: &Value, server_url: &str) -> Result<SecurityKeyRequest> {
        let (origin, host) = origin_of(server_url).ok_or(Error::TwoStep(
            "A security key works only with a server reached over HTTPS.",
        ))?;
        let options = options.get("publicKey").unwrap_or(options);
        let challenge = options["challenge"]
            .as_str()
            .and_then(base64_either)
            .filter(|c| c.len() >= 16 && c.len() <= 1024)
            .ok_or_else(|| malformed("no challenge"))?;
        let rp_id = options["rpId"]
            .as_str()
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| host.clone());
        if !rp_id_allowed(&rp_id, &host) {
            return Err(Error::TwoStep(
                "The server asked for a security key for another website, which PassBundle won't sign for.",
            ));
        }
        let mut allow = Vec::new();
        for credential in options["allowCredentials"].as_array().into_iter().flatten() {
            if credential["type"]
                .as_str()
                .is_some_and(|t| t != "public-key")
            {
                continue;
            }
            let id = credential["id"]
                .as_str()
                .and_then(base64_either)
                .filter(|id| !id.is_empty() && id.len() <= 1023)
                .ok_or_else(|| malformed("a credential ID that isn't one"))?;
            allow.push(id);
        }
        if allow.is_empty() {
            return Err(Error::TwoStep(
                "This account has no security key the server could name. Use another way to sign in.",
            ));
        }
        // The AppID only on the server's own origin (where PassBundle's
        // server puts it); anything else is left out.
        let app_id = options["extensions"]["appid"]
            .as_str()
            .filter(|a| origin_of(a).is_some_and(|(o, _)| o == origin))
            .map(str::to_owned);
        let timeout = options["timeout"]
            .as_u64()
            .map(|ms| Duration::from_millis(ms.clamp(15_000, 180_000)))
            .unwrap_or(Duration::from_secs(60));
        let user_verification = match options["userVerification"].as_str() {
            Some(uv @ ("required" | "discouraged")) => uv.to_owned(),
            _ => "preferred".to_owned(),
        };
        let client_data_json = client_data(&challenge, &origin);
        Ok(SecurityKeyRequest {
            origin,
            rp_id,
            app_id,
            allow,
            timeout,
            user_verification,
            client_data_json,
        })
    }

    /// What a key is asked.
    pub fn key_request(&self) -> KeyRequest {
        KeyRequest {
            rp_id: self.rp_id.clone(),
            app_id: self.app_id.clone(),
            client_data_hash: Sha256::digest(&self.client_data_json).into(),
            allow: self.allow.clone(),
        }
    }

    /// The key's answer as the server reads it (two-step login's token for
    /// provider 7), once it's checked: one of the account's credentials,
    /// signed for this site ID (or the AppID, when that's what it used), with
    /// the person there.
    pub fn token(&self, answer: &KeyAnswer) -> Result<String> {
        let wrong =
            || Error::TwoStep("The security key's answer wasn't for this sign-in. Try again.");
        if !self.allow.contains(&answer.credential_id) {
            return Err(wrong());
        }
        let signed_for = match (&self.app_id, answer.used_app_id) {
            (Some(app_id), true) => app_id.as_str(),
            (None, true) => return Err(wrong()),
            (_, false) => self.rp_id.as_str(),
        };
        let data = &answer.authenticator_data;
        let rp_id_hash: [u8; 32] = Sha256::digest(signed_for.as_bytes()).into();
        if data.len() < 37
            || data[..32] != rp_id_hash
            || data[32] & 0x01 == 0
            || answer.signature.is_empty()
        {
            return Err(wrong());
        }
        let id = URL_SAFE_NO_PAD.encode(&answer.credential_id);
        let extensions = if self.app_id.is_some() {
            json!({ "appid": answer.used_app_id })
        } else {
            json!({})
        };
        Ok(json!({
            "id": id,
            "rawId": id,
            "type": "public-key",
            "extensions": extensions,
            "response": {
                "authenticatorData": URL_SAFE_NO_PAD.encode(data),
                "clientDataJson": URL_SAFE_NO_PAD.encode(&self.client_data_json),
                "signature": URL_SAFE_NO_PAD.encode(&answer.signature),
                "userHandle": answer.user_handle.as_ref().map(|h| URL_SAFE_NO_PAD.encode(h)),
            },
        })
        .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What PassBundle's server sends (webauthn-rs's request options, with
    /// its AppID), for vault.example.test.
    fn options() -> Value {
        json!({
            "challenge": "dGhlIHNlcnZlcidzIGNoYWxsZW5nZSE",
            "timeout": 60000,
            "rpId": "vault.example.test",
            "allowCredentials": [
                { "type": "public-key", "id": "AQIDBA" },
                { "type": "public-key", "id": "BQYHCA", "transports": ["usb", "nfc"] },
            ],
            "userVerification": "discouraged",
            "extensions": { "appid": "https://vault.example.test/app-id.json", "uvm": null },
        })
    }

    #[test]
    fn origins_are_the_servers_scheme_host_and_port() {
        for (url, origin) in [
            ("https://vault.example.test", "https://vault.example.test"),
            (
                "https://Vault.Example.Test:443/",
                "https://vault.example.test",
            ),
            (
                "https://example.test:8443/vault",
                "https://example.test:8443",
            ),
            ("http://localhost:8593", "http://localhost:8593"),
            ("http://127.0.0.1:80", "http://127.0.0.1"),
        ] {
            assert_eq!(
                origin_of(url).map(|o| o.0).as_deref(),
                Some(origin),
                "{url}"
            );
        }
        for url in [
            "http://vault.example.test",
            "ftp://vault.example.test",
            "https://ana@vault.example.test",
            "https://",
            "https://vault.example.test:x",
            "vault.example.test",
        ] {
            assert_eq!(origin_of(url), None, "{url}");
        }
    }

    #[test]
    fn a_site_id_is_the_host_or_a_domain_above_it_but_never_a_public_suffix() {
        assert!(rp_id_allowed("vault.example.test", "vault.example.test"));
        assert!(rp_id_allowed("example.com", "vault.example.com"));
        assert!(rp_id_allowed("localhost", "localhost"));
        assert!(!rp_id_allowed("com", "vault.example.com"));
        assert!(!rp_id_allowed("co.uk", "vault.example.co.uk"));
        assert!(!rp_id_allowed("ample.com", "vault.example.com"));
        assert!(!rp_id_allowed("accounts.google.com", "vault.example.com"));
        assert!(!rp_id_allowed("vault.example.com", "example.com"));
        assert!(!rp_id_allowed("127.0.0.1", "127.0.0.1"));
        assert!(!rp_id_allowed("", "vault.example.com"));
    }

    #[test]
    fn reads_the_servers_request_and_makes_client_data_for_its_origin() {
        let request = SecurityKeyRequest::new(&options(), "https://vault.example.test/").unwrap();
        assert_eq!(request.origin, "https://vault.example.test");
        assert_eq!(request.rp_id, "vault.example.test");
        assert_eq!(request.allow, vec![vec![1, 2, 3, 4], vec![5, 6, 7, 8]]);
        assert_eq!(
            request.app_id.as_deref(),
            Some("https://vault.example.test/app-id.json")
        );
        assert_eq!(request.timeout, Duration::from_secs(60));
        assert_eq!(request.user_verification, "discouraged");
        // Byte for byte what a browser sends, so the server's checks hold.
        assert_eq!(
            String::from_utf8(request.client_data_json.clone()).unwrap(),
            r#"{"type":"webauthn.get","challenge":"dGhlIHNlcnZlcidzIGNoYWxsZW5nZSE","origin":"https://vault.example.test","crossOrigin":false}"#
        );
        assert_eq!(
            request.key_request().client_data_hash,
            <[u8; 32]>::from(Sha256::digest(&request.client_data_json))
        );
        // Upstream's shape, `publicKey` around the same.
        let wrapped = SecurityKeyRequest::new(
            &json!({ "publicKey": options() }),
            "https://vault.example.test",
        )
        .unwrap();
        assert_eq!(wrapped.client_data_json, request.client_data_json);
    }

    #[test]
    fn refuses_a_request_for_another_site_and_drops_another_sites_app_id() {
        let mut other = options();
        other["rpId"] = json!("accounts.example.org");
        assert!(matches!(
            SecurityKeyRequest::new(&other, "https://vault.example.test"),
            Err(Error::TwoStep(_))
        ));
        // A server on an address: WebAuthn takes only domains.
        assert!(SecurityKeyRequest::new(&options(), "https://192.0.2.10").is_err());
        assert!(SecurityKeyRequest::new(&options(), "http://vault.example.test").is_err());
        let mut app_id = options();
        app_id["extensions"]["appid"] = json!("https://evil.example.org/app-id.json");
        let request = SecurityKeyRequest::new(&app_id, "https://vault.example.test").unwrap();
        assert_eq!(request.app_id, None);
        // No credentials, or one that isn't base64: no request.
        let mut none = options();
        none["allowCredentials"] = json!([]);
        assert!(SecurityKeyRequest::new(&none, "https://vault.example.test").is_err());
        let mut bad = options();
        bad["allowCredentials"][0]["id"] = json!("not base64!");
        assert!(SecurityKeyRequest::new(&bad, "https://vault.example.test").is_err());
        let mut short = options();
        short["challenge"] = json!("AAEC");
        assert!(SecurityKeyRequest::new(&short, "https://vault.example.test").is_err());
    }

    fn answer(signed_for: &str, used_app_id: bool) -> KeyAnswer {
        let mut data = Sha256::digest(signed_for.as_bytes()).to_vec();
        data.extend([0x01, 0, 0, 0, 7]);
        KeyAnswer {
            credential_id: vec![5, 6, 7, 8],
            authenticator_data: data,
            signature: vec![0x30, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x01],
            user_handle: None,
            used_app_id,
        }
    }

    #[test]
    fn the_token_is_the_answer_in_the_servers_shape() {
        let request = SecurityKeyRequest::new(&options(), "https://vault.example.test").unwrap();
        let token: Value =
            serde_json::from_str(&request.token(&answer("vault.example.test", false)).unwrap())
                .unwrap();
        assert_eq!(token["id"], "BQYHCA");
        assert_eq!(token["rawId"], "BQYHCA");
        assert_eq!(token["type"], "public-key");
        assert_eq!(token["extensions"], json!({ "appid": false }));
        assert_eq!(
            token["response"]["clientDataJson"],
            URL_SAFE_NO_PAD.encode(&request.client_data_json)
        );
        assert_eq!(token["response"]["signature"], "MAYCAQECAQE");
        assert_eq!(token["response"]["userHandle"], Value::Null);
        let app = request
            .token(&answer("https://vault.example.test/app-id.json", true))
            .unwrap();
        assert!(app.contains(r#""appid":true"#));
    }

    #[test]
    fn an_answer_for_something_else_is_refused() {
        let request = SecurityKeyRequest::new(&options(), "https://vault.example.test").unwrap();
        // Another site's hash, the AppID's hash without saying so, a
        // credential the account doesn't have, and no one there.
        assert!(request.token(&answer("other.example.test", false)).is_err());
        assert!(request
            .token(&answer("https://vault.example.test/app-id.json", false))
            .is_err());
        let mut stranger = answer("vault.example.test", false);
        stranger.credential_id = vec![9];
        assert!(request.token(&stranger).is_err());
        let mut absent = answer("vault.example.test", false);
        absent.authenticator_data[32] = 0;
        assert!(request.token(&absent).is_err());
        let mut no_app_id = options();
        no_app_id["extensions"] = json!({});
        let request = SecurityKeyRequest::new(&no_app_id, "https://vault.example.test").unwrap();
        assert!(request
            .token(&answer("https://vault.example.test/app-id.json", true))
            .is_err());
    }
}
