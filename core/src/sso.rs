//! Single sign-on in the apps, the way RFC 8252 ("OAuth 2.0 for Native
//! Apps") has a native app do it (DESIGN.md in the meta repo, "Sign in with
//! Microsoft Entra ID", "In the desktop and phone apps"; docs/core.md,
//! "Single sign-on"): the authorization code flow with PKCE (S256) and a
//! fresh state, in the person's own browser, back to an address only this
//! app takes. On a computer that's a listener on `127.0.0.1`, on a port the
//! OS picks (`Listener`); on a phone, the app's private-use scheme
//! (`PHONE_REDIRECT`), which the phone's own code hands back as a link.
//!
//! This module makes the sign-in's secrets and its authorize address, and
//! takes the answer back, checking its state. Redeeming the code, two-step
//! login, and the master password that follows are server.rs's and
//! app.rs's: single sign-on signs in, the master password unlocks.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::{rngs::OsRng, RngCore};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::error::{Error, Result};

/// The phones' redirect: a private-use scheme named after the app's own
/// reverse domain name (RFC 8252, 7.1), which is its Android application id
/// and iOS bundle id. The server takes exactly this for `mobile`.
pub const PHONE_REDIRECT: &str = "com.passbundle.app:/sso";

/// The scheme alone, for the phones' own code (iOS's sign-in sheet, Android's
/// intent filter).
pub const PHONE_SCHEME: &str = "com.passbundle.app";

/// The path the computer's listener answers at: the server takes exactly
/// `http://127.0.0.1:<port>/sso` for `desktop`.
pub const LISTENER_PATH: &str = "/sso";

/// How long a sign-in waits for its answer: the server's state lasts ten minutes.
pub const WAIT: Duration = Duration::from_secs(10 * 60);

/// The most of a request the listener reads.
const MOST_READ: usize = 8 * 1024;

/// How long the listener waits for a browser's request once it connects.
const READ_TIME: Duration = Duration::from_secs(3);

/// How often the listener looks for a connection, and for Cancel.
const POLL: Duration = Duration::from_millis(50);

// What the screens say, as the core's errors (`Error::Sso`).
pub(crate) const EMAIL_FIRST: &str = "Enter your email first, then choose Use single sign-on.";
pub(crate) const NO_IDENTIFIER: &str =
    "Enter your organization's SSO identifier. Your administrator can tell you what it is.";
pub(crate) const NOT_AN_IDENTIFIER: &str =
    "An SSO identifier is 3 to 40 lowercase letters, digits, and hyphens. Check it with your administrator.";
pub(crate) const NO_SUCH_ORGANIZATION: &str =
    "No organization uses that SSO identifier, or its single sign-on is turned off. Check it with your administrator.";
pub(crate) const CANCELED: &str = "Single sign-on wasn't finished. Try again when you're ready.";
pub(crate) const TIMED_OUT: &str =
    "Single sign-on wasn't finished within 10 minutes, so PassBundle stopped waiting. Try again.";
pub(crate) const ANOTHER_SIGN_IN: &str =
    "That answer was for another sign-in. Use single sign-on again.";
pub(crate) const NO_ANSWER: &str =
    "Your server's answer didn't have the sign-in in it. Use single sign-on again.";
pub(crate) const NOT_STARTED: &str = "That sign-in was given up. Use single sign-on again.";
pub(crate) const NO_LISTENER: &str =
    "PassBundle couldn't get ready for your browser's answer on this computer. Try again.";

/// Cancel for the computer's wait (`Listener::wait`), from any thread: the
/// person chose Cancel while the browser was out. A new sign-in clears it.
static CANCEL: AtomicBool = AtomicBool::new(false);

/// Stops the computer's wait for the browser (`Listener::wait`).
pub fn cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

/// The flag `cancel` sets, for the wait.
pub fn cancel_flag() -> &'static AtomicBool {
    &CANCEL
}

/// The organization's SSO identifier as people type it, as the server keeps
/// them: 3 to 40 lowercase letters, digits, and hyphens, not starting or
/// ending with a hyphen. Nothing else goes in the authorize address.
pub fn identifier(typed: &str) -> Result<String> {
    let id = typed.trim().to_lowercase();
    if id.is_empty() {
        return Err(Error::Sso(NO_IDENTIFIER));
    }
    let ok = (3..=40).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-');
    if ok {
        Ok(id)
    } else {
        Err(Error::Sso(NOT_AN_IDENTIFIER))
    }
}

/// 32 random bytes, base64url without padding (43 characters): a PKCE
/// verifier (RFC 7636 asks for 43 to 128 of its unreserved characters), or
/// a state.
fn random_value() -> Zeroizing<String> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    OsRng.fill_bytes(&mut *bytes);
    Zeroizing::new(URL_SAFE_NO_PAD.encode(*bytes))
}

/// The S256 challenge for a PKCE verifier (RFC 7636, 4.2): base64url of its
/// SHA-256, without padding.
pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Percent-encodes a value for a URL's query: everything but RFC 3986's
/// unreserved characters.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A query value decoded: `%XX` escapes, and `+` as a space. None for an
/// escape that isn't one, or what isn't UTF-8.
fn decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = value.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// A query value decoded, for tests elsewhere in the core.
#[cfg(test)]
pub(crate) fn tests_decode(value: &str) -> String {
    decode(value).unwrap_or_default()
}

/// The first value of `name` in a query (without its `?`), decoded.
fn query_value(query: &str, name: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('=').or(Some((pair, ""))))
        .find(|(key, _)| decode(key).as_deref() == Some(name))
        .and_then(|(_, value)| decode(value))
}

/// Compares two values in time that doesn't depend on where they differ.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |differ, (x, y)| differ | (x ^ y))
            == 0
}

/// The code in an answer's query, if it's this sign-in's (its state, in
/// constant time) and has one. Another state, or none, is `ANOTHER_SIGN_IN`.
fn answer(query: &str, state: &str) -> Result<Zeroizing<String>> {
    let given = query_value(query, "state").map(Zeroizing::new);
    if !given.is_some_and(|given| same(&given, state)) {
        return Err(Error::Sso(ANOTHER_SIGN_IN));
    }
    // The server always answers with a code and says what went wrong when
    // it's redeemed; an error here came from elsewhere.
    if query_value(query, "error").is_some() {
        return Err(Error::Sso(NO_ANSWER));
    }
    match query_value(query, "code").map(Zeroizing::new) {
        Some(code) if !code.is_empty() && code.len() <= 1024 => Ok(code),
        _ => Err(Error::Sso(NO_ANSWER)),
    }
}

/// A sign-in under way: its PKCE verifier and state, the redirect it asked
/// for, and on a computer the listener. In memory only, until its answer
/// comes back (or it's given up).
pub struct Started {
    identifier: String,
    state: Zeroizing<String>,
    verifier: Zeroizing<String>,
    redirect_uri: String,
    listener: Option<Listener>,
    /// When its wait ends (`WAIT`).
    pub(crate) until: Instant,
}

/// What redeeming a sign-in's code needs (server.rs): the code, the PKCE
/// verifier, and the redirect the sign-in asked for.
pub struct Redeem {
    pub(crate) code: Zeroizing<String>,
    pub(crate) verifier: Zeroizing<String>,
    pub(crate) redirect_uri: String,
}

impl Started {
    /// A sign-in for this device: on a computer, with its listener bound
    /// first; on a phone, back to the app's scheme.
    pub fn here(identifier: &str) -> Result<Started> {
        if cfg!(phone) {
            Ok(Started::on_a_phone(identifier))
        } else {
            Started::on_this_computer(identifier)
        }
    }

    /// A sign-in that comes back to a listener on this computer, bound now.
    pub fn on_this_computer(identifier: &str) -> Result<Started> {
        let listener = Listener::bind()?;
        Ok(Started::with(
            identifier,
            random_value(),
            random_value(),
            Some(listener),
        ))
    }

    /// A sign-in that comes back to the phones' scheme (`PHONE_REDIRECT`).
    pub fn on_a_phone(identifier: &str) -> Started {
        Started::with(identifier, random_value(), random_value(), None)
    }

    /// A sign-in with these secrets: back to `listener` if there's one,
    /// otherwise to the phones' scheme.
    pub(crate) fn with(
        identifier: &str,
        verifier: Zeroizing<String>,
        state: Zeroizing<String>,
        listener: Option<Listener>,
    ) -> Started {
        // A new sign-in: a Cancel from before isn't for it.
        CANCEL.store(false, Ordering::SeqCst);
        let redirect_uri = listener
            .as_ref()
            .map_or_else(|| PHONE_REDIRECT.to_owned(), Listener::redirect_uri);
        Started {
            identifier: identifier.to_owned(),
            state,
            verifier,
            redirect_uri,
            listener,
            until: Instant::now() + WAIT,
        }
    }

    /// Whether the answer comes back to a listener on this computer
    /// (`take_listener`), rather than as a link the phone hands back.
    pub fn listens(&self) -> bool {
        self.listener.is_some()
    }

    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    pub fn state(&self) -> &str {
        &self.state
    }

    /// The address the browser opens: the server's authorize endpoint with
    /// this app's client (`desktop` or `mobile`), redirect, state, and S256
    /// challenge, and the organization's identifier as `domain_hint`, each
    /// percent-encoded. `base` is the server's address, as it's signed in
    /// to (no slash at the end).
    pub fn authorize_url(&self, base: &str, client_id: &str) -> String {
        format!(
            "{base}/identity/connect/authorize?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&code_challenge={}&code_challenge_method=S256&response_mode=query&domain_hint={}",
            encode(client_id),
            encode(&self.redirect_uri),
            encode("api offline_access"),
            encode(&self.state),
            challenge(&self.verifier),
            encode(&self.identifier),
        )
    }

    /// The computer's listener, for the wait, which happens without the
    /// app (ffi.rs): once.
    pub fn take_listener(&mut self) -> Option<Listener> {
        self.listener.take()
    }

    /// The code in a phone's link: exactly `com.passbundle.app:/sso?...`,
    /// with this sign-in's state.
    pub fn code_from_link(&self, link: &str) -> Result<Zeroizing<String>> {
        let query = link
            .strip_prefix(PHONE_REDIRECT)
            .and_then(|rest| rest.strip_prefix('?'))
            .filter(|_| link.len() <= 4096)
            .ok_or(Error::Sso(ANOTHER_SIGN_IN))?;
        let query = query.split('#').next().unwrap_or_default();
        answer(query, &self.state)
    }

    /// What redeeming `code` needs: this sign-in is used up.
    pub fn redeem(self, code: Zeroizing<String>) -> Redeem {
        Redeem {
            code,
            verifier: self.verifier,
            redirect_uri: self.redirect_uri,
        }
    }
}

/// The computer's listener for the browser's answer: on `127.0.0.1` only, on
/// a port the OS picked, bound before anything is sent, so it always has its
/// port and nothing off this computer reaches it.
pub struct Listener {
    socket: TcpListener,
    port: u16,
}

impl Listener {
    pub fn bind() -> Result<Listener> {
        let socket =
            TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|_| Error::Sso(NO_LISTENER))?;
        let port = socket
            .local_addr()
            .map_err(|_| Error::Sso(NO_LISTENER))?
            .port();
        socket
            .set_nonblocking(true)
            .map_err(|_| Error::Sso(NO_LISTENER))?;
        Ok(Listener { socket, port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// `http://127.0.0.1:<port>/sso`, as the server takes it for `desktop`.
    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}{LISTENER_PATH}", self.port)
    }

    /// Waits for the browser's answer to the sign-in whose state is `state`:
    /// the first request for `/sso` that carries it. A request for another
    /// path is "not found", and one with another state (or none) is refused
    /// and the wait goes on, so a page that finds the port can neither end
    /// the sign-in nor slip in a code. Until `cancel` is set, or `until`.
    /// The listener closes when it's dropped.
    pub fn wait(
        &self,
        state: &str,
        cancel: &AtomicBool,
        until: Instant,
    ) -> Result<Zeroizing<String>> {
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(Error::Sso(CANCELED));
            }
            if Instant::now() >= until {
                return Err(Error::Sso(TIMED_OUT));
            }
            match self.socket.accept() {
                Ok((stream, _)) => {
                    if let Some(answer) = serve(stream, state) {
                        return answer;
                    }
                }
                // Nothing yet, or a connection that went before it was
                // taken: look again.
                Err(_) => std::thread::sleep(POLL),
            }
        }
    }
}

/// One connection to the listener: its request read and answered. The
/// sign-in's answer once it's this sign-in's; None to go on waiting.
fn serve(mut stream: TcpStream, state: &str) -> Option<Result<Zeroizing<String>>> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(READ_TIME));
    let _ = stream.set_write_timeout(Some(READ_TIME));
    let head = read_head(&mut stream);
    let (status, page, answer) = match head {
        Some(head) => take(&head, state),
        // Too large, or nothing came in time: not the browser's answer.
        None => (400, Page::Refused, None),
    };
    respond(&mut stream, status, page);
    answer
}

/// A request's head, up to its blank line, or None if it's larger than
/// `MOST_READ` or doesn't come in time.
fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut chunk = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        head.extend_from_slice(&chunk[..read]);
        if head.len() > MOST_READ {
            return None;
        }
    }
    String::from_utf8(head).ok()
}

/// What the listener shows the browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    /// The answer was taken: back to PassBundle.
    Done,
    /// Not this sign-in's answer.
    Refused,
    NotFound,
}

/// What the listener does with a request's head (pure, for the tests): its
/// status, its page, and the sign-in's answer if it's this sign-in's.
fn take(head: &str, state: &str) -> (u16, Page, Option<Result<Zeroizing<String>>>) {
    let line = head.lines().next().unwrap_or_default();
    let mut parts = line.split(' ');
    let (method, target) = (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    );
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != LISTENER_PATH {
        return (404, Page::NotFound, None);
    }
    if method != "GET" {
        return (400, Page::Refused, None);
    }
    match answer(query, state) {
        Err(Error::Sso(ANOTHER_SIGN_IN)) => (400, Page::Refused, None),
        taken => (200, Page::Done, Some(taken)),
    }
}

fn respond(stream: &mut TcpStream, status: u16, page: Page) {
    let (reason, body) = match page {
        Page::Done => (
            "OK",
            "<h1>Back to PassBundle</h1><p>Go back to PassBundle to finish signing in. You can close this tab.</p>",
        ),
        Page::Refused => (
            "Bad Request",
            "<h1>Not PassBundle's sign-in</h1><p>PassBundle on this computer isn't waiting for this sign-in, so it didn't take it.</p>",
        ),
        Page::NotFound => ("Not Found", "<h1>Not found</h1>"),
    };
    let page = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>PassBundle</title><style>body{{font-family:system-ui,sans-serif;max-width:36rem;margin:3rem auto;padding:0 1rem;line-height:1.5}}</style></head><body><main>{body}</main></body></html>"
    );
    let answer = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\nConnection: close\r\n\r\n{page}",
        page.len()
    );
    let _ = stream.write_all(answer.as_bytes());
    let _ = stream.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_is_rfc_7636s_s256() {
        // RFC 7636, Appendix B.
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        // And its verifier is the base64url of the appendix's 32 octets.
        let octets: [u8; 32] = [
            116, 24, 223, 180, 151, 153, 224, 37, 79, 250, 96, 125, 216, 173, 187, 186, 22, 212,
            37, 77, 105, 214, 191, 240, 91, 88, 5, 88, 83, 132, 141, 121,
        ];
        assert_eq!(
            URL_SAFE_NO_PAD.encode(octets),
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
        );
    }

    #[test]
    fn verifiers_and_states_are_32_random_bytes_in_base64url() {
        let (a, b) = (random_value(), random_value());
        assert_ne!(*a, *b);
        for value in [&a, &b] {
            assert_eq!(value.len(), 43);
            assert!(value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
            assert_eq!(URL_SAFE_NO_PAD.decode(value.as_bytes()).unwrap().len(), 32);
        }
        let started = Started::on_a_phone("acme");
        assert_eq!(started.state().len(), 43);
        assert_ne!(started.state(), started.verifier.as_str());
    }

    #[test]
    fn identifiers_are_the_servers_shape() {
        assert_eq!(identifier("  Acme-Dental ").unwrap(), "acme-dental");
        assert_eq!(identifier("abc").unwrap(), "abc");
        assert_eq!(identifier(&"a".repeat(40)).unwrap(), "a".repeat(40));
        assert_eq!(identifier(""), Err(Error::Sso(NO_IDENTIFIER)));
        assert_eq!(identifier("   "), Err(Error::Sso(NO_IDENTIFIER)));
        for bad in [
            "ab",
            &"a".repeat(41),
            "-acme",
            "acme-",
            "acme dental",
            "acme_dental",
            "acme&x=1",
            "acmé",
            "acme/../x",
        ] {
            assert_eq!(identifier(bad), Err(Error::Sso(NOT_AN_IDENTIFIER)), "{bad}");
        }
    }

    #[test]
    fn the_authorize_address_has_every_parameter_encoded() {
        let started = Started::with(
            "acme-dental",
            Zeroizing::new("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into()),
            Zeroizing::new("s+t/a=te".into()),
            None,
        );
        assert_eq!(
            started.authorize_url("https://vault.example.test", "mobile"),
            "https://vault.example.test/identity/connect/authorize?client_id=mobile&redirect_uri=com.passbundle.app%3A%2Fsso&response_type=code&scope=api%20offline_access&state=s%2Bt%2Fa%3Dte&code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM&code_challenge_method=S256&response_mode=query&domain_hint=acme-dental"
        );
        let listener = Listener::bind().unwrap();
        let port = listener.port();
        let started = Started::with(
            "acme",
            Zeroizing::new("v".into()),
            Zeroizing::new("s".into()),
            Some(listener),
        );
        assert!(started.listens());
        assert_eq!(
            started.redirect_uri(),
            format!("http://127.0.0.1:{port}/sso")
        );
        assert!(started
            .authorize_url("http://127.0.0.1:8591", "desktop")
            .starts_with(&format!(
                "http://127.0.0.1:8591/identity/connect/authorize?client_id=desktop&redirect_uri=http%3A%2F%2F127.0.0.1%3A{port}%2Fsso&"
            )));
    }

    #[test]
    fn query_values_are_decoded() {
        assert_eq!(
            query_value("a=1&code=x%2By+z&state=s", "code").as_deref(),
            Some("x+y z")
        );
        assert_eq!(query_value("code=1&code=2", "code").as_deref(), Some("1"));
        assert_eq!(query_value("error", "error").as_deref(), Some(""));
        assert_eq!(query_value("state=%zz", "state"), None);
        assert_eq!(query_value("state=%", "state"), None);
        assert_eq!(query_value("", "code"), None);
    }

    fn phone(state: &str) -> Started {
        Started::with(
            "acme",
            Zeroizing::new("v".into()),
            Zeroizing::new(state.into()),
            None,
        )
    }

    #[test]
    fn a_phones_link_is_taken_only_at_its_redirect_with_its_state() {
        let started = phone("the-state");
        assert_eq!(
            *started
                .code_from_link("com.passbundle.app:/sso?code=the-code&state=the-state")
                .unwrap(),
            "the-code"
        );
        assert_eq!(
            *started
                .code_from_link("com.passbundle.app:/sso?state=the-state&code=c%2Bd#frag")
                .unwrap(),
            "c+d"
        );
        for (link, why) in [
            (
                "com.passbundle.app:/sso?code=c&state=another",
                ANOTHER_SIGN_IN,
            ),
            ("com.passbundle.app:/sso?code=c", ANOTHER_SIGN_IN),
            (
                "com.passbundle.app:/sso?code=c&state=the-stat",
                ANOTHER_SIGN_IN,
            ),
            (
                "com.passbundle.app:/sso?code=c&state=the-statee",
                ANOTHER_SIGN_IN,
            ),
            (
                "com.passbundle.app:/ssox?code=c&state=the-state",
                ANOTHER_SIGN_IN,
            ),
            (
                "com.passbundle.app://sso?code=c&state=the-state",
                ANOTHER_SIGN_IN,
            ),
            (
                "passbundle://duo-callback?code=c&state=the-state",
                ANOTHER_SIGN_IN,
            ),
            (
                "https://evil.example/sso?code=c&state=the-state",
                ANOTHER_SIGN_IN,
            ),
            ("", ANOTHER_SIGN_IN),
            ("com.passbundle.app:/sso?state=the-state", NO_ANSWER),
            ("com.passbundle.app:/sso?code=&state=the-state", NO_ANSWER),
            (
                "com.passbundle.app:/sso?error=access_denied&state=the-state",
                NO_ANSWER,
            ),
        ] {
            assert_eq!(
                started.code_from_link(link).err(),
                Some(Error::Sso(why)),
                "{link}"
            );
        }
        let long = format!(
            "com.passbundle.app:/sso?code={}&state=the-state",
            "c".repeat(4096)
        );
        assert_eq!(
            started.code_from_link(&long).err(),
            Some(Error::Sso(ANOTHER_SIGN_IN))
        );
    }

    #[test]
    fn the_listener_takes_only_its_own_answer_at_its_own_path() {
        let state = "the-state";
        let request = |line: &str| format!("{line}\r\nHost: 127.0.0.1\r\n\r\n");
        let (status, page, answer) =
            take(&request("GET /sso?code=c1&state=the-state HTTP/1.1"), state);
        assert_eq!((status, page), (200, Page::Done));
        assert_eq!(*answer.unwrap().unwrap(), "c1");
        // Another state, or none: refused, and the wait goes on.
        for line in [
            "GET /sso?code=c1&state=another HTTP/1.1",
            "GET /sso?code=c1 HTTP/1.1",
            "GET /sso HTTP/1.1",
        ] {
            let (status, page, answer) = take(&request(line), state);
            assert_eq!((status, page), (400, Page::Refused), "{line}");
            assert!(answer.is_none(), "{line}");
        }
        // Another path: not found, and the wait goes on.
        for line in [
            "GET /favicon.ico HTTP/1.1",
            "GET / HTTP/1.1",
            "GET /sso/?code=c1&state=the-state HTTP/1.1",
            "GET /SSO?code=c1&state=the-state HTTP/1.1",
            "GET http://127.0.0.1/sso?code=c1&state=the-state HTTP/1.1",
        ] {
            let (status, page, answer) = take(&request(line), state);
            assert_eq!((status, page), (404, Page::NotFound), "{line}");
            assert!(answer.is_none(), "{line}");
        }
        // Only a GET.
        let (status, _, answer) = take(
            &request("POST /sso?code=c1&state=the-state HTTP/1.1"),
            state,
        );
        assert_eq!(status, 400);
        assert!(answer.is_none());
        // This sign-in's state with no code ends the wait, saying so.
        let (status, _, answer) = take(&request("GET /sso?state=the-state HTTP/1.1"), state);
        assert_eq!(status, 200);
        assert_eq!(answer.unwrap().err(), Some(Error::Sso(NO_ANSWER)));
    }

    /// Sends a request to a listener on this computer and reads its answer's
    /// status line.
    fn ask(port: u16, request: &[u8]) -> String {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        stream.write_all(request).unwrap();
        let mut answer = String::new();
        let _ = stream.read_to_string(&mut answer);
        answer.lines().next().unwrap_or_default().to_owned()
    }

    #[test]
    fn the_listener_waits_through_other_requests_for_its_answer_then_closes() {
        let listener = Listener::bind().unwrap();
        let port = listener.port();
        assert!(listener.redirect_uri().starts_with("http://127.0.0.1:"));
        assert!(port >= 1024);
        let cancel = AtomicBool::new(false);
        let browser = std::thread::spawn(move || {
            let mut said = vec![
                ask(port, b"GET /favicon.ico HTTP/1.1\r\n\r\n"),
                ask(port, b"GET /sso?code=forged&state=guessed HTTP/1.1\r\n\r\n"),
                ask(port, &[b'a'; MOST_READ + 10]),
            ];
            said.push(ask(
                port,
                b"GET /sso?code=the-code&state=the-state HTTP/1.1\r\nHost: x\r\n\r\n",
            ));
            said
        });
        let code = listener
            .wait(
                "the-state",
                &cancel,
                Instant::now() + Duration::from_secs(30),
            )
            .unwrap();
        assert_eq!(*code, "the-code");
        let said = browser.join().unwrap();
        assert_eq!(
            said,
            [
                "HTTP/1.1 404 Not Found",
                "HTTP/1.1 400 Bad Request",
                "HTTP/1.1 400 Bad Request",
                "HTTP/1.1 200 OK"
            ]
        );
        // Once it's dropped, nothing listens there.
        drop(listener);
        assert!(TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_err());
    }

    #[test]
    fn the_listener_stops_on_cancel_and_when_its_time_is_up() {
        let listener = Listener::bind().unwrap();
        let cancel = AtomicBool::new(true);
        assert_eq!(
            listener
                .wait("s", &cancel, Instant::now() + Duration::from_secs(30))
                .err(),
            Some(Error::Sso(CANCELED))
        );
        let cancel = AtomicBool::new(false);
        assert_eq!(
            listener.wait("s", &cancel, Instant::now()).err(),
            Some(Error::Sso(TIMED_OUT))
        );
        // Cancel from another thread, while it waits.
        let flag = std::sync::Arc::new(AtomicBool::new(false));
        let setter = flag.clone();
        let stopper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            setter.store(true, Ordering::SeqCst);
        });
        assert_eq!(
            listener
                .wait("s", &flag, Instant::now() + Duration::from_secs(30))
                .err(),
            Some(Error::Sso(CANCELED))
        );
        stopper.join().unwrap();
    }

    #[test]
    fn the_listener_is_on_this_computer_only() {
        let listener = Listener::bind().unwrap();
        let address = listener.socket.local_addr().unwrap();
        assert_eq!(address.ip(), std::net::IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn a_sign_in_is_redeemed_with_its_own_verifier_and_redirect() {
        let started = Started::with(
            "acme",
            Zeroizing::new("the-verifier".into()),
            Zeroizing::new("s".into()),
            None,
        );
        let redeem = started.redeem(Zeroizing::new("the-code".into()));
        assert_eq!(*redeem.code, "the-code");
        assert_eq!(*redeem.verifier, "the-verifier");
        assert_eq!(redeem.redirect_uri, PHONE_REDIRECT);
    }

    #[test]
    fn a_new_sign_in_clears_a_cancel_from_before() {
        cancel();
        assert!(cancel_flag().load(Ordering::SeqCst));
        let _ = Started::on_a_phone("acme");
        assert!(!cancel_flag().load(Ordering::SeqCst));
    }

    #[test]
    fn values_compare_only_when_equal() {
        assert!(same("abc", "abc"));
        assert!(!same("abc", "abd"));
        assert!(!same("abc", "ab"));
        assert!(!same("", "a"));
        assert!(same("", ""));
    }
}
