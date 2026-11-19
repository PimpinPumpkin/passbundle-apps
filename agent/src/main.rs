// The PassBundle desktop agent: a tray app with no windows. See README.md and
// DESIGN.md, "Desktop agent".
//
//   passbundle-agent             runs in the tray (one per signed-in user) and
//                                serves the extension
//   passbundle-agent --serve     serves the extension without a tray (tests)
//   passbundle-agent --update    installs a newer signed release of the agent,
//                                its host, and the app, if there is one (Windows)
//   passbundle-agent --version

// A tray app: no console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{process::ExitCode, sync::Arc};

use passbundle_agent::{
    biometrics, brand, config, instance, ipc, reminders::Reminders, server::Server, version,
};

use config::Config;
use version::Version;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => run_tray(),
        ["--serve"] => run_serve(),
        ["--version"] => {
            println!("{} agent {}", brand::NAME, Version::current());
            ExitCode::SUCCESS
        }
        ["--update"] => run_update(),
        _ => {
            eprintln!(
                "usage: {} [--serve | --update | --version]",
                env!("CARGO_PKG_NAME")
            );
            ExitCode::from(2)
        }
    }
}

/// Starts serving the extension (and the app) on the agent's socket, on its
/// own thread; the app's expiration reminders go to `reminders`.
fn start_serving(reminders: Arc<Reminders>) -> Result<std::thread::JoinHandle<()>, ExitCode> {
    let listener = ipc::Listener::bind(&ipc::address()).map_err(|e| {
        eprintln!("can't listen for the extension: {e}");
        ExitCode::FAILURE
    })?;
    let server = Server::new(biometrics::platform(), reminders);
    Ok(std::thread::spawn(move || {
        if let Err(e) = server.serve(listener) {
            eprintln!("stopped serving the extension: {e}");
        }
    }))
}

#[cfg(any(windows, target_os = "macos"))]
fn run_tray() -> ExitCode {
    let Some(_lock) = instance::acquire() else {
        // Already running for this user.
        return ExitCode::SUCCESS;
    };
    if let Ok(exe) = std::env::current_exe() {
        passbundle_agent::update::clean_up(&exe);
    }
    let reminders = Arc::new(Reminders::default());
    if let Err(code) = start_serving(Arc::clone(&reminders)) {
        return code;
    }
    passbundle_agent::tray::run(Config::load(), reminders)
}

/// No tray on other platforms: only `--serve`, for tests.
#[cfg(not(any(windows, target_os = "macos")))]
fn run_tray() -> ExitCode {
    run_serve()
}

fn run_serve() -> ExitCode {
    let Some(_lock) = instance::acquire() else {
        eprintln!("{} agent is already running for this user.", brand::NAME);
        return ExitCode::FAILURE;
    };
    match start_serving(Arc::new(Reminders::default())) {
        Ok(thread) => {
            let _ = thread.join();
            ExitCode::FAILURE
        }
        Err(code) => code,
    }
}

/// On Windows, run by the scheduled task the meta repo's installer makes
/// (`Install-PassBundle.ps1`: as SYSTEM, at every start and daily), with the
/// rights to replace the installed folder: the agent, its host, and the app
/// (`update.rs`). Nothing runs it on macOS: the agent is inside the app
/// there, so it's updated with the app. A build with no update key compiled
/// in says so and does nothing.
fn run_update() -> ExitCode {
    let config = Config::load();
    if !config.updates {
        println!("Updates are turned off by policy.");
        return ExitCode::SUCCESS;
    }
    if brand::UPDATE_KEY.is_empty() {
        println!("This build doesn't update itself.");
        return ExitCode::SUCCESS;
    }
    update_installed()
}

#[cfg(not(windows))]
fn update_installed() -> ExitCode {
    println!(
        "{} agent is updated with the app on this computer.",
        brand::NAME
    );
    ExitCode::SUCCESS
}

#[cfg(windows)]
fn update_installed() -> ExitCode {
    use passbundle_agent::{machine::Windows, update, update::Outcome};
    Windows::harden();
    let install = match std::env::current_exe() {
        Ok(exe) => exe
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or(exe),
        Err(e) => {
            eprintln!("can't find the installed agent: {e}");
            return ExitCode::FAILURE;
        }
    };
    let name = brand::NAME;
    match update::run(
        brand::UPDATE_SOURCE,
        brand::UPDATE_KEY,
        &update::target(),
        &install,
        &Windows,
    ) {
        Ok(Outcome::UpToDate(version)) => {
            println!("{name} {version} is up to date.");
            ExitCode::SUCCESS
        }
        Ok(Outcome::Waiting(version)) => {
            println!(
                "{name} {version} is ready. It goes in when nothing runs from {}, such as when the computer next starts.",
                install.display()
            );
            ExitCode::SUCCESS
        }
        Ok(Outcome::Installed(version)) => {
            println!(
                "Installed {name} {version}: the app, the desktop agent, and its browser helper."
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Update failed: {e}");
            ExitCode::FAILURE
        }
    }
}
