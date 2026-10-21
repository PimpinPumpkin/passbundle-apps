//! PassBundle's core: everything the apps do except draw the screen. The
//! design is docs/core.md in the meta repo (PimpinPumpkin/Passbundle), with
//! test vectors in docs/vectors/core-v1.json (a copy is in tests/vectors).

pub mod crypto;
pub mod error;
pub mod kdf;
pub mod totp;
pub mod vault;

#[cfg(feature = "ffi")]
pub mod ffi;

#[cfg(test)]
mod vectors;

pub use error::{Error, Result};

/// This core's version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
