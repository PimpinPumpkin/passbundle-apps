//! The agent's local socket: the shared `passbundle-ipc` crate (../ipc), with
//! this build's brand. The native-messaging host checks it reached the agent
//! installed next to it, as the app's core does.

use std::{io, path::PathBuf};

pub use passbundle_ipc::{connect, read_frame, write_frame, ClientStream, Listener, ServerStream};

/// Where the agent listens, for the person running this.
pub fn address() -> PathBuf {
    passbundle_ipc::address(crate::brand::NAME, crate::brand::DOMAIN)
}

/// The agent this host was installed with: `passbundle-agent`, next to the
/// host's own program.
pub fn installed_agent() -> io::Result<PathBuf> {
    Ok(passbundle_ipc::agent_next_to(
        &std::env::current_exe()?,
        crate::brand::NAME,
    ))
}

/// Whether the program at the other end is the agent installed next to this host.
pub fn reached_installed_agent(stream: &ClientStream) -> bool {
    installed_agent().is_ok_and(|agent| passbundle_ipc::reached(stream, &agent))
}
