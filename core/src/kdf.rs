//! The master key, the login hash, and the stretched key, from the master
//! password and the server's prelogin answer (docs/core.md, "Keys from the
//! master password"). KDF settings are checked against bounds first.

use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD, Engine};
use hkdf::Hkdf;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::crypto::SymmetricKey;
use crate::error::{Error, Result};

/// The server's answer to `POST /identity/accounts/prelogin`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Prelogin {
    pub kdf: u8,
    pub kdf_iterations: u32,
    pub kdf_memory: Option<u32>,
    pub kdf_parallelism: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kdf {
    Pbkdf2 {
        iterations: u32,
    },
    Argon2id {
        iterations: u32,
        memory_mib: u32,
        parallelism: u32,
    },
}

impl Kdf {
    /// Checks the settings against docs/core.md's bounds: a hacked server could
    /// otherwise ask for weak ones and capture an easy-to-crack login hash.
    pub fn from_prelogin(p: &Prelogin) -> Result<Kdf> {
        match p.kdf {
            0 => match p.kdf_iterations {
                i if i < 600_000 => Err(Error::WeakKdf("PBKDF2 needs at least 600,000 iterations")),
                i if i > 2_000_000 => {
                    Err(Error::WeakKdf("PBKDF2 allows at most 2,000,000 iterations"))
                }
                iterations => Ok(Kdf::Pbkdf2 { iterations }),
            },
            1 => {
                let memory_mib = p.kdf_memory.unwrap_or(0);
                let parallelism = p.kdf_parallelism.unwrap_or(0);
                let iterations = p.kdf_iterations;
                if !(16..=1024).contains(&memory_mib) {
                    Err(Error::WeakKdf("Argon2id needs 16 to 1,024 MiB of memory"))
                } else if !(2..=10).contains(&iterations) {
                    Err(Error::WeakKdf("Argon2id needs 2 to 10 iterations"))
                } else if !(1..=16).contains(&parallelism) {
                    Err(Error::WeakKdf("Argon2id needs a parallelism of 1 to 16"))
                } else {
                    Ok(Kdf::Argon2id {
                        iterations,
                        memory_mib,
                        parallelism,
                    })
                }
            }
            _ => Err(Error::NewerApp),
        }
    }
}

/// The email as the salt: trimmed and lowercased.
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// The 32-byte master key. Never stored, never sent.
pub struct MasterKey(Zeroizing<[u8; 32]>);

impl MasterKey {
    pub fn derive(password: &str, email: &str, kdf: &Kdf) -> Result<MasterKey> {
        let salt = normalize_email(email);
        let mut out = Zeroizing::new([0u8; 32]);
        match *kdf {
            Kdf::Pbkdf2 { iterations } => {
                pbkdf2::pbkdf2_hmac::<Sha256>(
                    password.as_bytes(),
                    salt.as_bytes(),
                    iterations,
                    &mut *out,
                );
            }
            Kdf::Argon2id {
                iterations,
                memory_mib,
                parallelism,
            } => {
                let params = Params::new(memory_mib * 1024, iterations, parallelism, Some(32))
                    .map_err(|_| Error::WeakKdf("Argon2id settings it can't use"))?;
                Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
                    .hash_password_into(
                        password.as_bytes(),
                        &Sha256::digest(salt.as_bytes()),
                        &mut *out,
                    )
                    .map_err(|_| Error::WeakKdf("Argon2id settings it can't use"))?;
            }
        }
        Ok(MasterKey(out))
    }

    /// What signing in sends instead of the password: PBKDF2-SHA256 of the
    /// master key, salted with the password, one iteration, in base64.
    pub fn login_hash(&self, password: &str) -> String {
        let mut out = Zeroizing::new([0u8; 32]);
        pbkdf2::pbkdf2_hmac::<Sha256>(&*self.0, password.as_bytes(), 1, &mut *out);
        STANDARD.encode(*out)
    }

    /// HKDF-Expand-SHA256 with info "enc", then "mac": the key that opens the user key.
    pub fn stretch(&self) -> SymmetricKey {
        let hkdf = Hkdf::<Sha256>::from_prk(&*self.0).expect("a 32-byte PRK is long enough");
        let mut enc = [0u8; 32];
        let mut mac = [0u8; 32];
        hkdf.expand(b"enc", &mut enc)
            .expect("32 bytes is a valid length");
        hkdf.expand(b"mac", &mut mac)
            .expect("32 bytes is a valid length");
        let key = SymmetricKey::from_parts(enc, mac);
        zeroize::Zeroize::zeroize(&mut enc);
        zeroize::Zeroize::zeroize(&mut mac);
        key
    }

    #[cfg(test)]
    pub(crate) fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}
