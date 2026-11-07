//! A security key in software, for tests only (unit tests, and builds with
//! the test-hooks feature, which the core's server tests use; never a
//! release): CTAP2's getAssertion and U2F's authenticate over CTAPHID's
//! reports or NFC's APDUs, with one P-256 credential for one site ID, as a
//! real key keeps it. It can be slow to be touched, insist on a PIN, take
//! one credential at a time, speak only U2F, and talk over other channels'
//! traffic, so the core's side is tested against each.

use std::collections::VecDeque;
use std::time::Duration;

use p256::ecdsa::signature::{Signer, Verifier};
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

use super::cbor::{self, Value};
use super::ctap::{Hid, Nfc};
use crate::error::Result;

pub struct SoftKey {
    /// The site ID (or AppID) its one credential was made for.
    pub site: String,
    pub credential_id: Vec<u8>,
    key: SigningKey,
    counter: u32,
    /// Speaks CTAP2 (else only U2F).
    pub ctap2: bool,
    /// Refuses to sign without its PIN.
    pub wants_pin: bool,
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
            wants_pin: false,
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

    fn sign(&mut self, app: [u8; 32], client_data_hash: &[u8]) -> (Vec<u8>, Vec<u8>) {
        self.counter += 1;
        let mut data = app.to_vec();
        data.push(0x01);
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
        if self.wants_pin {
            return (vec![0x36], false);
        }
        if allow.len() > self.most_in_list {
            return (vec![0x15], false);
        }
        if rp_id != self.site || !allow.contains(&self.credential_id) {
            return (vec![0x2e], false);
        }
        let (data, signature) = self.sign(sha256(rp_id.as_bytes()), &hash);
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
        let (data, signature) = self.sign(app.try_into().unwrap(), challenge);
        [&data[32..], &signature[..], &[0x90, 0x00]].concat()
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
    pub chained: bool,
    pub got_more: bool,
    command: Vec<u8>,
    rest: Vec<u8>,
}

impl SoftNfc {
    pub fn new(key: SoftKey) -> SoftNfc {
        SoftNfc {
            key,
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
            let version: &[u8] = if self.key.ctap2 {
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
