//! Websites' icons for logins' tiles, as the web vault and the extension
//! show them (passbundle-clients, build-cipher-icon.ts and item-tile): from
//! the server's icon service, kept on this device for a while, with the
//! login's letter where there's none. The person can turn them off (each
//! one tells the server a website they have a login for).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How long an icon is kept before asking again, and how long "it has none".
const KEEP: Duration = Duration::from_secs(7 * 24 * 3600);
const KEEP_NONE: Duration = Duration::from_secs(3 * 24 * 3600);

/// The website a login's icon is for, as upstream's apps decide it: its
/// first address's host, for a website (an http(s) address, or a bare one
/// with a dot in it), not an app's (androidapp://, iosapp://), nor an onion
/// or I2P site. Only plain ASCII host names: anything else shows the letter.
pub fn host_of(uri: &str) -> Option<String> {
    let uri = uri.trim();
    if uri.is_empty() || uri.starts_with("androidapp://") || uri.starts_with("iosapp://") {
        return None;
    }
    let rest = match uri.split_once("://") {
        None if uri.contains('.') => uri,
        None => return None,
        Some((scheme, rest)) => {
            let scheme = scheme.to_ascii_lowercase();
            if (scheme != "http" && scheme != "https") || !rest.contains('.') {
                return None;
            }
            rest
        }
    };
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    if host.starts_with('[') {
        return None;
    }
    let host = host
        .split(':')
        .next()?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let valid = !host.is_empty()
        && host.len() <= 253
        && host.contains('.')
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && !host.starts_with('.')
        && !host.contains("..");
    if !valid || host.ends_with(".onion") || host.ends_with(".i2p") {
        return None;
    }
    Some(host)
}

/// The server's own small globe for a site with no icon (Vaultwarden's
/// fallback, a 19 by 19 pixel PNG): tiles show the letter instead.
pub fn is_fallback(image: &[u8]) -> bool {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if image.len() < 24 || !image.starts_with(PNG) || &image[12..16] != b"IHDR" {
        return false;
    }
    let width = u32::from_be_bytes([image[16], image[17], image[18], image[19]]);
    let height = u32::from_be_bytes([image[20], image[21], image[22], image[23]]);
    width == 19 && height == 19
}

/// Where icons are kept: a folder next to the vault.
pub fn folder(store: &Path) -> PathBuf {
    store.join("icons")
}

/// What's kept for a host: Some(Some(path)) for an icon, Some(None) when
/// the server had none lately, None when it's time to ask.
pub fn kept(folder: &Path, host: &str) -> Option<Option<PathBuf>> {
    let fresh = |path: &Path, keep: Duration| {
        std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|at| SystemTime::now().duration_since(at).ok())
            .is_some_and(|age| age < keep)
    };
    let icon = folder.join(format!("{host}.png"));
    if fresh(&icon, KEEP) {
        return Some(Some(icon));
    }
    if fresh(&folder.join(format!("{host}.none")), KEEP_NONE) {
        return Some(None);
    }
    None
}

/// Deletes every kept icon (on sign out, and when icons are turned off).
/// Windows can't delete a file another program has open for a moment (a
/// virus scanner reading a new file, say), so it tries again a few times,
/// for at most about a second and a half. True when they're gone.
pub fn forget(folder: &Path) -> bool {
    for attempt in 0..5 {
        match std::fs::remove_dir_all(folder) {
            Ok(()) => return true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) if attempt < 4 => std::thread::sleep(Duration::from_millis(100 << attempt)),
            Err(_) => {}
        }
    }
    !folder.exists()
}

/// Keeps what the server answered for a host: its icon, or that it has none.
pub fn keep(folder: &Path, host: &str, icon: Option<&[u8]>) -> Option<PathBuf> {
    std::fs::create_dir_all(folder).ok()?;
    let (path, other) = (
        folder.join(format!("{host}.png")),
        folder.join(format!("{host}.none")),
    );
    match icon {
        Some(bytes) => {
            let partial = folder.join(format!("{host}.partial"));
            std::fs::write(&partial, bytes).ok()?;
            std::fs::rename(&partial, &path).ok()?;
            let _ = std::fs::remove_file(other);
            Some(path)
        }
        None => {
            let _ = std::fs::write(&other, b"");
            let _ = std::fs::remove_file(path);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_logins_icon_is_for_its_websites_host() {
        for (uri, host) in [
            ("https://mail.example.test/login", Some("mail.example.test")),
            ("http://Example.COM:8080/path?q=1#x", Some("example.com")),
            ("example.com", Some("example.com")),
            (
                "https://user:pw@accounts.example.com",
                Some("accounts.example.com"),
            ),
            ("androidapp://com.example.app", None),
            ("iosapp://com.example.app", None),
            ("https://localhost:8443", None),
            ("ftp://files.example.com", None),
            ("https://hidden.onion", None),
            ("https://[::1]/", None),
            ("https://bücher.example", None),
            ("", None),
            ("just words", None),
        ] {
            assert_eq!(host_of(uri).as_deref(), host, "{uri}");
        }
    }

    #[test]
    fn the_servers_fallback_globe_counts_as_no_icon() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&19u32.to_be_bytes());
        png.extend_from_slice(&19u32.to_be_bytes());
        assert!(is_fallback(&png));
        png[19] = 32;
        png[23] = 32;
        assert!(!is_fallback(&png));
        assert!(!is_fallback(b"<svg/>"));
    }

    #[test]
    fn icons_are_kept_and_asked_for_again_later() {
        let dir = std::env::temp_dir().join(format!("pb-icons-{}", uuid::Uuid::new_v4().simple()));
        assert_eq!(kept(&dir, "example.com"), None);
        let path = keep(&dir, "example.com", Some(b"icon")).unwrap();
        assert_eq!(kept(&dir, "example.com"), Some(Some(path)));
        keep(&dir, "none.example", None);
        assert_eq!(kept(&dir, "none.example"), Some(None));
        assert!(forget(&dir));
        assert!(!dir.exists());
        assert!(forget(&dir), "nothing to forget is forgotten");
    }
}
