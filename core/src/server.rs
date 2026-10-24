//! The person's PassBundle server: prelogin, signing in (with two-step login),
//! refreshing the session, sync, and saving items (docs/core.md, "Signing
//! in" and "Network"). HTTPS only, except to this computer.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{json, Value};
use ureq::tls::{RootCerts, TlsConfig};
use ureq::Agent;
use zeroize::Zeroizing;

use crate::crypto::{EncString, SymmetricKey};
use crate::error::{Error, Result};
use crate::kdf::{normalize_email, Kdf, MasterKey, Prelogin};
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
}

impl Session {
    pub fn user_key(&self) -> &SymmetricKey {
        &self.user_key
    }
}

/// Two-step login's second step: a code from an authenticator app (0) or email (1).
#[derive(Clone, Debug)]
pub struct TwoFactor {
    pub provider: u8,
    pub code: String,
    /// Skip the second step on this device for a while.
    pub remember: bool,
}

/// A sign-in waiting for its second step. It keeps what the KDF made, so
/// finishing doesn't derive the master key again.
pub struct PendingSignIn {
    email: String,
    login_hash: Zeroizing<String>,
    stretched: SymmetricKey,
    /// What the server offers: 0 authenticator, 1 email, and others the core can't do yet.
    pub providers: Vec<u8>,
}

enum Token {
    Done(Session),
    TwoFactor(Vec<u8>),
}

pub enum SignIn {
    Done(Session),
    NeedsTwoFactor(PendingSignIn),
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
    #[serde(rename = "Key")]
    key: Option<String>,
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
            .map_err(|e| Error::Network(e.to_string()))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(Error::Server(format!(
                "the file couldn't be downloaded ({status})"
            )));
        }
        response
            .body_mut()
            .with_config()
            .limit(MAX_ATTACHMENT)
            .read_to_vec()
            .map_err(|e| Error::Network(e.to_string()))
    }
}

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

/// Whether a URL may be used: HTTPS anywhere, or plain HTTP to this computer only.
fn allowed(url: &str) -> bool {
    if url.starts_with("https://") {
        return true;
    }
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = if host.starts_with('[') {
        host.split(']').next().map(|h| &h[1..])
    } else {
        host.split(':').next()
    };
    matches!(host, Some("localhost" | "127.0.0.1" | "::1"))
}

/// The server's message from an error answer, in the shapes it uses.
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
        if !allowed(&base) {
            return Err(Error::Refused(
                "PassBundle only talks to servers over HTTPS",
            ));
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
        response: std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<(u16, Value)> {
        let mut response = response.map_err(|e| Error::Network(e.to_string()))?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|e| Error::Network(e.to_string()))?;
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
            429 => Err(Error::Server(
                "Too many tries. Wait a minute, then try again.".into(),
            )),
            _ => Err(Error::Server(
                message_of(&body).unwrap_or_else(|| format!("error {status}")),
            )),
        }
    }

    pub fn prelogin(&self, email: &str) -> Result<Prelogin> {
        let (status, body) = Self::answer(
            self.agent
                .post(self.url("/identity/accounts/prelogin"))
                .send_json(json!({ "email": normalize_email(email) })),
        )?;
        serde_json::from_value(Self::checked(status, body)?)
            .map_err(|e| Error::Malformed(format!("a prelogin answer the core can't read ({e})")))
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
        let kdf = Kdf::from_prelogin(prelogin)?;
        let master = MasterKey::derive(password, email, &kdf)?;
        let mut pending = PendingSignIn {
            email: normalize_email(email),
            login_hash: Zeroizing::new(master.login_hash(password)),
            stretched: master.stretch(),
            providers: Vec::new(),
        };
        match self.token(&pending, None)? {
            Token::Done(session) => Ok(SignIn::Done(session)),
            Token::TwoFactor(providers) => {
                pending.providers = providers;
                Ok(SignIn::NeedsTwoFactor(pending))
            }
        }
    }

    /// Finishes a sign-in with its second step. A wrong code leaves `pending`
    /// as it was, for another try without deriving the master key again.
    pub fn finish_sign_in(&self, pending: &PendingSignIn, second: &TwoFactor) -> Result<Session> {
        match self.token(pending, Some(second))? {
            Token::Done(session) => Ok(session),
            Token::TwoFactor(_) => Err(Error::WrongCode),
        }
    }

    /// Asks the server to email a sign-in code (provider 1).
    pub fn send_email_code(&self, pending: &PendingSignIn) -> Result<()> {
        let (status, body) = Self::answer(
            self.agent
                .post(self.url("/api/two-factor/send-email-login"))
                .send_json(json!({
                    "email": pending.email,
                    "masterPasswordHash": pending.login_hash.as_str(),
                })),
        )?;
        Self::checked(status, body).map(|_| ())
    }

    fn token(&self, pending: &PendingSignIn, second: Option<&TwoFactor>) -> Result<Token> {
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
        if let Some(second) = second {
            provider = second.provider.to_string();
            form.push(("twoFactorProvider", provider.as_str()));
            form.push(("twoFactorToken", second.code.trim()));
            form.push(("twoFactorRemember", if second.remember { "1" } else { "0" }));
        }
        let (status, body) = Self::answer(
            self.agent
                .post(self.url("/identity/connect/token"))
                .send_form(form),
        )?;
        if status == 400 {
            if let Some(providers) = body["TwoFactorProviders"].as_array() {
                return Ok(Token::TwoFactor(
                    providers
                        .iter()
                        .filter_map(|p| {
                            p.as_str()
                                .and_then(|s| s.parse().ok())
                                .or_else(|| p.as_u64().map(|n| n as u8))
                        })
                        .collect(),
                ));
            }
            // The server's words for a wrong password or code become the
            // core's; any other refusal (a disabled account, say) keeps the server's.
            return Err(match message_of(&body) {
                Some(m) if m.to_lowercase().contains("password is incorrect") => {
                    Error::WrongPassword
                }
                _ if second.is_some() => Error::WrongCode,
                Some(m) => Error::Server(m),
                None => Error::WrongPassword,
            });
        }
        let body = Self::checked(status, body)?;
        let token: TokenResponse = serde_json::from_value(body)
            .map_err(|e| Error::Malformed(format!("a sign-in answer the core can't read ({e})")))?;
        let key = EncString::parse(
            token
                .key
                .as_deref()
                .ok_or(Error::Malformed("a sign-in answer with no key".into()))?,
        )?;
        let user_key = match key.decrypt_key(&pending.stretched) {
            Err(Error::Refused(_)) => return Err(Error::WrongPassword),
            other => other?,
        };
        Ok(Token::Done(Session {
            access_token: Zeroizing::new(token.access_token),
            refresh_token: Zeroizing::new(token.refresh_token.unwrap_or_default()),
            expires_at: now() + token.expires_in,
            user_key,
        }))
    }

    /// A new access token from the refresh token.
    pub fn refresh(&self, session: &mut Session) -> Result<()> {
        let (status, body) = Self::answer(
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
            .map_err(|e| Error::Malformed(format!("a refresh answer the core can't read ({e})")))?;
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
        let (status, body) = Self::answer(response)?;
        Self::checked(status, body)
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
            .map_err(|e| Error::Network(e.to_string()))?;
        if response.status().as_u16() != 200 {
            return Ok(None);
        }
        let bytes = response
            .body_mut()
            .with_config()
            .limit(512 * 1024)
            .read_to_vec()
            .map_err(|e| Error::Network(e.to_string()))?;
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
    pub fn register(&self, account: &crate::signup::NewAccount) -> Result<()> {
        let token = self.raw_post(
            "/identity/accounts/register/send-verification-email",
            json!({
                "email": account.email,
                "name": account.name,
                "receiveMarketingEmails": false,
            }),
        )?;
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
            Error::Network(_) => Error::AccountMade {
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
        let (status, body) = Self::answer(self.agent.post(self.url(path)).send_json(body))?;
        Self::checked(status, body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server on this computer that answers each request with the next
    /// of `answers`, and says which paths were asked for.
    fn answering(answers: Vec<Vec<u8>>) -> (String, std::thread::JoinHandle<Vec<String>>) {
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
                asked.push(line.split(' ').nth(1).unwrap_or_default().to_owned());
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
        ] {
            assert!(!allowed(refused), "{refused}");
        }
        assert!(Server::new("http://vault.example.test", Device::this_one("x")).is_err());
    }
}
