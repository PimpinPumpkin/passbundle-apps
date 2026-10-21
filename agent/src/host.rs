// The native-messaging host: what the browser starts when the extension
// connects. A pipe between the browser (stdin and stdout) and the agent's
// socket, like upstream's `desktop_proxy`: it says `connected` once it reaches
// the agent and `disconnected` when the agent goes away, and exits when either
// side closes. It holds no keys and decides nothing.
//
// Browsers start it with the caller's origin (Chromium: `chrome-extension://
// <id>/`; Firefox: the manifest's path and the extension's id), and the
// manifest that names it allows only our extension.

use std::{
    io::{self, Write},
    process::ExitCode,
    thread,
};

use passbundle_agent::{
    ipc::{self, read_frame, write_frame},
    server::HELLO,
};
use serde_json::json;

fn main() -> ExitCode {
    let Ok(agent) = ipc::connect(&ipc::address()) else {
        // As upstream: no agent, no host. The extension sees the port close
        // and reports the desktop app as not running.
        return ExitCode::FAILURE;
    };
    // Only the agent installed next to this host, never a program that made
    // the pipe or socket first: it would get the user key on enrollment.
    if !ipc::reached_installed_agent(&agent) {
        return ExitCode::FAILURE;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let browser = if args.iter().any(|a| a.starts_with("chrome-extension://")) {
        "your browser"
    } else if args.len() >= 2 {
        "Firefox"
    } else {
        "your browser"
    };

    // Windows Hello prompts open behind the browser unless the agent may
    // bring them to the front. The browser started this host, so the host
    // may pass that right on.
    #[cfg(windows)]
    let agent_pid = agent.server_process_id();
    let mut to_agent = agent.try_clone().expect("socket handle");
    let mut from_agent = agent;
    if write_frame(
        &mut to_agent,
        json!({ HELLO: { "browser": browser } })
            .to_string()
            .as_bytes(),
    )
    .is_err()
    {
        return ExitCode::FAILURE;
    }
    let mut stdout = io::stdout().lock();
    if write_frame(&mut stdout, br#"{"command":"connected"}"#).is_err() {
        return ExitCode::FAILURE;
    }
    drop(stdout);

    // Browser to agent, on its own thread; the process ends when either
    // direction does.
    thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        while let Ok(Some(frame)) = read_frame(&mut stdin) {
            #[cfg(windows)]
            if let Some(pid) = agent_pid {
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid)
                };
            }
            if write_frame(&mut to_agent, &frame).is_err() {
                break;
            }
        }
        std::process::exit(0);
    });

    let mut stdout = io::stdout().lock();
    while let Ok(Some(frame)) = read_frame(&mut from_agent) {
        if write_frame(&mut stdout, &frame).is_err() {
            return ExitCode::SUCCESS;
        }
    }
    let _ = write_frame(&mut stdout, br#"{"command":"disconnected"}"#);
    let _ = stdout.flush();
    ExitCode::SUCCESS
}
