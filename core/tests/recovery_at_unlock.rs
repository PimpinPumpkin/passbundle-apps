//! Enrolling in account recovery at unlock (DESIGN.md in the meta repo,
//! "Enrolling at unlock", "Our apps"; src/app/recovery.rs): on the vectors'
//! sample vault kept on this device, with a stand-in for the server that
//! serves the organization's public key and takes the enrollment, and the
//! meta repo's recovery-at-unlock vectors for the organization's key pair
//! and its fingerprint, the pin. The fingerprint and the wrap themselves are
//! checked against the vectors in src/recovery.rs.
//!
//! The core's tests against a real server (tests/server.rs) can't drive
//! this: their servers send no email, so they don't offer account recovery,
//! and refuse enrolling in it.

#[path = "support/stand_in.rs"]
mod stand_in;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::STANDARD, Engine};
use passbundle_core::app::{App, Step};
use passbundle_core::crypto::{EncString, PrivateKey};
use passbundle_core::recovery::{Outcome, Tried, NOT_THE_PINNED_KEY, NO_PIN};
use passbundle_core::secrets::MemorySecretStore;
use passbundle_core::store::Store;
use serde_json::{json, Value};
use stand_in::{core_vectors, user_key, Sample, ORG};

/// A second organization the sample account is in, with the same key
/// (the vault opens it as it opens the first).
const ORG_2: &str = "9b8a7c6d-5e4f-4a3b-8c2d-1e0f9a8b7c6d";

fn vectors() -> Value {
    serde_json::from_str(include_str!("vectors/recovery-at-unlock-v1.json")).unwrap()
}

/// The install command's fingerprint of the organization's key: the pin.
fn pin() -> String {
    vectors()["organization"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn organization_public_key() -> String {
    vectors()["organization"]["publicKey"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// What a hacked server could serve in the organization's place.
fn other_public_key() -> String {
    vectors()["other"]["publicKey"].as_str().unwrap().to_owned()
}

fn user_id() -> String {
    core_vectors()["sync"]["profile"]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// The sample's membership as a server that offers account recovery sends
/// it, in an organization whose policy is on with automatic enrollment:
/// due to be enrolled.
fn due(sync: &mut Value) {
    let membership = &mut sync["profile"]["organizations"][0];
    for (field, value) in [
        ("usePolicies", true),
        ("useResetPassword", true),
        ("hasPublicAndPrivateKeys", true),
        ("resetPasswordEnrolled", false),
        ("keykeeperFromMsp", false),
    ] {
        membership[field] = value.into();
    }
    sync["policies"] = json!([{
        "id": "0c1d2e3f-4a5b-4c6d-8e7f-9a0b1c2d3e4f",
        "organizationId": ORG,
        "type": 8,
        "enabled": true,
        "data": { "autoEnrollEnabled": true, "autoEnrollMemberships": null },
        "object": "policy",
    }]);
}

/// What the stand-in holds and serves.
struct Held {
    sync: Value,
    /// Each organization's public key, as the server serves it.
    keys: HashMap<String, String>,
    /// The enrollment's refusal, in the server's words.
    refuse: Option<&'static str>,
}

/// A stand-in for the server: a session for the refresh token or a
/// password, the sync it holds, each organization's public key, and the
/// enrollment, which marks the membership enrolled in what it syncs next.
/// Every request, with its body.
struct StandIn {
    url: String,
    asked: Arc<Mutex<Vec<(String, Value)>>>,
}

impl StandIn {
    fn new(sync: Value, keys: &[(&str, String)], refuse: Option<&'static str>) -> StandIn {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr().to_ip().unwrap());
        let asked = Arc::new(Mutex::new(Vec::new()));
        let log = asked.clone();
        let mut held = Held {
            sync,
            keys: keys
                .iter()
                .map(|(org, key)| (org.to_string(), key.clone()))
                .collect(),
            refuse,
        };
        std::thread::spawn(move || {
            for mut request in server.incoming_requests() {
                let path = request.url().to_owned();
                let method = request.method().to_string();
                let mut body = String::new();
                let _ = request.as_reader().read_to_string(&mut body);
                let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                log.lock()
                    .unwrap()
                    .push((format!("{method} {path}"), body.clone()));
                let respond = |status: u16, body: String| {
                    tiny_http::Response::from_string(body)
                        .with_status_code(status)
                        .with_header(
                            tiny_http::Header::from_bytes("Content-Type", "application/json")
                                .unwrap(),
                        )
                };
                let (status, answer) = if path == "/identity/accounts/prelogin" {
                    let pre = &core_vectors()["account"]["pbkdf2"]["prelogin"];
                    (
                        200,
                        json!({ "kdf": 0, "kdfIterations": pre["kdfIterations"] }),
                    )
                } else if path == "/identity/connect/token" {
                    let profile = &held.sync["profile"];
                    (
                        200,
                        json!({
                            "access_token": "a.b.c", "expires_in": 3600, "refresh_token": "next",
                            "token_type": "Bearer", "Key": profile["key"], "PrivateKey": profile["privateKey"],
                        }),
                    )
                } else if path.starts_with("/api/sync") {
                    (200, held.sync.clone())
                } else if let Some(org) = path
                    .strip_prefix("/api/organizations/")
                    .and_then(|rest| rest.strip_suffix("/keys"))
                {
                    match held.keys.get(org) {
                        Some(key) => (
                            200,
                            json!({ "object": "organizationPublicKey", "publicKey": key }),
                        ),
                        None => (404, json!({ "message": "Organization not found" })),
                    }
                } else if let Some(org) = path
                    .strip_prefix("/api/organizations/")
                    .and_then(|rest| rest.strip_suffix("/reset-password-enrollment"))
                    .and_then(|rest| rest.split_once("/users/"))
                    .map(|(org, _)| org.to_owned())
                {
                    if let Some(words) = held.refuse {
                        (400, json!({ "message": words }))
                    } else {
                        for m in held.sync["profile"]["organizations"]
                            .as_array_mut()
                            .unwrap()
                        {
                            if m["id"] == org.as_str() {
                                m["resetPasswordEnrolled"] = true.into();
                            }
                        }
                        (200, Value::Null)
                    }
                } else {
                    (404, json!({}))
                };
                let text = if answer.is_null() {
                    String::new()
                } else {
                    answer.to_string()
                };
                let _ = request.respond(respond(status, text));
            }
        });
        StandIn { url, asked }
    }

    /// Every request, as `METHOD path`.
    fn asked(&self) -> Vec<String> {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .map(|(asked, _)| asked.clone())
            .collect()
    }

    /// The requests about the vault and its organizations: no session's,
    /// and none of PassBundle's own settings and numbers a sync also asks.
    fn about_the_vault(&self) -> Vec<String> {
        self.asked()
            .into_iter()
            .filter(|a| !a.contains("/identity/") && !a.contains("/api/keykeeper/"))
            .collect()
    }

    /// The bodies of the enrollments sent.
    fn enrollments(&self) -> Vec<Value> {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .filter(|(asked, _)| asked.ends_with("/reset-password-enrollment"))
            .map(|(_, body)| body.clone())
            .collect()
    }

    fn forget(&self) {
        self.asked.lock().unwrap().clear();
    }
}

fn keys_path(org: &str) -> String {
    format!("GET /api/organizations/{org}/keys")
}

fn enrollment_path(org: &str) -> String {
    format!(
        "PUT /api/organizations/{org}/users/{}/reset-password-enrollment",
        user_id()
    )
}

const SYNC: &str = "GET /api/sync?excludeDomains=true";

/// The sample vault kept here and the stand-in, each holding the sample's
/// sync as `edit` leaves it, the stand-in serving `keys`; and the app,
/// unlocked.
fn unlocked_with(
    edit: impl Fn(&mut Value),
    keys: &[(&str, String)],
    refuse: Option<&'static str>,
) -> (Sample, StandIn, App) {
    let mut sync = core_vectors()["sync"].clone();
    edit(&mut sync);
    let server = StandIn::new(sync, keys, refuse);
    let sample = Sample::new(&server.url, &[], |s| edit(s));
    let app = sample.unlocked();
    // Unlocking is offline.
    assert_eq!(server.asked(), Vec::<String>::new());
    (sample, server, app)
}

fn unlocked(edit: impl Fn(&mut Value)) -> (Sample, StandIn, App) {
    unlocked_with(edit, &[(ORG, organization_public_key())], None)
}

fn outcomes(tried: &[Tried]) -> Vec<(String, Outcome)> {
    tried
        .iter()
        .map(|t| (t.organization_id.clone(), t.outcome.clone()))
        .collect()
}

/// What the organization's private key opens a wrapped user key to.
fn opened_by_the_organization(wrapped: &Value) -> Vec<u8> {
    let private = PrivateKey::from_pkcs8_der(
        &STANDARD
            .decode(vectors()["organization"]["privateKey"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap();
    private
        .decrypt(&EncString::parse(wrapped.as_str().unwrap()).unwrap())
        .unwrap()
        .to_vec()
}

#[test]
fn a_member_whose_organization_enrolls_everyone_is_enrolled_to_the_pinned_key_and_synced_once_after(
) {
    let (_sample, server, mut app) = unlocked(due);
    let tried = app.recovery_at_unlock(Some(&pin()));
    assert_eq!(outcomes(&tried), vec![(ORG.to_owned(), Outcome::Enrolled)]);
    assert_eq!(tried[0].name, "Contoso");

    // The key, then the enrollment, as the vectors have them, then one
    // sync: none before.
    assert_eq!(
        server.about_the_vault(),
        vec![keys_path(ORG), enrollment_path(ORG), SYNC.to_owned()]
    );
    let v = vectors();
    let requests = v["enrollment"]["requests"].as_array().unwrap();
    for (asked, vector) in server.about_the_vault().iter().zip(requests) {
        assert_eq!(
            asked,
            &format!(
                "{} {}",
                vector["method"].as_str().unwrap(),
                vector["path"].as_str().unwrap()
            )
        );
    }

    // Only the wrapped user key, never the master password's hash: the
    // server's no-password path. The organization's private key opens it
    // to the user key, so it went to the pinned key.
    let sent = server.enrollments();
    assert_eq!(sent.len(), 1);
    let body = sent[0].as_object().unwrap();
    assert_eq!(body.keys().collect::<Vec<_>>(), vec!["resetPasswordKey"]);
    let wrapped = &sent[0]["resetPasswordKey"];
    assert!(wrapped.as_str().unwrap().starts_with("4."));
    assert_eq!(
        opened_by_the_organization(wrapped),
        user_key().to_bytes().to_vec()
    );

    // Once per unlock: nothing more now.
    server.forget();
    assert_eq!(app.recovery_at_unlock(Some(&pin())), vec![]);
    assert_eq!(server.asked(), Vec::<String>::new());

    // The sync after it brought the enrollment, so the next unlock has
    // nobody to enroll and sends nothing.
    app.lock();
    app.unlock(&_sample.password).unwrap();
    assert_eq!(app.recovery_at_unlock(Some(&pin())), vec![]);
    assert_eq!(server.asked(), Vec::<String>::new());
}

#[test]
fn each_condition_alone_keeps_anything_from_being_sent() {
    type Edit = Box<dyn Fn(&mut Value)>;
    let membership = |field: &'static str, value: Value| -> Edit {
        Box::new(move |s: &mut Value| {
            due(s);
            s["profile"]["organizations"][0][field] = value.clone();
        })
    };
    let cases: Vec<(&str, Edit)> = vec![
        ("invited, not confirmed", membership("status", 0.into())),
        ("accepted, not confirmed", membership("status", 1.into())),
        ("revoked", membership("status", (-1).into())),
        (
            "enrolled already",
            membership("resetPasswordEnrolled", true.into()),
        ),
        ("from an MSP", membership("keykeeperFromMsp", true.into())),
        (
            "the organization doesn't use policies",
            membership("usePolicies", false.into()),
        ),
        (
            "the server doesn't send email",
            membership("useResetPassword", false.into()),
        ),
        (
            "the organization has no keys",
            membership("hasPublicAndPrivateKeys", false.into()),
        ),
        (
            "the policy off",
            Box::new(|s: &mut Value| {
                due(s);
                s["policies"][0]["enabled"] = false.into();
            }),
        ),
        (
            "automatic enrollment off",
            Box::new(|s: &mut Value| {
                due(s);
                s["policies"][0]["data"]["autoEnrollEnabled"] = false.into();
            }),
        ),
        (
            "only another organization's policy",
            Box::new(|s: &mut Value| {
                due(s);
                s["policies"][0]["organizationId"] = ORG_2.into();
            }),
        ),
        (
            "no policy at all",
            Box::new(|s: &mut Value| {
                due(s);
                s["policies"] = json!([]);
            }),
        ),
    ];
    for (why, edit) in cases {
        let (_sample, server, mut app) = unlocked(edit);
        assert_eq!(app.recovery_at_unlock(Some(&pin())), vec![], "{why}");
        assert_eq!(server.asked(), Vec::<String>::new(), "{why}");
    }
}

#[test]
fn without_a_pin_nothing_is_asked_at_all_not_even_the_key() {
    let full = pin();
    for (why, given) in [
        ("none", None),
        ("empty", Some(String::new())),
        ("one character short", Some(full[..42].to_owned())),
        ("with base64's padding", Some(format!("{full}="))),
        ("in hex", Some("ab".repeat(32))),
    ] {
        let (_sample, server, mut app) = unlocked(due);
        let tried = app.recovery_at_unlock(given.as_deref());
        assert_eq!(
            outcomes(&tried),
            vec![(ORG.to_owned(), Outcome::NoPin)],
            "{why}"
        );
        assert_eq!(tried[0].outcome.why().as_deref(), Some(NO_PIN));
        assert_eq!(server.asked(), Vec::<String>::new(), "{why}");
    }
}

#[test]
fn a_pin_that_isnt_the_organizations_key_sends_nothing_but_the_question() {
    let (_sample, server, mut app) = unlocked(due);
    let other_pin = vectors()["other"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();
    let tried = app.recovery_at_unlock(Some(&other_pin));
    assert_eq!(
        outcomes(&tried),
        vec![(ORG.to_owned(), Outcome::NotThePinnedKey)]
    );
    assert_eq!(tried[0].outcome.why().as_deref(), Some(NOT_THE_PINNED_KEY));
    // Asked for the key, and nothing else: no enrollment, no sync.
    assert_eq!(server.about_the_vault(), vec![keys_path(ORG)]);
    assert!(server.enrollments().is_empty());
}

#[test]
fn another_key_from_the_server_is_refused_and_tried_again_only_at_the_next_unlock() {
    // A hacked server's own key in the organization's place.
    let (sample, server, mut app) = unlocked_with(due, &[(ORG, other_public_key())], None);
    let tried = app.recovery_at_unlock(Some(&pin()));
    assert_eq!(
        outcomes(&tried),
        vec![(ORG.to_owned(), Outcome::NotThePinnedKey)]
    );
    assert_eq!(server.about_the_vault(), vec![keys_path(ORG)]);
    assert!(server.enrollments().is_empty());

    // Not again this unlock.
    server.forget();
    assert_eq!(app.recovery_at_unlock(Some(&pin())), vec![]);
    assert_eq!(server.asked(), Vec::<String>::new());

    // The next unlock checks again, and refuses again.
    app.lock();
    app.unlock(&sample.password).unwrap();
    let tried = app.recovery_at_unlock(Some(&pin()));
    assert_eq!(
        outcomes(&tried),
        vec![(ORG.to_owned(), Outcome::NotThePinnedKey)]
    );
    assert_eq!(server.about_the_vault(), vec![keys_path(ORG)]);
    assert!(server.enrollments().is_empty());
}

#[test]
fn a_refusal_waits_for_the_next_unlock_with_no_sync() {
    let words = "MSP staff can't join an organization's account recovery";
    let (sample, server, mut app) =
        unlocked_with(due, &[(ORG, organization_public_key())], Some(words));
    let tried = app.recovery_at_unlock(Some(&pin()));
    let [Tried {
        outcome: Outcome::Failed(why),
        ..
    }] = tried.as_slice()
    else {
        panic!("refused: {tried:?}");
    };
    assert!(why.contains(words), "{why}");
    // Never with the wrapped key the server was sent.
    let sent = server.enrollments();
    assert!(!why.contains(sent[0]["resetPasswordKey"].as_str().unwrap()));
    assert_eq!(
        server.about_the_vault(),
        vec![keys_path(ORG), enrollment_path(ORG)]
    );

    server.forget();
    assert_eq!(app.recovery_at_unlock(Some(&pin())), vec![]);
    assert_eq!(server.asked(), Vec::<String>::new());

    app.lock();
    app.unlock(&sample.password).unwrap();
    app.recovery_at_unlock(Some(&pin()));
    assert_eq!(
        server.about_the_vault(),
        vec![keys_path(ORG), enrollment_path(ORG)]
    );
}

#[test]
fn offline_it_waits_for_the_next_unlock() {
    // Nothing listens here.
    let url = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    };
    let sample = Sample::new(&url, &[], due);
    let mut app = sample.unlocked();
    let tried = app.recovery_at_unlock(Some(&pin()));
    assert!(
        matches!(
            tried.as_slice(),
            [Tried {
                outcome: Outcome::Failed(_),
                ..
            }]
        ),
        "{tried:?}"
    );
    assert_eq!(app.recovery_at_unlock(Some(&pin())), vec![]);
    app.lock();
    app.unlock(&sample.password).unwrap();
    assert_eq!(app.recovery_at_unlock(Some(&pin())).len(), 1);
}

#[test]
fn every_role_is_enrolled_owners_and_admins_too() {
    // Owner, admin, user, manager, custom role.
    for role in 0..=4 {
        let (_sample, server, mut app) = unlocked(move |s: &mut Value| {
            due(s);
            s["profile"]["organizations"][0]["type"] = role.into();
        });
        assert_eq!(
            outcomes(&app.recovery_at_unlock(Some(&pin()))),
            vec![(ORG.to_owned(), Outcome::Enrolled)],
            "role {role}"
        );
        assert_eq!(server.enrollments().len(), 1, "role {role}");
    }
}

#[test]
fn another_organizations_key_is_refused_while_the_pinned_one_is_enrolled() {
    let two = |s: &mut Value| {
        due(s);
        let mut second = s["profile"]["organizations"][0].clone();
        second["id"] = ORG_2.into();
        second["name"] = "Fabrikam".into();
        s["profile"]["organizations"]
            .as_array_mut()
            .unwrap()
            .push(second);
        let mut policy = s["policies"][0].clone();
        policy["id"] = "1d2e3f4a-5b6c-4d7e-8f9a-0b1c2d3e4f5a".into();
        policy["organizationId"] = ORG_2.into();
        s["policies"].as_array_mut().unwrap().push(policy);
    };
    let (_sample, server, mut app) = unlocked_with(
        two,
        &[
            (ORG, organization_public_key()),
            (ORG_2, other_public_key()),
        ],
        None,
    );
    let tried = app.recovery_at_unlock(Some(&pin()));
    assert_eq!(
        outcomes(&tried),
        vec![
            (ORG.to_owned(), Outcome::Enrolled),
            (ORG_2.to_owned(), Outcome::NotThePinnedKey),
        ]
    );
    assert_eq!(
        server.about_the_vault(),
        vec![
            keys_path(ORG),
            enrollment_path(ORG),
            keys_path(ORG_2),
            SYNC.to_owned()
        ]
    );
    assert_eq!(server.enrollments().len(), 1);
}

#[test]
fn locked_nothing_is_tried_and_the_unlock_still_has_its_try() {
    let (sample, server, mut app) = unlocked(due);
    app.lock();
    assert_eq!(app.recovery_at_unlock(Some(&pin())), vec![]);
    assert_eq!(server.asked(), Vec::<String>::new());
    app.unlock(&sample.password).unwrap();
    assert_eq!(
        outcomes(&app.recovery_at_unlock(Some(&pin()))),
        vec![(ORG.to_owned(), Outcome::Enrolled)]
    );
}

#[test]
fn signing_in_is_an_unlock_too() {
    let mut sync = core_vectors()["sync"].clone();
    due(&mut sync);
    let server = StandIn::new(sync, &[(ORG, organization_public_key())], None);
    let dir = std::env::temp_dir().join(format!(
        "pb-core-recovery-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let mut app = App::open(Store::new(&dir), Box::new(MemorySecretStore::default()));
    let v = core_vectors();
    assert_eq!(
        app.sign_in(
            &server.url,
            v["account"]["salt"].as_str().unwrap(),
            v["account"]["password"].as_str().unwrap()
        )
        .unwrap(),
        Step::Done
    );
    // Signing in synced; enrolling starts none of its own before.
    server.forget();
    assert_eq!(
        outcomes(&app.recovery_at_unlock(Some(&pin()))),
        vec![(ORG.to_owned(), Outcome::Enrolled)]
    );
    assert_eq!(
        server.about_the_vault(),
        vec![keys_path(ORG), enrollment_path(ORG), SYNC.to_owned()]
    );
    let _ = std::fs::remove_dir_all(dir);
}
