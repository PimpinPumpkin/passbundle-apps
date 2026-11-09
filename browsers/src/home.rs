//! Where each browser keeps its profiles, for the person running PassBundle:
//! only their own folders, never another person's on the same computer.

use std::path::{Path, PathBuf};

use crate::{Browser, Platform};

/// The person's own folders that browsers keep profiles in.
#[derive(Clone, Debug)]
pub struct Home {
    /// Whose layout: this computer's, except in tests.
    pub platform: Platform,
    /// The home folder.
    pub home: PathBuf,
    /// Windows: `%LOCALAPPDATA%`. macOS: `~/Library/Application Support`.
    /// Linux: `$XDG_CONFIG_HOME` (`~/.config`).
    pub app_data: PathBuf,
    /// Windows: `%APPDATA%` (Roaming), where Firefox and Opera keep theirs.
    /// Elsewhere the same as `app_data`.
    pub roaming: PathBuf,
}

impl Home {
    /// This person's, as the OS says.
    pub fn this_person() -> Option<Home> {
        let home = dirs::home_dir()?;
        let platform = Platform::this();
        let (app_data, roaming) = match platform {
            Platform::Windows => (dirs::data_local_dir()?, dirs::data_dir()?),
            Platform::Mac => {
                let support = home.join("Library").join("Application Support");
                (support.clone(), support)
            }
            Platform::Linux => {
                let config = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
                (config.clone(), config)
            }
        };
        Some(Home {
            platform,
            home,
            app_data,
            roaming,
        })
    }

    /// A made-up home inside `root`, laid out as `platform` lays one out:
    /// for tests, which never look at the real one.
    pub fn inside(root: &Path, platform: Platform) -> Home {
        let home = root.join("home");
        let (app_data, roaming) = match platform {
            Platform::Windows => (
                home.join("AppData").join("Local"),
                home.join("AppData").join("Roaming"),
            ),
            Platform::Mac => {
                let support = home.join("Library").join("Application Support");
                (support.clone(), support)
            }
            Platform::Linux => (home.join(".config"), home.join(".config")),
        };
        Home {
            platform,
            home,
            app_data,
            roaming,
        }
    }

    /// The folder a Chromium browser keeps its profiles and Local State in,
    /// where it has one.
    pub(crate) fn chromium_dirs(&self, browser: Browser) -> Vec<PathBuf> {
        let parts: &[&str] = match (self.platform, browser) {
            (Platform::Windows, Browser::Chrome) => &["Google", "Chrome", "User Data"],
            (Platform::Windows, Browser::Edge) => &["Microsoft", "Edge", "User Data"],
            (Platform::Windows, Browser::Brave) => &["BraveSoftware", "Brave-Browser", "User Data"],
            (Platform::Windows, Browser::Vivaldi) => &["Vivaldi", "User Data"],
            (Platform::Windows, Browser::Chromium) => &["Chromium", "User Data"],
            (Platform::Windows, Browser::Opera) => &["Opera Software", "Opera Stable"],
            (Platform::Mac, Browser::Chrome) => &["Google", "Chrome"],
            (Platform::Mac, Browser::Edge) => &["Microsoft Edge"],
            (Platform::Mac, Browser::Brave) => &["BraveSoftware", "Brave-Browser"],
            (Platform::Mac, Browser::Vivaldi) => &["Vivaldi"],
            (Platform::Mac, Browser::Chromium) => &["Chromium"],
            (Platform::Mac, Browser::Opera) => &["com.operasoftware.Opera"],
            (Platform::Linux, Browser::Chrome) => &["google-chrome"],
            (Platform::Linux, Browser::Edge) => &["microsoft-edge"],
            (Platform::Linux, Browser::Brave) => &["BraveSoftware", "Brave-Browser"],
            (Platform::Linux, Browser::Vivaldi) => &["vivaldi"],
            (Platform::Linux, Browser::Chromium) => &["chromium"],
            (Platform::Linux, Browser::Opera) => &["opera"],
            _ => return Vec::new(),
        };
        // Opera keeps its folder with the roaming data on Windows.
        let base = if self.platform == Platform::Windows && browser == Browser::Opera {
            &self.roaming
        } else {
            &self.app_data
        };
        let dir = parts.iter().fold(base.clone(), |dir, part| dir.join(part));
        if dir.is_dir() {
            vec![dir]
        } else {
            Vec::new()
        }
    }

    /// The folders Firefox keeps `profiles.ini` in: its own, and on Linux
    /// also where the newer releases, the Snap, and the Flatpak keep it.
    pub(crate) fn firefox_dirs(&self) -> Vec<PathBuf> {
        let candidates = match self.platform {
            Platform::Windows => vec![self.roaming.join("Mozilla").join("Firefox")],
            Platform::Mac => vec![self.app_data.join("Firefox")],
            Platform::Linux => vec![
                self.home.join(".mozilla").join("firefox"),
                self.app_data.join("mozilla").join("firefox"),
                self.home
                    .join("snap")
                    .join("firefox")
                    .join("common")
                    .join(".mozilla")
                    .join("firefox"),
                self.home
                    .join(".var")
                    .join("app")
                    .join("org.mozilla.firefox")
                    .join(".mozilla")
                    .join("firefox"),
            ],
        };
        candidates
            .into_iter()
            .filter(|d| d.join("profiles.ini").is_file())
            .collect()
    }

    /// `path` as a profile's id names it: from the home folder, with `/`.
    pub(crate) fn id_path(&self, path: &Path) -> String {
        let relative = path.strip_prefix(&self.home).unwrap_or(path);
        relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/")
    }
}
