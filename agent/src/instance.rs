//! One agent per signed-in user: the first one holds a lock on a file in the
//! user's own app data folder until it exits, and the OS releases it even if
//! the agent crashes.

use std::{fs, path::PathBuf};

use crate::brand;

/// The lock, held for as long as the value lives. `None` if another agent
/// for this user already holds it.
pub fn acquire() -> Option<fs::File> {
    let dir = data_dir()?;
    fs::create_dir_all(&dir).ok()?;
    acquire_at(dir.join("agent.lock"))
}

fn acquire_at(path: PathBuf) -> Option<fs::File> {
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .ok()?;
    file.try_lock().ok()?;
    Some(file)
}

/// `%LOCALAPPDATA%\<Name>` on Windows, `~/Library/Application Support/<Name>` on macOS.
fn data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support"));
    #[cfg(not(any(windows, target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    Some(base?.join(brand::NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_agent_for_the_same_user_doesnt_start() {
        let path = std::env::temp_dir().join(format!("pb-agent-lock-{}", std::process::id()));
        let first = acquire_at(path.clone());
        assert!(first.is_some());
        assert!(acquire_at(path.clone()).is_none());
        drop(first);
        assert!(acquire_at(path.clone()).is_some());
        let _ = fs::remove_file(path);
    }
}
