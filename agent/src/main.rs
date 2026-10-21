// The PassBundle desktop agent: a tray app with no windows. See README.md and
// DESIGN.md, "Desktop agent".
//
//   passbundle-agent             runs in the tray (one per signed-in user) and
//                                serves the extension
//   passbundle-agent --serve     serves the extension without a tray (tests)
//   passbundle-agent --update    installs a newer signed release, if there is one
//   passbundle-agent --version

// A tray app: no console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

use passbundle_agent::{biometrics, brand, config, instance, ipc, server::Server, update, version};

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

/// Starts serving the extension on the agent's socket, on its own thread.
fn start_serving() -> Result<std::thread::JoinHandle<()>, ExitCode> {
    let listener = ipc::Listener::bind(&ipc::address()).map_err(|e| {
        eprintln!("can't listen for the extension: {e}");
        ExitCode::FAILURE
    })?;
    let server = Server::new(biometrics::platform());
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
        update::clean_up(&exe);
    }
    if let Err(code) = start_serving() {
        return code;
    }
    passbundle_agent::tray::run(Config::load())
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
    match start_serving() {
        Ok(thread) => {
            let _ = thread.join();
            ExitCode::FAILURE
        }
        Err(code) => code,
    }
}

/// Run by the installer's scheduled task (Windows) or launch daemon (macOS),
/// with the rights to replace the installed agent.
fn run_update() -> ExitCode {
    let config = Config::load();
    if !config.updates {
        println!("Updates are turned off by policy.");
        return ExitCode::SUCCESS;
    }
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            eprintln!("can't find the installed agent: {e}");
            return ExitCode::FAILURE;
        }
    };
    let base = format!("https://{}/agent", brand::DOMAIN);
    match update::run(&base, brand::UPDATE_KEY, Version::current(), &exe) {
        Ok(Some(version)) => {
            println!("Installed {} agent {version}.", brand::NAME);
            ExitCode::SUCCESS
        }
        Ok(None) => {
            println!(
                "{} agent {} is up to date.",
                brand::NAME,
                Version::current()
            );
            ExitCode::SUCCESS
        }
        Err(update::UpdateError::NoKey) => {
            println!("This build doesn't update itself.");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Update failed: {e}");
            ExitCode::FAILURE
        }
    }
}
