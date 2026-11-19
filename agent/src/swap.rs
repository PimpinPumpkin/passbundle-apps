//! Putting a checked update in the installed folder's place: all of it or
//! none of it, and only while nothing runs from that folder. Windows can't
//! replace a program while it runs (the agent, one per signed-in person; the
//! app; a host a browser started), so an update that can't go in now waits
//! in the work folder, checked, and goes in at the next run that finds
//! nothing running from the folder, such as the one as the computer starts,
//! before anyone signs in.
//!
//! The work folder (`.PassBundle-agent-update` beside `PassBundle`, on the
//! same disk) is one the update task makes for itself, that only SYSTEM and
//! Administrators can change. In it: the download, the unpacked update
//! (`new`), a note that it's checked (`staged.json`), the old version while
//! it's moved out (`old`), and, while the folders change, the list of moves
//! (`swap.json`), so a run that stops partway (the computer turned off) is
//! undone by the next one before anything else.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::machine::Machine;
use crate::release::{self, AGENT};
use crate::unpack::TOP;
use crate::version::Version;

pub const DOWNLOAD: &str = "download";
pub const NEW: &str = "new";
pub const OLD: &str = "old";
pub const STAGED: &str = "staged.json";
pub const JOURNAL: &str = "swap.json";

/// Where updates of the folder `install` are put together: beside it.
pub fn work_dir(install: &Path) -> PathBuf {
    let name = install
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| TOP.into());
    install.with_file_name(format!(".{name}-agent-update"))
}

/// What the install script writes into the folder itself, which an update
/// leaves where it is: the native-messaging host's manifest, which names the
/// host by its path and our extensions.
pub fn kept() -> Vec<String> {
    vec![format!("{}.json", crate::brand::NATIVE_MESSAGING_HOST)]
}

/// The folders an update touches are only administrators' to change: the
/// installed folder, the one it's in, and the work folder if it's there.
/// Otherwise someone else could plant a program or a library that SYSTEM
/// then runs, or put one in place of what the task moves.
pub fn check_places(install: &Path, work: &Path, machine: &impl Machine) -> Result<(), String> {
    let parent = install
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or("the agent isn't in a folder of its own")?;
    for folder in [install, parent] {
        if let Some(why) = machine.not_admin_only(folder) {
            return Err(format!(
                "{} isn't a folder only administrators can change ({why}), so the update task leaves it alone",
                folder.display()
            ));
        }
    }
    if let Ok(meta) = fs::symlink_metadata(work) {
        let why = if meta.is_dir() {
            machine.not_admin_only(work)
        } else {
            Some("it isn't a folder".into())
        };
        if let Some(why) = why {
            return Err(format!(
                "something that isn't the update task's is where it puts updates together ({}: {why}), so it won't update until that's gone",
                work.display()
            ));
        }
    }
    Ok(())
}

/// Everything in the installed folder is only administrators' to change
/// too, and none of it is a link: a file others could change could be a
/// library SYSTEM loads.
pub fn check_contents(install: &Path, machine: &impl Machine) -> Result<(), String> {
    let refuse = |path: &Path, why: &str| {
        Err(format!(
            "{} isn't only administrators' to change ({why}), so the update task leaves the folder alone",
            path.display()
        ))
    };
    let mut folders = vec![install.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let entries = fs::read_dir(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("{}: {e}", folder.display()))?;
            let path = entry.path();
            let kind = entry
                .file_type()
                .map_err(|e| format!("{}: {e}", path.display()))?;
            if kind.is_symlink() {
                return refuse(&path, "it's a link");
            }
            if let Some(why) = machine.not_admin_only(&path) {
                return refuse(&path, &why);
            }
            if kind.is_dir() {
                folders.push(path);
            }
        }
    }
    Ok(())
}

/// Renames `from` to `to`, never over something already at `to`, and only
/// on the same disk (never a copy). Tries again for a while: on Windows,
/// something that's just looked at a file (an antivirus, the search
/// indexer) can hold it for a moment.
pub fn move_patiently(from: &Path, to: &Path) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match rename_new(from, to) {
            Ok(()) => return Ok(()),
            Err(e)
                if Instant::now() < deadline
                    && !matches!(
                        e.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::AlreadyExists
                    ) =>
            {
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(windows)]
fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let wide = |p: &Path| -> Vec<u16> { p.as_os_str().encode_wide().chain(Some(0)).collect() };
    // No flags: fails if `to` is there, and never copies across disks.
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileExW(
            wide(from).as_ptr(),
            wide(to).as_ptr(),
            0,
        )
    };
    if ok == 0 {
        let error = io::Error::last_os_error();
        // ERROR_FILE_EXISTS, ERROR_ALREADY_EXISTS
        if matches!(error.raw_os_error(), Some(80) | Some(183)) {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, error));
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(not(windows))]
fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
    if fs::symlink_metadata(to).is_ok() {
        return Err(io::ErrorKind::AlreadyExists.into());
    }
    fs::rename(from, to)
}

/// One rename of the swap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Move {
    pub from: PathBuf,
    pub to: PathBuf,
}

/// The renames that put `work/new/PassBundle` in `install`'s place: first
/// everything in `install` but what the install script wrote there (`kept`)
/// out to `work/old`, the agent's program last; then everything in the new
/// version in, the agent's program first. So the agent's program, which the
/// update task runs, is missing from its place only between two renames.
pub fn plan(install: &Path, work: &Path) -> io::Result<Vec<Move>> {
    let kept: Vec<String> = kept().iter().map(|k| k.to_lowercase()).collect();
    let names = |dir: &Path| -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in fs::read_dir(dir)? {
            let name = entry?.file_name().into_string().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "a name that isn't Unicode")
            })?;
            names.push(name);
        }
        // The agent's program last.
        names.sort_by_key(|n| (n.eq_ignore_ascii_case(AGENT), n.to_lowercase()));
        Ok(names)
    };
    let new_root = work.join(NEW).join(TOP);
    let old = work.join(OLD);
    let mut moves: Vec<Move> = names(install)?
        .into_iter()
        .filter(|n| !kept.contains(&n.to_lowercase()))
        .map(|n| Move {
            from: install.join(&n),
            to: old.join(&n),
        })
        .collect();
    let mut incoming = names(&new_root)?;
    if let Some(agent) = incoming.pop_if(|n| n.eq_ignore_ascii_case(AGENT)) {
        incoming.insert(0, agent);
    }
    moves.extend(incoming.into_iter().map(|n| Move {
        from: new_root.join(&n),
        to: install.join(&n),
    }));
    Ok(moves)
}

fn write_journal(work: &Path, moves: &[Move]) -> io::Result<()> {
    let partial = work.join(format!("{JOURNAL}.partial"));
    let mut file = fs::File::create(&partial)?;
    file.write_all(&serde_json::to_vec(moves).map_err(io::Error::other)?)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&partial, work.join(JOURNAL))
}

fn read_journal(work: &Path) -> Option<Vec<Move>> {
    let bytes = fs::read(work.join(JOURNAL)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Undoes `moves`, last first: each one that's done (its `to` there, its
/// `from` not) goes back. Whether every one could be.
fn undo(moves: &[Move], move_one: &mut impl FnMut(&Path, &Path) -> io::Result<()>) -> bool {
    let mut all = true;
    for m in moves.iter().rev() {
        let done = fs::symlink_metadata(&m.to).is_ok() && fs::symlink_metadata(&m.from).is_err();
        if done && move_one(&m.to, &m.from).is_err() {
            all = false;
        }
    }
    all
}

/// A swap that didn't finish (the computer turned off partway, say) is
/// undone, so the old version is whole again before anything else; the
/// checked update stays, to go in again.
pub fn recover(work: &Path) -> Result<(), String> {
    let journal = work.join(JOURNAL);
    if fs::symlink_metadata(&journal).is_err() {
        return Ok(());
    }
    let Some(moves) = read_journal(work) else {
        return Err(format!(
            "an update that stopped partway can't be undone: {} can't be read. Run the install script again.",
            journal.display()
        ));
    };
    if !undo(&moves, &mut move_patiently) {
        return Err(
            "an update that stopped partway couldn't be undone yet; the task tries again next time"
                .into(),
        );
    }
    fs::remove_file(&journal).map_err(|e| format!("{}: {e}", journal.display()))
}

/// Removes a folder or file and everything in it, if it's there; whether
/// it's gone.
pub fn remove(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Err(_) => true,
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path).is_ok(),
        Ok(_) => fs::remove_file(path).is_ok(),
    }
}

/// Takes away a staged update, and the work folder if that leaves it empty.
pub fn discard(work: &Path) {
    remove(&work.join(STAGED));
    remove(&work.join(NEW));
    remove(&work.join(DOWNLOAD));
    let _ = fs::remove_dir(work);
}

/// What an earlier run left: a download, the old version, an update
/// unpacked but not checked. The work folder goes when nothing is left.
pub fn clean(work: &Path) {
    remove(&work.join(DOWNLOAD));
    remove(&work.join(format!("{JOURNAL}.partial")));
    remove(&work.join(OLD));
    if fs::symlink_metadata(work.join(STAGED)).is_err() {
        remove(&work.join(NEW));
    }
    // Only when it's empty.
    let _ = fs::remove_dir(work);
}

#[derive(Serialize, Deserialize)]
struct Staged {
    version: String,
}

/// Notes that the update in `work/new` is checked, at `version`.
pub fn mark_staged(work: &Path, version: Version) -> io::Result<()> {
    let staged = Staged {
        version: version.to_string(),
    };
    fs::write(
        work.join(STAGED),
        serde_json::to_vec(&staged).map_err(io::Error::other)?,
    )
}

/// The update waiting in `work`, if there's one, it's newer than
/// `installed`, and it still checks out; anything else there is taken away.
/// An install script run since it was staged may have put in the same
/// version or a newer one.
pub fn staged(work: &Path, installed: Version, platform: &str) -> Option<Version> {
    let version = fs::read(work.join(STAGED))
        .ok()
        .and_then(|b| serde_json::from_slice::<Staged>(&b).ok())
        .and_then(|s| Version::parse(&s.version));
    let waiting = version.filter(|&v| {
        v > installed && release::check(&work.join(NEW).join(TOP), v, platform, &kept()).is_ok()
    });
    if waiting.is_none() {
        discard(work);
    }
    waiting
}

/// Puts the staged update in `install`'s place, if nothing runs from it:
/// whether it went in. Every rename is listed in the work folder first; if
/// one fails, or the new agent doesn't start, they're all undone, last
/// first, and the old version is as it was. Afterwards the old version is
/// removed.
pub fn put_in_place(install: &Path, work: &Path, machine: &impl Machine) -> Result<bool, String> {
    put_in_place_with(install, work, machine, &mut move_patiently)
}

/// [`put_in_place`], with the renames done by `move_one` (tests make one
/// fail partway).
pub fn put_in_place_with(
    install: &Path,
    work: &Path,
    machine: &impl Machine,
    move_one: &mut impl FnMut(&Path, &Path) -> io::Result<()>,
) -> Result<bool, String> {
    let running = machine.running_in(install)?;
    if !running.is_empty() {
        return Ok(false);
    }
    // What's left of an earlier old version (a program someone ran from it
    // as it was moved out, still running) keeps this one waiting.
    let old = work.join(OLD);
    if !remove(&old) {
        return Ok(false);
    }
    let moves = plan(install, work).map_err(|e| format!("planning the update: {e}"))?;
    fs::create_dir(&old).map_err(|e| format!("{}: {e}", old.display()))?;
    write_journal(work, &moves).map_err(|e| format!("{}: {e}", work.join(JOURNAL).display()))?;

    let failed = moves
        .iter()
        .find_map(|m| move_one(&m.from, &m.to).err().map(|e| (m, e)))
        .map(|(m, e)| format!("{} couldn't be moved ({e})", m.from.display()))
        .or_else(|| {
            (!machine.starts(&install.join(AGENT)))
                .then(|| "the new version's agent didn't start".to_owned())
        });
    if let Some(why) = failed {
        if !undo(&moves, move_one) {
            return Err(format!(
                "{why}, and the old version couldn't be put back yet; the task tries again next time"
            ));
        }
        let _ = fs::remove_file(work.join(JOURNAL));
        remove(&old);
        return Err(format!("{why}, so the old version is still in place"));
    }
    // Done: the journal goes first, then everything else.
    fs::remove_file(work.join(JOURNAL))
        .map_err(|e| format!("{}: {e}", work.join(JOURNAL).display()))?;
    discard(work);
    // The update task's own program may be among the old version's, still
    // running: what can't go now goes next time.
    clean(work);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    pub fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pb-agent-swap-{name}-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    struct Quiet {
        starts: bool,
    }

    impl Machine for Quiet {
        fn not_admin_only(&self, _: &Path) -> Option<String> {
            None
        }
        fn make_admin_dir(&self, dir: &Path) -> io::Result<()> {
            fs::create_dir(dir)
        }
        fn running_in(&self, _: &Path) -> Result<Vec<PathBuf>, String> {
            Ok(Vec::new())
        }
        fn starts(&self, _: &Path) -> bool {
            self.starts
        }
    }

    /// Every file under `dir` and what it holds.
    fn contents(dir: &Path) -> Vec<(String, String)> {
        release::listing(dir)
            .unwrap()
            .into_iter()
            .filter(|p| dir.join(p).is_file())
            .map(|p| {
                let body = fs::read_to_string(dir.join(&p)).unwrap();
                (p, body)
            })
            .collect()
    }

    /// An installed folder at 0.1.4 with the install script's host manifest,
    /// and 0.1.5 staged beside it (one file more, one fewer).
    fn setup(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let base = temp(name);
        let install = base.join("PassBundle");
        let work = work_dir(&install);
        for (dir, version, extra) in [
            (install.clone(), "0.1.4", "old-only.dll"),
            (work.join(NEW).join(TOP), "0.1.5", "new-only.dll"),
        ] {
            for name in [
                AGENT,
                "passbundle.exe",
                extra,
                "data/app.so",
                release::RELEASE,
            ] {
                let path = dir.join(name);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, format!("{name} {version}")).unwrap();
            }
        }
        fs::write(install.join(&kept()[0]), "host manifest").unwrap();
        (base, install, work)
    }

    #[test]
    fn the_agents_program_goes_out_last_and_comes_in_first() {
        let (base, install, work) = setup("plan");
        let moves = plan(&install, &work).unwrap();
        let out: Vec<_> = moves
            .iter()
            .filter(|m| m.from.starts_with(&install))
            .collect();
        let into: Vec<_> = moves
            .iter()
            .filter(|m| m.to.starts_with(&install))
            .collect();
        assert_eq!(out.len(), 5);
        assert_eq!(into.len(), 5);
        assert!(out.last().unwrap().from.ends_with(AGENT));
        assert!(into[0].to.ends_with(AGENT));
        // What the install script wrote stays where it is.
        assert!(moves.iter().all(|m| !m.from.ends_with(&kept()[0])));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn the_new_version_takes_the_old_ones_place_and_the_rest_goes() {
        let (base, install, work) = setup("ok");
        let new = contents(&work.join(NEW).join(TOP));
        mark_staged(&work, Version::new(0, 1, 5)).unwrap();
        assert_eq!(
            put_in_place(&install, &work, &Quiet { starts: true }),
            Ok(true)
        );
        let mut expected = new;
        expected.push((kept()[0].clone(), "host manifest".into()));
        expected.sort();
        assert_eq!(contents(&install), expected);
        assert!(!work.exists());
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_swap_that_fails_partway_leaves_the_old_version_whole() {
        let (base, install, work) = setup("fails");
        let old = contents(&install);
        let new = contents(&work.join(NEW).join(TOP));
        let count = plan(&install, &work).unwrap().len();
        // Each rename in turn fails, the first to the last.
        for fail_at in 0..count {
            let n = Cell::new(0);
            let mut move_one = |from: &Path, to: &Path| {
                n.set(n.get() + 1);
                if n.get() == fail_at + 1 {
                    return Err(io::Error::other("held open"));
                }
                rename_new(from, to)
            };
            let answer = put_in_place_with(&install, &work, &Quiet { starts: true }, &mut move_one);
            assert!(
                answer.as_ref().is_err_and(|e| e.contains("still in place")),
                "{fail_at}: {answer:?}"
            );
            assert_eq!(contents(&install), old, "{fail_at}");
            // The update is still there to try again, and nothing else.
            assert_eq!(contents(&work.join(NEW).join(TOP)), new, "{fail_at}");
            assert!(!work.join(JOURNAL).exists() && !work.join(OLD).exists());
        }
        // A new agent that doesn't start is taken out again.
        let answer = put_in_place(&install, &work, &Quiet { starts: false });
        assert!(answer.unwrap_err().contains("didn't start"));
        assert_eq!(contents(&install), old);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_swap_cut_off_partway_is_undone_by_the_next_run() {
        let (base, install, work) = setup("cut");
        let old = contents(&install);
        let moves = plan(&install, &work).unwrap();
        for stop_at in [1, 4, moves.len() - 1] {
            fs::create_dir(work.join(OLD)).unwrap();
            write_journal(&work, &moves).unwrap();
            // The computer turns off after `stop_at` renames.
            for m in &moves[..stop_at] {
                rename_new(&m.from, &m.to).unwrap();
            }
            assert_ne!(contents(&install), old);
            recover(&work).unwrap();
            assert_eq!(contents(&install), old, "{stop_at}");
            assert!(!work.join(JOURNAL).exists());
            remove(&work.join(OLD));
        }
        // Nothing to undo: nothing changes.
        recover(&work).unwrap();
        assert_eq!(contents(&install), old);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn nothing_moves_while_a_program_runs_from_the_folder() {
        struct Busy;
        impl Machine for Busy {
            fn not_admin_only(&self, _: &Path) -> Option<String> {
                None
            }
            fn make_admin_dir(&self, dir: &Path) -> io::Result<()> {
                fs::create_dir(dir)
            }
            fn running_in(&self, dir: &Path) -> Result<Vec<PathBuf>, String> {
                Ok(vec![dir.join(AGENT)])
            }
            fn starts(&self, _: &Path) -> bool {
                true
            }
        }
        let (base, install, work) = setup("busy");
        let old = contents(&install);
        assert_eq!(put_in_place(&install, &work, &Busy), Ok(false));
        assert_eq!(contents(&install), old);
        assert!(work.join(NEW).join(TOP).join(AGENT).exists());
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn moves_never_replace_what_is_there() {
        let base = temp("noreplace");
        fs::write(base.join("a"), b"a").unwrap();
        fs::write(base.join("b"), b"b").unwrap();
        let e = rename_new(&base.join("a"), &base.join("b")).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(base.join("b")).unwrap(), b"b");
        fs::remove_dir_all(&base).unwrap();
    }
}
