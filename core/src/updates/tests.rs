//! The whole check and download against a stand-in for GitHub on this
//! computer, with releases signed by a throwaway key: what Settings' Updates
//! does, up to starting `passbundle-update` (swap.rs has its own tests, and
//! tools/desktop-update-test.sh runs the real app through all of it).

use super::*;
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use std::sync::{Arc, MutexGuard};

/// The updater keeps what the last check found for the whole process, as
/// the app does: one test at a time.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn one_at_a_time() -> MutexGuard<'static, ()> {
    lock(&ONE_AT_A_TIME)
}

const OS: &str = "linux";
const PLATFORM: &str = "linux-x64";

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pb-updates-{name}-{}-{}",
        std::process::id(),
        rand::random::<u32>()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A release's folder at `dir`: everything a release has, at `version`.
/// Its `passbundle-update` is a script that notes how it was run; on
/// Windows, which runs only programs, this test program (which, given
/// `finish <folder>`, runs no tests, taking them for test names).
fn release_folder(dir: &Path, version: &str) {
    let mut files = vec![bundle::RELEASE.to_owned()];
    for name in bundle::required() {
        let path = dir.join(&name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        if name == swap::helper_name() && cfg!(windows) {
            fs::copy(std::env::current_exe().unwrap(), &path).unwrap();
            files.push(name);
            continue;
        }
        let body = if name == swap::helper_name() {
            "#!/bin/sh\necho \"$@\" > \"$2/helper-ran\"\n".to_owned()
        } else {
            format!("{name} {version}")
        };
        fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        files.push(name.split('/').next().unwrap().to_owned());
    }
    files.sort();
    files.dedup();
    let release =
        json!({ "app": manifest::APP, "version": version, "platform": PLATFORM, "files": files });
    fs::write(dir.join(bundle::RELEASE), release.to_string()).unwrap();
}

/// The release's tarball, as the Linux release workflow packs it.
fn tarball(version: &str) -> Vec<u8> {
    let base = temp("pack");
    release_folder(&base.join("PassBundle"), version);
    let mut builder = tar::Builder::new(Vec::new());
    builder
        .append_dir_all("PassBundle", base.join("PassBundle"))
        .unwrap();
    let tar = builder.into_inner().unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&tar).unwrap();
    fs::remove_dir_all(&base).unwrap();
    gz.finish().unwrap()
}

struct Signed {
    tag: String,
    version: String,
    manifest: Vec<u8>,
    signature: String,
    archive: Vec<u8>,
}

/// A release signed with `key`; its manifest says the archive is `size`
/// bytes (by default, what it is) with the SHA-256 of `hashed` (by default,
/// the archive).
fn signed(key: &SigningKey, version: &str, archive: Vec<u8>) -> Signed {
    signed_saying(
        key,
        version,
        archive.clone(),
        archive.len() as u64,
        &archive,
    )
}

fn signed_saying(
    key: &SigningKey,
    version: &str,
    archive: Vec<u8>,
    size: u64,
    hashed: &[u8],
) -> Signed {
    let tag = format!("{OS}-v{version}");
    let manifest = json!({
        "format": 1, "app": manifest::APP, "tag": tag, "version": version,
        "commit": "0123456789abcdef0123456789abcdef01234567",
        "notes": format!("What's new since 0.1.4:\n\n- Faster sync\n- Version {version}\n\nFiles: ..."),
        "files": [{
            "name": manifest::file_name(version, PLATFORM), "platform": PLATFORM,
            "size": size, "sha256": manifest::hex(&Sha256::digest(hashed)),
        }],
    })
    .to_string()
    .into_bytes();
    let mut message = manifest::PREFIX.to_vec();
    message.extend_from_slice(&manifest);
    let signature = base64::engine::general_purpose::STANDARD.encode(key.sign(&message).to_bytes());
    Signed {
        tag,
        version: version.to_owned(),
        manifest,
        signature,
        archive,
    }
}

/// A stand-in for GitHub's API and its releases' files, as GitHub answers:
/// the list, and each file by redirecting to where it is.
struct Stand {
    base: String,
    stop: Arc<AtomicBool>,
}

impl Drop for Stand {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// GitHub's list of releases: an Android one, then `releases`, with each
/// asset's URL from `url` (the release's own, by default).
fn stand_in(releases: &[&Signed], url: impl Fn(&str, &str, &str) -> String) -> Stand {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let base = format!("http://{}", server.server_addr().to_ip().unwrap());
    let mut list = vec![
        json!({ "tag_name": "android-v0.1.40", "draft": false, "prerelease": false, "assets": [] }),
    ];
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for r in releases {
        let name = manifest::file_name(&r.version, PLATFORM);
        list.push(json!({
            "tag_name": r.tag, "draft": false, "prerelease": false,
            "body": "What GitHub's page says, which the app doesn't show.",
            "assets": [
                { "name": name, "size": r.archive.len(), "browser_download_url": url(&base, &r.tag, &name),
                  "digest": format!("sha256:{}", manifest::hex(&Sha256::digest(&r.archive))) },
                { "name": "manifest.json", "browser_download_url": url(&base, &r.tag, "manifest.json") },
                { "name": "manifest.json.sig", "browser_download_url": url(&base, &r.tag, "manifest.json.sig") },
            ],
        }));
        files.push((format!("{}/{name}", r.tag), r.archive.clone()));
        files.push((format!("{}/manifest.json", r.tag), r.manifest.clone()));
        files.push((
            format!("{}/manifest.json.sig", r.tag),
            r.signature.clone().into_bytes(),
        ));
    }
    let list = Value::Array(list);
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let here = base.clone();
    std::thread::spawn(move || {
        while !stopping.load(Ordering::SeqCst) {
            let Ok(Some(request)) = server.recv_timeout(Duration::from_millis(100)) else {
                continue;
            };
            let url = request.url().to_owned();
            let downloads = format!("/{REPO}/releases/download/");
            let response = if url == format!("/repos/{REPO}/releases?per_page=100") {
                tiny_http::Response::from_data(list.to_string().into_bytes())
            } else if let Some(path) = url.strip_prefix("/assets/") {
                match files.iter().find(|(p, _)| p == path) {
                    Some((_, body)) => tiny_http::Response::from_data(body.clone()),
                    None => tiny_http::Response::from_data(Vec::new()).with_status_code(404),
                }
            } else if let Some(path) = url.strip_prefix(&downloads) {
                tiny_http::Response::from_data(Vec::new())
                    .with_status_code(302)
                    .with_header(
                        tiny_http::Header::from_bytes(
                            &b"Location"[..],
                            format!("{here}/assets/{path}").as_bytes(),
                        )
                        .unwrap(),
                    )
            } else {
                tiny_http::Response::from_data(Vec::new()).with_status_code(404)
            };
            let _ = request.respond(response);
        }
    });
    Stand { base, stop }
}

fn own_urls(base: &str, tag: &str, name: &str) -> String {
    format!("{base}/{REPO}/releases/download/{tag}/{name}")
}

/// An installed release at 0.1.4, alone in its folder, with the vault's
/// folder elsewhere.
fn context(base: &Path, stand: &Stand, key: &SigningKey) -> Context {
    let dir = base.join("PassBundle");
    release_folder(&dir, "0.1.4");
    Context {
        source: Source::test(&stand.base),
        keys: vec![key.verifying_key().to_bytes()],
        installed: Installed::at(&dir.join(bundle::program_name())),
        data_dir: base.join("data"),
        os: OS,
        platform: PLATFORM,
    }
}

fn key() -> SigningKey {
    SigningKey::from_bytes(&[42; 32])
}

#[test]
fn a_newer_signed_release_is_offered_with_its_manifests_notes() {
    let _one = one_at_a_time();
    let base = temp("offer");
    let key = key();
    let older = signed(&key, "0.1.3", tarball("0.1.3"));
    let newer = signed(&key, "0.1.5", tarball("0.1.5"));
    let stand = stand_in(&[&older, &newer], own_urls);
    let ctx = context(&base, &stand, &key);
    let status = status(&ctx);
    assert_eq!(status["version"], "0.1.4");
    assert_eq!(status["restarts"], true);
    assert!(status.get("installNote").is_none(), "{status}");
    let answer = check(&ctx, false);
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["available"], true);
    assert_eq!(answer["version"], "0.1.5");
    assert_eq!(answer["notes"], "- Faster sync\n- Version 0.1.5");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn nothing_newer_is_nothing_to_offer() {
    let _one = one_at_a_time();
    let base = temp("none");
    let key = key();
    let same = signed(&key, "0.1.4", tarball("0.1.4"));
    let older = signed(&key, "0.1.2", tarball("0.1.2"));
    let stand = stand_in(&[&same, &older], own_urls);
    let ctx = context(&base, &stand, &key);
    let answer = check(&ctx, false);
    assert_eq!(answer, json!({ "ok": true, "available": false }));
    assert_eq!(install(&ctx)["error"], "Check for updates first.");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_release_signed_with_another_key_is_refused() {
    let _one = one_at_a_time();
    let base = temp("other-key");
    let theirs = SigningKey::from_bytes(&[7; 32]);
    let newer = signed(&theirs, "0.1.5", tarball("0.1.5"));
    let stand = stand_in(&[&newer], own_urls);
    let ctx = context(&base, &stand, &key());
    let answer = check(&ctx, false);
    assert_eq!(answer["ok"], false);
    assert!(answer["error"]
        .as_str()
        .unwrap()
        .contains("isn't signed with PassBundle's update key"));
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn files_that_arent_on_the_releases_own_addresses_are_refused() {
    let _one = one_at_a_time();
    let base = temp("elsewhere");
    let key = key();
    let newer = signed(&key, "0.1.5", tarball("0.1.5"));
    // The manifest named somewhere else on the same host, and on another host.
    for url in [
        |base: &str, tag: &str, name: &str| {
            format!("{base}/someone/else/releases/download/{tag}/{name}")
        },
        |_: &str, tag: &str, name: &str| {
            format!("https://evil.example/{REPO}/releases/download/{tag}/{name}")
        },
    ] {
        let stand = stand_in(&[&newer], url);
        let ctx = context(&base, &stand, &key);
        let answer = check(&ctx, false);
        assert_eq!(answer["ok"], false, "{answer}");
        assert!(
            answer["error"]
                .as_str()
                .unwrap()
                .contains("signed manifest"),
            "{answer}"
        );
    }
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn an_offered_update_downloads_unpacks_and_starts_its_updater() {
    let _one = one_at_a_time();
    let base = temp("install");
    let key = key();
    let newer = signed(&key, "0.1.5", tarball("0.1.5"));
    let stand = stand_in(&[&newer], own_urls);
    let ctx = context(&base, &stand, &key);
    assert_eq!(check(&ctx, false)["available"], true);
    let answer = install(&ctx);
    assert_eq!(answer, json!({ "ok": true, "restart": true }));
    let work = base.join(".PassBundle-update");
    // The new version, unpacked and checked; the download gone.
    let staged = work.join("new/PassBundle");
    assert_eq!(bundle::read_release(&staged).unwrap().version, "0.1.5");
    assert!(!work.join("download").exists());
    let state: swap::State =
        serde_json::from_slice(&fs::read(work.join(swap::STATE)).unwrap()).unwrap();
    assert_eq!(state.install, ctx.installed.dir);
    assert_eq!((state.from.as_str(), state.to.as_str()), ("0.1.4", "0.1.5"));
    assert_eq!(state.app, std::process::id());
    // This copy's updater, copied out, started on the work folder.
    #[cfg(unix)]
    {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !work.join("helper-ran").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        let ran = fs::read_to_string(work.join("helper-ran")).unwrap();
        assert_eq!(ran.trim(), format!("finish {}", work.display()));
    }
    assert_eq!(progress(), json!({ "percent": null }));
    // On Windows, once the stand-in updater has quite ended.
    assert!(swap::remove(&base, Duration::from_secs(30)));
}

#[test]
fn a_download_that_isnt_what_the_manifest_says_is_refused_and_nothing_stays() {
    let _one = one_at_a_time();
    let base = temp("tampered");
    let key = key();
    let real = tarball("0.1.5");
    // Its SHA-256, and its size: what's served is one byte longer than the
    // signed manifest says (GitHub's own size and digest agreeing with what's
    // served, as an attacker who replaced the file would make them).
    let mut other = real.clone();
    *other.last_mut().unwrap() ^= 1;
    let changed = signed_saying(&key, "0.1.5", other, real.len() as u64, &real);
    let mut longer = real.clone();
    longer.push(0);
    let long = signed_saying(&key, "0.1.5", longer, real.len() as u64, &real);
    for (release, words) in [
        (&changed, "didn't match the release"),
        (&long, "larger than its release says"),
    ] {
        let stand = stand_in(&[release], own_urls);
        let ctx = context(&base, &stand, &key);
        // GitHub's size or digest of what it serves disagrees with the
        // manifest's, and the check says so.
        let answer = check(&ctx, false);
        assert_eq!(answer["ok"], false, "{answer}");
        assert!(answer["error"]
            .as_str()
            .unwrap()
            .contains("isn't the one its manifest names"));
        // Were GitHub's list to say nothing of them, the download is checked.
        let expect = Expect {
            tag: &release.tag,
            os: OS,
            platform: PLATFORM,
            running_build: 4,
        };
        *lock(&OFFER) = Some(Offer {
            release: manifest::read(&release.manifest, &release.signature, &ctx.keys, &expect)
                .unwrap(),
            url: own_urls(
                &stand.base,
                &release.tag,
                &manifest::file_name("0.1.5", PLATFORM),
            ),
        });
        let answer = install(&ctx);
        assert_eq!(answer["ok"], false, "{answer}");
        assert!(
            answer["error"].as_str().unwrap().contains(words),
            "{answer}"
        );
        assert!(!base.join(".PassBundle-update").exists());
    }
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_copy_that_cant_replace_itself_only_says_theres_an_update() {
    let _one = one_at_a_time();
    let base = temp("cant");
    let key = key();
    let newer = signed(&key, "0.1.5", tarball("0.1.5"));
    let stand = stand_in(&[&newer], own_urls);
    let mut ctx = context(&base, &stand, &key);
    // Someone's own file in its folder.
    fs::write(ctx.installed.dir.join("notes.txt"), b"mine").unwrap();
    let status = status(&ctx);
    assert!(status["installNote"]
        .as_str()
        .unwrap()
        .contains("other files"));
    assert_eq!(check(&ctx, false)["available"], true);
    let answer = install(&ctx);
    assert!(answer["error"].as_str().unwrap().contains("other files"));
    assert!(ctx.installed.dir.join("notes.txt").exists());
    assert!(!base.join(".PassBundle-update").exists());
    // Not a release's copy at all.
    fs::remove_file(ctx.installed.dir.join("notes.txt")).unwrap();
    fs::remove_file(ctx.installed.dir.join(bundle::RELEASE)).unwrap();
    ctx.installed = Installed::at(&ctx.installed.dir.join(bundle::program_name()));
    assert_eq!(super::status(&ctx)["version"], Value::Null);
    assert!(super::status(&ctx)["installNote"]
        .as_str()
        .unwrap()
        .contains("built on a computer"));
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_check_as_the_app_opens_is_once_a_day_and_only_for_a_release() {
    let _one = one_at_a_time();
    let base = temp("daily");
    let key = key();
    let newer = signed(&key, "0.1.5", tarball("0.1.5"));
    let stand = stand_in(&[&newer], own_urls);
    let mut ctx = context(&base, &stand, &key);
    assert_eq!(check(&ctx, true)["available"], true);
    assert_eq!(check(&ctx, true)["skipped"], true);
    // A day later.
    let mut prefs = read_prefs(&ctx);
    prefs.last_check -= DAY_SECONDS + 1.0;
    write_prefs(&ctx, &prefs);
    assert_eq!(check(&ctx, true)["available"], true);
    // Turned off.
    set_auto(&ctx, false);
    let mut prefs = read_prefs(&ctx);
    prefs.last_check = 0.0;
    write_prefs(&ctx, &prefs);
    assert_eq!(status(&ctx)["auto"], false);
    assert_eq!(check(&ctx, true)["skipped"], true);
    // Asked for, it checks anyway.
    assert_eq!(check(&ctx, false)["available"], true);
    // A copy built on a computer never checks by itself.
    set_auto(&ctx, true);
    fs::remove_file(ctx.installed.dir.join(bundle::RELEASE)).unwrap();
    ctx.installed = Installed::at(&ctx.installed.dir.join(bundle::program_name()));
    assert_eq!(check(&ctx, true)["skipped"], true);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn starting_after_an_update_says_how_it_went_and_cleans_up() {
    let _one = one_at_a_time();
    let base = temp("started");
    let key = key();
    let stand = stand_in(&[], own_urls);
    let ctx = context(&base, &stand, &key);
    // Nothing going on.
    assert_eq!(started(&ctx), json!({}));
    // An update that didn't happen, its updater finished.
    let work = ctx.work_dir();
    fs::create_dir_all(work.join(swap::OLD)).unwrap();
    let failed = swap::Failed {
        version: "0.1.5".into(),
        reason: "PassBundle 0.1.5 didn't start, so PassBundle went back to 0.1.4.".into(),
    };
    fs::write(
        work.join(swap::FAILED),
        serde_json::to_vec(&failed).unwrap(),
    )
    .unwrap();
    fs::write(work.join(swap::DONE), "").unwrap();
    *lock(&FAILED) = None;
    assert_eq!(status(&ctx)["failed"]["version"], "0.1.5");
    let answer = started(&ctx);
    assert_eq!(answer["failed"]["reason"], failed.reason.as_str());
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while work.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!work.exists());
    *lock(&FAILED) = None;
    fs::remove_dir_all(&base).unwrap();
}
