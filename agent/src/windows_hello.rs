//! Windows Hello for biometric unlock, as upstream's GPL implementation does
//! it (`desktop_native/biometric/src/windows.rs`), without its SDK crates:
//!
//! - A Windows Hello key credential named for the agent, made on the first
//!   enrollment. Windows signs with it only after the person authenticates.
//! - Each enrollment's random challenge, signed, seals the account's user key
//!   ([`crate::sealed`]); the sealed key is stored in the user's app data,
//!   wrapped with DPAPI for the signed-in Windows user.
//! - Unlocking signs the same challenge again (a prompt) and opens it.
//!
//! Windows shows its own text in the key credential's prompt; the agent can't
//! add the browser's or the account's name to it. The prompt can open behind
//! the browser, so while one is up the agent brings it to the front, with the
//! foreground rights the native-messaging host hands it (see `host.rs`).

use std::{
    fs,
    path::PathBuf,
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use sha2::{Digest, Sha256};
use windows::{
    core::{Array, HSTRING},
    Security::{
        Credentials::{
            KeyCredentialCreationOption, KeyCredentialManager, KeyCredentialStatus,
            UI::{UserConsentVerificationResult, UserConsentVerifier},
        },
        Cryptography::CryptographicBuffer,
    },
};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    },
    UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow},
};
use zeroize::Zeroizing;

use crate::{
    brand,
    protocol::{Answer, Biometrics, BiometricsStatus},
    sealed::{self, Sealed},
};

pub struct WindowsHello {
    dir: PathBuf,
}

impl WindowsHello {
    pub fn new() -> Self {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_default();
        Self {
            dir: base.join(brand::NAME).join("biometric"),
        }
    }

    /// One file per account, named by a hash of its id.
    fn path(&self, user_id: &str) -> PathBuf {
        let name: String = Sha256::digest(user_id.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        self.dir.join(format!("{name}.bin"))
    }

    fn read(&self, user_id: &str) -> Option<Sealed> {
        let wrapped = fs::read(self.path(user_id)).ok()?;
        let json = dpapi_unprotect(&wrapped)?;
        serde_json::from_slice(&json).ok()
    }

    fn write(&self, user_id: &str, sealed: &Sealed) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let json = Zeroizing::new(serde_json::to_vec(sealed)?);
        let wrapped = dpapi_protect(&json)
            .ok_or_else(|| std::io::Error::other("DPAPI couldn't protect the key"))?;
        fs::write(self.path(user_id), wrapped)
    }
}

impl Default for WindowsHello {
    fn default() -> Self {
        Self::new()
    }
}

impl Biometrics for WindowsHello {
    fn status(&self) -> BiometricsStatus {
        match KeyCredentialManager::IsSupportedAsync().and_then(|op| op.join()) {
            Ok(true) => BiometricsStatus::Available,
            _ => BiometricsStatus::HardwareUnavailable,
        }
    }

    fn is_enrolled(&self, user_id: &str) -> bool {
        self.path(user_id).exists()
    }

    fn enroll(&self, user_id: &str, user_key: &[u8], _reason: &str) -> Answer<()> {
        let challenge = sealed::new_challenge();
        let signature = match sign(&challenge) {
            Answer::Approved(signature) => signature,
            Answer::Canceled => return Answer::Canceled,
            Answer::Failed(e) | Answer::Lost(e) => return Answer::Failed(e),
        };
        match self.write(
            user_id,
            &sealed::seal(user_id, &challenge, &signature, user_key),
        ) {
            Ok(()) => Answer::Approved(()),
            Err(e) => Answer::Failed(format!("couldn't store the key: {e}")),
        }
    }

    fn unlock(&self, user_id: &str, _reason: &str) -> Answer<Zeroizing<Vec<u8>>> {
        let Some(sealed) = self.read(user_id) else {
            return Answer::Lost("no stored key".into());
        };
        let Some(challenge) = sealed.challenge() else {
            return Answer::Lost("stored key is malformed".into());
        };
        match sign(&challenge) {
            // A signature that doesn't open it comes from a different Windows
            // Hello key: Windows Hello was set up again since.
            Answer::Approved(signature) => match sealed.open(user_id, &signature) {
                Some(key) => Answer::Approved(key),
                None => Answer::Lost("the stored key doesn't open with this Windows Hello".into()),
            },
            Answer::Canceled => Answer::Canceled,
            Answer::Failed(e) => Answer::Failed(e),
            Answer::Lost(e) => Answer::Lost(e),
        }
    }

    fn authenticate(&self, reason: &str) -> Answer<()> {
        let _focus = Focus::start();
        match UserConsentVerifier::RequestVerificationAsync(&HSTRING::from(reason))
            .and_then(|op| op.join())
        {
            Ok(UserConsentVerificationResult::Verified) => Answer::Approved(()),
            Ok(UserConsentVerificationResult::Canceled) => Answer::Canceled,
            Ok(other) => Answer::Failed(format!("Windows Hello said {other:?}")),
            Err(e) => Answer::Failed(e.to_string()),
        }
    }

    fn remove(&self, user_id: &str) {
        let _ = fs::remove_file(self.path(user_id));
    }
}

/// Signs `challenge` with the agent's Windows Hello key credential, making it
/// the first time. Each signature is a prompt.
fn sign(challenge: &[u8]) -> Answer<Vec<u8>> {
    let _focus = Focus::start();
    let name = HSTRING::from(format!("{} desktop agent", brand::NAME));
    let result = (|| -> windows::core::Result<Answer<Vec<u8>>> {
        let created = KeyCredentialManager::RequestCreateAsync(
            &name,
            KeyCredentialCreationOption::FailIfExists,
        )?
        .join()?;
        let credential = match created.Status()? {
            KeyCredentialStatus::Success => created.Credential()?,
            KeyCredentialStatus::CredentialAlreadyExists => KeyCredentialManager::OpenAsync(&name)?
                .join()?
                .Credential()?,
            KeyCredentialStatus::UserCanceled => return Ok(Answer::Canceled),
            other => {
                return Ok(Answer::Failed(format!(
                    "couldn't make the Windows Hello key: {other:?}"
                )))
            }
        };
        let signed = credential
            .RequestSignAsync(&CryptographicBuffer::CreateFromByteArray(challenge)?)?
            .join()?;
        match signed.Status()? {
            KeyCredentialStatus::Success => {
                let mut bytes = Array::<u8>::new();
                CryptographicBuffer::CopyToByteArray(&signed.Result()?, &mut bytes)?;
                Ok(Answer::Approved(bytes.to_vec()))
            }
            KeyCredentialStatus::UserCanceled => Ok(Answer::Canceled),
            other => Ok(Answer::Failed(format!(
                "Windows Hello didn't sign: {other:?}"
            ))),
        }
    })();
    result.unwrap_or_else(|e| Answer::Failed(e.to_string()))
}

/// While a prompt is up, keeps bringing it to the front: Windows Hello opens
/// it behind the browser when the asking program (the agent) isn't in front.
struct Focus(Arc<AtomicBool>);

impl Focus {
    fn start() -> Self {
        let done = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&done);
        thread::spawn(move || {
            let class: Vec<u16> = "Credential Dialog Xaml Host\0".encode_utf16().collect();
            while !stop.load(Ordering::Relaxed) {
                let window = unsafe { FindWindowW(class.as_ptr(), ptr::null()) };
                if !window.is_null() {
                    unsafe { SetForegroundWindow(window) };
                }
                thread::sleep(Duration::from_millis(250));
            }
        });
        Self(done)
    }
}

impl Drop for Focus {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

fn dpapi_protect(data: &[u8]) -> Option<Vec<u8>> {
    dpapi(data, true)
}

fn dpapi_unprotect(data: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
    dpapi(data, false).map(Zeroizing::new)
}

/// DPAPI for the signed-in Windows user: only they can unwrap it.
fn dpapi(data: &[u8], protect: bool) -> Option<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };
    let ok = unsafe {
        if protect {
            CryptProtectData(
                &input,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if ok == 0 || output.pbData.is_null() {
        return None;
    }
    let bytes =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    unsafe {
        // Wipe the unprotected copy Windows allocated before freeing it.
        if !protect {
            ptr::write_bytes(output.pbData, 0, output.cbData as usize);
        }
        LocalFree(output.pbData as _);
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpapi_round_trips_for_this_user() {
        let wrapped = dpapi_protect(b"user key").unwrap();
        assert_ne!(wrapped, b"user key");
        assert_eq!(&*dpapi_unprotect(&wrapped).unwrap(), b"user key");
        assert!(dpapi_unprotect(b"not protected").is_none());
    }

    #[test]
    fn status_answers_without_windows_hello() {
        // CI machines have no Windows Hello; this must report it, not fail.
        let status = WindowsHello::new().status();
        assert!(matches!(
            status,
            BiometricsStatus::Available | BiometricsStatus::HardwareUnavailable
        ));
    }

    #[test]
    fn a_sealed_key_stores_and_opens_from_disk() {
        let hello = WindowsHello {
            dir: std::env::temp_dir().join(format!("pb-hello-{}", std::process::id())),
        };
        let challenge = sealed::new_challenge();
        let signature = [7u8; 256];
        hello
            .write(
                "ana",
                &sealed::seal("ana", &challenge, &signature, b"user key"),
            )
            .unwrap();
        assert!(hello.is_enrolled("ana") && !hello.is_enrolled("ben"));
        let stored = hello.read("ana").unwrap();
        assert_eq!(&*stored.open("ana", &signature).unwrap(), b"user key");
        hello.remove("ana");
        assert!(!hello.is_enrolled("ana"));
        let _ = fs::remove_dir_all(&hello.dir);
    }
}
