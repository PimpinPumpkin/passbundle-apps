//! Updates, on Windows, from the update task the install script makes
//! (the meta repo's `Install-PassBundle.ps1`: as SYSTEM, as the computer
//! starts and daily): the agent, its native-messaging host, and the app,
//! which the script installs together in one folder, replaced together or
//! not at all. See DESIGN.md, "Desktop agent" and "Installer and agent".
//!
//! A release is a small JSON manifest on the website
//! (`https://<domain>/agent/windows-x86_64.json`), signed with the offline
//! update key in minisign's format (`windows-x86_64.json.minisig`). It names
//! one zip in the same place on the same site: a Windows release's, one
//! `PassBundle` folder with the agent, its host, the app, and `release.json`,
//! which lists every path in it. The agent trusts only the public key
//! compiled into it, and takes a release only if the signature checks out,
//! it's for this platform and newer than what's installed, the zip is served
//! over HTTPS from that place with the size and SHA-256 the manifest says
//! (hashed as it's written, and never more than that size), and it unpacks
//! to exactly what its `release.json` lists, with all three programs. Then
//! [`swap`] puts it in place, when nothing runs from the folder.

use std::{
    fmt, fs,
    io::{self, Read, Write},
    path::Path,
    time::Duration,
};

use minisign_verify::{PublicKey, Signature};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

use crate::machine::Machine;
use crate::version::Version;
use crate::{release, swap, unpack};

#[cfg(all(feature = "test-update", not(debug_assertions)))]
compile_error!(
    "the update test's key and site are for test builds only; release builds can't include them"
);

/// A Windows release is about 15 MB; anything much bigger isn't ours.
const MAX_RELEASE_BYTES: u64 = 200 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const PRODUCT: &str = "passbundle-agent";

/// This build's platform, as manifests name it: `windows-x86_64`, `macos-aarch64`, ...
pub fn target() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// The platform a target's releases are for, as their `release.json` says:
/// only Windows on x86-64 has releases the update task installs.
pub fn release_platform(target: &str) -> Option<&'static str> {
    (target == "windows-x86_64").then_some("windows-x64")
}

#[derive(Debug, PartialEq, Eq)]
pub enum UpdateError {
    /// This build has no update key, so it never updates itself.
    NoKey,
    BadKey,
    BadSignature,
    Malformed(String),
    WrongProduct,
    WrongTarget(String),
    /// The release's file isn't over HTTPS where the manifest is.
    NotOurs(String),
    TooLarge(u64),
    SizeMismatch {
        expected: u64,
        actual: u64,
    },
    /// The download went on past the manifest's size.
    Longer(u64),
    HashMismatch,
    Network(String),
    /// The installed folder isn't one the update task may change.
    Folder(String),
    /// The download isn't the release it should be.
    Release(String),
    /// The work folder can't be written.
    Disk(String),
    /// It couldn't be put in place: the old version still is.
    Kept(String),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoKey => write!(f, "this build doesn't update itself"),
            Self::BadKey => write!(f, "the update key compiled into this build is invalid"),
            Self::BadSignature => write!(f, "the release isn't signed with the update key"),
            Self::Malformed(e) => write!(f, "the release manifest is malformed: {e}"),
            Self::WrongProduct => write!(f, "the release manifest is for another product"),
            Self::WrongTarget(t) => write!(f, "the release is for {t}, not this computer"),
            Self::NotOurs(u) => {
                write!(
                    f,
                    "the release's file isn't served over HTTPS from the update site: {u}"
                )
            }
            Self::TooLarge(n) => write!(f, "the release is too large ({n} bytes)"),
            Self::SizeMismatch { expected, actual } => {
                write!(f, "the download is {actual} bytes, not {expected}")
            }
            Self::Longer(n) => write!(f, "the download is longer than the release's {n} bytes"),
            Self::HashMismatch => write!(f, "the download doesn't match the release's SHA-256"),
            Self::Network(e) => write!(f, "couldn't reach the update site: {e}"),
            Self::Folder(e) | Self::Release(e) | Self::Disk(e) | Self::Kept(e) => write!(f, "{e}"),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    product: String,
    version: String,
    target: String,
    url: String,
    size: u64,
    sha256: String,
}

/// A newer release for this platform, from a manifest whose signature checked out.
#[derive(Debug, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub url: String,
    pub size: u64,
    pub sha256: [u8; 32],
}

/// Checks a manifest's signature and contents. `source` is where the
/// manifest is (`https://<domain>/agent`), and the release's file must be
/// there too. `Ok(None)` means it's genuine but not newer than `current`.
pub fn verify_manifest(
    manifest: &[u8],
    signature: &str,
    key: &str,
    source: &str,
    current: Version,
    target: &str,
) -> Result<Option<Release>, UpdateError> {
    if key.is_empty() {
        return Err(UpdateError::NoKey);
    }
    let key = PublicKey::decode(key)
        .or_else(|_| PublicKey::from_base64(key))
        .map_err(|_| UpdateError::BadKey)?;
    let signature = Signature::decode(signature).map_err(|_| UpdateError::BadSignature)?;
    // Prehashed signatures only: minisign's legacy mode isn't accepted.
    key.verify(manifest, &signature, false)
        .map_err(|_| UpdateError::BadSignature)?;

    // Only now, with the signature checked, is the content worth reading.
    let m: Manifest =
        serde_json::from_slice(manifest).map_err(|e| UpdateError::Malformed(e.to_string()))?;
    if m.product != PRODUCT {
        return Err(UpdateError::WrongProduct);
    }
    if m.target != target {
        return Err(UpdateError::WrongTarget(m.target));
    }
    let version = Version::parse(&m.version)
        .ok_or_else(|| UpdateError::Malformed(format!("version {:?}", m.version)))?;
    if !ours(&m.url, source) {
        return Err(UpdateError::NotOurs(m.url));
    }
    if m.size > MAX_RELEASE_BYTES {
        return Err(UpdateError::TooLarge(m.size));
    }
    let sha256 = parse_sha256(&m.sha256)
        .ok_or_else(|| UpdateError::Malformed(format!("sha256 {:?}", m.sha256)))?;
    if version <= current {
        return Ok(None);
    }
    Ok(Some(Release {
        version,
        url: m.url,
        size: m.size,
        sha256,
    }))
}

/// Whether `url` is a zip right where the manifests are (`source`), over
/// HTTPS (plain HTTP only to this computer, in debug builds' tests): one
/// plain file name after it, nothing else.
fn ours(url: &str, source: &str) -> bool {
    let Some(name) = url
        .strip_prefix(source)
        .and_then(|rest| rest.strip_prefix('/'))
    else {
        return false;
    };
    (source.starts_with("https://") || allow_http(source))
        && name.ends_with(".zip")
        && !name.starts_with('.')
        && name.len() <= 200
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

fn parse_sha256(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Removes what a previous agent update left behind. Called at startup.
pub fn clean_up(exe: &Path) {
    let _ = fs::remove_file(sibling(exe, "old"));
    let _ = fs::remove_file(sibling(exe, "new"));
}

fn sibling(exe: &Path, suffix: &str) -> std::path::PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(suffix);
    exe.with_file_name(name)
}

/// Where this build looks for releases.
pub fn manifest_url(base: &str, target: &str) -> String {
    format!("{}/{target}.json", base.trim_end_matches('/'))
}

/// What came of a run.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing newer: the installed version.
    UpToDate(Version),
    /// A newer version is checked and waiting for nothing to run from the
    /// folder.
    Waiting(Version),
    /// This version is now in place.
    Installed(Version),
}

/// One run of the update task for the folder `install` (the agent's own):
/// an update that stopped partway is undone, one waiting goes in if nothing
/// runs from the folder, and the newest release at `source` is checked,
/// downloaded, and put in place if it's newer. Nothing is touched unless the
/// folder, the one it's in, and everything in it are only administrators' to
/// change, and the folder is a release's.
pub fn run(
    source: &str,
    key: &str,
    target: &str,
    install: &Path,
    machine: &impl Machine,
) -> Result<Outcome, UpdateError> {
    if key.is_empty() {
        return Err(UpdateError::NoKey);
    }
    let platform = release_platform(target).ok_or_else(|| {
        UpdateError::Folder(format!(
            "there are no releases for {target}: the agent is updated with the app here"
        ))
    })?;
    let work = swap::work_dir(install);
    swap::check_places(install, &work, machine).map_err(UpdateError::Folder)?;
    swap::recover(&work).map_err(UpdateError::Kept)?;
    swap::check_contents(install, machine).map_err(UpdateError::Folder)?;
    let installed = release::installed(install, platform).map_err(UpdateError::Folder)?;
    swap::clean(&work);

    // One that's waiting goes in first: at startup, before anyone signs in.
    let staged = swap::staged(&work, installed, platform);
    if let Some(version) = staged {
        if swap::put_in_place(install, &work, machine).map_err(UpdateError::Kept)? {
            return Ok(Outcome::Installed(version));
        }
    }

    let agent = http(source);
    let url = manifest_url(source, target);
    let manifest = fetch(&agent, &url, MAX_MANIFEST_BYTES)?;
    let signature = fetch(&agent, &format!("{url}.minisig"), MAX_MANIFEST_BYTES)?;
    let signature = String::from_utf8(signature).map_err(|_| UpdateError::BadSignature)?;
    let newest = staged.unwrap_or(installed);
    let Some(release) = verify_manifest(&manifest, &signature, key, source, newest, target)? else {
        return Ok(staged.map_or(Outcome::UpToDate(installed), Outcome::Waiting));
    };
    swap::discard(&work);
    stage(&agent, &release, &work, platform, machine)?;
    if swap::put_in_place(install, &work, machine).map_err(UpdateError::Kept)? {
        Ok(Outcome::Installed(release.version))
    } else {
        Ok(Outcome::Waiting(release.version))
    }
}

/// Downloads `release` into the work folder, hashed as it's written and
/// never more than its size, unpacks it, and checks what it unpacked to;
/// then notes it's ready. Whatever fails, nothing of it stays.
fn stage(
    agent: &ureq::Agent,
    release: &Release,
    work: &Path,
    platform: &str,
    machine: &impl Machine,
) -> Result<(), UpdateError> {
    let disk = |e: io::Error| UpdateError::Disk(format!("{}: {e}", work.display()));
    if fs::symlink_metadata(work).is_err() {
        machine.make_admin_dir(work).map_err(disk)?;
        if let Some(why) = machine.not_admin_only(work) {
            swap::remove(work);
            return Err(UpdateError::Disk(format!(
                "the work folder {} isn't only administrators' ({why})",
                work.display()
            )));
        }
    }
    let staged = (|| {
        let download = work.join(swap::DOWNLOAD);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&download)
            .map_err(disk)?;
        let mut hasher = Sha256::new();
        let mut read: u64 = 0;
        fetch_into(agent, &release.url, release.size, |chunk| {
            hasher.update(chunk);
            read += chunk.len() as u64;
            file.write_all(chunk)
        })?;
        file.sync_all().map_err(disk)?;
        drop(file);
        if read != release.size {
            return Err(UpdateError::SizeMismatch {
                expected: release.size,
                actual: read,
            });
        }
        if hasher.finalize().as_slice() != release.sha256 {
            return Err(UpdateError::HashMismatch);
        }
        let new = work.join(swap::NEW);
        fs::create_dir(&new).map_err(disk)?;
        unpack::zip(&download, &new, unpack::limits(release.size))
            .map_err(|e| UpdateError::Release(format!("the download can't be installed: {e}")))?;
        swap::remove(&download);
        release::check(
            &new.join(unpack::TOP),
            release.version,
            platform,
            &swap::kept(),
        )
        .map_err(UpdateError::Release)?;
        swap::mark_staged(work, release.version).map_err(disk)
    })();
    if staged.is_err() {
        swap::discard(work);
        swap::clean(work);
    }
    staged
}

/// Debug builds may test against a local server; release builds never use HTTP.
fn allow_http(base: &str) -> bool {
    cfg!(debug_assertions) && base.starts_with("http://127.0.0.1:")
}

fn http(source: &str) -> ureq::Agent {
    let tls = TlsConfig::builder()
        .provider(TlsProvider::NativeTls)
        .root_certs(RootCerts::PlatformVerifier)
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_global(Some(Duration::from_secs(15 * 60)))
        .https_only(!allow_http(source))
        // Only what's at the address itself: a redirect could lead anywhere.
        .max_redirects(0)
        .http_status_as_error(false)
        .user_agent("PassBundle agent")
        .build()
        .into()
}

/// GETs `url`, handing the body to `sink` as it comes; never more than `max`
/// bytes.
fn fetch_into(
    agent: &ureq::Agent,
    url: &str,
    max: u64,
    mut sink: impl FnMut(&[u8]) -> io::Result<()>,
) -> Result<(), UpdateError> {
    let network = |e: &dyn fmt::Display| UpdateError::Network(format!("{url}: {e}"));
    let mut response = agent.get(url).call().map_err(|e| network(&e))?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(network(&format!("HTTP {status}")));
    }
    // A byte past the limit, so a longer body is caught here rather than
    // taken for a short one.
    let mut reader = response.body_mut().with_config().limit(max + 1).reader();
    let mut buffer = vec![0u8; 64 << 10];
    let mut read: u64 = 0;
    loop {
        let n = match reader.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            // ureq's own limit, a byte past ours.
            Err(_) if read >= max => return Err(UpdateError::Longer(max)),
            Err(e) => return Err(network(&e)),
        };
        read += n as u64;
        if read > max {
            return Err(UpdateError::Longer(max));
        }
        sink(&buffer[..n]).map_err(|e| UpdateError::Disk(e.to_string()))?;
    }
}

fn fetch(agent: &ureq::Agent, url: &str, max: u64) -> Result<Vec<u8>, UpdateError> {
    let mut body = Vec::new();
    fetch_into(agent, url, max, |chunk| {
        body.extend_from_slice(chunk);
        Ok(())
    })?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn sign(keys: &Keys, data: &[u8]) -> String {
        minisign::sign(None, &keys.secret, io::Cursor::new(data), None, None)
            .unwrap()
            .to_string()
    }

    fn manifest(version: &str, target: &str, url: &str, bytes: &[u8]) -> Vec<u8> {
        serde_json::json!({
            "product": PRODUCT,
            "version": version,
            "target": target,
            "url": url,
            "size": bytes.len(),
            "sha256": Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(),
        })
        .to_string()
        .into_bytes()
    }

    const CURRENT: Version = Version::new(0, 1, 4);
    const SOURCE: &str = "https://passbundle.com/agent";
    const URL: &str = "https://passbundle.com/agent/PassBundle-0.1.5-windows-x64.zip";
    const TARGET: &str = "windows-x86_64";

    fn check(keys: &Keys, m: &[u8]) -> Result<Option<Release>, UpdateError> {
        verify_manifest(m, &sign(keys, m), &keys.public, SOURCE, CURRENT, TARGET)
    }

    #[test]
    fn a_newer_signed_release_for_this_platform_is_offered() {
        let keys = keys();
        let release = check(&keys, &manifest("0.1.5", TARGET, URL, b"zip"))
            .unwrap()
            .unwrap();
        assert_eq!(release.version, Version::new(0, 1, 5));
        assert_eq!(release.url, URL);
        assert_eq!(release.size, 3);
    }

    #[test]
    fn the_same_or_an_older_version_is_not_offered() {
        let keys = keys();
        for version in ["0.1.4", "0.1.3", "0.0.9"] {
            assert_eq!(
                check(&keys, &manifest(version, TARGET, URL, b"x")),
                Ok(None)
            );
        }
    }

    #[test]
    fn a_manifest_signed_by_another_key_or_changed_after_signing_is_refused() {
        let ours = keys();
        let theirs = keys();
        let m = manifest("9.0.0", TARGET, URL, b"x");
        assert_eq!(
            verify_manifest(
                &m,
                &sign(&theirs, &m),
                &ours.public,
                SOURCE,
                CURRENT,
                TARGET
            ),
            Err(UpdateError::BadSignature)
        );
        let signature = sign(&ours, &m);
        let mut changed = m.clone();
        let at = changed.iter().position(|&b| b == b'9').unwrap();
        changed[at] = b'8';
        assert_eq!(
            verify_manifest(&changed, &signature, &ours.public, SOURCE, CURRENT, TARGET),
            Err(UpdateError::BadSignature)
        );
    }

    #[test]
    fn signed_manifests_must_still_be_for_this_product_and_platform_and_on_the_site() {
        let keys = keys();
        assert_eq!(
            check(&keys, &manifest("0.1.5", "macos-aarch64", URL, b"x")),
            Err(UpdateError::WrongTarget("macos-aarch64".into()))
        );
        for elsewhere in [
            "http://passbundle.com/agent/PassBundle-0.1.5-windows-x64.zip",
            "https://passbundle.com.example.test/agent/a.zip",
            "https://example.test/agent/PassBundle-0.1.5-windows-x64.zip",
            "https://passbundle.com/PassBundle-0.1.5-windows-x64.zip",
            "https://passbundle.com/agent/old/PassBundle-0.1.5-windows-x64.zip",
            "https://passbundle.com/agent/../PassBundle-0.1.5-windows-x64.zip",
            "https://passbundle.com/agent/PassBundle-0.1.5-windows-x64.exe",
            "https://passbundle.com/agent/a.zip?b",
            "https://passbundle.com/agent/.zip",
        ] {
            assert_eq!(
                check(&keys, &manifest("0.1.5", TARGET, elsewhere, b"x")),
                Err(UpdateError::NotOurs(elsewhere.into())),
                "{elsewhere}"
            );
        }
        let other = serde_json::json!({
            "product": "something-else", "version": "0.1.5", "target": TARGET,
            "url": URL, "size": 1, "sha256": "00".repeat(32),
        });
        assert_eq!(
            check(&keys, other.to_string().as_bytes()),
            Err(UpdateError::WrongProduct)
        );
        let huge = serde_json::json!({
            "product": PRODUCT, "version": "0.1.5", "target": TARGET,
            "url": URL, "size": MAX_RELEASE_BYTES + 1, "sha256": "00".repeat(32),
        });
        assert_eq!(
            check(&keys, huge.to_string().as_bytes()),
            Err(UpdateError::TooLarge(MAX_RELEASE_BYTES + 1))
        );
    }

    #[test]
    fn a_build_without_an_update_key_never_updates() {
        let m = manifest("0.1.5", TARGET, URL, b"x");
        assert_eq!(
            verify_manifest(&m, "", "", SOURCE, CURRENT, TARGET),
            Err(UpdateError::NoKey)
        );
        struct Nothing;
        impl Machine for Nothing {
            fn not_admin_only(&self, _: &Path) -> Option<String> {
                unreachable!()
            }
            fn make_admin_dir(&self, _: &Path) -> io::Result<()> {
                unreachable!()
            }
            fn running_in(&self, _: &Path) -> Result<Vec<std::path::PathBuf>, String> {
                unreachable!()
            }
            fn starts(&self, _: &Path) -> bool {
                unreachable!()
            }
        }
        // Before it looks at anything.
        assert_eq!(
            run(SOURCE, "", TARGET, Path::new("/nowhere"), &Nothing),
            Err(UpdateError::NoKey)
        );
    }

    #[test]
    fn startup_cleans_up_what_an_earlier_agent_left() {
        let dir = std::env::temp_dir().join(format!("pb-agent-clean-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("passbundle-agent");
        fs::write(&exe, b"agent").unwrap();
        fs::write(sibling(&exe, "old"), b"old").unwrap();
        clean_up(&exe);
        assert!(!sibling(&exe, "old").exists());
        assert!(exe.exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
