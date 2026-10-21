//! Serves the extension: accepts the native-messaging host's connections on
//! the agent's socket and answers each with [`protocol::Connection`]. Nothing
//! here listens on the network.

use std::{io, sync::Arc, thread};

use serde_json::Value;

use crate::{
    biometrics,
    ipc::{self, read_frame, write_frame},
    protocol::{Biometrics, Connection, Context, PromptGate},
};

/// The first frame a host sends: who started it, for prompt text.
pub const HELLO: &str = "keykeeper.host";

pub struct Server {
    biometrics: Box<dyn Biometrics>,
    gate: PromptGate,
}

impl Server {
    pub fn new(biometrics: Box<dyn Biometrics>) -> Arc<Self> {
        Arc::new(Self {
            biometrics,
            gate: PromptGate::default(),
        })
    }

    /// Accepts connections until the listener fails; each gets a thread.
    pub fn serve(self: Arc<Self>, listener: ipc::Listener) -> io::Result<()> {
        loop {
            let stream = listener.accept()?;
            let server = Arc::clone(&self);
            thread::spawn(move || {
                if let Err(e) = server.handle(stream) {
                    eprintln!("connection ended: {e}");
                }
            });
        }
    }

    fn handle(&self, mut stream: ipc::ServerStream) -> io::Result<()> {
        let mut browser = String::from("your browser");
        let mut connection: Option<Connection> = None;
        while let Some(frame) = read_frame(&mut stream)? {
            let Ok(text) = std::str::from_utf8(&frame) else {
                continue;
            };
            if connection.is_none() {
                if let Some(name) = hello(text) {
                    browser = name;
                    continue;
                }
                connection = Some(Connection::new(
                    self.biometrics.as_ref(),
                    &self.gate,
                    Context {
                        browser: browser.clone(),
                        method: biometrics::METHOD,
                    },
                ));
            }
            if let Some(reply) = connection.as_mut().unwrap().handle(text) {
                write_frame(&mut stream, reply.as_bytes())?;
            }
        }
        Ok(())
    }
}

/// The browser's name from a host's hello, if this frame is one.
fn hello(text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    let name = value.get(HELLO)?.get("browser")?.as_str()?;
    // Prompt text only: a short plain name.
    let name: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ')
        .take(32)
        .collect();
    (!name.trim().is_empty()).then(|| name.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hello_names_the_browser_in_plain_words() {
        assert_eq!(
            hello(r#"{"keykeeper.host":{"browser":"Firefox"}}"#).as_deref(),
            Some("Firefox")
        );
        assert_eq!(
            hello(r#"{"keykeeper.host":{"browser":"Edge\u202e<script>"}}"#).as_deref(),
            Some("Edgescript")
        );
        assert_eq!(hello(r#"{"keykeeper.host":{"browser":"  "}}"#), None);
        assert_eq!(hello(r#"{"appId":"x","message":{}}"#), None);
    }
}
