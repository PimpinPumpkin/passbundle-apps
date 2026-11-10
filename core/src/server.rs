//! The person's PassBundle server: prelogin, signing in (with two-step login
//! and the new-device check), refreshing the session, sync, and saving items
//! (docs/core.md, "Signing in" and "Network"). HTTPS only, except to this
//! computer.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{json, Value};
use ureq::tls::{RootCerts, TlsConfig};
use ureq::Agent;
use zeroize::Zeroizing;

use crate::crypto::{EncString, SymmetricKey};
use crate::error::{BadAddress, Error, NoNewAccounts, Result};
use crate::kdf::{normalize_email, Kdf, MasterKey, Prelogin};
use crate::new_device::{self, Held};
use crate::two_step::Offer;
use crate::vault::Sync;

/// This app, as the server records it.
#[derive(Clone, Debug)]
pub struct Device {
    /// Upstream's device types: 6 Windows, 7 macOS, 8 Linux (desktop), 0 Android, 1 iOS.
    pub kind: u8,
    pub name: String,
    /// Made once per install and kept by the app.
    pub identifier: String,
}

impl Device {
    /// This computer or phone, with the identifier the app keeps.
    pub fn this_one(identifier: &str) -> Device {
        let (kind, os) = if cfg!(target_os = "windows") {
            (6, "Windows")
        } else if cfg!(target_os = "macos") {
            (7, "macOS")
        } else if cfg!(target_os = "android") {
            (0, "Android")
        } else if cfg!(target_os = "ios") {
            (1, "iOS")
        } else {
            (8, "Linux")
        };
        Device {
            kind,
            name: format!("PassBundle on {os}"),
            identifier: identifier.to_owned(),
        }
    }

    /// A new identifier for a new install.
    pub fn new_identifier() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    fn client_id(&self) -> &'static str {
        if self.kind <= 1 {
            "mobile"
        } else {
            "desktop"
        }
    }
}

/// A signed-in session. The user key opens the vault; the tokens reach the server.
pub struct Session {
    pub access_token: Zeroizing<String>,
    /// For the OS's secret store (docs/core.md, "What's kept where").
    pub refresh_token: Zeroizing<String>,
    /// Seconds since 1970 when the access token stops working.
    pub expires_at: u64,
    pub(crate) user_key: SymmetricKey,
    /// What the sign-in gave this device to send back next time (upstream's
    /// `TwoFactorToken`): the new-device check's token, or two-step login's
    /// "Don't ask again". The app keeps it in the OS's secret store.
    pub(crate) device_token: Option<Zeroizing<String>>,
}

impl Session {
    pub fn user_key(&self) -> &SymmetricKey {
        &self.user_key
    }

    /// The device token this sign-in gave, if any (`device_token`).
    pub fn device_token(&self) -> Option<&str> {
        self.device_token.as_deref().map(String::as_str)
    }
}

/// Two-step login's second step: the method, as the server numbers them
/// (two_step.rs), and its code or answer.
#[derive(Clone, Debug)]
pub struct TwoFactor {
    pub provider: u8,
    pub code: String,
    /// Skip the second step on this device for a while.
    pub remember: bool,
}

/// A sign-in waiting for its second step, or for the new-device check. It
/// keeps what the KDF made, so finishing doesn't derive the master key
/// again.
pub struct PendingSignIn {
    email: String,
    login_hash: Zeroizing<String>,
    stretched: SymmetricKey,
    /// The device token this device was given last time (secrets.rs), sent
    /// as two-step login's "remember" token (provider 5) until the server
    /// answers with two-step login instead.
    remembered: Option<Zeroizing<String>>,
    /// The server answered the device token with two-step login: the token
    /// is no good, and the app forgets it, as upstream's apps do.
    pub(crate) forget_remembered: bool,
    /// The new-device check holding this sign-in, from the server's latest
    /// answer, with this device's token for it; None while two-step login
    /// is what it waits for.
    pub(crate) check: Option<Held>,
    /// What the server offers, from its latest answer.
    pub offer: Offer,
    /// When the server made `offer`: Duo's address and a security key's
    /// challenge in it don't last.
    pub(crate) offered_at: Instant,
    /// Duo's address or the security key's challenge in `offer` was tried:
    /// the server took it, so the next try needs a new one.
    pub(crate) spent: bool,
}

impl PendingSignIn {
    fn renew(&mut self, offer: Offer) {
        self.offer = offer;
        self.offered_at = Instant::now();
        self.spent = false;
    }

    /// The new-device check holding it, as the screens show it.
    pub fn check(&self) -> Option<&new_device::Check> {
        self.check.as_ref().map(|held| &held.check)
    }
}

enum Token {
    Done(Session),
    TwoFactor(Offer),
    NewDevice(Held),
}

pub enum SignIn {
    Done(Session),
    NeedsTwoFactor(PendingSignIn),
    /// The new-device check holds it (`PendingSignIn::check`): another of
    /// the account's devices approves it, or a code by email.
    NeedsApproval(PendingSignIn),
}

/// Where a waiting sign-in got to after another answer from the server.
pub enum Next {
    Done(Session),
    /// Two-step login: what's offered is in the sign-in's `offer`.
    TwoFactor,
    /// The new-device check: it's in the sign-in's `check`.
    NewDevice,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
    #[serde(rename = "Key")]
    key: Option<String>,
    #[serde(rename = "TwoFactorToken")]
    two_factor_token: Option<String>,
}

/// The largest attachment file upstream's servers take: 500 MB before
/// encryption, so a little more as stored (the server allows 501 MiB).
const MAX_ATTACHMENT: u64 = 501 * 1024 * 1024 + 1;

/// How long an attachment's download may take once it starts: 500 MB at
/// about 2.5 Mbit/s.
const ATTACHMENT_TIME: Duration = Duration::from_secs(30 * 60);

/// An attachment's file to fetch (`Server::attachment_download`).
pub struct Download {
    agent: Agent,
    url: String,
}

impl Download {
    /// The file, still encrypted, up to the largest upstream takes. The
    /// whole download may take up to half an hour, not the 30 seconds other
    /// calls get; connecting and the server's first answer still get those.
    pub fn fetch(&self) -> Result<Vec<u8>> {
        let mut response = self
            .agent
            .get(&self.url)
            .config()
            .timeout_global(None)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_recv_body(Some(ATTACHMENT_TIME))
            .build()
            .call()
            .map_err(|e| Error::unreached(&self.url, &e))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(Error::Status(status));
        }
        response
            .body_mut()
            .with_config()
            .limit(MAX_ATTACHMENT)
            .read_to_vec()
            .map_err(|e| Error::unreached(&self.url, &e))
    }
}

#[derive(Clone)]
pub struct Server {
    base: String,
    agent: Agent,
    device: Device,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// An id the server gave, for a path: letters, digits, and hyphens only (a
/// UUID), so nothing in it can change which path is called.
fn path_id(id: &str) -> Result<&str> {
    if !id.is_empty()
        && id.len() <= 64
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        Ok(id)
    } else {
        Err(Error::Malformed("an id that isn't one".into()))
    }
}

/// Whether a URL may be used: HTTPS anywhere, or plain HTTP to this computer only.
fn allowed(url: &str) -> bool {
    if url.starts_with("https://") {
        return true;
    }
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    // A name and password before an `@` would put "localhost" in front of
    // another host (http://localhost:80@vault.example.test).
    if host.contains('@') {
        return false;
    }
    let host = if host.starts_with('[') {
        host.split(']').next().map(|h| &h[1..])
    } else {
        host.split(':').next()
    };
    matches!(host, Some("localhost" | "127.0.0.1" | "::1"))
}

/// The server's message from an error answer, in the shapes it uses.
/// Whether a refused sign-in is the server's setup session for two-step
/// login (`KeykeeperTwoStepSetup`), and who requires it. The core never uses
/// the session's token: setting it up is the web vault's.
fn setup_two_step_first(body: &Value) -> Option<Vec<String>> {
    body["KeykeeperTwoStepSetup"].as_str()?;
    Some(
        body["KeykeeperTwoStepRequiredBy"]
            .as_array()
            .map(|names| {
                names
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
    )
}

fn message_of(body: &Value) -> Option<String> {
    for key in ["message", "Message", "error_description", "ErrorModel"] {
        match &body[key] {
            Value::String(s) if !s.is_empty() => return Some(s.clone()),
            Value::Object(o) => {
                if let Some(Value::String(s)) = o.get("Message").or_else(|| o.get("message")) {
                    return Some(s.clone());
                }
            }
            _ => {}
        }
    }
    None
}

/// Whether a server takes new accounts, as its public settings say
/// (`/api/config`, `settings.disableUserRegistration`): `Some(false)` when
/// it says it takes none. A server that takes them only from some addresses
/// says it takes them, and refuses the others when asked (`no_new_accounts`).
/// `None` when it doesn't say, as an older server, or says something else.
fn takes_new_accounts(config: &Value) -> Option<bool> {
    config["settings"]["disableUserRegistration"]
        .as_bool()
        .map(|closed| !closed)
}

/// Why a server won't start a new account, when it says which beside its
/// message (`keykeeperNewAccounts`: "closed" or "address"). Without it, or
/// with anything else there, the answer is read as any other refusal.
fn no_new_accounts(refusal: &Value) -> Option<NoNewAccounts> {
    match refusal["keykeeperNewAccounts"].as_str()? {
        "closed" => Some(NoNewAccounts::Closed),
        "address" => Some(NoNewAccounts::Address),
        _ => None,
    }
}

/// The one certificate every connection trusts instead of the OS's, once a
/// test tool that drives the whole app against a throwaway HTTPS server
/// says so (`trust_only_for_tests`). Only in builds with the `test-trust`
/// feature, such as the `interop` example; never the app's.
#[cfg(feature = "test-trust")]
static TEST_TRUST: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();

/// Makes every connection from now on trust this certificate (PEM) instead
/// of the OS's, for a test tool driving the app (`App`) against a throwaway
/// HTTPS server. Only with the `test-trust` feature.
#[cfg(feature = "test-trust")]
pub fn trust_only_for_tests(pem: &[u8]) {
    let _ = TEST_TRUST.set(pem.to_vec());
}

/// The upstream client version whose vault format the core reads (the web
/// vault and extension PassBundle ships, passbundle-clients' base), sent with
/// every call to the server (`Bitwarden-Client-Version`, upstream's wire
/// name). The server holds back what an older client can't read: a client
/// that doesn't say gets no SSH keys at all.
pub const CLIENT_VERSION: &str = "2026.8.0";

/// Says which client version's format the core reads, on every call.
fn client_version(
    mut request: ureq::http::Request<ureq::SendBody>,
    next: ureq::middleware::MiddlewareNext,
) -> std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error> {
    request.headers_mut().insert(
        "Bitwarden-Client-Version",
        ureq::http::HeaderValue::from_static(CLIENT_VERSION),
    );
    next.handle(request)
}

impl Server {
    pub fn new(url: &str, device: Device) -> Result<Server> {
        #[cfg(feature = "test-trust")]
        if let Some(pem) = TEST_TRUST.get() {
            return Self::new_trusting(url, device, pem);
        }
        // The OS's own certificate checks, so an office's root works as in its browser.
        Self::build(url, device, RootCerts::PlatformVerifier)
    }

    /// Trusting one certificate (PEM) instead of the OS's, for tests against a
    /// throwaway HTTPS server. Only in builds with the `test-trust` feature,
    /// such as the `interop` example; never the app's.
    #[cfg(feature = "test-trust")]
    pub fn new_trusting(url: &str, device: Device, pem: &[u8]) -> Result<Server> {
        let cert = ureq::tls::Certificate::from_pem(pem)
            .map_err(|e| Error::Malformed(format!("a certificate the core can't read ({e})")))?;
        Self::build(
            url,
            device,
            RootCerts::Specific(std::sync::Arc::new(vec![cert])),
        )
    }

    fn build(url: &str, device: Device, roots: RootCerts) -> Result<Server> {
        let base = url.trim().trim_end_matches('/').to_owned();
        if base.is_empty() {
            return Err(Error::BadAddress(BadAddress::Empty));
        }
        // A scheme with nothing after it ("https://").
        if url.trim().ends_with("://") {
            return Err(Error::BadAddress(BadAddress::NotAnAddress));
        }
        if !allowed(&base) {
            return Err(Error::BadAddress(
                if base.starts_with("http://") || !base.contains("://") {
                    BadAddress::NotHttps
                } else {
                    BadAddress::NotAnAddress
                },
            ));
        }
        // A name to reach: "https://" alone, or a space in it, isn't one.
        let host = crate::error::host_of(&base);
        if host.is_empty() || host.contains(char::is_whitespace) {
            return Err(Error::BadAddress(BadAddress::NotAnAddress));
        }
        // A server reached over HTTPS is only ever left for HTTPS: no
        // redirect, or link it gives, goes to plain HTTP. (Plain HTTP is
        // only for a server on this computer, `allowed`.)
        let agent = Agent::config_builder()
            .https_only(base.starts_with("https://"))
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent(format!("PassBundle/{}", crate::version()))
            .tls_config(TlsConfig::builder().root_certs(roots).build())
            .middleware(client_version)
            .build()
            .into();
        Ok(Server {
            base,
            agent,
            device,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    /// Reads an answer: the body on success, the server's words otherwise.
    fn answer(
        &self,
        response: std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<(u16, Value)> {
        let mut response = response.map_err(|e| Error::unreached(&self.base, &e))?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|e| Error::unreached(&self.base, &e))?;
        let body = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or(Value::String(text))
        };
        Ok((status, body))
    }

    fn checked(status: u16, body: Value) -> Result<Value> {
        match status {
            200..=299 => Ok(body),
            401 => Err(Error::SignedOut),
            429 => Err(Error::TooManyTries),
            _ => Err(message_of(&body).map_or(Error::Status(status), Error::Server)),
        }
    }

    /// The account's KDF settings: the first thing signing in asks, so an
    /// address that answers, but not as a PassBundle server does (a web
    /// page, a page that isn't there, an answer that isn't the server's),
    /// is said to be one (`NotAServer`), never quoted.
    pub fn prelogin(&self, email: &str) -> Result<Prelogin> {
        let (status, body) = self.answer(
            self.agent
                .post(self.url("/identity/accounts/prelogin"))
                .send_json(json!({ "email": normalize_email(email) })),
        )?;
        // A page that isn't there, or a site that wants its own sign-in: not
        // a PassBundle server. The server's own refusals (its words, too
        // many tries, a problem of its own) say so.
        if matches!(status, 401 | 403 | 404 | 405) {
            return Err(self.not_a_server());
        }
        serde_json::from_value(Self::checked(status, body)?).map_err(|_| self.not_a_server())
    }

    fn not_a_server(&self) -> Error {
        Error::NotAServer(crate::error::host_of(&self.base))
    }

    /// Whether a PassBundle server answers at this address, before
    /// anything is sent to it (docs/core.md, "Checking a server"): its
    /// configuration, which every upstream server gives without signing
    /// in, as JSON with its version. A web page, a missing page, or
    /// anything else is `NotAServer`; a server that can't be reached says
    /// why (`Network`).
    pub fn check(&self) -> Result<()> {
        self.takes_new_accounts().map(|_| ())
    }

    /// `check`, and what the same answer says about new accounts
    /// (`takes_new_accounts`): the sign-in screen offers making one unless
    /// the server says it takes none.
    pub fn takes_new_accounts(&self) -> Result<Option<bool>> {
        let (status, body) = self.answer(
            self.agent
                .get(self.url("/api/config"))
                .config()
                .timeout_global(Some(Duration::from_secs(15)))
                .build()
                .call(),
        )?;
        match Self::checked(status, body) {
            Ok(body) if body["version"].is_string() => Ok(takes_new_accounts(&body)),
            Err(Error::TooManyTries) => Err(Error::TooManyTries),
            Err(Error::Status(code)) if code >= 500 => Err(Error::Status(code)),
            _ => Err(self.not_a_server()),
        }
    }

    /// Signs in with the master password. The KDF settings are checked before
    /// anything is derived; the user key is opened on the device, so a wrong
    /// password fails here as well as on the server.
    pub fn sign_in(&self, email: &str, password: &str) -> Result<SignIn> {
        let prelogin = self.prelogin(email)?;
        self.sign_in_with(&prelogin, email, password)
    }

    /// Signs in with a prelogin answer the app already asked for (and keeps).
    pub fn sign_in_with(&self, prelogin: &Prelogin, email: &str, password: &str) -> Result<SignIn> {
        self.sign_in_remembered(prelogin, email, password, None)
    }

    /// Signs in as `sign_in_with`, sending the device token this device was
    /// given last time (`remembered`), so it isn't new to the account.
    pub fn sign_in_remembered(
        &self,
        prelogin: &Prelogin,
        email: &str,
        password: &str,
        remembered: Option<Zeroizing<String>>,
    ) -> Result<SignIn> {
        let kdf = Kdf::from_prelogin(prelogin)?;
        let master = MasterKey::derive(password, email, &kdf)?;
        let mut pending = PendingSignIn {
            email: normalize_email(email),
            login_hash: Zeroizing::new(master.login_hash(password)),
            stretched: master.stretch(),
            remembered: remembered.filter(|t| !t.trim().is_empty()),
            forget_remembered: false,
            check: None,
            offer: Offer::default(),
            offered_at: Instant::now(),
            spent: false,
        };
        Ok(match self.send_sign_in(&mut pending, None, None)? {
            Next::Done(session) => SignIn::Done(session),
            Next::TwoFactor => SignIn::NeedsTwoFactor(pending),
            Next::NewDevice => SignIn::NeedsApproval(pending),
        })
    }

    /// Finishes a sign-in with its second step. A wrong code leaves `pending`
    /// waiting, for another try without deriving the master key again, with
    /// what the server offers now if it said.
    pub fn finish_sign_in(
        &self,
        pending: &mut PendingSignIn,
        second: &TwoFactor,
    ) -> Result<Session> {
        match self.send_sign_in(pending, Some(second), None)? {
            Next::Done(session) => Ok(session),
            Next::TwoFactor => Err(Error::WrongCode),
            // An account with two-step login isn't asked; one that turned it
            // off meanwhile starts over.
            Next::NewDevice => Err(Error::TwoStep(
                "This sign-in waits for approval from another of your devices now. Start over to see its number.",
            )),
        }
    }

    /// Sends a waiting sign-in again (the same request as signing in), with
    /// its second step, or the new-device check's token or emailed code
    /// (`new_device`, upstream's `newDeviceOtp`), and keeps what the server
    /// answers in `pending`: what two-step login offers now, or the check.
    pub fn send_sign_in(
        &self,
        pending: &mut PendingSignIn,
        second: Option<&TwoFactor>,
        new_device: Option<&str>,
    ) -> Result<Next> {
        if second.is_some_and(|s| {
            matches!(
                s.provider,
                crate::two_step::DUO | crate::two_step::SECURITY_KEY
            )
        }) {
            pending.spent = true;
        }
        match self.token(pending, second, new_device)? {
            Token::Done(session) => {
                pending.check = None;
                Ok(Next::Done(session))
            }
            Token::TwoFactor(offer) => {
                pending.renew(offer);
                pending.check = None;
                Ok(Next::TwoFactor)
            }
            Token::NewDevice(held) => {
                pending.check = Some(held);
                Ok(Next::NewDevice)
            }
        }
    }

    /// How the new-device check holding `pending` stands now (`POST
    /// /api/keykeeper/new-device-check/status`, with its id and this
    /// device's token, never in the address): kept in `pending`. One that's
    /// gone (its time is up) is `Ended`.
    pub fn new_device_status(&self, pending: &mut PendingSignIn) -> Result<()> {
        self.new_device_call(pending, "/api/keykeeper/new-device-check/status")
    }

    /// "Email me a code" for the new-device check holding `pending`: the
    /// server sends one now, or, when the account has devices that can
    /// approve, starts the day's wait ("Can't use your other devices?").
    pub fn new_device_email(&self, pending: &mut PendingSignIn) -> Result<()> {
        self.new_device_call(pending, "/api/keykeeper/new-device-check/email")
    }

    fn new_device_call(&self, pending: &mut PendingSignIn, path: &str) -> Result<()> {
        let held = pending.check.as_mut().ok_or(Error::SignedOut)?;
        let (status, body) = self.answer(
            self.agent
                .post(self.url(path))
                .config()
                .timeout_global(Some(Duration::from_secs(15)))
                .build()
                .send_json(json!({ "id": held.check.id, "token": held.token.as_str() })),
        )?;
        if status == 404 {
            held.check.status = new_device::Status::Ended;
            return Ok(());
        }
        // The server's own words for a code it won't send (three sent
        // already, say), rather than "too many tries".
        if status == 429 {
            if let Some(m) = message_of(&body) {
                return Err(Error::Server(m));
            }
        }
        held.check.update(&Self::checked(status, body)?)
    }

    /// Asks for the account's master password hint (upstream's "Get master
    /// password hint"), with no session: the server emails it, and answers
    /// None, as it does for an email with no account. A server that can't
    /// send email and is set to show hints answers with the hint instead,
    /// in its words ("Your password hint is: ..."), which comes back as it.
    /// Its other refusals keep its words ("This server is not configured to
    /// provide password hints.").
    pub fn password_hint(&self, email: &str) -> Result<Option<String>> {
        const SHOWN: &str = "Your password hint is: ";
        let (status, body) = self.answer(
            self.agent
                .post(self.url("/api/accounts/password-hint"))
                .send_json(json!({ "email": normalize_email(email) })),
        )?;
        match Self::checked(status, body) {
            Ok(_) => Ok(None),
            Err(Error::Server(m)) if m.starts_with(SHOWN) => Ok(Some(m[SHOWN.len()..].to_owned())),
            Err(e) => Err(e),
        }
    }

    /// Asks the server what it offers again, for a new Duo address or
    /// security key challenge: the same request as signing in, without a
    /// second step. If two-step login went off meanwhile, that's the
    /// session.
    pub fn renew_offer(&self, pending: &mut PendingSignIn) -> Result<Option<Session>> {
        match self.send_sign_in(pending, None, None)? {
            Next::Done(session) => Ok(Some(session)),
            Next::TwoFactor | Next::NewDevice => Ok(None),
        }
    }

    /// The server's address, as the app signs in to it.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Asks the server to email a sign-in code (provider 1).
    pub fn send_email_code(&self, pending: &PendingSignIn) -> Result<()> {
        let (status, body) = self.answer(
            self.agent
                .post(self.url("/api/two-factor/send-email-login"))
                .send_json(json!({
                    "email": pending.email,
                    "masterPasswordHash": pending.login_hash.as_str(),
                })),
        )?;
        Self::checked(status, body).map(|_| ())
    }

    fn token(
        &self,
        pending: &mut PendingSignIn,
        second: Option<&TwoFactor>,
        new_device: Option<&str>,
    ) -> Result<Token> {
        let kind = self.device.kind.to_string();
        let mut form: Vec<(&str, &str)> = vec![
            ("grant_type", "password"),
            ("username", pending.email.as_str()),
            ("password", pending.login_hash.as_str()),
            ("scope", "api offline_access"),
            ("client_id", self.device.client_id()),
            ("deviceType", kind.as_str()),
            ("deviceIdentifier", self.device.identifier.as_str()),
            ("deviceName", self.device.name.as_str()),
        ];
        let provider;
        // The device token, as two-step login's "remember" token, where
        // upstream's apps send it: so this device isn't new to the account
        // (the new-device check), and two-step login's "Don't ask again"
        // holds.
        let remembered = second.is_none() && pending.remembered.is_some();
        if let Some(second) = second {
            provider = second.provider.to_string();
            form.push(("twoFactorProvider", provider.as_str()));
            form.push(("twoFactorToken", second.code.trim()));
            form.push(("twoFactorRemember", if second.remember { "1" } else { "0" }));
        } else if let Some(token) = &pending.remembered {
            form.push(("twoFactorProvider", "5"));
            form.push(("twoFactorToken", token.as_str()));
            form.push(("twoFactorRemember", "0"));
        }
        if let Some(otp) = new_device {
            form.push(("newDeviceOtp", otp));
        }
        let (status, body) = self.answer(
            self.agent
                .post(self.url("/identity/connect/token"))
                .send_form(form),
        )?;
        if status == 400 {
            // The password checked out, but this device is new to the
            // account: the new-device check holds the sign-in until it's
            // approved. Before two-step login's offer and the code check
            // below, which would call it a wrong code.
            if let Some(held) = new_device::held(&body) {
                return Ok(Token::NewDevice(held));
            }
            // The password (and any email code) checked out, but the role
            // requires two-step login, which isn't set up: a setup session
            // for the web vault, not a sign-in. Before the code check below,
            // which would call it a wrong code.
            if let Some(required_by) = setup_two_step_first(&body) {
                return Err(Error::SetUpTwoStepFirst {
                    required_by,
                    web_vault: self.base.clone(),
                });
            }
            if let Some(offer) = Offer::read(&body, &self.base) {
                // The device token was sent, and the server asks for two-step
                // login instead: it's no good anymore.
                if remembered {
                    pending.remembered = None;
                    pending.forget_remembered = true;
                }
                return Ok(Token::TwoFactor(offer));
            }
            // The server's words for a wrong email or password, or code,
            // become the core's; any other refusal (a disabled account, say)
            // keeps the server's. The server answers a wrong password and an
            // email with no account the same ("Username or password is
            // incorrect"), and so does the core: never which one it was.
            return Err(match message_of(&body) {
                Some(m) if m.to_lowercase().contains("password is incorrect") => Error::WrongSignIn,
                _ if second.is_some() => Error::WrongCode,
                Some(m) => new_device::refusal(&m).unwrap_or(Error::Server(m)),
                None => Error::WrongSignIn,
            });
        }
        let body = Self::checked(status, body)?;
        let token: TokenResponse = serde_json::from_value(body)
            .map_err(|_| Error::Malformed("a sign-in answer it doesn't know".into()))?;
        let key = EncString::parse(
            token
                .key
                .as_deref()
                .ok_or(Error::Malformed("a sign-in answer with no key".into()))?,
        )?;
        let user_key = match key.decrypt_key(&pending.stretched) {
            Err(Error::Refused(_)) => return Err(Error::WrongSignIn),
            other => other?,
        };
        Ok(Token::Done(Session {
            access_token: Zeroizing::new(token.access_token),
            refresh_token: Zeroizing::new(token.refresh_token.unwrap_or_default()),
            expires_at: now() + token.expires_in,
            user_key,
            device_token: token
                .two_factor_token
                .filter(|t| !t.trim().is_empty())
                .map(Zeroizing::new),
        }))
    }

    /// A new access token from the refresh token.
    pub fn refresh(&self, session: &mut Session) -> Result<()> {
        let (status, body) = self.answer(
            self.agent
                .post(self.url("/identity/connect/token"))
                .send_form([
                    ("grant_type", "refresh_token"),
                    ("client_id", self.device.client_id()),
                    ("refresh_token", session.refresh_token.as_str()),
                ]),
        )?;
        if status == 400 {
            return Err(Error::SignedOut);
        }
        let token: TokenResponse = serde_json::from_value(Self::checked(status, body)?)
            .map_err(|_| Error::Malformed("a session answer it doesn't know".into()))?;
        session.access_token = Zeroizing::new(token.access_token);
        if let Some(refresh) = token.refresh_token {
            session.refresh_token = Zeroizing::new(refresh);
        }
        session.expires_at = now() + token.expires_in;
        Ok(())
    }

    /// Calls the API with the session, refreshing it first when it's about to expire.
    fn call(
        &self,
        session: &mut Session,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let (status, body) = self.call_answered(session, method, path, body)?;
        Self::checked(status, body)
    }

    /// `call`, with the server's status as it answered.
    fn call_answered(
        &self,
        session: &mut Session,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<(u16, Value)> {
        if session.expires_at <= now() + 60 {
            self.refresh(session)?;
        }
        let auth = format!("Bearer {}", session.access_token.as_str());
        let url = self.url(path);
        let response = match (method, body) {
            ("GET", _) => self.agent.get(&url).header("Authorization", &auth).call(),
            ("POST", Some(b)) => self
                .agent
                .post(&url)
                .header("Authorization", &auth)
                .send_json(b),
            ("PUT", Some(b)) => self
                .agent
                .put(&url)
                .header("Authorization", &auth)
                .send_json(b),
            ("PUT", None) => self
                .agent
                .put(&url)
                .header("Authorization", &auth)
                .send_empty(),
            ("DELETE", _) => self
                .agent
                .delete(&url)
                .header("Authorization", &auth)
                .call(),
            _ => unreachable!("only the methods above are used"),
        };
        self.answer(response)
    }

    /// The organizations whose owners and admins see how many of the
    /// person's own passwords are weak, reused, or exposed, by name: those
    /// they've joined that have "Members' own passwords" on (DESIGN.md in the
    /// meta repo). None when the server doesn't have it (an older or other
    /// server).
    pub fn own_health_seen_by(&self, session: &mut Session) -> Result<Option<Vec<String>>> {
        let (status, body) =
            self.call_answered(session, "GET", "/api/keykeeper/own-health/seen-by", None)?;
        if matches!(status, 404 | 405) {
            return Ok(None);
        }
        let body = Self::checked(status, body)?;
        Ok(Some(
            body["data"]
                .as_array()
                .map(|orgs| {
                    orgs.iter()
                        .filter_map(|o| o["name"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
        ))
    }

    /// Tells the server how many of the person's own logins have weak or
    /// reused passwords (health.rs, `OwnCounts`): numbers only, which their
    /// organizations' owners and admins see (DESIGN.md in the meta repo,
    /// "Members' own passwords"). Whether the server kept them (it doesn't
    /// once no organization of theirs has the setting on), or None when the
    /// server doesn't take them (an older or other server).
    pub fn report_own_health(
        &self,
        session: &mut Session,
        counts: &crate::health::OwnCounts,
    ) -> Result<Option<bool>> {
        let body = serde_json::to_value(counts).expect("counts always serialize");
        let (status, body) =
            self.call_answered(session, "PUT", "/api/keykeeper/own-health", Some(body))?;
        if matches!(status, 404 | 405) {
            return Ok(None);
        }
        let body = Self::checked(status, body)?;
        Ok(Some(body["kept"].as_bool() != Some(false)))
    }

    /// The whole vault, as the server sends it, and its text to keep (still encrypted).
    pub fn sync(&self, session: &mut Session) -> Result<(Sync, String)> {
        let body = self.call(session, "GET", "/api/sync?excludeDomains=true", None)?;
        let text = body.to_string();
        Ok((Sync::from_json(&text)?, text))
    }

    /// The account's settings document (synced_settings), still encrypted,
    /// or none saved yet.
    pub fn settings(&self, session: &mut Session) -> Result<Option<String>> {
        let body = self.call(session, "GET", "/api/keykeeper/settings", None)?;
        Ok(body["data"].as_str().map(str::to_owned))
    }

    /// Saves the account's settings document, already encrypted.
    pub fn save_settings(&self, session: &mut Session, data: &str) -> Result<()> {
        self.call(
            session,
            "PUT",
            "/api/keykeeper/settings",
            Some(json!({ "data": data })),
        )
        .map(|_| ())
    }

    /// Saves an item `Vault::encrypt_item` made: new when it has no id.
    pub fn save_item(
        &self,
        session: &mut Session,
        id: Option<&str>,
        encrypted: Value,
        collection_ids: &[String],
    ) -> Result<Value> {
        match id {
            Some(id) => self.call(
                session,
                "PUT",
                &format!("/api/ciphers/{id}"),
                Some(encrypted),
            ),
            None if encrypted["organizationId"].is_string() => self.call(
                session,
                "POST",
                "/api/ciphers/create",
                Some(json!({ "cipher": encrypted, "collectionIds": collection_ids })),
            ),
            None => self.call(session, "POST", "/api/ciphers", Some(encrypted)),
        }
    }

    /// Saves new items to the person's own vault in one request, as
    /// upstream's importers do (`POST /api/ciphers/import`): each already
    /// encrypted by `Vault::encrypt_item`, and `folder_of` putting some of
    /// them (by their place in `ciphers`) in `folders`, each an existing
    /// folder's id (with its encrypted name) or a new folder's encrypted name.
    pub fn import_items(
        &self,
        session: &mut Session,
        ciphers: &[Value],
        folders: &[(Option<String>, String)],
        folder_of: &[(usize, usize)],
    ) -> Result<()> {
        let folders: Vec<Value> = folders
            .iter()
            .map(|(id, name)| json!({ "id": id, "name": name }))
            .collect();
        let relationships: Vec<Value> = folder_of
            .iter()
            .map(|(item, folder)| json!({ "key": item, "value": folder }))
            .collect();
        self.call(
            session,
            "POST",
            "/api/ciphers/import",
            Some(json!({
                "ciphers": ciphers,
                "folders": folders,
                "folderRelationships": relationships,
            })),
        )
        .map(|_| ())
    }

    /// Puts an organization's item in these of its shared folders (and out
    /// of the others the person reaches), as upstream's apps do.
    pub fn set_item_collections(
        &self,
        session: &mut Session,
        id: &str,
        collection_ids: &[String],
    ) -> Result<()> {
        self.call(
            session,
            "PUT",
            &format!("/api/ciphers/{id}/collections_v2"),
            Some(json!({ "collectionIds": collection_ids })),
        )
        .map(|_| ())
    }

    /// Favorites an item, or not, and leaves its folder as it is: only the
    /// person's own marks, so an item they can't edit takes it too, and
    /// nothing is encrypted again (upstream's partial update).
    pub fn set_favorite(
        &self,
        session: &mut Session,
        id: &str,
        folder_id: Option<&str>,
        favorite: bool,
    ) -> Result<()> {
        self.call(
            session,
            "PUT",
            &format!("/api/ciphers/{id}/partial"),
            Some(json!({ "folderId": folder_id, "favorite": favorite })),
        )
        .map(|_| ())
    }

    /// An item's past versions (passbundle-server, keykeeper_cipher_history.rs):
    /// `{hidePasswords, savedBy, data: [{id, revisionDate, savedBy, cipher}]}`,
    /// newest first, each version's `cipher` encrypted as the item is.
    pub fn item_history(&self, session: &mut Session, id: &str) -> Result<Value> {
        self.call(
            session,
            "GET",
            &format!("/api/keykeeper/ciphers/{id}/history"),
            None,
        )
    }

    /// Clears an item's history, or deletes one of its versions.
    pub fn delete_history(
        &self,
        session: &mut Session,
        id: &str,
        version: Option<&str>,
    ) -> Result<()> {
        let path = match version {
            Some(v) => format!("/api/keykeeper/ciphers/{id}/history/{v}"),
            None => format!("/api/keykeeper/ciphers/{id}/history"),
        };
        self.call(session, "DELETE", &path, None).map(|_| ())
    }

    /// A website's icon from the server's icon service (`<server>/icons/<host>/icon.png`),
    /// as the web vault shows it: no session, at most 512 KB and 10 seconds,
    /// and None where there's none. The server's 19 by 19 fallback counts as
    /// none, and so do a redirect and an image the tiles don't draw or that's
    /// larger than an icon (icons.rs, `acceptable`).
    pub fn icon(&self, host: &str) -> Result<Option<Vec<u8>>> {
        let mut response = self
            .agent
            .get(self.url(&format!("/icons/{host}/icon.png")))
            .config()
            .timeout_global(Some(Duration::from_secs(10)))
            // An icon comes from the server itself: a redirect is none.
            .max_redirects(0)
            .build()
            .call()
            .map_err(|e| Error::unreached(&self.base, &e))?;
        if response.status().as_u16() != 200 {
            return Ok(None);
        }
        let bytes = response
            .body_mut()
            .with_config()
            .limit(512 * 1024)
            .read_to_vec()
            .map_err(|e| Error::unreached(&self.base, &e))?;
        Ok(Some(bytes).filter(|b| !crate::icons::is_fallback(b) && crate::icons::acceptable(b)))
    }

    /// An attachment's file, as the server stores it (still encrypted).
    pub fn attachment(
        &self,
        session: &mut Session,
        cipher_id: &str,
        attachment_id: &str,
    ) -> Result<Vec<u8>> {
        self.attachment_download(session, cipher_id, attachment_id)?
            .fetch()
    }

    /// Where an attachment's file is: the server says, and it's fetched from
    /// there without the session's token, only from the server's own address
    /// or over HTTPS. Fetching it needs nothing else of the server or the
    /// session, so it can happen while the app does other things.
    pub fn attachment_download(
        &self,
        session: &mut Session,
        cipher_id: &str,
        attachment_id: &str,
    ) -> Result<Download> {
        let answer = self.call(
            session,
            "GET",
            &format!("/api/ciphers/{cipher_id}/attachment/{attachment_id}"),
            None,
        )?;
        let url = answer["url"]
            .as_str()
            .ok_or_else(|| Error::Malformed("an attachment answer with no address".into()))?;
        let url = if url.starts_with('/') {
            self.url(url)
        } else {
            url.to_owned()
        };
        if !(url.starts_with(&format!("{}/", self.base)) || url.starts_with("https://")) {
            return Err(Error::Refused("an attachment's address that isn't secure"));
        }
        Ok(Download {
            agent: self.agent.clone(),
            url,
        })
    }

    /// Makes a folder; `name` is already encrypted. Answers its id.
    pub fn create_folder(&self, session: &mut Session, name: &str) -> Result<String> {
        let answer = self.call(
            session,
            "POST",
            "/api/folders",
            Some(json!({ "name": name })),
        )?;
        answer["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| Error::Malformed("a folder answer with no id".into()))
    }

    /// Renames a folder; `name` is already encrypted.
    pub fn rename_folder(&self, session: &mut Session, id: &str, name: &str) -> Result<()> {
        self.call(
            session,
            "PUT",
            &format!("/api/folders/{id}"),
            Some(json!({ "name": name })),
        )
        .map(|_| ())
    }

    /// Deletes a folder; its items stay, in no folder.
    pub fn delete_folder(&self, session: &mut Session, id: &str) -> Result<()> {
        self.call(session, "DELETE", &format!("/api/folders/{id}"), None)
            .map(|_| ())
    }

    /// Moves an item to the trash.
    pub fn trash_item(&self, session: &mut Session, id: &str) -> Result<()> {
        self.call(session, "PUT", &format!("/api/ciphers/{id}/delete"), None)
            .map(|_| ())
    }

    /// Moves several items to the trash in one call (upstream's bulk
    /// delete). The server stops at the first it refuses, with the ones it
    /// took before that in the trash already.
    pub fn trash_items(&self, session: &mut Session, ids: &[String]) -> Result<()> {
        self.call(
            session,
            "PUT",
            "/api/ciphers/delete",
            Some(json!({ "ids": ids })),
        )
        .map(|_| ())
    }

    /// Puts several items in one of the person's folders, or in none, in
    /// one call (upstream's bulk move): only their own filing of each, so
    /// nothing is encrypted again. The server moves the ones the person
    /// has, and says so if that wasn't all of them.
    pub fn move_items(
        &self,
        session: &mut Session,
        ids: &[String],
        folder_id: Option<&str>,
    ) -> Result<()> {
        self.call(
            session,
            "PUT",
            "/api/ciphers/move",
            Some(json!({ "ids": ids, "folderId": folder_id })),
        )
        .map(|_| ())
    }

    /// Sign-in requests waiting for this account's answer (device_approval.rs),
    /// as the server lists them, and the server's clock (its `Date`), so how
    /// long each has left is by the server's time, not this device's. With
    /// a session's access token, so the app needn't be held while it's asked
    /// (`App::sign_in_requests_call`); at most 10 seconds, as the app asks
    /// again soon.
    pub fn pending_sign_in_requests(&self, access_token: &str) -> Result<(Value, Option<u64>)> {
        let response = self
            .agent
            .get(self.url("/api/auth-requests/pending"))
            .header("Authorization", &format!("Bearer {access_token}"))
            .config()
            .timeout_global(Some(Duration::from_secs(10)))
            .build()
            .call();
        let server_now = response
            .as_ref()
            .ok()
            .and_then(|r| r.headers().get("Date"))
            .and_then(|d| d.to_str().ok())
            .and_then(crate::device_approval::parse_http_date);
        let (status, body) = self.answer(response)?;
        Ok((Self::checked(status, body)?, server_now))
    }

    /// One sign-in request, as the server has it now: whether it's still
    /// waiting, and its public key.
    pub fn sign_in_request(&self, session: &mut Session, id: &str) -> Result<Value> {
        self.call(
            session,
            "GET",
            &format!("/api/auth-requests/{}", path_id(id)?),
            None,
        )
    }

    /// Answers a sign-in request from this device: approved with the user
    /// key wrapped to the request's public key, or denied with nothing.
    pub fn answer_sign_in_request(
        &self,
        session: &mut Session,
        id: &str,
        wrapped_user_key: Option<&EncString>,
    ) -> Result<Value> {
        self.call(
            session,
            "PUT",
            &format!("/api/auth-requests/{}", path_id(id)?),
            Some(json!({
                "deviceIdentifier": self.device.identifier,
                "key": wrapped_user_key.map(ToString::to_string),
                "masterPasswordHash": null,
                "requestApproved": wrapped_user_key.is_some(),
            })),
        )
    }

    /// Sign-ins on new devices waiting for this account's approval
    /// (new_device.rs, `GET /api/keykeeper/new-device-checks`), as the
    /// server lists them, never with their numbers. With a session's access
    /// token, as `pending_sign_in_requests`, and at most 10 seconds. A server
    /// without the new-device check (an older one) has none.
    pub fn waiting_new_devices(&self, access_token: &str) -> Result<Value> {
        let response = self
            .agent
            .get(self.url("/api/keykeeper/new-device-checks"))
            .header("Authorization", &format!("Bearer {access_token}"))
            .config()
            .timeout_global(Some(Duration::from_secs(10)))
            .build()
            .call();
        let (status, body) = self.answer(response)?;
        if matches!(status, 404 | 405) {
            return Ok(json!({ "data": [] }));
        }
        Self::checked(status, body)
    }

    /// Approves another device's sign-in with the number typed (`number`,
    /// two digits, checked first), or denies it (None). No key goes with
    /// either: the number, shown only on the new device, is the proof that
    /// the person is at both. A wrong number denies it, at once.
    pub fn answer_new_device(
        &self,
        session: &mut Session,
        id: &str,
        number: Option<&str>,
    ) -> Result<new_device::Answered> {
        let id = path_id(id)?;
        let (path, body) = match number {
            Some(typed) => (
                format!("/api/keykeeper/new-device-checks/{id}/approve"),
                json!({ "number": new_device::typed_number(typed)? }),
            ),
            None => (
                format!("/api/keykeeper/new-device-checks/{id}/deny"),
                json!({}),
            ),
        };
        let (status, body) = self.call_answered(session, "POST", &path, Some(body))?;
        if status == 404 {
            return Err(Error::Refused(
                "that sign-in has ended, or was answered on another device",
            ));
        }
        Self::checked(status, body).map(|body| new_device::Answered::read(&body))
    }

    /// For tests and setup: any call, signed in.
    #[doc(hidden)]
    pub fn raw(
        &self,
        session: &mut Session,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        self.call(session, method, path, body)
    }

    /// Makes the account on the server (docs/core.md, "Making an account"),
    /// as upstream's apps do: asks for a token for the email, then sends the
    /// account with it. A server that emails the token instead (it checks
    /// the address) is refused here: the person finishes from that email.
    /// A server that takes no new accounts, or none for this address,
    /// refuses the first step and says which (`NoNewAccounts`).
    pub fn register(&self, account: &crate::signup::NewAccount) -> Result<()> {
        let (status, body) = self.answer(
            self.agent
                .post(self.url("/identity/accounts/register/send-verification-email"))
                .send_json(json!({
                    "email": account.email,
                    "name": account.name,
                    "receiveMarketingEmails": false,
                })),
        )?;
        if status == 400 {
            if let Some(why) = no_new_accounts(&body) {
                return Err(Error::NoNewAccounts(why));
            }
        }
        let token = Self::checked(status, body)?;
        // A token is a JWT: three base64url parts. Anything else (a web page
        // at a wrong address, say) isn't one, and nothing more is sent.
        let jwt = |t: &&str| {
            let parts: Vec<&str> = t.split('.').collect();
            parts.len() == 3
                && parts.iter().all(|p| {
                    !p.is_empty()
                        && p.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                })
        };
        if token.as_str().is_some_and(|t| !t.is_empty() && !jwt(&t)) {
            return Err(Error::Malformed(
                "not a token where the server should give one (is the address right?)".into(),
            ));
        }
        let Some(token) = token.as_str().filter(jwt) else {
            return Err(Error::Refused(
                "this server finishes new accounts by email, so open the link it sent, then sign in here",
            ));
        };
        self.raw_post(
            "/identity/accounts/register/finish",
            account.finish_body(token),
        )
        // Sent, but no answer: it may be there.
        .map_err(|e| match e {
            Error::Network { .. } => Error::AccountMade {
                sure: false,
                why: Box::new(e),
            },
            e => e,
        })?;
        Ok(())
    }

    /// For tests and setup: an unauthenticated POST.
    #[doc(hidden)]
    pub fn raw_post(&self, path: &str, body: Value) -> Result<Value> {
        let (status, body) = self.answer(self.agent.post(self.url(path)).send_json(body))?;
        Self::checked(status, body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server on this computer that answers each request with the next
    /// of `answers`, and says which paths were asked for.
    fn answering(answers: Vec<Vec<u8>>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let (url, served) = answering_with_bodies(answers);
        let paths = std::thread::spawn(move || {
            served
                .join()
                .unwrap()
                .into_iter()
                .map(|(path, _)| path)
                .collect()
        });
        (url, paths)
    }

    /// `answering`, saying what each request sent too (its body).
    fn answering_with_bodies(
        answers: Vec<Vec<u8>>,
    ) -> (String, std::thread::JoinHandle<Vec<(String, String)>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let served = std::thread::spawn(move || {
            let mut asked = Vec::new();
            for answer in answers {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                    request.push(byte[0]);
                }
                let line = String::from_utf8_lossy(&request);
                let path = line.split(' ').nth(1).unwrap_or_default().to_owned();
                // A request's body (a sign-in's form) is read before the
                // answer, so the connection never closes while it's sent.
                let length = line
                    .lines()
                    .find_map(|h| {
                        let (name, value) = h.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                let mut body = vec![0u8; length];
                let _ = stream.read_exact(&mut body);
                asked.push((path, String::from_utf8_lossy(&body).into_owned()));
                let _ = stream.write_all(&answer);
            }
            asked
        });
        (url, served)
    }

    fn answer(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
        let mut answer = format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        answer.extend_from_slice(body);
        answer
    }

    fn png(side: u32) -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&side.to_be_bytes());
        png.extend_from_slice(&side.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        png
    }

    /// What the server says when a role requires two-step login that isn't
    /// set up: a setup session, not a sign-in.
    const SET_UP_FIRST: &[u8] = br#"{"error":"invalid_grant","error_description":"Acme Dental requires two-step login for your role.","KeykeeperTwoStepSetup":"AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_abcde","KeykeeperTwoStepRequiredBy":["Acme Dental"],"ErrorModel":{"Message":"Acme Dental requires two-step login for your role.","Object":"error"}}"#;

    fn test_prelogin() -> crate::kdf::Prelogin {
        crate::kdf::Prelogin {
            kdf: 0,
            kdf_iterations: 600_000,
            kdf_memory: None,
            kdf_parallelism: None,
        }
    }

    #[test]
    fn a_role_that_needs_two_step_login_is_sent_to_the_web_vault() {
        let json = "Content-Type: application/json\r\n";
        let (url, served) = answering(vec![answer("400 Bad Request", json, SET_UP_FIRST)]);
        let server = Server::new(&url, Device::this_one("")).unwrap();
        let refused = server
            .sign_in_with(&test_prelogin(), "ana@acme.test", "correct horse battery")
            .err();
        assert_eq!(
            refused,
            Some(Error::SetUpTwoStepFirst {
                required_by: vec!["Acme Dental".into()],
                web_vault: url.clone(),
            })
        );
        assert_eq!(served.join().unwrap(), ["/identity/connect/token"]);

        // After an email code that checked out, the same: never a wrong code.
        let asked = br#"{"error":"invalid_grant","TwoFactorProviders":["1"],"TwoFactorProviders2":{"1":null}}"#;
        let (url, _) = answering(vec![
            answer("400 Bad Request", json, asked),
            answer("400 Bad Request", json, SET_UP_FIRST),
        ]);
        let server = Server::new(&url, Device::this_one("")).unwrap();
        let SignIn::NeedsTwoFactor(mut pending) = server
            .sign_in_with(&test_prelogin(), "ana@acme.test", "correct horse battery")
            .unwrap()
        else {
            panic!("an email code should be asked for");
        };
        let code = TwoFactor {
            provider: 1,
            code: "123456".into(),
            remember: false,
        };
        let refused = server.finish_sign_in(&mut pending, &code).err();
        assert_eq!(
            refused,
            Some(Error::SetUpTwoStepFirst {
                required_by: vec!["Acme Dental".into()],
                web_vault: url,
            })
        );
    }

    #[test]
    fn the_message_says_who_requires_it_and_where_to_set_it_up() {
        let message = |names: &[&str]| {
            Error::SetUpTwoStepFirst {
                required_by: names.iter().map(|n| (*n).to_owned()).collect(),
                web_vault: "https://vault.example.test".into(),
            }
            .to_string()
        };
        assert_eq!(
            message(&["Acme Dental"]),
            "Acme Dental requires two-step login for your role. Set it up in the web vault at https://vault.example.test (an authenticator app, a security key or passkey, or Duo), then sign in here again."
        );
        assert!(message(&["Acme Dental", "Contoso IT"])
            .starts_with("Acme Dental and Contoso IT require two-step login for your role."));
        assert!(message(&[])
            .starts_with("Your role requires two-step login. Set it up in the web vault"));
        // Other refusals keep the server's words.
        assert_eq!(
            setup_two_step_first(&json!({ "message": "Username or password is incorrect." })),
            None
        );
    }

    #[test]
    fn an_icon_is_the_servers_own_and_icon_sized() {
        // A redirect, even to the same server, is no icon, and isn't
        // followed: the next request is the next host's.
        let small = png(32);
        let (url, served) = answering(vec![
            answer(
                "302 Found",
                "Location: /icons/elsewhere.example/icon.png\r\n",
                b"",
            ),
            answer("200 OK", "Content-Type: image/png\r\n", &small),
        ]);
        let server = Server::new(&url, Device::this_one("")).unwrap();
        assert_eq!(server.icon("shop.example").unwrap(), None);
        assert_eq!(server.icon("next.example").unwrap(), Some(small.clone()));
        assert_eq!(
            served.join().unwrap(),
            [
                "/icons/shop.example/icon.png",
                "/icons/next.example/icon.png"
            ]
        );

        // A picture far larger than an icon (a small file that would take
        // 576 MB to draw) is none.
        let (url, _) = answering(vec![answer(
            "200 OK",
            "Content-Type: image/png\r\n",
            &png(12000),
        )]);
        let server = Server::new(&url, Device::this_one("")).unwrap();
        assert_eq!(server.icon("big.example").unwrap(), None);
    }

    #[test]
    fn an_address_that_isnt_a_passbundle_server_says_so_and_never_quotes_it() {
        let page = b"<!DOCTYPE html><html><body>Welcome to nginx!</body></html>";
        let html = || answer("200 OK", "Content-Type: text/html\r\n", page);
        let json = "Content-Type: application/json\r\n";
        let not_a_server = |url: &str| Error::NotAServer(url.trim_start_matches("http://").into());

        // Signing in asks for the KDF settings first: a web page there, or
        // a page that isn't, is no PassBundle server.
        for (status, body) in [
            ("200 OK", &page[..]),
            ("404 Not Found", b"Not Found"),
            ("405 Method Not Allowed", b""),
        ] {
            let (url, served) =
                answering(vec![answer(status, "Content-Type: text/html\r\n", body)]);
            let server = Server::new(&url, Device::this_one("")).unwrap();
            let refused = server.prelogin("ana@example.test").unwrap_err();
            assert_eq!(refused, not_a_server(&url), "{status}");
            assert!(refused.about_the_server());
            assert!(!refused.to_string().contains("nginx"), "{refused}");
            assert!(!refused.to_string().contains("DOCTYPE"), "{refused}");
            assert_eq!(served.join().unwrap(), ["/identity/accounts/prelogin"]);
        }

        // Checking a server: its configuration, with its version, before
        // anything is sent to it.
        let config = br#"{"version":"2026.6.0","server":{"name":"PassBundle"}}"#;
        let (url, served) = answering(vec![answer("200 OK", json, config), html()]);
        let server = Server::new(&url, Device::this_one("")).unwrap();
        assert_eq!(server.check(), Ok(()));
        assert_eq!(server.check(), Err(not_a_server(&url)));
        assert_eq!(served.join().unwrap(), ["/api/config", "/api/config"]);

        // A server's own trouble says so, with what to do.
        let (url, _) = answering(vec![answer("503 Service Unavailable", "", b"")]);
        let server = Server::new(&url, Device::this_one("")).unwrap();
        assert_eq!(server.check(), Err(Error::Status(503)));

        // Nothing listening there: it couldn't connect, and it says where.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let closed = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let server = Server::new(&closed, Device::this_one("")).unwrap();
        let refused = server.check().unwrap_err();
        assert_eq!(
            refused,
            Error::Network {
                host: closed.trim_start_matches("http://").into(),
                why: crate::error::Unreached::NoAnswer,
            }
        );
        assert!(refused.about_the_server() && refused.offline());
    }

    #[test]
    fn a_server_says_whether_it_takes_new_accounts_with_its_configuration() {
        let config = |text: &str| serde_json::from_str::<Value>(text).unwrap();
        // It says it takes none.
        assert_eq!(
            takes_new_accounts(&config(
                r#"{"version":"2026.6.0","settings":{"disableUserRegistration":true}}"#
            )),
            Some(false)
        );
        // It says it takes them.
        assert_eq!(
            takes_new_accounts(&config(
                r#"{"version":"2026.6.0","settings":{"disableUserRegistration":false}}"#
            )),
            Some(true)
        );
        // An older server's answer, without its settings, or without that
        // one: it doesn't say.
        for older in [
            r#"{"version":"2026.6.0","server":{"name":"PassBundle"}}"#,
            r#"{"version":"2026.6.0","settings":{}}"#,
            r#"{"version":"2026.6.0","settings":null}"#,
        ] {
            assert_eq!(takes_new_accounts(&config(older)), None, "{older}");
        }
        // Something else where yes or no should be says nothing either.
        for wrong in [
            r#"{"settings":{"disableUserRegistration":"true"}}"#,
            r#"{"settings":{"disableUserRegistration":1}}"#,
            r#"{"settings":{"disableUserRegistration":null}}"#,
            r#"{"settings":{"disableUserRegistration":{"closed":true}}}"#,
            r#"{"settings":[true]}"#,
            r#"{"settings":"disableUserRegistration"}"#,
            r#"{"disableUserRegistration":true}"#,
            r#""closed""#,
            "null",
        ] {
            assert_eq!(takes_new_accounts(&config(wrong)), None, "{wrong}");
        }

        // Checking a server asks once, and the one answer says both: a
        // server that takes no new accounts is a PassBundle server all the
        // same.
        let json = "Content-Type: application/json\r\n";
        let closed = br#"{"version":"2026.6.0","settings":{"disableUserRegistration":true}}"#;
        let open = br#"{"version":"2026.6.0","settings":{"disableUserRegistration":false}}"#;
        let older = br#"{"version":"2026.6.0"}"#;
        let (url, served) = answering(vec![
            answer("200 OK", json, closed),
            answer("200 OK", json, closed),
            answer("200 OK", json, open),
            answer("200 OK", json, older),
            answer(
                "200 OK",
                json,
                br#"{"settings":{"disableUserRegistration":true}}"#,
            ),
            answer("503 Service Unavailable", "", b""),
        ]);
        let server = Server::new(&url, Device::this_one("")).unwrap();
        assert_eq!(server.takes_new_accounts(), Ok(Some(false)));
        assert_eq!(server.check(), Ok(()));
        assert_eq!(server.takes_new_accounts(), Ok(Some(true)));
        assert_eq!(server.takes_new_accounts(), Ok(None));
        // No version: not a PassBundle server, whatever else it says.
        assert_eq!(
            server.takes_new_accounts(),
            Err(Error::NotAServer(url.trim_start_matches("http://").into()))
        );
        assert_eq!(server.takes_new_accounts(), Err(Error::Status(503)));
        assert_eq!(served.join().unwrap(), ["/api/config"; 6]);
    }

    /// A new account as the first step sends it: its email, and no name.
    /// Nothing else of it leaves before the server gives a token.
    fn new_account(email: &str) -> crate::signup::NewAccount {
        let key = SymmetricKey::generate();
        crate::signup::NewAccount {
            email: email.into(),
            name: None,
            hint: None,
            login_hash: Zeroizing::new("not sent at the first step".into()),
            kdf: crate::signup::KDF,
            user_key: key.encrypt(b"user key"),
            public_key: Vec::new(),
            private_key: key.encrypt(b"private key"),
        }
    }

    /// The server's refusal of a new account's first step, as it sends it
    /// (passbundle-server, `new_account_refused`), with `code` beside the
    /// message when there is one.
    fn refusal(message: &str, code: Option<Value>) -> Vec<u8> {
        let mut body = json!({
            "message": message,
            "error": "",
            "error_description": "",
            "validationErrors": { "": [message] },
            "errorModel": { "message": message, "object": "error" },
            "exceptionMessage": null,
            "exceptionStackTrace": null,
            "innerExceptionMessage": null,
            "object": "error",
        });
        if let Some(code) = code {
            body["keykeeperNewAccounts"] = code;
        }
        answer(
            "400 Bad Request",
            "Content-Type: application/json\r\n",
            body.to_string().as_bytes(),
        )
    }

    #[test]
    fn a_refused_new_account_says_why_when_the_server_does() {
        let body = |text: &str| serde_json::from_str::<Value>(text).unwrap();
        assert_eq!(
            no_new_accounts(&body(r#"{"keykeeperNewAccounts":"closed"}"#)),
            Some(NoNewAccounts::Closed)
        );
        assert_eq!(
            no_new_accounts(&body(r#"{"keykeeperNewAccounts":"address"}"#)),
            Some(NoNewAccounts::Address)
        );
        // An older server's refusal has no reason; one this app doesn't
        // know, or something that isn't a reason, is none either.
        for none in [
            r#"{"message":"Registration not allowed or user already exists"}"#,
            r#"{"keykeeperNewAccounts":"later"}"#,
            r#"{"keykeeperNewAccounts":"Closed"}"#,
            r#"{"keykeeperNewAccounts":""}"#,
            r#"{"keykeeperNewAccounts":true}"#,
            r#"{"keykeeperNewAccounts":1}"#,
            r#"{"keykeeperNewAccounts":null}"#,
            r#"{"keykeeperNewAccounts":["closed"]}"#,
            r#"{"keykeeperNewAccounts":{"reason":"closed"}}"#,
            r#"{"errorModel":{"keykeeperNewAccounts":"closed"}}"#,
            r#""closed""#,
            "null",
        ] {
            assert_eq!(no_new_accounts(&body(none)), None, "{none}");
        }

        // Making an account: the first step's refusal says which, and
        // nothing more is sent.
        let account = new_account("ana@example.test");
        for (code, why, words) in [
            (
                "closed",
                NoNewAccounts::Closed,
                "This server isn't taking new accounts.",
            ),
            (
                "address",
                NoNewAccounts::Address,
                "New accounts aren't open for that address.",
            ),
        ] {
            let (url, served) = answering(vec![refusal(words, Some(json!(code)))]);
            let server = Server::new(&url, Device::this_one("")).unwrap();
            let refused = server.register(&account).unwrap_err();
            assert_eq!(refused, Error::NoNewAccounts(why));
            assert_eq!(refused.to_string(), words);
            assert!(!refused.about_the_server() && !refused.offline());
            assert_eq!(
                served.join().unwrap(),
                ["/identity/accounts/register/send-verification-email"]
            );
        }

        // An older server's refusal, a reason this app doesn't know, and
        // one of the wrong kind are said as before: in the server's words.
        for code in [None, Some(json!("later")), Some(json!(true))] {
            let (url, served) = answering(vec![refusal(
                "Registration not allowed or user already exists",
                code.clone(),
            )]);
            let server = Server::new(&url, Device::this_one("")).unwrap();
            assert_eq!(
                server.register(&account),
                Err(Error::Server(
                    "Registration not allowed or user already exists".into()
                )),
                "{code:?}"
            );
            assert_eq!(served.join().unwrap().len(), 1);
        }

        // Only a refusal carries a reason: an answer that isn't one (the
        // server's own trouble) is read as it was.
        let (url, _) = answering(vec![answer(
            "500 Internal Server Error",
            "Content-Type: application/json\r\n",
            br#"{"keykeeperNewAccounts":"closed"}"#,
        )]);
        let server = Server::new(&url, Device::this_one("")).unwrap();
        assert_eq!(server.register(&account), Err(Error::Status(500)));
    }

    #[test]
    fn an_address_that_cant_be_used_says_why() {
        let bad = |url: &str| Server::new(url, Device::this_one("")).err();
        assert_eq!(bad(""), Some(Error::BadAddress(BadAddress::Empty)));
        assert_eq!(bad("   "), Some(Error::BadAddress(BadAddress::Empty)));
        assert_eq!(
            bad("http://vault.example.test"),
            Some(Error::BadAddress(BadAddress::NotHttps))
        );
        assert_eq!(
            bad("vault.example.test"),
            Some(Error::BadAddress(BadAddress::NotHttps))
        );
        assert_eq!(
            bad("ftp://vault.example.test"),
            Some(Error::BadAddress(BadAddress::NotAnAddress))
        );
        assert_eq!(
            bad("https://"),
            Some(Error::BadAddress(BadAddress::NotAnAddress))
        );
        assert_eq!(
            bad("https://vault example"),
            Some(Error::BadAddress(BadAddress::NotAnAddress))
        );
        assert!(bad("https://vault.example.test").is_none());
        assert!(Error::BadAddress(BadAddress::NotHttps).about_the_server());
    }

    #[test]
    fn a_wrong_email_and_a_wrong_password_are_said_the_same() {
        // The server's answer for both: it doesn't say which, and nor does
        // the core, so nobody learns which emails have accounts.
        let wrong = br#"{"error":"invalid_grant","error_description":"Username or password is incorrect. Try again","ErrorModel":{"Message":"Username or password is incorrect. Try again","Object":"error"}}"#;
        let json = "Content-Type: application/json\r\n";
        let (url, _) = answering(vec![
            answer("400 Bad Request", json, wrong),
            answer("400 Bad Request", json, wrong),
            answer("400 Bad Request", json, b"{}"),
        ]);
        let server = Server::new(&url, Device::this_one("")).unwrap();
        let said = |email: &str| {
            server
                .sign_in_with(&test_prelogin(), email, "not the password")
                .err()
        };
        let no_account = said("nobody@example.test");
        let wrong_password = said("ana@example.test");
        let no_words = said("ana@example.test");
        assert_eq!(no_account, Some(Error::WrongSignIn));
        assert_eq!(wrong_password, no_account);
        assert_eq!(no_words, no_account);
        assert_eq!(
            Error::WrongSignIn.to_string(),
            "That email or master password isn't right."
        );
        assert!(!Error::WrongSignIn.about_the_server());
    }

    #[test]
    fn a_master_password_hint_is_emailed_or_shown_as_the_server_says() {
        let json = "Content-Type: application/json\r\n";
        let (url, served) = answering(vec![
            answer("200 OK", json, b""),
            answer(
                "400 Bad Request",
                json,
                br#"{"message":"Your password hint is: the blue door","validationErrors":{"":["Your password hint is: the blue door"]},"object":"error"}"#,
            ),
            answer(
                "400 Bad Request",
                json,
                br#"{"message":"This server is not configured to provide password hints.","object":"error"}"#,
            ),
        ]);
        let server = Server::new(&url, Device::this_one("")).unwrap();
        // Emailed: nothing to show here.
        assert_eq!(server.password_hint("Ana@Example.test"), Ok(None));
        // A server without email that shows hints.
        assert_eq!(
            server.password_hint("ana@example.test"),
            Ok(Some("the blue door".into()))
        );
        // One that gives none: its words.
        assert_eq!(
            server.password_hint("ana@example.test"),
            Err(Error::Server(
                "This server is not configured to provide password hints.".into()
            ))
        );
        assert_eq!(served.join().unwrap(), ["/api/accounts/password-hint"; 3]);
    }

    const PASSWORD: &str = "correct horse battery";
    const CHECK_TOKEN: &str = "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_abcde";
    const JSON: &str = "Content-Type: application/json\r\n";

    /// The server holding a sign-in for the new-device check, with how it
    /// stands (`status`, `email`), as passbundle-server answers.
    fn held_answer(status: &str, email: &str) -> Vec<u8> {
        let body = json!({
            "error": "invalid_grant",
            "error_description": "new device verification required",
            "ErrorModel": { "Message": "new device verification required", "Object": "error" },
            "KeykeeperNewDeviceCheck": {
                "id": "5f0c6f1e-6f3b-4a43-9a52-0d3f1c2b7e11",
                "token": CHECK_TOKEN,
                "number": "47",
                "status": status,
                "deniedWhy": null,
                "expiresAt": "2026-11-08T12:10:00.000000Z",
                "approvers": email != "now",
                "email": email,
                "emailAfter": null,
                "codeSent": false,
                "object": "keykeeperNewDeviceCheck",
            },
        });
        answer("400 Bad Request", JSON, body.to_string().as_bytes())
    }

    /// How a check stands, as the status and email calls answer.
    fn state_answer(status: &str, email: &str, code_sent: bool) -> Vec<u8> {
        let body = json!({
            "id": "5f0c6f1e-6f3b-4a43-9a52-0d3f1c2b7e11",
            "number": "47",
            "status": status,
            "deniedWhy": if status == "denied" { json!("number") } else { Value::Null },
            "expiresAt": "2026-11-08T12:10:00.000000Z",
            "approvers": email != "now",
            "email": email,
            "emailAfter": null,
            "codeSent": code_sent,
            "emailTo": if code_sent { json!("a***@example.test") } else { Value::Null },
            "object": "keykeeperNewDeviceCheck",
        });
        answer("200 OK", JSON, body.to_string().as_bytes())
    }

    /// A sign-in that went through for `email` with `PASSWORD`: its user key
    /// sealed with the stretched master key, as the server keeps it, and
    /// the device token the server gives with it, if any.
    fn signed_in_answer(email: &str, device_token: Option<&str>) -> Vec<u8> {
        let kdf = Kdf::from_prelogin(&test_prelogin()).unwrap();
        let master = MasterKey::derive(PASSWORD, email, &kdf).unwrap();
        let key = master
            .stretch()
            .encrypt(&SymmetricKey::generate().to_bytes())
            .to_string();
        let mut body = json!({
            "access_token": "access",
            "refresh_token": "refresh",
            "expires_in": 3600,
            "Key": key,
        });
        if let Some(token) = device_token {
            body["TwoFactorToken"] = json!(token);
        }
        answer("200 OK", JSON, body.to_string().as_bytes())
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

    #[test]
    fn a_new_device_waits_then_finishes_with_its_token_and_gets_a_device_token() {
        let email = "ana@example.test";
        let (url, served) = answering_with_bodies(vec![
            held_answer("waiting", "later"),
            state_answer("waiting", "later", false),
            state_answer("approved", "later", false),
            signed_in_answer(email, Some("device-token-1")),
        ]);
        let server = Server::new(&url, Device::this_one("this-device")).unwrap();
        let SignIn::NeedsApproval(mut pending) = server
            .sign_in_with(&test_prelogin(), email, PASSWORD)
            .unwrap()
        else {
            panic!("the new-device check should hold it");
        };
        let check = pending.check().unwrap().clone();
        assert_eq!(check.number, "47");
        assert_eq!(check.status, new_device::Status::Waiting);
        assert_eq!(check.email, new_device::Email::Later);

        server.new_device_status(&mut pending).unwrap();
        assert_eq!(pending.check().unwrap().status, new_device::Status::Waiting);
        server.new_device_status(&mut pending).unwrap();
        assert_eq!(
            pending.check().unwrap().status,
            new_device::Status::Approved
        );
        let token = pending.check.as_ref().unwrap().token.clone();
        let Next::Done(session) = server
            .send_sign_in(&mut pending, None, Some(token.as_str()))
            .unwrap()
        else {
            panic!("an approved check should finish the sign-in");
        };
        assert_eq!(session.device_token(), Some("device-token-1"));
        assert!(pending.check().is_none());

        let asked = served.join().unwrap();
        let paths: Vec<&str> = asked.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            paths,
            [
                "/identity/connect/token",
                "/api/keykeeper/new-device-check/status",
                "/api/keykeeper/new-device-check/status",
                "/identity/connect/token",
            ]
        );
        // The first sign-in had no device token to send, and no check.
        let first = form(&asked[0].1);
        assert!(!first.contains_key("twoFactorProvider"));
        assert!(!first.contains_key("newDeviceOtp"));
        assert_eq!(first["deviceIdentifier"], "this-device");
        // Asking how it stands sends the id and this device's token, in the
        // body, never the address.
        let status: Value = serde_json::from_str(&asked[1].1).unwrap();
        assert_eq!(
            status,
            json!({ "id": "5f0c6f1e-6f3b-4a43-9a52-0d3f1c2b7e11", "token": CHECK_TOKEN })
        );
        // Finishing is the same sign-in again, with the check's token.
        let last = form(&asked[3].1);
        assert_eq!(last["newDeviceOtp"], CHECK_TOKEN);
        assert_eq!(last["password"], first["password"]);
        assert_eq!(last["deviceIdentifier"], "this-device");
        assert_eq!(last["deviceType"], first["deviceType"]);
    }

    #[test]
    fn a_device_token_is_sent_as_two_step_logins_remember_token_and_dropped_when_refused() {
        let email = "ana@example.test";
        let two_step = br#"{"error":"invalid_grant","TwoFactorProviders":["0"],"TwoFactorProviders2":{"0":null}}"#;
        let (url, served) = answering_with_bodies(vec![
            signed_in_answer(email, None),
            answer("400 Bad Request", JSON, two_step),
            answer("400 Bad Request", JSON, two_step),
        ]);
        let server = Server::new(&url, Device::this_one("this-device")).unwrap();
        let kept = || Some(Zeroizing::new("device-token-1".to_owned()));
        // Known: straight in, and nothing new to keep.
        let SignIn::Done(session) = server
            .sign_in_remembered(&test_prelogin(), email, PASSWORD, kept())
            .unwrap()
        else {
            panic!("a known device should sign in");
        };
        assert_eq!(session.device_token(), None);
        // The account has two-step login now: the token is no good, and is
        // dropped, so asking for a new offer doesn't send it again.
        let SignIn::NeedsTwoFactor(mut pending) = server
            .sign_in_remembered(&test_prelogin(), email, PASSWORD, kept())
            .unwrap()
        else {
            panic!("two-step login should be asked for");
        };
        assert!(pending.forget_remembered);
        assert!(server.renew_offer(&mut pending).unwrap().is_none());

        let asked = served.join().unwrap();
        for (_, body) in &asked[..2] {
            let sent = form(body);
            assert_eq!(sent["twoFactorProvider"], "5");
            assert_eq!(sent["twoFactorToken"], "device-token-1");
            assert_eq!(sent["twoFactorRemember"], "0");
        }
        assert!(!form(&asked[2].1).contains_key("twoFactorProvider"));
    }

    #[test]
    fn an_email_code_is_sent_or_waited_for_and_a_wrong_one_is_said_plainly() {
        let email = "ana@example.test";
        let wrong = br#"{"error":"invalid_grant","error_description":"That code didn't work. Check the email and try again.","ErrorModel":{"Message":"That code didn't work. Check the email and try again.","Object":"error"}}"#;
        let three = br#"{"message":"A code was sent three times already. Check your email, or sign in again later.","object":"error"}"#;
        let (url, served) = answering_with_bodies(vec![
            held_answer("waiting", "now"),
            state_answer("waiting", "now", true),
            answer("429 Too Many Requests", JSON, three),
            answer("400 Bad Request", JSON, wrong),
            signed_in_answer(email, Some("device-token-2")),
            answer(
                "404 Not Found",
                JSON,
                br#"{"message":"This sign-in has ended. Sign in again."}"#,
            ),
        ]);
        let server = Server::new(&url, Device::this_one("this-device")).unwrap();
        let SignIn::NeedsApproval(mut pending) = server
            .sign_in_with(&test_prelogin(), email, PASSWORD)
            .unwrap()
        else {
            panic!("the new-device check should hold it");
        };
        assert_eq!(pending.check().unwrap().email, new_device::Email::Now);
        server.new_device_email(&mut pending).unwrap();
        assert!(pending.check().unwrap().code_sent);
        assert_eq!(
            pending.check().unwrap().email_to.as_deref(),
            Some("a***@example.test")
        );
        // The server's own words for a code it won't send again.
        assert_eq!(
            server.new_device_email(&mut pending),
            Err(Error::Server(
                "A code was sent three times already. Check your email, or sign in again later."
                    .into()
            ))
        );
        assert_eq!(
            server
                .send_sign_in(&mut pending, None, Some("12345678"))
                .err(),
            Some(Error::TwoStep(
                "That code didn't work. Check the email and try again."
            ))
        );
        // A wrong code leaves it waiting for another try.
        assert!(pending.check().is_some());
        assert!(matches!(
            server.send_sign_in(&mut pending, None, Some("87654321")),
            Ok(Next::Done(_))
        ));
        // A check that's gone, ended.
        let SignIn::NeedsApproval(mut again) = ({
            let (url, _) = answering(vec![held_answer("waiting", "now")]);
            Server::new(&url, Device::this_one("this-device"))
                .unwrap()
                .sign_in_with(&test_prelogin(), email, PASSWORD)
                .unwrap()
        }) else {
            panic!("the new-device check should hold it");
        };
        server.new_device_status(&mut again).unwrap();
        assert_eq!(again.check().unwrap().status, new_device::Status::Ended);

        let asked = served.join().unwrap();
        assert_eq!(asked[1].0, "/api/keykeeper/new-device-check/email");
        assert_eq!(form(&asked[3].1)["newDeviceOtp"], "12345678");
        assert_eq!(form(&asked[4].1)["newDeviceOtp"], "87654321");
    }

    #[test]
    fn approving_sends_only_a_two_digit_number_and_says_what_happened() {
        let (url, served) = answering_with_bodies(vec![
            answer("200 OK", JSON, br#"{"approved":true,"denied":false}"#),
            answer(
                "200 OK",
                JSON,
                br#"{"approved":false,"denied":true,"deniedWhy":"number"}"#,
            ),
            answer(
                "200 OK",
                JSON,
                br#"{"approved":false,"denied":true,"deniedWhy":"denied"}"#,
            ),
            answer(
                "404 Not Found",
                JSON,
                br#"{"message":"This sign-in has ended or was answered"}"#,
            ),
        ]);
        let server = Server::new(&url, Device::this_one("approving")).unwrap();
        let mut session = Session {
            access_token: Zeroizing::new("access".into()),
            refresh_token: Zeroizing::new("refresh".into()),
            expires_at: now() + 3600,
            user_key: SymmetricKey::generate(),
            device_token: None,
        };
        // Nothing is sent for anything but two digits.
        for typed in ["", "4", "470", "ab"] {
            assert_eq!(
                server.answer_new_device(&mut session, "check-1", Some(typed)),
                Err(Error::TwoStep(
                    "Type the two-digit number shown on the new device."
                ))
            );
        }
        assert_eq!(
            server.answer_new_device(&mut session, "check-1", Some(" 47 ")),
            Ok(new_device::Answered::Approved)
        );
        assert_eq!(
            server.answer_new_device(&mut session, "check-2", Some("12")),
            Ok(new_device::Answered::WrongNumber)
        );
        assert_eq!(
            server.answer_new_device(&mut session, "check-3", None),
            Ok(new_device::Answered::Denied)
        );
        assert!(matches!(
            server.answer_new_device(&mut session, "check-4", None),
            Err(Error::Refused(_))
        ));
        let asked = served.join().unwrap();
        assert_eq!(
            asked
                .iter()
                .map(|(p, b)| (p.as_str(), serde_json::from_str::<Value>(b).unwrap()))
                .collect::<Vec<_>>(),
            [
                (
                    "/api/keykeeper/new-device-checks/check-1/approve",
                    json!({ "number": "47" })
                ),
                (
                    "/api/keykeeper/new-device-checks/check-2/approve",
                    json!({ "number": "12" })
                ),
                ("/api/keykeeper/new-device-checks/check-3/deny", json!({})),
                ("/api/keykeeper/new-device-checks/check-4/deny", json!({})),
            ]
        );
    }

    #[test]
    fn https_anywhere_and_plain_http_only_to_this_computer() {
        for ok in [
            "https://vault.example.test",
            "http://localhost:8000",
            "http://127.0.0.1:8591/",
            "http://[::1]:80",
        ] {
            assert!(allowed(ok), "{ok}");
        }
        for refused in [
            "http://vault.example.test",
            "http://127.0.0.1.example.test",
            "ftp://localhost",
            "localhost:8000",
            "http://10.0.0.5",
            "http://localhost:80@vault.example.test",
            "http://localhost@vault.example.test/",
            "http://[::1]@vault.example.test",
        ] {
            assert!(!allowed(refused), "{refused}");
        }
        assert!(Server::new("http://vault.example.test", Device::this_one("x")).is_err());
    }
}
