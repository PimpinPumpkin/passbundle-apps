//! The agent and the native-messaging host as separate processes, over the
//! real socket (a named pipe on Windows), with this test as the extension:
//! the host says `connected`, the key exchange works, a key enrolls and comes
//! back only after a prompt, and the host says `disconnected` when the agent
//! goes away. Needs the stand-in authenticator:
//! `cargo test --features test-authenticator`.
#![cfg(feature = "test-authenticator")]

use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use passbundle_agent::channel::SessionKey;
use rsa::{
    pkcs8::{DecodePrivateKey, EncodePublicKey},
    Oaep, RsaPrivateKey,
};
use serde_json::{json, Value};

const USER: &str = "3f1c2a64-8d0e-4b7f-9a55-6c2e1d0b9f10";

/// A home folder of its own, so the test's agent never meets a real one.
fn home() -> PathBuf {
    let base = if cfg!(unix) {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    let dir = base.join(format!("pb-host-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn command(program: &str, home: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("LOCALAPPDATA", home)
        .env("PASSBUNDLE_AGENT_TEST_AUTHENTICATOR", home);
    command
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn write_frame(stdin: &mut ChildStdin, value: &Value) {
    let bytes = value.to_string().into_bytes();
    stdin
        .write_all(&(bytes.len() as u32).to_ne_bytes())
        .unwrap();
    stdin.write_all(&bytes).unwrap();
    stdin.flush().unwrap();
}

fn read_frame(stdout: &mut ChildStdout) -> Value {
    let mut length = [0u8; 4];
    stdout.read_exact(&mut length).unwrap();
    let mut frame = vec![0u8; u32::from_ne_bytes(length) as usize];
    stdout.read_exact(&mut frame).unwrap();
    serde_json::from_slice(&frame).unwrap()
}

struct Extension {
    stdin: ChildStdin,
    stdout: ChildStdout,
    session: Option<SessionKey>,
    message_id: i64,
}

impl Extension {
    fn call(&mut self, mut message: Value) -> Value {
        self.message_id += 1;
        message["messageId"] = json!(self.message_id);
        message["userId"] = json!(USER);
        message["timestamp"] = json!(now());
        let session = self.session.as_ref().unwrap();
        write_frame(
            &mut self.stdin,
            &json!({ "appId": "app-1", "message": { "encryptedString": session.encrypt(&message.to_string()), "encryptionType": 2 } }),
        );
        let reply = read_frame(&mut self.stdout);
        assert_eq!(reply["messageId"], self.message_id);
        let inner = session
            .decrypt(reply["message"]["encryptedString"].as_str().unwrap())
            .unwrap();
        serde_json::from_str(&inner).unwrap()
    }
}

struct Agent(Child);

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn the_extension_enrolls_and_unlocks_through_the_host_and_the_agent() {
    let home = home();
    fs::write(home.join("answers"), "yes\nyes\nno\n").unwrap();
    let mut agent = Agent(
        command(env!("CARGO_BIN_EXE_passbundle-agent"), &home)
            .arg("--serve")
            .spawn()
            .unwrap(),
    );

    // The browser starts the host with the extension's origin.
    let start = Instant::now();
    let mut host = loop {
        let mut host = command(env!("CARGO_BIN_EXE_passbundle-agent-host"), &home)
            .arg("chrome-extension://abcdefghijklmnopabcdefghijklmnop/")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        // Until the agent is listening, the host exits at once.
        thread::sleep(Duration::from_millis(200));
        if host.try_wait().unwrap().is_none() {
            break host;
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "the agent never started listening"
        );
    };
    let mut extension = Extension {
        stdin: host.stdin.take().unwrap(),
        stdout: host.stdout.take().unwrap(),
        session: None,
        message_id: 0,
    };
    assert_eq!(
        read_frame(&mut extension.stdout),
        json!({ "command": "connected" })
    );

    // The key exchange, with the vectors' fixed test key.
    let vectors: Value =
        serde_json::from_str(include_str!("vectors/native-messaging.json")).unwrap();
    let private_key = RsaPrivateKey::from_pkcs8_der(
        &STANDARD
            .decode(vectors["rsa"]["privateKeyPkcs8"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap();
    let public_key = private_key.to_public_key().to_public_key_der().unwrap();
    write_frame(
        &mut extension.stdin,
        &json!({ "appId": "app-1", "message": {
            "command": "setupEncryption",
            "publicKey": STANDARD.encode(public_key.as_bytes()),
            "userId": USER, "messageId": 0, "timestamp": now(),
        } }),
    );
    let reply = read_frame(&mut extension.stdout);
    assert_eq!(reply["command"], "setupEncryption");
    let shared = private_key
        .decrypt(
            Oaep::new::<sha1::Sha1>(),
            &STANDARD
                .decode(reply["sharedSecret"].as_str().unwrap())
                .unwrap(),
        )
        .unwrap();
    extension.session = Some(SessionKey::from_test_bytes(&shared));

    let user_key = STANDARD.encode((0u8..64).collect::<Vec<_>>());
    let status = extension.call(json!({ "command": "getBiometricsStatusForUser" }));
    assert_eq!(status["response"], 8, "not enrolled yet");
    let enrolled = extension.call(json!({
        "command": "keykeeper.biometric.enroll", "userKeyB64": user_key, "email": "ana@example.test",
    }));
    assert_eq!(enrolled["response"], true);
    assert_eq!(
        extension.call(json!({ "command": "getBiometricsStatusForUser" }))["response"],
        0
    );
    let unlocked = extension.call(json!({ "command": "unlockWithBiometricsForUser" }));
    assert_eq!(unlocked["response"], true);
    assert_eq!(unlocked["userKeyB64"], user_key);
    // The third prompt is canceled: nothing comes back.
    let canceled = extension.call(json!({ "command": "unlockWithBiometricsForUser" }));
    assert_eq!(canceled["response"], false);
    assert!(canceled.get("userKeyB64").is_none());

    let prompts = fs::read_to_string(home.join("prompts")).unwrap();
    let prompts: Vec<&str> = prompts.lines().collect();
    assert_eq!(prompts.len(), 3);
    assert!(prompts[0].starts_with("Turn on unlock with "));
    assert!(prompts[0].ends_with(" for ana@example.test in your browser"));
    assert!(prompts[1].starts_with("Unlock ") && prompts[1].ends_with(" in your browser"));

    // The agent goes away: the host says so and exits.
    agent.0.kill().unwrap();
    agent.0.wait().unwrap();
    assert_eq!(
        read_frame(&mut extension.stdout),
        json!({ "command": "disconnected" })
    );
    let exit = host.wait().unwrap();
    assert!(exit.success());
    let _ = fs::remove_dir_all(home);
}
