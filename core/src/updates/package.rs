//! Whether a package manager installed this copy, on Linux: PassBundle's
//! .deb or .rpm, which put it in /opt/passbundle as the meta repo's install
//! script does, or its Flatpak (README, "Linux packages"; the meta repo's
//! DESIGN.md, "Linux packages"). That package manager updates it, so the app
//! only says there's a new version, and how to get it that way. There's no
//! apt or dnf repository yet, so for a .deb or an .rpm that's the next
//! release's package, installed as the first one was.

use std::path::Path;
use std::process::{Command, Stdio};

/// What installed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Package {
    Deb,
    /// An .rpm, and the tool that installs one here: dnf, or zypper on
    /// openSUSE.
    Rpm(&'static str),
    Flatpak,
}

impl Package {
    /// What the person is told instead of an Update button, for this
    /// platform's package.
    pub fn note(self, platform: &str) -> String {
        let arm = platform == "linux-arm64";
        match self {
            Package::Deb => format!(
                "PassBundle was installed from its .deb package, so it can't update itself. Download the new version's .deb from PassBundle's releases on GitHub and install it with apt: sudo apt install ./passbundle_<version>_{}.deb",
                if arm { "arm64" } else { "amd64" }
            ),
            Package::Rpm(tool) => format!(
                "PassBundle was installed from its .rpm package, so it can't update itself. Download the new version's .rpm from PassBundle's releases on GitHub and install it with {tool}: sudo {tool} install ./passbundle-<version>-1.{}.rpm",
                if arm { "aarch64" } else { "x86_64" }
            ),
            Package::Flatpak => {
                "PassBundle came from Flatpak, so it can't update itself. Update it with Flatpak: flatpak update com.passbundle.app".into()
            }
        }
    }
}

/// Where the answers are: the file Flatpak puts in every sandbox, and the
/// package managers' own tools, by their full paths.
struct Places {
    flatpak_info: &'static str,
    dpkg_query: &'static str,
    rpm: &'static str,
    zypper: &'static str,
}

const SYSTEM: Places = Places {
    flatpak_info: "/.flatpak-info",
    dpkg_query: "/usr/bin/dpkg-query",
    rpm: "/usr/bin/rpm",
    zypper: "/usr/bin/zypper",
};

/// What installed the program at `program`: None when no package did, and
/// anywhere but Linux. It asks as the install script does, which package
/// owns the program (`dpkg-query -S`, `rpm -qf`).
pub fn installed_by(program: &Path) -> Option<Package> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    installed_by_in(program, &SYSTEM)
}

fn installed_by_in(program: &Path, places: &Places) -> Option<Package> {
    if Path::new(places.flatpak_info).is_file() {
        return Some(Package::Flatpak);
    }
    if owns(places.dpkg_query, "-S", program) {
        return Some(Package::Deb);
    }
    if owns(places.rpm, "-qf", program) {
        let tool = if Path::new(places.zypper).is_file() {
            "zypper"
        } else {
            "dnf"
        };
        return Some(Package::Rpm(tool));
    }
    None
}

/// Whether the package manager's `tool` says a package owns `path`: it's
/// there, and its query succeeds.
fn owns(tool: &str, query: &str, path: &Path) -> bool {
    Path::new(tool).is_file()
        && Command::new(tool)
            .arg(query)
            .arg(path)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_package_says_its_own_way_to_update() {
        let deb = Package::Deb.note("linux-x64");
        assert!(deb.contains("can't update itself"), "{deb}");
        assert!(
            deb.contains("sudo apt install ./passbundle_<version>_amd64.deb"),
            "{deb}"
        );
        assert!(Package::Deb
            .note("linux-arm64")
            .contains("./passbundle_<version>_arm64.deb"));
        let rpm = Package::Rpm("dnf").note("linux-x64");
        assert!(
            rpm.contains("sudo dnf install ./passbundle-<version>-1.x86_64.rpm"),
            "{rpm}"
        );
        assert!(Package::Rpm("zypper")
            .note("linux-arm64")
            .contains("sudo zypper install ./passbundle-<version>-1.aarch64.rpm"));
        let flatpak = Package::Flatpak.note("linux-x64");
        assert!(
            flatpak.contains("flatpak update com.passbundle.app"),
            "{flatpak}"
        );
    }

    #[test]
    fn a_copy_no_package_installed_has_none() {
        let program = std::env::temp_dir().join(format!(
            "pb-package-{}-{}/passbundle",
            std::process::id(),
            rand::random::<u32>()
        ));
        assert_eq!(installed_by(&program), None);
    }

    /// Stand-ins for the package managers' tools, in a folder of the test's
    /// own: each says it owns `owned` and nothing else, as the real ones do.
    #[cfg(unix)]
    #[test]
    fn the_package_managers_say_which_package_owns_it() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "pb-package-tools-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let owned = Path::new("/opt/passbundle/passbundle");
        let other = Path::new("/opt/someone-else/passbundle");
        let leak = |path: std::path::PathBuf| -> &'static str {
            Box::leak(path.to_string_lossy().into_owned().into_boxed_str())
        };
        let tool = |name: &str, query: &str| -> &'static str {
            let path = dir.join(name);
            std::fs::write(
                &path,
                format!(
                    "#!/bin/sh\n[ \"$1\" = {query} ] && [ \"$2\" = {} ]\n",
                    owned.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            leak(path)
        };
        let none = leak(dir.join("none"));
        let nothing = Places {
            flatpak_info: none,
            dpkg_query: none,
            rpm: none,
            zypper: none,
        };
        assert_eq!(installed_by_in(owned, &nothing), None);

        let debian = Places {
            dpkg_query: tool("dpkg-query", "-S"),
            ..nothing
        };
        assert_eq!(installed_by_in(owned, &debian), Some(Package::Deb));
        assert_eq!(installed_by_in(other, &debian), None);

        let fedora = Places {
            rpm: tool("rpm", "-qf"),
            ..nothing
        };
        assert_eq!(installed_by_in(owned, &fedora), Some(Package::Rpm("dnf")));
        assert_eq!(installed_by_in(other, &fedora), None);
        let zypper = dir.join("zypper");
        std::fs::write(&zypper, b"").unwrap();
        let opensuse = Places {
            zypper: leak(zypper),
            ..fedora
        };
        assert_eq!(
            installed_by_in(owned, &opensuse),
            Some(Package::Rpm("zypper"))
        );

        // Inside a Flatpak, whatever else there is.
        let info = dir.join(".flatpak-info");
        std::fs::write(&info, b"[Application]\nname=com.passbundle.app\n").unwrap();
        let sandbox = Places {
            flatpak_info: leak(info),
            ..debian
        };
        assert_eq!(installed_by_in(other, &sandbox), Some(Package::Flatpak));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
