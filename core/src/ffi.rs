//! The app's bridge: plain C functions for Flutter's dart:ffi, around one
//! `App`. No key crosses it, and a secret crosses only when the person reveals
//! it or edits it (docs/core.md, "Memory" and "The bridge to the app").
//! Strings in and out are UTF-8 JSON; free every returned string with
//! `pb_free`. Calls that take a while (signing in, unlocking, syncing,
//! saving) are made from a background isolate; the app doesn't read while one
//! is running.

use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use zeroize::Zeroizing;

use crate::agent::Status;
use crate::app::{App, State, Step};
use crate::error::Error;
use crate::generator;
use crate::secrets::{MemorySecretStore, OsSecretStore, SecretStore};
use crate::store::Store;
use crate::totp::Totp;
use crate::vault::{Field, Item, ItemDraft, ItemKind, Vault};

static APP: Mutex<Option<App>> = Mutex::new(None);

fn text(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    // SAFETY: the caller passes a NUL-terminated UTF-8 string it owns for the call.
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

fn out(value: Value) -> *mut c_char {
    CString::new(value.to_string())
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

/// Runs a call on the app, turning a panic into an error instead of unwinding into Dart.
fn with_app(call: impl FnOnce(&mut App) -> Value) -> *mut c_char {
    out(catch_unwind(AssertUnwindSafe(|| {
        let mut guard = APP.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let app =
            guard.get_or_insert_with(|| App::open(Store::new(Store::default_dir()), secrets()));
        call(app)
    }))
    .unwrap_or_else(|_| json!({ "ok": false, "error": "The core stopped unexpectedly." })))
}

fn secrets() -> Box<dyn SecretStore> {
    // Tests and CI keep the refresh token in memory, never in the OS's store.
    if std::env::var("PB_SECRETS").as_deref() == Ok("memory") {
        Box::new(MemorySecretStore::default())
    } else {
        Box::new(OsSecretStore)
    }
}

fn failed(e: Error) -> Value {
    json!({ "ok": false, "error": e.to_string(), "signedOut": e == Error::SignedOut })
}

fn done(result: crate::Result<()>) -> Value {
    match result {
        Ok(()) => json!({ "ok": true }),
        Err(e) => failed(e),
    }
}

fn step(result: crate::Result<Step>) -> Value {
    match result {
        Ok(Step::Done) => json!({ "ok": true, "step": "done" }),
        Ok(Step::TwoFactor(providers)) => {
            json!({ "ok": true, "step": "twoFactor", "providers": providers })
        }
        Err(e) => failed(e),
    }
}

fn state(app: &App) -> Value {
    match app.state() {
        State::SignedOut { server, email } => {
            json!({ "state": "signedOut", "server": server, "email": email })
        }
        State::Locked { server, email } => {
            json!({ "state": "locked", "server": server, "email": email })
        }
        State::Unlocked { server, email } => {
            json!({ "state": "unlocked", "server": server, "email": email })
        }
    }
}

fn kind(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Login => "login",
        ItemKind::SecureNote => "note",
        ItemKind::Card => "card",
        ItemKind::Identity => "identity",
        ItemKind::SshKey => "sshKey",
        ItemKind::Other(_) => "other",
    }
}

fn summary(vault: &Vault, item: &Item) -> Value {
    json!({
        "id": item.id,
        "kind": kind(item.kind),
        "name": item.name,
        "subtitle": item.username.clone().or_else(|| item.uris.first().cloned()).unwrap_or_default(),
        "website": item.uris.first(),
        "favorite": item.favorite,
        "folderId": item.folder_id,
        "folder": item.folder_id.as_ref().and_then(|f| vault.folders.get(f)),
        "organizationId": item.organization_id,
        "organization": item.organization_id.as_ref().and_then(|o| vault.organizations.get(o)),
    })
}

/// Opens the app on a folder of its own instead of the usual one, with the
/// refresh token kept in memory when `memory_secrets`: for tests and tools.
///
/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_open(dir: *const c_char, memory_secrets: bool) -> *mut c_char {
    let dir = text(dir);
    let secrets: Box<dyn SecretStore> = if memory_secrets {
        Box::new(MemorySecretStore::default())
    } else {
        Box::new(OsSecretStore)
    };
    *APP.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) =
        Some(App::open(Store::new(dir), secrets));
    with_app(|app| state(app))
}

/// Talks to the agent whose home folder is `home` and whose program is
/// `program`, instead of the one installed next to the app: for tests.
///
/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_use_agent_home(
    home: *const c_char,
    program: *const c_char,
) -> *mut c_char {
    let (home, program) = (text(home), text(program));
    with_app(|app| {
        let address = passbundle_ipc::address_for(
            std::ffi::OsStr::new(&home),
            crate::brand::NAME,
            crate::brand::DOMAIN,
        );
        app.use_agent_at(address, program.into());
        json!({ "ok": true })
    })
}

/// What unlocking with the OS's biometrics is called here.
fn method() -> &'static str {
    if cfg!(target_os = "windows") {
        "Windows Hello"
    } else if cfg!(target_os = "macos") {
        "Touch ID"
    } else {
        "biometrics"
    }
}

/// Whether this account can unlock with Windows Hello or Touch ID here:
/// "setUp", "notSetUp", "unavailable", or "noAgent" (the desktop agent isn't
/// running), with the method's name.
#[no_mangle]
pub extern "C" fn pb_biometric_status() -> *mut c_char {
    with_app(|app| {
        let status = match app.biometric_status() {
            Ok(Status::SetUp) => "setUp",
            Ok(Status::NotSetUp) => "notSetUp",
            Ok(Status::Unavailable) => "unavailable",
            Err(Error::NoAgent) => "noAgent",
            Err(_) => "unavailable",
        };
        json!({ "status": status, "method": method() })
    })
}

/// Turns it on (the agent asks); `on` says whether the person agreed.
#[no_mangle]
pub extern "C" fn pb_biometric_enroll() -> *mut c_char {
    with_app(|app| match app.enroll_biometric() {
        Ok(on) => json!({ "ok": true, "on": on }),
        Err(e) => failed(e),
    })
}

/// Unlocks with it (the agent asks); `unlocked` is false if the person cancelled.
#[no_mangle]
pub extern "C" fn pb_biometric_unlock() -> *mut c_char {
    with_app(|app| match app.unlock_with_biometric() {
        Ok(unlocked) => json!({ "ok": true, "unlocked": unlocked }),
        Err(e) => failed(e),
    })
}

/// Turns it off, for the browser extension too.
#[no_mangle]
pub extern "C" fn pb_biometric_remove() -> *mut c_char {
    with_app(|app| done(app.remove_biometric()))
}

#[no_mangle]
pub extern "C" fn pb_version() -> *mut c_char {
    out(json!(crate::version()))
}

/// Signed out, locked, or unlocked, with the server and email.
#[no_mangle]
pub extern "C" fn pb_state() -> *mut c_char {
    with_app(|app| state(app))
}

/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_sign_in(
    server: *const c_char,
    email: *const c_char,
    password: *const c_char,
) -> *mut c_char {
    let (server, email, password) = (text(server), text(email), Zeroizing::new(text(password)));
    with_app(|app| step(app.sign_in(&server, &email, &password)))
}

/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_finish_sign_in(
    provider: u8,
    code: *const c_char,
    remember: bool,
) -> *mut c_char {
    let code = text(code);
    with_app(|app| step(app.finish_sign_in(provider, &code, remember)))
}

#[no_mangle]
pub extern "C" fn pb_send_email_code() -> *mut c_char {
    with_app(|app| done(app.send_email_code()))
}

/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_unlock(password: *const c_char) -> *mut c_char {
    let password = Zeroizing::new(text(password));
    with_app(|app| {
        let started = Instant::now();
        match app.unlock(&password) {
            Ok(()) => json!({ "ok": true, "ms": started.elapsed().as_millis() as u64 }),
            Err(e) => failed(e),
        }
    })
}

#[no_mangle]
pub extern "C" fn pb_sync() -> *mut c_char {
    with_app(|app| done(app.sync()))
}

#[no_mangle]
pub extern "C" fn pb_lock() -> *mut c_char {
    with_app(|app| {
        app.lock();
        state(app)
    })
}

#[no_mangle]
pub extern "C" fn pb_sign_out() -> *mut c_char {
    with_app(|app| {
        app.sign_out();
        state(app)
    })
}

/// Folders, and organizations with their collections, for the screens and the editor.
#[no_mangle]
pub extern "C" fn pb_places() -> *mut c_char {
    with_app(|app| {
        let Some(vault) = app.vault() else {
            return json!({ "folders": [], "organizations": [] });
        };
        let by_name = |v: &Value| v["name"].as_str().unwrap_or_default().to_lowercase();
        let mut folders: Vec<Value> = vault
            .folders
            .iter()
            .map(|(id, name)| json!({ "id": id, "name": name }))
            .collect();
        folders.sort_by_key(by_name);
        let mut organizations: Vec<Value> = vault
            .organizations
            .iter()
            .map(|(id, name)| {
                let mut collections: Vec<Value> = vault
                    .collection_orgs
                    .iter()
                    .filter(|(_, org)| *org == id)
                    .filter_map(|(c, _)| {
                        vault
                            .collections
                            .get(c)
                            .map(|name| json!({ "id": c, "name": name }))
                    })
                    .collect();
                collections.sort_by_key(by_name);
                json!({ "id": id, "name": name, "collections": collections })
            })
            .collect();
        organizations.sort_by_key(by_name);
        json!({ "folders": folders, "organizations": organizations })
    })
}

/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_list(query: *const c_char) -> *mut c_char {
    let query = text(query);
    with_app(|app| match app.vault() {
        Some(vault) => Value::Array(
            vault
                .search(&query)
                .into_iter()
                .map(|i| summary(vault, i))
                .collect(),
        ),
        None => json!([]),
    })
}

/// One item, as its view shows it before anything is revealed.
///
/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_item(id: *const c_char) -> *mut c_char {
    let id = text(id);
    with_app(|app| {
        let Some(vault) = app.vault() else {
            return Value::Null;
        };
        let Some(item) = vault.item(&id) else {
            return Value::Null;
        };
        let mut view = summary(vault, item);
        view["username"] = json!(item.username);
        view["uris"] = json!(item.uris);
        view["collectionIds"] = json!(item.collection_ids);
        view["hasPassword"] = json!(item.password.is_some());
        view["hasTotp"] = json!(item.totp.is_some());
        view["hasNotes"] = json!(item.notes.is_some());
        view["fields"] = item
            .fields
            .iter()
            .map(|f| json!({ "name": f.name, "hidden": f.hidden, "value": if f.hidden { None } else { Some(f.value.as_str()) } }))
            .collect();
        view
    })
}

/// A secret, while the person reveals it or edits the item: "password",
/// "notes", "totp" (the secret itself, for the editor), or "field:<index>".
///
/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_reveal(id: *const c_char, what: *const c_char) -> *mut c_char {
    let (id, what) = (text(id), text(what));
    with_app(|app| {
        let Some(item) = app.vault().and_then(|v| v.item(&id)) else {
            return Value::Null;
        };
        let secret = match what.as_str() {
            "password" => item.password.as_ref().map(|p| p.as_str()),
            "notes" => item.notes.as_ref().map(|n| n.as_str()),
            "totp" => item.totp.as_ref().map(|t| t.as_str()),
            other => other
                .strip_prefix("field:")
                .and_then(|i| i.parse::<usize>().ok())
                .and_then(|i| item.fields.get(i))
                .map(|f| f.value.as_str()),
        };
        json!(secret)
    })
}

/// Copies a value without it passing through Dart: "username", "password",
/// "totp", "notes", or "field:<index>". Secrets clear after 30 seconds.
///
/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_copy(id: *const c_char, what: *const c_char) -> *mut c_char {
    let (id, what) = (text(id), text(what));
    with_app(|app| done(app.copy(&id, &what)))
}

/// The item's current one-time code and the seconds it has left.
///
/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_totp(id: *const c_char) -> *mut c_char {
    let id = text(id);
    with_app(|app| {
        let Some(secret) = app
            .vault()
            .and_then(|v| v.item(&id))
            .and_then(|i| i.totp.as_ref())
        else {
            return Value::Null;
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        match Totp::parse(secret) {
            Ok(totp) => {
                json!({ "code": totp.code_at(now), "secondsLeft": totp.seconds_left(now), "period": totp.period() })
            }
            Err(e) => json!({ "error": e.to_string() }),
        }
    })
}

/// Saves an item from the editor: `id` empty for a new one. The draft is
/// `{name, note, username, password, totp, notes, uris, fields: [{name,
/// value, hidden}], folderId, organizationId, collectionIds, favorite}`.
///
/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_save(id: *const c_char, draft: *const c_char) -> *mut c_char {
    let id = text(id);
    let draft_json = Zeroizing::new(text(draft));
    with_app(|app| {
        let d: Value = match serde_json::from_str(&draft_json) {
            Ok(d) => d,
            Err(e) => {
                return failed(Error::Malformed(format!(
                    "a draft the core can't read ({e})"
                )))
            }
        };
        let string = |key: &str| d[key].as_str().filter(|s| !s.is_empty()).map(str::to_owned);
        let secret = |key: &str| string(key).map(Zeroizing::new);
        let draft = ItemDraft {
            note: d["note"].as_bool().unwrap_or(false),
            name: string("name").unwrap_or_default(),
            notes: secret("notes"),
            username: string("username"),
            password: secret("password"),
            totp: secret("totp"),
            uris: d["uris"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|u| u.as_str())
                .filter(|u| !u.trim().is_empty())
                .map(|u| u.trim().to_owned())
                .collect(),
            fields: d["fields"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|f| Field {
                    name: f["name"].as_str().unwrap_or_default().to_owned(),
                    value: Zeroizing::new(f["value"].as_str().unwrap_or_default().to_owned()),
                    hidden: f["hidden"].as_bool().unwrap_or(false),
                })
                .collect(),
            folder_id: string("folderId"),
            organization_id: string("organizationId"),
            favorite: d["favorite"].as_bool().unwrap_or(false),
        };
        if draft.name.trim().is_empty() {
            return failed(Error::Refused("an item needs a name"));
        }
        let collections: Vec<String> = d["collectionIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c.as_str().map(str::to_owned))
            .collect();
        if draft.organization_id.is_some() && collections.is_empty() {
            return failed(Error::Refused("an organization's item needs a collection"));
        }
        done(app.save(
            Some(id.as_str()).filter(|i| !i.is_empty()),
            &draft,
            &collections,
        ))
    })
}

/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_trash(id: *const c_char) -> *mut c_char {
    let id = text(id);
    with_app(|app| done(app.trash(&id)))
}

/// A new password for the editor (it goes into the field, so the person sees it).
///
/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_generate(options: *const c_char) -> *mut c_char {
    let o: Value = serde_json::from_str(&text(options)).unwrap_or(Value::Null);
    let defaults = generator::Options::default();
    let flag = |key: &str, default: bool| o[key].as_bool().unwrap_or(default);
    let options = generator::Options {
        length: o["length"]
            .as_u64()
            .map(|l| l as usize)
            .unwrap_or(defaults.length),
        uppercase: flag("uppercase", defaults.uppercase),
        lowercase: flag("lowercase", defaults.lowercase),
        digits: flag("digits", defaults.digits),
        symbols: flag("symbols", defaults.symbols),
        avoid_ambiguous: flag("avoidAmbiguous", defaults.avoid_ambiguous),
    };
    out(match generator::password(&options) {
        Ok(p) => json!({ "ok": true, "password": p.as_str() }),
        Err(e) => failed(e),
    })
}

/// Frees a string the core returned.
///
/// # Safety
/// `ptr` is null or a string this core returned, not yet freed.
#[no_mangle]
pub unsafe extern "C" fn pb_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        // SAFETY: ptr came from CString::into_raw in this module and is freed once.
        drop(unsafe { CString::from_raw(ptr) });
    }
}
