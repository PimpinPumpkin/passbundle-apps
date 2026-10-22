//! What an organization set for the app by policy, where only an
//! administrator can write (docs/core.md, "Servers"): the server, which the
//! RMM or MDM install sets for the desktop agent too. On Windows it's the
//! machine's policy key (`HKLM\SOFTWARE\Policies\<Name>\Agent`, `ServerUrl`),
//! which Install-PassBundle.ps1 writes; on macOS the managed preferences an
//! MDM profile sets (`com.<domain>.agent`, which make-profile.sh writes); on
//! Linux a file only root can write (`/etc/passbundle/policy.json`,
//! `ServerUrl`), which the Linux install script writes. On Android the app's
//! managed configuration, which an MDM sets, is read by the app itself.
//! Nothing is read from where a person, or something running as them, could
//! write, so nobody can point the app at another server this way.

/// The server the organization set, if any.
pub fn server() -> Option<String> {
    os::server()
        .map(|s| s.trim().trim_end_matches('/').to_owned())
        .filter(|s| !s.is_empty())
}

#[cfg(windows)]
mod os {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn server() -> Option<String> {
        let key = wide(&format!(r"SOFTWARE\Policies\{}\Agent", crate::brand::NAME));
        let value = wide("ServerUrl");
        let mut buffer = vec![0u16; 2048];
        let mut bytes = (buffer.len() * 2) as u32;
        // SAFETY: the key and value names are NUL-terminated, and the call
        // writes at most `bytes` bytes into `buffer`, telling us how many.
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if status != 0 {
            return None;
        }
        let chars = (bytes as usize / 2).saturating_sub(1);
        Some(String::from_utf16_lossy(&buffer[..chars.min(buffer.len())]))
    }
}

#[cfg(target_os = "macos")]
mod os {
    pub fn server() -> Option<String> {
        let domain: Vec<&str> = crate::brand::DOMAIN.rsplit('.').collect();
        let path = format!(
            "/Library/Managed Preferences/{}.agent.plist",
            domain.join(".")
        );
        let plist::Value::Dictionary(values) = plist::Value::from_file(path).ok()? else {
            return None;
        };
        values.get("ServerUrl")?.as_string().map(str::to_owned)
    }
}

#[cfg(target_os = "linux")]
mod os {
    use std::os::unix::fs::MetadataExt;

    pub fn server() -> Option<String> {
        let path = format!("/etc/{}/policy.json", crate::brand::NAME.to_lowercase());
        // Only root's, and only root can change it.
        let meta = std::fs::metadata(&path).ok()?;
        if meta.uid() != 0 || meta.mode() & 0o022 != 0 {
            return None;
        }
        let policy: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
        policy["ServerUrl"].as_str().map(str::to_owned)
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod os {
    pub fn server() -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_without_failing() {
        // CI's machines have no policy; a managed one would show here.
        eprintln!("the managed server: {:?}", super::server());
    }
}
