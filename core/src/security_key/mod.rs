//! Two-step login with a security key (provider 7), in the apps (DESIGN.md
//! in the meta repo, "Two-step login in the apps"). Built from the W3C's
//! WebAuthn and the FIDO Alliance's CTAP specifications and each platform's
//! own documentation, not from any other client's code:
//!
//! - webauthn.rs: the browser's half: the server's request checked, the
//!   client data made for the server's own origin, the answer checked and
//!   put in the server's shape.
//! - ctap.rs: talking to a key over USB (CTAPHID) or NFC, in CTAP2 or U2F.
//! - client_pin.rs: a key's PIN, sent with CTAP2's clientPIN only when the
//!   key asks for it (Linux, the Mac, Android; Windows asks itself).
//! - Where: Windows asks through its own WebAuthn API (windows.rs); Linux
//!   (hid_linux.rs) and the Mac (hid_mac.rs) reach keys on USB directly, as
//!   browsers do; Android holds keys to the phone or on USB through the
//!   app's Kotlin (android.rs); iPhones can't yet (`here`).

pub mod cbor;
pub mod client_pin;
pub mod ctap;
pub mod webauthn;

#[cfg(target_os = "linux")]
mod hid_linux;
#[cfg(target_os = "macos")]
mod hid_mac;
#[cfg(any(test, feature = "test-hooks"))]
pub mod soft;
#[cfg(windows)]
mod windows;

use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::time::Duration;
use std::time::Instant;

pub use client_pin::Pin;
pub use ctap::{KeyAnswer, KeyRequest, Wait};
pub use webauthn::SecurityKeyRequest;

use crate::error::Result;

/// The person gave up on the key (`cancel`); the next ask clears it.
static CANCELED: AtomicBool = AtomicBool::new(false);

/// Stops a security key's ask that's waiting, from any thread.
pub fn cancel() {
    CANCELED.store(true, Ordering::SeqCst);
}

/// How this device reaches a security key, for the app's words: "windows"
/// (Windows's own dialog), "usb" (a key plugged in), "phone" (held to the
/// phone, or plugged in), or None where the apps can't yet.
pub fn here() -> Option<&'static str> {
    if cfg!(windows) {
        Some("windows")
    } else if cfg!(target_os = "android") {
        Some("phone")
    } else if cfg!(target_os = "ios") {
        // Apple lets an iPhone app reach a key only with entitlements tied
        // to an Apple developer account and the server's own domain
        // (Associated Domains), which PassBundle doesn't have yet.
        None
    } else {
        Some("usb")
    }
}

pub const NOT_HERE: &str =
    "This device can't use a security key with PassBundle yet. Use another way to sign in, or the web vault.";

/// Whether a HID report descriptor names the FIDO Alliance's usage page
/// (a Usage Page item, 0x05 or 0x06, of 0xF1D0): a security key.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn is_fido(descriptor: &[u8]) -> bool {
    let mut at = 0;
    while let Some(&prefix) = descriptor.get(at) {
        if prefix == 0xfe {
            // A long item: its size is next, then its tag.
            let Some(&size) = descriptor.get(at + 1) else {
                return false;
            };
            at += 3 + size as usize;
            continue;
        }
        let size = match prefix & 0x03 {
            3 => 4,
            n => n as usize,
        };
        let Some(data) = descriptor.get(at + 1..at + 1 + size) else {
            return false;
        };
        // Usage Page: a global item, tag 0.
        if prefix & 0xfc == 0x04 {
            let page = data.iter().rev().fold(0u32, |n, &b| (n << 8) | b as u32);
            if page == 0xf1d0 {
                return true;
            }
        }
        at += 1 + size;
    }
    false
}

/// Asks this device's security key to sign the request: Windows's dialog;
/// on Linux and the Mac, the keys plugged in (waiting for one, then asking
/// each in turn until one holds a credential the account has); on Android,
/// the app's Kotlin. It waits as long as the server's request says, or
/// until `cancel`. `pin` is what the person typed after a key asked for its
/// PIN (`Error::KeyPin`), sent to a key only if it asks for it again, and
/// never on Windows, which asks for it itself.
pub fn ask(request: &SecurityKeyRequest, pin: Option<&Pin>) -> Result<KeyAnswer> {
    CANCELED.store(false, Ordering::SeqCst);
    let wait = Wait {
        until: Instant::now() + request.timeout,
        canceled: &CANCELED,
    };
    ask_here(request, pin, &wait)
}

#[cfg(windows)]
fn ask_here(request: &SecurityKeyRequest, _pin: Option<&Pin>, wait: &Wait) -> Result<KeyAnswer> {
    windows::ask(request, wait.canceled)
}

#[cfg(target_os = "android")]
fn ask_here(request: &SecurityKeyRequest, pin: Option<&Pin>, wait: &Wait) -> Result<KeyAnswer> {
    crate::android::ask_security_key(&request.key_request(), pin, wait)
}

#[cfg(target_os = "ios")]
fn ask_here(_request: &SecurityKeyRequest, _pin: Option<&Pin>, _wait: &Wait) -> Result<KeyAnswer> {
    Err(crate::Error::TwoStep(NOT_HERE))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn ask_here(request: &SecurityKeyRequest, pin: Option<&Pin>, wait: &Wait) -> Result<KeyAnswer> {
    #[cfg(target_os = "linux")]
    use hid_linux::{devices, Device};
    #[cfg(target_os = "macos")]
    use hid_mac::{devices, Device};

    let key = request.key_request();
    loop {
        let found = devices();
        if !found.is_empty() {
            return ask_each(found, |one, alone| {
                let pin = pin.filter(|_| alone);
                Device::open(one).and_then(|mut device| ctap::ask_hid(&mut device, &key, pin, wait))
            });
        }
        // None plugged in yet.
        if let Some(stop) = wait.over() {
            return Err(stop);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// The keys plugged in, asked in turn until one holds a credential the
/// account has. A PIN goes to a key only when it's the only one plugged in
/// (`ask` gets no PIN to send otherwise), so a PIN typed for one key is
/// never tried on another, where a wrong one costs a try; with several, a
/// key that asks for its PIN says to leave only that one plugged in.
#[cfg_attr(
    not(any(test, target_os = "linux", target_os = "macos")),
    allow(dead_code)
)]
fn ask_each<D>(
    found: Vec<D>,
    mut ask: impl FnMut(D, bool) -> Result<KeyAnswer>,
) -> Result<KeyAnswer> {
    use crate::error::Error;
    let several = found.len() > 1;
    let mut last = Error::TwoStep(ctap::NOT_THIS_KEY);
    let mut wants_pin = false;
    for one in found {
        match ask(one, !several) {
            Ok(answer) => return Ok(answer),
            // Another key plugged in may hold it.
            Err(Error::TwoStep(ctap::NOT_THIS_KEY)) => {}
            Err(e @ Error::TwoStep(ctap::CANCELED | ctap::TIMED_OUT)) => return Err(e),
            Err(Error::KeyPin(_)) if several => wants_pin = true,
            Err(e @ Error::KeyPin(_)) => return Err(e),
            Err(e) => last = e,
        }
    }
    if wants_pin {
        return Err(Error::TwoStep(client_pin::SEVERAL));
    }
    Err(last)
}

#[cfg(not(any(
    windows,
    target_os = "android",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos"
)))]
fn ask_here(_request: &SecurityKeyRequest, _pin: Option<&Pin>, _wait: &Wait) -> Result<KeyAnswer> {
    Err(crate::Error::TwoStep(NOT_HERE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_a_security_keys_report_descriptor_apart() {
        // A YubiKey's FIDO interface: Usage Page (FIDO Alliance), Usage (U2F
        // Authenticator Device), a collection of 64-byte input and output reports.
        let fido = [
            0x06, 0xd0, 0xf1, 0x09, 0x01, 0xa1, 0x01, 0x09, 0x20, 0x15, 0x00, 0x26, 0xff, 0x00,
            0x75, 0x08, 0x95, 0x40, 0x81, 0x02, 0x09, 0x21, 0x15, 0x00, 0x26, 0xff, 0x00, 0x75,
            0x08, 0x95, 0x40, 0x91, 0x02, 0xc0,
        ];
        assert!(is_fido(&fido));
        // A keyboard (Generic Desktop, Keyboard), as a YubiKey's OTP interface is.
        let keyboard = [
            0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0xc0,
        ];
        assert!(!is_fido(&keyboard));
        // Cut short, or a long item, never read past the end.
        assert!(!is_fido(&[0x06, 0xd0]));
        assert!(!is_fido(&[0xfe, 0x10, 0x00]));
        assert!(is_fido(&[0xfe, 0x01, 0x00, 0xaa, 0x06, 0xd0, 0xf1]));
    }

    #[test]
    fn a_pin_goes_to_a_key_only_when_its_the_only_one_plugged_in() {
        use crate::error::{Error, KeyPin};
        use soft::SoftKey;
        let no = AtomicBool::new(false);
        let wait = Wait {
            until: Instant::now() + std::time::Duration::from_secs(10),
            canceled: &no,
        };
        let pin = Pin::new("2580", false).unwrap();
        let mut wants = SoftKey::with_pin("vault.example.test", "2580");
        let mut other = SoftKey::new("vault.example.test");
        let request = |allow: &[&SoftKey]| KeyRequest {
            rp_id: "vault.example.test".into(),
            app_id: None,
            client_data_hash: [7; 32],
            allow: allow.iter().map(|k| k.credential_id.clone()).collect(),
            uv_required: false,
        };
        let ask = |keys: Vec<&mut SoftKey>, request: &KeyRequest| {
            ask_each(keys, |key, alone| {
                ctap::ask_hid(key, request, alone.then_some(&pin), &wait)
            })
        };
        // The account's key wants its PIN, and another is plugged in too:
        // the PIN goes to neither.
        let only_wants = request(&[&wants]);
        assert_eq!(
            ask(vec![&mut other, &mut wants], &only_wants),
            Err(Error::TwoStep(client_pin::SEVERAL))
        );
        assert_eq!((wants.pin_hashes, other.pin_hashes), (0, 0));
        // Unless the other one is the account's, and needs none.
        let both = request(&[&wants, &other]);
        assert!(ask(vec![&mut wants, &mut other], &both).is_ok());
        assert_eq!(wants.pin_hashes, 0);
        // On its own, it gets the PIN.
        assert!(ask(vec![&mut wants], &only_wants).is_ok());
        assert_eq!(wants.pin_hashes, 1);
        // And without one, says it wants it.
        assert_eq!(
            ask_each(vec![&mut wants], |key, _| ctap::ask_hid(
                key,
                &only_wants,
                None,
                &wait
            )),
            Err(Error::KeyPin(KeyPin::Asks { left: Some(8) }))
        );
    }

    #[test]
    fn says_where_keys_work() {
        let here = here();
        if cfg!(target_os = "ios") {
            assert_eq!(here, None);
        } else {
            assert!(matches!(here, Some("windows" | "usb" | "phone")));
        }
    }
}
