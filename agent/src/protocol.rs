//! What the agent says to the browser extension: upstream's native-messaging
//! protocol for biometric unlock (as upstream's desktop app speaks it in
//! `apps/desktop/src/services/biometric-message-handler.service.ts`), plus our
//! two enrollment commands. Pure logic: the OS prompts and key storage are
//! behind [`Biometrics`], so every rule here is tested without them. See
//! DESIGN.md, "Biometric unlock".

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use zeroize::{Zeroize, Zeroizing};

use crate::channel::SessionKey;

/// Both sides ignore messages more than this far from their own clock.
const MESSAGE_VALID_MS: i64 = 10_000;
/// After someone cancels a prompt, no new prompt for this long, so a program
/// can't nag them into approving.
const AFTER_CANCEL: Duration = Duration::from_secs(10);
/// The user keys the extension can hand over: 64 bytes for upstream's
/// AES-CBC-HMAC keys, more for its newer COSE keys.
const USER_KEY_BYTES: std::ops::RangeInclusive<usize> = 32..=1024;

/// Upstream's `BiometricsStatus` (libs/key-management/src/biometrics/biometrics-status.ts).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Every value is part of the protocol; platforms use some.
pub enum BiometricsStatus {
    Available = 0,
    UnlockNeeded = 1,
    HardwareUnavailable = 2,
    AutoSetupNeeded = 3,
    ManualSetupNeeded = 4,
    PlatformUnsupported = 5,
    DesktopDisconnected = 6,
    NotEnabledLocally = 7,
    NotEnabledInConnectedDesktopApp = 8,
    NativeMessagingPermissionMissing = 9,
}

/// Overwrites the user key in a message or reply before it's dropped.
fn forget_key(message: &mut Value) {
    if let Some(Value::String(key)) = message.get_mut("userKeyB64") {
        key.zeroize();
    }
}

/// The person's answer to a prompt.
#[derive(Debug, PartialEq, Eq)]
pub enum Answer<T> {
    Approved(T),
    Canceled,
    /// The prompt couldn't be shown or didn't finish: a locked or busy
    /// device, say. The stored key stays; the next try may work.
    Failed(String),
    /// The stored key can't be opened and never will be: Windows Hello was
    /// set up again, the fingerprints changed, or the stored key is damaged.
    Lost(String),
}

/// Windows Hello or Touch ID, and where enrolled keys are kept.
pub trait Biometrics: Send + Sync {
    /// Whether this computer can prompt at all.
    fn status(&self) -> BiometricsStatus;
    fn is_enrolled(&self, user_id: &str) -> bool;
    /// Prompts with `reason`; stores `user_key` only if approved.
    fn enroll(&self, user_id: &str, user_key: &[u8], reason: &str) -> Answer<()>;
    /// Prompts with `reason`; returns the stored key if approved.
    fn unlock(&self, user_id: &str, reason: &str) -> Answer<Zeroizing<Vec<u8>>>;
    fn authenticate(&self, reason: &str) -> Answer<()>;
    fn remove(&self, user_id: &str);
}

/// One prompt at a time across every connection, and a pause after a cancel.
#[derive(Default)]
pub struct PromptGate {
    last_cancel: Mutex<Option<Instant>>,
    showing: Mutex<()>,
}

impl PromptGate {
    fn prompt<T>(&self, now: Instant, show: impl FnOnce() -> Answer<T>) -> Answer<T> {
        if let Some(at) = *self.last_cancel.lock().unwrap() {
            if now.saturating_duration_since(at) < AFTER_CANCEL {
                return Answer::Canceled;
            }
        }
        // Waits for a prompt already showing, rather than stacking two.
        let _showing = self.showing.lock().unwrap();
        let answer = show();
        if matches!(
            answer,
            Answer::Canceled | Answer::Failed(_) | Answer::Lost(_)
        ) {
            *self.last_cancel.lock().unwrap() = Some(Instant::now());
        }
        answer
    }
}

/// What the agent knows about who it's talking to, for prompt text.
pub struct Context {
    /// The browser that started the host, such as "Firefox", or "your browser".
    pub browser: String,
    /// "Windows Hello" or "Touch ID".
    pub method: &'static str,
}

/// One connection from the extension (one native-messaging host process).
pub struct Connection<'a> {
    biometrics: &'a dyn Biometrics,
    gate: &'a PromptGate,
    context: Context,
    /// Session keys by the extension's `appId`.
    sessions: HashMap<String, SessionKey>,
    now_ms: Box<dyn Fn() -> i64 + Send + 'a>,
}

impl<'a> Connection<'a> {
    pub fn new(biometrics: &'a dyn Biometrics, gate: &'a PromptGate, context: Context) -> Self {
        Self {
            biometrics,
            gate,
            context,
            sessions: HashMap::new(),
            now_ms: Box::new(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0)
            }),
        }
    }

    #[cfg(test)]
    fn with_clock(mut self, now_ms: impl Fn() -> i64 + Send + 'a) -> Self {
        self.now_ms = Box::new(now_ms);
        self
    }

    /// Handles one frame from the extension. Returns the frame to send back,
    /// if any.
    pub fn handle(&mut self, frame: &str) -> Option<String> {
        let outer: Value = serde_json::from_str(frame).ok()?;
        // The extension's second port carries the SDK's own IPC, which the
        // agent doesn't speak (DESIGN.md: not upstream's "shared unlock").
        if outer.get("type").is_some() {
            return None;
        }
        let app_id = outer.get("appId")?.as_str()?.to_owned();
        let message = outer.get("message")?;
        if message.get("command").and_then(Value::as_str) == Some("setupEncryption") {
            return self.setup_encryption(app_id, message);
        }
        let encrypted = message.get("encryptedString")?.as_str()?;
        let Some(session) = self.sessions.get(&app_id) else {
            return Some(invalidate(&app_id));
        };
        // Messages can carry the user key: every copy here is overwritten
        // before it's dropped (serde_json's own buffers are gone by then).
        let Ok(plaintext) = session.decrypt(encrypted).map(Zeroizing::new) else {
            self.sessions.remove(&app_id);
            return Some(invalidate(&app_id));
        };
        let mut inner: Value = serde_json::from_str(&plaintext).ok()?;
        if !self.fresh(&inner) {
            return None;
        }
        let message_id = inner.get("messageId")?.as_i64()?;
        let command = inner.get("command")?.as_str()?.to_owned();
        let user_id = inner
            .get("userId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let reply = self.command(&command, &user_id, &inner);
        forget_key(&mut inner);
        let mut reply = reply?;
        reply.insert("command".into(), json!(command));
        reply.insert("messageId".into(), json!(message_id));
        // Stamped as it's sent: a prompt can take most of the extension's
        // 60 seconds, and it drops replies more than 10 seconds old.
        reply.insert("timestamp".into(), json!((self.now_ms)()));
        let session = self.sessions.get(&app_id)?;
        let mut reply = Value::Object(reply);
        let plain = Zeroizing::new(reply.to_string());
        forget_key(&mut reply);
        let enc_string = session.encrypt(&plain);
        Some(
            json!({
                "appId": app_id,
                "messageId": message_id,
                "message": enc_string_object(&enc_string),
            })
            .to_string(),
        )
    }

    fn fresh(&self, message: &Value) -> bool {
        message
            .get("timestamp")
            .and_then(Value::as_i64)
            .is_some_and(|t| ((self.now_ms)() - t).abs() <= MESSAGE_VALID_MS)
    }

    fn setup_encryption(&mut self, app_id: String, message: &Value) -> Option<String> {
        if !self.fresh(message) {
            return None;
        }
        let public_key = STANDARD.decode(message.get("publicKey")?.as_str()?).ok()?;
        let session = SessionKey::generate();
        let shared_secret = session.wrap_for(&public_key).ok()?;
        // Any user may set up a channel: enrolling needs one, and nothing is
        // returned without a prompt. (Upstream answers `wrongUserId` for an
        // account its desktop app doesn't have; the agent has none.)
        self.sessions.insert(app_id.clone(), session);
        Some(
            json!({
                "appId": app_id,
                "command": "setupEncryption",
                "messageId": -1,
                "sharedSecret": STANDARD.encode(shared_secret),
            })
            .to_string(),
        )
    }

    fn command(
        &self,
        command: &str,
        user_id: &str,
        message: &Value,
    ) -> Option<serde_json::Map<String, Value>> {
        let mut reply = serde_json::Map::new();
        let now = Instant::now();
        let (browser, method) = (self.context.browser.as_str(), self.context.method);
        match command {
            "getBiometricsStatus" => {
                reply.insert("response".into(), json!(self.biometrics.status() as i32));
            }
            "getBiometricsStatusForUser" => {
                let status = match self.biometrics.status() {
                    BiometricsStatus::Available if !self.biometrics.is_enrolled(user_id) => {
                        BiometricsStatus::NotEnabledInConnectedDesktopApp
                    }
                    status => status,
                };
                reply.insert("response".into(), json!(status as i32));
            }
            "authenticateWithBiometrics" => {
                let reason = format!("Confirm it's you in {browser}");
                let answer = self
                    .gate
                    .prompt(now, || self.biometrics.authenticate(&reason));
                reply.insert(
                    "response".into(),
                    json!(matches!(answer, Answer::Approved(()))),
                );
            }
            "unlockWithBiometricsForUser" => {
                let mut unlocked = false;
                if self.biometrics.is_enrolled(user_id) {
                    let reason = format!("Unlock {} in {browser}", crate::brand::NAME);
                    match self
                        .gate
                        .prompt(now, || self.biometrics.unlock(user_id, &reason))
                    {
                        Answer::Approved(key) => {
                            reply.insert("userKeyB64".into(), json!(STANDARD.encode(&*key)));
                            unlocked = true;
                        }
                        // Nothing lost: the person can try again.
                        Answer::Canceled | Answer::Failed(_) => {}
                        // The stored key can't be opened any more (the
                        // person's Windows Hello or fingerprints changed,
                        // say). Forget it, so the extension asks for the
                        // master password and offers to turn this on again.
                        Answer::Lost(_) => self.biometrics.remove(user_id),
                    }
                }
                reply.insert("response".into(), json!(unlocked));
            }
            "keykeeper.biometric.enroll" => {
                let key = message
                    .get("userKeyB64")
                    .and_then(Value::as_str)
                    .and_then(|k| STANDARD.decode(k).ok())
                    .map(Zeroizing::new)
                    .filter(|k| USER_KEY_BYTES.contains(&k.len()));
                let email = message
                    .get("email")
                    .and_then(Value::as_str)
                    .unwrap_or("this account");
                let enrolled = match key {
                    Some(key) if !user_id.is_empty() => {
                        let reason =
                            format!("Turn on unlock with {method} for {email} in {browser}");
                        let answer = self
                            .gate
                            .prompt(now, || self.biometrics.enroll(user_id, &key, &reason));
                        matches!(answer, Answer::Approved(()))
                    }
                    _ => false,
                };
                reply.insert("response".into(), json!(enrolled));
            }
            "keykeeper.biometric.remove" => {
                // The extension stamps every message with the active
                // account; logging out another one names it.
                let target = message
                    .get("forUserId")
                    .and_then(Value::as_str)
                    .unwrap_or(user_id);
                self.biometrics.remove(target);
                reply.insert("response".into(), json!(true));
            }
            // As upstream: unknown commands get no reply.
            _ => return None,
        }
        Some(reply)
    }
}

fn invalidate(app_id: &str) -> String {
    json!({ "appId": app_id, "command": "invalidateEncryption" }).to_string()
}

/// An EncString the way the extension's `EncString` serializes, which is what
/// it reads replies as.
fn enc_string_object(enc_string: &str) -> Value {
    let parts: Vec<&str> = enc_string[2..].split('|').collect();
    json!({
        "encryptedString": enc_string,
        "encryptionType": 2,
        "iv": parts[0],
        "data": parts[1],
        "mac": parts[2],
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use rsa::{
        pkcs8::{DecodePrivateKey, EncodePublicKey},
        Oaep, RsaPrivateKey,
    };

    use super::*;

    const NOW: i64 = 1_760_000_000_000;
    const USER: &str = "3f1c2a64-8d0e-4b7f-9a55-6c2e1d0b9f10";

    /// A stand-in for Windows Hello: answers prompts from a script.
    struct Fake {
        status: BiometricsStatus,
        answers: StdMutex<Vec<&'static str>>,
        keys: StdMutex<HashMap<String, Vec<u8>>>,
        reasons: StdMutex<Vec<String>>,
    }

    impl Fake {
        fn new(answers: &[&'static str]) -> Self {
            Self {
                status: BiometricsStatus::Available,
                answers: StdMutex::new(answers.iter().rev().copied().collect()),
                keys: StdMutex::default(),
                reasons: StdMutex::default(),
            }
        }

        fn answer<T>(&self, reason: &str, value: impl FnOnce() -> T) -> Answer<T> {
            self.reasons.lock().unwrap().push(reason.to_owned());
            match self.answers.lock().unwrap().pop() {
                Some("yes") => Answer::Approved(value()),
                Some("fail") => Answer::Failed("the device is locked".into()),
                Some("lost") => Answer::Lost("Windows Hello was set up again".into()),
                _ => Answer::Canceled,
            }
        }
    }

    impl Biometrics for Fake {
        fn status(&self) -> BiometricsStatus {
            self.status
        }
        fn is_enrolled(&self, user_id: &str) -> bool {
            self.keys.lock().unwrap().contains_key(user_id)
        }
        fn enroll(&self, user_id: &str, user_key: &[u8], reason: &str) -> Answer<()> {
            let answer = self.answer(reason, || ());
            if answer == Answer::Approved(()) {
                self.keys
                    .lock()
                    .unwrap()
                    .insert(user_id.into(), user_key.to_vec());
            }
            answer
        }
        fn unlock(&self, user_id: &str, reason: &str) -> Answer<Zeroizing<Vec<u8>>> {
            let key = self
                .keys
                .lock()
                .unwrap()
                .get(user_id)
                .cloned()
                .unwrap_or_default();
            self.answer(reason, || Zeroizing::new(key))
        }
        fn authenticate(&self, reason: &str) -> Answer<()> {
            self.answer(reason, || ())
        }
        fn remove(&self, user_id: &str) {
            self.keys.lock().unwrap().remove(user_id);
        }
    }

    /// The extension's side, as nativeMessaging.background.ts does it.
    struct Extension {
        private_key: RsaPrivateKey,
        session: Option<SessionKey>,
    }

    impl Extension {
        fn new() -> Self {
            // The fixed test key from the vectors: generating one per test is slow.
            let vectors: Value =
                serde_json::from_str(include_str!("../tests/vectors/native-messaging.json"))
                    .unwrap();
            let der = STANDARD
                .decode(vectors["rsa"]["privateKeyPkcs8"].as_str().unwrap())
                .unwrap();
            Self {
                private_key: RsaPrivateKey::from_pkcs8_der(&der).unwrap(),
                session: None,
            }
        }

        fn setup(&mut self, connection: &mut Connection, timestamp: i64) -> Option<Value> {
            let public_key = self
                .private_key
                .to_public_key()
                .to_public_key_der()
                .unwrap();
            let frame = json!({
                "appId": "app-1",
                "message": {
                    "command": "setupEncryption",
                    "publicKey": STANDARD.encode(public_key.as_bytes()),
                    "userId": USER,
                    "messageId": 0,
                    "timestamp": timestamp,
                },
            });
            let reply: Value =
                serde_json::from_str(&connection.handle(&frame.to_string())?).unwrap();
            assert_eq!(reply["command"], "setupEncryption");
            assert_eq!(reply["appId"], "app-1");
            let wrapped = STANDARD
                .decode(reply["sharedSecret"].as_str().unwrap())
                .unwrap();
            let key = self
                .private_key
                .decrypt(Oaep::new::<sha1::Sha1>(), &wrapped)
                .unwrap();
            self.session = Some(SessionKey::from_test_bytes(&key));
            Some(reply)
        }

        fn call(&self, connection: &mut Connection, mut message: Value) -> Option<Value> {
            message["userId"] = json!(USER);
            message["timestamp"] = json!(message.get("timestamp").cloned().unwrap_or(json!(NOW)));
            message["messageId"] = json!(5);
            let session = self.session.as_ref().unwrap();
            let frame = json!({
                "appId": "app-1",
                "message": enc_string_object(&session.encrypt(&message.to_string())),
            });
            let reply: Value =
                serde_json::from_str(&connection.handle(&frame.to_string())?).unwrap();
            if reply.get("command").is_some() {
                return Some(reply);
            }
            assert_eq!(reply["appId"], "app-1");
            assert_eq!(reply["messageId"], 5);
            assert_eq!(reply["message"]["encryptionType"], 2);
            let inner = session
                .decrypt(reply["message"]["encryptedString"].as_str().unwrap())
                .unwrap();
            Some(serde_json::from_str(&inner).unwrap())
        }
    }

    fn context() -> Context {
        Context {
            browser: "Chrome".into(),
            method: "Windows Hello",
        }
    }

    fn user_key() -> Vec<u8> {
        (0u8..64).collect()
    }

    fn enroll_message() -> Value {
        json!({
            "command": "keykeeper.biometric.enroll",
            "userKeyB64": STANDARD.encode(user_key()),
            "email": "ana@example.test",
        })
    }

    #[test]
    fn enroll_then_unlock_returns_the_key_only_after_each_prompt() {
        let fake = Fake::new(&["yes", "yes"]);
        let gate = PromptGate::default();
        let mut connection = Connection::new(&fake, &gate, context()).with_clock(|| NOW);
        let mut extension = Extension::new();
        extension.setup(&mut connection, NOW).unwrap();

        let status = json!({ "command": "getBiometricsStatusForUser" });
        assert_eq!(
            extension.call(&mut connection, status.clone()).unwrap()["response"],
            BiometricsStatus::NotEnabledInConnectedDesktopApp as i32
        );
        let enrolled = extension.call(&mut connection, enroll_message()).unwrap();
        assert_eq!(enrolled["response"], true);
        assert_eq!(enrolled["command"], "keykeeper.biometric.enroll");
        assert_eq!(
            extension.call(&mut connection, status).unwrap()["response"],
            BiometricsStatus::Available as i32
        );

        let unlocked = extension
            .call(
                &mut connection,
                json!({ "command": "unlockWithBiometricsForUser" }),
            )
            .unwrap();
        assert_eq!(unlocked["response"], true);
        assert_eq!(unlocked["userKeyB64"], STANDARD.encode(user_key()));
        assert_eq!(unlocked["timestamp"], NOW);
        assert_eq!(
            *fake.reasons.lock().unwrap(),
            [
                "Turn on unlock with Windows Hello for ana@example.test in Chrome".to_owned(),
                format!("Unlock {} in Chrome", crate::brand::NAME),
            ]
        );
    }

    #[test]
    fn a_canceled_prompt_returns_nothing_and_blocks_prompts_for_a_while() {
        let fake = Fake::new(&["yes", "no", "yes"]);
        let gate = PromptGate::default();
        let mut connection = Connection::new(&fake, &gate, context()).with_clock(|| NOW);
        let mut extension = Extension::new();
        extension.setup(&mut connection, NOW).unwrap();
        extension.call(&mut connection, enroll_message()).unwrap();

        let unlock = json!({ "command": "unlockWithBiometricsForUser" });
        let canceled = extension.call(&mut connection, unlock.clone()).unwrap();
        assert_eq!(canceled["response"], false);
        assert!(canceled.get("userKeyB64").is_none());
        // Right after a cancel there's no new prompt at all.
        let again = extension.call(&mut connection, unlock).unwrap();
        assert_eq!(again["response"], false);
        assert_eq!(fake.reasons.lock().unwrap().len(), 2);
        // The key is still enrolled: a cancel isn't a failure.
        assert!(fake.is_enrolled(USER));
    }

    #[test]
    fn a_key_that_cant_be_opened_any_more_is_forgotten() {
        let fake = Fake::new(&["yes", "lost"]);
        let gate = PromptGate::default();
        let mut connection = Connection::new(&fake, &gate, context()).with_clock(|| NOW);
        let mut extension = Extension::new();
        extension.setup(&mut connection, NOW).unwrap();
        extension.call(&mut connection, enroll_message()).unwrap();
        let unlock = json!({ "command": "unlockWithBiometricsForUser" });
        assert_eq!(
            extension.call(&mut connection, unlock).unwrap()["response"],
            false
        );
        assert!(!fake.is_enrolled(USER));
    }

    #[test]
    fn a_prompt_that_fails_for_now_keeps_the_key() {
        let fake = Fake::new(&["yes", "fail"]);
        let gate = PromptGate::default();
        let mut connection = Connection::new(&fake, &gate, context()).with_clock(|| NOW);
        let mut extension = Extension::new();
        extension.setup(&mut connection, NOW).unwrap();
        extension.call(&mut connection, enroll_message()).unwrap();
        let unlock = json!({ "command": "unlockWithBiometricsForUser" });
        assert_eq!(
            extension.call(&mut connection, unlock).unwrap()["response"],
            false
        );
        // A locked or busy device isn't a reason to make the person set this up again.
        assert!(fake.is_enrolled(USER));
    }

    #[test]
    fn nothing_is_returned_for_a_user_who_never_enrolled() {
        let fake = Fake::new(&["yes"]);
        let gate = PromptGate::default();
        let mut connection = Connection::new(&fake, &gate, context()).with_clock(|| NOW);
        let mut extension = Extension::new();
        extension.setup(&mut connection, NOW).unwrap();
        let unlocked = extension
            .call(
                &mut connection,
                json!({ "command": "unlockWithBiometricsForUser" }),
            )
            .unwrap();
        assert_eq!(unlocked["response"], false);
        // No prompt was even shown.
        assert!(fake.reasons.lock().unwrap().is_empty());
    }

    #[test]
    fn enrolling_needs_a_plausible_key_and_removing_needs_no_prompt() {
        let fake = Fake::new(&["yes", "yes"]);
        let gate = PromptGate::default();
        let mut connection = Connection::new(&fake, &gate, context()).with_clock(|| NOW);
        let mut extension = Extension::new();
        extension.setup(&mut connection, NOW).unwrap();
        for bad in [
            json!("not base64!"),
            json!(STANDARD.encode([1u8; 8])),
            json!(null),
        ] {
            let mut message = enroll_message();
            message["userKeyB64"] = bad;
            assert_eq!(
                extension.call(&mut connection, message).unwrap()["response"],
                false
            );
        }
        assert!(fake.reasons.lock().unwrap().is_empty());
        extension.call(&mut connection, enroll_message()).unwrap();
        // Logging out another account removes that account's key only.
        fake.keys
            .lock()
            .unwrap()
            .insert("other-user".into(), vec![9; 64]);
        let removed = extension
            .call(
                &mut connection,
                json!({ "command": "keykeeper.biometric.remove", "forUserId": "other-user" }),
            )
            .unwrap();
        assert_eq!(removed["response"], true);
        assert!(fake.is_enrolled(USER) && !fake.is_enrolled("other-user"));
        let removed = extension
            .call(
                &mut connection,
                json!({ "command": "keykeeper.biometric.remove" }),
            )
            .unwrap();
        assert_eq!(removed["response"], true);
        assert!(!fake.is_enrolled(USER));
    }

    #[test]
    fn old_messages_unknown_commands_and_sdk_ipc_get_no_reply() {
        let fake = Fake::new(&[]);
        let gate = PromptGate::default();
        let mut connection = Connection::new(&fake, &gate, context()).with_clock(|| NOW);
        let mut extension = Extension::new();
        // A key exchange from more than 10 seconds ago is ignored.
        assert!(extension.setup(&mut connection, NOW - 10_001).is_none());
        extension.setup(&mut connection, NOW - 10_000).unwrap();
        let old = json!({ "command": "getBiometricsStatus", "timestamp": NOW - 10_001 });
        assert!(extension.call(&mut connection, old).is_none());
        assert!(extension
            .call(&mut connection, json!({ "command": "lockVault" }))
            .is_none());
        let sdk = json!({ "type": "bitwarden-ipc-message", "message": { "destination": "DesktopRenderer" } });
        assert!(connection.handle(&sdk.to_string()).is_none());
        assert!(connection.handle("not json").is_none());
    }

    #[test]
    fn a_message_without_a_session_or_with_a_bad_mac_invalidates_the_channel() {
        let fake = Fake::new(&[]);
        let gate = PromptGate::default();
        let mut connection = Connection::new(&fake, &gate, context()).with_clock(|| NOW);
        let mut extension = Extension::new();
        extension.session = Some(SessionKey::generate());
        let reply = extension
            .call(&mut connection, json!({ "command": "getBiometricsStatus" }))
            .unwrap();
        assert_eq!(
            reply,
            json!({ "appId": "app-1", "command": "invalidateEncryption" })
        );
        extension.setup(&mut connection, NOW).unwrap();
        // Now a message under another key: refused, and the session is gone.
        let real = extension.session.replace(SessionKey::generate());
        let reply = extension
            .call(&mut connection, json!({ "command": "getBiometricsStatus" }))
            .unwrap();
        assert_eq!(reply["command"], "invalidateEncryption");
        extension.session = real;
        let reply = extension
            .call(&mut connection, json!({ "command": "getBiometricsStatus" }))
            .unwrap();
        assert_eq!(reply["command"], "invalidateEncryption");
    }

    #[test]
    fn status_reports_the_platform_first() {
        let mut fake = Fake::new(&[]);
        fake.status = BiometricsStatus::HardwareUnavailable;
        let gate = PromptGate::default();
        let mut connection = Connection::new(&fake, &gate, context()).with_clock(|| NOW);
        let mut extension = Extension::new();
        extension.setup(&mut connection, NOW).unwrap();
        for command in ["getBiometricsStatus", "getBiometricsStatusForUser"] {
            let reply = extension
                .call(&mut connection, json!({ "command": command }))
                .unwrap();
            assert_eq!(
                reply["response"],
                BiometricsStatus::HardwareUnavailable as i32
            );
        }
    }
}
