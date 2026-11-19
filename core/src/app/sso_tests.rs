//! Single sign-on through the app (sso.rs, `App::start_sso`), against a
//! stand-in for the server on this computer that answers as PassBundle's
//! does: its identifier checked, the browser's answer taken at the
//! computer's listener or a phone's link, the code redeemed once with the
//! sign-in's own verifier and redirect, two-step login with the same code,
//! and the master password opening the vectors' account key. The core's
//! tests against a real server and the Entra stand-in are core/tests/server.rs.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::*;
use crate::secrets::MemorySecretStore;

/// The vectors' account and sample sync.
fn vectors() -> Value {
    serde_json::from_str(include_str!("../../tests/vectors/core-v1.json")).unwrap()
}

/// What the stand-in answers the token request with, in turn.
#[derive(Clone)]
enum TokenAnswer {
    /// The vectors' account, signed in.
    SignedIn,
    /// Two-step login, with an authenticator app.
    TwoStep,
    /// A refusal, in the server's words.
    Refused(&'static str),
    /// An account that has no master password yet.
    NoMasterPassword,
}

/// What the stand-in heard: each path, and each token request's form.
#[derive(Default)]
struct Heard {
    paths: Vec<String>,
    forms: Vec<std::collections::HashMap<String, String>>,
}

/// A stand-in for the server: prevalidate (only `acme` is an
/// organization's), the domain lookup (example.test is acme's), the token
/// endpoint answering `answers` in turn, and the vectors' sync.
fn stand_in(answers: Vec<TokenAnswer>) -> (String, Arc<Mutex<Heard>>) {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let url = format!("http://{}", server.server_addr().to_ip().unwrap());
    let heard = Arc::new(Mutex::new(Heard::default()));
    let log = heard.clone();
    let v = vectors();
    std::thread::spawn(move || {
        let mut answers = answers.into_iter();
        for mut request in server.incoming_requests() {
            let path = request.url().to_owned();
            let mut body = String::new();
            let _ = request.as_reader().read_to_string(&mut body);
            log.lock().unwrap().paths.push(path.clone());
            let (status, answer) = if path.starts_with("/identity/sso/prevalidate") {
                if path.ends_with("domainHint=acme") {
                    (200, json!({ "token": "" }))
                } else {
                    (
                        400,
                        json!({ "message": "No organization uses that SSO identifier, or its single sign-on is turned off." }),
                    )
                }
            } else if path == "/api/organizations/domain/sso/verified" {
                let data = if body.contains("@example.test") {
                    json!([{ "organizationIdentifier": "acme" }])
                } else {
                    json!([])
                };
                (200, json!({ "object": "list", "data": data }))
            } else if path == "/identity/connect/token" {
                let form: std::collections::HashMap<String, String> = body
                    .split('&')
                    .filter_map(|pair| pair.split_once('='))
                    .map(|(k, v)| {
                        let decode = |s: &str| crate::sso::tests_decode(s);
                        (decode(k), decode(v))
                    })
                    .collect();
                log.lock().unwrap().forms.push(form);
                token_answer(
                    &v,
                    answers.next().unwrap_or(TokenAnswer::Refused("No more.")),
                )
            } else if path.starts_with("/api/sync") {
                (200, v["sync"].clone())
            } else {
                (404, Value::Null)
            };
            let _ = request.respond(
                tiny_http::Response::from_string(answer.to_string())
                    .with_status_code(status)
                    .with_header(
                        tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap(),
                    ),
            );
        }
    });
    (url, heard)
}

fn token_answer(v: &Value, answer: TokenAnswer) -> (u16, Value) {
    let account = &v["account"];
    let pre = &account["pbkdf2"]["prelogin"];
    match answer {
        TokenAnswer::SignedIn => (
            200,
            json!({
                "access_token": "a.b.c",
                "expires_in": 3600,
                "refresh_token": "the-refresh-token",
                "token_type": "Bearer",
                "Key": v["sync"]["profile"]["key"],
                "TwoFactorToken": "the-device-token",
                "UserDecryptionOptions": {
                    "HasMasterPassword": true,
                    "MasterPasswordUnlock": {
                        "Kdf": { "KdfType": 0, "Iterations": pre["kdfIterations"], "Memory": null, "Parallelism": null },
                        "MasterKeyWrappedUserKey": v["sync"]["profile"]["key"],
                        "Salt": account["salt"],
                    },
                },
            }),
        ),
        TokenAnswer::TwoStep => (
            400,
            json!({
                "error": "invalid_grant",
                "error_description": "Two factor required.",
                "TwoFactorProviders": ["0"],
                "TwoFactorProviders2": { "0": null },
            }),
        ),
        TokenAnswer::Refused(message) => (
            400,
            json!({ "error": "invalid_grant", "error_description": message, "ErrorModel": { "Message": message } }),
        ),
        TokenAnswer::NoMasterPassword => (
            200,
            json!({
                "access_token": "a.b.c",
                "expires_in": 3600,
                "refresh_token": "r",
                "UserDecryptionOptions": { "HasMasterPassword": false, "MasterPasswordUnlock": null },
            }),
        ),
    }
}

fn app() -> (App, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("pb-core-sso-{}", uuid::Uuid::new_v4().simple()));
    let app = App::open(
        Store::new(&dir),
        Box::new(Arc::new(MemorySecretStore::default())),
    );
    (app, dir)
}

/// A query parameter of the authorize address.
fn param(url: &str, name: &str) -> String {
    let query = url.split_once('?').unwrap().1;
    query
        .split('&')
        .find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            (k == name).then(|| crate::sso::tests_decode(v))
        })
        .unwrap()
}

/// The browser's answer at the computer's listener: GETs `redirect` with
/// the server's code and `state`, from another thread, as a browser would.
fn browser_answers(redirect: String, query: String) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        let address = redirect.strip_prefix("http://").unwrap();
        let (host, path) = address.split_once('/').unwrap();
        let mut stream = std::net::TcpStream::connect(host).unwrap();
        write!(
            stream,
            "GET /{path}?{query} HTTP/1.1\r\nHost: {host}\r\n\r\n"
        )
        .unwrap();
        let mut answer = String::new();
        let _ = stream.read_to_string(&mut answer);
        answer.lines().next().unwrap_or_default().to_owned()
    })
}

/// Starts a single sign-on on this computer and has the browser answer it
/// with `code`: the app's step after redeeming it, and the authorize address.
fn sign_in_here(app: &mut App, url: &str, code: &str) -> (Result<Step>, String) {
    // A computer's sign-in, on phones too, where the core's own tests run.
    let start = if cfg!(phone) {
        let server = Server::new(url, Device::this_one("d")).unwrap();
        let started = crate::sso::Started::on_this_computer("acme").unwrap();
        app.start_sso_with(url, server, "d".into(), "Ana.Lopez@example.test", started)
            .unwrap()
    } else {
        app.start_sso(url, "Ana.Lopez@example.test", " ACME ")
            .unwrap()
    };
    assert!(start.listens);
    let state = param(&start.url, "state");
    let redirect = param(&start.url, "redirect_uri");
    let wait = app.sso_wait().unwrap();
    let never = AtomicBool::new(false);
    let waited = std::thread::scope(|scope| {
        let waiting = scope.spawn(|| {
            wait.listener.wait(
                &wait.state,
                &never,
                Instant::now() + Duration::from_secs(30),
            )
        });
        // A page that found the port, with a made-up state: refused, and
        // the wait goes on.
        let forged = browser_answers(redirect.clone(), "code=forged&state=guessed".into());
        assert_eq!(forged.join().unwrap(), "HTTP/1.1 400 Bad Request");
        let browser = browser_answers(redirect, format!("code={code}&state={state}"));
        assert_eq!(browser.join().unwrap(), "HTTP/1.1 200 OK");
        waiting.join().unwrap()
    });
    (app.finish_sso(waited), start.url)
}

#[test]
fn single_sign_on_redeems_its_code_once_then_the_master_password_unlocks() {
    let (url, heard) = stand_in(vec![TokenAnswer::SignedIn]);
    let (mut app, dir) = app();
    let v = vectors();
    let (step, authorize) = sign_in_here(&mut app, &url, "the-servers-code");
    assert_eq!(
        step,
        Ok(Step::MasterPassword("ana.lopez@example.test".into()))
    );
    assert!(app.signing_in());

    // The authorize address: this app, its listener, and the identifier as typed, normalized.
    let client = if cfg!(phone) { "mobile" } else { "desktop" };
    assert!(authorize.starts_with(&format!(
        "{url}/identity/connect/authorize?client_id={client}&"
    )));
    assert_eq!(param(&authorize, "domain_hint"), "acme");
    assert_eq!(param(&authorize, "code_challenge_method"), "S256");
    assert_eq!(param(&authorize, "response_type"), "code");
    let redirect = param(&authorize, "redirect_uri");
    assert!(redirect.starts_with("http://127.0.0.1:") && redirect.ends_with("/sso"));

    // The token request: the code, the verifier for the challenge, the same redirect and client.
    let form = heard.lock().unwrap().forms[0].clone();
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["code"], "the-servers-code");
    assert_eq!(form["redirect_uri"], redirect);
    assert_eq!(form["client_id"], client);
    assert_eq!(form["scope"], "api offline_access");
    assert_eq!(
        crate::sso::challenge(&form["code_verifier"]),
        param(&authorize, "code_challenge")
    );
    assert!(!form.contains_key("password") && !form.contains_key("username"));

    // A wrong master password says so, and the right one can follow.
    assert_eq!(app.unlock_after_sso("not it"), Err(Error::WrongPassword));
    let password = v["account"]["password"].as_str().unwrap();
    assert_eq!(app.unlock_after_sso(password), Ok(Step::Done));
    assert_eq!(
        app.state(),
        State::Unlocked {
            server: url.clone(),
            email: "ana.lopez@example.test".into()
        }
    );
    assert!(!app.signing_in());
    // Kept as any sign-in is: the account's KDF settings and the device token.
    let kept = app.store.settings().unwrap();
    assert_eq!(kept.kdf_iterations, 600_000);
    assert_eq!(
        app.secrets
            .get_secret(Secret::DeviceToken, &url, "ana.lopez@example.test")
            .as_deref()
            .map(String::as_str),
        Some("the-device-token")
    );

    // The code went once, and nothing more of the sign-in is left to send.
    assert_eq!(
        app.finish_sso(Ok(Zeroizing::new("the-servers-code".into()))),
        Err(Error::Sso(crate::sso::NOT_STARTED))
    );
    assert_eq!(
        app.unlock_after_sso(password),
        Err(Error::Sso(crate::sso::NOT_STARTED))
    );
    assert_eq!(heard.lock().unwrap().forms.len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn two_step_login_after_single_sign_on_goes_back_with_the_same_code() {
    let (url, heard) = stand_in(vec![TokenAnswer::TwoStep, TokenAnswer::SignedIn]);
    let (mut app, dir) = app();
    let (step, _) = sign_in_here(&mut app, &url, "the-code");
    let Ok(Step::TwoFactor(offer)) = step else {
        panic!("two-step login should be asked for: {step:?}");
    };
    assert_eq!(offer.providers, [0]);
    assert_eq!(
        app.finish_sign_in(0, "123456", false),
        Ok(Step::MasterPassword("ana.lopez@example.test".into()))
    );
    let forms = heard.lock().unwrap().forms.clone();
    assert_eq!(forms.len(), 2);
    assert_eq!(forms[1]["grant_type"], "authorization_code");
    assert_eq!(forms[1]["code"], forms[0]["code"]);
    assert_eq!(forms[1]["code_verifier"], forms[0]["code_verifier"]);
    assert_eq!(forms[1]["redirect_uri"], forms[0]["redirect_uri"]);
    assert_eq!(forms[1]["twoFactorProvider"], "0");
    assert_eq!(forms[1]["twoFactorToken"], "123456");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_refused_code_is_never_sent_again() {
    let refusal = "ana.lopez@example.test isn't a member of Acme Dental.";
    let (url, heard) = stand_in(vec![TokenAnswer::Refused(refusal)]);
    let (mut app, dir) = app();
    let (step, _) = sign_in_here(&mut app, &url, "the-code");
    assert_eq!(step, Err(Error::Server(refusal.into())));
    assert!(!app.signing_in());
    assert_eq!(
        app.finish_sso(Ok(Zeroizing::new("the-code".into()))),
        Err(Error::Sso(crate::sso::NOT_STARTED))
    );
    assert_eq!(heard.lock().unwrap().forms.len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_account_without_a_master_password_is_sent_to_the_web_vault() {
    let (url, _) = stand_in(vec![TokenAnswer::NoMasterPassword]);
    let (mut app, dir) = app();
    let (step, _) = sign_in_here(&mut app, &url, "the-code");
    assert_eq!(
        step,
        Err(Error::SsoNoMasterPassword {
            web_vault: url.clone()
        })
    );
    assert!(!app.signing_in());
    assert!(matches!(app.state(), State::SignedOut { .. }));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_identifier_no_organization_uses_or_none_is_refused_before_the_browser() {
    let (url, heard) = stand_in(vec![]);
    let (mut app, dir) = app();
    assert_eq!(
        app.start_sso(&url, "ana@example.test", "globex").err(),
        Some(Error::Sso(crate::sso::NO_SUCH_ORGANIZATION))
    );
    assert_eq!(
        app.start_sso(&url, "ana@example.test", "not an id").err(),
        Some(Error::Sso(crate::sso::NOT_AN_IDENTIFIER))
    );
    assert_eq!(
        app.start_sso(&url, "ana@example.test", "").err(),
        Some(Error::Sso(crate::sso::NO_IDENTIFIER))
    );
    assert_eq!(
        app.start_sso(&url, " ", "acme").err(),
        Some(Error::Sso(crate::sso::EMAIL_FIRST))
    );
    assert!(!app.signing_in());
    // Only the identifier the device took was asked about.
    assert_eq!(
        heard.lock().unwrap().paths,
        ["/identity/sso/prevalidate?domainHint=globex"]
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_email_finds_its_organization() {
    let (url, _) = stand_in(vec![]);
    let server = Server::new(&url, Device::this_one("")).unwrap();
    assert_eq!(
        server.sso_identifier_for("Ana@Example.test"),
        Ok(Some("acme".into()))
    );
    assert_eq!(server.sso_identifier_for("ana@elsewhere.test"), Ok(None));
}

#[test]
fn a_phones_link_comes_back_to_the_sign_in_it_was_for() {
    let (url, heard) = stand_in(vec![TokenAnswer::SignedIn]);
    let (mut app, dir) = app();
    let server = Server::new(&url, Device::this_one("d")).unwrap();
    let started = crate::sso::Started::on_a_phone("acme");
    let start = app
        .start_sso_with(&url, server, "d".into(), "ana.lopez@example.test", started)
        .unwrap();
    assert!(!start.listens);
    assert_eq!(
        param(&start.url, "redirect_uri"),
        crate::sso::PHONE_REDIRECT
    );
    let state = param(&start.url, "state");
    // Another sign-in's link changes nothing.
    assert_eq!(
        app.finish_sso_link("com.passbundle.app:/sso?code=c&state=another"),
        Err(Error::Sso(crate::sso::ANOTHER_SIGN_IN))
    );
    assert!(app.signing_in());
    assert_eq!(
        app.finish_sso_link(&format!(
            "com.passbundle.app:/sso?code=phone-code&state={state}"
        )),
        Ok(Step::MasterPassword("ana.lopez@example.test".into()))
    );
    let form = heard.lock().unwrap().forms[0].clone();
    assert_eq!(form["code"], "phone-code");
    assert_eq!(form["redirect_uri"], crate::sso::PHONE_REDIRECT);

    // A browser closed without an answer ends the sign-in.
    let server = Server::new(&url, Device::this_one("d")).unwrap();
    let started = crate::sso::Started::on_a_phone("acme");
    app.start_sso_with(&url, server, "d".into(), "ana.lopez@example.test", started)
        .unwrap();
    assert_eq!(
        app.finish_sso_link(""),
        Err(Error::Sso(crate::sso::CANCELED))
    );
    assert!(!app.signing_in());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_canceled_wait_ends_the_sign_in() {
    let (url, _) = stand_in(vec![]);
    let (mut app, dir) = app();
    let server = Server::new(&url, Device::this_one("d")).unwrap();
    let started = crate::sso::Started::on_this_computer("acme").unwrap();
    app.start_sso_with(&url, server, "d".into(), "ana@example.test", started)
        .unwrap();
    let wait = app.sso_wait().unwrap();
    // Once: the listener is the wait's.
    assert!(app.sso_wait().is_err());
    let canceled = AtomicBool::new(true);
    let waited = wait.listener.wait(&wait.state, &canceled, wait.until);
    assert_eq!(
        app.finish_sso(waited),
        Err(Error::Sso(crate::sso::CANCELED))
    );
    assert!(!app.signing_in());
    let _ = std::fs::remove_dir_all(dir);
}
