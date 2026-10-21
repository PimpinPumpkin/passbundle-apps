//! Copying a secret without it passing through Dart (docs/core.md, "Memory"):
//! the core puts it on the clipboard, marked to stay out of clipboard history
//! where the OS has a way (Windows: history, cloud clipboard, and clipboard
//! monitors; macOS: the concealed type clipboard managers respect), and clears
//! it after 30 seconds if it's still there. Phones copy through the app's own
//! native piece instead (docs/phones.md, "Copying"), which isn't built yet.

#[cfg(not(phone))]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(phone))]
use std::sync::Mutex;
use std::time::Duration;

#[cfg(not(phone))]
use arboard::Clipboard;
#[cfg(not(phone))]
use zeroize::Zeroizing;

use crate::error::{Error, Result};

pub const CLEAR_AFTER: Duration = Duration::from_secs(30);

// One clipboard for the process: on Linux the copying process serves the
// contents, so it must stay alive.
#[cfg(not(phone))]
static CLIPBOARD: Mutex<Option<Clipboard>> = Mutex::new(None);
// Each copy's number; a later copy cancels an earlier one's clearing.
#[cfg(not(phone))]
static GENERATION: AtomicU64 = AtomicU64::new(0);

#[cfg(not(phone))]
fn with_clipboard<T>(
    f: impl FnOnce(&mut Clipboard) -> std::result::Result<T, arboard::Error>,
) -> Result<T> {
    let mut guard = CLIPBOARD
        .lock()
        .map_err(|_| Error::Damaged("the clipboard"))?;
    if guard.is_none() {
        *guard = Some(Clipboard::new().map_err(|e| Error::Network(format!("no clipboard ({e})")))?);
    }
    f(guard.as_mut().expect("just made"))
        .map_err(|e| Error::Network(format!("the clipboard refused ({e})")))
}

/// Copies a secret, and clears it after `clear_after` if nothing replaced it.
#[cfg(not(phone))]
pub fn copy_secret(text: &str, clear_after: Duration) -> Result<()> {
    with_clipboard(|c| {
        let set = c.set();
        #[cfg(target_os = "windows")]
        let set = {
            use arboard::SetExtWindows;
            set.exclude_from_history()
                .exclude_from_cloud()
                .exclude_from_monitoring()
        };
        #[cfg(target_os = "macos")]
        let set = {
            use arboard::SetExtApple;
            set.exclude_from_history()
        };
        set.text(text)
    })?;
    let mine = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let copied = Zeroizing::new(text.to_owned());
    std::thread::spawn(move || {
        std::thread::sleep(clear_after);
        if GENERATION.load(Ordering::SeqCst) != mine {
            return;
        }
        // Only if it's still what we put there: never clear something the person copied since.
        let _ = with_clipboard(|c| {
            let now = Zeroizing::new(c.get_text().unwrap_or_default());
            if *now == *copied {
                c.clear()
            } else {
                Ok(())
            }
        });
    });
    Ok(())
}

/// Copies something that isn't secret (a username, a website).
#[cfg(not(phone))]
pub fn copy_text(text: &str) -> Result<()> {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    with_clipboard(|c| c.set_text(text))
}

#[cfg(phone)]
pub fn copy_secret(_text: &str, _clear_after: Duration) -> Result<()> {
    Err(Error::Refused("copying isn't built for phones yet"))
}

#[cfg(phone)]
pub fn copy_text(_text: &str) -> Result<()> {
    Err(Error::Refused("copying isn't built for phones yet"))
}

#[cfg(all(test, not(phone)))]
mod tests {
    use super::*;

    /// Uses the real clipboard, so it runs only where PB_TEST_CLIPBOARD is set
    /// (CI's macOS and Windows machines), never on someone's own computer.
    #[test]
    fn clears_a_secret_later_but_never_what_was_copied_since() {
        if std::env::var_os("PB_TEST_CLIPBOARD").is_none() {
            eprintln!("PB_TEST_CLIPBOARD isn't set; skipping the clipboard test.");
            return;
        }
        let read = || with_clipboard(|c| c.get_text()).unwrap_or_default();
        copy_secret("not-a-real-password-1", Duration::from_millis(500)).unwrap();
        assert_eq!(read(), "not-a-real-password-1");
        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(read(), "", "cleared after the delay");

        copy_secret("not-a-real-password-2", Duration::from_millis(500)).unwrap();
        copy_text("ana@example.test").unwrap();
        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(read(), "ana@example.test", "what was copied since stays");
    }
}
