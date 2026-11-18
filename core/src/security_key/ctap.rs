//! Asking a security key to sign, as a browser does (FIDO Alliance, "Client
//! to Authenticator Protocol (CTAP)" 2.1, and "FIDO U2F Raw Message
//! Formats" for keys from before it): over USB, in HID reports of 64 bytes
//! (CTAPHID); over NFC, in ISO 7816-4 APDUs. A key that speaks CTAP2 gets
//! authenticatorGetAssertion; one that only speaks U2F gets U2F's
//! authenticate, whose answer is turned into the same authenticator data a
//! CTAP2 key gives. Two-step login asks keys for a touch (user verification
//! "discouraged"); a CTAP2 key that says it wants its PIN, or any CTAP2 key
//! when the server requires user verification, is sent the PIN the person
//! typed (client_pin.rs), never otherwise.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use super::cbor::{self, Value};
use super::client_pin::{self, Info, Pin, Token};
use crate::error::{Error, KeyPin, Result};

/// A security key on USB, as HID reports of 64 bytes each way (CTAP 2.1, "USB HID").
pub trait Hid {
    fn write(&mut self, report: &[u8; 64]) -> Result<()>;
    /// The next report, or None if none came in `timeout`.
    fn read(&mut self, timeout: Duration) -> Result<Option<[u8; 64]>>;
}

/// A security key held to the phone, as ISO 7816-4 APDUs (CTAP 2.1, "NFC").
pub trait Nfc {
    /// The key's answer, its status word last.
    fn transceive(&mut self, apdu: &[u8]) -> Result<Vec<u8>>;
}

/// What the core asks a key: sign this client data's hash for this site,
/// with one of these credentials. The site's ID first; then, for a key
/// added with upstream's older U2F dialog, its AppID (WebAuthn's `appid`
/// extension).
#[derive(Clone, Debug)]
pub struct KeyRequest {
    pub rp_id: String,
    pub app_id: Option<String>,
    pub client_data_hash: [u8; 32],
    /// The credentials the account has (`allowCredentials`); a key has to
    /// hold one of them.
    pub allow: Vec<Vec<u8>>,
    /// The server requires user verification ("required"): a CTAP2 key is
    /// sent its PIN even before it asks. PassBundle's server says
    /// "discouraged" for two-step login.
    pub uv_required: bool,
}

/// A key's answer: what the server checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyAnswer {
    pub credential_id: Vec<u8>,
    pub authenticator_data: Vec<u8>,
    pub signature: Vec<u8>,
    pub user_handle: Option<Vec<u8>>,
    /// Signed for the AppID, not the site's ID.
    pub used_app_id: bool,
}

/// How long to wait, and whether the person gave up.
pub struct Wait<'a> {
    pub until: Instant,
    pub canceled: &'a AtomicBool,
}

impl Wait<'_> {
    /// Why to stop now, if it's time to.
    pub fn over(&self) -> Option<Error> {
        if self.canceled.load(Ordering::SeqCst) {
            Some(Error::TwoStep(CANCELED))
        } else if Instant::now() >= self.until {
            Some(Error::TwoStep(TIMED_OUT))
        } else {
            None
        }
    }

    fn left(&self) -> Duration {
        self.until.saturating_duration_since(Instant::now())
    }
}

pub const CANCELED: &str = "Canceled.";
pub const TIMED_OUT: &str = "No security key was touched in time. Try again.";
pub const NOT_THIS_KEY: &str =
    "That security key isn't one this account uses. Try another, or use another way to sign in.";
pub const REFUSED: &str = "The security key said no. Try again.";
const KEY_ERROR: &str = "The security key didn't answer as security keys do. Unplug it, plug it in again, and try again.";

pub(super) fn key_error() -> Error {
    Error::TwoStep(KEY_ERROR)
}

/// What a CTAP2 key answered other than success, in the person's words.
pub(super) fn ctap2_error(status: u8) -> Error {
    match status {
        // KEEPALIVE_CANCEL
        0x2d => Error::TwoStep(CANCELED),
        // USER_ACTION_TIMEOUT
        0x2f => Error::TwoStep(TIMED_OUT),
        // OPERATION_DENIED, NOT_ALLOWED
        0x27 | 0x30 => Error::TwoStep(REFUSED),
        // PIN_BLOCKED
        0x32 => Error::TwoStep(client_pin::BLOCKED),
        // PIN_AUTH_INVALID: the PIN's token wasn't taken.
        0x33 => Error::TwoStep(client_pin::NOT_TAKEN),
        // PIN_AUTH_BLOCKED: 3 wrong PINs in a row.
        0x34 => Error::KeyPin(KeyPin::FreshStart),
        // PIN_NOT_SET
        0x35 => Error::TwoStep(client_pin::NOT_SET),
        // PIN_POLICY_VIOLATION
        0x37 => Error::TwoStep(client_pin::CHANGE_FIRST),
        // PUAT_REQUIRED even with the PIN's token, PIN_INVALID (a key's own
        // prompt), UV_BLOCKED, UV_INVALID, UNAUTHORIZED_PERMISSION: a check
        // of the person the core doesn't do.
        0x36 | 0x31 | 0x3c | 0x3f | 0x40 => Error::TwoStep(client_pin::OTHER_CHECK),
        // NO_CREDENTIALS, INVALID_CREDENTIAL
        0x2e | 0x22 => Error::TwoStep(NOT_THIS_KEY),
        _ => key_error(),
    }
}

fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

// CTAPHID (CTAP 2.1, "USB Human Interface Device (USB HID)").
const BROADCAST: u32 = 0xffff_ffff;
const INIT: u8 = 0x86;
const MSG: u8 = 0x83;
const CBOR: u8 = 0x90;
const CANCEL: u8 = 0x91;
const KEEPALIVE: u8 = 0xbb;
const ERROR: u8 = 0xbf;
const CAPABILITY_CBOR: u8 = 0x04;
const CAPABILITY_NMSG: u8 = 0x08;
/// The most one message carries: an initial packet's 57 bytes and 128
/// continuation packets' 59.
const MOST: usize = 57 + 128 * 59;

/// A channel to a key on USB (CTAPHID_INIT).
pub struct Channel<'a> {
    device: &'a mut dyn Hid,
    cid: u32,
    /// It speaks CTAP2.
    pub cbor: bool,
    /// It speaks U2F (CTAPHID_MSG).
    pub msg: bool,
}

impl<'a> Channel<'a> {
    /// Asks the key for a channel of its own.
    pub fn open(device: &'a mut dyn Hid, wait: &Wait) -> Result<Channel<'a>> {
        let nonce: [u8; 8] = rand::random();
        let mut channel = Channel {
            device,
            cid: BROADCAST,
            cbor: false,
            msg: true,
        };
        channel.send(INIT, &nonce)?;
        // A key answers INIT at once; others' answers on the broadcast
        // channel (another program's) don't carry this nonce.
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(stop) = wait.over() {
                return Err(stop);
            }
            if Instant::now() >= until {
                return Err(key_error());
            }
            let Some(packet) = channel.device.read(Duration::from_millis(100))? else {
                continue;
            };
            if u32::from_be_bytes(packet[..4].try_into().unwrap()) != BROADCAST
                || packet[4] != INIT
                || u16::from_be_bytes([packet[5], packet[6]]) < 17
                || packet[7..15] != nonce
            {
                continue;
            }
            channel.cid = u32::from_be_bytes(packet[15..19].try_into().unwrap());
            if channel.cid == BROADCAST || channel.cid == 0 {
                return Err(key_error());
            }
            let capabilities = packet[23];
            channel.cbor = capabilities & CAPABILITY_CBOR != 0;
            channel.msg = capabilities & CAPABILITY_NMSG == 0;
            return Ok(channel);
        }
    }

    fn send(&mut self, command: u8, data: &[u8]) -> Result<()> {
        if data.len() > MOST {
            return Err(key_error());
        }
        let mut packet = [0u8; 64];
        packet[..4].copy_from_slice(&self.cid.to_be_bytes());
        packet[4] = command;
        packet[5..7].copy_from_slice(&(data.len() as u16).to_be_bytes());
        let first = data.len().min(57);
        packet[7..7 + first].copy_from_slice(&data[..first]);
        self.device.write(&packet)?;
        for (seq, chunk) in data[first..].chunks(59).enumerate() {
            let mut packet = [0u8; 64];
            packet[..4].copy_from_slice(&self.cid.to_be_bytes());
            packet[4] = seq as u8;
            packet[5..5 + chunk.len()].copy_from_slice(chunk);
            self.device.write(&packet)?;
        }
        Ok(())
    }

    /// The answer to a command: keepalives are waited through, and the
    /// person giving up cancels it on the key (CTAPHID_CANCEL), which then
    /// answers with an error.
    fn receive(&mut self, command: u8, wait: &Wait) -> Result<Vec<u8>> {
        loop {
            if let Some(stop) = wait.over() {
                self.send(CANCEL, &[])?;
                // The key gets a moment to answer the cancel.
                let grace = Instant::now() + Duration::from_millis(500);
                while Instant::now() < grace {
                    if let Some(packet) = self.device.read(Duration::from_millis(100))? {
                        if self.ours(&packet) && packet[4] != KEEPALIVE {
                            break;
                        }
                    }
                }
                return Err(stop);
            }
            let Some(packet) = self.device.read(Duration::from_millis(100))? else {
                continue;
            };
            if !self.ours(&packet) {
                continue;
            }
            match packet[4] {
                KEEPALIVE => continue,
                ERROR => return Err(key_error()),
                c if c == command => {}
                _ => return Err(key_error()),
            }
            let length = u16::from_be_bytes([packet[5], packet[6]]) as usize;
            if length > MOST {
                return Err(key_error());
            }
            let mut data = packet[7..7 + length.min(57)].to_vec();
            let mut seq = 0u8;
            while data.len() < length {
                let packet = self
                    .device
                    .read(Duration::from_secs(1).min(wait.left().max(Duration::from_millis(100))))?
                    .ok_or_else(key_error)?;
                if !self.ours(&packet) {
                    continue;
                }
                if packet[4] != seq {
                    return Err(key_error());
                }
                let take = (length - data.len()).min(59);
                data.extend_from_slice(&packet[5..5 + take]);
                seq += 1;
            }
            return Ok(data);
        }
    }

    fn ours(&self, packet: &[u8; 64]) -> bool {
        u32::from_be_bytes(packet[..4].try_into().unwrap()) == self.cid
    }

    fn call(&mut self, command: u8, data: &[u8], wait: &Wait) -> Result<Vec<u8>> {
        self.send(command, data)?;
        self.receive(command, wait)
    }
}

/// A key, however it's reached: CTAP2's messages and U2F's.
pub(super) trait Link {
    fn cbor(&mut self, body: &[u8], wait: &Wait) -> Result<Vec<u8>>;
    /// A U2F command's answer and status word.
    fn u2f(&mut self, apdu: &[u8], wait: &Wait) -> Result<(Vec<u8>, u16)>;
}

impl Link for Channel<'_> {
    fn cbor(&mut self, body: &[u8], wait: &Wait) -> Result<Vec<u8>> {
        self.call(CBOR, body, wait)
    }

    fn u2f(&mut self, apdu: &[u8], wait: &Wait) -> Result<(Vec<u8>, u16)> {
        let mut answer = self.call(MSG, apdu, wait)?;
        split_status(&mut answer)
    }
}

fn split_status(answer: &mut Vec<u8>) -> Result<(Vec<u8>, u16)> {
    if answer.len() < 2 {
        return Err(key_error());
    }
    let status = u16::from_be_bytes([answer[answer.len() - 2], answer[answer.len() - 1]]);
    answer.truncate(answer.len() - 2);
    Ok((std::mem::take(answer), status))
}

/// The FIDO applet on a key held to the phone.
const FIDO_AID: [u8; 8] = [0xa0, 0x00, 0x00, 0x06, 0x47, 0x2f, 0x00, 0x01];

struct NfcLink<'a> {
    device: &'a mut dyn Nfc,
}

impl NfcLink<'_> {
    /// Sends a command, in pieces of 255 bytes if it's longer (command
    /// chaining), and gathers a long answer (GET RESPONSE).
    fn exchange(
        &mut self,
        cla: u8,
        ins: u8,
        p1: u8,
        p2: u8,
        data: &[u8],
    ) -> Result<(Vec<u8>, u16)> {
        let mut chunks = data.chunks(255).peekable();
        let mut answer = loop {
            let chunk = chunks.next().unwrap_or(&[]);
            let last = chunks.peek().is_none();
            let mut apdu = vec![if last { cla } else { cla | 0x10 }, ins, p1, p2];
            if !chunk.is_empty() {
                apdu.push(chunk.len() as u8);
                apdu.extend_from_slice(chunk);
            }
            if last {
                apdu.push(0x00);
            }
            let mut answer = self.device.transceive(&apdu)?;
            if last {
                break answer;
            }
            let (_, status) = split_status(&mut answer)?;
            if status != 0x9000 {
                return Ok((Vec::new(), status));
            }
        };
        let (mut data, mut status) = split_status(&mut answer)?;
        while status >> 8 == 0x61 {
            let mut more = self
                .device
                .transceive(&[0x00, 0xc0, 0x00, 0x00, status as u8])?;
            let (chunk, next) = split_status(&mut more)?;
            data.extend(chunk);
            status = next;
            if data.len() > MOST {
                return Err(key_error());
            }
        }
        Ok((data, status))
    }
}

impl Link for NfcLink<'_> {
    fn cbor(&mut self, body: &[u8], wait: &Wait) -> Result<Vec<u8>> {
        if let Some(stop) = wait.over() {
            return Err(stop);
        }
        // NFCCTAP_MSG, answered when done (P1 0: no status updates).
        let (data, status) = self.exchange(0x80, 0x10, 0x00, 0x00, body)?;
        if status != 0x9000 {
            return Err(key_error());
        }
        Ok(data)
    }

    fn u2f(&mut self, apdu: &[u8], wait: &Wait) -> Result<(Vec<u8>, u16)> {
        if let Some(stop) = wait.over() {
            return Err(stop);
        }
        // U2F's APDU as `u2f_authenticate` builds it: the header, then the data.
        let data = &apdu[7..apdu.len() - 2];
        self.exchange(apdu[0], apdu[1], apdu[2], apdu[3], data)
    }
}

/// What a CTAP2 key made of a request.
enum Found {
    Answer(KeyAnswer),
    /// It holds none of the credentials.
    None,
    /// The list is longer than it takes.
    TooMany,
    /// It wants its PIN first (PUAT_REQUIRED, which CTAP 2.0 calls
    /// PIN_REQUIRED).
    NeedsPin,
}

/// CTAP2's authenticatorGetAssertion for one site ID, as sent: a touch
/// always ("up"), and with a PIN's token, its pinUvAuthParam and protocol.
pub fn get_assertion_request(
    rp_id: &str,
    client_data_hash: &[u8; 32],
    allow: &[Vec<u8>],
    token: Option<&Token>,
) -> Vec<u8> {
    let mut members = vec![
        (Value::Unsigned(1), Value::text(rp_id)),
        (Value::Unsigned(2), Value::Bytes(client_data_hash.to_vec())),
        (
            Value::Unsigned(5),
            Value::Map(vec![(Value::text("up"), Value::Bool(true))]),
        ),
    ];
    if let Some(token) = token {
        members.push((
            Value::Unsigned(6),
            Value::Bytes(token.param(client_data_hash)),
        ));
        members.push((
            Value::Unsigned(7),
            Value::Unsigned(token.protocol().number()),
        ));
    }
    if !allow.is_empty() {
        members.push((
            Value::Unsigned(3),
            Value::Array(
                allow
                    .iter()
                    .map(|id| {
                        Value::Map(vec![
                            (Value::text("id"), Value::Bytes(id.clone())),
                            (Value::text("type"), Value::text("public-key")),
                        ])
                    })
                    .collect(),
            ),
        ));
    }
    let mut body = vec![0x02];
    body.extend(cbor::encode(&Value::Map(members)));
    body
}

/// CTAP2's authenticatorGetAssertion for one site ID.
fn get_assertion(
    link: &mut dyn Link,
    rp_id: &str,
    request: &KeyRequest,
    allow: &[Vec<u8>],
    token: Option<&Token>,
    wait: &Wait,
) -> Result<Found> {
    let body = get_assertion_request(rp_id, &request.client_data_hash, allow, token);
    let answer = link.cbor(&body, wait)?;
    let (&status, rest) = answer.split_first().ok_or_else(key_error)?;
    match status {
        0x00 => {}
        0x2e => return Ok(Found::None),
        // The command's too long for the key, or lists more credentials
        // than it takes.
        0x39 | 0x15 if allow.len() > 1 => return Ok(Found::TooMany),
        0x36 if token.is_none() => return Ok(Found::NeedsPin),
        _ => return Err(ctap2_error(status)),
    }
    let read = cbor::decode(rest)?;
    let credential_id = match read
        .get(1)
        .and_then(|c| c.get_text("id"))
        .and_then(Value::as_bytes)
    {
        Some(id) => id.to_vec(),
        // A key may leave the credential out when only one was asked about.
        None if allow.len() == 1 => allow[0].clone(),
        None => return Err(key_error()),
    };
    let authenticator_data = read
        .get(2)
        .and_then(Value::as_bytes)
        .ok_or_else(key_error)?
        .to_vec();
    let signature = read
        .get(3)
        .and_then(Value::as_bytes)
        .ok_or_else(key_error)?
        .to_vec();
    let user_handle = read
        .get(4)
        .and_then(|u| u.get_text("id"))
        .and_then(Value::as_bytes)
        .map(<[u8]>::to_vec);
    Ok(Found::Answer(KeyAnswer {
        credential_id,
        authenticator_data,
        signature,
        user_handle,
        used_app_id: false,
    }))
}

/// A key's PIN for one ask: whether the key needs it, what the key said
/// about it, and the tokens it gave, by site ID. Dropped (and the tokens
/// with it, overwritten) when the ask ends.
struct PinAsk<'p> {
    pin: Option<&'p Pin>,
    needed: bool,
    info: Option<Info>,
    tokens: Vec<(String, Token)>,
}

impl PinAsk<'_> {
    /// A token for `rp_id`: one this ask has, or one from the PIN. A CTAP
    /// 2.1 key ties a token to the site ID it's first used with, so each
    /// site ID gets its own; a CTAP 2.0 key's token is good for any.
    fn token(&mut self, link: &mut dyn Link, rp_id: &str, wait: &Wait) -> Result<&Token> {
        if self.info.is_none() {
            self.info = Some(client_pin::get_info(link, wait)?);
        }
        let info = self.info.as_ref().expect("read above");
        let kept = self
            .tokens
            .iter()
            .position(|(id, _)| id == rp_id || !info.permissions);
        let at = match kept {
            Some(at) => at,
            None => {
                let token = client_pin::token(link, info, rp_id, self.pin, wait)?;
                self.tokens.push((rp_id.to_owned(), token));
                self.tokens.len() - 1
            }
        };
        Ok(&self.tokens[at].1)
    }
}

/// getAssertion for one site ID and these credentials, with the PIN's
/// token once the key wants it (or the server requires it).
fn ask_for(
    link: &mut dyn Link,
    rp_id: &str,
    request: &KeyRequest,
    allow: &[Vec<u8>],
    pin: &mut PinAsk,
    wait: &Wait,
) -> Result<Found> {
    loop {
        let token = if pin.needed {
            Some(pin.token(link, rp_id, wait)?)
        } else {
            None
        };
        match get_assertion(link, rp_id, request, allow, token, wait)? {
            // It wants its PIN: asked again, with it.
            Found::NeedsPin if !pin.needed => pin.needed = true,
            Found::NeedsPin => return Err(Error::TwoStep(client_pin::OTHER_CHECK)),
            found => return Ok(found),
        }
    }
}

/// CTAP2, for the site ID, then the AppID; the whole list, or, for a key
/// that takes fewer, one credential at a time. The PIN goes to the key
/// only once it says it wants it, or when the server requires the person
/// verified.
fn ctap2(
    link: &mut dyn Link,
    request: &KeyRequest,
    pin: Option<&Pin>,
    wait: &Wait,
) -> Result<KeyAnswer> {
    let ids: Vec<(&str, bool)> = std::iter::once((request.rp_id.as_str(), false))
        .chain(request.app_id.as_deref().map(|a| (a, true)))
        .collect();
    let mut pin = PinAsk {
        pin,
        needed: request.uv_required,
        info: None,
        tokens: Vec::new(),
    };
    for (id, app) in ids {
        let mut found = ask_for(link, id, request, &request.allow, &mut pin, wait)?;
        if let Found::TooMany = found {
            for one in &request.allow {
                found = ask_for(link, id, request, std::slice::from_ref(one), &mut pin, wait)?;
                if let Found::Answer(_) = found {
                    break;
                }
            }
        }
        if let Found::Answer(mut answer) = found {
            answer.used_app_id = app;
            return Ok(answer);
        }
    }
    Err(Error::TwoStep(NOT_THIS_KEY))
}

/// U2F's authenticate: `check` only asks whether the key knows the key
/// handle (P1 7); otherwise it signs once touched (P1 3). Extended length,
/// as U2F's HID transport takes it.
fn u2f_authenticate(check: bool, request: &KeyRequest, app: &[u8; 32], handle: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(65 + handle.len());
    data.extend_from_slice(&request.client_data_hash);
    data.extend_from_slice(app);
    data.push(handle.len() as u8);
    data.extend_from_slice(handle);
    let mut apdu = vec![0x00, 0x02, if check { 0x07 } else { 0x03 }, 0x00, 0x00];
    apdu.extend((data.len() as u16).to_be_bytes());
    apdu.extend(data);
    apdu.extend([0x00, 0x00]);
    apdu
}

/// A key that speaks only U2F: find which credential (and which of the site
/// ID and the AppID) it knows, then have it sign once touched.
fn u2f(link: &mut dyn Link, request: &KeyRequest, wait: &Wait) -> Result<KeyAnswer> {
    let apps: Vec<([u8; 32], bool)> = std::iter::once((sha256(request.rp_id.as_bytes()), false))
        .chain(
            request
                .app_id
                .as_deref()
                .map(|a| (sha256(a.as_bytes()), true)),
        )
        .collect();
    let mut known = None;
    'search: for handle in request.allow.iter().filter(|h| h.len() <= 255) {
        for (app, used_app_id) in &apps {
            let (_, status) = link.u2f(&u2f_authenticate(true, request, app, handle), wait)?;
            // "Conditions not satisfied": it knows the handle and would sign once touched.
            if status == 0x6985 {
                known = Some((handle.clone(), *app, *used_app_id));
                break 'search;
            }
        }
    }
    let (handle, app, used_app_id) = known.ok_or(Error::TwoStep(NOT_THIS_KEY))?;
    loop {
        let (data, status) = link.u2f(&u2f_authenticate(false, request, &app, &handle), wait)?;
        match status {
            0x9000 => {
                if data.len() < 6 {
                    return Err(key_error());
                }
                let mut authenticator_data = app.to_vec();
                authenticator_data.push(data[0] & 0x01);
                authenticator_data.extend_from_slice(&data[1..5]);
                return Ok(KeyAnswer {
                    credential_id: handle,
                    authenticator_data,
                    signature: data[5..].to_vec(),
                    user_handle: None,
                    used_app_id,
                });
            }
            // Not touched yet.
            0x6985 => {
                if let Some(stop) = wait.over() {
                    return Err(stop);
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            _ => return Err(key_error()),
        }
    }
}

/// Asks a key on USB; with the person's PIN, for a key that asks for it.
pub fn ask_hid(
    device: &mut dyn Hid,
    request: &KeyRequest,
    pin: Option<&Pin>,
    wait: &Wait,
) -> Result<KeyAnswer> {
    let mut channel = Channel::open(device, wait)?;
    if channel.cbor {
        ctap2(&mut channel, request, pin, wait)
    } else if channel.msg {
        u2f(&mut channel, request, wait)
    } else {
        Err(key_error())
    }
}

/// Asks a key held to the phone; with the person's PIN, for a key that
/// asks for it.
pub fn ask_nfc(
    device: &mut dyn Nfc,
    request: &KeyRequest,
    pin: Option<&Pin>,
    wait: &Wait,
) -> Result<KeyAnswer> {
    let mut link = NfcLink { device };
    let (version, status) = link.exchange(0x00, 0xa4, 0x04, 0x00, &FIDO_AID)?;
    if status != 0x9000 {
        return Err(Error::TwoStep(
            "That isn't a security key PassBundle can use. Try another way to sign in.",
        ));
    }
    // A key that speaks both says "U2F_V2" (CTAP 2.1, "Applet selection"),
    // so one that doesn't say "FIDO_2_0" is asked authenticatorGetInfo:
    // CTAP2 if it answers, as browsers do, else U2F.
    let speaks_ctap2 = version == b"FIDO_2_0"
        || link
            .cbor(&[0x04], wait)
            .is_ok_and(|info| info.first() == Some(&0x00));
    if speaks_ctap2 {
        ctap2(&mut link, request, pin, wait)
    } else {
        u2f(&mut link, request, wait)
    }
}

#[cfg(test)]
mod tests {
    use super::super::soft::{SoftKey, SoftNfc, PIN_TRIES};
    use super::*;

    fn wait(canceled: &AtomicBool) -> Wait<'_> {
        Wait {
            until: Instant::now() + Duration::from_secs(10),
            canceled,
        }
    }

    fn request(key: &SoftKey) -> KeyRequest {
        KeyRequest {
            rp_id: "vault.example.test".into(),
            app_id: Some("https://vault.example.test/app-id.json".into()),
            client_data_hash: sha256(b"client data"),
            allow: vec![vec![9; 64], key.credential_id.clone()],
            uv_required: false,
        }
    }

    #[test]
    fn a_ctap2_key_on_usb_signs_for_the_site() {
        let mut key = SoftKey::new("vault.example.test");
        let request = request(&key);
        let no = AtomicBool::new(false);
        let answer = ask_hid(&mut key, &request, None, &wait(&no)).unwrap();
        assert_eq!(answer.credential_id, key.credential_id);
        assert!(!answer.used_app_id);
        assert_eq!(
            &answer.authenticator_data[..32],
            &sha256(b"vault.example.test")
        );
        assert_eq!(answer.authenticator_data[32] & 0x01, 0x01, "user present");
        assert!(key.verifies(
            &answer.authenticator_data,
            &request.client_data_hash,
            &answer.signature
        ));
        // Long enough to need continuation packets both ways.
        assert!(key.longest_message > 57);
    }

    #[test]
    fn a_key_added_with_u2f_signs_for_the_app_id() {
        let mut key = SoftKey::new("https://vault.example.test/app-id.json");
        let request = request(&key);
        let no = AtomicBool::new(false);
        let answer = ask_hid(&mut key, &request, None, &wait(&no)).unwrap();
        assert!(answer.used_app_id);
        assert_eq!(
            &answer.authenticator_data[..32],
            &sha256(b"https://vault.example.test/app-id.json")
        );
        assert!(key.verifies(
            &answer.authenticator_data,
            &request.client_data_hash,
            &answer.signature
        ));
    }

    #[test]
    fn a_u2f_only_key_is_asked_with_u2f() {
        for site in [
            "vault.example.test",
            "https://vault.example.test/app-id.json",
        ] {
            let mut key = SoftKey::new(site);
            key.ctap2 = false;
            // Not touched for the first few asks.
            key.touch_after = 3;
            let request = request(&key);
            let no = AtomicBool::new(false);
            let answer = ask_hid(&mut key, &request, None, &wait(&no)).unwrap();
            assert_eq!(answer.used_app_id, site.starts_with("https://"));
            assert_eq!(&answer.authenticator_data[..32], &sha256(site.as_bytes()));
            assert_eq!(answer.authenticator_data.len(), 37);
            assert!(key.verifies(
                &answer.authenticator_data,
                &request.client_data_hash,
                &answer.signature
            ));
        }
    }

    #[test]
    fn a_key_without_the_credential_says_so() {
        let mut key = SoftKey::new("vault.example.test");
        let mut request = request(&key);
        request.allow = vec![vec![1; 32]];
        let no = AtomicBool::new(false);
        assert_eq!(
            ask_hid(&mut key, &request, None, &wait(&no)),
            Err(Error::TwoStep(NOT_THIS_KEY))
        );
        key.ctap2 = false;
        assert_eq!(
            ask_hid(&mut key, &request, None, &wait(&no)),
            Err(Error::TwoStep(NOT_THIS_KEY))
        );
    }

    fn pin(typed: &str) -> Pin {
        Pin::new(typed, false).unwrap()
    }

    fn verified(answer: &KeyAnswer) -> bool {
        answer.authenticator_data[32] & 0x04 != 0
    }

    #[test]
    fn a_key_that_doesnt_ask_for_its_pin_is_never_sent_it() {
        let mut key = SoftKey::with_pin("vault.example.test", "2580");
        key.always_uv = false;
        let request = request(&key);
        let no = AtomicBool::new(false);
        let answer = ask_hid(&mut key, &request, Some(&pin("2580")), &wait(&no)).unwrap();
        assert!(key.verifies(
            &answer.authenticator_data,
            &request.client_data_hash,
            &answer.signature
        ));
        assert!(!verified(&answer));
        assert!(key.heard.is_empty(), "{:?}", key.heard);
        assert_eq!(key.pin_hashes, 0);
    }

    #[test]
    fn a_key_that_asks_for_its_pin_gets_it_with_each_protocol_and_way_of_getting_a_token() {
        // What the key lists, whether it's CTAP 2.1, the protocol, and the
        // subcommand that gets its token.
        for (protocols, ctap21, protocol, subcommand) in [
            (vec![2, 1], true, 2, 0x09),
            (vec![1, 2], true, 2, 0x09),
            (vec![1], true, 1, 0x09),
            (vec![], false, 1, 0x05),
            (vec![1], false, 1, 0x05),
            (vec![2], false, 2, 0x05),
        ] {
            let case = format!("{protocols:?} {ctap21}");
            let mut key = SoftKey::with_pin("vault.example.test", "2580");
            key.protocols = protocols;
            key.ctap21 = ctap21;
            let request = request(&key);
            let no = AtomicBool::new(false);
            // Asked without one: it says so, with the tries left, and no
            // PIN reached it.
            assert_eq!(
                ask_hid(&mut key, &request, None, &wait(&no)),
                Err(Error::KeyPin(KeyPin::Asks { left: Some(8) })),
                "{case}"
            );
            assert_eq!(key.pin_hashes, 0, "{case}");
            assert_eq!(key.heard, vec![0x01], "{case}");
            // With it: signed, with the person verified.
            let answer = ask_hid(&mut key, &request, Some(&pin("2580")), &wait(&no)).unwrap();
            assert!(key.verifies(
                &answer.authenticator_data,
                &request.client_data_hash,
                &answer.signature
            ));
            assert!(verified(&answer), "{case}");
            assert_eq!(answer.credential_id, key.credential_id, "{case}");
            assert_eq!(key.token_protocol, Some(protocol), "{case}");
            assert_eq!(key.heard, vec![0x01, 0x01, 0x02, subcommand], "{case}");
            assert_eq!(key.pin_hashes, 1, "{case}");
        }
    }

    #[test]
    fn a_key_held_to_the_phone_takes_its_pin() {
        for ctap21 in [true, false] {
            let mut nfc = SoftNfc::new(SoftKey::with_pin("vault.example.test", "2580"));
            nfc.key.ctap21 = ctap21;
            let request = request(&nfc.key);
            let no = AtomicBool::new(false);
            assert_eq!(
                ask_nfc(&mut nfc, &request, None, &wait(&no)),
                Err(Error::KeyPin(KeyPin::Asks { left: Some(8) }))
            );
            let answer = ask_nfc(&mut nfc, &request, Some(&pin("2580")), &wait(&no)).unwrap();
            assert!(verified(&answer));
            assert!(nfc.key.verifies(
                &answer.authenticator_data,
                &request.client_data_hash,
                &answer.signature
            ));
        }
    }

    #[test]
    fn a_wrong_pin_says_how_many_tries_are_left_and_three_in_a_row_want_a_fresh_start() {
        for ctap21 in [true, false] {
            let mut key = SoftKey::with_pin("vault.example.test", "2580");
            key.ctap21 = ctap21;
            let request = request(&key);
            let no = AtomicBool::new(false);
            let ask = |key: &mut SoftKey, typed: &str| {
                ask_hid(key, &request, Some(&pin(typed)), &wait(&no))
            };
            let wrong = ask(&mut key, "1111").unwrap_err();
            assert_eq!(wrong, Error::KeyPin(KeyPin::Wrong { left: Some(7) }));
            assert_eq!(wrong.to_string(), "That PIN isn't right. 7 tries left.");
            assert_eq!(
                ask(&mut key, "1112"),
                Err(Error::KeyPin(KeyPin::Wrong { left: Some(6) }))
            );
            // The third in a row.
            let third = ask(&mut key, "1113").unwrap_err();
            assert_eq!(third, Error::KeyPin(KeyPin::FreshStart));
            assert!(third.to_string().contains("unplug it and plug it in again"));
            assert_eq!(key.pin_tries, 5);
            // Until it's unplugged, nothing more counts as a try.
            assert_eq!(
                ask(&mut key, "2580"),
                Err(Error::KeyPin(KeyPin::FreshStart))
            );
            assert_eq!(key.pin_tries, 5, "{ctap21}");
            key.replug();
            assert!(verified(&ask(&mut key, "2580").unwrap()));
            assert_eq!(key.pin_tries, PIN_TRIES);
        }
    }

    #[test]
    fn the_last_try_is_used_only_when_the_person_says_so() {
        let mut key = SoftKey::with_pin("vault.example.test", "2580");
        key.pin_tries = 1;
        let request = request(&key);
        let no = AtomicBool::new(false);
        assert_eq!(
            ask_hid(&mut key, &request, None, &wait(&no)),
            Err(Error::KeyPin(KeyPin::Asks { left: Some(1) }))
        );
        // Not said: nothing sent.
        let stopped = ask_hid(&mut key, &request, Some(&pin("2580")), &wait(&no)).unwrap_err();
        assert_eq!(stopped, Error::KeyPin(KeyPin::LastTry));
        assert_eq!(stopped.to_string(), crate::error::LAST_TRY);
        assert_eq!((key.pin_hashes, key.pin_tries), (0, 1));
        // Said, and right: signed, and the tries are back.
        let last = Pin::new("2580", true).unwrap();
        assert!(verified(
            &ask_hid(&mut key, &request, Some(&last), &wait(&no)).unwrap()
        ));
        assert_eq!(key.pin_tries, PIN_TRIES);
        // Said, and wrong: the key's blocked.
        key.pin_tries = 1;
        let wrong = Pin::new("1111", true).unwrap();
        assert_eq!(
            ask_hid(&mut key, &request, Some(&wrong), &wait(&no)),
            Err(Error::TwoStep(client_pin::BLOCKED))
        );
        assert_eq!(key.pin_tries, 0);
    }

    #[test]
    fn a_blocked_pin_no_pin_set_no_pin_at_all_and_a_pin_to_change_each_say_so() {
        let no = AtomicBool::new(false);
        let mut blocked = SoftKey::with_pin("vault.example.test", "2580");
        blocked.pin_tries = 0;
        let request = request(&blocked);
        for given in [None, Some(&pin("2580"))] {
            assert_eq!(
                ask_hid(&mut blocked, &request, given, &wait(&no)),
                Err(Error::TwoStep(client_pin::BLOCKED))
            );
        }
        let mut not_set = SoftKey::new("vault.example.test");
        not_set.always_uv = true;
        assert_eq!(
            ask_hid(&mut not_set, &request, Some(&pin("2580")), &wait(&no)),
            Err(Error::TwoStep(client_pin::NOT_SET))
        );
        let mut no_pin = SoftKey::new("vault.example.test");
        no_pin.always_uv = true;
        no_pin.takes_pin = false;
        assert_eq!(
            ask_hid(&mut no_pin, &request, Some(&pin("2580")), &wait(&no)),
            Err(Error::TwoStep(client_pin::OTHER_CHECK))
        );
        let mut change = SoftKey::with_pin("vault.example.test", "2580");
        change.force_pin_change = true;
        assert_eq!(
            ask_hid(&mut change, &request, Some(&pin("2580")), &wait(&no)),
            Err(Error::TwoStep(client_pin::CHANGE_FIRST))
        );
        for key in [&blocked, &not_set, &no_pin, &change] {
            assert_eq!(key.pin_hashes, 0);
        }
    }

    #[test]
    fn the_app_id_gets_a_token_of_its_own_from_a_ctap_2_1_key() {
        // getPINRetries, getKeyAgreement, and the token, for each site ID
        // from a 2.1 key; once from a 2.0 key, whose token is good for any.
        for (ctap21, heard) in [
            (true, vec![0x01, 0x01, 0x02, 0x09, 0x01, 0x02, 0x09]),
            (false, vec![0x01, 0x01, 0x02, 0x05]),
        ] {
            let mut key = SoftKey::with_pin("https://vault.example.test/app-id.json", "2580");
            key.ctap21 = ctap21;
            let request = request(&key);
            let no = AtomicBool::new(false);
            assert!(matches!(
                ask_hid(&mut key, &request, None, &wait(&no)),
                Err(Error::KeyPin(KeyPin::Asks { .. }))
            ));
            let answer = ask_hid(&mut key, &request, Some(&pin("2580")), &wait(&no)).unwrap();
            assert!(answer.used_app_id);
            assert!(verified(&answer));
            assert_eq!(key.heard, heard, "{ctap21}");
        }
    }

    #[test]
    fn a_key_that_takes_one_credential_at_a_time_uses_one_token_for_them() {
        let mut key = SoftKey::with_pin("vault.example.test", "2580");
        key.most_in_list = 1;
        let request = request(&key);
        let no = AtomicBool::new(false);
        let answer = ask_hid(&mut key, &request, Some(&pin("2580")), &wait(&no)).unwrap();
        assert!(verified(&answer));
        assert_eq!(key.pin_hashes, 1);
    }

    #[test]
    fn a_server_that_requires_verification_gets_the_pin_sent_before_the_key_asks() {
        let mut key = SoftKey::with_pin("vault.example.test", "2580");
        key.always_uv = false;
        let mut request = request(&key);
        request.uv_required = true;
        let no = AtomicBool::new(false);
        assert_eq!(
            ask_hid(&mut key, &request, None, &wait(&no)),
            Err(Error::KeyPin(KeyPin::Asks { left: Some(8) }))
        );
        assert!(verified(
            &ask_hid(&mut key, &request, Some(&pin("2580")), &wait(&no)).unwrap()
        ));
        // A key with no PIN set says so; "preferred" (uv_required false)
        // asks nothing of it.
        let mut none = SoftKey::new("vault.example.test");
        let mut request = super::tests::request(&none);
        request.uv_required = true;
        assert_eq!(
            ask_hid(&mut none, &request, None, &wait(&no)),
            Err(Error::TwoStep(client_pin::NOT_SET))
        );
        request.uv_required = false;
        none.heard.clear();
        assert!(!verified(
            &ask_hid(&mut none, &request, None, &wait(&no)).unwrap()
        ));
        assert!(none.heard.is_empty());
    }

    #[test]
    fn the_pin_is_never_in_an_errors_words() {
        let mut key = SoftKey::with_pin("vault.example.test", "2580");
        let request = request(&key);
        let no = AtomicBool::new(false);
        for typed in ["8642", "97531", "Grüße 7"] {
            let said = ask_hid(&mut key, &request, Some(&pin(typed)), &wait(&no))
                .unwrap_err()
                .to_string();
            assert!(!said.contains(typed), "{said}");
            key.replug();
        }
        for why in [
            KeyPin::Asks { left: Some(3) },
            KeyPin::Wrong { left: None },
            KeyPin::LastTry,
            KeyPin::FreshStart,
            KeyPin::TooShort,
            KeyPin::TooLong,
            KeyPin::NotAPin,
        ] {
            assert!(!Error::KeyPin(why).to_string().is_empty());
        }
    }

    #[test]
    fn giving_up_cancels_on_the_key() {
        let mut key = SoftKey::new("vault.example.test");
        key.touch_after = u32::MAX;
        let request = request(&key);
        let canceled = AtomicBool::new(false);
        let stop = &canceled;
        std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_millis(300));
                stop.store(true, Ordering::SeqCst);
            });
            assert_eq!(
                ask_hid(&mut key, &request, None, &wait(&canceled)),
                Err(Error::TwoStep(CANCELED))
            );
        });
        assert!(key.canceled, "CTAPHID_CANCEL reached the key");
        // And with no one there, the wait ends.
        let mut key = SoftKey::new("vault.example.test");
        key.touch_after = u32::MAX;
        let request = super::tests::request(&key);
        let no = AtomicBool::new(false);
        let short = Wait {
            until: Instant::now() + Duration::from_millis(400),
            canceled: &no,
        };
        assert_eq!(
            ask_hid(&mut key, &request, None, &short),
            Err(Error::TwoStep(TIMED_OUT))
        );
    }

    #[test]
    fn a_key_held_to_the_phone_signs_over_nfc() {
        // CTAP2 only, both (which says "U2F_V2" when selected), and U2F only.
        for (ctap2, says_u2f) in [(true, false), (true, true), (false, true)] {
            let mut nfc = SoftNfc::new(SoftKey::new("vault.example.test"));
            nfc.key.ctap2 = ctap2;
            nfc.says_u2f = says_u2f;
            let request = request(&nfc.key);
            let no = AtomicBool::new(false);
            let answer = ask_nfc(&mut nfc, &request, None, &wait(&no)).unwrap();
            assert_eq!(answer.credential_id, nfc.key.credential_id);
            assert!(nfc.key.verifies(
                &answer.authenticator_data,
                &request.client_data_hash,
                &answer.signature
            ));
            // CTAP2 whenever the key speaks it: the command went in pieces
            // (chaining), and the answer came back in pieces (GET RESPONSE).
            assert_eq!(nfc.chained, ctap2, "{ctap2} {says_u2f}");
            assert_eq!(nfc.got_more, ctap2, "{ctap2} {says_u2f}");
        }
    }

    #[test]
    fn a_key_that_takes_one_credential_at_a_time_gets_them_so() {
        let mut key = SoftKey::new("vault.example.test");
        key.most_in_list = 1;
        let request = request(&key);
        let no = AtomicBool::new(false);
        let answer = ask_hid(&mut key, &request, None, &wait(&no)).unwrap();
        assert_eq!(answer.credential_id, key.credential_id);
    }

    #[test]
    fn answers_on_other_channels_are_passed_over() {
        let mut key = SoftKey::new("vault.example.test");
        key.noise = true;
        let request = request(&key);
        let no = AtomicBool::new(false);
        assert!(ask_hid(&mut key, &request, None, &wait(&no)).is_ok());
    }
}
