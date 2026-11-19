//! The PassBundle desktop agent's code, shared by the tray app
//! (`src/main.rs`) and the native-messaging host (`src/host.rs`). See
//! README.md and DESIGN.md, "Desktop agent".

// Both programs link the C runtime in on Windows, so the update task (as
// SYSTEM) loads no runtime library that could come from PATH
// (`.cargo/config.toml`, which a build from this folder takes). Rustdoc
// doesn't get those flags, and its builds make no program: `cfg(doc)` and
// `cfg(doctest)` are left out, and the library has no doctests to run
// (`doctest = false` in Cargo.toml).
#[cfg(all(
    windows,
    target_env = "msvc",
    not(target_feature = "crt-static"),
    not(doc),
    not(doctest)
))]
compile_error!("build the agent from its own folder, so it takes .cargo/config.toml: on Windows it links the C runtime in (+crt-static)");

pub mod brand {
    include!(concat!(env!("OUT_DIR"), "/brand.rs"));
}
pub mod biometrics;
pub mod channel;
pub mod config;
pub mod instance;
pub mod ipc;
pub mod machine;
pub mod protocol;
pub mod release;
pub mod reminders;
#[cfg_attr(not(windows), allow(dead_code))] // Used by Windows Hello.
pub mod sealed;
pub mod server;
pub mod swap;
#[cfg(any(windows, target_os = "macos"))]
pub mod tray;
pub mod unpack;
pub mod update;
pub mod version;
#[cfg(windows)]
pub mod windows_hello;
