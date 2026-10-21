//! Keys and encrypted strings, in the format the other apps use (docs/core.md,
//! "The format"): type 2 (AES-256-CBC, then HMAC-SHA256) for everything
//! symmetric, and types 3 and 4 (RSA-2048 OAEP) for organization keys.

use std::fmt;

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use base64::{engine::general_purpose::STANDARD, Engine};
use hmac::{Hmac, Mac};
use rand::{rngs::OsRng, RngCore};
use rsa::{
    pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePublicKey},
    Oaep, RsaPrivateKey, RsaPublicKey,
};
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::error::{Error, Result};

type HmacSha256 = Hmac<Sha256>;
type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

/// A 64-byte symmetric key: 32 for AES-256, then 32 for HMAC-SHA256. The user
/// key, organization keys, item keys, and the stretched master key.
#[derive(Clone, Zeroize, ZeroizeOnDrop, PartialEq, Eq)]
pub struct SymmetricKey {
    enc: [u8; 32],
    mac: [u8; 32],
}

impl SymmetricKey {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 64 {
            // 32-byte keys are the old format without a MAC key.
            return Err(if bytes.len() == 32 {
                Error::Refused("it's an old key with no integrity check")
            } else {
                Error::Damaged("a key of the wrong length")
            });
        }
        let mut key = SymmetricKey {
            enc: [0; 32],
            mac: [0; 32],
        };
        key.enc.copy_from_slice(&bytes[..32]);
        key.mac.copy_from_slice(&bytes[32..]);
        Ok(key)
    }

    pub(crate) fn from_parts(enc: [u8; 32], mac: [u8; 32]) -> Self {
        SymmetricKey { enc, mac }
    }

    /// A new random key.
    pub fn generate() -> Self {
        let mut key = SymmetricKey {
            enc: [0; 32],
            mac: [0; 32],
        };
        OsRng.fill_bytes(&mut key.enc);
        OsRng.fill_bytes(&mut key.mac);
        key
    }

    pub fn to_bytes(&self) -> Zeroizing<Vec<u8>> {
        let mut out = Zeroizing::new(Vec::with_capacity(64));
        out.extend_from_slice(&self.enc);
        out.extend_from_slice(&self.mac);
        out
    }

    /// Encrypts with a random IV, as type 2.
    pub fn encrypt(&self, plaintext: &[u8]) -> EncString {
        let mut iv = [0u8; 16];
        OsRng.fill_bytes(&mut iv);
        self.encrypt_with_iv(plaintext, iv)
    }

    pub fn encrypt_str(&self, plaintext: &str) -> EncString {
        self.encrypt(plaintext.as_bytes())
    }

    fn encrypt_with_iv(&self, plaintext: &[u8], iv: [u8; 16]) -> EncString {
        let data = Aes256CbcEnc::new(&self.enc.into(), &iv.into())
            .encrypt_padded_vec_mut::<Pkcs7>(plaintext);
        let mac = self.mac_of(&iv, &data).finalize().into_bytes().into();
        EncString::Aes { iv, data, mac }
    }

    fn mac_of(&self, iv: &[u8], data: &[u8]) -> HmacSha256 {
        let mut mac =
            <HmacSha256 as Mac>::new_from_slice(&self.mac).expect("HMAC takes any key length");
        mac.update(iv);
        mac.update(data);
        mac
    }
}

impl fmt::Debug for SymmetricKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SymmetricKey(..)")
    }
}

/// An encrypted string, as the server stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EncString {
    /// Type 2: AES-256-CBC with PKCS#7, then HMAC-SHA256 over the IV and ciphertext.
    Aes {
        iv: [u8; 16],
        data: Vec<u8>,
        mac: [u8; 32],
    },
    /// Types 3 (SHA-256) and 4 (SHA-1): RSA-2048 OAEP.
    Rsa { sha256: bool, data: Vec<u8> },
}

fn decode(part: &str) -> Result<Vec<u8>> {
    STANDARD
        .decode(part)
        .map_err(|_| Error::Damaged("not base64"))
}

impl EncString {
    pub fn parse(text: &str) -> Result<Self> {
        let Some((kind, rest)) = text.split_once('.') else {
            return Err(Error::Refused("it isn't an encrypted value"));
        };
        match kind {
            "2" => {
                let mut parts = rest.split('|');
                let (Some(iv), Some(data), Some(mac), None) =
                    (parts.next(), parts.next(), parts.next(), parts.next())
                else {
                    return Err(Error::Damaged("the wrong number of parts"));
                };
                let iv: [u8; 16] = decode(iv)?
                    .try_into()
                    .map_err(|_| Error::Damaged("an IV of the wrong length"))?;
                let mac: [u8; 32] = decode(mac)?
                    .try_into()
                    .map_err(|_| Error::Damaged("a MAC of the wrong length"))?;
                let data = decode(data)?;
                if data.is_empty() || data.len() % 16 != 0 {
                    return Err(Error::Damaged("a ciphertext of the wrong length"));
                }
                Ok(EncString::Aes { iv, data, mac })
            }
            "3" | "4" => {
                if rest.contains('|') {
                    return Err(Error::Damaged("the wrong number of parts"));
                }
                Ok(EncString::Rsa {
                    sha256: kind == "3",
                    data: decode(rest)?,
                })
            }
            "0" => Err(Error::Refused("it's an old format with no integrity check")),
            "1" => Err(Error::Refused("it's an old format with a 128-bit key")),
            "5" | "6" => Err(Error::Refused("it's an older RSA format")),
            _ if kind.chars().all(|c| c.is_ascii_digit()) && !kind.is_empty() => {
                Err(Error::NewerApp)
            }
            _ => Err(Error::Refused("it isn't an encrypted value")),
        }
    }

    /// Opens a type 2 string. The MAC is checked first, in constant time.
    pub fn decrypt(&self, key: &SymmetricKey) -> Result<Zeroizing<Vec<u8>>> {
        let EncString::Aes { iv, data, mac } = self else {
            return Err(Error::Damaged("an RSA value where a symmetric one belongs"));
        };
        key.mac_of(iv, data)
            .verify_slice(mac)
            .map_err(|_| Error::Refused("its integrity check failed"))?;
        Aes256CbcDec::new(&key.enc.into(), &(*iv).into())
            .decrypt_padded_vec_mut::<Pkcs7>(data)
            .map(Zeroizing::new)
            .map_err(|_| Error::Damaged("bad padding"))
    }

    pub fn decrypt_string(&self, key: &SymmetricKey) -> Result<Zeroizing<String>> {
        let bytes = self.decrypt(key)?;
        std::str::from_utf8(&bytes)
            .map(|s| Zeroizing::new(s.to_owned()))
            .map_err(|_| Error::Damaged("text that isn't UTF-8"))
    }

    /// Opens a type 2 string that holds a 64-byte key.
    pub fn decrypt_key(&self, key: &SymmetricKey) -> Result<SymmetricKey> {
        SymmetricKey::from_bytes(&self.decrypt(key)?)
    }
}

impl fmt::Display for EncString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EncString::Aes { iv, data, mac } => {
                write!(
                    f,
                    "2.{}|{}|{}",
                    STANDARD.encode(iv),
                    STANDARD.encode(data),
                    STANDARD.encode(mac)
                )
            }
            EncString::Rsa { sha256, data } => write!(
                f,
                "{}.{}",
                if *sha256 { 3 } else { 4 },
                STANDARD.encode(data)
            ),
        }
    }
}

/// The person's RSA-2048 private key, which opens organization keys.
pub struct PrivateKey(RsaPrivateKey);

impl PrivateKey {
    /// From PKCS#8 DER, as it decrypts from the profile's `privateKey`.
    pub fn from_pkcs8_der(der: &[u8]) -> Result<Self> {
        let key = RsaPrivateKey::from_pkcs8_der(der)
            .map_err(|_| Error::Damaged("not an RSA private key"))?;
        use rsa::traits::PublicKeyParts;
        if key.size() != 256 {
            return Err(Error::NewerApp);
        }
        Ok(PrivateKey(key))
    }

    /// The public key, as SPKI DER.
    pub fn public_key_der(&self) -> Vec<u8> {
        self.0
            .to_public_key()
            .to_public_key_der()
            .expect("an RSA public key always encodes")
            .into_vec()
    }

    /// Opens a type 3 or 4 string, such as an organization key.
    pub fn decrypt(&self, value: &EncString) -> Result<Zeroizing<Vec<u8>>> {
        let EncString::Rsa { sha256, data } = value else {
            return Err(Error::Damaged("a symmetric value where an RSA one belongs"));
        };
        let padding = if *sha256 {
            Oaep::new::<Sha256>()
        } else {
            Oaep::new::<sha1::Sha1>()
        };
        self.0
            .decrypt(padding, data)
            .map(Zeroizing::new)
            .map_err(|_| Error::Refused("it wasn't made for this account's key"))
    }
}

impl fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PrivateKey(..)")
    }
}

/// Encrypts for someone's public key (SPKI DER) as type 4, as the other apps
/// share organization keys.
pub fn encrypt_for(public_key_der: &[u8], data: &[u8]) -> Result<EncString> {
    let key = RsaPublicKey::from_public_key_der(public_key_der)
        .map_err(|_| Error::Damaged("not an RSA public key"))?;
    let data = key
        .encrypt(&mut OsRng, Oaep::new::<sha1::Sha1>(), data)
        .map_err(|_| Error::Damaged("too long for RSA"))?;
    Ok(EncString::Rsa {
        sha256: false,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_uses_a_new_iv_each_time() {
        let key = SymmetricKey::generate();
        let a = key.encrypt_str("hello");
        let b = key.encrypt_str("hello");
        assert_ne!(a, b);
        let reparsed = EncString::parse(&a.to_string()).unwrap();
        assert_eq!(&*reparsed.decrypt_string(&key).unwrap(), "hello");
    }

    #[test]
    fn a_key_never_prints() {
        let key = SymmetricKey::generate();
        assert_eq!(format!("{key:?}"), "SymmetricKey(..)");
    }
}
