//! PassBundle's core: everything the apps do except draw the screen. The
//! design is docs/core.md in the meta repo (PimpinPumpkin/Passbundle), with
//! test vectors in docs/vectors/core-v1.json (a copy is in tests/vectors).

pub mod agent;
pub mod app;
pub mod asset_links;
pub mod autolock;
pub mod clipboard;
pub mod crypto;
pub mod error;
pub mod generator;
pub mod generator_policy;
pub mod health;
pub mod history;
pub mod icons;
pub mod kdf;
pub mod lock_policy;
pub mod managed;
pub mod matching;
pub mod passkey;
pub mod places;
pub mod secrets;
pub mod server;
pub mod signup;
pub mod store;
pub mod synced_settings;
pub mod templates;
pub mod totp;
pub mod vault;

#[cfg(target_os = "android")]
pub mod android;
#[cfg(feature = "ffi")]
pub mod ffi;

#[cfg(test)]
mod vectors;

pub use error::{Error, Result};

/// The brand, from ../brand/brand.json.
pub mod brand {
    include!(concat!(env!("OUT_DIR"), "/brand.rs"));
}

/// This core's version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
