//! Unpacking a Windows release's zip, which holds one `PassBundle` folder.
//! Its signature and SHA-256 are checked before this runs, and this checks
//! again what any archive could do: every entry a plain file or folder inside
//! that one folder (no links, no absolute or `..` paths, no names Windows
//! reads as something else), and no more than it may hold, counted as it's
//! unpacked. The same checks as the app's own updater (the core's
//! `src/updates/unpack.rs`, which this is taken from): keep the two alike.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

/// The folder every release holds.
pub const TOP: &str = "PassBundle";

/// The most a release may hold once unpacked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub bytes: u64,
    pub files: u64,
}

/// 8 times the archive (PassBundle unpacks to about 3 times its archive),
/// never less than 64 MB nor more than 1 GB; at most 100,000 files.
pub fn limits(archive_size: u64) -> Limits {
    Limits {
        bytes: archive_size.saturating_mul(8).clamp(64 << 20, 1 << 30),
        files: 100_000,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnpackError {
    /// Not a zip, or damaged.
    Unreadable(String),
    /// An entry it mustn't have.
    Entry(String),
    /// More than it may hold.
    TooBig,
    /// Writing it out failed (the disk is full, say).
    Write(String),
}

impl std::fmt::Display for UnpackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnpackError::Unreadable(why) => write!(f, "it can't be unpacked ({why})"),
            UnpackError::Entry(name) => write!(f, "it holds something it mustn't ({name})"),
            UnpackError::TooBig => write!(f, "it would unpack to more than a release may"),
            UnpackError::Write(why) => write!(f, "it couldn't be unpacked here ({why})"),
        }
    }
}

/// Where an entry named `name` goes under `into`, or None when it isn't
/// plainly inside [`TOP`]. Forward slashes only; each part a plain name.
pub fn place(name: &str, into: &Path) -> Option<PathBuf> {
    let name = name.strip_suffix('/').unwrap_or(name);
    let mut parts = name.split('/');
    if parts.next() != Some(TOP) {
        return None;
    }
    let mut path = into.join(TOP);
    for part in parts {
        if !plain_part(part) {
            return None;
        }
        path.push(part);
    }
    // Belt and braces: nothing but plain parts below `into`.
    let below = path.strip_prefix(into).ok()?;
    below
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
        .then_some(path)
}

/// A part of a path Windows takes as just a name.
pub fn plain_part(part: &str) -> bool {
    if part.is_empty() || part == "." || part == ".." || part.len() > 255 {
        return false;
    }
    if part.ends_with('.') || part.ends_with(' ') {
        return false;
    }
    if part
        .chars()
        .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
    {
        return false;
    }
    // Windows' device names, with or without an extension (and the spaces
    // Windows drops before one): CON, PRN, AUX, NUL, CONIN$, CONOUT$, and
    // COM and LPT with a digit, superscript ones too.
    let stem = part
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_uppercase();
    let numbered = |prefix: &str| {
        stem.strip_prefix(prefix).is_some_and(|rest| {
            let mut chars = rest.chars();
            matches!(
                (chars.next(), chars.next()),
                (Some('0'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}'), None)
            )
        })
    };
    let device = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || numbered("COM")
        || numbered("LPT");
    !device
}

/// Makes `dir` and any folders above it (0755 on Linux and macOS, where
/// only the tests run; on Windows they take the work folder's access list).
fn make_dirs(dir: &Path) -> Result<(), UnpackError> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o755);
    }
    builder.create(dir).map_err(write_error)
}

/// Counts what's unpacked against the limits.
struct Budget {
    limits: Limits,
    bytes: u64,
    files: u64,
}

impl Budget {
    fn file(&mut self) -> Result<(), UnpackError> {
        self.files += 1;
        if self.files > self.limits.files {
            return Err(UnpackError::TooBig);
        }
        Ok(())
    }

    /// Copies `from` to a new file at `to`, within the budget.
    fn copy(&mut self, from: &mut impl Read, to: &Path) -> Result<(), UnpackError> {
        if let Some(parent) = to.parent() {
            make_dirs(parent)?;
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(to)
            .map_err(|e| {
                if e.kind() == io::ErrorKind::AlreadyExists {
                    UnpackError::Entry(format!("{} twice", to.display()))
                } else {
                    write_error(e)
                }
            })?;
        let mut buffer = vec![0u8; 64 << 10];
        loop {
            let n = match from.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(UnpackError::Unreadable(e.to_string())),
            };
            self.bytes += n as u64;
            if self.bytes > self.limits.bytes {
                return Err(UnpackError::TooBig);
            }
            file.write_all(&buffer[..n]).map_err(write_error)?;
        }
        file.sync_all().map_err(write_error)
    }
}

fn write_error(e: io::Error) -> UnpackError {
    UnpackError::Write(e.to_string())
}

/// Unpacks a release's zip into `into`, which must be empty.
pub fn zip(archive: &Path, into: &Path, limits: Limits) -> Result<(), UnpackError> {
    let file = fs::File::open(archive).map_err(|e| UnpackError::Unreadable(e.to_string()))?;
    let mut zip = zip::ZipArchive::new(io::BufReader::new(file))
        .map_err(|e| UnpackError::Unreadable(e.to_string()))?;
    if zip.len() as u64 > limits.files {
        return Err(UnpackError::TooBig);
    }
    // What it says it holds, before anything's written.
    let mut said: u64 = 0;
    for i in 0..zip.len() {
        let entry = zip
            .by_index_raw(i)
            .map_err(|e| UnpackError::Unreadable(e.to_string()))?;
        said = said.saturating_add(entry.size());
    }
    if said > limits.bytes {
        return Err(UnpackError::TooBig);
    }
    let mut budget = Budget {
        limits,
        bytes: 0,
        files: 0,
    };
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| UnpackError::Unreadable(e.to_string()))?;
        let name = entry
            .name()
            .map_err(|e| UnpackError::Unreadable(e.to_string()))?
            .into_owned();
        let to = place(&name, into).ok_or_else(|| UnpackError::Entry(name.clone()))?;
        budget.file()?;
        if entry.is_symlink() {
            return Err(UnpackError::Entry(name));
        }
        if entry.is_dir() {
            make_dirs(&to)?;
            continue;
        }
        if !entry.is_file() {
            return Err(UnpackError::Entry(name));
        }
        let declared = entry.size();
        let before = budget.bytes;
        budget.copy(&mut entry, &to)?;
        if budget.bytes - before != declared {
            return Err(UnpackError::Unreadable(format!(
                "{name} isn't the size it says"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pb-agent-unpack-{name}-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in entries {
            if name.ends_with('/') {
                zip.add_directory(*name, options).unwrap();
            } else {
                zip.start_file(*name, options).unwrap();
                zip.write_all(body).unwrap();
            }
        }
        zip.finish().unwrap();
    }

    #[test]
    fn only_plain_paths_inside_the_one_folder() {
        let into = Path::new("/tmp/x");
        assert_eq!(
            place("PassBundle/data/app.so", into),
            Some(into.join("PassBundle").join("data").join("app.so"))
        );
        assert_eq!(place("PassBundle/", into), Some(into.join("PassBundle")));
        for fine in ["PassBundle/COM10", "PassBundle/console.txt"] {
            assert!(place(fine, into).is_some(), "{fine}");
        }
        for bad in [
            "Other/passbundle.exe",
            "passbundle.exe",
            "/PassBundle/x",
            "PassBundle/../x",
            "PassBundle/./x",
            "PassBundle//x",
            "PassBundle/a\\..\\..\\x",
            "PassBundle/C:x",
            "PassBundle/x:stream",
            "PassBundle/NUL",
            "PassBundle/com1.txt",
            "PassBundle/CONOUT$",
            "PassBundle/lpt\u{b3}.txt",
            "PassBundle/nul .txt",
            "PassBundle/trailing.",
            "PassBundle/new\nline",
            "../PassBundle/x",
            "",
        ] {
            assert_eq!(place(bad, into), None, "{bad:?}");
        }
    }

    #[test]
    fn a_release_zip_unpacks() {
        let dir = temp("zip");
        let archive = dir.join("a.zip");
        make_zip(
            &archive,
            &[
                ("PassBundle/", b""),
                ("PassBundle/passbundle-agent.exe", b"MZ agent"),
                (
                    "PassBundle/data/flutter_assets/AssetManifest.bin",
                    b"assets",
                ),
            ],
        );
        let into = dir.join("out");
        fs::create_dir(&into).unwrap();
        zip(&archive, &into, limits(1000)).unwrap();
        assert_eq!(
            fs::read(into.join("PassBundle/passbundle-agent.exe")).unwrap(),
            b"MZ agent"
        );
        assert_eq!(
            fs::read(into.join("PassBundle/data/flutter_assets/AssetManifest.bin")).unwrap(),
            b"assets"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_zip_with_anything_outside_the_folder_or_too_much_in_it_is_refused() {
        let dir = temp("zip-bad");
        for (i, name) in [
            "PassBundle/../evil.exe",
            "Elsewhere/x",
            "PassBundle/a/../../x",
        ]
        .iter()
        .enumerate()
        {
            let archive = dir.join(format!("{i}.zip"));
            make_zip(&archive, &[("PassBundle/ok", b"ok"), (name, b"evil")]);
            let into = dir.join(format!("out{i}"));
            fs::create_dir(&into).unwrap();
            assert!(
                matches!(
                    zip(&archive, &into, limits(1000)),
                    Err(UnpackError::Entry(_))
                ),
                "{name}"
            );
            assert!(!dir.join("evil.exe").exists());
        }
        // Compresses to almost nothing, unpacks to more than it may.
        let archive = dir.join("bomb.zip");
        make_zip(&archive, &[("PassBundle/big", &vec![0u8; 2 << 20])]);
        let into = dir.join("bomb");
        fs::create_dir(&into).unwrap();
        let tight = Limits {
            bytes: 1 << 20,
            files: 10,
        };
        assert_eq!(zip(&archive, &into, tight), Err(UnpackError::TooBig));
        let archive = dir.join("many.zip");
        make_zip(&archive, &[("PassBundle/a", b"a"), ("PassBundle/b", b"b")]);
        let into = dir.join("many");
        fs::create_dir(&into).unwrap();
        let few = Limits {
            bytes: 64 << 20,
            files: 1,
        };
        assert_eq!(zip(&archive, &into, few), Err(UnpackError::TooBig));
        // Not a zip at all.
        let junk = dir.join("junk.zip");
        fs::write(&junk, b"not a zip").unwrap();
        assert!(matches!(
            zip(&junk, &dir, limits(9)),
            Err(UnpackError::Unreadable(_))
        ));
        fs::remove_dir_all(&dir).unwrap();
    }
}
