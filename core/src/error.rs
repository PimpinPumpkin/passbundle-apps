//! What can go wrong, in the words the apps show as they are (docs/core.md,
//! "What the apps say when something goes wrong"): plain words and what to
//! do next. Never a library's own text (io, TLS, JSON), a status number on
//! its own, or a server's answer quoted back.

use std::fmt;

/// Why a server couldn't be reached (`Error::Network`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unreached {
    /// Its name led nowhere: this device is offline, or the address is wrong.
    NotFound,
    /// Nothing answered there, or the connection dropped.
    NoAnswer,
    /// It didn't answer in time.
    TimedOut,
    /// Its certificate isn't one this device trusts, so nothing was sent.
    Certificate,
}

/// A server address that can't be used (`Error::BadAddress`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadAddress {
    /// Nothing typed.
    Empty,
    /// Plain HTTP to another computer: only HTTPS goes beyond this one.
    NotHttps,
    /// Not a web address at all.
    NotAnAddress,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The master password didn't open the user key.
    WrongPassword,
    /// Signing in: the server didn't take the email and master password
    /// together. Said the same whether or not there's an account with that
    /// email, as the server answers the same, so it never tells anyone
    /// which emails have accounts.
    WrongSignIn,
    /// The item asks for the master password before its secrets are used
    /// (docs/core.md, "Asking for the master password again").
    MasterPasswordFirst,
    /// Two-step login's code wasn't right, or has expired.
    WrongCode,
    /// The server asked for KDF settings outside the bounds in docs/core.md.
    WeakKdf(&'static str),
    /// An encrypted value the core won't open, or something it won't do,
    /// and why.
    Refused(&'static str),
    /// Made by a newer app: a key or format this version doesn't know.
    NewerApp,
    /// Something that should open doesn't: a wrong length, bad padding, not UTF-8.
    Damaged(&'static str),
    /// Data that isn't what the server sends.
    Malformed(String),
    /// A one-time code secret the core can't use, and why.
    Totp(&'static str),
    /// The server couldn't be reached: its name (and port), and why.
    Network { host: String, why: Unreached },
    /// The server answered with an error, in its words.
    Server(String),
    /// The server answered with an error status, and no words of its own.
    Status(u16),
    /// The server says to slow down (429).
    TooManyTries,
    /// A server address that can't be used, and why (signing in, making an
    /// account, checking a server).
    BadAddress(BadAddress),
    /// Signing in, making an account, or checking a server: something
    /// answers at the address, but not as a PassBundle server does (a web
    /// page, a missing page, an answer that isn't the server's). Its name.
    NotAServer(String),
    /// Signed out: the session ended and needs the master password again.
    SignedOut,
    /// The desktop agent isn't running here (or isn't installed).
    NoAgent,
    /// The desktop agent stopped answering partway.
    AgentStopped,
    /// The clipboard couldn't be used.
    Clipboard,
    /// What PassBundle keeps on this device couldn't be written: `full`
    /// when the device is out of space.
    Disk { full: bool },
    /// The password (and any email code) checked out, but the person's role
    /// requires two-step login (an authenticator app, a security key or
    /// passkey, or Duo) and they haven't set it up: the server gave a setup
    /// session the web vault uses, not a sign-in. Who requires it, and the
    /// web vault's address. See DESIGN.md, "Two-step login for admins".
    SetUpTwoStepFirst {
        required_by: Vec<String>,
        web_vault: String,
    },
    /// Approving a sign-in asks for biometrics or the master password
    /// first, and they weren't given just now (docs/core.md, "Approving a
    /// sign-in").
    ConfirmFirst,
    /// Making an account got as far as the server (`sure`: it said so;
    /// otherwise the connection dropped after the account was sent), then
    /// signing in to it didn't work, and why: sign in, rather than make it
    /// again.
    AccountMade { sure: bool, why: Box<Error> },
    /// Adding an account to a device that has the most it keeps
    /// (accounts.rs, `MOST`).
    TooManyAccounts,
    /// Adding an account that's on this device already under another
    /// email or server: the server says it's the same account (its id).
    AlreadyHere,
    /// Two-step login's second step couldn't go on, in a sentence the app
    /// shows as it is: a security key that wasn't touched or holds none of
    /// the account's credentials, a Duo answer for another sign-in, a code
    /// that isn't a YubiKey's (two_step.rs, security_key).
    TwoStep(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// A call that never got the server's answer, as the person reads it:
    /// which server (`url`'s name and port), and why, from what the HTTP
    /// library says, without its words.
    pub(crate) fn unreached(url: &str, e: &ureq::Error) -> Error {
        Error::Network {
            host: host_of(url),
            why: why_unreached(e),
        }
    }

    /// Whether this device couldn't reach the server: offline, or the
    /// server is down. The apps then say they show this device's copy.
    pub fn offline(&self) -> bool {
        matches!(self, Error::Network { .. })
    }

    /// Whether it's about the server's address rather than the person's
    /// email or password: the sign-in screen shows it under the server
    /// field (docs/core.md, "Checking a server").
    pub fn about_the_server(&self) -> bool {
        matches!(
            self,
            Error::Network { .. } | Error::BadAddress(_) | Error::NotAServer(_)
        )
    }
}

/// A URL's host, and its port when it names one ("vault.example.com:8443"),
/// as the person typed it: without the scheme, a name and password before
/// an `@`, or the path.
pub(crate) fn host_of(url: &str) -> String {
    let rest = url
        .trim()
        .split_once("://")
        .map_or(url.trim(), |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host)
        .to_owned()
}

/// What kind of failure the HTTP library had, by its error, never by
/// showing its words.
fn why_unreached(e: &ureq::Error) -> Unreached {
    use std::io::ErrorKind;
    // The TLS errors, rustls's and the OS verifier's, say which part failed,
    // as their own error or inside an io error from the connection: a
    // certificate the OS doesn't trust (expired, for another name, from an
    // authority it doesn't know) is one this device won't send to.
    let certificate = e.to_string().to_lowercase().contains("certificate");
    match e {
        ureq::Error::HostNotFound => Unreached::NotFound,
        ureq::Error::Timeout(_) => Unreached::TimedOut,
        ureq::Error::Io(io) if matches!(io.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {
            Unreached::TimedOut
        }
        ureq::Error::Io(io) if lookup_failed(io) => Unreached::NotFound,
        _ if certificate => Unreached::Certificate,
        _ => Unreached::NoAnswer,
    }
}

/// Whether looking up a server's name failed, as the standard library says
/// it: "failed to lookup address information" on Unix, and Windows' "no such
/// host" codes.
fn lookup_failed(io: &std::io::Error) -> bool {
    // WSAHOST_NOT_FOUND, WSATRY_AGAIN, WSANO_RECOVERY, WSANO_DATA.
    matches!(io.raw_os_error(), Some(11001..=11004)) || io.to_string().contains("lookup address")
}

/// A sentence from words that may not end in one.
fn sentence(words: &str) -> String {
    let words = words.trim();
    let mut out: String = match words.chars().next() {
        Some(first) => first.to_uppercase().chain(words.chars().skip(1)).collect(),
        None => return String::new(),
    };
    if !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
}

/// A refusal's reason, as a sentence. The crypto code's reasons are about
/// "it" (an encrypted value it won't open: "its integrity check failed"),
/// so they say what "it" is and what to do; the others are whole reasons
/// already ("a folder needs a name"), often with what to do.
fn refusal(f: &mut fmt::Formatter<'_>, why: &str) -> fmt::Result {
    let about_it = ["it's ", "its ", "it isn't", "it wasn't"]
        .iter()
        .any(|start| why.starts_with(start));
    if about_it {
        write!(
            f,
            "PassBundle won't open this: {why}. Sync, then try again; if it keeps happening, open it in the web vault."
        )
    } else {
        write!(f, "{}", sentence(why))
    }
}

/// What a status with no words of the server's means, and what to do.
fn status(f: &mut fmt::Formatter<'_>, code: u16) -> fmt::Result {
    match code {
        403 => write!(f, "Your server refused that (error 403). If you should have access, ask whoever runs it."),
        404 => write!(f, "Your server doesn't have that (error 404). It may need an update; if it keeps happening, ask whoever runs it."),
        413 => write!(f, "That's larger than your server takes (error 413)."),
        502..=504 => write!(f, "Your server isn't answering right now (error {code}). Try again in a few minutes."),
        500.. => write!(f, "Your server had a problem (error {code}). Try again in a few minutes."),
        _ => write!(f, "Your server refused that (error {code}). Try again; if it keeps happening, ask whoever runs it."),
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::WrongPassword => write!(f, "That master password isn't right."),
            Error::WrongSignIn => write!(f, "That email or master password isn't right."),
            Error::MasterPasswordFirst => {
                write!(f, "This item asks for your master password first.")
            }
            Error::WrongCode => write!(f, "That code isn't right, or it's expired. Try the newest one."),
            Error::WeakKdf(why) => write!(
                f,
                "This account's security settings are weaker than PassBundle allows ({why}). Change them in the web vault, under Settings, Security, Keys."
            ),
            Error::Refused(why) => refusal(f, why),
            Error::NewerApp => write!(f, "This was made by a newer PassBundle app. Update this one."),
            Error::Damaged(why) => match *why {
                "no such item" => write!(f, "That item isn't here anymore. Sync to see what's there now."),
                "no such folder" => write!(f, "That folder isn't here anymore. Sync to see what's there now."),
                "no such attachment" => write!(f, "That attachment isn't here anymore. Sync to see what's there now."),
                "no such Send" => write!(f, "That Send isn't here anymore. Sync to see what's there now."),
                "no such version" => write!(f, "That version isn't here anymore. Open the history again."),
                "the file couldn't be written" => write!(f, "PassBundle couldn't save that file. Check that this device has free space, then try again."),
                why => write!(f, "Something PassBundle read is damaged ({why}). Sync, then try again; if it keeps happening, open it in the web vault."),
            },
            Error::Malformed(what) => write!(
                f,
                "Your server sent something PassBundle can't read ({what}). It may need an update; if it keeps happening, ask whoever runs it."
            ),
            Error::Totp(why) => write!(f, "That one-time code secret can't be used: {why}. Check it in the item, or set it up again from the site."),
            Error::Network { host, why } => match why {
                Unreached::NotFound => write!(f, "PassBundle couldn't find {host}. Check that this device is online and the address is right."),
                Unreached::NoAnswer => write!(f, "PassBundle couldn't connect to {host}. Check that this device is online, then try again."),
                Unreached::TimedOut => write!(f, "PassBundle gave up waiting for {host}. Check that this device is online, then try again."),
                Unreached::Certificate => write!(f, "PassBundle didn't connect to {host}: its certificate isn't trusted on this device. If it's your organization's server, ask your IT team."),
            },
            Error::Server(message) => write!(f, "Your server said: {}", sentence(message)),
            Error::Status(code) => status(f, *code),
            Error::TooManyTries => write!(f, "Too many tries. Wait a minute, then try again."),
            Error::BadAddress(BadAddress::Empty) => write!(f, "Enter your server's address, like vault.example.com."),
            Error::BadAddress(BadAddress::NotHttps) => write!(f, "Use an address that starts with https://. PassBundle talks to other computers only over HTTPS."),
            Error::BadAddress(BadAddress::NotAnAddress) => write!(f, "That isn't a server's address. Enter it like vault.example.com."),
            Error::NotAServer(host) => write!(f, "There's no PassBundle server at {host}. Check the address: it's where you open your web vault."),
            Error::SignedOut => write!(f, "You've been signed out. Sign in again."),
            Error::NoAgent => write!(f, "PassBundle's desktop agent isn't running. Quit PassBundle and open it again, which starts it; if that doesn't help, install PassBundle again."),
            Error::AgentStopped => write!(f, "PassBundle's desktop agent stopped answering. Quit PassBundle and open it again."),
            Error::Clipboard => write!(f, "PassBundle couldn't use the clipboard. Try copying again."),
            Error::Disk { full: true } => write!(f, "This device is out of space, so PassBundle couldn't save what it keeps here. Free some space, then try again."),
            Error::Disk { full: false } => write!(f, "PassBundle couldn't write to its folder on this device. Try again; if it keeps happening, quit PassBundle and open it again."),
            Error::SetUpTwoStepFirst { required_by, web_vault } => {
                match required_by.as_slice() {
                    [] => write!(f, "Your role requires two-step login.")?,
                    [one] => write!(f, "{one} requires two-step login for your role.")?,
                    [a, b] => write!(f, "{a} and {b} require two-step login for your role.")?,
                    [rest @ .., last] => write!(f, "{}, and {last} require two-step login for your role.", rest.join(", "))?,
                }
                write!(
                    f,
                    " Set it up in the web vault at {web_vault} (an authenticator app, a security key or passkey, or Duo), then sign in here again."
                )
            }
            Error::ConfirmFirst => write!(f, "Confirm it's you first."),
            Error::AccountMade { sure: true, why } => write!(f, "Your account was made, but signing in to it didn't work: {why} Sign in with it."),
            Error::AccountMade { sure: false, why } => write!(f, "The server may have made your account before the connection dropped: {why} Try signing in with it."),
            Error::TooManyAccounts => write!(f, "PassBundle keeps up to {} accounts on this device. Sign out of one to add another.", crate::accounts::MOST),
            Error::AlreadyHere => write!(f, "That account is in PassBundle on this device already, under another email or server. Sign out of it there first."),
            Error::TwoStep(sentence) => write!(f, "{sentence}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    fn io(kind: std::io::ErrorKind, words: &str) -> ureq::Error {
        ureq::Error::Io(std::io::Error::new(kind, words))
    }

    #[test]
    fn a_server_that_cant_be_reached_is_named_and_says_what_to_do() {
        let url = "https://vault.example.test:8443/api/sync?x=1";
        let said = |e: ureq::Error| Error::unreached(url, &e).to_string();
        assert_eq!(
            said(ureq::Error::HostNotFound),
            "PassBundle couldn't find vault.example.test:8443. Check that this device is online and the address is right."
        );
        // The standard library's words for a name that didn't resolve are never shown.
        let lookup = said(io(
            std::io::ErrorKind::Other,
            "failed to lookup address information: nodename nor servname provided, or not known",
        ));
        assert!(
            lookup.starts_with("PassBundle couldn't find vault.example.test:8443."),
            "{lookup}"
        );
        assert!(!lookup.contains("nodename"));
        assert_eq!(
            said(io(std::io::ErrorKind::ConnectionRefused, "Connection refused (os error 61)")),
            "PassBundle couldn't connect to vault.example.test:8443. Check that this device is online, then try again."
        );
        assert_eq!(
            said(io(std::io::ErrorKind::TimedOut, "timed out")),
            "PassBundle gave up waiting for vault.example.test:8443. Check that this device is online, then try again."
        );
        let cert = said(ureq::Error::Tls("invalid peer certificate: UnknownIssuer"));
        assert_eq!(
            cert,
            "PassBundle didn't connect to vault.example.test:8443: its certificate isn't trusted on this device. If it's your organization's server, ask your IT team."
        );
        // The same inside an io error from the connection, as iOS's verifier
        // gives it.
        assert_eq!(
            said(io(
                std::io::ErrorKind::InvalidData,
                "invalid peer certificate: Other(OtherError(\"not trusted\"))"
            )),
            cert
        );
        assert!(Error::unreached(url, &ureq::Error::HostNotFound).offline());
        assert!(Error::unreached(url, &ureq::Error::HostNotFound).about_the_server());
        assert!(!Error::WrongSignIn.about_the_server());
    }

    #[test]
    fn hosts_are_named_as_typed_without_their_paths() {
        assert_eq!(host_of("https://vault.example.test/"), "vault.example.test");
        assert_eq!(host_of("http://127.0.0.1:8591/api"), "127.0.0.1:8591");
        assert_eq!(
            host_of("https://ana:x@vault.example.test#a"),
            "vault.example.test"
        );
        assert_eq!(host_of("vault.example.test"), "vault.example.test");
    }

    #[test]
    fn nothing_shows_a_status_on_its_own_or_a_library_s_words() {
        let words = [
            Error::Status(404),
            Error::Status(500),
            Error::Status(502),
            Error::Status(418),
            Error::TooManyTries,
            Error::Clipboard,
            Error::AgentStopped,
            Error::NoAgent,
            Error::Disk { full: true },
            Error::Disk { full: false },
            Error::NotAServer("example.test".into()),
            Error::BadAddress(BadAddress::Empty),
            Error::BadAddress(BadAddress::NotHttps),
            Error::BadAddress(BadAddress::NotAnAddress),
            Error::Refused("a folder needs a name"),
            Error::Refused("its integrity check failed"),
            Error::Damaged("no such item"),
            Error::Damaged("bad padding"),
            Error::Server("Username or password is incorrect. Try again".into()),
        ]
        .map(|e| e.to_string());
        for w in &words {
            // A sentence, with a capital, ending in a full stop.
            assert!(w.chars().next().unwrap().is_uppercase(), "{w}");
            assert!(w.ends_with('.'), "{w}");
            assert!(!w.contains("This was refused"), "{w}");
            assert!(!w.contains("os error"), "{w}");
            assert_ne!(w, "error 404");
        }
        assert_eq!(words[14], "A folder needs a name.");
        assert!(words[15].starts_with("PassBundle won't open this: its integrity check failed."));
        assert_eq!(
            words[16],
            "That item isn't here anymore. Sync to see what's there now."
        );
        assert_eq!(
            words[18],
            "Your server said: Username or password is incorrect. Try again."
        );
        // A status number is said with what it means.
        assert!(words[0].contains("doesn't have that (error 404)"));
        assert!(words[1].contains("had a problem (error 500)"));
        assert!(words[2].contains("isn't answering right now (error 502)"));
    }

    #[test]
    fn a_wrong_sign_in_never_says_which_half_was_wrong() {
        assert_eq!(
            Error::WrongSignIn.to_string(),
            "That email or master password isn't right."
        );
        // Unlocking knows the email, so it's the master password.
        assert_eq!(
            Error::WrongPassword.to_string(),
            "That master password isn't right."
        );
    }
}
