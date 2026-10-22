//! Passkeys (docs/core.md, "Passkeys"): what an authenticator does, in
//! upstream's format. ES256 only (ECDSA on P-256 with SHA-256). The private
//! key is a PKCS#8 document, base64url without padding, as upstream stores
//! it in `keyValue`; it's read here only to sign, and zeroized after.
//!
//! - **Signing in:** `authenticator_data` (the site ID's hash, the flags, the
//!   counter), then `Key::sign` over it and the client data hash, DER-encoded,
//!   with RFC 6979's deterministic nonces.
//! - **Making one:** `Key::generate`, `new_credential_id`, then
//!   `authenticator_data` with the attested credential data (PassBundle's
//!   AAGUID, the credential ID, the COSE key) and `attestation_object`
//!   (attestation "none").

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::error::{Error, Result};

/// PassBundle's AAGUID, `09c5ee5f-0845-48e6-aef4-c7c443317de5`, made at
/// random for the design: what a site sees as the passkey's maker.
pub const AAGUID: [u8; 16] = [
    0x09, 0xc5, 0xee, 0x5f, 0x08, 0x45, 0x48, 0xe6, 0xae, 0xf4, 0xc7, 0xc4, 0x43, 0x31, 0x7d, 0xe5,
];

/// The flags in the authenticator data (WebAuthn, "Authenticator Data").
pub mod flags {
    /// The person was there.
    pub const USER_PRESENT: u8 = 0x01;
    /// The person proved who they are (biometrics or the master password).
    pub const USER_VERIFIED: u8 = 0x04;
    /// It can be backed up, and it is: the vault syncs.
    pub const BACKUP_ELIGIBLE: u8 = 0x08;
    pub const BACKED_UP: u8 = 0x10;
    /// Attested credential data follows (only when making one).
    pub const ATTESTED: u8 = 0x40;
}

/// A credential ID as stored (`credentialId`): a UUID, whose 16 bytes are the
/// ID, or `b64.` and the ID in base64url for one imported from elsewhere.
/// Anything else is refused.
pub fn credential_id(stored: &str) -> Result<Vec<u8>> {
    if let Some(encoded) = stored.strip_prefix("b64.") {
        let id = URL_SAFE_NO_PAD
            .decode(encoded.trim_end_matches('='))
            .map_err(|_| Error::Damaged("a passkey's credential ID that isn't base64url"))?;
        if id.is_empty() || id.len() > 1023 {
            return Err(Error::Damaged(
                "a passkey's credential ID of the wrong length",
            ));
        }
        return Ok(id);
    }
    uuid::Uuid::try_parse(stored)
        .map(|id| id.as_bytes().to_vec())
        .map_err(|_| Error::Damaged("a passkey's credential ID that isn't a UUID"))
}

/// A new credential ID, as upstream makes them: a random UUID.
pub fn new_credential_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// A passkey's private key.
pub struct Key(SigningKey);

impl Key {
    /// A new key, from the OS's random numbers.
    pub fn generate() -> Key {
        Key(SigningKey::random(&mut rand::rngs::OsRng))
    }

    /// The key as upstream stores it: PKCS#8, base64url without padding.
    /// Anything that isn't a P-256 key is refused.
    pub fn from_stored(key_value: &str) -> Result<Key> {
        let der = Zeroizing::new(
            URL_SAFE_NO_PAD
                .decode(key_value.trim_end_matches('='))
                .map_err(|_| Error::Damaged("a passkey's key that isn't base64url"))?,
        );
        SigningKey::from_pkcs8_der(&der)
            .map(Key)
            .map_err(|_| Error::Refused("it's a passkey that isn't ES256 (P-256)"))
    }

    /// The key to store (`keyValue`).
    pub fn to_stored(&self) -> Result<Zeroizing<String>> {
        let der = self
            .0
            .to_pkcs8_der()
            .map_err(|_| Error::Damaged("a passkey's key that can't be written"))?;
        Ok(Zeroizing::new(URL_SAFE_NO_PAD.encode(der.as_bytes())))
    }

    /// The public key's coordinates, for the COSE key.
    pub fn public(&self) -> ([u8; 32], [u8; 32]) {
        let point = self.0.verifying_key().to_encoded_point(false);
        let x = point.x().expect("an uncompressed point has x");
        let y = point.y().expect("an uncompressed point has y");
        ((*x).into(), (*y).into())
    }

    /// The public key as SubjectPublicKeyInfo (DER), which Android and
    /// browsers want alongside the attestation object.
    pub fn public_der(&self) -> Result<Vec<u8>> {
        use p256::pkcs8::EncodePublicKey;
        self.0
            .verifying_key()
            .to_public_key_der()
            .map(|der| der.as_bytes().to_vec())
            .map_err(|_| Error::Damaged("a passkey's public key that can't be written"))
    }

    /// An assertion's signature: ECDSA with SHA-256 over the authenticator
    /// data followed by the client data hash, DER-encoded. The nonce is
    /// RFC 6979's, so the same input signs to the same bytes.
    pub fn sign(&self, authenticator_data: &[u8], client_data_hash: &[u8; 32]) -> Vec<u8> {
        let mut message = Vec::with_capacity(authenticator_data.len() + 32);
        message.extend_from_slice(authenticator_data);
        message.extend_from_slice(client_data_hash);
        let signature: Signature = self.0.sign(&message);
        signature.to_der().as_bytes().to_vec()
    }
}

/// What making a passkey adds to the authenticator data: the credential ID
/// and the public key.
pub struct Attested<'a> {
    pub credential_id: &'a [u8],
    pub public: ([u8; 32], [u8; 32]),
}

/// The authenticator data: SHA-256 of the site's ID, the flags, and the
/// counter (4 bytes, big-endian), then, when making one, PassBundle's AAGUID,
/// the credential ID's length (2 bytes) and the ID, and the COSE key.
pub fn authenticator_data(
    rp_id: &str,
    flags: u8,
    counter: u32,
    attested: Option<&Attested<'_>>,
) -> Vec<u8> {
    let mut data = Vec::with_capacity(37);
    data.extend_from_slice(&Sha256::digest(rp_id.as_bytes()));
    data.push(if attested.is_some() {
        flags | flags::ATTESTED
    } else {
        flags & !flags::ATTESTED
    });
    data.extend_from_slice(&counter.to_be_bytes());
    if let Some(attested) = attested {
        data.extend_from_slice(&AAGUID);
        let length = u16::try_from(attested.credential_id.len()).expect("credential IDs are short");
        data.extend_from_slice(&length.to_be_bytes());
        data.extend_from_slice(attested.credential_id);
        data.extend_from_slice(&cose_key(&attested.public.0, &attested.public.1));
    }
    data
}

/// The public key as a COSE key, in CTAP2's canonical CBOR: a map of
/// kty 2 (EC2), alg -7 (ES256), crv 1 (P-256), x, and y, in that order.
pub fn cose_key(x: &[u8; 32], y: &[u8; 32]) -> Vec<u8> {
    let mut key = vec![
        0xa5, // a map of 5
        0x01, 0x02, // kty: EC2
        0x03, 0x26, // alg: -7, ES256
        0x20, 0x01, // crv (-1): P-256
        0x21, 0x58, 0x20, // x (-2): 32 bytes
    ];
    key.extend_from_slice(x);
    key.extend_from_slice(&[0x22, 0x58, 0x20]); // y (-3): 32 bytes
    key.extend_from_slice(y);
    key
}

/// The attestation object for attestation "none", in CTAP2's canonical CBOR:
/// `{"fmt": "none", "attStmt": {}, "authData": <authenticator data>}`.
pub fn attestation_object(authenticator_data: &[u8]) -> Vec<u8> {
    let mut object = vec![0xa3]; // a map of 3
    object.extend_from_slice(&[0x63, b'f', b'm', b't', 0x64, b'n', b'o', b'n', b'e']);
    object.push(0x67);
    object.extend_from_slice(b"attStmt");
    object.push(0xa0); // an empty map
    object.push(0x68);
    object.extend_from_slice(b"authData");
    let length = authenticator_data.len();
    match length {
        0..=23 => object.push(0x40 | length as u8),
        24..=0xff => object.extend_from_slice(&[0x58, length as u8]),
        _ => {
            let length = u16::try_from(length).expect("authenticator data is short");
            object.push(0x59);
            object.extend_from_slice(&length.to_be_bytes());
        }
    }
    object.extend_from_slice(authenticator_data);
    object
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::signature::Verifier;
    use p256::ecdsa::VerifyingKey;

    fn hex(text: &str) -> Vec<u8> {
        let text: String = text.split_whitespace().collect();
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    /// RFC 6979, A.2.5: P-256's key pair.
    fn rfc_key() -> SigningKey {
        SigningKey::from_slice(&hex(
            "C9AFA9D845BA75166B5C215767B1D6934E50C3DB36E89B127B8A622B120F6721",
        ))
        .unwrap()
    }

    /// Signing is RFC 6979's, to the byte: its own examples for P-256 with
    /// SHA-256 ("sample" and "test").
    #[test]
    fn signs_as_rfc_6979_says() {
        let key = Key(rfc_key());
        let (x, y) = key.public();
        assert_eq!(
            x.to_vec(),
            hex("60FED4BA255A9D31C961EB74C6356D68C049B8923B61FA6CE669622E60F29FB6")
        );
        assert_eq!(
            y.to_vec(),
            hex("7903FE1008B8BC99A41AE9E95628BC64F2F1B20C2D7E9F5177A3C294D4462299")
        );
        for (message, r, s) in [
            (
                "sample",
                "EFD48B2AACB6A8FD1140DD9CD45E81D69D2C877B56AAF991C34D0EA84EAF3716",
                "F7CB1C942D657C41D436C7A1B6E29F65F3E900DBB9AFF4064DC4AB2F843ACDA8",
            ),
            (
                "test",
                "F1ABB023518351CD71D881567B1EA663ED3EFCF6C5132B354F28D3B0B7D38367",
                "019F4113742A2B14BD25926B49C649155F267E60D3814B4C0CC84250E46F0083",
            ),
        ] {
            let signature: Signature = key.0.sign(message.as_bytes());
            assert_eq!(signature.r().to_bytes().to_vec(), hex(r), "{message}");
            assert_eq!(signature.s().to_bytes().to_vec(), hex(s), "{message}");
        }
    }

    /// An assertion's signature covers the authenticator data and the client
    /// data hash, together, and is DER.
    #[test]
    fn an_assertion_signs_the_data_and_the_hash() {
        let key = Key(rfc_key());
        let data = authenticator_data(
            "example.test",
            flags::USER_PRESENT | flags::BACKUP_ELIGIBLE | flags::BACKED_UP,
            0,
            None,
        );
        assert_eq!(data.len(), 37);
        assert_eq!(&data[..32], Sha256::digest(b"example.test").as_slice());
        assert_eq!(data[32], 0x19);
        assert_eq!(&data[33..], &[0, 0, 0, 0]);
        let hash = [7u8; 32];
        let der = key.sign(&data, &hash);
        let signature = Signature::from_der(&der).unwrap();
        let mut signed = data.clone();
        signed.extend_from_slice(&hash);
        let verifying = VerifyingKey::from(&key.0);
        assert!(verifying.verify(&signed, &signature).is_ok());
        assert!(verifying.verify(&data, &signature).is_err());
        // The same input signs to the same bytes.
        assert_eq!(der, key.sign(&data, &hash));
    }

    #[test]
    fn a_key_round_trips_through_upstreams_format_and_others_are_refused() {
        let key = Key::generate();
        let stored = key.to_stored().unwrap();
        assert!(!stored.contains('=') && !stored.contains('+') && !stored.contains('/'));
        let again = Key::from_stored(&stored).unwrap();
        assert_eq!(key.public(), again.public());
        assert!(Key::from_stored("not base64 !").is_err());
        assert!(Key::from_stored(&URL_SAFE_NO_PAD.encode(b"not a key")).is_err());
        // A P-256 SubjectPublicKeyInfo is DER too, but not a private key.
        let public = URL_SAFE_NO_PAD.encode(key.public_der().unwrap());
        assert!(Key::from_stored(&public).is_err());
    }

    #[test]
    fn credential_ids_are_uuids_or_base64url() {
        assert_eq!(
            credential_id("09c5ee5f-0845-48e6-aef4-c7c443317de5").unwrap(),
            AAGUID.to_vec()
        );
        assert_eq!(credential_id("b64.AQID").unwrap(), vec![1, 2, 3]);
        assert!(credential_id("b64.").is_err());
        assert!(credential_id("not an id").is_err());
        let new = new_credential_id();
        assert_eq!(credential_id(&new).unwrap().len(), 16);
    }

    /// Making one: the attested credential data and the attestation object,
    /// byte by byte.
    #[test]
    fn making_one_attests_the_key() {
        let key = Key(rfc_key());
        let id = credential_id("09c5ee5f-0845-48e6-aef4-c7c443317de5").unwrap();
        let attested = Attested {
            credential_id: &id,
            public: key.public(),
        };
        let data = authenticator_data(
            "example.test",
            flags::USER_PRESENT | flags::USER_VERIFIED | flags::BACKUP_ELIGIBLE | flags::BACKED_UP,
            0,
            Some(&attested),
        );
        assert_eq!(data[32], 0x5d);
        assert_eq!(&data[37..53], &AAGUID);
        assert_eq!(&data[53..55], &[0, 16]);
        assert_eq!(&data[55..71], id.as_slice());
        let (x, y) = key.public();
        let cose = &data[71..];
        assert_eq!(cose.len(), 77);
        assert_eq!(
            &cose[..10],
            &[0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20]
        );
        assert_eq!(&cose[10..42], &x);
        assert_eq!(&cose[42..45], &[0x22, 0x58, 0x20]);
        assert_eq!(&cose[45..], &y);

        let object = attestation_object(&data);
        let mut head = vec![0xa3, 0x63];
        head.extend_from_slice(b"fmt");
        head.push(0x64);
        head.extend_from_slice(b"none");
        head.push(0x67);
        head.extend_from_slice(b"attStmt");
        head.extend_from_slice(&[0xa0, 0x68]);
        head.extend_from_slice(b"authData");
        // 148 bytes of authenticator data: one length byte after 0x58.
        assert_eq!(data.len(), 148);
        head.extend_from_slice(&[0x58, 148]);
        assert_eq!(&object[..head.len()], head.as_slice());
        assert_eq!(&object[head.len()..], data.as_slice());
    }
}
