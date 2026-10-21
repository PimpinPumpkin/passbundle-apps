//! What can go wrong, in words the apps show as they are.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The master password didn't open the user key.
    WrongPassword,
    /// Two-step login's code wasn't right, or has expired.
    WrongCode,
    /// The server asked for KDF settings outside the bounds in docs/core.md.
    WeakKdf(&'static str),
    /// An encrypted value the core won't open, and why.
    Refused(&'static str),
    /// Made by a newer app: a key or format this version doesn't know.
    NewerApp,
    /// Something that should open doesn't: a wrong length, bad padding, not UTF-8.
    Damaged(&'static str),
    /// Data that isn't what the server sends.
    Malformed(String),
    /// A one-time code secret the core can't use, and why.
    Totp(&'static str),
    /// The server couldn't be reached.
    Network(String),
    /// The server answered with an error, in its words.
    Server(String),
    /// Signed out: the session ended and needs the master password again.
    SignedOut,
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::WrongPassword => write!(f, "That master password isn't right."),
            Error::WrongCode => write!(f, "That code isn't right, or it's expired. Try the newest one."),
            Error::WeakKdf(why) => write!(
                f,
                "This account's security settings are weaker than PassBundle allows ({why}). Change them in the web vault, under Settings, Security, Keys."
            ),
            Error::Refused(why) => write!(f, "This was refused: {why}."),
            Error::NewerApp => write!(f, "This was made by a newer PassBundle app. Update this one."),
            Error::Damaged(why) => write!(f, "This is damaged ({why})."),
            Error::Malformed(what) => write!(f, "The server sent something unexpected: {what}."),
            Error::Totp(why) => write!(f, "That one-time code secret can't be used: {why}."),
            Error::Network(why) => write!(f, "Couldn't reach the server: {why}."),
            Error::Server(message) => write!(f, "The server said: {message}"),
            Error::SignedOut => write!(f, "You've been signed out. Sign in again."),
        }
    }
}

impl std::error::Error for Error {}
