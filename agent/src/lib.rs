//! The PassBundle desktop agent's code, shared by the tray app
//! (`src/main.rs`) and the native-messaging host (`src/host.rs`). See
//! README.md and DESIGN.md, "Desktop agent".

pub mod brand {
    include!(concat!(env!("OUT_DIR"), "/brand.rs"));
}
pub mod biometrics;
pub mod channel;
pub mod config;
pub mod instance;
pub mod ipc;
pub mod protocol;
#[cfg_attr(not(windows), allow(dead_code))] // Used by Windows Hello.
pub mod sealed;
pub mod server;
#[cfg(any(windows, target_os = "macos"))]
pub mod tray;
pub mod update;
pub mod version;
#[cfg(windows)]
pub mod windows_hello;
