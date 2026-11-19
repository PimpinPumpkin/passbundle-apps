//! A security key's PIN, sent as CTAP 2.1 has a platform send it (FIDO
//! Alliance, "Client to Authenticator Protocol (CTAP)" 2.1:
//! "authenticatorClientPIN", "PIN/UV Auth Protocol One" and "Two"; DESIGN.md
//! in the meta repo, "A security key's PIN"), from the public specification
//! only, and checked against the meta repo's vectors
//! (docs/vectors/security-key-pin-v1.json, copied into tests/vectors):
//!
//! - only when a key says it needs it: ctap.rs asks for a touch first, and
//!   comes here when the key answers that it wants its PIN, or when the
//!   server requires user verification;
//! - the key's tries read first (getPINRetries), and its last one used only
//!   when the person says so;
//! - ECDH on P-256 with a key of the core's own, new for each try, then the
//!   shared secret's KDF, AES-256-CBC, and HMAC-SHA-256 as the protocol the
//!   key lists has them (2 when it lists it, else 1);
//! - a token with the `ga` permission for the site ID (getPinToken for a
//!   CTAP 2.0 key), and getAssertion's pinUvAuthParam from it.
//!
//! The PIN, its hash, the shared secret, the core's key-agreement key, and
//! the token are in memory only, for one try, and overwritten when they're
//! dropped. None of them is ever logged, put in an error's words, or sent
//! anywhere but the key.

use std::fmt;

use aes::cipher::{block_padding::NoPadding, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use icu_normalizer::ComposingNormalizerBorrowed;
use p256::{PublicKey, SecretKey};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

use super::cbor::{self, Value};
use super::ctap::{ctap2_error, key_error, Link, Wait};
use crate::error::{Error, KeyPin, Result};

type HmacSha256 = Hmac<Sha256>;
type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

pub const NOT_SET: &str = "This security key needs a PIN, and it doesn't have one yet. Set one with the app from the key's maker, then try again, or use another way to sign in.";
pub const OTHER_CHECK: &str = "This security key wants to check it's you in a way PassBundle can't ask for yet. Use another way to sign in, or the web vault.";
pub const CHANGE_FIRST: &str = "This security key wants its PIN changed before it's used again. Change it with the app from the key's maker, then try again, or use another way to sign in.";
pub const BLOCKED: &str = "This security key's PIN is blocked after too many wrong tries. Only resetting the key with the app from its maker makes it usable again, and that erases it. Use another way to sign in.";
pub const SEVERAL: &str = "More than one security key is plugged in, and one asks for its PIN. Leave only the key for this account plugged in, then try again.";
pub const NOT_TAKEN: &str = "The security key didn't take its PIN this time. Try again.";

/// The most a PIN as typed may be before it's normalized: NFC never makes
/// text more than three times longer, and a key's PIN is 63 bytes at most,
/// so anything longer than this can't be one.
const MOST_TYPED: usize = 1024;

/// A security key's PIN as the person typed it, as CTAP 2.1 has it sent
/// ("PIN Composition Requirements"): in Unicode Normalization Form C, as
/// UTF-8, at least 4 characters (code points) and at most 63 bytes, not
/// ending in a zero byte. Nothing is trimmed or folded. Overwritten when
/// it's dropped; never shown, not even by `Debug`.
pub struct Pin {
    utf8: Zeroizing<String>,
    /// The person said to use the key's last try, if it comes to that.
    pub last_try: bool,
}

impl fmt::Debug for Pin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Pin(..)")
    }
}

impl Pin {
    /// The PIN as typed, or why it can't be a key's PIN (before any key is
    /// asked, so it costs no try).
    pub fn new(typed: &str, last_try: bool) -> Result<Pin> {
        if typed.len() > MOST_TYPED {
            return Err(Error::KeyPin(KeyPin::TooLong));
        }
        // Room for the most NFC can make, so writing into it never moves
        // the PIN to a new buffer and leaves the old one behind.
        let mut utf8 = Zeroizing::new(String::with_capacity(typed.len() * 3 + 4));
        ComposingNormalizerBorrowed::new_nfc()
            .normalize_to(typed, &mut *utf8)
            .map_err(|_| Error::KeyPin(KeyPin::NotAPin))?;
        if utf8.chars().count() < 4 {
            return Err(Error::KeyPin(KeyPin::TooShort));
        }
        if utf8.len() > 63 {
            return Err(Error::KeyPin(KeyPin::TooLong));
        }
        if utf8.ends_with('\0') {
            return Err(Error::KeyPin(KeyPin::NotAPin));
        }
        Ok(Pin { utf8, last_try })
    }

    /// What's sent (hashed): the PIN in NFC, as UTF-8.
    pub fn as_bytes(&self) -> &[u8] {
        self.utf8.as_bytes()
    }

    /// The first 16 bytes of SHA-256 of the PIN: what pinHashEnc encrypts.
    pub fn hash(&self) -> Zeroizing<[u8; 16]> {
        let mut digest = Sha256::digest(self.as_bytes());
        let mut out = Zeroizing::new([0u8; 16]);
        out.copy_from_slice(&digest[..16]);
        digest.as_mut_slice().zeroize();
        out
    }
}

/// The PIN/UV auth protocols the core speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    One,
    Two,
}

impl Protocol {
    pub fn number(self) -> u64 {
        match self {
            Protocol::One => 1,
            Protocol::Two => 2,
        }
    }

    /// authenticate(key, message): protocol 1 keeps the first 16 bytes of
    /// HMAC-SHA-256; protocol 2 keeps all 32.
    fn authenticate(self, key: &[u8], message: &[u8]) -> Vec<u8> {
        let mut mac = <HmacSha256 as Mac>::new_from_slice(key).expect("HMAC takes any key");
        mac.update(message);
        let tag = mac.finalize().into_bytes();
        match self {
            Protocol::One => tag[..16].to_vec(),
            Protocol::Two => tag.to_vec(),
        }
    }
}

/// The shared secret of a key agreement (each protocol's kdf of Z, the
/// x-coordinate of the ECDH point): protocol 1, SHA-256(Z), one 32-byte key
/// for both AES and HMAC; protocol 2, HKDF-SHA-256 with a salt of 32 zero
/// bytes, info "CTAP2 HMAC key" for 32 bytes, then "CTAP2 AES key" for 32
/// more (two calls, which isn't one call for 64 bytes).
pub struct SharedSecret {
    protocol: Protocol,
    hmac_key: Zeroizing<[u8; 32]>,
    aes_key: Zeroizing<[u8; 32]>,
}

impl fmt::Debug for SharedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SharedSecret({:?}, ..)", self.protocol)
    }
}

impl SharedSecret {
    pub fn new(protocol: Protocol, z: &[u8]) -> SharedSecret {
        let mut hmac_key = Zeroizing::new([0u8; 32]);
        let mut aes_key = Zeroizing::new([0u8; 32]);
        match protocol {
            Protocol::One => {
                let mut digest = Sha256::digest(z);
                hmac_key.copy_from_slice(&digest);
                aes_key.copy_from_slice(&digest);
                digest.as_mut_slice().zeroize();
            }
            Protocol::Two => {
                let kdf = Hkdf::<Sha256>::new(Some(&[0u8; 32]), z);
                kdf.expand(b"CTAP2 HMAC key", &mut hmac_key[..])
                    .expect("32 bytes is within HKDF's reach");
                kdf.expand(b"CTAP2 AES key", &mut aes_key[..])
                    .expect("32 bytes is within HKDF's reach");
            }
        }
        SharedSecret {
            protocol,
            hmac_key,
            aes_key,
        }
    }

    pub fn protocol(&self) -> Protocol {
        self.protocol
    }

    /// The secret as the specification writes it: 32 bytes for protocol 1,
    /// the HMAC key and the AES key for protocol 2.
    pub fn to_bytes(&self) -> Zeroizing<Vec<u8>> {
        match self.protocol {
            Protocol::One => Zeroizing::new(self.aes_key.to_vec()),
            Protocol::Two => Zeroizing::new([&self.hmac_key[..], &self.aes_key[..]].concat()),
        }
    }

    /// encrypt(secret, plaintext), no padding (the plaintext is whole AES
    /// blocks): protocol 1 with an all-zero IV; protocol 2 under the AES key
    /// with `iv` (random, new each time), which goes in front.
    pub fn encrypt(&self, plain: &[u8], iv: [u8; 16]) -> Vec<u8> {
        debug_assert!(plain.len().is_multiple_of(16));
        let iv = match self.protocol {
            Protocol::One => [0u8; 16],
            Protocol::Two => iv,
        };
        let cipher = Aes256CbcEnc::new_from_slices(&self.aes_key[..], &iv)
            .expect("a 32-byte key and a 16-byte IV");
        let body = cipher.encrypt_padded_vec_mut::<NoPadding>(plain);
        match self.protocol {
            Protocol::One => body,
            Protocol::Two => [&iv[..], &body].concat(),
        }
    }

    /// decrypt(secret, ciphertext): whole AES blocks, after protocol 2's IV.
    pub fn decrypt(&self, data: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        let (iv, body) = match self.protocol {
            Protocol::One => ([0u8; 16], data),
            Protocol::Two => {
                if data.len() < 16 {
                    return Err(key_error());
                }
                let (iv, body) = data.split_at(16);
                (iv.try_into().expect("16 bytes"), body)
            }
        };
        if body.is_empty() || !body.len().is_multiple_of(16) {
            return Err(key_error());
        }
        let cipher = Aes256CbcDec::new_from_slices(&self.aes_key[..], &iv)
            .expect("a 32-byte key and a 16-byte IV");
        cipher
            .decrypt_padded_vec_mut::<NoPadding>(body)
            .map(Zeroizing::new)
            .map_err(|_| key_error())
    }
}

/// A key's P-256 public key as CTAP2's key agreement carries it: a COSE_Key
/// of type EC2 (1: 2), with the algorithm the specification names (3: -25,
/// ECDH-ES+HKDF-256, though neither protocol uses it), P-256 (-1: 1), x
/// (-2), and y (-3).
pub fn cose_key(public: &PublicKey) -> Value {
    use p256::elliptic_curve::sec1::ToEncodedPoint;
    let point = public.to_encoded_point(false);
    Value::Map(vec![
        (Value::Unsigned(1), Value::Unsigned(2)),
        (Value::Unsigned(3), Value::Negative(24)),
        (Value::Negative(0), Value::Unsigned(1)),
        (
            Value::Negative(1),
            Value::Bytes(point.x().expect("not the identity").to_vec()),
        ),
        (
            Value::Negative(2),
            Value::Bytes(point.y().expect("uncompressed").to_vec()),
        ),
    ])
}

/// A key's COSE_Key for key agreement: EC2 on P-256, and a point on the
/// curve, or it's refused.
pub fn read_cose_key(value: &Value) -> Result<PublicKey> {
    let kty = value.get_int(1).and_then(Value::as_unsigned);
    let crv = value.get_int(-1).and_then(Value::as_unsigned);
    let x = value.get_int(-2).and_then(Value::as_bytes);
    let y = value.get_int(-3).and_then(Value::as_bytes);
    let (Some(2), Some(1), Some(x), Some(y)) = (kty, crv, x, y) else {
        return Err(key_error());
    };
    if x.len() != 32 || y.len() != 32 {
        return Err(key_error());
    }
    let point = [&[0x04][..], x, y].concat();
    PublicKey::from_sec1_bytes(&point).map_err(|_| key_error())
}

/// The core's side of a key agreement with a key, for one try: its own
/// P-256 key's public half, which the key is sent, and the shared secret.
pub struct Agreement {
    secret: SharedSecret,
    platform: PublicKey,
}

impl Agreement {
    /// With the key's public key (getKeyAgreement), from a key of the
    /// core's own (`platform`, new for each try).
    pub fn new(protocol: Protocol, key: &PublicKey, platform: &SecretKey) -> Agreement {
        let mut scalar = platform.to_nonzero_scalar();
        let shared = p256::ecdh::diffie_hellman(&scalar, key.as_affine());
        scalar.zeroize();
        Agreement {
            secret: SharedSecret::new(protocol, shared.raw_secret_bytes()),
            platform: platform.public_key(),
        }
    }

    pub fn secret(&self) -> &SharedSecret {
        &self.secret
    }

    /// The command for a token with the PIN's hash: with `rp_id`,
    /// getPinUvAuthTokenUsingPinWithPermissions (0x09) with the `ga`
    /// permission for that site ID; without, getPinToken (0x05), as a CTAP
    /// 2.0 key takes it. `iv` is protocol 2's for pinHashEnc.
    pub fn token_request(&self, pin_hash: &[u8; 16], iv: [u8; 16], rp_id: Option<&str>) -> Vec<u8> {
        let protocol = self.secret.protocol.number();
        let mut members = vec![
            (Value::Unsigned(1), Value::Unsigned(protocol)),
            (
                Value::Unsigned(2),
                Value::Unsigned(if rp_id.is_some() { 0x09 } else { 0x05 }),
            ),
            (Value::Unsigned(3), cose_key(&self.platform)),
            (
                Value::Unsigned(6),
                Value::Bytes(self.secret.encrypt(pin_hash, iv)),
            ),
        ];
        if let Some(rp_id) = rp_id {
            // ga: getAssertion.
            members.push((Value::Unsigned(9), Value::Unsigned(0x02)));
            members.push((Value::Unsigned(10), Value::text(rp_id)));
        }
        command(&members)
    }

    /// The token in the key's answer (2: pinUvAuthToken, encrypted with the
    /// shared secret): 32 bytes for protocol 2, 16 or 32 for protocol 1.
    pub fn read_token(&self, answer: &Value) -> Result<Token> {
        let sealed = answer
            .get(2)
            .and_then(Value::as_bytes)
            .ok_or_else(key_error)?;
        let token = self.secret.decrypt(sealed)?;
        let fits = match self.secret.protocol {
            Protocol::One => token.len() == 16 || token.len() == 32,
            Protocol::Two => token.len() == 32,
        };
        if !fits {
            return Err(key_error());
        }
        Ok(Token {
            protocol: self.secret.protocol,
            token,
        })
    }
}

/// A key's PIN token, good for getAssertion until it's dropped.
pub struct Token {
    protocol: Protocol,
    token: Zeroizing<Vec<u8>>,
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Token({:?}, ..)", self.protocol)
    }
}

impl Token {
    pub fn protocol(&self) -> Protocol {
        self.protocol
    }

    /// getAssertion's pinUvAuthParam: authenticate(token, clientDataHash).
    pub fn param(&self, client_data_hash: &[u8; 32]) -> Vec<u8> {
        self.protocol.authenticate(&self.token, client_data_hash)
    }
}

/// authenticatorClientPIN (0x06) with these members.
fn command(members: &[(Value, Value)]) -> Vec<u8> {
    let mut body = vec![0x06];
    body.extend(cbor::encode(&Value::Map(members.to_vec())));
    body
}

/// getPINRetries (0x01).
pub fn get_pin_retries(protocol: Protocol) -> Vec<u8> {
    command(&[
        (Value::Unsigned(1), Value::Unsigned(protocol.number())),
        (Value::Unsigned(2), Value::Unsigned(0x01)),
    ])
}

/// getKeyAgreement (0x02).
pub fn get_key_agreement(protocol: Protocol) -> Vec<u8> {
    command(&[
        (Value::Unsigned(1), Value::Unsigned(protocol.number())),
        (Value::Unsigned(2), Value::Unsigned(0x02)),
    ])
}

/// What a key's authenticatorGetInfo says about its PIN.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Info {
    /// `clientPin`: Some(true) with a PIN set, Some(false) for none set yet,
    /// None for a key that takes no PIN.
    pub client_pin: Option<bool>,
    /// `pinUvAuthToken`: it gives tokens with permissions
    /// (getPinUvAuthTokenUsingPinWithPermissions).
    pub permissions: bool,
    /// `noMcGaPermissionsWithClientPin`: a PIN's token can't sign.
    pub no_signing_with_pin: bool,
    /// forcePINChange (0x0C).
    pub force_pin_change: bool,
    /// pinUvAuthProtocols (0x06), in the key's order.
    pub protocols: Vec<u64>,
}

impl Info {
    /// authenticatorGetInfo's answer.
    pub fn read(info: &Value) -> Info {
        let option = |name: &str| match info.get(4).and_then(|o| o.get_text(name)) {
            Some(Value::Bool(b)) => Some(*b),
            _ => None,
        };
        let protocols = match info.get(6) {
            Some(Value::Array(items)) => items.iter().filter_map(Value::as_unsigned).collect(),
            _ => Vec::new(),
        };
        Info {
            client_pin: option("clientPin"),
            permissions: option("pinUvAuthToken") == Some(true),
            no_signing_with_pin: option("noMcGaPermissionsWithClientPin") == Some(true),
            force_pin_change: matches!(info.get(0x0c), Some(Value::Bool(true))),
            protocols,
        }
    }

    /// Protocol 2 when the key lists it, else protocol 1 (which a CTAP 2.0
    /// key, listing none, speaks); None for a key that lists only others.
    pub fn protocol(&self) -> Option<Protocol> {
        if self.protocols.contains(&2) {
            Some(Protocol::Two)
        } else if self.protocols.is_empty() || self.protocols.contains(&1) {
            Some(Protocol::One)
        } else {
            None
        }
    }
}

/// A command's answer: what the key said with success, or its status.
fn call(link: &mut dyn Link, body: &[u8], wait: &Wait) -> Result<std::result::Result<Value, u8>> {
    let answer = link.cbor(body, wait)?;
    let (&status, rest) = answer.split_first().ok_or_else(key_error)?;
    if status != 0x00 {
        return Ok(Err(status));
    }
    if rest.is_empty() {
        return Ok(Ok(Value::Map(Vec::new())));
    }
    cbor::decode(rest).map(Ok).map_err(|_| key_error())
}

/// The key's authenticatorGetInfo.
pub(super) fn get_info(link: &mut dyn Link, wait: &Wait) -> Result<Info> {
    match call(link, &[0x04], wait)? {
        Ok(info) => Ok(Info::read(&info)),
        Err(status) => Err(ctap2_error(status)),
    }
}

/// getPINRetries: the tries left, and whether the key needs a fresh start
/// (unplugged, or taken away from the phone) before the next.
fn retries(link: &mut dyn Link, protocol: Protocol, wait: &Wait) -> Result<(u32, bool)> {
    match call(link, &get_pin_retries(protocol), wait)? {
        Ok(answer) => {
            let left = answer
                .get(3)
                .and_then(Value::as_unsigned)
                .ok_or_else(key_error)?;
            let fresh_start = matches!(answer.get(4), Some(Value::Bool(true)));
            Ok((u32::try_from(left).unwrap_or(u32::MAX), fresh_start))
        }
        Err(status) => Err(refused(status)),
    }
}

/// What a key answered a PIN command with, other than success.
fn refused(status: u8) -> Error {
    match status {
        // PIN_INVALID (without the count; `token` reads it).
        0x31 => Error::KeyPin(KeyPin::Wrong { left: None }),
        // PIN_BLOCKED
        0x32 => Error::TwoStep(BLOCKED),
        // PIN_AUTH_BLOCKED: 3 wrong in a row.
        0x34 => Error::KeyPin(KeyPin::FreshStart),
        // PIN_NOT_SET
        0x35 => Error::TwoStep(NOT_SET),
        // PIN_POLICY_VIOLATION: it wants its PIN changed first.
        0x37 => Error::TwoStep(CHANGE_FIRST),
        // UNAUTHORIZED_PERMISSION
        0x40 => Error::TwoStep(OTHER_CHECK),
        other => ctap2_error(other),
    }
}

/// A token to sign for `rp_id` with, from the person's PIN; or, without a
/// PIN, why not: `KeyPin::Asks` with the key's tries left, when the app can
/// ask for it. `info` is the key's authenticatorGetInfo. The PIN goes to
/// the key only after its tries are read: never with none left, never on
/// its last one unless the person said so, and never to a key that needs a
/// fresh start or a new PIN first.
pub(super) fn token(
    link: &mut dyn Link,
    info: &Info,
    rp_id: &str,
    pin: Option<&Pin>,
    wait: &Wait,
) -> Result<Token> {
    match info.client_pin {
        None => return Err(Error::TwoStep(OTHER_CHECK)),
        Some(false) => return Err(Error::TwoStep(NOT_SET)),
        Some(true) => {}
    }
    if info.no_signing_with_pin {
        return Err(Error::TwoStep(OTHER_CHECK));
    }
    let protocol = info.protocol().ok_or(Error::TwoStep(OTHER_CHECK))?;
    let (left, fresh_start) = retries(link, protocol, wait)?;
    if left == 0 {
        return Err(Error::TwoStep(BLOCKED));
    }
    if fresh_start {
        return Err(Error::KeyPin(KeyPin::FreshStart));
    }
    if info.force_pin_change {
        return Err(Error::TwoStep(CHANGE_FIRST));
    }
    let Some(pin) = pin else {
        return Err(Error::KeyPin(KeyPin::Asks { left: Some(left) }));
    };
    if left == 1 && !pin.last_try {
        return Err(Error::KeyPin(KeyPin::LastTry));
    }
    let key = match call(link, &get_key_agreement(protocol), wait)? {
        Ok(answer) => read_cose_key(answer.get(1).ok_or_else(key_error)?)?,
        Err(status) => return Err(refused(status)),
    };
    let platform = SecretKey::random(&mut rand::rngs::OsRng);
    let agreement = Agreement::new(protocol, &key, &platform);
    drop(platform);
    let request = agreement.token_request(
        &pin.hash(),
        rand::random(),
        info.permissions.then_some(rp_id),
    );
    match call(link, &request, wait)? {
        Ok(answer) => agreement.read_token(&answer),
        Err(0x31) => {
            // The count is the key's own, read again.
            let left = retries(link, protocol, wait).ok().map(|(left, _)| left);
            Err(match left {
                Some(0) => Error::TwoStep(BLOCKED),
                left => Error::KeyPin(KeyPin::Wrong { left }),
            })
        }
        Err(status) => Err(refused(status)),
    }
}

#[cfg(test)]
mod tests {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use serde_json::Value as Json;

    use super::*;

    fn vectors() -> Json {
        serde_json::from_str(include_str!("../../tests/vectors/security-key-pin-v1.json")).unwrap()
    }

    fn b64(v: &Json) -> Vec<u8> {
        STANDARD.decode(v.as_str().unwrap()).unwrap()
    }

    fn secret_key(v: &Json) -> SecretKey {
        SecretKey::from_slice(&b64(v)).unwrap()
    }

    #[test]
    fn the_vectors_key_agreement_secret_pin_hash_token_and_param_for_each_protocol() {
        let vectors = vectors();
        for (name, protocol) in [("protocol1", Protocol::One), ("protocol2", Protocol::Two)] {
            let v = &vectors[name];
            assert_eq!(v["protocol"], protocol.number(), "{name}");
            let key_private = secret_key(&v["keyPrivateKey"]);
            let platform = secret_key(&v["platformPrivateKey"]);
            let key_public = PublicKey::from_sec1_bytes(&b64(&v["keyPublicKey"])).unwrap();
            assert_eq!(key_private.public_key(), key_public, "{name}");
            // The key's answer to getKeyAgreement, read as the core reads it.
            let answer = b64(&v["answers"]["getKeyAgreement"]);
            assert_eq!(answer[0], 0x00);
            let read = read_cose_key(cbor::decode(&answer[1..]).unwrap().get(1).unwrap()).unwrap();
            assert_eq!(read, key_public, "{name}");
            // The core's own key, as the key is sent it.
            assert_eq!(
                cbor::encode(&cose_key(&platform.public_key())),
                b64(&v["platformCoseKey"]),
                "{name}"
            );
            let agreement = Agreement::new(protocol, &read, &platform);
            assert_eq!(
                *agreement.secret().to_bytes(),
                b64(&v["sharedSecret"]),
                "{name}"
            );
            // Z, from either side.
            let z = p256::ecdh::diffie_hellman(
                key_private.to_nonzero_scalar(),
                platform.public_key().as_affine(),
            );
            assert_eq!(z.raw_secret_bytes().to_vec(), b64(&v["z"]), "{name}");

            let pin = Pin::new(v["pinTyped"].as_str().unwrap(), false).unwrap();
            assert_eq!(pin.as_bytes(), b64(&v["pinUtf8"]), "{name}");
            assert_eq!(pin.as_bytes(), v["pin"].as_str().unwrap().as_bytes());
            assert_eq!(pin.hash().to_vec(), b64(&v["pinHash"]), "{name}");
            let iv: [u8; 16] = match protocol {
                Protocol::One => [0x55; 16], // unused
                Protocol::Two => b64(&v["pinHashIv"]).try_into().unwrap(),
            };
            let hash_enc = agreement.secret().encrypt(&pin.hash()[..], iv);
            assert_eq!(hash_enc, b64(&v["pinHashEnc"]), "{name}");
            assert_eq!(
                &agreement.secret().decrypt(&hash_enc).unwrap()[..],
                &pin.hash()[..]
            );

            // The requests, byte for byte.
            let requests = &v["requests"];
            assert_eq!(
                get_pin_retries(protocol),
                b64(&requests["getPinRetries"]),
                "{name}"
            );
            assert_eq!(
                get_key_agreement(protocol),
                b64(&requests["getKeyAgreement"]),
                "{name}"
            );
            assert_eq!(
                agreement.token_request(&pin.hash(), iv, None),
                b64(&requests["getPinToken"]),
                "{name}"
            );
            assert_eq!(
                agreement.token_request(&pin.hash(), iv, Some(v["rpId"].as_str().unwrap())),
                b64(&requests["getPinUvAuthTokenUsingPinWithPermissions"]),
                "{name}"
            );

            // The key's token opens, and makes the param.
            let answer = b64(&v["answers"]["token"]);
            assert_eq!(answer[0], 0x00);
            let token = agreement
                .read_token(&cbor::decode(&answer[1..]).unwrap())
                .unwrap();
            assert_eq!(*token.token, b64(&v["token"]), "{name}");
            assert_eq!(token.protocol(), protocol);
            let hash: [u8; 32] = b64(&v["clientDataHash"]).try_into().unwrap();
            assert_eq!(token.param(&hash), b64(&v["pinUvAuthParam"]), "{name}");
            assert_eq!(
                super::super::ctap::get_assertion_request(
                    v["rpId"].as_str().unwrap(),
                    &hash,
                    &[b64(&v["credentialId"])],
                    Some(&token),
                ),
                b64(&requests["getAssertion"]),
                "{name}"
            );

            // getPINRetries' answer.
            let answer = b64(&v["answers"]["getPinRetries"]);
            let read = cbor::decode(&answer[1..]).unwrap();
            assert_eq!(read.get(3).and_then(Value::as_unsigned), Some(7));
        }
        // A CTAP 2.0 key's 16-byte token, under protocol 1.
        let v = &vectors["protocol1"];
        let agreement = Agreement::new(
            Protocol::One,
            &PublicKey::from_sec1_bytes(&b64(&v["keyPublicKey"])).unwrap(),
            &secret_key(&v["platformPrivateKey"]),
        );
        let answer = Value::Map(vec![(
            Value::Unsigned(2),
            Value::Bytes(b64(&v["token16Enc"])),
        )]);
        let token = agreement.read_token(&answer).unwrap();
        assert_eq!(*token.token, b64(&v["token16"]));
        let hash: [u8; 32] = b64(&v["clientDataHash"]).try_into().unwrap();
        assert_eq!(token.param(&hash), b64(&v["pinUvAuthParam16"]));
    }

    #[test]
    fn pins_are_normalized_and_checked_as_the_vectors_say() {
        for pin in vectors()["pins"]["pins"].as_array().unwrap() {
            let typed = pin["typed"].as_str().unwrap();
            let made = Pin::new(typed, false);
            if pin["ok"] == true {
                assert_eq!(made.unwrap().as_bytes(), b64(&pin["utf8"]), "{typed:?}");
            } else {
                let why = match pin["why"].as_str().unwrap() {
                    "tooShort" => KeyPin::TooShort,
                    "tooLong" => KeyPin::TooLong,
                    "notAPin" => KeyPin::NotAPin,
                    other => panic!("{other}"),
                };
                assert_eq!(made.unwrap_err(), Error::KeyPin(why), "{typed:?}");
            }
        }
        // Far too long, before it's even normalized.
        assert_eq!(
            Pin::new(&"1".repeat(MOST_TYPED + 1), false).unwrap_err(),
            Error::KeyPin(KeyPin::TooLong)
        );
        // Never shown.
        let pin = Pin::new("2580", true).unwrap();
        assert_eq!(format!("{pin:?}"), "Pin(..)");
        assert!(pin.last_try);
    }

    #[test]
    fn what_the_vectors_refuse_is_refused() {
        let vectors = vectors();
        let refused = &vectors["refused"];
        assert!(PublicKey::from_sec1_bytes(&b64(&refused["offCurveKey"])).is_err());
        let cose = cbor::decode(&b64(&refused["offCurveCoseKey"])).unwrap();
        assert_eq!(read_cose_key(&cose), Err(key_error()));
        let answer = b64(&refused["offCurveAnswer"]);
        assert!(read_cose_key(cbor::decode(&answer[1..]).unwrap().get(1).unwrap()).is_err());
        for (name, protocol, field) in [
            ("protocol1", Protocol::One, "protocol1Token48Enc"),
            ("protocol2", Protocol::Two, "protocol2Token16Enc"),
            ("protocol2", Protocol::Two, "protocol2Short"),
        ] {
            let v = &vectors[name];
            let agreement = Agreement::new(
                protocol,
                &PublicKey::from_sec1_bytes(&b64(&v["keyPublicKey"])).unwrap(),
                &secret_key(&v["platformPrivateKey"]),
            );
            let answer = Value::Map(vec![(
                Value::Unsigned(2),
                Value::Bytes(b64(&refused[field])),
            )]);
            assert_eq!(
                agreement.read_token(&answer).unwrap_err(),
                key_error(),
                "{field}"
            );
        }
        // A COSE key of another type or curve, or the wrong lengths.
        let platform = SecretKey::from_slice(&[7; 32]).unwrap();
        let good = cose_key(&platform.public_key());
        assert!(read_cose_key(&good).is_ok());
        let Value::Map(pairs) = good else {
            unreachable!()
        };
        for (at, wrong) in [
            (0, Value::Unsigned(3)),
            (2, Value::Unsigned(2)),
            (3, Value::Bytes(vec![1; 31])),
            (4, Value::Text("y".into())),
        ] {
            let mut changed = pairs.clone();
            changed[at].1 = wrong;
            assert!(read_cose_key(&Value::Map(changed)).is_err(), "{at}");
        }
    }

    #[test]
    fn reads_what_a_key_says_about_its_pin() {
        let info = |options: Vec<(&str, bool)>, protocols: Option<Vec<u64>>| {
            let mut members = vec![(
                Value::Unsigned(4),
                Value::Map(
                    options
                        .into_iter()
                        .map(|(k, v)| (Value::text(k), Value::Bool(v)))
                        .collect(),
                ),
            )];
            if let Some(protocols) = protocols {
                members.push((
                    Value::Unsigned(6),
                    Value::Array(protocols.into_iter().map(Value::Unsigned).collect()),
                ));
            }
            Info::read(&Value::Map(members))
        };
        let both = info(
            vec![("clientPin", true), ("pinUvAuthToken", true)],
            Some(vec![2, 1]),
        );
        assert_eq!(both.client_pin, Some(true));
        assert!(both.permissions);
        assert_eq!(both.protocol(), Some(Protocol::Two));
        // Protocol 2 wherever it's listed, even second.
        assert_eq!(
            info(vec![], Some(vec![1, 2])).protocol(),
            Some(Protocol::Two)
        );
        assert_eq!(info(vec![], Some(vec![1])).protocol(), Some(Protocol::One));
        // A CTAP 2.0 key lists none, and speaks 1.
        let old = info(vec![("clientPin", false)], None);
        assert_eq!(old.protocol(), Some(Protocol::One));
        assert_eq!(old.client_pin, Some(false));
        assert!(!old.permissions);
        assert_eq!(info(vec![], Some(vec![3])).protocol(), None);
        assert_eq!(info(vec![], None).client_pin, None);
        assert!(info(vec![("noMcGaPermissionsWithClientPin", true)], None).no_signing_with_pin);
        let force = Info::read(&Value::Map(vec![(
            Value::Unsigned(0x0c),
            Value::Bool(true),
        )]));
        assert!(force.force_pin_change);
    }
}
