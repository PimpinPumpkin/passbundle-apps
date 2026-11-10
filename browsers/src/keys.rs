//! The keys a Chromium browser's saved passwords are encrypted with, from
//! where the OS keeps them for the person, through the OS's own way of
//! handing them to a program the person runs:
//!
//! - **macOS:** the browser's "Safe Storage" item in the person's login
//!   keychain. macOS asks the person first whether PassBundle may use it
//!   (the browser's item lets only the browser in without asking), and they
//!   can say no.
//! - **Linux:** the browser's item in the desktop's keyring, through the
//!   Secret Service on the session's D-Bus. A locked keyring stays locked.
//! - **Windows:** Windows' data protection for the person (DPAPI), which
//!   opens what it protected for that same person and no one else.
//!
//! Nothing here runs as anyone else, asks a browser's own services for
//! anything, or turns any protection off. Tests use made-up keys instead
//! (`fixtures::TestKeys`), and never these.

use zeroize::Zeroizing;

use crate::Browser;

/// Why the OS didn't hand over a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyError {
    /// The person said no (macOS's keychain prompt), or it asked and
    /// couldn't (nobody at the screen).
    Denied,
    /// There's no such key.
    NotFound,
    /// The keyring that holds it is locked.
    Locked,
    /// No keychain, keyring, or data protection to ask here.
    Unavailable,
}

/// Where the keys come from.
pub trait Keys {
    /// macOS and Linux: the secret a Chromium browser keeps in the person's
    /// keychain or keyring ("Chrome Safe Storage").
    fn safe_storage(&self, browser: Browser) -> Result<Zeroizing<Vec<u8>>, KeyError>;

    /// Windows: something Windows protected for this person (DPAPI), opened.
    fn unprotect(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, KeyError>;
}

/// The OS's own, for this person.
pub struct SystemKeys;

/// A Chromium browser's item in the macOS keychain: its service and account.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn keychain_item(browser: Browser) -> Option<(&'static str, &'static str)> {
    Some(match browser {
        Browser::Chrome => ("Chrome Safe Storage", "Chrome"),
        Browser::Edge => ("Microsoft Edge Safe Storage", "Microsoft Edge"),
        Browser::Brave => ("Brave Safe Storage", "Brave"),
        Browser::Vivaldi => ("Vivaldi Safe Storage", "Vivaldi"),
        Browser::Opera => ("Opera Safe Storage", "Opera"),
        Browser::Chromium => ("Chromium Safe Storage", "Chromium"),
        Browser::Firefox | Browser::Safari => return None,
    })
}

/// A Chromium browser's `application` in the keyring's attributes (Linux).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn keyring_application(browser: Browser) -> Option<&'static str> {
    Some(match browser {
        Browser::Chrome => "chrome",
        Browser::Edge => "microsoft-edge",
        Browser::Brave => "brave",
        Browser::Vivaldi => "vivaldi",
        Browser::Opera => "opera",
        Browser::Chromium => "chromium",
        Browser::Firefox | Browser::Safari => return None,
    })
}

impl Keys for SystemKeys {
    fn safe_storage(&self, browser: Browser) -> Result<Zeroizing<Vec<u8>>, KeyError> {
        os::safe_storage(browser)
    }

    fn unprotect(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, KeyError> {
        os::unprotect(blob)
    }
}

#[cfg(target_os = "macos")]
mod os {
    use zeroize::Zeroizing;

    use super::{keychain_item, KeyError};
    use crate::Browser;

    pub fn safe_storage(browser: Browser) -> Result<Zeroizing<Vec<u8>>, KeyError> {
        let (service, account) = keychain_item(browser).ok_or(KeyError::NotFound)?;
        match security_framework::passwords::get_generic_password(service, account) {
            Ok(secret) => Ok(Zeroizing::new(secret)),
            Err(e) => Err(match e.code() {
                // errSecItemNotFound
                -25300 => KeyError::NotFound,
                // errSecUserCanceled, errSecAuthFailed: the person said no.
                -128 | -25293 => KeyError::Denied,
                // errSecInteractionNotAllowed: no one to ask; errSecNoSuchKeychain.
                -25308 => KeyError::Denied,
                -25294 => KeyError::Unavailable,
                _ => KeyError::Denied,
            }),
        }
    }

    pub fn unprotect(_: &[u8]) -> Result<Zeroizing<Vec<u8>>, KeyError> {
        Err(KeyError::Unavailable)
    }
}

#[cfg(target_os = "linux")]
mod os {
    use std::collections::HashMap;
    use std::time::Duration;

    use dbus::arg::{RefArg, Variant};
    use dbus::blocking::Connection;
    use dbus::Path;
    use zeroize::Zeroizing;

    use super::{keyring_application, KeyError};
    use crate::Browser;

    const SERVICE: &str = "org.freedesktop.secrets";

    /// A secret as the Secret Service hands it over: its session, its
    /// parameters, its value, and its content type.
    type Secret = (Path<'static>, Vec<u8>, Vec<u8>, String);
    const WAIT: Duration = Duration::from_secs(5);

    pub fn safe_storage(browser: Browser) -> Result<Zeroizing<Vec<u8>>, KeyError> {
        let application = keyring_application(browser).ok_or(KeyError::NotFound)?;
        let bus = Connection::new_session().map_err(|_| KeyError::Unavailable)?;
        let secrets = bus.with_proxy(SERVICE, "/org/freedesktop/secrets", WAIT);
        let attributes: HashMap<&str, &str> = HashMap::from([("application", application)]);
        let (unlocked, locked): (Vec<Path<'static>>, Vec<Path<'static>>) = secrets
            .method_call(
                "org.freedesktop.Secret.Service",
                "SearchItems",
                (attributes,),
            )
            .map_err(|_| KeyError::Unavailable)?;
        let Some(item) = unlocked.into_iter().next() else {
            return Err(if locked.is_empty() {
                KeyError::NotFound
            } else {
                KeyError::Locked
            });
        };
        // "plain": the secret comes over the person's own session bus as it
        // is, which only their programs can reach.
        let (_, session): (Variant<Box<dyn RefArg>>, Path<'static>) = secrets
            .method_call(
                "org.freedesktop.Secret.Service",
                "OpenSession",
                ("plain", Variant(String::new())),
            )
            .map_err(|_| KeyError::Unavailable)?;
        let proxy = bus.with_proxy(SERVICE, item, WAIT);
        let answer: Result<(Secret,), _> = proxy.method_call(
            "org.freedesktop.Secret.Item",
            "GetSecret",
            (session.clone(),),
        );
        let _ = bus
            .with_proxy(SERVICE, session, WAIT)
            .method_call::<(), _, _, _>("org.freedesktop.Secret.Session", "Close", ());
        let ((_, _, value, _),) = answer.map_err(|_| KeyError::Locked)?;
        Ok(Zeroizing::new(value))
    }

    pub fn unprotect(_: &[u8]) -> Result<Zeroizing<Vec<u8>>, KeyError> {
        Err(KeyError::Unavailable)
    }
}

#[cfg(windows)]
pub(crate) mod os {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    use zeroize::{Zeroize, Zeroizing};

    use super::KeyError;
    use crate::Browser;

    pub fn safe_storage(_: Browser) -> Result<Zeroizing<Vec<u8>>, KeyError> {
        Err(KeyError::Unavailable)
    }

    /// Opens what Windows protected for this person, without showing
    /// anything (it never needs to for a person's own data).
    pub fn unprotect(blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, KeyError> {
        let input = CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(blob.len()).map_err(|_| KeyError::NotFound)?,
            pbData: blob.as_ptr().cast_mut(),
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: `input` points at `blob`, which outlives the call; Windows
        // allocates `output` with LocalAlloc, which is copied, wiped, and freed.
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 || output.pbData.is_null() {
            return Err(KeyError::NotFound);
        }
        // SAFETY: Windows says `output` holds `cbData` bytes at `pbData`.
        let opened = unsafe {
            let bytes = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
            let copy = Zeroizing::new(bytes.to_vec());
            bytes.zeroize();
            LocalFree(output.pbData.cast());
            copy
        };
        Ok(opened)
    }

    /// Protects bytes for this person, as a browser does its key: for tests
    /// that make a Windows profile with the real data protection.
    #[cfg(any(test, feature = "fixtures"))]
    pub fn protect(bytes: &[u8]) -> Vec<u8> {
        use windows_sys::Win32::Security::Cryptography::CryptProtectData;
        let input = CRYPT_INTEGER_BLOB {
            cbData: bytes.len() as u32,
            pbData: bytes.as_ptr().cast_mut(),
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: as in `unprotect`.
        let ok = unsafe {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        assert!(ok != 0, "Windows protected the test key");
        // SAFETY: as in `unprotect`.
        unsafe {
            let copy = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
            LocalFree(output.pbData.cast());
            copy
        }
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod os {
    use zeroize::Zeroizing;

    use super::KeyError;
    use crate::Browser;

    pub fn safe_storage(_: Browser) -> Result<Zeroizing<Vec<u8>>, KeyError> {
        Err(KeyError::Unavailable)
    }

    pub fn unprotect(_: &[u8]) -> Result<Zeroizing<Vec<u8>>, KeyError> {
        Err(KeyError::Unavailable)
    }
}
