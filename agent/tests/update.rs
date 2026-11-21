//! The update task's whole run (`update::run`), against a stand-in for the
//! update site on this computer, with releases signed by a throwaway key:
//! the agent, its host, and the app replaced together or not at all, each
//! check over all three, and an update that waits while something runs from
//! the folder. A stand-in for Windows (`Fake`) says who can change what and
//! what's running; `machine.rs` tests Windows' own answers on Windows.

use std::{
    collections::HashMap,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use passbundle_agent::{
    machine::Machine,
    release, swap,
    update::{self, Outcome, UpdateError},
    version::Version,
};
use sha2::{Digest, Sha256};

const TARGET: &str = "windows-x86_64";
const PLATFORM: &str = "windows-x64";
const HOST_MANIFEST: &str = "com.passbundle.agent.json";
/// What a Windows release's folder holds, besides release.json.
const FILES: &[&str] = &[
    "passbundle-agent.exe",
    "passbundle-agent-host.exe",
    "passbundle.exe",
    "passbundle_core.dll",
    "passbundle-update.exe",
    "flutter_windows.dll",
    "data/icudtl.dat",
    "data/flutter_assets/AssetManifest.bin",
];

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pb-agent-update-{name}-{}-{}",
        std::process::id(),
        rand::random::<u32>()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Windows as the update task sees it: what others can change, what's
/// running, and whether a new agent starts.
#[derive(Default)]
struct Fake {
    shared: Mutex<Vec<PathBuf>>,
    running: Mutex<Vec<PathBuf>>,
    agent_broken: AtomicBool,
}

impl Machine for Fake {
    fn not_admin_only(&self, path: &Path) -> Option<String> {
        match fs::symlink_metadata(path) {
            Err(_) => Some("it can't be read".into()),
            Ok(meta) if meta.file_type().is_symlink() => Some("it's a link".into()),
            Ok(_) => self
                .shared
                .lock()
                .unwrap()
                .contains(&path.to_path_buf())
                .then(|| "Users can change it".into()),
        }
    }

    fn make_admin_dir(&self, dir: &Path) -> io::Result<()> {
        fs::create_dir(dir)
    }

    fn running_in(&self, dir: &Path) -> Result<Vec<PathBuf>, String> {
        Ok(self
            .running
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.starts_with(dir))
            .cloned()
            .collect())
    }

    fn starts(&self, program: &Path) -> bool {
        !self.agent_broken.load(Ordering::SeqCst)
            && fs::read_to_string(program).is_ok_and(|s| s.starts_with("passbundle-agent.exe "))
    }
}

/// A stand-in for the update site: `/agent/<name>` from `files`, and the
/// paths it was asked for.
struct Site {
    base: String,
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    asked: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Site {
    fn new() -> Site {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let base = format!("http://{}/agent", server.server_addr().to_ip().unwrap());
        let files: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::default();
        let asked: Arc<Mutex<Vec<String>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (files, asked, stop) = (files.clone(), asked.clone(), stop.clone());
            thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let Ok(Some(request)) = server.recv_timeout(Duration::from_millis(50)) else {
                        continue;
                    };
                    let path = request.url().to_owned();
                    asked.lock().unwrap().push(path.clone());
                    let body = path
                        .strip_prefix("/agent/")
                        .and_then(|name| files.lock().unwrap().get(name).cloned());
                    let _ = match body {
                        Some(body) => request.respond(tiny_http::Response::from_data(body)),
                        None => request.respond(tiny_http::Response::empty(404)),
                    };
                }
            })
        };
        Site {
            base,
            files,
            asked,
            stop,
            thread: Some(thread),
        }
    }

    /// A stand-in that answers one request on each connection and then ends
    /// it, without saying so first (an `HTTP/1.0` answer with no
    /// `Connection: close`): what an HTTP/1.0 server does, such as Python's
    /// http.server, which the meta repo's end-to-end test on Windows uses as
    /// the site, and what any server does that ends a kept connection just
    /// as the next request comes. It waits a moment before it ends the
    /// connection, reading nothing more, so a request sent on that
    /// connection in the meantime is always cut off.
    fn ending_each_connection() -> Site {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}/agent", listener.local_addr().unwrap());
        let files: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::default();
        let asked: Arc<Mutex<Vec<String>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (files, asked, stop) = (files.clone(), asked.clone(), stop.clone());
            thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let Ok((mut stream, _)) = listener.accept() else {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    };
                    let (files, asked) = (files.clone(), asked.clone());
                    thread::spawn(move || {
                        stream.set_nonblocking(false).unwrap();
                        let mut head = Vec::new();
                        let mut byte = [0u8];
                        while !head.ends_with(b"\r\n\r\n") {
                            match stream.read(&mut byte) {
                                Ok(1) => head.push(byte[0]),
                                _ => return,
                            }
                        }
                        let head = String::from_utf8_lossy(&head);
                        let path = head.split(' ').nth(1).unwrap_or_default().to_owned();
                        asked.lock().unwrap().push(path.clone());
                        let body = path
                            .strip_prefix("/agent/")
                            .and_then(|name| files.lock().unwrap().get(name).cloned());
                        let answer = match body {
                            Some(body) => [
                                format!(
                                    "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n",
                                    body.len()
                                )
                                .into_bytes(),
                                body,
                            ]
                            .concat(),
                            None => b"HTTP/1.0 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
                        };
                        let _ = stream.write_all(&answer);
                        thread::sleep(Duration::from_millis(200));
                        // Dropped: the connection ends.
                    });
                }
            })
        };
        Site {
            base,
            files,
            asked,
            stop,
            thread: Some(thread),
        }
    }

    fn put(&self, name: &str, body: Vec<u8>) {
        self.files.lock().unwrap().insert(name.to_owned(), body);
    }

    fn take(&self, name: &str) {
        self.files.lock().unwrap().remove(name);
    }

    /// Whether anything but manifests was downloaded since the last call.
    fn downloaded(&self) -> bool {
        let asked = std::mem::take(&mut *self.asked.lock().unwrap());
        asked.iter().any(|p| p.ends_with(".zip"))
    }
}

impl Drop for Site {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Keys {
    public: String,
    secret: minisign::SecretKey,
}

fn keys() -> Keys {
    let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    Keys {
        public: pair.pk.to_box().unwrap().to_string(),
        secret: pair.sk,
    }
}

/// A release's folder at `dir`, as release-folder.sh writes one: each of
/// `files` saying its name and `version`, and release.json listing them all.
fn release_folder(dir: &Path, version: &str, platform: &str, files: &[&str]) {
    for name in files {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!("{name} {version}")).unwrap();
    }
    fs::write(dir.join(release::RELEASE), b"").unwrap();
    let listing = release::listing(dir).unwrap();
    let info = serde_json::json!({
        "app": release::APP, "version": version, "platform": platform, "files": listing,
    });
    fs::write(dir.join(release::RELEASE), info.to_string()).unwrap();
}

/// `parent/PassBundle` as the release's zip, as zip-release.py packs it.
fn zip_of(parent: &Path) -> Vec<u8> {
    zip_with(parent, zip::CompressionMethod::Deflated)
}

/// The same, with each file compressed `method`'s way (stored, so a change
/// in one keeps the zip's size).
fn zip_with(parent: &Path, method: zip::CompressionMethod) -> Vec<u8> {
    fn add(
        zip: &mut zip::ZipWriter<io::Cursor<Vec<u8>>>,
        root: &Path,
        dir: &Path,
        method: zip::CompressionMethod,
    ) {
        let options = zip::write::SimpleFileOptions::default().compression_method(method);
        let name = |p: &Path| {
            p.strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        };
        zip.add_directory(format!("{}/", name(dir)), options)
            .unwrap();
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                add(zip, root, &path, method);
            } else {
                zip.start_file(name(&path), options).unwrap();
                zip.write_all(&fs::read(&path).unwrap()).unwrap();
            }
        }
    }
    let mut zip = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
    add(&mut zip, parent, &parent.join("PassBundle"), method);
    zip.finish().unwrap().into_inner()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A signed manifest for `zip`, at `version`, for `target`.
fn publish_manifest(site: &Site, keys: &Keys, version: &str, target: &str, zip: &[u8]) {
    let name = format!("PassBundle-{version}-windows-x64.zip");
    let manifest = serde_json::json!({
        "product": "passbundle-agent",
        "version": version,
        "target": target,
        "url": format!("{}/{name}", site.base),
        "size": zip.len(),
        "sha256": hex(&Sha256::digest(zip)),
    })
    .to_string()
    .into_bytes();
    let signature = minisign::sign(None, &keys.secret, io::Cursor::new(&manifest), None, None)
        .unwrap()
        .to_string();
    site.put(&format!("{TARGET}.json"), manifest);
    site.put(&format!("{TARGET}.json.minisig"), signature.into_bytes());
}

/// A release at `version` on the site, with its manifest: what's in it as
/// `files` and release.json say.
fn publish(site: &Site, keys: &Keys, version: &str, platform: &str, files: &[&str]) -> Vec<u8> {
    let pack = temp("pack");
    release_folder(&pack.join("PassBundle"), version, platform, files);
    let zip = zip_of(&pack);
    fs::remove_dir_all(&pack).unwrap();
    site.put(
        &format!("PassBundle-{version}-windows-x64.zip"),
        zip.clone(),
    );
    publish_manifest(site, keys, version, TARGET, &zip);
    zip
}

/// Program Files, with PassBundle 0.1.4 installed as the install script
/// puts it: a release's folder, and the host's manifest beside its programs.
fn installed(base: &Path) -> PathBuf {
    let install = base.join("Program Files").join("PassBundle");
    release_folder(&install, "0.1.4", PLATFORM, FILES);
    fs::write(
        install.join(HOST_MANIFEST),
        b"{\"name\":\"com.passbundle.agent\"}",
    )
    .unwrap();
    install
}

/// Whether every one of the release's files in `install` is `version`'s,
/// with the host's manifest as the install script wrote it, and nothing else.
fn is_version(install: &Path, version: &str) -> bool {
    let mut expected: Vec<String> = FILES.iter().map(|f| f.to_string()).collect();
    expected.extend(
        [
            "data",
            "data/flutter_assets",
            release::RELEASE,
            HOST_MANIFEST,
        ]
        .map(String::from),
    );
    expected.sort();
    release::listing(install).unwrap() == expected
        && FILES
            .iter()
            .all(|f| fs::read_to_string(install.join(f)).unwrap() == format!("{f} {version}"))
        && release::installed(install, PLATFORM) == Ok(Version::parse(version).unwrap())
        && fs::read(install.join(HOST_MANIFEST)).unwrap() == b"{\"name\":\"com.passbundle.agent\"}"
}

fn run(site: &Site, keys: &Keys, install: &Path, machine: &Fake) -> Result<Outcome, UpdateError> {
    update::run(&site.base, &keys.public, TARGET, install, machine)
}

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

#[test]
fn a_newer_release_replaces_the_agent_its_host_and_the_app_together() {
    let base = temp("newer");
    let install = installed(&base);
    let (site, keys, machine) = (Site::new(), keys(), Fake::default());
    publish(&site, &keys, "0.1.5", PLATFORM, FILES);
    assert_eq!(
        run(&site, &keys, &install, &machine),
        Ok(Outcome::Installed(v("0.1.5")))
    );
    assert!(is_version(&install, "0.1.5"));
    // Nothing left beside it: no download, no old version.
    assert!(!swap::work_dir(&install).exists());
    assert!(site.downloaded());
    // Again: nothing newer, nothing downloaded.
    assert_eq!(
        run(&site, &keys, &install, &machine),
        Ok(Outcome::UpToDate(v("0.1.5")))
    );
    assert!(!site.downloaded());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_same_or_an_older_release_is_never_downloaded() {
    let base = temp("older");
    let install = installed(&base);
    let (site, keys, machine) = (Site::new(), keys(), Fake::default());
    for version in ["0.1.4", "0.1.3"] {
        publish(&site, &keys, version, PLATFORM, FILES);
        assert_eq!(
            run(&site, &keys, &install, &machine),
            Ok(Outcome::UpToDate(v("0.1.4")))
        );
        assert!(!site.downloaded(), "{version}");
        assert!(is_version(&install, "0.1.4"));
    }
    assert!(!swap::work_dir(&install).exists());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_release_missing_a_program_or_with_a_file_it_doesnt_list_is_refused() {
    let base = temp("contents");
    let install = installed(&base);
    let (site, keys, machine) = (Site::new(), keys(), Fake::default());
    // Without its host, and release.json doesn't list one either.
    let without_host: Vec<&str> = FILES
        .iter()
        .copied()
        .filter(|f| *f != "passbundle-agent-host.exe")
        .collect();
    publish(&site, &keys, "0.1.5", PLATFORM, &without_host);
    let answer = run(&site, &keys, &install, &machine);
    assert!(
        matches!(&answer, Err(UpdateError::Release(why)) if why.contains("missing passbundle-agent-host.exe")),
        "{answer:?}"
    );
    assert!(is_version(&install, "0.1.4"));
    assert!(!swap::work_dir(&install).exists());

    // A file release.json lists that isn't there, or one it doesn't list.
    for change in ["missing", "extra"] {
        let pack = temp("pack");
        let folder = pack.join("PassBundle");
        release_folder(&folder, "0.1.5", PLATFORM, FILES);
        match change {
            "missing" => fs::remove_file(folder.join("data/icudtl.dat")).unwrap(),
            _ => fs::write(folder.join("version.dll"), b"planted").unwrap(),
        }
        let zip = zip_of(&pack);
        fs::remove_dir_all(&pack).unwrap();
        site.put("PassBundle-0.1.5-windows-x64.zip", zip.clone());
        publish_manifest(&site, &keys, "0.1.5", TARGET, &zip);
        let answer = run(&site, &keys, &install, &machine);
        assert!(
            matches!(&answer, Err(UpdateError::Release(why)) if why.contains("exactly what")),
            "{change}: {answer:?}"
        );
        assert!(is_version(&install, "0.1.4"), "{change}");
        assert!(!swap::work_dir(&install).exists(), "{change}");
    }
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_download_with_one_file_changed_is_refused() {
    let base = temp("hash");
    let install = installed(&base);
    let (site, keys, machine) = (Site::new(), keys(), Fake::default());
    // The release the manifest names, and the same with its host changed:
    // a whole zip, of the same size, that would unpack and check out.
    let pack = temp("pack");
    let folder = pack.join("PassBundle");
    release_folder(&folder, "0.1.5", PLATFORM, FILES);
    let signed = zip_with(&pack, zip::CompressionMethod::Stored);
    fs::write(
        folder.join("passbundle-agent-host.exe"),
        b"passbundle-agent-host.exe 6.6.6",
    )
    .unwrap();
    let swapped = zip_with(&pack, zip::CompressionMethod::Stored);
    fs::remove_dir_all(&pack).unwrap();
    assert_eq!(signed.len(), swapped.len());
    publish_manifest(&site, &keys, "0.1.5", TARGET, &signed);
    site.put("PassBundle-0.1.5-windows-x64.zip", swapped);
    assert_eq!(
        run(&site, &keys, &install, &machine),
        Err(UpdateError::HashMismatch)
    );
    assert!(is_version(&install, "0.1.4"));
    assert!(!swap::work_dir(&install).exists());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_release_for_another_platform_is_refused() {
    let base = temp("platform");
    let install = installed(&base);
    let (site, keys, machine) = (Site::new(), keys(), Fake::default());
    // A manifest for another platform, where Windows' should be.
    let zip = publish(&site, &keys, "0.1.5", PLATFORM, FILES);
    publish_manifest(&site, &keys, "0.1.5", "linux-x86_64", &zip);
    assert_eq!(
        run(&site, &keys, &install, &machine),
        Err(UpdateError::WrongTarget("linux-x86_64".into()))
    );
    assert!(!site.downloaded());
    // Windows' manifest, naming another platform's release.
    publish(&site, &keys, "0.1.5", "linux-x64", FILES);
    let answer = run(&site, &keys, &install, &machine);
    assert!(
        matches!(&answer, Err(UpdateError::Release(why)) if why.contains("this computer")),
        "{answer:?}"
    );
    assert!(is_version(&install, "0.1.4"));
    assert!(!swap::work_dir(&install).exists());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn while_a_program_runs_from_the_folder_the_update_waits_and_goes_in_next_time() {
    for program in [
        "passbundle-agent.exe",
        "passbundle.exe",
        "passbundle-agent-host.exe",
    ] {
        let base = temp("running");
        let install = installed(&base);
        let (site, keys, machine) = (Site::new(), keys(), Fake::default());
        publish(&site, &keys, "0.1.5", PLATFORM, FILES);
        // Someone's agent, the app, or a host a browser started.
        *machine.running.lock().unwrap() = vec![install.join(program)];
        assert_eq!(
            run(&site, &keys, &install, &machine),
            Ok(Outcome::Waiting(v("0.1.5")))
        );
        assert!(is_version(&install, "0.1.4"), "{program}");
        assert!(site.downloaded());
        // Still running: still waiting, and not downloaded again.
        assert_eq!(
            run(&site, &keys, &install, &machine),
            Ok(Outcome::Waiting(v("0.1.5")))
        );
        assert!(!site.downloaded());
        // Nothing runs from it (the computer starts): it goes in, without
        // asking the site again.
        machine.running.lock().unwrap().clear();
        site.take(&format!("{TARGET}.json"));
        assert_eq!(
            run(&site, &keys, &install, &machine),
            Ok(Outcome::Installed(v("0.1.5")))
        );
        assert!(is_version(&install, "0.1.5"), "{program}");
        assert!(!site.downloaded());
        assert!(!swap::work_dir(&install).exists());
        fs::remove_dir_all(&base).unwrap();
    }
}

#[test]
fn a_site_that_ends_each_connection_after_one_answer_still_updates() {
    let base = temp("ending");
    let install = installed(&base);
    let (site, keys, machine) = (Site::ending_each_connection(), keys(), Fake::default());
    publish(&site, &keys, "0.1.5", PLATFORM, FILES);
    // The manifest, its signature, and the zip, each asked for on a
    // connection of its own: one kept from the request before is cut off.
    *machine.running.lock().unwrap() = vec![install.join("passbundle-agent.exe")];
    assert_eq!(
        run(&site, &keys, &install, &machine),
        Ok(Outcome::Waiting(v("0.1.5")))
    );
    assert!(site.downloaded());
    machine.running.lock().unwrap().clear();
    assert_eq!(
        run(&site, &keys, &install, &machine),
        Ok(Outcome::Installed(v("0.1.5")))
    );
    assert!(is_version(&install, "0.1.5"));
    // The manifest and its signature again: nothing newer.
    assert_eq!(
        run(&site, &keys, &install, &machine),
        Ok(Outcome::UpToDate(v("0.1.5")))
    );
    assert!(!swap::work_dir(&install).exists());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_new_agent_that_doesnt_start_is_taken_out_and_the_old_version_stays() {
    let base = temp("broken");
    let install = installed(&base);
    let (site, keys, machine) = (Site::new(), keys(), Fake::default());
    publish(&site, &keys, "0.1.5", PLATFORM, FILES);
    machine.agent_broken.store(true, Ordering::SeqCst);
    let answer = run(&site, &keys, &install, &machine);
    assert!(
        matches!(&answer, Err(UpdateError::Kept(why)) if why.contains("didn't start") && why.contains("still in place")),
        "{answer:?}"
    );
    assert!(is_version(&install, "0.1.4"));
    // It waits, checked, and goes in once its agent starts.
    machine.agent_broken.store(false, Ordering::SeqCst);
    assert_eq!(
        run(&site, &keys, &install, &machine),
        Ok(Outcome::Installed(v("0.1.5")))
    );
    assert!(is_version(&install, "0.1.5"));
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_folder_others_can_change_or_that_isnt_a_release_is_left_alone() {
    let base = temp("places");
    let install = installed(&base);
    let (site, keys, machine) = (Site::new(), keys(), Fake::default());
    publish(&site, &keys, "0.1.5", PLATFORM, FILES);
    // The folder, the one it's in, or anything in it, that others can change.
    for shared in [
        install.clone(),
        install.parent().unwrap().to_path_buf(),
        install.join("passbundle_core.dll"),
        install.join("data/flutter_assets"),
    ] {
        *machine.shared.lock().unwrap() = vec![shared.clone()];
        let answer = run(&site, &keys, &install, &machine);
        assert!(
            matches!(&answer, Err(UpdateError::Folder(why)) if why.contains("only administrators")),
            "{shared:?}: {answer:?}"
        );
        assert!(!site.downloaded());
    }
    // A work folder someone else made is never used or removed.
    let work = swap::work_dir(&install);
    fs::create_dir(&work).unwrap();
    fs::write(work.join("theirs"), b"theirs").unwrap();
    *machine.shared.lock().unwrap() = vec![work.clone()];
    assert!(matches!(
        run(&site, &keys, &install, &machine),
        Err(UpdateError::Folder(_))
    ));
    assert_eq!(fs::read(work.join("theirs")).unwrap(), b"theirs");
    fs::remove_dir_all(&work).unwrap();
    machine.shared.lock().unwrap().clear();
    // Not installed from a release: nothing says which version it is.
    fs::remove_file(install.join(release::RELEASE)).unwrap();
    let answer = run(&site, &keys, &install, &machine);
    assert!(
        matches!(&answer, Err(UpdateError::Folder(why)) if why.contains("wasn't installed from one of PassBundle's releases")),
        "{answer:?}"
    );
    assert!(!site.downloaded());
    assert_eq!(
        fs::read_to_string(install.join("passbundle.exe")).unwrap(),
        "passbundle.exe 0.1.4"
    );
    fs::remove_dir_all(&base).unwrap();
}

#[cfg(unix)]
#[test]
fn a_link_in_the_folder_is_left_alone() {
    let base = temp("link");
    let install = installed(&base);
    let (site, keys, machine) = (Site::new(), keys(), Fake::default());
    publish(&site, &keys, "0.1.5", PLATFORM, FILES);
    std::os::unix::fs::symlink(&base, install.join("data/elsewhere")).unwrap();
    assert!(matches!(
        run(&site, &keys, &install, &machine),
        Err(UpdateError::Folder(_))
    ));
    assert!(!site.downloaded());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn an_update_waiting_when_the_install_script_puts_in_a_newer_one_is_dropped() {
    let base = temp("rmm");
    let install = installed(&base);
    let (site, keys, machine) = (Site::new(), keys(), Fake::default());
    publish(&site, &keys, "0.1.5", PLATFORM, FILES);
    *machine.running.lock().unwrap() = vec![install.join("passbundle.exe")];
    assert_eq!(
        run(&site, &keys, &install, &machine),
        Ok(Outcome::Waiting(v("0.1.5")))
    );
    // The RMM runs the install script again, with 0.1.6.
    release_folder(&install, "0.1.6", PLATFORM, FILES);
    machine.running.lock().unwrap().clear();
    assert_eq!(
        run(&site, &keys, &install, &machine),
        Ok(Outcome::UpToDate(v("0.1.6")))
    );
    assert!(is_version(&install, "0.1.6"));
    assert!(!swap::work_dir(&install).exists());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_release_from_anywhere_but_the_update_site_is_refused() {
    let base = temp("elsewhere");
    let install = installed(&base);
    let (site, keys, machine) = (Site::new(), keys(), Fake::default());
    let zip = publish(&site, &keys, "0.1.5", PLATFORM, FILES);
    // The same zip, signed, but named somewhere else on the same server.
    let manifest = serde_json::json!({
        "product": "passbundle-agent", "version": "0.1.5", "target": TARGET,
        "url": site.base.replace("/agent", "/elsewhere/PassBundle-0.1.5-windows-x64.zip"),
        "size": zip.len(), "sha256": hex(&Sha256::digest(&zip)),
    })
    .to_string()
    .into_bytes();
    let signature = minisign::sign(None, &keys.secret, io::Cursor::new(&manifest), None, None)
        .unwrap()
        .to_string();
    site.put(&format!("{TARGET}.json"), manifest);
    site.put(&format!("{TARGET}.json.minisig"), signature.into_bytes());
    assert!(matches!(
        run(&site, &keys, &install, &machine),
        Err(UpdateError::NotOurs(_))
    ));
    assert!(!site.downloaded());
    assert!(is_version(&install, "0.1.4"));
    fs::remove_dir_all(&base).unwrap();
}
