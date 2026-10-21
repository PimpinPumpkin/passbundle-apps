//! The encrypted channel between the browser extension and the agent, exactly
//! as upstream's extension sets it up with upstream's desktop app
//! (`apps/browser/src/background/nativeMessaging.background.ts`):
//!
//! - The extension sends a new RSA-2048 public key (SPKI). The agent makes a
//!   64-byte session key (a 32-byte AES-256-CBC key, then a 32-byte
//!   HMAC-SHA256 key) and sends it back encrypted to that key with RSA-OAEP
//!   (SHA-1 for the hash and MGF1, as upstream).
//! - Every message after that is a type-2 EncString under the session key:
//!   `2.<iv>|<ciphertext>|<mac>`, base64, with the MAC over IV and ciphertext.
//!
//! `tests/vectors/native-messaging.json` pins both against upstream's own
//! crypto (`tests/vectors/check-upstream.mjs`). See DESIGN.md, "Biometric unlock".

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use base64::{engine::general_purpose::STANDARD, Engine};
use hmac::{Hmac, Mac};
use rand::{rngs::OsRng, RngCore};
use rsa::{pkcs8::DecodePublicKey, Oaep, RsaPublicKey};
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop};

type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;
type HmacSha256 = Hmac<Sha256>;

/// The extension's key is RSA-2048; refuse anything weaker or absurd.
const RSA_BITS: std::ops::RangeInclusive<usize> = 2048..=4096;

#[derive(Debug, PartialEq, Eq)]
pub enum ChannelError {
    BadPublicKey,
    Malformed,
    /// The MAC doesn't match: changed, or not under this session key.
    BadMac,
}

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SessionKey([u8; 64]);

impl SessionKey {
    pub fn generate() -> Self {
        let mut key = [0u8; 64];
        OsRng.fill_bytes(&mut key);
        Self(key)
    }

    #[cfg(test)]
    fn from_bytes(bytes: [u8; 64]) -> Self {
        Self(bytes)
    }

    /// For tests that play the extension's side (and so know the key).
    #[cfg(any(test, feature = "test-authenticator"))]
    pub fn from_test_bytes(bytes: &[u8]) -> Self {
        Self(bytes.try_into().expect("64 bytes"))
    }

    fn enc_key(&self) -> &[u8] {
        &self.0[..32]
    }

    fn mac_key(&self) -> &[u8] {
        &self.0[32..]
    }

    /// The session key encrypted to the extension's public key (SPKI DER),
    /// for `setupEncryption`'s `sharedSecret`.
    ///
    /// The agent only ever encrypts with RSA, never decrypts or signs, so the
    /// timing side channel in the `rsa` crate's private-key operations
    /// (RUSTSEC-2023-0071) doesn't apply.
    pub fn wrap_for(&self, spki_der: &[u8]) -> Result<Vec<u8>, ChannelError> {
        let public_key =
            RsaPublicKey::from_public_key_der(spki_der).map_err(|_| ChannelError::BadPublicKey)?;
        if !RSA_BITS.contains(&(rsa::traits::PublicKeyParts::size(&public_key) * 8)) {
            return Err(ChannelError::BadPublicKey);
        }
        public_key
            .encrypt(&mut OsRng, Oaep::new::<sha1::Sha1>(), &self.0)
            .map_err(|_| ChannelError::BadPublicKey)
    }

    /// Encrypts a message with a fresh random IV.
    pub fn encrypt(&self, plaintext: &str) -> String {
        let mut iv = [0u8; 16];
        OsRng.fill_bytes(&mut iv);
        self.encrypt_with_iv(plaintext.as_bytes(), iv)
    }

    fn encrypt_with_iv(&self, plaintext: &[u8], iv: [u8; 16]) -> String {
        let ciphertext = Aes256CbcEnc::new(self.enc_key().into(), &iv.into())
            .encrypt_padded_vec_mut::<Pkcs7>(plaintext);
        let mac = self.mac(&iv, &ciphertext).finalize().into_bytes();
        format!(
            "2.{}|{}|{}",
            STANDARD.encode(iv),
            STANDARD.encode(&ciphertext),
            STANDARD.encode(mac)
        )
    }

    /// Decrypts a type-2 EncString, checking the MAC (in constant time) before
    /// anything else.
    pub fn decrypt(&self, enc_string: &str) -> Result<String, ChannelError> {
        let body = enc_string
            .strip_prefix("2.")
            .ok_or(ChannelError::Malformed)?;
        let mut parts = body.split('|');
        let (Some(iv), Some(ciphertext), Some(mac), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(ChannelError::Malformed);
        };
        let decode = |s: &str| STANDARD.decode(s).map_err(|_| ChannelError::Malformed);
        let (iv, ciphertext, mac) = (decode(iv)?, decode(ciphertext)?, decode(mac)?);
        let iv: [u8; 16] = iv.try_into().map_err(|_| ChannelError::Malformed)?;
        self.mac(&iv, &ciphertext)
            .verify_slice(&mac)
            .map_err(|_| ChannelError::BadMac)?;
        let plaintext = Aes256CbcDec::new(self.enc_key().into(), &iv.into())
            .decrypt_padded_vec_mut::<Pkcs7>(&ciphertext)
            .map_err(|_| ChannelError::Malformed)?;
        String::from_utf8(plaintext).map_err(|_| ChannelError::Malformed)
    }

    fn mac(&self, iv: &[u8], ciphertext: &[u8]) -> HmacSha256 {
        let mut mac =
            <HmacSha256 as Mac>::new_from_slice(self.mac_key()).expect("HMAC takes any key size");
        mac.update(iv);
        mac.update(ciphertext);
        mac
    }
}

#[cfg(test)]
mod tests {
    use rsa::{pkcs8::DecodePrivateKey, RsaPrivateKey};
    use serde::Deserialize;

    use super::*;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Vectors {
        session_key: String,
        messages: Vec<Message>,
        rsa: Rsa,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Message {
        iv: String,
        plaintext: String,
        enc_string: String,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Rsa {
        public_key_spki: String,
        private_key_pkcs8: String,
        wrapped_session_key: String,
    }

    fn vectors() -> Vectors {
        serde_json::from_str(include_str!("../tests/vectors/native-messaging.json")).unwrap()
    }

    fn b64(s: &str) -> Vec<u8> {
        STANDARD.decode(s).unwrap()
    }

    fn session_key(v: &Vectors) -> SessionKey {
        SessionKey::from_bytes(b64(&v.session_key).try_into().unwrap())
    }

    #[test]
    fn messages_match_upstreams_bytes_both_ways() {
        let v = vectors();
        let key = session_key(&v);
        for m in &v.messages {
            assert_eq!(key.decrypt(&m.enc_string).unwrap(), m.plaintext);
            let iv: [u8; 16] = b64(&m.iv).try_into().unwrap();
            assert_eq!(
                key.encrypt_with_iv(m.plaintext.as_bytes(), iv),
                m.enc_string
            );
        }
    }

    #[test]
    fn a_fresh_encryption_round_trips_with_a_new_iv_each_time() {
        let key = SessionKey::generate();
        let (a, b) = (key.encrypt("hello"), key.encrypt("hello"));
        assert_ne!(a, b);
        assert_eq!(key.decrypt(&a).unwrap(), "hello");
    }

    #[test]
    fn any_change_or_another_key_is_refused() {
        let v = vectors();
        let key = session_key(&v);
        let original = &v.messages[0].enc_string;
        let (iv, rest) = original[2..].split_once('|').unwrap();
        let (ct, mac) = rest.split_once('|').unwrap();
        let flip = |s: &str| {
            let mut bytes = b64(s);
            bytes[0] ^= 1;
            STANDARD.encode(bytes)
        };
        for changed in [
            format!("2.{}|{ct}|{mac}", flip(iv)),
            format!("2.{iv}|{}|{mac}", flip(ct)),
            format!("2.{iv}|{ct}|{}", flip(mac)),
        ] {
            assert_eq!(key.decrypt(&changed), Err(ChannelError::BadMac));
        }
        assert_eq!(
            SessionKey::generate().decrypt(original),
            Err(ChannelError::BadMac)
        );
        for malformed in [
            "",
            "2.",
            "0.abc|def",
            "1.a|b|c",
            "2.a|b",
            "2.a|b|c|d",
            "2.!!|b|c",
        ] {
            assert_eq!(
                key.decrypt(malformed),
                Err(ChannelError::Malformed),
                "{malformed:?}"
            );
        }
    }

    #[test]
    fn the_session_key_wraps_as_upstream_unwraps_it() {
        let v = vectors();
        let private_key = RsaPrivateKey::from_pkcs8_der(&b64(&v.rsa.private_key_pkcs8)).unwrap();
        let oaep = || Oaep::new::<sha1::Sha1>();
        // The vector's wrapped key (made with Node, unwrapped by upstream's
        // code in check-upstream.mjs) uses the same parameters as ours.
        let from_vector = private_key
            .decrypt(oaep(), &b64(&v.rsa.wrapped_session_key))
            .unwrap();
        assert_eq!(from_vector, b64(&v.session_key));
        let key = session_key(&v);
        let wrapped = key.wrap_for(&b64(&v.rsa.public_key_spki)).unwrap();
        assert_eq!(
            private_key.decrypt(oaep(), &wrapped).unwrap(),
            b64(&v.session_key)
        );
    }

    #[test]
    fn a_weak_or_malformed_public_key_is_refused() {
        let key = SessionKey::generate();
        assert_eq!(key.wrap_for(b"not a key"), Err(ChannelError::BadPublicKey));
        let weak = RsaPrivateKey::new(&mut OsRng, 1024).unwrap();
        let spki = rsa::pkcs8::EncodePublicKey::to_public_key_der(&weak.to_public_key()).unwrap();
        assert_eq!(
            key.wrap_for(spki.as_bytes()),
            Err(ChannelError::BadPublicKey)
        );
    }
}
