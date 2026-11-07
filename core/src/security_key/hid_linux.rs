//! Security keys on USB on Linux: the hidraw devices whose report descriptor
//! names the FIDO Alliance's usage page (0xF1D0), read and written directly,
//! as browsers do. udev gives the person at the computer access to them
//! (systemd 244 and later tag security keys for it, `60-fido-id.rules`).

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::time::Duration;

use super::ctap::Hid;
use crate::error::{Error, Result};

pub use super::is_fido;

/// The security keys plugged in.
pub fn devices() -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir("/sys/class/hidraw") else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| {
            fs::read(entry.path().join("device/report_descriptor")).is_ok_and(|d| is_fido(&d))
        })
        .map(|entry| PathBuf::from("/dev").join(entry.file_name()))
        .collect();
    found.sort();
    found
}

pub struct Device {
    file: File,
}

impl Device {
    pub fn open(path: PathBuf) -> Result<Device> {
        match OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => Ok(Device { file }),
            Err(e) if e.kind() == ErrorKind::PermissionDenied => Err(Error::TwoStep(
                "This computer doesn't let PassBundle reach the security key. Your Linux needs the udev rules for security keys (systemd 244 or later has them), or use another way to sign in.",
            )),
            Err(e) => Err(Error::TwoStep(if e.kind() == ErrorKind::NotFound {
                "The security key was unplugged. Plug it in again."
            } else {
                "PassBundle couldn't reach the security key. Unplug it, plug it in again, and try again."
            })),
        }
    }
}

fn unplugged() -> Error {
    Error::TwoStep("The security key was unplugged. Plug it in again.")
}

impl Hid for Device {
    fn write(&mut self, report: &[u8; 64]) -> Result<()> {
        // Report number 0 first: a security key's reports aren't numbered.
        let mut out = [0u8; 65];
        out[1..].copy_from_slice(report);
        self.file.write_all(&out).map_err(|_| unplugged())
    }

    fn read(&mut self, timeout: Duration) -> Result<Option<[u8; 64]>> {
        let mut poll = libc::pollfd {
            fd: self.file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        // SAFETY: one pollfd, valid for the call.
        let ready = unsafe { libc::poll(&mut poll, 1, ms) };
        if ready < 0 {
            let e = std::io::Error::last_os_error();
            return if e.kind() == ErrorKind::Interrupted {
                Ok(None)
            } else {
                Err(unplugged())
            };
        }
        if ready == 0 {
            return Ok(None);
        }
        if poll.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err(unplugged());
        }
        let mut report = [0u8; 64];
        match self.file.read(&mut report) {
            Ok(64) => Ok(Some(report)),
            Ok(_) => Ok(None),
            Err(e) if e.kind() == ErrorKind::Interrupted => Ok(None),
            Err(_) => Err(unplugged()),
        }
    }
}
