//! Release versions: `major.minor.patch`, compared numerically.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    major: u32,
    minor: u32,
    patch: u32,
}

impl Version {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// This build's version, from Cargo.toml.
    pub fn current() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).expect("Cargo.toml version is major.minor.patch")
    }

    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = s.split('.').map(|p| {
            // No leading zeros, signs, or spaces: one spelling per version.
            if p.is_empty()
                || (p.len() > 1 && p.starts_with('0'))
                || !p.bytes().all(|b| b.is_ascii_digit())
            {
                None
            } else {
                p.parse::<u32>().ok()
            }
        });
        let version = Self::new(parts.next()??, parts.next()??, parts.next()??);
        parts.next().is_none().then_some(version)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        let v = |s| Version::parse(s).unwrap();
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("1.0.0") > v("0.99.99"));
        assert_eq!(v("2.3.4").to_string(), "2.3.4");
    }

    #[test]
    fn only_plain_major_minor_patch_parses() {
        for bad in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "1.2.x",
            "01.2.3",
            "1.2.3-beta",
            " 1.2.3",
            "+1.2.3",
            "1..3",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad:?}");
        }
        assert_eq!(Version::parse("0.0.0"), Some(Version::new(0, 0, 0)));
    }
}
