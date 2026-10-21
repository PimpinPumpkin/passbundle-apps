//! Unlocking with Windows Hello or Touch ID through the desktop agent
//! (docs/core.md, "Unlock with Windows Hello or Touch ID"). The app is one
//! more client of the agent, on the agent's own socket, speaking the protocol
//! the browser extension does, and sharing its enrollment. It sends nothing
//! until it knows it reached the agent installed next to the app.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD, Engine};
use passbundle_ipc::{read_frame, write_frame, ClientStream};
use serde_json::{json, Value};
use zeroize::{Zeroize, Zeroizing};

use crate::crypto::{EncString, PrivateKey, SymmetricKey};
use crate::error::{Error, Result};

/// What the agent says about unlocking an account this way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Turned on for this account: the app can offer it.
    SetUp,
    /// Available here, but not turned on for this account.
    NotSetUp,
    /// No Windows Hello or Touch ID on this computer, or not set up in the OS.
    Unavailable,
}

/// How the agent's prompts name the app ("Unlock PassBundle in the PassBundle app").
const CALLER: &str = "the PassBundle app";
/// Both sides drop a message more than 10 seconds from their clock.
const VALID_MS: i64 = 10_000;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn lost(e: std::io::Error) -> Error {
    Error::Network(format!("the desktop agent stopped answering ({e})"))
}

/// Overwrites a user key in a message before it's dropped.
fn forget_key(message: &mut Value) {
    if let Some(Value::String(key)) = message.get_mut("userKeyB64") {
        key.zeroize();
    }
}

pub struct Agent {
    stream: ClientStream,
    app_id: String,
    session: SymmetricKey,
    user_id: String,
    next_message: i64,
}

impl Agent {
    /// Where this person's agent listens, and the agent installed next to this program.
    pub fn installed() -> Result<(PathBuf, PathBuf)> {
        let program = std::env::current_exe().map_err(|_| Error::NoAgent)?;
        Ok((
            passbundle_ipc::address(crate::brand::NAME, crate::brand::DOMAIN),
            passbundle_ipc::agent_next_to(&program, crate::brand::NAME),
        ))
    }

    /// Starts the agent installed next to this program when it isn't running,
    /// so its tray is there (and Windows Hello with it). It keeps running when
    /// the app closes; only one runs per person. Returns whether it started one.
    pub fn start_installed() -> Result<bool> {
        let (address, program) = Agent::installed()?;
        if passbundle_ipc::connect(&address).is_ok() {
            return Ok(false);
        }
        if !program.is_file() {
            return Err(Error::NoAgent);
        }
        std::process::Command::new(&program)
            .spawn()
            .map(|_| true)
            .map_err(|e| Error::Network(format!("the desktop agent didn't start ({e})")))
    }

    /// Connects to the agent at `address`, which must be the program at
    /// `expected`, and sets up the encrypted channel for `user_id`.
    pub fn connect(address: &Path, expected: &Path, user_id: &str) -> Result<Agent> {
        let mut stream = passbundle_ipc::connect(address).map_err(|_| Error::NoAgent)?;
        if !passbundle_ipc::reached(&stream, expected) {
            return Err(Error::Refused(
                "the program there isn't PassBundle's desktop agent",
            ));
        }
        let hello = json!({ "keykeeper.host": { "browser": CALLER } });
        write_frame(&mut stream, hello.to_string().as_bytes()).map_err(lost)?;

        // The key exchange: a new RSA key; the agent's 64-byte session key comes back wrapped with it.
        let key = PrivateKey::generate()?;
        let app_id = uuid::Uuid::new_v4().to_string();
        let setup = json!({
            "appId": app_id,
            "message": {
                "command": "setupEncryption",
                "publicKey": STANDARD.encode(key.public_key_der()),
                "userId": user_id,
                "timestamp": now_ms(),
            },
        });
        write_frame(&mut stream, setup.to_string().as_bytes()).map_err(lost)?;
        let session = loop {
            let frame = read_frame(&mut stream)
                .map_err(lost)?
                .ok_or(Error::NoAgent)?;
            let reply: Value = serde_json::from_slice(&frame)
                .map_err(|_| Error::Damaged("an answer from the agent"))?;
            if reply["appId"] != app_id.as_str() || reply["command"] != "setupEncryption" {
                continue;
            }
            let wrapped = reply["sharedSecret"]
                .as_str()
                .ok_or(Error::Damaged("the agent's session key"))?;
            let data = STANDARD
                .decode(wrapped)
                .map_err(|_| Error::Damaged("the agent's session key"))?;
            break SymmetricKey::from_bytes(&key.decrypt(&EncString::Rsa {
                sha256: false,
                data,
            })?)?;
        };
        Ok(Agent {
            stream,
            app_id,
            session,
            user_id: user_id.to_owned(),
            next_message: 0,
        })
    }

    /// One command over the channel, and the agent's answer to it.
    fn call(&mut self, command: &str, extra: Value) -> Result<Value> {
        self.next_message += 1;
        let id = self.next_message;
        let mut message = json!({ "command": command, "messageId": id, "userId": self.user_id, "timestamp": now_ms() });
        if let (Some(m), Value::Object(extra)) = (message.as_object_mut(), extra) {
            m.extend(extra);
        }
        let plain = Zeroizing::new(message.to_string());
        forget_key(&mut message);
        let encrypted = self.session.encrypt(plain.as_bytes()).to_string();
        let frame = json!({ "appId": self.app_id, "messageId": id, "message": { "encryptedString": encrypted } });
        write_frame(&mut self.stream, frame.to_string().as_bytes()).map_err(lost)?;
        loop {
            let frame = read_frame(&mut self.stream)
                .map_err(lost)?
                .ok_or(Error::NoAgent)?;
            let outer: Value = serde_json::from_slice(&frame)
                .map_err(|_| Error::Damaged("an answer from the agent"))?;
            if outer["command"] == "invalidateEncryption" {
                return Err(Error::Refused(
                    "the desktop agent ended the encrypted session",
                ));
            }
            if outer["appId"] != self.app_id.as_str() || outer["messageId"] != id {
                continue;
            }
            let enc = outer["message"]["encryptedString"]
                .as_str()
                .ok_or(Error::Damaged("an answer from the agent"))?;
            let text = EncString::parse(enc)?.decrypt_string(&self.session)?;
            let inner: Value = serde_json::from_str(&text)
                .map_err(|_| Error::Damaged("an answer from the agent"))?;
            let fresh = inner["timestamp"]
                .as_i64()
                .is_some_and(|t| (now_ms() - t).abs() <= VALID_MS);
            if inner["messageId"] != id || !fresh {
                return Err(Error::Refused(
                    "an answer from the agent was stale or for another request",
                ));
            }
            return Ok(inner);
        }
    }

    pub fn status(&mut self) -> Result<Status> {
        let reply = self.call("getBiometricsStatusForUser", json!({}))?;
        Ok(match reply["response"].as_i64() {
            Some(0) => Status::SetUp,
            // Not turned on for this account (upstream's "not enabled in the connected desktop app").
            Some(8) => Status::NotSetUp,
            _ => Status::Unavailable,
        })
    }

    /// Turns it on for this account: the agent asks for Windows Hello or Touch
    /// ID, then keeps the user key sealed. Returns whether the person agreed.
    pub fn enroll(&mut self, user_key: &SymmetricKey, email: &str) -> Result<bool> {
        let key_b64 = Zeroizing::new(STANDARD.encode(&*user_key.to_bytes()));
        let mut extra = json!({ "userKeyB64": key_b64.as_str(), "email": email });
        let reply = self.call("keykeeper.biometric.enroll", extra.clone());
        forget_key(&mut extra);
        Ok(reply?["response"] == true)
    }

    /// Asks for Windows Hello or Touch ID and returns the user key, or None if
    /// the person cancelled (or it isn't turned on for this account).
    pub fn unlock(&mut self) -> Result<Option<SymmetricKey>> {
        let mut reply = self.call("unlockWithBiometricsForUser", json!({}))?;
        let key = match (reply["response"] == true, reply["userKeyB64"].as_str()) {
            (true, Some(b64)) => {
                let bytes = Zeroizing::new(
                    STANDARD
                        .decode(b64)
                        .map_err(|_| Error::Damaged("the key from the agent"))?,
                );
                Some(SymmetricKey::from_bytes(&bytes)?)
            }
            _ => None,
        };
        forget_key(&mut reply);
        Ok(key)
    }

    /// Turns it off for this account (for the extension too: it's one enrollment).
    pub fn remove(&mut self) -> Result<()> {
        self.call("keykeeper.biometric.remove", json!({}))
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The protocol's vectors (agent/tests/vectors/native-messaging.json),
    /// which upstream's own crypto also checks: the core's half of the channel
    /// makes and opens exactly what the agent and the extension do.
    #[test]
    fn speaks_the_channel_as_the_vectors_say() {
        let v: Value = serde_json::from_str(include_str!(
            "../../agent/tests/vectors/native-messaging.json"
        ))
        .unwrap();
        let b64 = |v: &Value| STANDARD.decode(v.as_str().unwrap()).unwrap();
        let session = SymmetricKey::from_bytes(&b64(&v["sessionKey"])).unwrap();
        for m in v["messages"].as_array().unwrap() {
            let enc = m["encString"].as_str().unwrap();
            assert_eq!(
                &*EncString::parse(enc)
                    .unwrap()
                    .decrypt_string(&session)
                    .unwrap(),
                m["plaintext"].as_str().unwrap()
            );
            let iv: [u8; 16] = b64(&m["iv"]).try_into().unwrap();
            assert_eq!(
                session
                    .encrypt_with_iv(m["plaintext"].as_str().unwrap().as_bytes(), iv)
                    .to_string(),
                enc
            );
        }
        let private = PrivateKey::from_pkcs8_der(&b64(&v["rsa"]["privateKeyPkcs8"])).unwrap();
        assert_eq!(private.public_key_der(), b64(&v["rsa"]["publicKeySpki"]));
        let unwrapped = private
            .decrypt(&EncString::Rsa {
                sha256: false,
                data: b64(&v["rsa"]["wrappedSessionKey"]),
            })
            .unwrap();
        assert_eq!(*unwrapped, b64(&v["sessionKey"]));
    }
}
