//! Approving a sign-in from another device (DESIGN.md in the meta repo,
//! "Approving a sign-in from another device", and docs/core.md, "Approving a
//! sign-in"): the web vault or the browser extension asks to sign in "with a
//! device", and the person approves it here, while this app is unlocked.
//!
//! The protocol is upstream's, from its GPL clients: the requesting device
//! makes an RSA-2048 key pair and an access code and sends the public key to
//! the server; an approving device wraps the user key to that public key
//! with RSA-OAEP (SHA-1) and sends the wrap; the requesting device opens it
//! with its private key and signs in with the access code. Both devices show
//! the same fingerprint phrase, made from the account's email and the
//! request's public key, so the person can tell the request they're
//! approving is the one on the other screen, not one the server, or anyone
//! else, swapped in.
//!
//! The user key goes only to the public key whose phrase the person saw
//! (`App::answer_sign_in_request` checks the phrase it's given against the
//! request it kept), and never anywhere else.

use base64::{engine::general_purpose::STANDARD, Engine};
use hkdf::Hkdf;
use rsa::{pkcs8::DecodePublicKey, traits::PublicKeyParts, BigUint, RsaPublicKey};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::crypto::{encrypt_for, EncString, SymmetricKey};
use crate::error::{Error, Result};

/// How long a request lasts from when it's made, as the server and
/// upstream's clients have it: 15 minutes.
pub const LIFETIME_SECONDS: u64 = 15 * 60;

/// How many words the phrase has: as many as 64 bits need from EFF's list
/// of 7,776 (12.9 bits each), as upstream's clients make it.
const PHRASE_WORDS: usize = 5;

/// A sign-in waiting for this account's answer, as the person sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignInRequest {
    pub id: String,
    /// The requesting device's identifier, so the app prompts once per
    /// device, and shows a request it sent again ("Resend") in its place.
    /// Empty from a server that doesn't say.
    pub device_id: String,
    /// Its type, as the server names it ("Chrome"); with the identifier and
    /// the address, what says a request sent again is the same device's.
    pub device_type: String,
    /// The requesting device, in words ("Web vault in Chrome").
    pub device: String,
    /// Its address, as the server saw it.
    pub ip: String,
    /// When it was made: seconds since 1970, by the server's clock.
    pub created: u64,
    /// How old it was, and how long it had left, when the server listed it
    /// (seconds, by the server's clock, so this device's clock doesn't matter).
    pub age: u64,
    pub left: u64,
    /// The phrase both devices show.
    pub fingerprint: String,
    /// The request's public key (SubjectPublicKeyInfo, DER), which the user
    /// key is wrapped to: kept, so an approval goes to the key whose phrase
    /// the person saw.
    public_key: Vec<u8>,
}

impl SignInRequest {
    /// The public key it was made with (DER), as the server first sent it.
    pub(crate) fn public_key(&self) -> &[u8] {
        &self.public_key
    }

    /// As it stands `elapsed` seconds after it was listed: older, with less
    /// left, or None once its time is up.
    pub fn after(&self, elapsed: u64) -> Option<SignInRequest> {
        let left = self.left.checked_sub(elapsed).filter(|left| *left > 0)?;
        Some(SignInRequest {
            age: self.age + elapsed,
            left,
            ..self.clone()
        })
    }
}

/// The fingerprint phrase for an account's email and a request's public key
/// (DER), as upstream's clients make it: SHA-256 of the key, expanded with
/// HKDF-SHA256 under the email in lower case to 32 bytes, read as one
/// big-endian number, then written in base 7,776 from its lowest digit up,
/// one word of EFF's long list per digit, joined with hyphens.
pub fn fingerprint_phrase(email: &str, public_key: &[u8]) -> String {
    let key_hash = Sha256::digest(public_key);
    let hkdf = Hkdf::<Sha256>::from_prk(&key_hash).expect("a SHA-256 hash is a PRK");
    let mut number = [0u8; 32];
    hkdf.expand(email.to_lowercase().as_bytes(), &mut number)
        .expect("32 bytes is a valid HKDF length");
    let words = crate::generator::words();
    let base = words.len() as u32;
    let mut phrase = Vec::with_capacity(PHRASE_WORDS);
    for _ in 0..PHRASE_WORDS {
        // Divide the big-endian number by the base in place; the remainder
        // is the next word.
        let mut remainder = 0u32;
        for byte in number.iter_mut() {
            let value = (remainder << 8) | u32::from(*byte);
            *byte = (value / base) as u8;
            remainder = value % base;
        }
        phrase.push(words[remainder as usize]);
    }
    phrase.join("-")
}

/// The request's public key, if the user key may be wrapped to it: RSA-2048
/// with the exponent 65537, what upstream's clients make. Anything else is
/// refused, with nothing sent.
fn checked_public_key(der: &[u8]) -> Result<RsaPublicKey> {
    let key = RsaPublicKey::from_public_key_der(der)
        .map_err(|_| Error::Refused("the sign-in request's key isn't an RSA public key"))?;
    if key.size() != 256 {
        return Err(Error::Refused("the sign-in request's key isn't RSA-2048"));
    }
    if *key.e() != BigUint::from(65_537u32) {
        return Err(Error::Refused(
            "the sign-in request's key has an unusual exponent",
        ));
    }
    Ok(key)
}

/// The user key, wrapped to the request's public key (DER) with RSA-OAEP
/// and SHA-1, as a type 4 string: what the approving device sends.
pub fn wrap_user_key(public_key: &[u8], user_key: &SymmetricKey) -> Result<EncString> {
    checked_public_key(public_key)?;
    encrypt_for(public_key, &user_key.to_bytes())
}

/// A date the server writes (RFC 3339 in UTC, `2026-10-26T12:00:00.123456Z`),
/// as seconds since 1970.
fn parse_date(text: &str) -> Option<u64> {
    let text = text.strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut d = date.splitn(3, '-').map(|p| p.parse::<i64>().ok());
    let (year, month, day) = (d.next()??, d.next()??, d.next()??);
    let time = time.split('.').next()?;
    let mut t = time.splitn(3, ':').map(|p| p.parse::<i64>().ok());
    let (hour, minute, second) = (t.next()??, t.next()??, t.next()??);
    seconds_since_1970(year, month, day, hour, minute, second)
}

/// An HTTP `Date` (`Mon, 26 Oct 2026 12:00:00 GMT`), as seconds since 1970.
pub fn parse_http_date(text: &str) -> Option<u64> {
    let mut parts = text.split_whitespace().skip(1);
    let (day, month, year, time, zone) = (
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
    );
    if zone != "GMT" || parts.next().is_some() {
        return None;
    }
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month = MONTHS.iter().position(|m| *m == month)? as i64 + 1;
    let mut t = time.splitn(3, ':').map(|p| p.parse::<i64>().ok());
    let (hour, minute, second) = (t.next()??, t.next()??, t.next()??);
    seconds_since_1970(
        year.parse().ok()?,
        month,
        day.parse().ok()?,
        hour,
        minute,
        second,
    )
}

fn seconds_since_1970(
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
) -> Option<u64> {
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=60).contains(&second)
    {
        return None;
    }
    // Days since 1970 for a date in the proleptic Gregorian calendar
    // (Howard Hinnant's days_from_civil).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + hour * 3_600 + minute * 60 + second).ok()
}

/// The requesting device in words, from the server's name for its type.
fn device_words(kind: &str) -> String {
    match kind {
        "Chrome" | "Firefox" | "Opera" | "Edge" | "Safari" | "Vivaldi" | "DuckDuckGo" => {
            format!("Web vault in {kind}")
        }
        "Internet Explorer" | "Unknown Browser" => "Web vault in a browser".to_owned(),
        _ if kind.ends_with(" Extension") => {
            format!(
                "Browser extension in {}",
                kind.trim_end_matches(" Extension")
            )
        }
        "Windows" | "macOS" | "Linux" => format!("App on {kind}"),
        "Android" | "iOS" => format!("App on {kind}"),
        _ if kind.ends_with(" CLI") => {
            format!("Command line on {}", kind.trim_end_matches(" CLI"))
        }
        "" => "A device".to_owned(),
        _ => kind.to_owned(),
    }
}

/// One request from the server's answer, for this account's email, at
/// `server_now` (the server's clock): None for one the server marks
/// answered, one that's expired, or one whose key can't be wrapped to (it
/// isn't what upstream's clients make).
pub fn parse_request(value: &Value, email: &str, server_now: u64) -> Option<SignInRequest> {
    if value["requestApproved"].as_bool().is_some() || !value["responseDate"].is_null() {
        return None;
    }
    let id = value["id"].as_str().filter(|id| !id.is_empty())?.to_owned();
    let public_key = STANDARD.decode(value["publicKey"].as_str()?).ok()?;
    checked_public_key(&public_key).ok()?;
    let created = parse_date(value["creationDate"].as_str()?)?;
    let age = server_now.saturating_sub(created);
    let left = LIFETIME_SECONDS.checked_sub(age).filter(|left| *left > 0)?;
    Some(SignInRequest {
        fingerprint: fingerprint_phrase(email, &public_key),
        id,
        device_id: value["requestDeviceIdentifier"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        device_type: value["requestDeviceType"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        device: device_words(value["requestDeviceType"].as_str().unwrap_or_default()),
        ip: value["requestIpAddress"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        created,
        age,
        left,
        public_key,
    })
}

/// The requests in the server's pending list (`GET /api/auth-requests/pending`)
/// still waiting at `server_now`, newest first.
pub fn parse_pending(body: &Value, email: &str, server_now: u64) -> Vec<SignInRequest> {
    let mut requests: Vec<SignInRequest> = body["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| parse_request(r, email, server_now))
        .collect();
    requests.sort_by_key(|r| std::cmp::Reverse(r.created));
    requests
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::PrivateKey;
    use serde_json::json;

    fn b64(v: &Value) -> Vec<u8> {
        STANDARD.decode(v.as_str().unwrap()).unwrap()
    }

    /// docs/vectors/device-approval-v1.json in the meta repo, which plain
    /// Node made and upstream's own code checks (the phrase in the clients'
    /// unit tests, the wrap with the SDK's PureCrypto).
    #[test]
    fn makes_the_vectors_fingerprint_phrases() {
        let v = vectors();
        for f in v["fingerprints"].as_array().unwrap() {
            let phrase = fingerprint_phrase(f["email"].as_str().unwrap(), &b64(&f["publicKey"]));
            assert_eq!(phrase, f["phrase"].as_str().unwrap(), "{}", f["why"]);
        }
    }

    #[test]
    fn the_vectors_wrap_and_ours_open_to_the_user_key() {
        let v = vectors();
        let private = PrivateKey::from_pkcs8_der(&b64(&v["request"]["privateKey"])).unwrap();
        let user_key = SymmetricKey::from_bytes(&b64(&v["approval"]["userKey"])).unwrap();
        let theirs = EncString::parse(v["approval"]["key"].as_str().unwrap()).unwrap();
        assert_eq!(&*private.decrypt(&theirs).unwrap(), &*user_key.to_bytes());

        // Ours: type 4, a new seed each time, and it opens the same way.
        let public = b64(&v["request"]["publicKey"]);
        let ours = wrap_user_key(&public, &user_key).unwrap();
        assert!(ours.to_string().starts_with("4."));
        assert_ne!(ours, wrap_user_key(&public, &user_key).unwrap());
        assert_eq!(&*private.decrypt(&ours).unwrap(), &*user_key.to_bytes());
    }

    #[test]
    fn refuses_keys_upstream_doesnt_make() {
        let v = vectors();
        let user_key = SymmetricKey::generate();
        for r in v["refusedPublicKeys"].as_array().unwrap() {
            let text = r["publicKey"].as_str().unwrap();
            let wrapped = STANDARD
                .decode(text)
                .map_err(|_| Error::Refused("not base64"))
                .and_then(|der| wrap_user_key(&der, &user_key));
            assert!(wrapped.is_err(), "{}", r["why"]);
            let mut request = request_at(&v, "2026-10-26T12:00:00Z");
            request["publicKey"] = json!(text);
            assert_eq!(
                parse_request(&request, "dana@example.test", 0),
                None,
                "{}",
                r["why"]
            );
        }
    }

    #[test]
    fn reads_the_dates_the_server_writes() {
        assert_eq!(parse_date("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_date("2026-10-26T12:00:00.123456Z"),
            Some(1_793_016_000)
        );
        assert_eq!(
            parse_http_date("Mon, 26 Oct 2026 12:00:00 GMT"),
            Some(1_793_016_000)
        );
        assert_eq!(parse_http_date("Mon, 26 Oct 2026 12:00:00 PST"), None);
        assert_eq!(parse_date("2000-02-29T23:59:59Z"), Some(951_868_799));
        for bad in [
            "2026-10-26T12:00:00",
            "2026-13-01T00:00:00Z",
            "yesterday",
            "",
        ] {
            assert_eq!(parse_date(bad), None, "{bad}");
        }
    }

    #[test]
    fn names_the_requesting_device() {
        assert_eq!(device_words("Chrome"), "Web vault in Chrome");
        assert_eq!(
            device_words("Firefox Extension"),
            "Browser extension in Firefox"
        );
        assert_eq!(device_words("Windows"), "App on Windows");
        assert_eq!(device_words("macOS CLI"), "Command line on macOS");
        assert_eq!(device_words("Unknown Browser"), "Web vault in a browser");
    }

    fn vectors() -> Value {
        serde_json::from_str(include_str!("../tests/vectors/device-approval-v1.json")).unwrap()
    }

    fn request_at(v: &Value, created: &str) -> Value {
        json!({
            "id": "6f1c2b0e-0000-4000-8000-000000000001",
            "requestDeviceIdentifier": "2f9e3c1a-0000-4000-8000-0000000000aa",
            "publicKey": v["request"]["publicKey"],
            "requestDeviceType": "Chrome",
            "requestIpAddress": "203.0.113.7",
            "creationDate": created,
            "responseDate": null,
            "requestApproved": null,
        })
    }

    #[test]
    fn a_request_shows_its_phrase_and_goes_after_fifteen_minutes() {
        let v = vectors();
        let made = parse_date("2026-10-26T12:00:00.000000Z").unwrap();
        let body = json!({ "data": [request_at(&v, "2026-10-26T12:00:00.000000Z")] });
        let [request] = parse_pending(&body, "dana@example.test", made + 60)
            .try_into()
            .unwrap();
        assert_eq!(
            request.fingerprint,
            v["fingerprints"][0]["phrase"].as_str().unwrap()
        );
        assert_eq!(request.device, "Web vault in Chrome");
        assert_eq!(request.device_id, "2f9e3c1a-0000-4000-8000-0000000000aa");
        assert_eq!(request.device_type, "Chrome");
        assert_eq!(request.ip, "203.0.113.7");
        assert_eq!((request.age, request.left), (60, 14 * 60));
        assert_eq!(
            parse_pending(&body, "dana@example.test", made + 14 * 60 + 59).len(),
            1
        );
        assert!(parse_pending(&body, "dana@example.test", made + 15 * 60).is_empty());
    }

    #[test]
    fn answered_and_unusable_requests_are_left_out() {
        let v = vectors();
        let made = "2026-10-26T12:00:00.000000Z";
        let now = parse_date(made).unwrap();
        let mut approved = request_at(&v, made);
        approved["requestApproved"] = json!(true);
        approved["responseDate"] = json!(made);
        let mut small_key = request_at(&v, made);
        small_key["publicKey"] = v["refusedPublicKeys"][0]["publicKey"].clone();
        let mut no_id = request_at(&v, made);
        no_id["id"] = json!("");
        let body = json!({ "data": [approved, small_key, no_id] });
        assert!(parse_pending(&body, "dana@example.test", now).is_empty());
    }
}
