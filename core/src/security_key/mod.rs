//! Two-step login with a security key (provider 7), in the apps (DESIGN.md
//! in the meta repo, "Two-step login in the apps"). Built from the W3C's
//! WebAuthn and the FIDO Alliance's CTAP specifications and each platform's
//! own documentation, not from any other client's code:
//!
//! - webauthn.rs: the browser's half: the server's request checked, the
//!   client data made for the server's own origin, the answer checked and
//!   put in the server's shape.
//! - ctap.rs: talking to a key over USB (CTAPHID) or NFC, in CTAP2 or U2F.
//! - Where: Windows asks through its own WebAuthn API (windows.rs); Linux
//!   (hid_linux.rs) and the Mac (hid_mac.rs) reach keys on USB directly, as
//!   browsers do; Android holds keys to the phone or on USB through the
//!   app's Kotlin (android.rs); iPhones can't yet (`here`).

pub mod cbor;
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
/// until `cancel`.
pub fn ask(request: &SecurityKeyRequest) -> Result<KeyAnswer> {
    CANCELED.store(false, Ordering::SeqCst);
    let wait = Wait {
        until: Instant::now() + request.timeout,
        canceled: &CANCELED,
    };
    ask_here(request, &wait)
}

#[cfg(windows)]
fn ask_here(request: &SecurityKeyRequest, wait: &Wait) -> Result<KeyAnswer> {
    windows::ask(request, wait.canceled)
}

#[cfg(target_os = "android")]
fn ask_here(request: &SecurityKeyRequest, wait: &Wait) -> Result<KeyAnswer> {
    crate::android::ask_security_key(&request.key_request(), wait)
}

#[cfg(target_os = "ios")]
fn ask_here(_request: &SecurityKeyRequest, _wait: &Wait) -> Result<KeyAnswer> {
    Err(crate::Error::TwoStep(NOT_HERE))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn ask_here(request: &SecurityKeyRequest, wait: &Wait) -> Result<KeyAnswer> {
    use crate::error::Error;
    #[cfg(target_os = "linux")]
    use hid_linux::{devices, Device};
    #[cfg(target_os = "macos")]
    use hid_mac::{devices, Device};

    let key = request.key_request();
    loop {
        let found = devices();
        if !found.is_empty() {
            let mut last = Error::TwoStep(ctap::NOT_THIS_KEY);
            for one in found {
                let asked =
                    Device::open(one).and_then(|mut device| ctap::ask_hid(&mut device, &key, wait));
                match asked {
                    Ok(answer) => return Ok(answer),
                    // Another key plugged in may hold it.
                    Err(Error::TwoStep(ctap::NOT_THIS_KEY)) => {}
                    Err(e @ Error::TwoStep(ctap::CANCELED | ctap::TIMED_OUT)) => return Err(e),
                    Err(e) => last = e,
                }
            }
            return Err(last);
        }
        // None plugged in yet.
        if let Some(stop) = wait.over() {
            return Err(stop);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(not(any(
    windows,
    target_os = "android",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos"
)))]
fn ask_here(_request: &SecurityKeyRequest, _wait: &Wait) -> Result<KeyAnswer> {
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
    fn says_where_keys_work() {
        let here = here();
        if cfg!(target_os = "ios") {
            assert_eq!(here, None);
        } else {
            assert!(matches!(here, Some("windows" | "usb" | "phone")));
        }
    }
}
