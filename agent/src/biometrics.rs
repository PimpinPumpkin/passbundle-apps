//! The platform's biometrics behind [`Biometrics`]: Windows Hello on Windows
//! ([`crate::windows_hello`]). On macOS, Touch ID waits for the agent to be
//! signed (DESIGN.md, "Biometric unlock"), so it reports none for now.

use zeroize::Zeroizing;

use crate::protocol::{Answer, Biometrics, BiometricsStatus};

#[cfg(all(feature = "test-authenticator", not(debug_assertions)))]
compile_error!("the test authenticator is for test builds only; release builds can't include it");

/// What prompts are called, for prompt text.
#[cfg(windows)]
pub const METHOD: &str = "Windows Hello";
#[cfg(not(windows))]
pub const METHOD: &str = "Touch ID";

pub fn platform() -> Box<dyn Biometrics> {
    #[cfg(feature = "test-authenticator")]
    if let Some(dir) = std::env::var_os("PASSBUNDLE_AGENT_TEST_AUTHENTICATOR") {
        return Box::new(test::Scripted::new(dir.into()));
    }
    #[cfg(windows)]
    return Box::new(crate::windows_hello::WindowsHello::new());
    #[cfg(not(windows))]
    Box::new(Unsupported)
}

/// No biometrics on this platform (yet).
pub struct Unsupported;

impl Biometrics for Unsupported {
    fn status(&self) -> BiometricsStatus {
        // Not PlatformUnsupported: the extension still offers the setting
        // for that one, and turning it on would only fail.
        BiometricsStatus::HardwareUnavailable
    }
    fn is_enrolled(&self, _user_id: &str) -> bool {
        false
    }
    fn enroll(&self, _user_id: &str, _user_key: &[u8], _reason: &str) -> Answer<()> {
        Answer::Failed("not supported on this computer".into())
    }
    fn unlock(&self, _user_id: &str, _reason: &str) -> Answer<Zeroizing<Vec<u8>>> {
        Answer::Failed("not supported on this computer".into())
    }
    fn authenticate(&self, _reason: &str) -> Answer<()> {
        Answer::Failed("not supported on this computer".into())
    }
    fn remove(&self, _user_id: &str) {}
}

/// A stand-in for Windows Hello in end-to-end tests: each prompt takes the
/// next answer ("yes" or "no") from `<dir>/answers`, one per line, and the
/// prompt texts go to `<dir>/prompts`. Keys stay in memory.
#[cfg(feature = "test-authenticator")]
mod test {
    use std::{collections::HashMap, fs, io::Write, path::PathBuf, sync::Mutex};

    use super::*;

    pub struct Scripted {
        dir: PathBuf,
        keys: Mutex<HashMap<String, Vec<u8>>>,
    }

    impl Scripted {
        pub fn new(dir: PathBuf) -> Self {
            Self {
                dir,
                keys: Mutex::default(),
            }
        }

        fn answer(&self, reason: &str) -> bool {
            let mut prompts = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.dir.join("prompts"))
                .expect("prompts file");
            writeln!(prompts, "{reason}").unwrap();
            let path = self.dir.join("answers");
            let answers = fs::read_to_string(&path).unwrap_or_default();
            let mut lines = answers.lines();
            let answer = lines.next() == Some("yes");
            fs::write(&path, lines.collect::<Vec<_>>().join("\n")).unwrap();
            answer
        }
    }

    impl Biometrics for Scripted {
        fn status(&self) -> BiometricsStatus {
            BiometricsStatus::Available
        }
        fn is_enrolled(&self, user_id: &str) -> bool {
            self.keys.lock().unwrap().contains_key(user_id)
        }
        fn enroll(&self, user_id: &str, user_key: &[u8], reason: &str) -> Answer<()> {
            if !self.answer(reason) {
                return Answer::Canceled;
            }
            self.keys
                .lock()
                .unwrap()
                .insert(user_id.into(), user_key.to_vec());
            Answer::Approved(())
        }
        fn unlock(&self, user_id: &str, reason: &str) -> Answer<Zeroizing<Vec<u8>>> {
            if !self.answer(reason) {
                return Answer::Canceled;
            }
            match self.keys.lock().unwrap().get(user_id) {
                Some(key) => Answer::Approved(Zeroizing::new(key.clone())),
                None => Answer::Lost("not enrolled".into()),
            }
        }
        fn authenticate(&self, reason: &str) -> Answer<()> {
            if self.answer(reason) {
                Answer::Approved(())
            } else {
                Answer::Canceled
            }
        }
        fn remove(&self, user_id: &str) {
            self.keys.lock().unwrap().remove(user_id);
        }
    }
}
