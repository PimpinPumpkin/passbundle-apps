//! Push to phones (DESIGN.md in the meta repo, "Push to phones"; docs/phones.md,
//! "Approving a sign-in"). The phone's own code asks these for an account,
//! whether or not its vault is unlocked, so they never touch the vault or
//! its keys: the server's push settings (whether it pushes, its VAPID key,
//! and whether this device's endpoint is there); registering and removing
//! this device's endpoint; and, when a message says a sign-in is waiting,
//! how many sign-ins wait for the account's answer, in numbers only.
//!
//! Locked, the session token comes from the refresh token the OS's secret
//! store keeps beside the vault (`App::push_call`). Nothing here is kept,
//! and no request's key, phrase, device, or address leaves this module.

use std::time::SystemTime;

use serde_json::Value;
use zeroize::Zeroizing;

use crate::error::Result;
use crate::server::Server;

/// Asking the server about push without holding the app: the server, and a
/// session token good for a minute at least (`App::push_call`).
pub struct PushCall {
    server: Server,
    access_token: Zeroizing<String>,
    email: String,
}

/// How many sign-ins wait for the account's answer: "Sign in with a device"
/// requests, and new-device checks. Never which.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Waiting {
    pub requests: usize,
    pub checks: usize,
}

impl Waiting {
    pub fn any(&self) -> bool {
        self.requests + self.checks > 0
    }
}

/// The server's push to phones, for this device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Whether the server pushes at all.
    pub enabled: bool,
    /// The key the phone registers with its distributor (base64url).
    pub vapid_public_key: Option<String>,
    /// Whether the server has this device's endpoint.
    pub registered: bool,
}

impl Settings {
    fn read(body: &Value) -> Settings {
        Settings {
            enabled: body["enabled"].as_bool().unwrap_or(false),
            vapid_public_key: body["vapidPublicKey"]
                .as_str()
                .filter(|k| !k.is_empty())
                .map(str::to_owned),
            registered: body["registered"].as_bool().unwrap_or(false),
        }
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl PushCall {
    pub(crate) fn new(server: Server, access_token: Zeroizing<String>, email: String) -> Self {
        PushCall {
            server,
            access_token,
            email,
        }
    }

    /// Whether this session may approve "Sign in with a device" requests:
    /// not one that began from another device's approval
    /// (`Session::from_approval`), so for such a session they don't count.
    fn approves(&self) -> bool {
        crate::server::signed_in_with(&self.access_token).as_deref() != Some("authrequest")
    }

    pub fn settings(&self) -> Result<Settings> {
        Ok(Settings::read(
            &self.server.push_settings(&self.access_token)?,
        ))
    }

    /// Keeps this device's endpoint on the server, with its keys as the
    /// distributor's library gives them (base64url).
    pub fn register(&self, endpoint: &str, public_key: &str, auth: &str) -> Result<Settings> {
        Ok(Settings::read(&self.server.register_push(
            &self.access_token,
            endpoint,
            public_key,
            auth,
        )?))
    }

    pub fn unregister(&self) -> Result<()> {
        self.server.unregister_push(&self.access_token)
    }

    /// How many sign-ins wait for the account's answer, as the app would
    /// list them while open (the same server calls and the same reading of
    /// them): only ones still waiting, not past their time, and "Sign in
    /// with a device" requests only for a session that may approve them.
    pub fn waiting(&self) -> Result<Waiting> {
        let requests = if self.approves() {
            let (body, server_now) = self.server.pending_sign_in_requests(&self.access_token)?;
            crate::device_approval::parse_pending(
                &body,
                &self.email,
                server_now.unwrap_or_else(now),
            )
            .len()
        } else {
            0
        };
        let checks = crate::new_device::parse_waiting(
            &self.server.waiting_new_devices(&self.access_token)?,
            now(),
        )
        .0
        .len();
        Ok(Waiting { requests, checks })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::*;
    use crate::app::App;
    use crate::error::Error;
    use crate::secrets::{MemorySecretStore, SecretStore};
    use crate::store::{Settings as Kept, Store};

    /// An access token as passbundle-server makes one, saying how its
    /// session signed in.
    fn access_token(signed_in: &str) -> String {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine;
        let claims = json!({ "sub": "u", "keykeeper_signin": signed_in });
        format!(
            "eyJhbGciOiJSUzI1NiJ9.{}.c2lnbmF0dXJl",
            URL_SAFE_NO_PAD.encode(claims.to_string())
        )
    }

    /// A time as the server writes one.
    fn iso(secs: u64) -> String {
        let days = (secs / 86_400) as i64;
        let rest = secs % 86_400;
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        format!(
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000000Z",
            rest / 3600,
            rest % 3600 / 60,
            rest % 60
        )
    }

    /// What the stand-in was asked: method, path, Authorization, body.
    type Heard = Arc<Mutex<Vec<(String, String, String, String)>>>;

    /// The request's public key upstream's clients make (the vectors').
    fn request_key() -> String {
        let v: Value =
            serde_json::from_str(include_str!("../tests/vectors/device-approval-v1.json")).unwrap();
        v["request"]["publicKey"].as_str().unwrap().to_owned()
    }

    /// A stand-in server: the refresh grant answers `signed_in`'s token (or
    /// 400 when `refused`); two sign-in requests wait (one past its time),
    /// and one new-device check; push's settings and registration.
    fn stand_in(signed_in: &'static str, refused: bool) -> (String, Heard) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr().to_ip().unwrap());
        let heard: Heard = Arc::default();
        let log = heard.clone();
        std::thread::spawn(move || {
            for mut request in server.incoming_requests() {
                let mut body = String::new();
                let _ = request.as_reader().read_to_string(&mut body);
                let auth = request
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("Authorization"))
                    .map(|h| h.value.as_str().to_owned())
                    .unwrap_or_default();
                let method = request.method().to_string();
                let path = request.url().to_owned();
                log.lock()
                    .unwrap()
                    .push((method.clone(), path.clone(), auth, body.clone()));
                let now = now();
                let (status, answer) = match (method.as_str(), path.as_str()) {
                    ("POST", "/identity/connect/token") if refused => {
                        (400, json!({ "error": "invalid_grant" }))
                    }
                    ("POST", "/identity/connect/token") => (
                        200,
                        json!({ "access_token": access_token(signed_in), "expires_in": 3600 }),
                    ),
                    ("GET", "/api/auth-requests/pending") => (
                        200,
                        json!({ "data": [
                            { "id": "r1", "publicKey": request_key(), "requestDeviceType": "Firefox",
                              "requestIpAddress": "203.0.113.7", "creationDate": iso(now - 60),
                              "requestApproved": null, "responseDate": null },
                            { "id": "r2", "publicKey": request_key(), "requestDeviceType": "Chrome",
                              "requestIpAddress": "203.0.113.8", "creationDate": iso(now - 3600),
                              "requestApproved": null, "responseDate": null },
                        ] }),
                    ),
                    ("GET", "/api/keykeeper/new-device-checks") => (
                        200,
                        json!({ "serverDate": iso(now), "data": [
                            { "id": "c1", "device": "Web vault in Firefox", "deviceType": 10,
                              "ipAddress": "203.0.113.9", "creationDate": iso(now - 30),
                              "expirationDate": iso(now + 570) },
                        ] }),
                    ),
                    ("GET", "/api/keykeeper/push/unified-push") => (
                        200,
                        json!({ "object": "unifiedPush", "enabled": true,
                                "vapidPublicKey": "BKey", "registered": false }),
                    ),
                    ("PUT", "/api/keykeeper/push/unified-push") => (
                        200,
                        json!({ "object": "unifiedPush", "enabled": true,
                                "vapidPublicKey": "BKey", "registered": true }),
                    ),
                    ("DELETE", "/api/keykeeper/push/unified-push") => (200, Value::Null),
                    _ => (404, json!({ "message": "Not found" })),
                };
                let _ = request.respond(
                    tiny_http::Response::from_string(answer.to_string())
                        .with_status_code(status)
                        .with_header(
                            tiny_http::Header::from_bytes("Content-Type", "application/json")
                                .unwrap(),
                        ),
                );
            }
        });
        (url, heard)
    }

    /// An account on this device, signed in before and locked now: its
    /// settings and kept sync on the disk, its refresh token in the OS's
    /// store, and no vault open.
    fn locked(url: &str, secrets: Arc<MemorySecretStore>) -> (App, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("pb-push-{}", uuid::Uuid::new_v4()));
        let store = Store::new(&dir);
        store
            .save_settings(&Kept {
                server: url.to_owned(),
                email: "ana@example.test".to_owned(),
                kdf: 0,
                kdf_iterations: 600_000,
                kdf_memory: None,
                kdf_parallelism: None,
                device_identifier: "3a1b5c7d-0000-4000-8000-0000000000aa".to_owned(),
            })
            .unwrap();
        store.save_sync("{}").unwrap();
        assert!(secrets.keep(url, "ana@example.test", "kept-refresh-token"));
        let app = App::open(store, Box::new(secrets));
        assert!(app.vault().is_none(), "locked");
        (app, dir)
    }

    #[test]
    fn locked_it_asks_with_the_kept_refresh_token_and_hears_only_how_many() {
        let (url, heard) = stand_in("password", false);
        let secrets = Arc::new(MemorySecretStore::default());
        let (mut app, dir) = locked(&url, secrets.clone());

        let call = app.push_call().unwrap();
        // One waits; the other is past its 15 minutes. And one new device.
        assert_eq!(
            call.waiting().unwrap(),
            Waiting {
                requests: 1,
                checks: 1
            }
        );
        assert!(call.waiting().unwrap().any());

        let heard = heard.lock().unwrap();
        // The kept refresh token, once; then the session's token, and
        // nothing that unlocks.
        let (_, _, _, form) = &heard[0];
        assert!(form.contains("grant_type=refresh_token"), "{form}");
        assert!(form.contains("refresh_token=kept-refresh-token"), "{form}");
        let bearer = format!("Bearer {}", access_token("password"));
        for (method, path, auth, _) in &heard[1..] {
            assert_eq!(auth, &bearer, "{method} {path}");
            assert_eq!(method, "GET");
        }
        assert!(app.vault().is_none(), "still locked");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_session_from_an_approval_counts_only_new_devices() {
        let (url, heard) = stand_in("authrequest", false);
        let (mut app, dir) = locked(&url, Arc::new(MemorySecretStore::default()));
        assert_eq!(
            app.push_call().unwrap().waiting().unwrap(),
            Waiting {
                requests: 0,
                checks: 1
            }
        );
        assert!(!heard
            .lock()
            .unwrap()
            .iter()
            .any(|(_, path, _, _)| path == "/api/auth-requests/pending"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn registers_and_removes_this_devices_endpoint() {
        let (url, heard) = stand_in("password", false);
        let (mut app, dir) = locked(&url, Arc::new(MemorySecretStore::default()));
        let call = app.push_call().unwrap();
        assert_eq!(
            call.settings().unwrap(),
            Settings {
                enabled: true,
                vapid_public_key: Some("BKey".into()),
                registered: false
            }
        );
        let registered = call
            .register("https://ntfy.example.net/upAbC?up=1", "BPhone", "QXV0aA")
            .unwrap();
        assert!(registered.registered);
        call.unregister().unwrap();
        let heard = heard.lock().unwrap();
        let put = heard.iter().find(|(m, _, _, _)| m == "PUT").unwrap();
        let sent: Value = serde_json::from_str(&put.3).unwrap();
        assert_eq!(
            sent,
            json!({ "endpoint": "https://ntfy.example.net/upAbC?up=1", "publicKey": "BPhone", "auth": "QXV0aA" })
        );
        assert!(heard.iter().any(|(m, _, _, _)| m == "DELETE"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_refresh_token_the_server_refuses_is_a_session_thats_over() {
        let (url, _) = stand_in("password", true);
        let (mut app, dir) = locked(&url, Arc::new(MemorySecretStore::default()));
        assert_eq!(app.push_call().err(), Some(Error::SignedOut));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn signed_out_or_without_a_kept_session_it_asks_nothing() {
        let (url, heard) = stand_in("password", false);
        let (mut app, dir) = locked(&url, Arc::new(MemorySecretStore::default()));
        app.sign_out();
        assert_eq!(app.push_call().err(), Some(Error::SignedOut));
        // No account at all.
        assert_eq!(App::nobody().push_call().err(), Some(Error::SignedOut));
        assert!(heard.lock().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_server_without_push_says_it_doesnt_push() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr().to_ip().unwrap());
        std::thread::spawn(move || {
            for request in server.incoming_requests() {
                let (status, body) = if request.url() == "/identity/connect/token" {
                    (
                        200,
                        json!({ "access_token": access_token("password"), "expires_in": 3600 }),
                    )
                } else {
                    (404, json!({ "message": "Not found" }))
                };
                let _ = request.respond(
                    tiny_http::Response::from_string(body.to_string()).with_status_code(status),
                );
            }
        });
        let (mut app, dir) = locked(&url, Arc::new(MemorySecretStore::default()));
        let call = app.push_call().unwrap();
        assert_eq!(
            call.settings().unwrap(),
            Settings {
                enabled: false,
                vapid_public_key: None,
                registered: false
            }
        );
        call.unregister().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
