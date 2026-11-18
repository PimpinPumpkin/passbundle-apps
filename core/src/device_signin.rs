//! Signing in with a device, the asking side (DESIGN.md in the meta repo,
//! "Approving a sign-in from another device", "Our apps asking"; docs/core.md,
//! "Signing in with a device"): on a device that has signed in to the account
//! before, the person picks "Sign in with a device" instead of typing the
//! master password, and approves it from another of their devices.
//!
//! The protocol is upstream's, from its GPL clients, as the approving side
//! (device_approval.rs) has it: this device makes an RSA-2048 key pair and an
//! access code and sends the public key to the server; the approving device
//! wraps the user key to it (RSA-OAEP, SHA-1); this device opens the wrap with
//! its private key, and signs in with the password grant, the access code as
//! the password. Both devices show the same fingerprint phrase, so the person
//! can tell the approving device has this device's key, not one the server
//! swapped in.
//!
//! The private key and the access code live here, in memory only, for as
//! long as the request does: whatever ends it (signing in, a denial, the 15
//! minutes, a key that doesn't work, a new request, giving up, locking) drops
//! them, and the private key's numbers are zeroed as it's dropped (the `rsa`
//! crate's key zeroes itself). The screens get the phrase, how long is left,
//! and how it stands, never the key or the code.

use std::time::{Duration, Instant, SystemTime};

use base64::{engine::general_purpose::STANDARD, Engine};
use rand::{rngs::OsRng, Rng};
use serde_json::Value;
use zeroize::Zeroizing;

use crate::crypto::{EncString, PrivateKey, SymmetricKey};
use crate::device_approval::{fingerprint_phrase, LIFETIME_SECONDS};
use crate::error::{Error, Result};
use crate::kdf::normalize_email;
use crate::server::Server;

/// The access code's characters: upstream's generator's letters and digits
/// without look-alikes (no I, O, l, 0, or 1), none of which needs encoding
/// in the answer's address.
pub const ACCESS_CODE_ALPHABET: &[u8] =
    b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";

/// How many: upstream's 25, about 146 bits.
pub const ACCESS_CODE_LENGTH: usize = 25;

/// How long a request lasts, from when it was sent: the server's 15 minutes.
const LIFETIME: Duration = Duration::from_secs(LIFETIME_SECONDS);

/// How often the server is asked for the answer: every 5 seconds for the
/// first 2 minutes, when most approvals come, then every 10, so a request's
/// 15 minutes take about 100 calls, inside the address's limit for calls
/// without a session (a burst of 150, which everyone behind one address
/// shares). The screens may ask the core more often; it answers from what
/// it has meanwhile.
const ASK_EVERY: Duration = Duration::from_secs(5);
const ASK_EVERY_LATER: Duration = Duration::from_secs(10);
const LATER_AFTER: Duration = Duration::from_secs(2 * 60);

/// Slack for a screen's timer that fires a little early.
const EARLY: Duration = Duration::from_secs(1);

/// On a device that hasn't finished a sign-in to the account: the server
/// takes requests only from one that has.
pub(crate) const NOT_KNOWN_HERE: Error = Error::Refused(
    "sign in with a device works on a device that has signed in to this account before. Use your master password this time",
);

/// An approval whose key doesn't work here: not a type 4 string, one that
/// doesn't open with this request's private key, one that isn't 64 bytes,
/// or a user key that doesn't open the account's private key.
pub(crate) const BAD_KEY: Error = Error::Refused(
    "the approval didn't carry a key that opens your vault. Send a new request, or use your master password",
);

/// An approval the server won't sign in with: used already, past its 15
/// minutes, or asked from another address than this device has now.
pub(crate) const UNUSABLE: Error = Error::Refused(
    "that approval can't be used to sign in here. Send a new request, or use your master password",
);

/// How a request stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Waiting for the person to approve it on another device.
    Waiting,
    /// Denied on another device.
    Denied,
    /// Its 15 minutes are up.
    Expired,
    /// Approved, with a key that doesn't open this account's vault.
    BadKey,
    /// Approved, but the server won't sign in with it (`UNUSABLE`).
    Unusable,
}

impl Status {
    /// As the screens name it.
    pub fn name(self) -> &'static str {
        match self {
            Status::Waiting => "waiting",
            Status::Denied => "denied",
            Status::Expired => "expired",
            Status::BadKey => "badKey",
            Status::Unusable => "unusable",
        }
    }
}

/// A request this device made, as the screens show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// The account's email, in lower case.
    pub email: String,
    /// The phrase the approving device shows too.
    pub fingerprint: String,
    pub status: Status,
    /// Seconds left of its 15 minutes, by this device's clock; 0 once it's
    /// over.
    pub left: u64,
}

/// A new access code: `ACCESS_CODE_LENGTH` characters of
/// `ACCESS_CODE_ALPHABET`, from the OS's random source.
pub fn access_code() -> Zeroizing<String> {
    let mut code = Zeroizing::new(String::with_capacity(ACCESS_CODE_LENGTH));
    for _ in 0..ACCESS_CODE_LENGTH {
        code.push(char::from(
            ACCESS_CODE_ALPHABET[OsRng.gen_range(0..ACCESS_CODE_ALPHABET.len())],
        ));
    }
    code
}

/// The user key from an approval's key (`key`, as the server sends it): a
/// type 4 string (RSA-OAEP with SHA-1), opened with this request's private
/// key, of 64 bytes. Anything else is `BAD_KEY`.
pub fn open_approval(private_key: &PrivateKey, key: &str) -> Result<SymmetricKey> {
    let wrapped = EncString::parse(key).map_err(|_| BAD_KEY)?;
    if !matches!(wrapped, EncString::Rsa { sha256: false, .. }) {
        return Err(BAD_KEY);
    }
    let opened = private_key.decrypt(&wrapped).map_err(|_| BAD_KEY)?;
    if opened.len() != 64 {
        return Err(BAD_KEY);
    }
    SymmetricKey::from_bytes(&opened).map_err(|_| BAD_KEY)
}

/// Whether a user key is the account's: it opens the account's private key
/// as the server's sign-in answer carries it (`PrivateKey`, a type 2 string
/// whose MAC the key checks first). Checked before anything of the sign-in
/// is kept; `BAD_KEY` otherwise, and for an answer without it.
pub fn opens_account(user_key: &SymmetricKey, private_key: Option<&str>) -> Result<()> {
    let wrapped = EncString::parse(private_key.ok_or(BAD_KEY)?).map_err(|_| BAD_KEY)?;
    let der = wrapped.decrypt(user_key).map_err(|_| BAD_KEY)?;
    PrivateKey::from_pkcs8_der(&der).map_err(|_| BAD_KEY)?;
    Ok(())
}

/// What the server's answer to reading a request says.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Reading {
    Waiting,
    /// Approved, with the wrapped key, if it sent one.
    Approved(Option<String>),
    Denied,
    Expired,
}

/// Reads the server's answer to `GET /api/auth-requests/<id>/response`,
/// `age` after the request was sent. Our server deletes a denied request and
/// answers one that's gone like one that's expired (400; a server that
/// answers 404 is read the same); upstream's keeps a denied one, answered.
/// Gone before its 15 minutes is a denial: nothing else removes a waiting
/// request but this device's own next one, or the server's purge once its 15
/// minutes are up. A 429 is still waiting. Anything else is the server's.
pub(crate) fn read_answer(status: u16, body: &Value, age: Duration) -> Result<Reading> {
    match status {
        200..=299 => Ok(match body["requestApproved"].as_bool() {
            Some(true) => Reading::Approved(body["key"].as_str().map(str::to_owned)),
            Some(false) => Reading::Denied,
            None => Reading::Waiting,
        }),
        400 | 404 if age < LIFETIME => Ok(Reading::Denied),
        400 | 404 => Ok(Reading::Expired),
        429 => Ok(Reading::Waiting),
        _ => Server::checked(status, body.clone()).map(|_| Reading::Waiting),
    }
}

/// Where a request got to after asking (`Asking::ask_at`).
pub(crate) enum Next {
    Waiting,
    Ended(Status),
    /// Approved: the user key it carried.
    Approved(SymmetricKey),
}

/// A request this device sent and is waiting on: its private key and access
/// code, in memory only (see the module's notes).
pub struct Asking {
    email: String,
    id: String,
    private_key: PrivateKey,
    access_code: Zeroizing<String>,
    fingerprint: String,
    /// When it was sent: before the server made it, so the 15 minutes here
    /// never end after the server's. On both clocks: the monotonic one
    /// stops while a computer sleeps, and the wall clock can be set back.
    sent: Instant,
    sent_wall: SystemTime,
    /// When the server was last asked for the answer.
    asked: Option<Instant>,
    /// The user key, once an approval brought it and the sign-in with it
    /// couldn't reach the server: signing in is tried again without asking.
    approved: Option<SymmetricKey>,
}

impl std::fmt::Debug for Asking {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Asking")
            .field("email", &self.email)
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Asking {
    /// Makes a key pair and an access code and sends the request (`POST
    /// /api/auth-requests`), for `email` (in lower case, as the phrase has
    /// it).
    pub(crate) fn start(server: &Server, email: &str) -> Result<Asking> {
        let email = normalize_email(email);
        let private_key = PrivateKey::generate()?;
        let public_key = private_key.public_key_der();
        let access_code = access_code();
        let (sent, sent_wall) = (Instant::now(), SystemTime::now());
        let id = server.ask_to_sign_in(&email, &STANDARD.encode(&public_key), &access_code)?;
        Ok(Asking {
            fingerprint: fingerprint_phrase(&email, &public_key),
            email,
            id,
            private_key,
            access_code,
            sent,
            sent_wall,
            asked: None,
            approved: None,
        })
    }

    /// How long ago it was sent, at `now`: the longer of the two clocks',
    /// so a computer that slept meanwhile counts the time it slept.
    fn age_at(&self, now: Instant) -> Duration {
        let wall = SystemTime::now()
            .duration_since(self.sent_wall)
            .unwrap_or_default();
        now.saturating_duration_since(self.sent).max(wall)
    }

    /// How it stands at `now`, as the screens show it, waiting.
    pub(crate) fn request(&self, now: Instant) -> Request {
        self.request_as(Status::Waiting, now)
    }

    /// The same, as `status`.
    pub(crate) fn request_as(&self, status: Status, now: Instant) -> Request {
        let left = LIFETIME.saturating_sub(self.age_at(now));
        Request {
            email: self.email.clone(),
            fingerprint: self.fingerprint.clone(),
            status,
            left: if status == Status::Waiting {
                left.as_secs()
            } else {
                0
            },
        }
    }

    /// The request's id and access code, and the account's email, for the
    /// sign-in with its approval.
    pub(crate) fn proof(&self) -> (&str, &str, &str) {
        (&self.id, &self.access_code, &self.email)
    }

    /// Keeps the user key an approval brought, while signing in with it
    /// couldn't reach the server.
    pub(crate) fn keep_approved(&mut self, user_key: SymmetricKey) {
        self.approved = Some(user_key);
    }

    /// Whether the server is asked now: not after the 15 minutes, and not
    /// sooner than every 5 seconds, then 10.
    fn due(&self, now: Instant) -> std::result::Result<bool, Status> {
        let age = self.age_at(now);
        if age >= LIFETIME {
            return Err(Status::Expired);
        }
        let every = if age < LATER_AFTER {
            ASK_EVERY
        } else {
            ASK_EVERY_LATER
        };
        Ok(self
            .asked
            .is_none_or(|at| now.saturating_duration_since(at) + EARLY >= every))
    }

    /// Asks the server for the answer, if it's time (`due`), at `now`.
    pub(crate) fn ask_at(&mut self, server: &Server, now: Instant) -> Result<Next> {
        if let Some(user_key) = &self.approved {
            return Ok(Next::Approved(user_key.clone()));
        }
        match self.due(now) {
            Err(status) => return Ok(Next::Ended(status)),
            Ok(false) => return Ok(Next::Waiting),
            Ok(true) => {}
        }
        self.asked = Some(now);
        let (status, body) = server.sign_in_answer(&self.id, &self.access_code)?;
        let age = self.age_at(now);
        Ok(match read_answer(status, &body, age)? {
            Reading::Waiting => Next::Waiting,
            Reading::Denied => Next::Ended(Status::Denied),
            Reading::Expired => Next::Ended(Status::Expired),
            Reading::Approved(key) => {
                match open_approval(&self.private_key, key.as_deref().unwrap_or_default()) {
                    Ok(user_key) => Next::Approved(user_key),
                    Err(_) => Next::Ended(Status::BadKey),
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn b64(v: &Value) -> Vec<u8> {
        STANDARD.decode(v.as_str().unwrap()).unwrap()
    }

    /// docs/vectors/device-signin-v1.json in the meta repo, which plain Node
    /// made and upstream's own crypto checks.
    fn vectors() -> Value {
        serde_json::from_str(include_str!("../tests/vectors/device-signin-v1.json")).unwrap()
    }

    fn request_key(v: &Value) -> PrivateKey {
        PrivateKey::from_pkcs8_der(&b64(&v["request"]["privateKey"])).unwrap()
    }

    #[test]
    fn an_access_code_is_25_characters_of_upstreams_alphabet_and_new_each_time() {
        let v = vectors();
        assert_eq!(
            ACCESS_CODE_ALPHABET,
            v["accessCode"]["alphabet"].as_str().unwrap().as_bytes()
        );
        assert_eq!(
            ACCESS_CODE_LENGTH as u64,
            v["accessCode"]["length"].as_u64().unwrap()
        );
        assert_eq!(ACCESS_CODE_ALPHABET.len(), 57);
        let codes: Vec<_> = (0..50).map(|_| access_code()).collect();
        for code in &codes {
            assert_eq!(code.len(), 25);
            assert!(code.bytes().all(|b| ACCESS_CODE_ALPHABET.contains(&b)));
            // Nothing in it changes in an address, or is a look-alike.
            assert!(!code.contains(['0', '1', 'I', 'O', 'l', '#', '/', '%', '+']));
        }
        let mut distinct: Vec<&str> = codes.iter().map(|c| c.as_str()).collect();
        distinct.sort();
        distinct.dedup();
        assert_eq!(distinct.len(), 50);
    }

    #[test]
    fn the_key_pair_is_rsa_2048_with_upstreams_exponent_and_new_each_time() {
        use rsa::{pkcs8::DecodePublicKey, traits::PublicKeyParts, BigUint, RsaPublicKey};
        let made = PrivateKey::generate().unwrap();
        let public = RsaPublicKey::from_public_key_der(&made.public_key_der()).unwrap();
        assert_eq!(public.size(), 256);
        assert_eq!(*public.e(), BigUint::from(65_537u32));
        // The approving side takes it: a user key wraps to it and opens.
        let user_key = SymmetricKey::generate();
        let wrapped =
            crate::device_approval::wrap_user_key(&made.public_key_der(), &user_key).unwrap();
        assert_eq!(
            open_approval(&made, &wrapped.to_string()).unwrap(),
            user_key
        );
        assert_ne!(
            PrivateKey::generate().unwrap().public_key_der(),
            made.public_key_der()
        );
    }

    #[test]
    fn makes_the_vectors_fingerprint_phrase_for_the_requests_key() {
        let v = vectors();
        let public = request_key(&v).public_key_der();
        assert_eq!(public, b64(&v["request"]["publicKey"]));
        for f in v["fingerprints"].as_array().unwrap() {
            let email = f["email"].as_str().unwrap();
            // As `Asking::start` makes it: the email in lower case.
            assert_eq!(
                fingerprint_phrase(&normalize_email(email), &public),
                f["phrase"].as_str().unwrap(),
                "{}",
                f["why"]
            );
        }
    }

    #[test]
    fn the_vectors_approval_opens_to_the_user_key_which_opens_the_account() {
        let v = vectors();
        let private = request_key(&v);
        let user_key = open_approval(&private, v["approval"]["key"].as_str().unwrap()).unwrap();
        assert_eq!(&*user_key.to_bytes(), &*b64(&v["approval"]["userKey"]));
        opens_account(&user_key, v["account"]["privateKey"].as_str()).unwrap();
        // An answer without the account's keys isn't one to keep.
        assert_eq!(opens_account(&user_key, None), Err(BAD_KEY));
    }

    #[test]
    fn another_accounts_user_key_opens_but_not_the_account() {
        let v = vectors();
        let other = &v["notTheAccount"];
        let user_key = open_approval(&request_key(&v), other["key"].as_str().unwrap()).unwrap();
        assert_eq!(&*user_key.to_bytes(), &*b64(&other["userKey"]));
        assert_eq!(
            opens_account(&user_key, v["account"]["privateKey"].as_str()),
            Err(BAD_KEY),
            "{}",
            other["why"]
        );
    }

    #[test]
    fn refuses_approvals_that_arent_a_user_key_for_this_request() {
        let v = vectors();
        let private = request_key(&v);
        for r in v["refused"].as_array().unwrap() {
            assert_eq!(
                open_approval(&private, r["key"].as_str().unwrap()),
                Err(BAD_KEY),
                "{}",
                r["why"]
            );
        }
        // And from another request's key, the vectors' own approval.
        assert_eq!(
            open_approval(
                &PrivateKey::generate().unwrap(),
                v["approval"]["key"].as_str().unwrap()
            ),
            Err(BAD_KEY)
        );
    }

    #[test]
    fn reads_the_servers_answers() {
        let v = vectors();
        let early = Duration::from_secs(60);
        let waiting = json!({ "requestApproved": null, "key": null, "responseDate": null });
        assert_eq!(read_answer(200, &waiting, early), Ok(Reading::Waiting));
        let approved = json!({ "requestApproved": true, "key": v["approval"]["key"] });
        assert_eq!(
            read_answer(200, &approved, early),
            Ok(Reading::Approved(Some(
                v["approval"]["key"].as_str().unwrap().into()
            )))
        );
        assert_eq!(
            read_answer(200, &json!({ "requestApproved": true }), early),
            Ok(Reading::Approved(None))
        );
        // Upstream's servers keep a denied request, answered no.
        let denied = json!({ "requestApproved": false, "responseDate": "2026-11-18T12:00:00Z" });
        assert_eq!(read_answer(200, &denied, early), Ok(Reading::Denied));
        // Ours deletes it: gone before the 15 minutes is a denial, gone
        // after them expired, whether the server says 400 or 404.
        let gone = json!({ "message": "AuthRequest doesn't exist" });
        for status in [400, 404] {
            assert_eq!(read_answer(status, &gone, early), Ok(Reading::Denied));
            assert_eq!(
                read_answer(status, &gone, Duration::from_secs(15 * 60 - 1)),
                Ok(Reading::Denied)
            );
            assert_eq!(
                read_answer(status, &gone, Duration::from_secs(15 * 60)),
                Ok(Reading::Expired)
            );
        }
        // Too many calls from this address for a moment: still waiting.
        assert_eq!(
            read_answer(429, &json!({ "message": "Too many requests" }), early),
            Ok(Reading::Waiting)
        );
        // A problem of the server's is said, and the request goes on.
        assert_eq!(
            read_answer(500, &Value::Null, early),
            Err(Error::Status(500))
        );
    }

    #[test]
    fn what_a_request_says_is_left() {
        let v = vectors();
        let mut asking = Asking {
            email: "dana@example.test".into(),
            id: "6f1c2b0e-0000-4000-8000-000000000001".into(),
            private_key: request_key(&v),
            access_code: access_code(),
            fingerprint: v["fingerprints"][0]["phrase"].as_str().unwrap().into(),
            sent: Instant::now(),
            sent_wall: SystemTime::now(),
            asked: None,
            approved: None,
        };
        let sent = asking.sent;
        let at = |seconds: u64| sent + Duration::from_secs(seconds);
        let request = asking.request(at(60));
        assert_eq!(request.status, Status::Waiting);
        assert_eq!(request.left, 14 * 60);
        assert_eq!(request.email, "dana@example.test");
        assert_eq!(
            request.fingerprint,
            "pacifier-unvisited-amuser-sublime-tropics"
        );
        assert_eq!(asking.request_as(Status::Denied, at(60)).left, 0);

        // Asked no more often than every 5 seconds for 2 minutes, then
        // every 10, and not at all after the 15 minutes.
        assert_eq!(asking.due(sent), Ok(true));
        asking.asked = Some(at(10));
        assert_eq!(asking.due(at(12)), Ok(false));
        assert_eq!(asking.due(sent + Duration::from_millis(14_500)), Ok(true));
        asking.asked = Some(at(180));
        assert_eq!(asking.due(at(185)), Ok(false));
        assert_eq!(asking.due(at(189)), Ok(true));
        assert_eq!(asking.due(at(15 * 60 - 1)), Ok(true));
        assert_eq!(asking.due(at(15 * 60)), Err(Status::Expired));
        assert_eq!(asking.request(at(15 * 60)).left, 0);
        assert_eq!(asking.request(at(20 * 60)).left, 0);

        // A computer that slept: the monotonic clock stood still, the wall
        // clock didn't, and the longer counts.
        asking.sent_wall = SystemTime::UNIX_EPOCH;
        assert_eq!(asking.due(sent), Err(Status::Expired));
        assert_eq!(asking.request(sent).left, 0);
    }

    #[test]
    fn statuses_have_the_screens_names() {
        let names: Vec<_> = [
            Status::Waiting,
            Status::Denied,
            Status::Expired,
            Status::BadKey,
            Status::Unusable,
        ]
        .map(Status::name)
        .into();
        assert_eq!(
            names,
            ["waiting", "denied", "expired", "badKey", "unusable"]
        );
        assert_eq!(
            NOT_KNOWN_HERE.to_string(),
            "Sign in with a device works on a device that has signed in to this account before. Use your master password this time."
        );
    }
}

/// The whole asking side through `App`, against a stand-in server: what goes
/// on the wire, signing in and unlocking with the approval's user key, and
/// nothing held once a request ends.
#[cfg(test)]
mod signing_in {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::*;
    use crate::app::{App, State, Step};
    use crate::secrets::{MemorySecretStore, Secret, SecretStore};
    use crate::store::Store;

    /// One request the stand-in took: method, address, headers (names in
    /// lower case), and body.
    #[derive(Clone, Debug)]
    struct Asked {
        method: String,
        url: String,
        headers: Vec<(String, String)>,
        body: String,
    }

    impl Asked {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str())
        }
    }

    /// How the stand-in answers a read of a request's answer.
    #[derive(Clone, Copy)]
    enum Reply {
        Waiting,
        /// Gone: denied (our server deletes it), or expired.
        Gone,
        /// Approved with the account's user key, wrapped to the request.
        Approve,
        /// Approved with another account's user key.
        ApproveOther,
    }

    #[derive(Default)]
    struct Script {
        known: bool,
        replies: VecDeque<Reply>,
        /// Answers to sign-ins, in order; then a sign-in that goes through.
        sign_ins: VecDeque<(u16, Value)>,
        /// The public key of the newest request.
        public_key: Vec<u8>,
        requests: u32,
    }

    fn core_vectors() -> Value {
        serde_json::from_str(include_str!("../tests/vectors/core-v1.json")).unwrap()
    }

    /// An access token as passbundle-server makes one, saying how its
    /// session signed in.
    fn access_token(signed_in: &str) -> String {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let claims = json!({ "sub": "u", "keykeeper_signin": signed_in });
        format!(
            "eyJhbGciOiJSUzI1NiJ9.{}.c2lnbmF0dXJl",
            URL_SAFE_NO_PAD.encode(claims.to_string())
        )
    }

    /// What the stand-in answers with, and what it was asked, shared with the
    /// test.
    type Shared<T> = Arc<Mutex<T>>;

    fn stand_in(known: bool) -> (String, Shared<Script>, Shared<Vec<Asked>>) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr().to_ip().unwrap());
        let script = Arc::new(Mutex::new(Script {
            known,
            ..Script::default()
        }));
        let log = Arc::new(Mutex::new(Vec::new()));
        let (s, l) = (script.clone(), log.clone());
        std::thread::spawn(move || {
            let v = core_vectors();
            for mut request in server.incoming_requests() {
                let mut body = String::new();
                let _ = request.as_reader().read_to_string(&mut body);
                let asked = Asked {
                    method: request.method().to_string(),
                    url: request.url().to_owned(),
                    headers: request
                        .headers()
                        .iter()
                        .map(|h| {
                            (
                                h.field.as_str().as_str().to_lowercase(),
                                h.value.as_str().to_owned(),
                            )
                        })
                        .collect(),
                    body: body.clone(),
                };
                l.lock().unwrap().push(asked.clone());
                let mut script = s.lock().unwrap();
                let path = asked.url.split('?').next().unwrap_or_default().to_owned();
                let (status, answer) = match (asked.method.as_str(), path.as_str()) {
                    ("POST", "/identity/accounts/prelogin") => {
                        (200, v["account"]["pbkdf2"]["prelogin"].clone())
                    }
                    ("GET", "/api/devices/knowndevice") => (200, json!(script.known)),
                    ("POST", "/api/auth-requests") => {
                        let sent: Value = serde_json::from_str(&body).unwrap();
                        script.public_key = b64(&sent["publicKey"]);
                        script.requests += 1;
                        let id = format!("6f1c2b0e-0000-4000-8000-00000000000{}", script.requests);
                        (200, json!({ "id": id, "requestApproved": false }))
                    }
                    ("GET", p) if p.starts_with("/api/auth-requests/") => {
                        let reply = script.replies.pop_front().unwrap_or(Reply::Waiting);
                        let public_key = script.public_key.clone();
                        let wrap = |user_key: &str| {
                            let key = SymmetricKey::from_bytes(&b64(&json!(user_key))).unwrap();
                            crate::device_approval::wrap_user_key(&public_key, &key)
                                .unwrap()
                                .to_string()
                        };
                        match reply {
                            Reply::Waiting => (200, json!({ "requestApproved": null })),
                            Reply::Gone => (400, json!({ "message": "AuthRequest doesn't exist" })),
                            Reply::Approve => (
                                200,
                                json!({
                                    "requestApproved": true,
                                    "key": wrap(v["keys"]["userKey"].as_str().unwrap()),
                                }),
                            ),
                            Reply::ApproveOther => (
                                200,
                                json!({
                                    "requestApproved": true,
                                    "key": wrap(&STANDARD.encode(SymmetricKey::generate().to_bytes())),
                                }),
                            ),
                        }
                    }
                    ("POST", "/identity/connect/token") => {
                        script.sign_ins.pop_front().unwrap_or_else(|| {
                            (
                                200,
                                json!({
                                    "access_token": access_token("authrequest"),
                                    "refresh_token": "refresh",
                                    "expires_in": 3600,
                                    "Key": v["sync"]["profile"]["key"],
                                    "PrivateKey": v["sync"]["profile"]["privateKey"],
                                }),
                            )
                        })
                    }
                    ("GET", "/api/sync") => (200, v["sync"].clone()),
                    ("POST", "/api/two-factor/send-email-login") => (200, Value::Null),
                    _ => (404, json!({ "message": "Not found" })),
                };
                drop(script);
                let _ = request.respond(
                    tiny_http::Response::from_string(answer.to_string())
                        .with_status_code(status)
                        .with_header(
                            tiny_http::Header::from_bytes("Content-Type", "application/json")
                                .unwrap(),
                        ),
                );
            }
        });
        (url, script, log)
    }

    fn b64(v: &Value) -> Vec<u8> {
        STANDARD.decode(v.as_str().unwrap()).unwrap()
    }

    /// A sign-in's form, field by field.
    fn form(body: &str) -> std::collections::HashMap<String, String> {
        let decode = |text: &str| {
            let bytes = text.replace('+', " ").into_bytes();
            let mut out = Vec::new();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'%' && i + 2 < bytes.len() {
                    let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
                    out.push(u8::from_str_radix(hex, 16).unwrap());
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            String::from_utf8(out).unwrap()
        };
        body.split('&')
            .filter_map(|pair| pair.split_once('='))
            .map(|(k, v)| (decode(k), decode(v)))
            .collect()
    }

    fn app() -> (App, Arc<MemorySecretStore>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("pb-device-signin-{}", uuid::Uuid::new_v4()));
        let secrets = Arc::new(MemorySecretStore::default());
        let mut app = App::open(Store::new(&dir), Box::new(secrets.clone()));
        app.use_install_identifier("3a1b5c7d-0000-4000-8000-0000000000aa");
        (app, secrets, dir)
    }

    fn requested(step: Step) -> Request {
        match step {
            Step::DeviceSignIn(request) => request,
            other => panic!("not a request to sign in with a device: {other:?}"),
        }
    }

    fn email() -> String {
        core_vectors()["account"]["salt"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn an_approval_signs_in_opens_the_vault_and_the_master_password_unlocks_it_later() {
        let (url, script, log) = stand_in(true);
        let (mut app, secrets, dir) = app();
        let v = core_vectors();
        assert!(secrets.keep_secret(Secret::DeviceToken, &url, &email(), "device-token-1"));

        let typed = v["account"]["emailAsTyped"].as_str().unwrap();
        let request = requested(app.sign_in_with_device(&format!("{url}/"), typed).unwrap());
        assert_eq!(request.status, Status::Waiting);
        assert_eq!(request.email, email());
        assert!(request.left > 14 * 60);
        assert!(app.signing_in());
        // Nothing of the account is kept here yet.
        assert_eq!(
            app.state(),
            State::SignedOut {
                server: None,
                email: None
            }
        );

        let asked = log.lock().unwrap().clone();
        let paths: Vec<&str> = asked.iter().map(|a| a.url.as_str()).collect();
        assert_eq!(
            paths,
            [
                "/identity/accounts/prelogin",
                "/api/devices/knowndevice",
                "/api/auth-requests"
            ]
        );
        // The known-device check: the email in base64url, this install's
        // identifier.
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        assert_eq!(
            asked[1].header("x-request-email"),
            Some(URL_SAFE_NO_PAD.encode(email()).as_str())
        );
        assert_eq!(
            asked[1].header("x-device-identifier"),
            Some("3a1b5c7d-0000-4000-8000-0000000000aa")
        );
        // The request: the email in lower case, this device, the key, the
        // code, type 0, and the device's type where the server reads it.
        let sent: Value = serde_json::from_str(&asked[2].body).unwrap();
        assert_eq!(sent["email"], json!(email()));
        assert_eq!(
            sent["deviceIdentifier"],
            "3a1b5c7d-0000-4000-8000-0000000000aa"
        );
        assert_eq!(sent["type"], 0);
        let code = sent["accessCode"].as_str().unwrap().to_owned();
        assert_eq!(code.len(), ACCESS_CODE_LENGTH);
        let kind = crate::server::Device::this_one("x").kind.to_string();
        assert_eq!(asked[2].header("device-type"), Some(kind.as_str()));
        // The phrase is the one the approving device makes from that key.
        let public = b64(&sent["publicKey"]);
        assert_eq!(request.fingerprint, fingerprint_phrase(&email(), &public));

        // Waiting, then approved: signed in, and the vault open with the
        // approval's user key.
        script.lock().unwrap().replies = [Reply::Waiting, Reply::Approve].into();
        let now = Instant::now();
        let waiting = requested(app.device_sign_in_status_at(now).unwrap());
        assert_eq!(waiting.status, Status::Waiting);
        // Asked again at once: no call, the same answer.
        let calls = log.lock().unwrap().len();
        assert_eq!(
            requested(app.device_sign_in_status_at(now).unwrap()).status,
            Status::Waiting
        );
        assert_eq!(log.lock().unwrap().len(), calls);
        let step = app
            .device_sign_in_status_at(now + Duration::from_secs(5))
            .unwrap();
        assert_eq!(step, Step::Done);
        assert!(!app.signing_in());
        assert!(matches!(app.state(), State::Unlocked { .. }));
        assert_eq!(
            &*app.vault().unwrap().user_key().to_bytes(),
            &*b64(&v["keys"]["userKey"])
        );

        let asked = log.lock().unwrap().clone();
        let reads: Vec<&Asked> = asked
            .iter()
            .filter(|a| a.url.starts_with("/api/auth-requests/"))
            .collect();
        assert_eq!(reads.len(), 2);
        for read in &reads {
            assert_eq!(
                read.url,
                format!(
                    "/api/auth-requests/6f1c2b0e-0000-4000-8000-000000000001/response?code={code}"
                )
            );
            assert_eq!(read.header("device-type"), Some(kind.as_str()));
        }
        // The sign-in: the access code as the password, the request, this
        // device, and its device token, as a password sign-in sends it.
        let sign_in = asked
            .iter()
            .find(|a| a.url == "/identity/connect/token")
            .unwrap();
        let form = form(&sign_in.body);
        assert_eq!(form["grant_type"], "password");
        assert_eq!(form["username"], email());
        assert_eq!(form["password"], code);
        assert_eq!(form["authRequest"], "6f1c2b0e-0000-4000-8000-000000000001");
        assert_eq!(
            form["deviceIdentifier"],
            "3a1b5c7d-0000-4000-8000-0000000000aa"
        );
        assert_eq!(form["deviceType"], kind);
        assert_eq!(form["twoFactorProvider"], "5");
        assert_eq!(form["twoFactorToken"], "device-token-1");

        // Later, the master password unlocks it, as after any sign-in.
        app.lock();
        assert!(matches!(app.state(), State::Locked { .. }));
        app.unlock(v["account"]["password"].as_str().unwrap())
            .unwrap();
        assert!(matches!(app.state(), State::Unlocked { .. }));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_device_the_account_doesnt_know_makes_no_key_and_sends_no_request() {
        let (url, _, log) = stand_in(false);
        let (mut app, _, dir) = app();
        assert_eq!(app.sign_in_with_device(&url, &email()), Err(NOT_KNOWN_HERE));
        assert!(!app.signing_in());
        assert!(!log
            .lock()
            .unwrap()
            .iter()
            .any(|a| a.url.starts_with("/api/auth-requests")));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Whatever ends a request, nothing of it is held after: the core has
    /// no request to ask about, and the screens heard how it ended.
    fn ended(app: &mut App, status: Status) {
        assert!(!app.signing_in(), "{status:?}");
        assert_eq!(app.device_sign_in_status(), Err(Error::SignedOut));
    }

    #[test]
    fn a_denial_expiry_or_key_that_doesnt_work_ends_the_request_and_drops_its_key() {
        let (url, script, log) = stand_in(true);
        let (mut app, _, dir) = app();
        let now = Instant::now();

        // Denied: gone before its 15 minutes.
        requested(app.sign_in_with_device(&url, &email()).unwrap());
        script.lock().unwrap().replies = [Reply::Gone].into();
        let denied = requested(app.device_sign_in_status_at(now).unwrap());
        assert_eq!((denied.status, denied.left), (Status::Denied, 0));
        ended(&mut app, Status::Denied);

        // Gone after its 15 minutes: expired. And past them, the core
        // says so without asking.
        requested(app.sign_in_with_device(&url, &email()).unwrap());
        let calls = log.lock().unwrap().len();
        let later = Instant::now() + LIFETIME;
        let expired = requested(app.device_sign_in_status_at(later).unwrap());
        assert_eq!(expired.status, Status::Expired);
        assert_eq!(log.lock().unwrap().len(), calls);
        ended(&mut app, Status::Expired);

        // A key wrapped to this request, but not this account's: refused
        // when it doesn't open the account's private key, with nothing kept.
        requested(app.sign_in_with_device(&url, &email()).unwrap());
        script.lock().unwrap().replies = [Reply::ApproveOther].into();
        let bad = requested(app.device_sign_in_status_at(Instant::now()).unwrap());
        assert_eq!(bad.status, Status::BadKey);
        ended(&mut app, Status::BadKey);
        assert_eq!(
            app.state(),
            State::SignedOut {
                server: None,
                email: None
            }
        );

        // An approval the server won't sign in with (used, or from another
        // address).
        requested(app.sign_in_with_device(&url, &email()).unwrap());
        {
            let mut s = script.lock().unwrap();
            s.replies = [Reply::Approve].into();
            s.sign_ins = [(
                400,
                json!({ "error": "invalid_grant", "error_description": "Username or access code is incorrect. Try again" }),
            )]
            .into();
        }
        let unusable = requested(app.device_sign_in_status_at(Instant::now()).unwrap());
        assert_eq!(unusable.status, Status::Unusable);
        ended(&mut app, Status::Unusable);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn resend_replaces_the_request_and_giving_up_or_locking_drops_it() {
        let (url, _, log) = stand_in(true);
        let (mut app, _, dir) = app();
        let first = requested(app.sign_in_with_device(&url, &email()).unwrap());
        // Resend: a new key, so a new phrase, and the old one's gone.
        let second = requested(app.sign_in_with_device(&url, &email()).unwrap());
        assert_ne!(first.fingerprint, second.fingerprint);
        let posted: Vec<Value> = log
            .lock()
            .unwrap()
            .iter()
            .filter(|a| a.url == "/api/auth-requests")
            .map(|a| serde_json::from_str(&a.body).unwrap())
            .collect();
        assert_eq!(posted.len(), 2);
        assert_ne!(posted[0]["publicKey"], posted[1]["publicKey"]);
        assert_ne!(posted[0]["accessCode"], posted[1]["accessCode"]);
        // The core asks about the newest only.
        app.device_sign_in_status().unwrap();
        assert!(log
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .url
            .starts_with("/api/auth-requests/6f1c2b0e-0000-4000-8000-000000000002/response"));

        // "Use master password instead".
        app.cancel_sign_in();
        ended(&mut app, Status::Waiting);
        // Locking.
        requested(app.sign_in_with_device(&url, &email()).unwrap());
        app.lock();
        ended(&mut app, Status::Waiting);
        // A sign-in with the master password starting meanwhile.
        requested(app.sign_in_with_device(&url, &email()).unwrap());
        let _ = app.sign_in(&url, &email(), "not the password");
        assert_eq!(app.device_sign_in_status(), Err(Error::SignedOut));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn two_step_login_and_an_organizations_rules_follow_as_for_a_password() {
        let (url, script, log) = stand_in(true);
        let (mut app, _, dir) = app();

        // Two-step login: its screen follows, and its code goes with the
        // same approval; an emailed code is asked for with the request,
        // never a password.
        requested(app.sign_in_with_device(&url, &email()).unwrap());
        let two_step = json!({
            "error": "invalid_grant",
            "TwoFactorProviders": ["0", "1"],
            "TwoFactorProviders2": { "0": null, "1": { "Email": "a***@example.test" } },
        });
        {
            let mut s = script.lock().unwrap();
            s.replies = [Reply::Approve].into();
            s.sign_ins = [(400, two_step)].into();
        }
        let step = app.device_sign_in_status().unwrap();
        assert!(matches!(step, Step::TwoFactor(_)), "{step:?}");
        assert!(app.signing_in());
        app.send_email_code().unwrap();
        assert_eq!(app.finish_sign_in(0, "123456", false), Ok(Step::Done));
        let asked = log.lock().unwrap().clone();
        let email_code: Value = serde_json::from_str(
            &asked
                .iter()
                .find(|a| a.url == "/api/two-factor/send-email-login")
                .unwrap()
                .body,
        )
        .unwrap();
        let code = serde_json::from_str::<Value>(
            &asked
                .iter()
                .find(|a| a.url == "/api/auth-requests")
                .unwrap()
                .body,
        )
        .unwrap()["accessCode"]
            .clone();
        assert_eq!(
            email_code,
            json!({
                "email": email(),
                "authRequestId": "6f1c2b0e-0000-4000-8000-000000000001",
                "authRequestAccessCode": code,
            })
        );
        let finished = form(
            &asked
                .iter()
                .rfind(|a| a.url == "/identity/connect/token")
                .unwrap()
                .body,
        );
        assert_eq!(
            finished["authRequest"],
            "6f1c2b0e-0000-4000-8000-000000000001"
        );
        assert_eq!(finished["password"], code.as_str().unwrap());
        assert_eq!(finished["twoFactorProvider"], "0");
        assert_eq!(finished["twoFactorToken"], "123456");

        // "Require single sign-on": the server's sentence, as a password
        // sign-in hears it, and the request is over.
        app.sign_out();
        requested(app.sign_in_with_device(&url, &email()).unwrap());
        let sso = "Your organization requires you to sign in with single sign-on.";
        {
            let mut s = script.lock().unwrap();
            s.replies = [Reply::Approve].into();
            s.sign_ins = [(
                400,
                json!({ "error": "invalid_grant", "error_description": sso, "SsoOrganizationIdentifier": "acme" }),
            )]
            .into();
        }
        assert_eq!(app.device_sign_in_status(), Err(Error::Server(sso.into())));
        ended(&mut app, Status::Waiting);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_session_from_an_approval_lists_no_requests_and_refuses_to_approve_one() {
        let (url, script, log) = stand_in(true);
        let (mut app, _, dir) = app();
        requested(app.sign_in_with_device(&url, &email()).unwrap());
        script.lock().unwrap().replies = [Reply::Approve].into();
        assert_eq!(app.device_sign_in_status(), Ok(Step::Done));

        // Its access token says it came from an approval: no request is
        // asked for or listed, and approving is refused before anything is
        // sent.
        let calls = log.lock().unwrap().len();
        let call = app.sign_in_requests_call().unwrap();
        assert!(!call.approves);
        assert!(app.sign_in_requests().unwrap().is_empty());
        assert_eq!(log.lock().unwrap().len(), calls);
        assert_eq!(
            app.answer_sign_in_request("6f1c2b0e-0000-4000-8000-0000000000ff", "a-b-c-d-e", true),
            Err(APPROVAL_SESSION_WORDS)
        );
        assert_eq!(log.lock().unwrap().len(), calls);

        // A session that began with the master password asks, as before.
        let v = core_vectors();
        script.lock().unwrap().sign_ins = [(
            200,
            json!({
                "access_token": access_token("password"),
                "refresh_token": "refresh",
                "expires_in": 3600,
                "Key": v["sync"]["profile"]["key"],
                "PrivateKey": v["sync"]["profile"]["privateKey"],
            }),
        )]
        .into();
        assert_eq!(
            app.sign_in(&url, &email(), v["account"]["password"].as_str().unwrap()),
            Ok(Step::Done)
        );
        assert!(app.sign_in_requests_call().unwrap().approves);
        app.sign_in_requests().unwrap();
        assert!(log
            .lock()
            .unwrap()
            .iter()
            .any(|a| a.url == "/api/auth-requests/pending"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// What approving from a session that began from an approval says.
    const APPROVAL_SESSION_WORDS: Error = Error::Refused(
        "this device signed in with another device's approval, so it can't approve another device's sign-in. To approve one here, sign out, then sign in with your master password",
    );
}
