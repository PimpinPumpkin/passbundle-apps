//! A Windows or Linux test release's manifest, and its Ed25519 signature
//! (the meta repo's DESIGN.md, "Desktop updates", with test vectors in
//! docs/vectors/desktop-update-v1.json, copied into tests/vectors).
//!
//! Each release on GitHub has `manifest.json`, which names the release's
//! tag and version and, for each file, its name, platform, size, and
//! SHA-256, and `manifest.json.sig`: base64 of the Ed25519 signature of
//! [`PREFIX`] followed by the manifest's exact bytes. The app trusts only
//! the public keys compiled into it ([`super::PINNED_KEYS`]), and reads
//! nothing from a manifest before its signature checks out.

use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;

/// What's signed is this, then the manifest: a signature made for anything
/// else, with the same key, is never taken for a manifest's.
pub const PREFIX: &[u8] = b"PassBundle desktop release v1\n";

/// The app a manifest must be for.
pub const APP: &str = "com.passbundle.app";

/// The manifest format this app reads.
pub const FORMAT: u64 = 1;

/// The most a manifest or its signature may be.
pub const MAX_MANIFEST_BYTES: u64 = 256 << 10;
pub const MAX_SIGNATURE_BYTES: u64 = 1 << 10;

/// The most a release's file may be: far more than PassBundle is (about 15
/// MB compressed on Windows and Linux).
pub const MAX_FILE_BYTES: u64 = 200 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// No key this app trusts signed it, or the signature isn't one.
    Signature,
    /// Signed, but not a manifest this app can read.
    Malformed(String),
    /// For another app.
    App,
    /// For another release than the one it's on.
    Tag { expected: String, found: String },
    /// Its version isn't the one its tag names.
    Version { tag: String, version: String },
    /// Not newer than this copy: the same release, or an older one.
    NotNewer { version: String },
    /// Nothing in it for this platform.
    NoFile { platform: String },
    /// A file name that isn't the release's own.
    Name(String),
    /// A file larger than any PassBundle could be.
    TooLarge(u64),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::Signature => write!(f, "it isn't signed with PassBundle's update key"),
            Refused::Malformed(why) => write!(f, "its manifest can't be read ({why})"),
            Refused::App => write!(f, "it's for another app"),
            Refused::Tag { expected, found } => {
                write!(f, "its manifest is for {found}, not {expected}")
            }
            Refused::Version { tag, version } => {
                write!(f, "its manifest says {version}, but it's {tag}")
            }
            Refused::NotNewer { version } => {
                write!(f, "{version} isn't newer than this copy")
            }
            Refused::NoFile { platform } => write!(f, "it has nothing for {platform}"),
            Refused::Name(name) => write!(f, "it names a file that isn't its own ({name})"),
            Refused::TooLarge(size) => write!(f, "its file is too large ({size} bytes)"),
        }
    }
}

/// A version as the releases number them: `<major>.<minor>.<build>`, three
/// numbers of ASCII digits. The build number counts every release of a
/// platform and only goes up, so it alone says which is newer (as on Android
/// and the Mac).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub build: u32,
}

impl Version {
    pub fn parse(text: &str) -> Option<Version> {
        let mut parts = text.split('.');
        let mut next = || -> Option<u32> {
            let part = parts.next()?;
            if part.is_empty() || part.len() > 9 || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            part.parse().ok()
        };
        let version = Version {
            major: next()?,
            minor: next()?,
            build: next()?,
        };
        parts.next().is_none().then_some(version)
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.build)
    }
}

/// The version a tag names (`windows-v0.1.12` for `windows`), or None for
/// another platform's tag or anything else.
pub fn version_of(tag: &str, os: &str) -> Option<Version> {
    Version::parse(tag.strip_prefix(os)?.strip_prefix("-v")?)
}

/// The newest of `tags` for `os`, by build number.
pub fn newest<'a>(tags: impl IntoIterator<Item = &'a str>, os: &str) -> Option<&'a str> {
    tags.into_iter()
        .filter_map(|tag| version_of(tag, os).map(|v| (tag, v.build)))
        .max_by_key(|&(_, build)| build)
        .map(|(tag, _)| tag)
}

/// A release's file name: `PassBundle-<version>-<platform>.<ext>`.
pub fn file_name(version: &str, platform: &str) -> String {
    let ext = if platform.starts_with("windows-") {
        "zip"
    } else {
        "tar.gz"
    };
    format!("PassBundle-{version}-{platform}.{ext}")
}

/// Whether `part` is plain enough to be a tag or a file name in a URL:
/// letters, digits, dots, hyphens, underscores, and plus signs, never "." or
/// "..".
pub fn is_plain(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-+".contains(&b))
}

#[derive(Deserialize)]
struct Raw {
    format: u64,
    app: String,
    tag: String,
    version: String,
    #[serde(default)]
    notes: String,
    files: Vec<RawFile>,
}

#[derive(Deserialize)]
struct RawFile {
    name: String,
    platform: String,
    size: u64,
    sha256: String,
}

/// What a release must be for this copy to take it.
#[derive(Debug, Clone)]
pub struct Expect<'a> {
    /// The tag the manifest was found on.
    pub tag: &'a str,
    /// `windows` or `linux`.
    pub os: &'a str,
    /// `windows-x64`, `linux-x64`, or `linux-arm64`.
    pub platform: &'a str,
    /// This copy's build number; 0 for a copy that isn't a release's.
    pub running_build: u32,
}

/// A manifest whose signature checked out, for this copy: what to download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub version: Version,
    pub notes: String,
    pub file: File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub name: String,
    pub size: u64,
    pub sha256: [u8; 32],
}

/// Whether `signature` (base64, as `manifest.json.sig` holds it) is one of
/// `keys`' over [`PREFIX`] and `manifest`. Strict verification: no
/// malleable signatures, and no weak keys.
pub fn verify(manifest: &[u8], signature: &str, keys: &[[u8; 32]]) -> Result<(), Refused> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(signature.trim_matches(|c: char| c.is_ascii_whitespace()))
        .map_err(|_| Refused::Signature)?;
    let signature = Signature::from_slice(&bytes).map_err(|_| Refused::Signature)?;
    let mut signed = Vec::with_capacity(PREFIX.len() + manifest.len());
    signed.extend_from_slice(PREFIX);
    signed.extend_from_slice(manifest);
    let good = keys.iter().any(|key| {
        VerifyingKey::from_bytes(key)
            .map(|key| key.verify_strict(&signed, &signature).is_ok())
            .unwrap_or(false)
    });
    if good {
        Ok(())
    } else {
        Err(Refused::Signature)
    }
}

/// Checks a manifest's signature, then what it says: for this app, for the
/// tag it was found on, at the version that tag names, newer than this
/// copy, with one file for this platform, named as the release's file is,
/// no larger than [`MAX_FILE_BYTES`], and with a SHA-256.
pub fn read(
    manifest: &[u8],
    signature: &str,
    keys: &[[u8; 32]],
    expect: &Expect,
) -> Result<Release, Refused> {
    verify(manifest, signature, keys)?;
    // Only now, signed, is it worth reading.
    let raw: Raw =
        serde_json::from_slice(manifest).map_err(|e| Refused::Malformed(e.to_string()))?;
    if raw.format != FORMAT {
        return Err(Refused::Malformed(format!("format {}", raw.format)));
    }
    if raw.app != APP {
        return Err(Refused::App);
    }
    if raw.tag != expect.tag {
        return Err(Refused::Tag {
            expected: expect.tag.to_owned(),
            found: raw.tag,
        });
    }
    let Some(version) = version_of(&raw.tag, expect.os) else {
        return Err(Refused::Tag {
            expected: format!("a {} release", expect.os),
            found: raw.tag,
        });
    };
    if Version::parse(&raw.version) != Some(version) || raw.version != version.to_string() {
        return Err(Refused::Version {
            tag: raw.tag,
            version: raw.version,
        });
    }
    if version.build <= expect.running_build {
        return Err(Refused::NotNewer {
            version: raw.version,
        });
    }
    let mut ours = raw
        .files
        .into_iter()
        .filter(|f| f.platform == expect.platform);
    let (Some(file), None) = (ours.next(), ours.next()) else {
        return Err(Refused::NoFile {
            platform: expect.platform.to_owned(),
        });
    };
    if file.name != file_name(&raw.version, expect.platform) {
        return Err(Refused::Name(file.name));
    }
    if file.size == 0 {
        return Err(Refused::Malformed("an empty file".into()));
    }
    if file.size > MAX_FILE_BYTES {
        return Err(Refused::TooLarge(file.size));
    }
    let sha256 = sha256_of(&file.sha256)
        .ok_or_else(|| Refused::Malformed(format!("SHA-256 {:?}", file.sha256)))?;
    Ok(Release {
        tag: raw.tag,
        version,
        notes: raw.notes,
        file: File {
            name: file.name,
            size: file.size,
            sha256,
        },
    })
}

/// 64 lowercase hex digits as 32 bytes.
pub fn sha256_of(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// What's new, from a release's notes: its "- " lines (the commit subjects
/// the release workflow lists), or the notes as they are if it has none. At
/// most `max` characters, as on Android and the Mac.
pub fn changes(notes: &str, max: usize) -> String {
    let lines: Vec<&str> = notes
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| l.starts_with("- "))
        .collect();
    let text = if lines.is_empty() {
        notes.trim().to_owned()
    } else {
        lines.join("\n")
    };
    if text.chars().count() <= max {
        return text;
    }
    let cut: String = text.chars().take(max).collect();
    let cut = cut.rsplit_once('\n').map(|(a, _)| a).unwrap_or(&cut);
    format!("{cut}\n…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn sign(key: &SigningKey, manifest: &[u8]) -> String {
        let mut signed = PREFIX.to_vec();
        signed.extend_from_slice(manifest);
        base64::engine::general_purpose::STANDARD.encode(key.sign(&signed).to_bytes())
    }

    fn manifest(tag: &str, version: &str, files: serde_json::Value) -> Vec<u8> {
        serde_json::json!({
            "format": 1,
            "app": APP,
            "tag": tag,
            "version": version,
            "commit": "0123456789abcdef0123456789abcdef01234567",
            "notes": "What's new since 0.1.4:\n\n- Faster sync\n- Updates on Windows\n\nFiles: ...",
            "files": files,
        })
        .to_string()
        .into_bytes()
    }

    fn windows_file(version: &str, size: u64, body: &[u8]) -> serde_json::Value {
        serde_json::json!([{
            "name": file_name(version, "windows-x64"),
            "platform": "windows-x64",
            "size": size,
            "sha256": hex(&Sha256::digest(body)),
        }])
    }

    const EXPECT: Expect = Expect {
        tag: "windows-v0.1.5",
        os: "windows",
        platform: "windows-x64",
        running_build: 4,
    };

    #[test]
    fn a_signed_newer_release_for_this_platform_is_taken() {
        let ours = key(7);
        let m = manifest("windows-v0.1.5", "0.1.5", windows_file("0.1.5", 3, b"zip"));
        let release = read(
            &m,
            &sign(&ours, &m),
            &[ours.verifying_key().to_bytes()],
            &EXPECT,
        )
        .unwrap();
        assert_eq!(release.version.to_string(), "0.1.5");
        assert_eq!(release.file.name, "PassBundle-0.1.5-windows-x64.zip");
        assert_eq!(release.file.size, 3);
        assert_eq!(
            release.file.sha256.as_slice(),
            Sha256::digest(b"zip").as_slice()
        );
        assert_eq!(
            changes(&release.notes, 4000),
            "- Faster sync\n- Updates on Windows"
        );
    }

    #[test]
    fn a_manifest_signed_with_another_key_is_refused() {
        let ours = key(7);
        let theirs = key(8);
        let m = manifest("windows-v0.1.5", "0.1.5", windows_file("0.1.5", 3, b"zip"));
        assert_eq!(
            read(
                &m,
                &sign(&theirs, &m),
                &[ours.verifying_key().to_bytes()],
                &EXPECT
            ),
            Err(Refused::Signature)
        );
        // Either of two pinned keys will do (a key being rotated in).
        let both = [
            ours.verifying_key().to_bytes(),
            theirs.verifying_key().to_bytes(),
        ];
        assert!(read(&m, &sign(&theirs, &m), &both, &EXPECT).is_ok());
    }

    #[test]
    fn a_manifest_changed_after_signing_is_refused() {
        let ours = key(7);
        let keys = [ours.verifying_key().to_bytes()];
        let m = manifest("windows-v0.1.5", "0.1.5", windows_file("0.1.5", 3, b"zip"));
        let signature = sign(&ours, &m);
        // The size, and the SHA-256.
        let text = String::from_utf8(m.clone()).unwrap();
        let bigger = text.replace("\"size\":3", "\"size\":4");
        assert_ne!(bigger, text);
        assert_eq!(
            read(bigger.as_bytes(), &signature, &keys, &EXPECT),
            Err(Refused::Signature)
        );
        let digest = hex(&Sha256::digest(b"zip"));
        let other = text.replace(&digest, &hex(&Sha256::digest(b"zap")));
        assert_eq!(
            read(other.as_bytes(), &signature, &keys, &EXPECT),
            Err(Refused::Signature)
        );
        // Any byte at all.
        for at in [0, m.len() / 2, m.len() - 1] {
            let mut changed = m.clone();
            changed[at] ^= 1;
            assert_eq!(
                read(&changed, &signature, &keys, &EXPECT),
                Err(Refused::Signature)
            );
        }
    }

    #[test]
    fn a_signature_without_the_prefix_or_that_isnt_one_is_refused() {
        let ours = key(7);
        let keys = [ours.verifying_key().to_bytes()];
        let m = manifest("windows-v0.1.5", "0.1.5", windows_file("0.1.5", 3, b"zip"));
        let bare = base64::engine::general_purpose::STANDARD.encode(ours.sign(&m).to_bytes());
        assert_eq!(read(&m, &bare, &keys, &EXPECT), Err(Refused::Signature));
        for junk in ["", "not base64!", "AAAA", &"A".repeat(88)] {
            assert_eq!(read(&m, junk, &keys, &EXPECT), Err(Refused::Signature));
        }
        // No keys, no updates.
        assert_eq!(
            read(&m, &sign(&ours, &m), &[], &EXPECT),
            Err(Refused::Signature)
        );
    }

    #[test]
    fn a_signed_manifest_must_be_for_its_tag_at_that_tags_version() {
        let ours = key(7);
        let keys = [ours.verifying_key().to_bytes()];
        let check = |m: Vec<u8>| read(&m, &sign(&ours, &m), &keys, &EXPECT);
        // A newer release's manifest put on this tag (or this one's on another).
        assert_eq!(
            check(manifest(
                "windows-v0.1.9",
                "0.1.9",
                windows_file("0.1.9", 3, b"z")
            )),
            Err(Refused::Tag {
                expected: "windows-v0.1.5".into(),
                found: "windows-v0.1.9".into()
            })
        );
        assert_eq!(
            check(manifest(
                "windows-v0.1.5",
                "0.1.6",
                windows_file("0.1.6", 3, b"z")
            )),
            Err(Refused::Version {
                tag: "windows-v0.1.5".into(),
                version: "0.1.6".into()
            })
        );
        assert_eq!(
            check(manifest(
                "windows-v0.1.5",
                "0.1.05",
                windows_file("0.1.05", 3, b"z")
            )),
            Err(Refused::Version {
                tag: "windows-v0.1.5".into(),
                version: "0.1.05".into()
            })
        );
        // Linux's manifest, even on a Windows tag.
        let linux = Expect {
            tag: "windows-v0.1.5",
            os: "linux",
            platform: "linux-x64",
            running_build: 4,
        };
        let m = manifest("windows-v0.1.5", "0.1.5", windows_file("0.1.5", 3, b"z"));
        assert!(matches!(
            read(&m, &sign(&ours, &m), &keys, &linux),
            Err(Refused::Tag { .. })
        ));
        let other = serde_json::to_vec(&serde_json::json!({
            "format": 1, "app": "com.example.other", "tag": "windows-v0.1.5",
            "version": "0.1.5", "files": windows_file("0.1.5", 3, b"z"),
        }))
        .unwrap();
        assert_eq!(check(other), Err(Refused::App));
        let future = serde_json::to_vec(&serde_json::json!({
            "format": 2, "app": APP, "tag": "windows-v0.1.5",
            "version": "0.1.5", "files": windows_file("0.1.5", 3, b"z"),
        }))
        .unwrap();
        assert!(matches!(check(future), Err(Refused::Malformed(_))));
    }

    #[test]
    fn the_same_release_or_an_older_one_is_refused() {
        let ours = key(7);
        let keys = [ours.verifying_key().to_bytes()];
        for (tag, version) in [("windows-v0.1.4", "0.1.4"), ("windows-v0.1.3", "0.1.3")] {
            let m = manifest(tag, version, windows_file(version, 3, b"z"));
            let expect = Expect { tag, ..EXPECT };
            assert_eq!(
                read(&m, &sign(&ours, &m), &keys, &expect),
                Err(Refused::NotNewer {
                    version: version.into()
                })
            );
        }
        // A copy that isn't a release's (build 0) takes any release.
        let m = manifest("windows-v0.1.1", "0.1.1", windows_file("0.1.1", 3, b"z"));
        let expect = Expect {
            tag: "windows-v0.1.1",
            running_build: 0,
            ..EXPECT
        };
        assert!(read(&m, &sign(&ours, &m), &keys, &expect).is_ok());
    }

    #[test]
    fn the_file_must_be_this_platforms_named_as_the_release_and_not_too_large() {
        let ours = key(7);
        let keys = [ours.verifying_key().to_bytes()];
        let check = |files: serde_json::Value| {
            let m = manifest("windows-v0.1.5", "0.1.5", files);
            read(&m, &sign(&ours, &m), &keys, &EXPECT)
        };
        let digest = hex(&Sha256::digest(b"z"));
        assert_eq!(
            check(serde_json::json!([{
                "name": file_name("0.1.5", "linux-x64"), "platform": "linux-x64",
                "size": 3, "sha256": digest,
            }])),
            Err(Refused::NoFile {
                platform: "windows-x64".into()
            })
        );
        assert_eq!(
            check(serde_json::json!([{
                "name": "../../passbundle.exe", "platform": "windows-x64",
                "size": 3, "sha256": digest,
            }])),
            Err(Refused::Name("../../passbundle.exe".into()))
        );
        assert_eq!(
            check(serde_json::json!([{
                "name": file_name("0.1.5", "windows-x64"), "platform": "windows-x64",
                "size": MAX_FILE_BYTES + 1, "sha256": digest,
            }])),
            Err(Refused::TooLarge(MAX_FILE_BYTES + 1))
        );
        assert!(matches!(
            check(serde_json::json!([{
                "name": file_name("0.1.5", "windows-x64"), "platform": "windows-x64",
                "size": 3, "sha256": digest.to_uppercase(),
            }])),
            Err(Refused::Malformed(_))
        ));
        // Two files for one platform: which would it be?
        let one = serde_json::json!({
            "name": file_name("0.1.5", "windows-x64"), "platform": "windows-x64",
            "size": 3, "sha256": digest,
        });
        assert!(matches!(
            check(serde_json::json!([one, one])),
            Err(Refused::NoFile { .. })
        ));
    }

    #[test]
    fn tags_and_versions() {
        assert_eq!(
            version_of("linux-v0.1.12", "linux"),
            Some(Version {
                major: 0,
                minor: 1,
                build: 12
            })
        );
        for tag in [
            "linux-v0.1",
            "linux-v0.1.12.1",
            "linux-v0.1.x",
            "linux-v0.1.-1",
            "linux-v 0.1.1",
            "macos-v0.1.12",
            "android-v0.1.12",
            "linux-v0.1.1234567890",
        ] {
            assert_eq!(version_of(tag, "linux"), None, "{tag}");
        }
        assert_eq!(
            newest(
                [
                    "android-v0.1.40",
                    "linux-v0.1.9",
                    "linux-v0.1.12",
                    "macos-v0.1.50",
                    "linux-v0.1.10"
                ],
                "linux"
            ),
            Some("linux-v0.1.12")
        );
        assert_eq!(newest(["android-v0.1.40"], "windows"), None);
        assert!(is_plain("PassBundle-0.1.5-windows-x64.zip"));
        for bad in ["", ".", "..", "a/b", "a b", "a%2Fb", "a\\b"] {
            assert!(!is_plain(bad), "{bad}");
        }
    }
}
