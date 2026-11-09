//! Updates from PassBundle's GitHub releases on Windows and Linux (README,
//! "Windows and Linux test releases"), as the Android and Mac apps update
//! themselves: the newest release for this platform, when it's newer than
//! this copy, offered in Settings (app/lib/updates.dart). On a click it's
//! downloaded over HTTPS, only from this repository's releases, and checked:
//! its manifest's Ed25519 signature with a key compiled in here
//! ([`PINNED_KEYS`]; the meta repo's DESIGN.md, "Desktop updates"), then
//! everything the manifest says ([`manifest::read`]), then the file's size
//! and SHA-256 as it downloads, then what it unpacks to. Then the app quits
//! and `passbundle-update` puts it in place, opens it, and puts the old
//! version back if it doesn't start ([`swap`]).
//!
//! It contacts api.github.com, and github.com for the files (which GitHub
//! hands to objects.githubusercontent.com or
//! release-assets.githubusercontent.com), and no other host, only to check
//! or download: when the person clicks "Check for updates" or "Update", and
//! at most once a day as the app opens, if the person leaves that on and the
//! copy is a release's. Where it can't replace itself (a folder only an
//! administrator can change, as a system-wide install or a package's), it
//! only says there's a new version.

pub mod access;
pub mod bundle;
pub mod fetch;
pub mod manifest;
pub mod process;
pub mod swap;
pub mod unpack;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use bundle::Installed;
use fetch::{FetchError, Hosts};
use manifest::{Expect, Refused, Release};

/// The public keys whose signature a release's manifest must have: the
/// test release key, whose private half is the repository's
/// `PASSBUNDLE_DESKTOP_UPDATE_KEY` secret (and the owner's copy). More than
/// one only while a key is being rotated in (DESIGN.md, "Desktop updates").
/// The release workflows check their key is this one.
pub const PINNED_KEYS: &[[u8; 32]] = &[hex32(
    "5db5f4827dab67b3d7ef0726a202921728e5159ee70199d6e4ea059a1e72b0c5",
)];

const REPO: &str = "PimpinPumpkin/passbundle-apps";

/// The most the list of releases may be.
const MAX_LIST_BYTES: u64 = 4 << 20;

const DAY_SECONDS: f64 = 24.0 * 60.0 * 60.0;

const fn hex32(text: &str) -> [u8; 32] {
    const fn digit(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => panic!("not lowercase hex"),
        }
    }
    let bytes = text.as_bytes();
    assert!(bytes.len() == 64);
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = digit(bytes[2 * i]) << 4 | digit(bytes[2 * i + 1]);
        i += 1;
    }
    out
}

/// `windows` or `linux`, as the release tags name them.
pub fn os() -> Option<&'static str> {
    if cfg!(windows) {
        Some("windows")
    } else if cfg!(target_os = "linux") {
        Some("linux")
    } else {
        None
    }
}

/// This build's platform, as the releases' files name it.
pub fn platform() -> Option<&'static str> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some("windows-x64")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("linux-x64")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("linux-arm64")
    } else {
        None
    }
}

/// Where releases are: GitHub's list of this repository's releases, and
/// its releases' files.
#[derive(Debug, Clone)]
pub struct Source {
    pub list: String,
    pub downloads: String,
    pub api_hosts: Hosts,
    pub download_hosts: Hosts,
}

impl Source {
    pub fn github() -> Source {
        Source {
            list: format!("https://api.github.com/repos/{REPO}/releases?per_page=100"),
            downloads: format!("https://github.com/{REPO}/releases/download/"),
            api_hosts: fetch::API,
            download_hosts: fetch::DOWNLOADS,
        }
    }

    /// A stand-in for GitHub on this computer, at `base`
    /// (`http://127.0.0.1:<port>`), with GitHub's paths.
    pub fn test(base: &str) -> Source {
        let base = base.trim_end_matches('/');
        Source {
            list: format!("{base}/repos/{REPO}/releases?per_page=100"),
            downloads: format!("{base}/{REPO}/releases/download/"),
            api_hosts: fetch::TEST,
            download_hosts: fetch::TEST,
        }
    }

    /// Exactly the file `name` on the release tagged `tag`.
    fn asset(&self, tag: &str, name: &str) -> Option<String> {
        (manifest::is_plain(tag) && manifest::is_plain(name))
            .then(|| format!("{}{tag}/{name}", self.downloads))
    }
}

/// Everything the updater goes by.
#[derive(Debug, Clone)]
pub struct Context {
    pub source: Source,
    pub keys: Vec<[u8; 32]>,
    pub installed: Installed,
    /// Where PassBundle keeps the vault, and the updater its settings.
    pub data_dir: PathBuf,
    pub os: &'static str,
    pub platform: &'static str,
    /// Running as an administrator doesn't stop it, and a folder
    /// Administrators own counts as this person's: only in builds with test
    /// hooks, on CI's Windows machines, which run everything elevated.
    pub trust_admin: bool,
}

impl Context {
    /// The running app's, on Windows and Linux; None elsewhere. Builds with
    /// test hooks (never a release) take PB_UPDATE_SOURCE (a stand-in for
    /// GitHub on this computer), PB_UPDATE_KEY (the public key, in hex), and
    /// PB_UPDATE_ELEVATED_OK=1 (`trust_admin`) from their environment, for
    /// tools/desktop-update-test.sh.
    pub fn for_this_app() -> Option<Context> {
        let (os, platform) = (os()?, platform()?);
        let program = std::env::current_exe().ok()?;
        #[allow(unused_mut)]
        let mut source = Source::github();
        #[allow(unused_mut)]
        let mut keys = PINNED_KEYS.to_vec();
        #[cfg(feature = "test-hooks")]
        let trust_admin = std::env::var("PB_UPDATE_ELEVATED_OK").as_deref() == Ok("1");
        #[cfg(not(feature = "test-hooks"))]
        let trust_admin = false;
        #[cfg(feature = "test-hooks")]
        {
            if let Some(base) = std::env::var("PB_UPDATE_SOURCE")
                .ok()
                .filter(|s| !s.is_empty())
            {
                source = Source::test(&base);
            }
            if let Some(key) = std::env::var("PB_UPDATE_KEY")
                .ok()
                .and_then(|k| manifest::sha256_of(k.trim()))
            {
                keys = vec![key];
            }
        }
        Some(Context {
            source,
            keys,
            installed: Installed::at(&program),
            data_dir: crate::store::Store::default_dir(),
            os,
            platform,
            trust_admin,
        })
    }

    fn work_dir(&self) -> PathBuf {
        bundle::work_dir(&self.installed.dir)
    }

    fn running_version(&self) -> Option<&str> {
        self.installed
            .release_for(self.platform)
            .map(|r| r.version.as_str())
    }
}

/// An update the last check found.
#[derive(Debug, Clone)]
struct Offer {
    release: Release,
    url: String,
}

static OFFER: Mutex<Option<Offer>> = Mutex::new(None);
static BUSY: AtomicBool = AtomicBool::new(false);
/// How far a download is, 0 to 100; -1 when there's none.
static PROGRESS: AtomicI32 = AtomicI32::new(-1);
/// What the last update that didn't happen said, once heard.
static FAILED: Mutex<Option<swap::Failed>> = Mutex::new(None);

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The updater's own settings, next to the vault.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Prefs {
    #[serde(default = "yes")]
    auto: bool,
    #[serde(default, rename = "lastCheck")]
    last_check: f64,
}

fn yes() -> bool {
    true
}

fn prefs_path(ctx: &Context) -> PathBuf {
    ctx.data_dir.join("updates.json")
}

fn read_prefs(ctx: &Context) -> Prefs {
    fs::read(prefs_path(ctx))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Prefs {
            auto: true,
            last_check: 0.0,
        })
}

fn write_prefs(ctx: &Context, prefs: &Prefs) {
    let _ = fs::create_dir_all(&ctx.data_dir);
    let path = prefs_path(ctx);
    let partial = path.with_extension("json.partial");
    if let Ok(text) = serde_json::to_string(prefs) {
        if fs::write(&partial, text).is_ok() {
            let _ = fs::rename(&partial, &path);
        }
    }
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn fail(error: impl Into<String>) -> Value {
    json!({ "ok": false, "error": error.into() })
}

/// Why this copy can't update itself (the person is told there's a new
/// version, and how to get it), or None.
fn install_note(ctx: &Context) -> Option<String> {
    bundle::why_not(&ctx.installed, ctx.platform, &ctx.data_dir, ctx.trust_admin)
}

/// The work folder, if it's one this updater made (`access::is_private`):
/// anything else there is no one's to trust, and is never read or written.
fn own_work_dir(ctx: &Context) -> Option<PathBuf> {
    let work = ctx.work_dir();
    access::is_private(&work).then_some(work)
}

/// The last update that didn't happen, from its work folder or as heard.
fn failed_note(ctx: &Context) -> Option<swap::Failed> {
    let mut kept = lock(&FAILED);
    if kept.is_none() {
        *kept = own_work_dir(ctx)
            .and_then(|work| access::read_file(&work.join(swap::FAILED), 64 << 10))
            .and_then(|b| serde_json::from_slice(&b).ok());
    }
    kept.clone()
}

/// This copy's version (a release's, or none), whether it checks once a
/// day, that updating restarts it, why it can't update itself if it can't
/// (`installNote`), and the last update that didn't happen (`failed`).
pub fn status(ctx: &Context) -> Value {
    let mut answer = json!({
        "version": ctx.running_version(),
        "auto": read_prefs(ctx).auto,
        "restarts": true,
        "store": null,
    });
    if let Some(note) = install_note(ctx) {
        answer["installNote"] = json!(note);
    }
    if let Some(failed) = failed_note(ctx) {
        answer["failed"] = json!({ "version": failed.version, "reason": failed.reason });
    }
    answer
}

pub fn set_auto(ctx: &Context, on: bool) {
    let mut prefs = read_prefs(ctx);
    prefs.auto = on;
    write_prefs(ctx, &prefs);
}

/// How far the download is: `{percent}`, null when there's none.
pub fn progress() -> Value {
    let percent = PROGRESS.load(Ordering::SeqCst);
    json!({ "percent": (percent >= 0).then_some(percent) })
}

/// Looks for a newer release (slow): `{ok, available, version, notes}`, or
/// `{ok: false, error}`. `quiet` is the check as the app opens, at most
/// once a day, and only for a release's copy with the daily check on:
/// otherwise `{ok, available: false, skipped: true}`.
pub fn check(ctx: &Context, quiet: bool) -> Value {
    let skipped = json!({ "ok": true, "available": false, "skipped": true });
    let mut prefs = read_prefs(ctx);
    let time = now();
    if quiet {
        if ctx.running_version().is_none() || !prefs.auto {
            return skipped;
        }
        if time >= prefs.last_check && time - prefs.last_check < DAY_SECONDS {
            return skipped;
        }
    }
    prefs.last_check = time;
    write_prefs(ctx, &prefs);
    *lock(&OFFER) = None;
    match newer(ctx) {
        Ok(None) => json!({ "ok": true, "available": false }),
        Ok(Some(offer)) => {
            let answer = json!({
                "ok": true,
                "available": true,
                "version": offer.release.version.to_string(),
                "notes": manifest::changes(&offer.release.notes, 4000),
            });
            *lock(&OFFER) = Some(offer);
            answer
        }
        Err(error) => fail(error),
    }
}

fn network(e: FetchError) -> String {
    match e {
        FetchError::Refused(_) => {
            "PassBundle's releases sent it somewhere else, so it didn't check.".into()
        }
        _ => "Couldn't check for updates. Check your connection and try again.".into(),
    }
}

/// How many of the newest releases a check tries, newest first.
const CANDIDATES: usize = 10;

/// Why a release wasn't taken: something about it (tried no further), or
/// the network (no use trying others now).
enum Skip {
    Refused(String),
    Network(String),
}

/// The newest release for this platform that's newer than this copy and
/// whose manifest is signed and says what this copy needs. Releases are
/// tried newest first, at most [`CANDIDATES`]: one that doesn't check out
/// (a release someone with access to GitHub made up, say) can't keep the
/// real ones behind it from being offered.
fn newer(ctx: &Context) -> Result<Option<Offer>, String> {
    let list = fetch::get_bytes(&ctx.source.list, &ctx.source.api_hosts, MAX_LIST_BYTES, 60)
        .map_err(network)?;
    let list: Vec<Value> = serde_json::from_slice(&list)
        .map_err(|_| "GitHub's answer about PassBundle's releases can't be read.".to_owned())?;
    let shipped: Vec<&Value> = list
        .iter()
        .filter(|r| r["draft"] != true && r["prerelease"] != true)
        .collect();
    let running_build = ctx.installed.build(ctx.platform);
    let mut candidates: Vec<(&str, manifest::Version, &Value)> = shipped
        .iter()
        .filter_map(|r| {
            let tag = r["tag_name"].as_str()?;
            let version = manifest::version_of(tag, ctx.os)?;
            (version.build > running_build).then_some((tag, version, *r))
        })
        .collect();
    candidates.sort_by_key(|c| std::cmp::Reverse(c.1.build));
    let mut refused = None;
    for (tag, version, release) in candidates.into_iter().take(CANDIDATES) {
        match offer_for(ctx, release, tag, version, running_build) {
            Ok(Some(offer)) => return Ok(Some(offer)),
            Ok(None) => {}
            Err(Skip::Refused(why)) => {
                refused.get_or_insert(why);
            }
            Err(Skip::Network(why)) => return Err(why),
        }
    }
    refused.map_or(Ok(None), Err)
}

/// The release tagged `tag` (the list's `release`), if its manifest checks
/// out for this copy.
fn offer_for(
    ctx: &Context,
    release: &Value,
    tag: &str,
    version: manifest::Version,
    running_build: u32,
) -> Result<Option<Offer>, Skip> {
    let assets = release["assets"].as_array().cloned().unwrap_or_default();
    // Exactly these files, on this release.
    let asset = |name: &str| -> Option<(&Value, String)> {
        let url = ctx.source.asset(tag, name)?;
        assets
            .iter()
            .find(|a| a["name"] == name && a["browser_download_url"] == url.as_str())
            .map(|a| (a, url))
    };
    let (Some((_, manifest_url)), Some((_, signature_url))) =
        (asset("manifest.json"), asset("manifest.json.sig"))
    else {
        return Err(Skip::Refused(format!(
            "PassBundle {version} has no signed manifest, so it can't be installed from here."
        )));
    };
    let get = |url: &str, max| {
        fetch::get_bytes(url, &ctx.source.download_hosts, max, 60).map_err(|e| match e {
            // Nothing there, or somewhere it may not go: about this release.
            FetchError::NotFound | FetchError::Refused(_) | FetchError::TooLong => {
                Skip::Refused(format!(
                    "PassBundle {version}'s manifest can't be read, so it can't be installed from here."
                ))
            }
            e => Skip::Network(network(e)),
        })
    };
    let manifest_bytes = get(&manifest_url, manifest::MAX_MANIFEST_BYTES)?;
    let signature = get(&signature_url, manifest::MAX_SIGNATURE_BYTES)?;
    let signature = String::from_utf8(signature).unwrap_or_default();
    let expect = Expect {
        tag,
        os: ctx.os,
        platform: ctx.platform,
        running_build,
    };
    let release = match manifest::read(&manifest_bytes, &signature, &ctx.keys, &expect) {
        Ok(release) => release,
        Err(Refused::NotNewer { .. }) => return Ok(None),
        Err(refused) => {
            return Err(Skip::Refused(format!(
                "PassBundle {version} can't be installed from here: {refused}."
            )))
        }
    };
    let Some((file, url)) = asset(&release.file.name) else {
        return Err(Skip::Refused(format!(
            "PassBundle {version}'s files aren't on PassBundle's releases, so it can't be installed from here."
        )));
    };
    // What GitHub says of the file, where it says it, agrees.
    let size = file["size"].as_u64().unwrap_or(0);
    let digest = file["digest"]
        .as_str()
        .and_then(|d| d.strip_prefix("sha256:"))
        .and_then(manifest::sha256_of);
    if (size > 0 && size != release.file.size) || digest.is_some_and(|d| d != release.file.sha256) {
        return Err(Skip::Refused(format!(
            "PassBundle {version}'s file isn't the one its manifest names, so it can't be installed from here."
        )));
    }
    Ok(Some(Offer { release, url }))
}

/// Downloads the update the last check found, checks it, unpacks it next
/// to this copy, and starts `passbundle-update` to put it in place once the
/// app quits (slow): `{ok, restart}`, after which the app quits; or `{ok:
/// false, error}`.
pub fn install(ctx: &Context) -> Value {
    if BUSY.swap(true, Ordering::SeqCst) {
        return fail("The update is already downloading.");
    }
    let answer = match lock(&OFFER).clone() {
        None => fail("Check for updates first."),
        Some(offer) => match prepare(ctx, &offer) {
            Ok(()) => json!({ "ok": true, "restart": true }),
            Err(error) => fail(error),
        },
    };
    PROGRESS.store(-1, Ordering::SeqCst);
    BUSY.store(false, Ordering::SeqCst);
    answer
}

fn prepare(ctx: &Context, offer: &Offer) -> Result<(), String> {
    if let Some(note) = install_note(ctx) {
        return Err(note);
    }
    let install = &ctx.installed.dir;
    let me = std::process::id();
    let others = process::under(install)
        .into_iter()
        .filter(|p| p.pid != me && !swap::is_agent(&p.program))
        .count();
    if others > 0 {
        return Err("PassBundle is open in another window. Close it, then update.".into());
    }
    let work = ctx.work_dir();
    if fs::symlink_metadata(&work).is_ok() {
        // Only ever a folder this updater made, left behind: never anything
        // else (it isn't PassBundle's to remove, and could be a trap).
        if !access::is_private(&work) {
            return Err(format!(
                "Something that isn't PassBundle's is where it puts an update together ({}), so it won't update until that's gone.",
                work.display()
            ));
        }
        if helper_at_work(&work) {
            return Err("An update is already being put in place.".into());
        }
        swap::remove(&work, Duration::from_secs(10));
    }
    let release = &offer.release;
    let size = release.file.size;
    let limits = unpack::limits(size);
    if free_space(install.parent().unwrap_or(install))
        .is_some_and(|free| free < size + limits.bytes.min(size * 4) + (64 << 20))
    {
        return Err("There isn't enough space on this computer for the update.".into());
    }
    access::make_private_dir(&work).map_err(|_| "Couldn't make room for the update.".to_owned())?;
    // Whatever goes wrong from here, nothing of it stays.
    let mut cleanup = Cleanup(Some(work.clone()));

    // The download, hashed as it's written, never more than the manifest says.
    let archive = work.join("download");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&archive)
        .map_err(|_| "Couldn't keep the download.".to_owned())?;
    let mut hasher = Sha256::new();
    let mut read: u64 = 0;
    PROGRESS.store(0, Ordering::SeqCst);
    let fetched = fetch::get(
        &offer.url,
        &ctx.source.download_hosts,
        size,
        15 * 60,
        |_| {},
        |chunk| {
            hasher.update(chunk);
            file.write_all(chunk)?;
            read += chunk.len() as u64;
            PROGRESS.store((99 * read / size.max(1)) as i32, Ordering::SeqCst);
            Ok(())
        },
    );
    match fetched {
        Ok(()) => {}
        Err(FetchError::TooLong) => {
            return Err(
                "The download is larger than its release says, so it wasn't installed.".into(),
            )
        }
        Err(FetchError::TooSlow) => {
            return Err("The download took too long, so it stopped. Try again later.".into())
        }
        Err(FetchError::NotFound) => {
            return Err("Couldn't download the update: its file isn't there.".into())
        }
        Err(_) => {
            return Err("Couldn't download the update. Check your connection and try again.".into())
        }
    }
    file.sync_all()
        .map_err(|_| "Couldn't keep the download.".to_owned())?;
    drop(file);
    if read != size || hasher.finalize().as_slice() != release.file.sha256 {
        return Err("The download didn't match the release, so it wasn't installed.".into());
    }
    PROGRESS.store(100, Ordering::SeqCst);

    // Unpacked, and checked again.
    let new = work.join(swap::NEW);
    fs::create_dir(&new).map_err(|_| "Couldn't make room for the update.".to_owned())?;
    let unpacked = if release.file.name.ends_with(".zip") {
        unpack::zip(&archive, &new, limits)
    } else {
        unpack::tar_gz(&archive, &new, limits)
    };
    let _ = fs::remove_file(&archive);
    unpacked.map_err(|e| match e {
        unpack::UnpackError::Write(_) => {
            "There isn't enough space on this computer for the update.".to_owned()
        }
        e => format!("The download can't be installed: {e}."),
    })?;
    let staged = new.join(unpack::TOP);
    let version = release.version.to_string();
    if let Some(problem) = bundle::check_staged(&staged, &version, ctx.platform) {
        return Err(problem);
    }

    // The updater that puts it in place: this copy's own, run from outside
    // the folder it replaces.
    let helper = work.join(swap::helper_name());
    fs::copy(install.join(swap::helper_name()), &helper).map_err(|_| {
        format!("This copy of PassBundle is missing its updater, so it can't update itself. Download PassBundle {version} from GitHub instead.")
    })?;
    let state = swap::State {
        install: install.clone(),
        program: bundle::program_name().to_owned(),
        from: ctx.running_version().unwrap_or("0.0.0").to_owned(),
        to: version,
        app: me,
    };
    access::write_file(
        &work.join(swap::STATE),
        &serde_json::to_vec(&state).expect("a state is JSON"),
    )
    .map_err(|_| "Couldn't make room for the update.".to_owned())?;
    process::let_next_come_forward();
    let parent = install.parent().unwrap_or(install);
    process::spawn_detached(&helper, &["finish".as_ref(), work.as_os_str()], parent)
        .map_err(|_| "Couldn't start putting the update in place.".to_owned())?;
    cleanup.0 = None;
    Ok(())
}

/// Removes a work folder when dropped, unless it's been kept.
struct Cleanup(Option<PathBuf>);

impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some(dir) = self.0.take() {
            swap::remove(&dir, Duration::from_secs(10));
        }
    }
}

/// Whether a `passbundle-update` is still at work in `work`.
fn helper_at_work(work: &Path) -> bool {
    access::read_file(&work.join(swap::HELPER), 64)
        .and_then(|pid| String::from_utf8_lossy(&pid).trim().parse::<u32>().ok())
        .is_some_and(process::alive)
        && !work.join(swap::DONE).exists()
}

/// The space free for this user on the disk `dir` is on, where the OS says.
fn free_space(dir: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
            return None;
        }
        #[allow(clippy::unnecessary_cast)]
        Some(stat.f_bavail as u64 * stat.f_frsize as u64)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
        let mut free: u64 = 0;
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some(free)
    }
}

/// The app has drawn its first frame: tells a `passbundle-update` waiting
/// for it that it started, and, once that has finished, removes the work
/// folder (the old version, and the updater's copy). `{updated}` when this
/// is a new version just put in place; `{failed: {version, reason}}` when an
/// update didn't happen.
pub fn started(ctx: &Context) -> Value {
    // Nothing going on, or something there that isn't this updater's.
    let Some(work) = own_work_dir(ctx) else {
        return json!({});
    };
    let state: Option<swap::State> = access::read_file(&work.join(swap::STATE), 64 << 10)
        .and_then(|b| serde_json::from_slice(&b).ok());
    let failed = failed_note(ctx);
    let _ = access::write_file(
        &work.join(swap::STARTED),
        std::process::id().to_string().as_bytes(),
    );
    std::thread::spawn(move || {
        // The updater sees this, finishes, and says so; or isn't there.
        for _ in 0..240 {
            if work.join(swap::DONE).exists() || !helper_at_work(&work) {
                // On Windows, until its program has quite ended.
                std::thread::sleep(Duration::from_millis(500));
                swap::remove(&work, Duration::from_secs(30));
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
    let mut answer = json!({});
    if let Some(failed) = failed {
        answer["failed"] = json!({ "version": failed.version, "reason": failed.reason });
    } else if let Some(state) = state {
        if ctx.running_version() == Some(state.to.as_str()) {
            answer["updated"] = json!(state.to);
        }
    }
    // Builds with test hooks (never a release) note it next to the vault,
    // for tools/desktop-update-test.sh to read.
    #[cfg(feature = "test-hooks")]
    if let Ok(mut log) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(ctx.data_dir.join("update-test.log"))
    {
        let _ = writeln!(log, "{answer}");
    }
    answer
}

#[cfg(test)]
mod tests;
