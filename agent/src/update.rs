//! Self-update. Each platform has a small JSON manifest next to the website
//! (`https://<domain>/agent/<target>.json`), signed with the offline update
//! key in minisign's format (`<target>.json.minisig`). The agent trusts only
//! the public key compiled into it, and installs a release only if the
//! signature checks out, it's for this platform and newer than what's running,
//! and the file it names is served over HTTPS with the size and SHA-256 the
//! manifest says. See DESIGN.md, "Desktop agent".

use std::{
    fmt, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use minisign_verify::{PublicKey, Signature};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

use crate::version::Version;

/// Releases are a few MB; anything much bigger isn't ours.
const MAX_RELEASE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const PRODUCT: &str = "passbundle-agent";

/// This build's platform, as manifests name it: `windows-x86_64`, `macos-aarch64`, ...
pub fn target() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
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
    NotHttps(String),
    TooLarge(u64),
    SizeMismatch {
        expected: u64,
        actual: u64,
    },
    HashMismatch,
    Network(String),
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
            Self::NotHttps(u) => write!(f, "the release isn't served over HTTPS: {u}"),
            Self::TooLarge(n) => write!(f, "the release is too large ({n} bytes)"),
            Self::SizeMismatch { expected, actual } => {
                write!(f, "the download is {actual} bytes, not {expected}")
            }
            Self::HashMismatch => write!(f, "the download doesn't match the release's SHA-256"),
            Self::Network(e) => write!(f, "couldn't reach the update server: {e}"),
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

/// Checks a manifest's signature and contents. `Ok(None)` means it's
/// genuine but not newer than `current`.
pub fn verify_manifest(
    manifest: &[u8],
    signature: &str,
    key: &str,
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
    if !m.url.starts_with("https://") && !allow_http(&m.url) {
        return Err(UpdateError::NotHttps(m.url));
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

/// Checks that a download is the release the manifest describes.
pub fn verify_download(release: &Release, bytes: &[u8]) -> Result<(), UpdateError> {
    if bytes.len() as u64 != release.size {
        return Err(UpdateError::SizeMismatch {
            expected: release.size,
            actual: bytes.len() as u64,
        });
    }
    if Sha256::digest(bytes).as_slice() != release.sha256 {
        return Err(UpdateError::HashMismatch);
    }
    Ok(())
}

/// Replaces the executable at `exe` with `bytes`. The running file is moved
/// aside first (Windows allows renaming a running executable, not overwriting
/// it), so a failure part way leaves either the old or the new one in place.
pub fn install(exe: &Path, bytes: &[u8]) -> io::Result<()> {
    let new = sibling(exe, "new");
    let old = sibling(exe, "old");
    {
        let mut file = fs::File::create(&new)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&new, fs::Permissions::from_mode(0o755))?;
    }
    let _ = fs::remove_file(&old);
    fs::rename(exe, &old)?;
    if let Err(e) = fs::rename(&new, exe) {
        // Put the old one back rather than leave nothing.
        let _ = fs::rename(&old, exe);
        return Err(e);
    }
    Ok(())
}

/// Removes what a previous update left behind. Called at startup.
pub fn clean_up(exe: &Path) {
    let _ = fs::remove_file(sibling(exe, "old"));
    let _ = fs::remove_file(sibling(exe, "new"));
}

fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(suffix);
    exe.with_file_name(name)
}

/// Where this build looks for releases.
pub fn manifest_url(base: &str, target: &str) -> String {
    format!("{}/{target}.json", base.trim_end_matches('/'))
}

/// Fetches, checks, downloads, and installs the newest release, if there is
/// one. Returns the version installed.
pub fn run(
    base: &str,
    key: &str,
    current: Version,
    exe: &Path,
) -> Result<Option<Version>, UpdateError> {
    if key.is_empty() {
        return Err(UpdateError::NoKey);
    }
    let target = target();
    let url = manifest_url(base, &target);
    let tls = TlsConfig::builder()
        .provider(TlsProvider::NativeTls)
        .root_certs(RootCerts::PlatformVerifier)
        .build();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(Duration::from_secs(120)))
        .https_only(!allow_http(base))
        .build()
        .into();
    let manifest = fetch(&agent, &url, MAX_MANIFEST_BYTES)?;
    let signature = fetch(&agent, &format!("{url}.minisig"), MAX_MANIFEST_BYTES)?;
    let signature = String::from_utf8(signature).map_err(|_| UpdateError::BadSignature)?;
    let Some(release) = verify_manifest(&manifest, &signature, key, current, &target)? else {
        return Ok(None);
    };
    // ureq refuses a body as large as its limit, so allow one byte more;
    // verify_download then holds it to the exact size.
    let bytes = fetch(&agent, &release.url, release.size + 1)?;
    verify_download(&release, &bytes)?;
    install(exe, &bytes).map_err(|e| UpdateError::Network(format!("installing: {e}")))?;
    Ok(Some(release.version))
}

/// Debug builds may test against a local server; release builds never use HTTP.
fn allow_http(base: &str) -> bool {
    cfg!(debug_assertions) && base.starts_with("http://127.0.0.1:")
}

fn fetch(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Vec<u8>, UpdateError> {
    let mut response = agent
        .get(url)
        .call()
        .map_err(|e| UpdateError::Network(format!("{url}: {e}")))?;
    response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|e| UpdateError::Network(format!("{url}: {e}")))
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

    const CURRENT: Version = Version::new(0, 1, 0);
    const URL: &str = "https://passbundle.com/agent/passbundle-agent-0.2.0.exe";

    #[test]
    fn a_newer_signed_release_for_this_platform_is_offered() {
        let keys = keys();
        let m = manifest("0.2.0", "windows-x86_64", URL, b"new agent");
        let release = verify_manifest(
            &m,
            &sign(&keys, &m),
            &keys.public,
            CURRENT,
            "windows-x86_64",
        )
        .unwrap()
        .unwrap();
        assert_eq!(release.version, Version::new(0, 2, 0));
        assert_eq!(release.url, URL);
        assert_eq!(verify_download(&release, b"new agent"), Ok(()));
    }

    #[test]
    fn the_same_or_an_older_version_is_not_offered() {
        let keys = keys();
        for version in ["0.1.0", "0.0.9"] {
            let m = manifest(version, "macos-aarch64", URL, b"x");
            assert_eq!(
                verify_manifest(&m, &sign(&keys, &m), &keys.public, CURRENT, "macos-aarch64"),
                Ok(None)
            );
        }
    }

    #[test]
    fn a_manifest_signed_by_another_key_or_changed_after_signing_is_refused() {
        let ours = keys();
        let theirs = keys();
        let m = manifest("9.0.0", "windows-x86_64", URL, b"x");
        assert_eq!(
            verify_manifest(
                &m,
                &sign(&theirs, &m),
                &ours.public,
                CURRENT,
                "windows-x86_64"
            ),
            Err(UpdateError::BadSignature)
        );
        let signature = sign(&ours, &m);
        let mut changed = m.clone();
        let at = changed.iter().position(|&b| b == b'9').unwrap();
        changed[at] = b'8';
        assert_eq!(
            verify_manifest(
                &changed,
                &signature,
                &ours.public,
                CURRENT,
                "windows-x86_64"
            ),
            Err(UpdateError::BadSignature)
        );
    }

    #[test]
    fn signed_manifests_must_still_be_for_this_product_platform_and_https() {
        let keys = keys();
        let check = |m: Vec<u8>| {
            verify_manifest(
                &m,
                &sign(&keys, &m),
                &keys.public,
                CURRENT,
                "windows-x86_64",
            )
        };
        assert_eq!(
            check(manifest("0.2.0", "macos-aarch64", URL, b"x")),
            Err(UpdateError::WrongTarget("macos-aarch64".into()))
        );
        assert_eq!(
            check(manifest(
                "0.2.0",
                "windows-x86_64",
                "http://passbundle.com/a.exe",
                b"x"
            )),
            Err(UpdateError::NotHttps("http://passbundle.com/a.exe".into()))
        );
        let other = serde_json::json!({
            "product": "something-else", "version": "0.2.0", "target": "windows-x86_64",
            "url": URL, "size": 1, "sha256": "00".repeat(32),
        });
        assert_eq!(
            check(other.to_string().into_bytes()),
            Err(UpdateError::WrongProduct)
        );
        let huge = serde_json::json!({
            "product": PRODUCT, "version": "0.2.0", "target": "windows-x86_64",
            "url": URL, "size": MAX_RELEASE_BYTES + 1, "sha256": "00".repeat(32),
        });
        assert_eq!(
            check(huge.to_string().into_bytes()),
            Err(UpdateError::TooLarge(MAX_RELEASE_BYTES + 1))
        );
    }

    #[test]
    fn a_build_without_an_update_key_never_updates() {
        let m = manifest("0.2.0", "windows-x86_64", URL, b"x");
        assert_eq!(
            verify_manifest(&m, "", "", CURRENT, "windows-x86_64"),
            Err(UpdateError::NoKey)
        );
    }

    #[test]
    fn a_download_must_match_the_manifest() {
        let keys = keys();
        let m = manifest("0.2.0", "windows-x86_64", URL, b"new agent");
        let release = verify_manifest(
            &m,
            &sign(&keys, &m),
            &keys.public,
            CURRENT,
            "windows-x86_64",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            verify_download(&release, b"new agenT"),
            Err(UpdateError::HashMismatch)
        );
        assert_eq!(
            verify_download(&release, b"new agent!"),
            Err(UpdateError::SizeMismatch {
                expected: 9,
                actual: 10
            })
        );
    }

    #[test]
    fn installing_replaces_the_executable_and_startup_cleans_up() {
        let dir = std::env::temp_dir().join(format!("pb-agent-install-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("passbundle-agent");
        fs::write(&exe, b"old").unwrap();
        install(&exe, b"new").unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"new");
        assert!(sibling(&exe, "old").exists());
        clean_up(&exe);
        assert!(!sibling(&exe, "old").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    /// The whole path against a local server: manifest, signature, download, install.
    #[test]
    fn run_installs_a_newer_release_from_the_server() {
        let keys = keys();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let base = format!("http://{}", server.server_addr().to_ip().unwrap());
        let target = target();
        let binary = b"release 0.2.0".to_vec();
        let m = manifest(
            "0.2.0",
            &target,
            &format!("{base}/passbundle-agent-0.2.0"),
            &binary,
        );
        let files: Vec<(String, Vec<u8>)> = vec![
            (format!("/{target}.json"), m.clone()),
            (
                format!("/{target}.json.minisig"),
                sign(&keys, &m).into_bytes(),
            ),
            ("/passbundle-agent-0.2.0".into(), binary.clone()),
        ];
        let handle = std::thread::spawn(move || {
            for _ in 0..files.len() {
                let request = server.recv().unwrap();
                let body = files
                    .iter()
                    .find(|(path, _)| path == request.url())
                    .map(|(_, body)| body.clone())
                    .unwrap_or_default();
                request
                    .respond(tiny_http::Response::from_data(body))
                    .unwrap();
            }
        });
        let dir = std::env::temp_dir().join(format!("pb-agent-run-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("passbundle-agent");
        fs::write(&exe, b"release 0.1.0").unwrap();
        assert_eq!(
            run(&base, &keys.public, CURRENT, &exe),
            Ok(Some(Version::new(0, 2, 0)))
        );
        assert_eq!(fs::read(&exe).unwrap(), binary);
        handle.join().unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }
}
