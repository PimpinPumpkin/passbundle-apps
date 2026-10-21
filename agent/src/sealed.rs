//! Sealing a user key with Windows Hello, as upstream's GPL implementation
//! does it (`desktop_native/biometric/src/windows.rs`), with RustCrypto
//! instead of Bitwarden's SDK crates.
//!
//! Each enrollment gets a random 32-byte challenge. Windows Hello signs it
//! with the person's key credential only after they authenticate, and the
//! signature (RSA PKCS#1 v1.5, so the same every time) goes through
//! HKDF-SHA256, with the challenge as salt, into an AES-256-GCM key. The
//! account's id is bound in as associated data, so one account's sealed key
//! can't be opened as another's. `tests/vectors/windows-hello-seal.json`,
//! made with plain Node, pins it. See DESIGN.md, "Biometric unlock".

use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use hkdf::Hkdf;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroizing;

const INFO: &[u8] = b"passbundle-agent windows-hello seal v1";

/// What the agent stores per enrolled account (DPAPI-wrapped on disk).
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Sealed {
    pub v: u8,
    pub challenge: String,
    nonce: String,
    ciphertext: String,
}

pub fn new_challenge() -> [u8; 32] {
    let mut challenge = [0u8; 32];
    OsRng.fill_bytes(&mut challenge);
    challenge
}

fn key(challenge: &[u8], signature: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut key = Zeroizing::new([0u8; 32]);
    Hkdf::<Sha256>::new(Some(challenge), signature)
        .expand(INFO, key.as_mut())
        .expect("32 bytes is a valid HKDF-SHA256 length");
    key
}

pub fn seal(user_id: &str, challenge: &[u8; 32], signature: &[u8], user_key: &[u8]) -> Sealed {
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    seal_with_nonce(user_id, challenge, signature, user_key, nonce)
}

fn seal_with_nonce(
    user_id: &str,
    challenge: &[u8; 32],
    signature: &[u8],
    user_key: &[u8],
    nonce: [u8; 12],
) -> Sealed {
    let cipher = Aes256Gcm::new(key(challenge, signature).as_ref().into());
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: user_key,
                aad: user_id.as_bytes(),
            },
        )
        .expect("AES-GCM encrypts any length we hand it");
    Sealed {
        v: 1,
        challenge: STANDARD.encode(challenge),
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(ciphertext),
    }
}

impl Sealed {
    pub fn challenge(&self) -> Option<Vec<u8>> {
        STANDARD
            .decode(&self.challenge)
            .ok()
            .filter(|c| c.len() == 32)
    }

    /// The user key, if `signature` is Windows Hello's signature of this
    /// enrollment's challenge and the sealed key is this account's.
    pub fn open(&self, user_id: &str, signature: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
        if self.v != 1 {
            return None;
        }
        let challenge = self.challenge()?;
        let nonce = STANDARD
            .decode(&self.nonce)
            .ok()
            .filter(|n| n.len() == 12)?;
        let ciphertext = STANDARD.decode(&self.ciphertext).ok()?;
        let cipher = Aes256Gcm::new(key(&challenge, signature).as_ref().into());
        cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: user_id.as_bytes(),
                },
            )
            .ok()
            .map(Zeroizing::new)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    fn vector() -> Value {
        serde_json::from_str(include_str!("../tests/vectors/windows-hello-seal.json")).unwrap()
    }

    fn b64(v: &Value, field: &str) -> Vec<u8> {
        STANDARD.decode(v[field].as_str().unwrap()).unwrap()
    }

    #[test]
    fn sealing_matches_the_vector_made_with_node() {
        let v = vector();
        assert_eq!(v["info"].as_str().unwrap().as_bytes(), INFO);
        let user_id = v["userId"].as_str().unwrap();
        let challenge: [u8; 32] = b64(&v, "challenge").try_into().unwrap();
        let nonce: [u8; 12] = b64(&v, "nonce").try_into().unwrap();
        let sealed = seal_with_nonce(
            user_id,
            &challenge,
            &b64(&v, "signature"),
            &b64(&v, "userKey"),
            nonce,
        );
        assert_eq!(sealed.ciphertext, v["ciphertext"].as_str().unwrap());
        assert_eq!(
            *sealed.open(user_id, &b64(&v, "signature")).unwrap(),
            b64(&v, "userKey")
        );
    }

    #[test]
    fn another_signature_or_another_account_opens_nothing() {
        let v = vector();
        let user_id = v["userId"].as_str().unwrap();
        let challenge = new_challenge();
        let signature = b64(&v, "signature");
        let sealed = seal(user_id, &challenge, &signature, &b64(&v, "userKey"));
        let mut other = signature.clone();
        other[0] ^= 1;
        assert!(sealed.open(user_id, &other).is_none());
        assert!(sealed.open("another-account", &signature).is_none());
        // Stored as JSON, and back.
        let stored: Sealed =
            serde_json::from_str(&serde_json::to_string(&sealed).unwrap()).unwrap();
        assert_eq!(stored, sealed);
        assert!(stored.open(user_id, &signature).is_some());
    }
}
