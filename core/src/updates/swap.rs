//! Putting a checked update in place, once the app has quit: what
//! `passbundle-update` does (src/bin/passbundle-update.rs). Windows can't
//! replace a program while it runs, nor rename the folder it runs from, so
//! the app quits first, and this program, run from a copy outside that
//! folder, does the rest:
//!
//! 1. waits for the app to end, and stops its desktop agent (which runs
//!    from the same folder, and which the new version starts again);
//! 2. renames the app's folder aside, and the new version's into its place
//!    (both next to it, on the same disk, so each step is a rename);
//! 3. opens the new version, and waits for it to say it has started (its
//!    first frame, [`STARTED`]);
//! 4. then removes the old version; or, if the new one ended or didn't say
//!    so in time, ends it, puts the old version back, and opens that, which
//!    says what happened ([`FAILED`]).
//!
//! Everything it uses is in the work folder ([`super::bundle::work_dir`]),
//! which the app that opens last removes once this has finished ([`DONE`]),
//! so no old version or download is left behind.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::process;

/// What the app leaves for this program: where it is, and what's going on.
pub const STATE: &str = "update.json";
/// The new version, unpacked and checked: the folder inside it.
pub const NEW: &str = "new";
/// The version being replaced, while the new one starts.
pub const OLD: &str = "old";
/// A new version that didn't start, on its way out.
pub const BROKEN: &str = "broken";
/// This program's own copy, outside the app's folder.
pub fn helper_name() -> String {
    format!("passbundle-update{}", std::env::consts::EXE_SUFFIX)
}
/// Written by the app when it has drawn its first frame: its process ID.
pub const STARTED: &str = "started";
/// Written by this program when it's finished, for the app to clean up.
pub const DONE: &str = "done";
/// Written by this program when the update didn't happen: why, and which.
pub const FAILED: &str = "failed.json";
/// This program's process ID, so the app knows whether it's still at work.
pub const HELPER: &str = "helper.pid";

/// How long the app has to quit, and the new version to start.
pub const QUIT_WAIT: Duration = Duration::from_secs(60);
pub const START_WAIT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// The app's folder.
    pub install: PathBuf,
    /// Its program's name in that folder.
    pub program: String,
    /// The version running, and the one replacing it.
    pub from: String,
    pub to: String,
    /// The app, which quits for the update.
    pub app: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failed {
    /// The version that wasn't put in place.
    pub version: String,
    /// Why, for the person.
    pub reason: String,
}

/// What came of [`finish`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Updated,
    /// The old version is still in place, or back in place.
    Kept(String),
}

/// Renames `from` to `to`, trying again for a while: on Windows, something
/// that's just looked in a folder (an antivirus, the search indexer) can hold
/// it for a moment.
pub fn rename(from: &Path, to: &Path, patience: Duration) -> std::io::Result<()> {
    let deadline = Instant::now() + patience;
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if Instant::now() < deadline && e.kind() != std::io::ErrorKind::NotFound => {
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Removes a folder and everything in it, trying again for a while; whether
/// it's gone.
pub fn remove(path: &Path, patience: Duration) -> bool {
    let deadline = Instant::now() + patience;
    loop {
        match fs::remove_dir_all(path) {
            Ok(()) => return true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(250)),
            Err(_) => return false,
        }
    }
}

fn write_failed(work: &Path, state: &State, reason: &str) {
    let failed = Failed {
        version: state.to.clone(),
        reason: reason.to_owned(),
    };
    if let Ok(text) = serde_json::to_string(&failed) {
        let _ = fs::write(work.join(FAILED), text);
    }
}

/// Whether the app at `program`, started as `child`, has said it started.
fn wait_started(work: &Path, child: &mut std::process::Child, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    let pid = child.id().to_string();
    loop {
        if let Ok(text) = fs::read_to_string(work.join(STARTED)) {
            if text.trim() == pid {
                return true;
            }
        }
        // Ended before it said so.
        if !matches!(child.try_wait(), Ok(None)) {
            return false;
        }
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// The desktop agent and its native-messaging host: programs of the app's
/// own folder that keep running when the app quits.
pub fn is_agent(program: &Path) -> bool {
    let name = program
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    name == "passbundle-agent" || name == "passbundle-agent-host"
}

/// Opens the version in `install` again, after a failure, and waits a while
/// for it to say it started, so it's there to clean up.
fn reopen(work: &Path, state: &State) {
    let program = state.install.join(&state.program);
    if let Ok(mut child) = process::start_app(&program) {
        wait_started(work, &mut child, START_WAIT);
    }
}

/// The whole update, from the work folder `work` that the app made.
pub fn finish(work: &Path) -> Outcome {
    let _ = fs::write(work.join(HELPER), std::process::id().to_string());
    let state: State = match fs::read(work.join(STATE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    {
        Some(state) => state,
        None => return Outcome::Kept("the update's details are missing".into()),
    };
    let install = &state.install;
    let new = work.join(NEW).join(super::unpack::TOP);
    let old = work.join(OLD);
    let broken = work.join(BROKEN);
    let kept = |reason: &str, reopen_it: bool| {
        write_failed(work, &state, reason);
        remove(&new, Duration::from_secs(10));
        if reopen_it && process::under(install).is_empty() {
            reopen(work, &state);
        }
        let _ = fs::write(work.join(DONE), "");
        Outcome::Kept(reason.to_owned())
    };

    // 1. The app quits; its agent and anything else from its folder stop.
    if !process::wait_for(state.app, QUIT_WAIT) {
        return kept("PassBundle didn't quit, so it wasn't updated.", false);
    }
    let agents: Vec<_> = process::under(install)
        .into_iter()
        .filter(|p| is_agent(&p.program))
        .collect();
    process::stop(&agents, Duration::from_secs(10));
    let deadline = Instant::now() + Duration::from_secs(20);
    while !process::under(install).is_empty() {
        if Instant::now() > deadline {
            return kept(
                "PassBundle was still open in another window, so it wasn't updated. Close every PassBundle window, then update again.",
                false,
            );
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    // 2. The old version aside, the new one in its place.
    if !new.is_dir() {
        return kept("The update wasn't there to put in place.", true);
    }
    if rename(install, &old, Duration::from_secs(20)).is_err() {
        return kept(
            "Something had PassBundle's folder open, so it wasn't updated. Try again in a moment.",
            true,
        );
    }
    if rename(&new, install, Duration::from_secs(20)).is_err() {
        if rename(&old, install, Duration::from_secs(60)).is_err() {
            // Nothing more to try: say where the old version is.
            write_failed(
                work,
                &state,
                &format!(
                    "PassBundle couldn't be put back in place. It's in {}.",
                    old.display()
                ),
            );
            return Outcome::Kept("the old version couldn't be put back".into());
        }
        return kept(
            "The new version couldn't be put in place, so PassBundle wasn't updated.",
            true,
        );
    }

    // 3. The new version opens, and says it started.
    let program = install.join(&state.program);
    let started = match process::start_app(&program) {
        Ok(mut child) => {
            let ok = wait_started(work, &mut child, START_WAIT);
            if !ok {
                let _ = child.kill();
                let _ = child.wait();
            }
            ok
        }
        Err(_) => false,
    };

    // 4. The old version goes; or it comes back.
    if started {
        remove(&old, Duration::from_secs(60));
        remove(&work.join(NEW), Duration::from_secs(10));
        let _ = fs::write(work.join(DONE), "");
        return Outcome::Updated;
    }
    // Whatever the new version left running from its folder (its agent).
    process::stop(&process::under(install), Duration::from_secs(10));
    let back = rename(install, &broken, Duration::from_secs(20))
        .and_then(|()| rename(&old, install, Duration::from_secs(60)));
    if back.is_err() {
        write_failed(
            work,
            &state,
            &format!(
                "PassBundle {} didn't start, and the old version couldn't be put back. It's in {}.",
                state.to,
                old.display()
            ),
        );
        return Outcome::Kept("the old version couldn't be put back".into());
    }
    remove(&broken, Duration::from_secs(30));
    kept(
        &format!(
            "PassBundle {} didn't start, so PassBundle went back to {}.",
            state.to, state.from
        ),
        true,
    )
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A folder like an installed PassBundle's, whose "program" is a shell
    /// script: it says it started (or doesn't), and stays up a while.
    fn app(dir: &Path, work: &Path, version: &str, starts: bool) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("version"), version).unwrap();
        let script = if starts {
            format!(
                "#!/bin/sh\necho $$ > '{}/started'\nexec sleep 30\n",
                work.display()
            )
        } else {
            "#!/bin/sh\nexit 3\n".to_owned()
        };
        let program = dir.join("passbundle");
        fs::write(&program, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn setup(name: &str, new_starts: bool) -> (PathBuf, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "pb-swap-{name}-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        let install = base.join("PassBundle");
        let work = base.join(".PassBundle-update");
        app(&install, &work, "0.1.1", true);
        app(
            &work.join(NEW).join("PassBundle"),
            &work,
            "0.1.2",
            new_starts,
        );
        // The app that quits: one that's already ended.
        let mut quit = std::process::Command::new("true").spawn().unwrap();
        quit.wait().unwrap();
        let state = State {
            install: install.clone(),
            program: "passbundle".into(),
            from: "0.1.1".into(),
            to: "0.1.2".into(),
            app: quit.id(),
        };
        fs::write(work.join(STATE), serde_json::to_vec(&state).unwrap()).unwrap();
        (base, install, work)
    }

    /// The script that said it started (`exec sleep`, so the same process).
    fn stop_apps(work: &Path) {
        if let Ok(pid) = fs::read_to_string(work.join(STARTED)) {
            let pid: u32 = pid.trim().parse().unwrap();
            let running = process::Running {
                pid,
                program: PathBuf::new(),
            };
            process::stop(&[running], Duration::from_secs(5));
        }
    }

    #[test]
    fn the_new_version_takes_the_old_ones_place_and_the_old_one_goes() {
        let (base, install, work) = setup("ok", true);
        assert_eq!(finish(&work), Outcome::Updated);
        assert_eq!(
            fs::read_to_string(install.join("version")).unwrap(),
            "0.1.2"
        );
        assert!(!work.join(OLD).exists() && !work.join(NEW).exists());
        assert!(work.join(DONE).exists() && !work.join(FAILED).exists());
        stop_apps(&work);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_new_version_that_doesnt_start_is_taken_out_and_the_old_one_opens_again() {
        let (base, install, work) = setup("broken", false);
        match finish(&work) {
            Outcome::Kept(reason) => assert!(reason.contains("didn't start"), "{reason}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            fs::read_to_string(install.join("version")).unwrap(),
            "0.1.1"
        );
        let failed: Failed = serde_json::from_slice(&fs::read(work.join(FAILED)).unwrap()).unwrap();
        assert_eq!(failed.version, "0.1.2");
        assert!(failed.reason.contains("went back to 0.1.1"));
        // The old version opened again, and said so.
        assert!(work.join(STARTED).exists());
        assert!(!work.join(OLD).exists() && !work.join(BROKEN).exists());
        stop_apps(&work);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn without_its_details_nothing_is_touched() {
        let (base, install, work) = setup("nostate", true);
        fs::remove_file(work.join(STATE)).unwrap();
        assert!(matches!(finish(&work), Outcome::Kept(_)));
        assert_eq!(
            fs::read_to_string(install.join("version")).unwrap(),
            "0.1.1"
        );
        fs::remove_dir_all(&base).unwrap();
    }
}
