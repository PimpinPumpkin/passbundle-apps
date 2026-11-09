//! Firefox's key database (`key4.db`), as NSS (Mozilla's MPL-licensed
//! security library) lays it out, and the values Firefox encrypts with the
//! key it holds. The details are in DESIGN.md, "Automatic import".
//!
//! - `metaData`, row `password`: a salt for the whole database (`item1`),
//!   and "password-check" encrypted under the Primary Password (`item2`).
//!   With no Primary Password set, it's the empty password.
//! - `nssPrivate`: the key Firefox encrypts saved logins with (`a11`, its
//!   value, encrypted the same way; `a102`, its id).
//! - Encrypted the same way: PKCS #5's PBES2, a key made with PBKDF2
//!   (HMAC-SHA256) from SHA-1 of the salt and the password, and AES-256-CBC
//!   (NSS writes the IV's 14 bytes, and the IV is them after their own DER
//!   header). Older databases use PKCS #12's SHA-1 and Triple DES scheme.
//! - A saved login's username and password: DER with the key's id, the
//!   cipher (Triple DES or AES-256, CBC) and its IV, and the ciphertext.

use std::path::Path;

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use hmac::{Hmac, Mac};
use sha1::{Digest, Sha1};
use zeroize::Zeroizing;

use crate::der::{Reader, NULL, OCTET_STRING, OID};

pub(crate) const PBES2: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x05, 0x0d];
pub(crate) const PBKDF2: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x05, 0x0c];
pub(crate) const HMAC_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x02, 0x09];
pub(crate) const HMAC_SHA1: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x02, 0x07];
pub(crate) const AES256_CBC: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x01, 0x2a];
pub(crate) const PBE_SHA1_3DES: &[u8] = &[
    0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x0c, 0x05, 0x01, 0x03,
];
pub(crate) const DES_EDE3_CBC: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x03, 0x07];

/// What the database holds, still encrypted.
pub(crate) struct KeyDb {
    salt: Vec<u8>,
    check: Vec<u8>,
    /// Each key: its id and its encrypted value.
    keys: Vec<(Vec<u8>, Vec<u8>)>,
}

/// Why the database couldn't be used.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DbError {
    /// It couldn't be opened or read.
    Unreadable,
    /// It isn't laid out as NSS lays one out.
    Unknown,
}

/// The keys, opened with the right password.
pub(crate) struct Unlocked {
    keys: Vec<(Vec<u8>, Zeroizing<Vec<u8>>)>,
}

impl KeyDb {
    pub(crate) fn open(path: &Path) -> Result<KeyDb, DbError> {
        let conn = crate::sqlite::open(path).map_err(|_| DbError::Unreadable)?;
        let (salt, check): (Vec<u8>, Vec<u8>) = conn
            .query_row(
                "SELECT item1, item2 FROM metaData WHERE id = 'password'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| DbError::Unknown)?;
        let mut statement = conn
            .prepare("SELECT a102, a11 FROM nssPrivate")
            .map_err(|_| DbError::Unknown)?;
        let keys = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, Option<Vec<u8>>>(0)?,
                    r.get::<_, Option<Vec<u8>>>(1)?,
                ))
            })
            .map_err(|_| DbError::Unknown)?
            .filter_map(|row| match row {
                Ok((Some(id), Some(value))) => Some((id, value)),
                _ => None,
            })
            .collect();
        Ok(KeyDb { salt, check, keys })
    }

    /// Whether the empty password opens it: no Primary Password.
    pub(crate) fn needs_password(&self) -> bool {
        !self.password_opens(b"")
    }

    fn password_opens(&self, password: &[u8]) -> bool {
        decrypt_pbe(&self.check, &self.salt, password)
            .is_some_and(|c| c.as_slice() == b"password-check")
    }

    /// The keys, if `password` is the Primary Password (empty for none).
    pub(crate) fn unlock(&self, password: &[u8]) -> Option<Unlocked> {
        if !self.password_opens(password) {
            return None;
        }
        let keys = self
            .keys
            .iter()
            .filter_map(|(id, value)| Some((id.clone(), decrypt_pbe(value, &self.salt, password)?)))
            .collect();
        Some(Unlocked { keys })
    }
}

impl Unlocked {
    /// One of a login's values, as logins.json keeps it (base64 DER).
    pub(crate) fn decrypt(&self, encoded: &str) -> Option<Zeroizing<String>> {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let der = STANDARD.decode(encoded.trim()).ok()?;
        let mut outer = Reader::new(&der);
        let mut item = outer.sequence()?;
        let key_id = item.take(OCTET_STRING)?;
        let mut algorithm = item.sequence()?;
        let cipher = algorithm.take(OID)?;
        let iv = algorithm.take(OCTET_STRING)?;
        let ciphertext = item.take(OCTET_STRING)?;
        let (_, key) = self.keys.iter().find(|(id, _)| id.as_slice() == key_id)?;
        let plain = decrypt_cbc(cipher, key, iv, ciphertext)?;
        String::from_utf8(plain.to_vec()).ok().map(Zeroizing::new)
    }
}

/// Decrypts with the cipher named, under `key` (as long or longer than the
/// cipher's key), removing the padding.
fn decrypt_cbc(
    cipher: &[u8],
    key: &[u8],
    iv: &[u8],
    ciphertext: &[u8],
) -> Option<Zeroizing<Vec<u8>>> {
    let opened = if cipher == DES_EDE3_CBC {
        let key = key.get(..24)?;
        let iv: [u8; 8] = iv.try_into().ok()?;
        cbc::Decryptor::<des::TdesEde3>::new_from_slices(key, &iv)
            .ok()?
            .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
            .ok()?
    } else if cipher == AES256_CBC {
        let key = key.get(..32)?;
        let iv: [u8; 16] = iv.try_into().ok()?;
        cbc::Decryptor::<aes::Aes256>::new_from_slices(key, &iv)
            .ok()?
            .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
            .ok()?
    } else {
        return None;
    };
    Some(Zeroizing::new(opened))
}

/// Opens a value the database encrypted under the password.
pub(crate) fn decrypt_pbe(item: &[u8], salt: &[u8], password: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
    let mut outer = Reader::new(item);
    let mut top = outer.sequence()?;
    let mut algorithm = top.sequence()?;
    let scheme = algorithm.take(OID)?;
    let ciphertext = top.take(OCTET_STRING)?;
    let mut password_hash = Sha1::new();
    password_hash.update(salt);
    password_hash.update(password);
    let hashed = Zeroizing::new(password_hash.finalize().to_vec());
    if scheme == PBES2 {
        let mut params = algorithm.sequence()?;
        let mut kdf = params.sequence()?;
        if kdf.take(OID)? != PBKDF2 {
            return None;
        }
        let mut kdf_params = kdf.sequence()?;
        let kdf_salt = kdf_params.take(OCTET_STRING)?;
        let rounds = kdf_params.integer()?;
        if rounds == 0 || rounds > 10_000_000 {
            return None;
        }
        let mut key_length = 32;
        let mut sha256 = false;
        while !kdf_params.is_empty() {
            match kdf_params.any()? {
                (crate::der::INTEGER, bytes) => {
                    key_length = crate::der::uint(bytes)? as usize;
                }
                (crate::der::SEQUENCE, prf) => {
                    let mut prf = Reader::new(prf);
                    let oid = prf.take(OID)?;
                    if oid == HMAC_SHA256 {
                        sha256 = true;
                    } else if oid != HMAC_SHA1 {
                        return None;
                    }
                    if !prf.is_empty() {
                        prf.take(NULL)?;
                    }
                }
                _ => return None,
            }
        }
        if key_length != 32 {
            return None;
        }
        let mut cipher = params.sequence()?;
        let cipher_oid = cipher.take(OID)?;
        let iv = cipher.take(OCTET_STRING)?;
        // NSS's IV for AES here: its 14 bytes, after their own DER header.
        let iv = match iv.len() {
            14 => [&[OCTET_STRING, 14][..], iv].concat(),
            16 => iv.to_vec(),
            _ => return None,
        };
        let mut key = Zeroizing::new([0u8; 32]);
        if sha256 {
            pbkdf2::pbkdf2_hmac::<sha2::Sha256>(&hashed, kdf_salt, rounds, key.as_mut());
        } else {
            pbkdf2::pbkdf2_hmac::<Sha1>(&hashed, kdf_salt, rounds, key.as_mut());
        }
        decrypt_cbc(cipher_oid, key.as_ref(), &iv, ciphertext)
    } else if scheme == PBE_SHA1_3DES {
        let mut params = algorithm.sequence()?;
        let entry_salt = params.take(OCTET_STRING)?;
        let (key, iv) = triple_des_key(&hashed, entry_salt);
        decrypt_cbc(DES_EDE3_CBC, key.as_ref(), &iv, ciphertext)
    } else {
        None
    }
}

/// The key and IV of NSS's older scheme (as in its `sftkdb`'s PKCS #12
/// SHA-1 key derivation for Triple DES), from SHA-1 of the salt and
/// password, and the entry's own salt.
pub(crate) fn triple_des_key(hashed: &[u8], entry_salt: &[u8]) -> (Zeroizing<Vec<u8>>, [u8; 8]) {
    let mut padded = entry_salt.to_vec();
    padded.resize(20.max(entry_salt.len()), 0);
    let mut chp_hash = Sha1::new();
    chp_hash.update(hashed);
    chp_hash.update(entry_salt);
    let chp = chp_hash.finalize();
    let hmac = |parts: &[&[u8]]| {
        let mut mac = Hmac::<Sha1>::new_from_slice(&chp).expect("HMAC takes any key");
        for part in parts {
            mac.update(part);
        }
        mac.finalize().into_bytes().to_vec()
    };
    let k1 = hmac(&[&padded, entry_salt]);
    let tk = hmac(&[&padded]);
    let k2 = hmac(&[&tk, entry_salt]);
    let k = Zeroizing::new([k1, k2].concat());
    let iv: [u8; 8] = k[k.len() - 8..].try_into().expect("40 bytes");
    (Zeroizing::new(k[..24].to_vec()), iv)
}
