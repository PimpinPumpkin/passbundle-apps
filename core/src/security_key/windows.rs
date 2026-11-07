//! Security keys on Windows: Windows's own WebAuthn API (webauthn.dll,
//! `WebAuthNAuthenticatorGetAssertion`), which browsers use too, since apps
//! can't reach a key's HID device themselves. Windows shows its own dialog
//! (insert, touch, a PIN if the key has one) for USB, NFC, and Bluetooth
//! keys, and for a phone or Windows Hello where the account added one as its
//! security key. The core gives it the client data it made for the server's
//! origin (webauthn.rs), and the site ID and AppID it checked.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use windows_sys::core::{GUID, HRESULT};
use windows_sys::Win32::Foundation::{BOOL, HWND};
use windows_sys::Win32::Networking::WindowsWebServices::{
    WebAuthNAuthenticatorGetAssertion, WebAuthNCancelCurrentOperation, WebAuthNFreeAssertion,
    WebAuthNGetApiVersionNumber, WebAuthNGetCancellationId, WEBAUTHN_ASSERTION,
    WEBAUTHN_AUTHENTICATOR_ATTACHMENT_ANY, WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS,
    WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS_VERSION_3, WEBAUTHN_CLIENT_DATA,
    WEBAUTHN_CLIENT_DATA_CURRENT_VERSION, WEBAUTHN_CREDENTIAL, WEBAUTHN_CREDENTIALS,
    WEBAUTHN_CREDENTIAL_CURRENT_VERSION, WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY,
    WEBAUTHN_HASH_ALGORITHM_SHA_256, WEBAUTHN_USER_VERIFICATION_REQUIREMENT_DISCOURAGED,
    WEBAUTHN_USER_VERIFICATION_REQUIREMENT_PREFERRED,
    WEBAUTHN_USER_VERIFICATION_REQUIREMENT_REQUIRED,
};
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowThreadProcessId, IsWindowVisible,
};

use super::ctap::{KeyAnswer, CANCELED, NOT_THIS_KEY, TIMED_OUT};
use super::webauthn::SecurityKeyRequest;
use crate::error::{Error, Result};

const NTE_USER_CANCELLED: HRESULT = 0x8009_0036_u32 as HRESULT;
const NTE_NOT_FOUND: HRESULT = 0x8009_0011_u32 as HRESULT;
const NTE_INVALID_PARAMETER: HRESULT = 0x8009_0027_u32 as HRESULT;
const ERROR_CANCELLED: HRESULT = 0x8007_04c7_u32 as HRESULT;
const ERROR_TIMEOUT: HRESULT = 0x8007_05b4_u32 as HRESULT;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// This app's window, which Windows's dialog sits over.
fn our_window() -> HWND {
    // SAFETY: plain Win32 calls; the callback only reads its own window's details.
    unsafe {
        let mine = GetCurrentProcessId();
        let foreground = GetForegroundWindow();
        let mut owner = 0u32;
        if !foreground.is_null() {
            GetWindowThreadProcessId(foreground, &mut owner);
            if owner == mine {
                return foreground;
            }
        }
        unsafe extern "system" fn each(window: HWND, found: isize) -> BOOL {
            let found = found as *mut HWND;
            let mut owner = 0u32;
            GetWindowThreadProcessId(window, &mut owner);
            if owner == GetCurrentProcessId() && IsWindowVisible(window) != 0 {
                *found = window;
                return 0;
            }
            1
        }
        let mut found: HWND = std::ptr::null_mut();
        EnumWindows(Some(each), &mut found as *mut HWND as isize);
        if found.is_null() {
            foreground
        } else {
            found
        }
    }
}

/// Whether Windows has its WebAuthn API (Windows 10 1903 and later).
pub fn available() -> bool {
    // SAFETY: takes nothing; webauthn.dll is loaded when first called.
    unsafe { WebAuthNGetApiVersionNumber() >= 1 }
}

/// Asks a key through Windows. `canceled` cancels Windows's dialog.
pub fn ask(request: &SecurityKeyRequest, canceled: &AtomicBool) -> Result<KeyAnswer> {
    if !available() {
        return Err(Error::TwoStep(
            "This version of Windows can't ask a security key for PassBundle. Use another way to sign in, or the web vault.",
        ));
    }
    let rp_id = wide(&request.rp_id);
    let app_id = request.app_id.as_deref().map(wide);
    let mut ids = request.allow.clone();
    let mut credentials: Vec<WEBAUTHN_CREDENTIAL> = ids
        .iter_mut()
        .map(|id| WEBAUTHN_CREDENTIAL {
            dwVersion: WEBAUTHN_CREDENTIAL_CURRENT_VERSION,
            cbId: id.len() as u32,
            pbId: id.as_mut_ptr(),
            pwszCredentialType: WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY,
        })
        .collect();
    let mut client_data_json = request.client_data_json.clone();
    let client_data = WEBAUTHN_CLIENT_DATA {
        dwVersion: WEBAUTHN_CLIENT_DATA_CURRENT_VERSION,
        cbClientDataJSON: client_data_json.len() as u32,
        pbClientDataJSON: client_data_json.as_mut_ptr(),
        pwszHashAlgId: WEBAUTHN_HASH_ALGORITHM_SHA_256,
    };
    let mut cancellation = GUID {
        data1: 0,
        data2: 0,
        data3: 0,
        data4: [0; 8],
    };
    // SAFETY: fills the GUID it's given.
    let can_cancel = unsafe { WebAuthNGetCancellationId(&mut cancellation) } == 0;
    let mut used_app_id: BOOL = 0;
    // SAFETY: an all-zero options struct is "nothing set"; the fields up to
    // version 3's are set below.
    let mut options: WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS = unsafe { std::mem::zeroed() };
    options.dwVersion = WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS_VERSION_3;
    options.dwTimeoutMilliseconds = request.timeout.as_millis().min(u32::MAX as u128) as u32;
    options.CredentialList = WEBAUTHN_CREDENTIALS {
        cCredentials: credentials.len() as u32,
        pCredentials: credentials.as_mut_ptr(),
    };
    options.dwAuthenticatorAttachment = WEBAUTHN_AUTHENTICATOR_ATTACHMENT_ANY;
    options.dwUserVerificationRequirement = match request.user_verification.as_str() {
        "required" => WEBAUTHN_USER_VERIFICATION_REQUIREMENT_REQUIRED,
        "discouraged" => WEBAUTHN_USER_VERIFICATION_REQUIREMENT_DISCOURAGED,
        _ => WEBAUTHN_USER_VERIFICATION_REQUIREMENT_PREFERRED,
    };
    if let Some(app_id) = &app_id {
        options.pwszU2fAppId = app_id.as_ptr();
        options.pbU2fAppId = &mut used_app_id;
    }
    if can_cancel {
        options.pCancellationId = &mut cancellation;
    }

    let window = our_window() as isize;
    let done = AtomicBool::new(false);
    let mut assertion: *mut WEBAUTHN_ASSERTION = std::ptr::null_mut();
    let result = std::thread::scope(|scope| {
        if can_cancel {
            let (done, cancellation) = (&done, &cancellation);
            scope.spawn(move || {
                while !done.load(Ordering::SeqCst) {
                    if canceled.load(Ordering::SeqCst) {
                        // SAFETY: the GUID of the operation running now.
                        unsafe { WebAuthNCancelCurrentOperation(cancellation) };
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            });
        }
        // SAFETY: every pointer in the arguments points into values above,
        // which outlive the call; Windows allocates the assertion, freed below.
        let result = unsafe {
            WebAuthNAuthenticatorGetAssertion(
                window as HWND,
                rp_id.as_ptr(),
                &client_data,
                &options,
                &mut assertion,
            )
        };
        done.store(true, Ordering::SeqCst);
        result
    });
    if result != 0 || assertion.is_null() {
        return Err(Error::TwoStep(match result {
            _ if canceled.load(Ordering::SeqCst) => CANCELED,
            NTE_USER_CANCELLED | ERROR_CANCELLED => CANCELED,
            ERROR_TIMEOUT => TIMED_OUT,
            NTE_NOT_FOUND | NTE_INVALID_PARAMETER => NOT_THIS_KEY,
            _ => "Windows couldn't ask the security key. Try again, or use another way to sign in.",
        }));
    }
    // SAFETY: Windows gave a valid assertion, whose buffers hold the lengths
    // it gives; it's copied, then freed once.
    unsafe {
        let a = &*assertion;
        let bytes = |ptr: *mut u8, len: u32| {
            if ptr.is_null() || len == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(ptr, len as usize).to_vec()
            }
        };
        let answer = KeyAnswer {
            credential_id: bytes(a.Credential.pbId, a.Credential.cbId),
            authenticator_data: bytes(a.pbAuthenticatorData, a.cbAuthenticatorData),
            signature: bytes(a.pbSignature, a.cbSignature),
            user_handle: Some(bytes(a.pbUserId, a.cbUserId)).filter(|h| !h.is_empty()),
            used_app_id: used_app_id != 0,
        };
        WebAuthNFreeAssertion(assertion);
        Ok(answer)
    }
}
