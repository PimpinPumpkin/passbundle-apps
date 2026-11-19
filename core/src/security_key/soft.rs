//! A security key in software, for tests only (unit tests, and builds with
//! the test-hooks feature, which the core's server tests use; never a
//! release): CTAP2's getAssertion and U2F's authenticate over CTAPHID's
//! reports or NFC's APDUs, with one P-256 credential for one site ID, as a
//! real key keeps it. It can be slow to be touched, take one credential at a
//! time, speak only U2F, and talk over other channels' traffic, so the
//! core's side is tested against each. It can have a PIN (CTAP 2.1's
//! authenticatorClientPIN, as an authenticator does it, written apart from
//! the core's side in client_pin.rs): set or not, wanted for every
//! signature (`alwaysUv`) or not, CTAP 2.0's tokens or 2.1's with
//! permissions, PIN/UV auth protocols 1, 2, or both, with its tries, three
//! wrong in a row wanting a fresh start, and blocked at none left.

use std::collections::VecDeque;
use std::time::Duration;

use aes::cipher::{block_padding::NoPadding, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use hmac::{Hmac, Mac};
use p256::ecdsa::signature::{Signer, Verifier};
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::{PublicKey, SecretKey};
use sha2::{Digest, Sha256};

use super::cbor::{self, Value};
use super::ctap::{Hid, Nfc};
use crate::error::Result;

/// The tries a PIN starts with, and gets back after a right one.
pub const PIN_TRIES: u32 = 8;

pub struct SoftKey {
    /// The site ID (or AppID) its one credential was made for.
    pub site: String,
    pub credential_id: Vec<u8>,
    key: SigningKey,
    counter: u32,
    /// Speaks CTAP2 (else only U2F).
    pub ctap2: bool,
    /// Its PIN, if one is set.
    pub pin: Option<String>,
    /// It takes a PIN at all (getInfo's `clientPin` is there).
    pub takes_pin: bool,
    /// Wants its PIN for every signature (`alwaysUv`).
    pub always_uv: bool,
    /// CTAP 2.1: tokens with permissions (getPinUvAuthTokenUsingPinWithPermissions),
    /// tied to the first site ID they're used with. Else CTAP 2.0's getPinToken.
    pub ctap21: bool,
    /// The PIN/UV auth protocols it lists; none, as a CTAP 2.0 key lists, means 1.
    pub protocols: Vec<u64>,
    /// Its PIN's tries left.
    pub pin_tries: u32,
    /// Wants its PIN changed first (forcePINChange).
    pub force_pin_change: bool,
    /// The clientPIN subcommands it heard, in order.
    pub heard: Vec<u64>,
    /// How many PIN hashes reached it.
    pub pin_hashes: u32,
    /// The protocol of the last token it gave.
    pub token_protocol: Option<u64>,
    wrong_in_a_row: u32,
    key_agreement: SecretKey,
    /// Its token: the bytes, its protocol, the site ID it's tied to, and its
    /// permissions.
    token: Option<(Vec<u8>, u64, Option<String>, u8)>,
    /// How many of the core's reads (or U2F asks) pass before it's touched.
    pub touch_after: u32,
    /// The most credentials it takes in one request.
    pub most_in_list: usize,
    /// Answers on another channel come between its own.
    pub noise: bool,
    /// It heard CTAPHID_CANCEL while waiting for a touch.
    pub canceled: bool,
    /// The longest message it was sent.
    pub longest_message: usize,
    cid: u32,
    out: VecDeque<[u8; 64]>,
    incoming: Option<(u8, usize, Vec<u8>, u8)>,
    /// A CTAP2 answer waiting for the touch, and the reads so far.
    waiting: Option<(Vec<u8>, u32)>,
    u2f_asks: u32,
}

fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

impl SoftKey {
    pub fn new(site: &str) -> SoftKey {
        SoftKey {
            site: site.to_owned(),
            credential_id: (0..112).map(|_| rand::random::<u8>()).collect(),
            key: SigningKey::random(&mut rand::rngs::OsRng),
            counter: 0,
            ctap2: true,
            pin: None,
            takes_pin: true,
            always_uv: false,
            ctap21: true,
            protocols: vec![2, 1],
            pin_tries: PIN_TRIES,
            force_pin_change: false,
            heard: Vec::new(),
            pin_hashes: 0,
            token_protocol: None,
            wrong_in_a_row: 0,
            key_agreement: SecretKey::random(&mut rand::rngs::OsRng),
            token: None,
            touch_after: 2,
            most_in_list: 8,
            noise: false,
            canceled: false,
            longest_message: 0,
            cid: 0x0102_0304,
            out: VecDeque::new(),
            incoming: None,
            waiting: None,
            u2f_asks: 0,
        }
    }

    /// A key with this PIN set, that wants it for every signature.
    pub fn with_pin(site: &str, pin: &str) -> SoftKey {
        let mut key = SoftKey::new(site);
        key.pin = Some(pin.to_owned());
        key.always_uv = true;
        key
    }

    /// Unplugged and plugged in again (or held to the phone again): a new
    /// key agreement, no token, and the count of wrong PINs in a row gone.
    pub fn replug(&mut self) {
        self.wrong_in_a_row = 0;
        self.key_agreement = SecretKey::random(&mut rand::rngs::OsRng);
        self.token = None;
    }

    /// The credential's public key, as its x and y.
    pub fn public(&self) -> ([u8; 32], [u8; 32]) {
        let point = self.key.verifying_key().to_encoded_point(false);
        (
            point.x().unwrap().as_slice().try_into().unwrap(),
            point.y().unwrap().as_slice().try_into().unwrap(),
        )
    }

    /// Whether a signature is this key's, over the data and the hash.
    pub fn verifies(
        &self,
        authenticator_data: &[u8],
        client_data_hash: &[u8; 32],
        signature: &[u8],
    ) -> bool {
        let Ok(signature) = Signature::from_der(signature) else {
            return false;
        };
        let message = [authenticator_data, client_data_hash].concat();
        VerifyingKey::from(&self.key)
            .verify(&message, &signature)
            .is_ok()
    }

    fn sign(&mut self, app: [u8; 32], client_data_hash: &[u8], flags: u8) -> (Vec<u8>, Vec<u8>) {
        self.counter += 1;
        let mut data = app.to_vec();
        data.push(flags);
        data.extend(self.counter.to_be_bytes());
        let signature: Signature = self.key.sign(&[&data[..], client_data_hash].concat());
        (data, signature.to_der().as_bytes().to_vec())
    }

    /// CTAP2's getAssertion: the answer, or None to wait for a touch first.
    fn get_assertion(&mut self, body: &[u8]) -> (Vec<u8>, bool) {
        let Ok(request) = cbor::decode(body) else {
            return (vec![0x12], false);
        };
        let rp_id = request
            .get(1)
            .and_then(Value::as_text)
            .unwrap_or_default()
            .to_owned();
        let hash = request
            .get(2)
            .and_then(Value::as_bytes)
            .unwrap_or_default()
            .to_vec();
        let allow: Vec<Vec<u8>> = match request.get(3) {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|i| {
                    i.get_text("id")
                        .and_then(Value::as_bytes)
                        .map(<[u8]>::to_vec)
                })
                .collect(),
            _ => Vec::new(),
        };
        // The person verified with the PIN's token, checked as CTAP 2.1's
        // getAssertion checks it; or none, for a key that always wants it.
        let mut flags = 0x01;
        match request.get(6).and_then(Value::as_bytes) {
            Some(param) => {
                let protocol = request.get(7).and_then(Value::as_unsigned);
                let Some((token, made_for, tied_to, permissions)) = &mut self.token else {
                    return (vec![0x33], false);
                };
                if protocol != Some(*made_for)
                    || param != authenticate(*made_for, token, &hash)
                    || *permissions & 0x02 == 0
                    || tied_to.as_ref().is_some_and(|id| *id != rp_id)
                {
                    return (vec![0x33], false);
                }
                if self.ctap21 {
                    *tied_to = Some(rp_id.clone());
                }
                flags |= 0x04;
            }
            None if self.always_uv => return (vec![0x36], false),
            None => {}
        }
        if allow.len() > self.most_in_list {
            return (vec![0x15], false);
        }
        if rp_id != self.site || !allow.contains(&self.credential_id) {
            return (vec![0x2e], false);
        }
        // Used: a CTAP 2.1 key's token keeps no permissions.
        if flags & 0x04 != 0 && self.ctap21 {
            if let Some(token) = &mut self.token {
                token.3 = 0;
            }
        }
        let (data, signature) = self.sign(sha256(rp_id.as_bytes()), &hash, flags);
        let answer = Value::Map(vec![
            (
                Value::Unsigned(1),
                Value::Map(vec![
                    (Value::text("id"), Value::Bytes(self.credential_id.clone())),
                    (Value::text("type"), Value::text("public-key")),
                ]),
            ),
            (Value::Unsigned(2), Value::Bytes(data)),
            (Value::Unsigned(3), Value::Bytes(signature)),
        ]);
        ([vec![0x00], cbor::encode(&answer)].concat(), true)
    }

    /// U2F's authenticate, as an APDU's data and status word.
    fn u2f(&mut self, apdu: &[u8], touched: bool) -> Vec<u8> {
        // Extended (00 Lc1 Lc2) or short (Lc) length.
        let data = if apdu.len() > 7 && apdu[4] == 0 {
            &apdu[7..7 + u16::from_be_bytes([apdu[5], apdu[6]]) as usize]
        } else {
            &apdu[5..5 + apdu[4] as usize]
        };
        if apdu[1] != 0x02 || data.len() < 65 {
            return vec![0x6d, 0x00];
        }
        let (challenge, app, handle) = (&data[..32], &data[32..64], &data[65..]);
        if app != sha256(self.site.as_bytes()) || handle != self.credential_id {
            return vec![0x6a, 0x80];
        }
        if apdu[2] == 0x07 || !touched {
            return vec![0x69, 0x85];
        }
        let (data, signature) = self.sign(app.try_into().unwrap(), challenge, 0x01);
        [&data[32..], &signature[..], &[0x90, 0x00]].concat()
    }

    /// authenticatorGetInfo: the versions it speaks, and its PIN's options.
    fn get_info(&self) -> Vec<u8> {
        let mut versions = vec![Value::text("U2F_V2"), Value::text("FIDO_2_0")];
        let mut options = vec![
            (Value::text("rk"), Value::Bool(false)),
            (Value::text("up"), Value::Bool(true)),
        ];
        if self.takes_pin {
            options.push((Value::text("clientPin"), Value::Bool(self.pin.is_some())));
        }
        if self.ctap21 {
            versions.push(Value::text("FIDO_2_1"));
            options.push((Value::text("alwaysUv"), Value::Bool(self.always_uv)));
            options.push((Value::text("pinUvAuthToken"), Value::Bool(true)));
        }
        let mut info = vec![
            (Value::Unsigned(1), Value::Array(versions)),
            (Value::Unsigned(3), Value::Bytes(vec![0x5a; 16])),
            (Value::Unsigned(4), Value::Map(options)),
        ];
        if !self.protocols.is_empty() {
            info.push((
                Value::Unsigned(6),
                Value::Array(
                    self.protocols
                        .iter()
                        .copied()
                        .map(Value::Unsigned)
                        .collect(),
                ),
            ));
        }
        if self.ctap21 && self.force_pin_change {
            info.push((Value::Unsigned(0x0c), Value::Bool(true)));
        }
        [vec![0x00], cbor::encode(&Value::Map(info))].concat()
    }

    fn speaks(&self, protocol: u64) -> bool {
        if self.protocols.is_empty() {
            protocol == 1
        } else {
            self.protocols.contains(&protocol)
        }
    }

    /// authenticatorClientPIN, as CTAP 2.1 has an authenticator answer it.
    fn client_pin(&mut self, body: &[u8]) -> Vec<u8> {
        let Ok(request) = cbor::decode(body) else {
            return vec![0x12];
        };
        let Some(sub) = request.get(2).and_then(Value::as_unsigned) else {
            return vec![0x14];
        };
        self.heard.push(sub);
        if !self.takes_pin {
            return vec![0x01];
        }
        let Some(protocol) = request.get(1).and_then(Value::as_unsigned) else {
            return vec![0x14];
        };
        if !self.speaks(protocol) {
            return vec![0x02];
        }
        let ok = |members: Vec<(Value, Value)>| {
            [vec![0x00], cbor::encode(&Value::Map(members))].concat()
        };
        match sub {
            // getPINRetries
            0x01 => {
                let mut members =
                    vec![(Value::Unsigned(3), Value::Unsigned(self.pin_tries as u64))];
                if self.ctap21 {
                    members.push((Value::Unsigned(4), Value::Bool(self.wrong_in_a_row >= 3)));
                }
                ok(members)
            }
            // getKeyAgreement
            0x02 => ok(vec![(
                Value::Unsigned(1),
                cose(&self.key_agreement.public_key()),
            )]),
            // getPinToken, getPinUvAuthTokenUsingPinWithPermissions
            0x05 | 0x09 => {
                if sub == 0x09 && !self.ctap21 {
                    return vec![0x3e];
                }
                let (permissions, tied_to) = if sub == 0x09 {
                    let Some(permissions) = request.get(9).and_then(Value::as_unsigned) else {
                        return vec![0x14];
                    };
                    let rp_id = request.get(10).and_then(Value::as_text).map(str::to_owned);
                    (permissions as u8, rp_id)
                } else {
                    if request.get(9).is_some() || request.get(10).is_some() {
                        return vec![0x02];
                    }
                    // mc and ga.
                    (0x03, None)
                };
                let Some(pin) = self.pin.clone() else {
                    return vec![0x35];
                };
                if self.pin_tries == 0 {
                    return vec![0x32];
                }
                if self.wrong_in_a_row >= 3 {
                    return vec![0x34];
                }
                if self.force_pin_change {
                    return vec![0x37];
                }
                let Some(platform) = request.get(3).and_then(read_cose) else {
                    return vec![0x02];
                };
                let shared = p256::ecdh::diffie_hellman(
                    self.key_agreement.to_nonzero_scalar(),
                    platform.as_affine(),
                );
                let secret = kdf(protocol, shared.raw_secret_bytes());
                let Some(hash) = request
                    .get(6)
                    .and_then(Value::as_bytes)
                    .and_then(|sealed| decrypt(protocol, &secret, sealed))
                else {
                    return vec![0x02];
                };
                self.pin_hashes += 1;
                // Counted before it's checked, as the specification has it.
                self.pin_tries -= 1;
                if hash[..] != sha256(pin.as_bytes())[..16] {
                    self.key_agreement = SecretKey::random(&mut rand::rngs::OsRng);
                    self.wrong_in_a_row += 1;
                    return if self.pin_tries == 0 {
                        vec![0x32]
                    } else if self.wrong_in_a_row >= 3 {
                        vec![0x34]
                    } else {
                        vec![0x31]
                    };
                }
                self.pin_tries = PIN_TRIES;
                self.wrong_in_a_row = 0;
                let token: Vec<u8> = (0..32).map(|_| rand::random::<u8>()).collect();
                let sealed = encrypt(protocol, &secret, &token);
                self.token = Some((token, protocol, tied_to, permissions));
                self.token_protocol = Some(protocol);
                ok(vec![(Value::Unsigned(2), Value::Bytes(sealed))])
            }
            _ => vec![0x3e],
        }
    }

    fn queue(&mut self, cid: u32, command: u8, data: &[u8]) {
        let mut packet = [0u8; 64];
        packet[..4].copy_from_slice(&cid.to_be_bytes());
        packet[4] = command;
        packet[5..7].copy_from_slice(&(data.len() as u16).to_be_bytes());
        let first = data.len().min(57);
        packet[7..7 + first].copy_from_slice(&data[..first]);
        self.out.push_back(packet);
        for (seq, chunk) in data[first..].chunks(59).enumerate() {
            let mut packet = [0u8; 64];
            packet[..4].copy_from_slice(&cid.to_be_bytes());
            packet[4] = seq as u8;
            packet[5..5 + chunk.len()].copy_from_slice(chunk);
            self.out.push_back(packet);
        }
    }

    fn message(&mut self, command: u8, data: Vec<u8>) {
        self.longest_message = self.longest_message.max(data.len());
        if self.noise {
            self.queue(0x0a0b_0c0d, 0xbb, &[1]);
        }
        match command {
            0x90 if self.ctap2 && data.first() == Some(&0x02) => {
                let (answer, wait) = self.get_assertion(&data[1..]);
                if wait {
                    self.waiting = Some((answer, 0));
                } else {
                    self.queue(self.cid, 0x90, &answer);
                }
            }
            0x90 if self.ctap2 && data.first() == Some(&0x04) => {
                let info = self.get_info();
                self.queue(self.cid, 0x90, &info);
            }
            0x90 if self.ctap2 && data.first() == Some(&0x06) => {
                let answer = self.client_pin(&data[1..]);
                self.queue(self.cid, 0x90, &answer);
            }
            0x90 if self.ctap2 => self.queue(self.cid, 0x90, &[0x01]),
            0x83 => {
                self.u2f_asks += 1;
                let touched = self.u2f_asks > self.touch_after;
                let answer = self.u2f(&data, touched);
                self.queue(self.cid, 0x83, &answer);
            }
            0x91 => {
                if self.waiting.take().is_some() {
                    self.canceled = true;
                    self.queue(self.cid, 0x90, &[0x2d]);
                }
            }
            // CTAPHID_ERROR, ERR_INVALID_CMD.
            _ => self.queue(self.cid, 0xbf, &[0x01]),
        }
    }
}

impl Hid for SoftKey {
    fn write(&mut self, report: &[u8; 64]) -> Result<()> {
        let cid = u32::from_be_bytes(report[..4].try_into().unwrap());
        let command = report[4];
        if cid == 0xffff_ffff && command == 0x86 {
            let mut answer = report[7..15].to_vec();
            answer.extend(self.cid.to_be_bytes());
            // Protocol 2, version 5.4.3, and its capabilities: wink, CBOR (if it speaks CTAP2).
            answer.extend([2, 5, 4, 3, 0x01 | if self.ctap2 { 0x04 } else { 0 }]);
            self.queue(0xffff_ffff, 0x86, &answer);
            return Ok(());
        }
        if cid != self.cid {
            return Ok(());
        }
        if command & 0x80 != 0 {
            let length = u16::from_be_bytes([report[5], report[6]]) as usize;
            let data = report[7..7 + length.min(57)].to_vec();
            if data.len() == length {
                self.message(command, data);
            } else {
                self.incoming = Some((command, length, data, 0));
            }
        } else if let Some((command, length, mut data, seq)) = self.incoming.take() {
            assert_eq!(report[4], seq, "continuation packets in order");
            let take = (length - data.len()).min(59);
            data.extend_from_slice(&report[5..5 + take]);
            if data.len() == length {
                self.message(command, data);
            } else {
                self.incoming = Some((command, length, data, seq + 1));
            }
        }
        Ok(())
    }

    fn read(&mut self, timeout: Duration) -> Result<Option<[u8; 64]>> {
        if let Some(packet) = self.out.pop_front() {
            return Ok(Some(packet));
        }
        if let Some((answer, reads)) = self.waiting.take() {
            if reads >= self.touch_after {
                self.queue(self.cid, 0x90, &answer);
            } else {
                // Still waiting: a keepalive, "user presence needed".
                self.queue(self.cid, 0xbb, &[0x02]);
                self.waiting = Some((answer, reads + 1));
            }
            return Ok(self.out.pop_front());
        }
        std::thread::sleep(timeout.min(Duration::from_millis(5)));
        Ok(None)
    }
}

/// The same key held to the phone: NFC's APDUs, with chained commands and
/// answers given out in pieces.
pub struct SoftNfc {
    pub key: SoftKey,
    /// Says "U2F_V2" when selected, as a key that speaks both does.
    pub says_u2f: bool,
    pub chained: bool,
    pub got_more: bool,
    command: Vec<u8>,
    rest: Vec<u8>,
}

impl SoftNfc {
    pub fn new(key: SoftKey) -> SoftNfc {
        SoftNfc {
            key,
            says_u2f: false,
            chained: false,
            got_more: false,
            command: Vec::new(),
            rest: Vec::new(),
        }
    }

    /// An answer, 100 bytes at a time (61xx says how much is left).
    fn give(&mut self, mut answer: Vec<u8>) -> Vec<u8> {
        if answer.len() <= 100 {
            answer.extend([0x90, 0x00]);
            return answer;
        }
        self.rest = answer.split_off(100);
        answer.extend([0x61, self.rest.len().min(255) as u8]);
        answer
    }
}

impl Nfc for SoftNfc {
    fn transceive(&mut self, apdu: &[u8]) -> Result<Vec<u8>> {
        let (cla, ins) = (apdu[0], apdu[1]);
        if ins == 0xa4 {
            let version: &[u8] = if self.key.ctap2 && !self.says_u2f {
                b"FIDO_2_0"
            } else {
                b"U2F_V2"
            };
            return Ok([version, &[0x90, 0x00]].concat());
        }
        if ins == 0xc0 {
            self.got_more = true;
            let rest = std::mem::take(&mut self.rest);
            return Ok(self.give(rest));
        }
        let data = if apdu.len() > 5 {
            &apdu[5..5 + apdu[4] as usize]
        } else {
            &[][..]
        };
        if ins == 0x10 {
            self.command.extend_from_slice(data);
            if cla & 0x10 != 0 {
                self.chained = true;
                return Ok(vec![0x90, 0x00]);
            }
            let command = std::mem::take(&mut self.command);
            if !self.key.ctap2 {
                return Ok(vec![0x6d, 0x00]);
            }
            if command == [0x04] {
                let info = self.key.get_info();
                return Ok(self.give(info));
            }
            if command.first() == Some(&0x06) {
                let answer = self.key.client_pin(&command[1..]);
                return Ok(self.give(answer));
            }
            // Held to the phone: touched already.
            let (answer, _) = self.key.get_assertion(&command[1..]);
            return Ok(self.give(answer));
        }
        if ins == 0x02 {
            let mut answer = self.key.u2f(apdu, true);
            let status = answer.split_off(answer.len() - 2);
            if status != [0x90, 0x00] {
                return Ok(status);
            }
            return Ok(self.give(answer));
        }
        Ok(vec![0x6d, 0x00])
    }
}

/// A P-256 public key as a COSE_Key for key agreement (EC2, alg -25, P-256).
fn cose(public: &PublicKey) -> Value {
    let point = public.to_encoded_point(false);
    Value::Map(vec![
        (Value::Unsigned(1), Value::Unsigned(2)),
        (Value::Unsigned(3), Value::Negative(24)),
        (Value::Negative(0), Value::Unsigned(1)),
        (
            Value::Negative(1),
            Value::Bytes(point.x().unwrap().to_vec()),
        ),
        (
            Value::Negative(2),
            Value::Bytes(point.y().unwrap().to_vec()),
        ),
    ])
}

/// The platform's key agreement key, if it's one.
fn read_cose(value: &Value) -> Option<PublicKey> {
    let x = value.get_int(-2).and_then(Value::as_bytes)?;
    let y = value.get_int(-3).and_then(Value::as_bytes)?;
    PublicKey::from_sec1_bytes(&[&[0x04][..], x, y].concat()).ok()
}

/// The shared secret, as the key works it out: protocol 1, SHA-256(Z);
/// protocol 2, HKDF-SHA-256 for "CTAP2 HMAC key", then "CTAP2 AES key".
fn kdf(protocol: u64, z: &[u8]) -> Vec<u8> {
    if protocol == 1 {
        return sha256(z).to_vec();
    }
    let kdf = hkdf::Hkdf::<Sha256>::new(Some(&[0u8; 32]), z);
    let mut out = vec![0u8; 64];
    kdf.expand(b"CTAP2 HMAC key", &mut out[..32]).unwrap();
    kdf.expand(b"CTAP2 AES key", &mut out[32..]).unwrap();
    out
}

fn decrypt(protocol: u64, secret: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    let (key, iv, body) = if protocol == 1 {
        (secret, [0u8; 16], data)
    } else {
        if data.len() < 16 {
            return None;
        }
        (&secret[32..], data[..16].try_into().unwrap(), &data[16..])
    };
    cbc::Decryptor::<aes::Aes256>::new_from_slices(key, &iv)
        .ok()?
        .decrypt_padded_vec_mut::<NoPadding>(body)
        .ok()
}

fn encrypt(protocol: u64, secret: &[u8], plain: &[u8]) -> Vec<u8> {
    if protocol == 1 {
        return cbc::Encryptor::<aes::Aes256>::new_from_slices(secret, &[0u8; 16])
            .unwrap()
            .encrypt_padded_vec_mut::<NoPadding>(plain);
    }
    let iv: [u8; 16] = rand::random();
    let body = cbc::Encryptor::<aes::Aes256>::new_from_slices(&secret[32..], &iv)
        .unwrap()
        .encrypt_padded_vec_mut::<NoPadding>(plain);
    [&iv[..], &body].concat()
}

/// authenticate(token, message): 16 bytes for protocol 1, 32 for 2.
fn authenticate(protocol: u64, key: &[u8], message: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).unwrap();
    mac.update(message);
    let tag = mac.finalize().into_bytes();
    if protocol == 1 {
        tag[..16].to_vec()
    } else {
        tag.to_vec()
    }
}
