//! Send (DESIGN.md, "Send in the apps"; docs/core.md, "Send"): a text or a
//! file, encrypted on this device, that someone without an account opens
//! from a link until a date or a number of views. Upstream's format, so the
//! web vault's access page opens a Send made here, and the Sends the web
//! vault and the extension make are listed and edited here.
//!
//! - **The seed:** 16 new bytes from the operating system's random source,
//!   one per Send. The server gets it only wrapped under the account's user
//!   key; it leaves the core unwrapped only inside the link.
//! - **The key:** HKDF-SHA256 of the seed, with the protocol's salt and info
//!   (upstream's constants), 64 bytes: 32 for AES-256-CBC, 32 for HMAC.
//! - **The fields:** type 2 EncStrings under that key, each with a new IV.
//! - **A password:** the server gets PBKDF2-HMAC-SHA256 of it, the seed as
//!   salt, 100,000 rounds, in base64, and never the password itself.

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use hkdf::Hkdf;
use rand::{rngs::OsRng, RngCore};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::crypto::{EncString, SymmetricKey};
use crate::error::{Error, Result};

/// The protocol's salt and info for a Send's key, as upstream has them.
const KEY_SALT: &[u8] = b"bitwarden-send";
const KEY_INFO: &[u8] = b"send";
/// How long a seed is.
pub const SEED_LEN: usize = 16;
/// PBKDF2's rounds for a Send's password (upstream's `SEND_KDF_ITERATIONS`).
pub const PASSWORD_ROUNDS: u32 = 100_000;
/// The latest deletion date the server takes, in days from now.
pub const MOST_DAYS: i64 = 31;
/// What a Send whose key or name doesn't open is called.
pub const CANT_OPEN_SEND: &str = "A Send that can't be opened";

/// A Send's seed, wiped when it's dropped.
pub type Seed = Zeroizing<[u8; SEED_LEN]>;

/// A new seed from the operating system's random source.
pub fn new_seed() -> Seed {
    let mut seed = Zeroizing::new([0u8; SEED_LEN]);
    OsRng.fill_bytes(&mut *seed);
    seed
}

/// The Send's key from its seed.
pub fn key_from_seed(seed: &[u8]) -> SymmetricKey {
    let hkdf = Hkdf::<Sha256>::new(Some(KEY_SALT), seed);
    let mut okm = Zeroizing::new([0u8; 64]);
    hkdf.expand(KEY_INFO, &mut *okm)
        .expect("64 bytes is a length HKDF-SHA256 makes");
    SymmetricKey::from_bytes(&*okm).expect("64 bytes is a key's length")
}

/// What the server checks a Send's password against: PBKDF2-HMAC-SHA256 of
/// the password, with the seed as its salt, in base64.
pub fn password_proof(password: &str, seed: &[u8]) -> String {
    let mut out = Zeroizing::new([0u8; 32]);
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), seed, PASSWORD_ROUNDS, &mut *out);
    STANDARD.encode(*out)
}

/// The link someone opens a Send with, on the web vault's access page:
/// `<server>/#/send/<access id>/<seed in base64url>`. The seed is in the
/// fragment, which browsers don't send to servers. The access id is the
/// server's, so it's checked to be one a link can carry as it is.
pub fn link(server: &str, access_id: &str, seed: &[u8]) -> Result<Zeroizing<String>> {
    let fits = !access_id.is_empty()
        && access_id.len() <= 64
        && access_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !fits {
        return Err(Error::Damaged("a Send's access id that isn't one"));
    }
    Ok(Zeroizing::new(format!(
        "{}/#/send/{access_id}/{}",
        server.trim_end_matches('/'),
        URL_SAFE_NO_PAD.encode(seed)
    )))
}

/// A Send as the server sends it, in a sync or an answer.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendData {
    pub id: String,
    #[serde(default)]
    pub access_id: String,
    #[serde(rename = "type")]
    pub kind: u8,
    pub name: Option<String>,
    pub notes: Option<String>,
    pub text: Option<SendTextData>,
    pub file: Option<SendFileData>,
    pub key: Option<String>,
    pub max_access_count: Option<Value>,
    pub access_count: Option<Value>,
    pub password: Option<String>,
    /// Upstream's: 0 asks for an email address, 1 for a password, 2 nothing.
    pub auth_type: Option<u8>,
    #[serde(default)]
    pub disabled: bool,
    pub hide_email: Option<bool>,
    pub revision_date: Option<String>,
    pub expiration_date: Option<String>,
    pub deletion_date: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SendTextData {
    pub text: Option<String>,
    #[serde(default)]
    pub hidden: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendFileData {
    pub id: Option<String>,
    pub file_name: Option<String>,
    /// The encrypted file's length, as a number or a string of one.
    pub size: Option<Value>,
}

/// A whole number the server sends as a number or a string of one.
fn number(value: &Option<Value>) -> Option<u64> {
    match value {
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::String(s)) => s.parse().ok(),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendKind {
    Text,
    File,
}

impl SendKind {
    pub fn word(self) -> &'static str {
        match self {
            SendKind::Text => "text",
            SendKind::File => "file",
        }
    }

    fn number(self) -> u8 {
        match self {
            SendKind::Text => 0,
            SendKind::File => 1,
        }
    }
}

/// A Send, opened with its account's user key.
#[derive(Clone)]
pub struct Send {
    pub id: String,
    pub access_id: String,
    pub kind: SendKind,
    /// Its name; `CANT_OPEN_SEND` when its key or name doesn't open.
    pub name: String,
    /// Whether its key and fields open: one that doesn't can only be
    /// deleted.
    pub opens: bool,
    pub notes: Option<Zeroizing<String>>,
    pub text: Option<Zeroizing<String>>,
    /// Whether the text starts hidden on the access page.
    pub hidden: bool,
    pub file_name: Option<String>,
    /// The encrypted file's length, as the server keeps it.
    pub file_size: Option<u64>,
    pub max_access_count: Option<u64>,
    pub access_count: u64,
    pub has_password: bool,
    /// One that asks whoever opens it for an email address (upstream's
    /// newer kind, which this server doesn't take): listed, deleted only.
    pub asks_for_email: bool,
    pub disabled: bool,
    pub hide_email: bool,
    pub revision_date: Option<String>,
    pub expiration_date: Option<String>,
    pub deletion_date: Option<String>,
    seed: Option<Seed>,
    key: Option<SymmetricKey>,
}

impl std::fmt::Debug for Send {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Send")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("opens", &self.opens)
            .finish_non_exhaustive()
    }
}

/// Opens one of a Send's fields under its key.
fn field(value: &Option<String>, key: &SymmetricKey) -> Result<Option<Zeroizing<String>>> {
    value
        .as_deref()
        .filter(|v| !v.is_empty())
        .map(|v| EncString::parse(v).and_then(|e| e.decrypt_string(key)))
        .transpose()
}

impl Send {
    /// Opens a Send under the user key that wraps its seed. One whose seed,
    /// name, or fields don't open is kept, as one that can't be opened.
    pub fn open(data: &SendData, user: &SymmetricKey) -> Send {
        let kind = if data.kind == 1 {
            SendKind::File
        } else {
            SendKind::Text
        };
        let mut send = Send {
            id: data.id.clone(),
            access_id: data.access_id.clone(),
            kind,
            name: CANT_OPEN_SEND.into(),
            opens: false,
            notes: None,
            text: None,
            hidden: data.text.as_ref().is_some_and(|t| t.hidden),
            file_name: None,
            file_size: data.file.as_ref().and_then(|f| number(&f.size)),
            max_access_count: number(&data.max_access_count),
            access_count: number(&data.access_count).unwrap_or(0),
            has_password: data.password.as_deref().is_some_and(|p| !p.is_empty())
                || data.auth_type == Some(1),
            asks_for_email: data.auth_type == Some(0),
            disabled: data.disabled,
            hide_email: data.hide_email == Some(true),
            revision_date: data.revision_date.clone(),
            expiration_date: data.expiration_date.clone(),
            deletion_date: data.deletion_date.clone(),
            seed: None,
            key: None,
        };
        let opened = (|| -> Result<_> {
            let wrapped = data
                .key
                .as_deref()
                .ok_or(Error::Damaged("a Send with no key"))?;
            let bytes = EncString::parse(wrapped)?.decrypt(user)?;
            let mut seed = Zeroizing::new([0u8; SEED_LEN]);
            if bytes.len() != SEED_LEN {
                return Err(Error::Damaged("a Send's key of the wrong length"));
            }
            seed.copy_from_slice(&bytes);
            let key = key_from_seed(&*seed);
            let name = field(&data.name, &key)?.ok_or(Error::Damaged("a Send with no name"))?;
            let notes = field(&data.notes, &key)?;
            let text = match &data.text {
                Some(t) => field(&t.text, &key)?,
                None => None,
            };
            let file_name = match &data.file {
                Some(f) => field(&f.file_name, &key)?.map(|n| n.as_str().to_owned()),
                None => None,
            };
            Ok((seed, key, name, notes, text, file_name))
        })();
        if let Ok((seed, key, name, notes, text, file_name)) = opened {
            send.name = name.as_str().to_owned();
            send.notes = notes;
            send.text = text;
            send.file_name = file_name;
            send.seed = Some(seed);
            send.key = Some(key);
            send.opens = true;
        }
        send
    }

    /// Its link, on `server`'s access page.
    pub fn link(&self, server: &str) -> Result<Zeroizing<String>> {
        let seed = self
            .seed
            .as_ref()
            .ok_or(Error::Refused("this Send doesn't open, so it has no link"))?;
        link(server, &self.access_id, &**seed)
    }

    /// Its seed, for an edit (which keeps it) or a check.
    pub(crate) fn seed(&self) -> Option<&Seed> {
        self.seed.as_ref()
    }

    /// Whether a Send's answer from the server is this seed's: its seed,
    /// wrapped under the user key, opens to the one sent.
    pub fn has_seed(&self, seed: &[u8]) -> bool {
        self.seed.as_ref().is_some_and(|s| s.as_slice() == seed)
    }

    /// Whether its expiration date has passed (`now` as RFC 3339, UTC, as
    /// the server writes dates).
    pub fn expired(&self, now: &str) -> bool {
        self.expiration_date.as_deref().is_some_and(|d| d < now)
    }

    /// Whether it's been opened as many times as it may be.
    pub fn used_up(&self) -> bool {
        self.max_access_count
            .is_some_and(|most| self.access_count >= most)
    }
}

/// A Send as the person edits it, before it's encrypted.
#[derive(Debug, Default)]
pub struct SendDraft {
    pub kind: Option<SendKind>,
    pub name: String,
    pub notes: Option<Zeroizing<String>>,
    pub text: Option<Zeroizing<String>>,
    pub hidden: bool,
    pub max_access_count: Option<u64>,
    /// RFC 3339 dates in UTC, as the server takes them.
    pub expiration_date: Option<String>,
    pub deletion_date: String,
    pub disabled: bool,
    pub hide_email: bool,
}

/// The date `seconds` from now, as the server takes it.
pub fn date_after(seconds: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    crate::vault::iso8601(now.as_secs() + seconds, 0)
}

/// Whether `date` is an RFC 3339 date and time in UTC, as the server takes
/// it: `2026-11-16T12:00:00Z`, with or without a fraction of a second.
pub fn is_date(date: &str) -> bool {
    let b = date.as_bytes();
    let digits =
        |r: std::ops::Range<usize>| b.get(r).is_some_and(|d| d.iter().all(u8::is_ascii_digit));
    let head = b.len() >= 20
        && digits(0..4)
        && b[4] == b'-'
        && digits(5..7)
        && b[7] == b'-'
        && digits(8..10)
        && b[10] == b'T'
        && digits(11..13)
        && b[13] == b':'
        && digits(14..16)
        && b[16] == b':'
        && digits(17..19);
    if !head || b.last() != Some(&b'Z') {
        return false;
    }
    let rest = &b[19..b.len() - 1];
    rest.is_empty()
        || (rest[0] == b'.' && rest.len() > 1 && rest[1..].iter().all(u8::is_ascii_digit))
}

impl SendDraft {
    /// What the server would refuse anyway, said before anything is sent.
    pub fn check(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(Error::Refused("a Send needs a name"));
        }
        if self.kind == Some(SendKind::Text) && self.text.as_deref().is_none_or(|t| t.is_empty()) {
            return Err(Error::Refused("a text Send needs its text"));
        }
        if !is_date(&self.deletion_date) {
            return Err(Error::Refused("a Send needs a date it's deleted on"));
        }
        if self.expiration_date.as_deref().is_some_and(|d| !is_date(d)) {
            return Err(Error::Damaged("an expiration date that isn't one"));
        }
        if self.max_access_count == Some(0)
            || self.max_access_count.is_some_and(|n| n > i32::MAX as u64)
        {
            return Err(Error::Refused(
                "the most times a Send can be opened is a whole number, 1 or more",
            ));
        }
        Ok(())
    }

    /// The body the server takes for a Send of `kind`: every field encrypted
    /// under the Send's key (from `seed`), each with a new IV; the seed
    /// wrapped under the account's user key, with a new IV; the password's
    /// proof, or null to leave the password as it is. A file Send's file
    /// name and the encrypted file's length come in `file`.
    pub fn body(
        &self,
        kind: SendKind,
        seed: &[u8],
        user: &SymmetricKey,
        proof: Option<String>,
        file: Option<(&str, u64)>,
    ) -> Value {
        let key = key_from_seed(seed);
        let encrypt = |text: &str| key.encrypt_str(text).to_string();
        let mut body = json!({
            "type": kind.number(),
            "name": encrypt(&self.name),
            "notes": self.notes.as_deref().filter(|n| !n.is_empty()).map(|n| encrypt(n)),
            "key": user.encrypt(seed).to_string(),
            "maxAccessCount": self.max_access_count,
            "expirationDate": self.expiration_date,
            "deletionDate": self.deletion_date,
            "password": proof,
            "emails": null,
            "disabled": self.disabled,
            "hideEmail": self.hide_email,
        });
        match kind {
            SendKind::Text => {
                body["text"] = json!({
                    "text": self.text.as_deref().map(|t| encrypt(t)),
                    "hidden": self.hidden,
                });
            }
            SendKind::File => {
                if let Some((name, length)) = file {
                    body["file"] = json!({ "fileName": encrypt(name) });
                    body["fileLength"] = json!(length);
                }
            }
        }
        body
    }
}

/// The Sends in a sync, each opened with the user key.
pub fn open_all(sends: &[SendData], user: &SymmetricKey) -> Vec<Send> {
    sends.iter().map(|s| Send::open(s, user)).collect()
}

/// The organizations' Send policies (upstream's types 6, "Disable Send",
/// and 7, "Send options"), as the server enforces them: they bind members
/// of an organization that turned them on, and not its owners or admins.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SendPolicy {
    /// No new Sends and no edits; deleting is still allowed.
    pub disabled: bool,
    /// Sends can't hide the sender's email (an existing one can stop).
    pub no_hiding_email: bool,
}

const DISABLE_SEND: u64 = 6;
const SEND_OPTIONS: u64 = 7;

impl SendPolicy {
    /// From a sync (the kept `sync.json`): its `policies`, and the person's
    /// role in each organization (`profile.organizations`).
    pub fn from_sync(text: &str) -> SendPolicy {
        let mut policy = SendPolicy::default();
        let Ok(sync) = serde_json::from_str::<Value>(text) else {
            return policy;
        };
        let organizations = sync["profile"]["organizations"].as_array();
        // Owners (0) and admins (1) aren't bound, as on the server.
        let exempt = |org: &Value| {
            organizations
                .into_iter()
                .flatten()
                .any(|o| o["id"] == *org && matches!(o["type"].as_u64(), Some(0 | 1)))
        };
        for p in sync["policies"].as_array().into_iter().flatten() {
            if p["enabled"] != true || exempt(&p["organizationId"]) {
                continue;
            }
            match p["type"].as_u64() {
                Some(DISABLE_SEND) => policy.disabled = true,
                Some(SEND_OPTIONS) if p["data"]["disableHideEmail"] == true => {
                    policy.no_hiding_email = true;
                }
                _ => {}
            }
        }
        policy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_is_rfc_3339_in_utc() {
        assert!(is_date("2026-11-16T12:00:00Z"));
        assert!(is_date("2026-11-16T12:00:00.123Z"));
        for not in [
            "",
            "2026-11-16",
            "2026-11-16T12:00:00",
            "2026-11-16T12:00:00+01:00",
            "2026-11-16T12:00:00.Z",
            "2026-11-16 12:00:00Z",
            "20x6-11-16T12:00:00Z",
        ] {
            assert!(!is_date(not), "{not}");
        }
    }

    #[test]
    fn a_link_carries_only_an_access_id_that_fits() {
        let seed = [7u8; 16];
        assert!(link("https://vault.example.test", "ESIzRFVm-_A", &seed).is_ok());
        for bad in ["", "a/b", "a#b", "a b", &"a".repeat(65)] {
            assert!(
                link("https://vault.example.test", bad, &seed).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn each_seed_is_new() {
        assert_ne!(*new_seed(), *new_seed());
    }

    #[test]
    fn the_policies_bind_members_and_not_owners_or_admins() {
        let sync = |role: u64, policies: Value| {
            json!({
                "profile": { "organizations": [{ "id": "o", "type": role }] },
                "policies": policies,
            })
            .to_string()
        };
        let both = json!([
            { "organizationId": "o", "type": 6, "enabled": true },
            { "organizationId": "o", "type": 7, "enabled": true, "data": { "disableHideEmail": true } },
        ]);
        assert_eq!(
            SendPolicy::from_sync(&sync(2, both.clone())),
            SendPolicy {
                disabled: true,
                no_hiding_email: true
            }
        );
        for role in [0, 1] {
            assert_eq!(
                SendPolicy::from_sync(&sync(role, both.clone())),
                SendPolicy::default()
            );
        }
        // Off, or Send options without the switch, binds no one.
        let off = json!([
            { "organizationId": "o", "type": 6, "enabled": false },
            { "organizationId": "o", "type": 7, "enabled": true, "data": { "disableHideEmail": false } },
        ]);
        assert_eq!(SendPolicy::from_sync(&sync(2, off)), SendPolicy::default());
    }

    #[test]
    fn a_draft_is_checked_before_anything_is_sent() {
        let good = SendDraft {
            kind: Some(SendKind::Text),
            name: "Door code".into(),
            text: Some(Zeroizing::new("4711".into())),
            deletion_date: "2026-11-16T12:00:00Z".into(),
            ..Default::default()
        };
        assert!(good.check().is_ok());
        let without = |change: fn(&mut SendDraft)| {
            let mut d = SendDraft {
                kind: good.kind,
                name: good.name.clone(),
                text: good.text.clone(),
                deletion_date: good.deletion_date.clone(),
                ..Default::default()
            };
            change(&mut d);
            d.check()
        };
        assert!(without(|d| d.name = " ".into()).is_err());
        assert!(without(|d| d.text = None).is_err());
        assert!(without(|d| d.deletion_date = "soon".into()).is_err());
        assert!(without(|d| d.max_access_count = Some(0)).is_err());
        assert!(without(|d| d.expiration_date = Some("later".into())).is_err());
        assert!(without(|d| d.max_access_count = Some(3)).is_ok());
    }

    #[test]
    fn a_send_opens_with_its_accounts_key_only() {
        let user = SymmetricKey::generate();
        let seed = new_seed();
        let draft = SendDraft {
            kind: Some(SendKind::Text),
            name: "Door code".into(),
            notes: Some(Zeroizing::new("For Malia".into())),
            text: Some(Zeroizing::new("4711".into())),
            hidden: true,
            deletion_date: "2026-11-16T12:00:00Z".into(),
            ..Default::default()
        };
        let mut body = draft.body(SendKind::Text, &*seed, &user, None, None);
        body["id"] = json!("s");
        body["accessId"] = json!("abc");
        let data: SendData = serde_json::from_value(body).unwrap();
        let send = Send::open(&data, &user);
        assert!(send.opens);
        assert!(send.has_seed(&*seed));
        assert_eq!(send.name, "Door code");
        assert_eq!(send.text.as_deref().map(String::as_str), Some("4711"));
        assert_eq!(send.notes.as_deref().map(String::as_str), Some("For Malia"));
        assert!(send.hidden);
        let other = Send::open(&data, &SymmetricKey::generate());
        assert!(!other.opens);
        assert_eq!(other.name, CANT_OPEN_SEND);
        assert!(other.text.is_none());
        assert!(other.link("https://vault.example.test").is_err());
    }
}
