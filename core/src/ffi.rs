//! The app's bridge: plain C functions for Flutter's dart:ffi, around one
//! `App`. No key crosses it, and a secret crosses only when the person reveals
//! it or edits it (docs/core.md, "Memory" and "The bridge to the app").
//! Strings in and out are UTF-8 JSON; free every returned string with
//! `pb_free`. Calls that take a while (signing in, unlocking, syncing,
//! saving) are made from a background isolate; the app doesn't read while one
//! is running.

use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use zeroize::Zeroizing;

use crate::agent::Status;
use crate::app::{App, State, Step};
use crate::autolock::{self, Preferences};
use crate::error::Error;
use crate::generator;
use crate::secrets::{MemorySecretStore, OsSecretStore, SecretStore};
use crate::store::Store;
use crate::totp::Totp;
use crate::vault::{
    Card, Field, Identity, Item, ItemDraft, ItemKind, Vault, EXPIRES_FIELD, SECRET_DETAILS,
};

static APP: Mutex<Option<App>> = Mutex::new(None);
/// The OS said the computer is going to sleep while a slow call had the app:
/// the app hears it as soon as that call is done.
static SLEEP_NOTICE: AtomicBool = AtomicBool::new(false);
/// The same for the app leaving the screen (a phone's "Immediately").
static LEFT_NOTICE: AtomicBool = AtomicBool::new(false);

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
    out(app_value(call))
}

fn app_value(call: impl FnOnce(&mut App) -> Value) -> Value {
    catch_unwind(AssertUnwindSafe(|| {
        let guard = APP.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        on_app(guard, call)
    }))
    .unwrap_or_else(|_| json!({ "ok": false, "error": "The core stopped unexpectedly." }))
}

/// `with_app`, for calls that wait on the network or the desktop agent's
/// socket: run on a thread of the core's own. Dart signals the threads that
/// run Dart (its profiler, in debug builds), and a signal arriving in a
/// blocking read or connect ends it with "Interrupted system call".
fn with_app_waiting(call: impl FnOnce(&mut App) -> Value + Send) -> *mut c_char {
    out(std::thread::scope(|scope| {
        scope
            .spawn(|| app_value(call))
            .join()
            .unwrap_or_else(|_| json!({ "ok": false, "error": "The core stopped unexpectedly." }))
    }))
}

/// `with_app`, but None instead of waiting while a slow call (a sync, say) has the app.
fn with_app_now(call: impl FnOnce(&mut App) -> Value) -> Option<*mut c_char> {
    let guard = match APP.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return None,
    };
    Some(out(catch_unwind(AssertUnwindSafe(|| on_app(guard, call)))
        .unwrap_or_else(
            |_| json!({ "ok": false, "error": "The core stopped unexpectedly." }),
        )))
}

fn on_app(mut guard: MutexGuard<'_, Option<App>>, call: impl FnOnce(&mut App) -> Value) -> Value {
    let app = guard.get_or_insert_with(|| {
        let dir = Store::default_dir();
        App::open(Store::new(&dir), secrets(&dir))
    });
    notices(app);
    let value = call(app);
    notices(app);
    value
}

/// What the OS said while a slow call had the app.
fn notices(app: &mut App) {
    if SLEEP_NOTICE.swap(false, Ordering::SeqCst) {
        app.sleeping();
    }
    if LEFT_NOTICE.swap(false, Ordering::SeqCst) {
        app.left();
    }
}

fn secrets(dir: &std::path::Path) -> Box<dyn SecretStore> {
    // Tests and CI keep the refresh token in memory, never in the OS's store.
    if std::env::var("PB_SECRETS").as_deref() == Ok("memory") {
        Box::new(MemorySecretStore::default())
    } else {
        Box::new(OsSecretStore::new(dir))
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
        "subtitle": item.card.as_ref().map(|c| c.summary())
            .or_else(|| item.identity.as_ref().map(|i| i.full_name()))
            .or_else(|| item.ssh_key.as_ref().and_then(|k| k.fingerprint.clone()))
            .or_else(|| item.username.clone())
            .or_else(|| item.uris.first().cloned())
            .unwrap_or_default(),
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
        Box::new(OsSecretStore::new(std::path::Path::new(&dir)))
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
    with_app_waiting(|app| {
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

/// Starts the desktop agent installed next to the app if it isn't running
/// (its tray, and Windows Hello). `started` says whether it started one.
#[no_mangle]
pub extern "C" fn pb_start_agent() -> *mut c_char {
    out(match crate::agent::Agent::start_installed() {
        Ok(started) => json!({ "ok": true, "started": started }),
        Err(e) => failed(e),
    })
}

/// Turns it on (the agent asks); `on` says whether the person agreed.
#[no_mangle]
pub extern "C" fn pb_biometric_enroll() -> *mut c_char {
    with_app_waiting(|app| match app.enroll_biometric() {
        Ok(on) => json!({ "ok": true, "on": on }),
        Err(e) => failed(e),
    })
}

/// Unlocks with it (the agent asks); `unlocked` is false if the person cancelled.
#[no_mangle]
pub extern "C" fn pb_biometric_unlock() -> *mut c_char {
    with_app_waiting(|app| match app.unlock_with_biometric() {
        Ok(unlocked) => json!({ "ok": true, "unlocked": unlocked }),
        Err(e) => failed(e),
    })
}

/// Turns it off, for the browser extension too.
#[no_mangle]
pub extern "C" fn pb_biometric_remove() -> *mut c_char {
    with_app_waiting(|app| done(app.remove_biometric()))
}

#[no_mangle]
pub extern "C" fn pb_version() -> *mut c_char {
    out(json!(crate::version()))
}

/// Where to sign in: `ours`, PassBundle's own server, and `managed`, the
/// server an organization set by policy (managed.rs), which then is the one.
#[no_mangle]
pub extern "C" fn pb_servers() -> *mut c_char {
    out(json!({ "ours": crate::brand::SERVER, "managed": crate::managed::server() }))
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
    with_app_waiting(|app| step(app.sign_in(&server, &email, &password)))
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
    with_app_waiting(|app| step(app.finish_sign_in(provider, &code, remember)))
}

#[no_mangle]
pub extern "C" fn pb_send_email_code() -> *mut c_char {
    with_app_waiting(|app| done(app.send_email_code()))
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
    with_app_waiting(|app| done(app.sync()))
}

#[no_mangle]
pub extern "C" fn pb_lock() -> *mut c_char {
    with_app(|app| {
        app.lock();
        state(app)
    })
}

/// An item's value for the clipboard, for the phone app's native piece
/// (android.rs, or `pb_secret` for iOS's Swift). None while locked, while a
/// slow call has the app, or if there's nothing to copy.
#[cfg(phone)]
pub(crate) fn value_for_copy(id: &str, what: &str) -> Option<Zeroizing<String>> {
    let guard = match APP.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return None,
    };
    guard.as_ref()?.value(id, what).ok()
}

/// An item's value for the iOS app's Swift to put on the pasteboard
/// ("username", "password", "totp", "notes", or "field:<index>"), as UTF-8
/// bytes, without passing through Dart; null while locked or busy, or if
/// there's nothing to copy. `len` gets its length. Free it with
/// `pb_free_secret`, which wipes it first.
///
/// # Safety
/// `id` and `what` are NUL-terminated UTF-8 strings, and `len` points to
/// writable memory, for the call.
#[cfg(target_os = "ios")]
#[no_mangle]
pub unsafe extern "C" fn pb_secret(
    id: *const c_char,
    what: *const c_char,
    len: *mut usize,
) -> *mut u8 {
    let Some(value) = value_for_copy(&text(id), &text(what)) else {
        return std::ptr::null_mut();
    };
    let bytes: Box<[u8]> = value.as_bytes().into();
    *len = bytes.len();
    Box::into_raw(bytes).cast()
}

/// Wipes and frees what `pb_secret` returned.
///
/// # Safety
/// `bytes` and `len` are what `pb_secret` returned and set, or null, and
/// aren't used after.
#[cfg(target_os = "ios")]
#[no_mangle]
pub unsafe extern "C" fn pb_free_secret(bytes: *mut u8, len: usize) {
    if bytes.is_null() {
        return;
    }
    let mut owned = Box::from_raw(std::ptr::slice_from_raw_parts_mut(bytes, len));
    zeroize::Zeroize::zeroize(&mut *owned);
}

/// The unlocked vault's user key, for the iOS app's Swift to keep in the
/// Keychain for unlocking with Face ID or Touch ID (docs/phones.md,
/// "Unlocking with biometrics"), without passing through Dart; null while
/// locked or busy. `len` gets its length. Free it with `pb_free_secret`,
/// which wipes it.
///
/// # Safety
/// `len` points to writable memory, for the call.
#[cfg(target_os = "ios")]
#[no_mangle]
pub unsafe extern "C" fn pb_user_key(len: *mut usize) -> *mut u8 {
    let Some(key) = user_key_for_biometric() else {
        return std::ptr::null_mut();
    };
    let bytes: Box<[u8]> = key.as_slice().into();
    *len = bytes.len();
    Box::into_raw(bytes).cast()
}

/// Opens the kept vault with the user key Swift read from the Keychain after
/// Face ID or Touch ID: "" if it opened, else the words to show. A key that
/// no longer opens it means biometric unlock is out of date, and is turned
/// off. Free the answer with `pb_free`.
///
/// # Safety
/// `key` points to `len` readable bytes, for the call.
#[cfg(target_os = "ios")]
#[no_mangle]
pub unsafe extern "C" fn pb_unlock_with_key(key: *const u8, len: usize) -> *mut c_char {
    let key = Zeroizing::new(std::slice::from_raw_parts(key, len).to_vec());
    let why = match unlock_with_key_for_biometric(&key) {
        Ok(()) => String::new(),
        Err(why) => why,
    };
    CString::new(why).unwrap_or_default().into_raw()
}

/// What Swift does when the core turns biometric unlock off (signing out, or
/// a key that's out of date): it deletes the Keychain item. Swift sets it at
/// start.
#[cfg(target_os = "ios")]
static REMOVE_BIOMETRIC: std::sync::OnceLock<extern "C" fn()> = std::sync::OnceLock::new();

/// Swift's function to turn biometric unlock off, set once at start.
#[cfg(target_os = "ios")]
#[no_mangle]
pub extern "C" fn pb_set_remove_biometric(remove: extern "C" fn()) {
    let _ = REMOVE_BIOMETRIC.set(remove);
}

/// Turns biometric unlock off on iOS: Swift deletes the Keychain item.
#[cfg(target_os = "ios")]
pub(crate) fn remove_biometric() {
    if let Some(remove) = REMOVE_BIOMETRIC.get() {
        remove();
    }
}

/// Opens the core on the Android app's folder, unless it's open already: the
/// autofill service can start the app's process without its screens.
#[cfg(target_os = "android")]
pub(crate) fn open_if_closed(dir: &str) {
    let mut guard = APP.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.is_none() {
        let secrets = Box::new(OsSecretStore::new(std::path::Path::new(dir)));
        *guard = Some(App::open(Store::new(dir), secrets));
    }
}

/// The logins for an Android app or the page a browser shows, for its
/// autofill (android.rs): names and usernames only. `locked` while there's no
/// open vault, and `busy` while a slow call has the app.
#[cfg(target_os = "android")]
pub(crate) fn fillable(package: &str, domain: &str) -> Value {
    use crate::matching::Target;
    let guard = match APP.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return json!({ "busy": true }),
    };
    let Some(app) = guard.as_ref() else {
        return json!({ "locked": true, "signedOut": true });
    };
    let Some(vault) = app.vault() else {
        let signed_out = matches!(app.state(), State::SignedOut { .. });
        return json!({ "locked": true, "signedOut": signed_out });
    };
    let target = if domain.is_empty() {
        Target::App(package)
    } else {
        Target::Website(domain)
    };
    let items: Vec<Value> = vault
        .matching(&target)
        .iter()
        .map(|i| json!({ "id": i.id, "name": i.name, "username": i.username, "reprompt": i.kept.reprompt != 0 }))
        .collect();
    json!({ "locked": false, "items": items })
}

/// The master password, for a login that asks for it again before Android's
/// autofill fills it (android.rs): `{ok}`, or the words to show.
#[cfg(target_os = "android")]
pub(crate) fn confirm_master_password(id: &str, password: &str) -> Value {
    app_value(|app| done(app.confirm_master_password(id, password)))
}

/// The unlocked vault's user key, for the phone app to seal for biometric
/// unlock (android.rs, or `pb_user_key` for iOS's Keychain). None while locked
/// or busy.
#[cfg(phone)]
pub(crate) fn user_key_for_biometric() -> Option<Zeroizing<Vec<u8>>> {
    let guard = match APP.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return None,
    };
    Some(Zeroizing::new(
        guard.as_ref()?.vault()?.user_key().to_bytes().to_vec(),
    ))
}

/// Opens the kept vault with the user key the phone app's biometric unlock
/// opened (android.rs, or `pb_unlock_with_key` for iOS); the error's words
/// otherwise.
#[cfg(phone)]
pub(crate) fn unlock_with_key_for_biometric(key: &[u8]) -> Result<(), String> {
    let mut guard = match APP.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return Err("PassBundle is busy; try again.".into()),
    };
    let app = guard.as_mut().ok_or_else(|| Error::SignedOut.to_string())?;
    let key = crate::crypto::SymmetricKey::from_bytes(key).map_err(|e| e.to_string())?;
    match app.unlock_with_key(key) {
        // Out of date (the account's keys changed): it's turned off.
        Err(e @ Error::Refused(_)) => {
            let _ = app.remove_biometric();
            Err(e.to_string())
        }
        other => other.map_err(|e| e.to_string()),
    }
}

fn unlocked(app: &App) -> bool {
    matches!(app.state(), State::Unlocked { .. })
}

/// When to lock by itself: `lockAfterMinutes` (null: only when the app
/// closes) and `lockOnSleep` (when the screen locks or the computer sleeps).
#[no_mangle]
pub extern "C" fn pb_preferences() -> *mut c_char {
    with_app(|app| json!(app.preferences()))
}

/// The organizations' "Vault lock" policy (docs/core.md, "Locking by
/// policy"): `maxMinutes` (the longest wait, or null), `immediately`,
/// `onSystemLock`, and `signOut`. All off when no organization sets it.
#[no_mangle]
pub extern "C" fn pb_lock_policy() -> *mut c_char {
    with_app(|app| json!(app.lock_policy()))
}

/// # Safety
/// `preferences` is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_set_preferences(preferences: *const c_char) -> *mut c_char {
    let preferences = serde_json::from_str::<Preferences>(&text(preferences));
    with_app(|app| match preferences {
        Ok(preferences) => done(app.set_preferences(preferences)),
        Err(_) => failed(Error::Damaged("those choices")),
    })
}

/// The person used the app just now. Skipped while a slow call has the app.
#[no_mangle]
pub extern "C" fn pb_used() -> *mut c_char {
    with_app_now(|app| {
        app.used();
        json!({ "ok": true })
    })
    .unwrap_or_else(|| out(json!({ "ok": true })))
}

/// Locks if it's time to (docs/core.md, "Locking by itself"). The app asks
/// every two seconds while unlocked: `locked` says whether the vault is locked
/// now, and `reason` why it just locked ("idle", "screenLocked", "slept").
/// While a slow call has the app, `busy` is true, and the next tick decides.
#[no_mangle]
pub extern "C" fn pb_tick() -> *mut c_char {
    // Asked outside the app's lock: on Linux it's a D-Bus call.
    let readings = autolock::readings();
    with_app_now(|app| {
        let reason = app.tick(&readings);
        json!({ "busy": false, "locked": !unlocked(app), "reason": reason.map(|r| r.name()) })
    })
    .unwrap_or_else(|| out(json!({ "busy": true, "locked": false })))
}

/// The app left the screen (a phone's app switcher or home): locks now if the
/// person chose "Immediately". `locked` says whether the vault is locked now.
#[no_mangle]
pub extern "C" fn pb_left() -> *mut c_char {
    with_app_now(|app| {
        app.left();
        json!({ "busy": false, "locked": !unlocked(app) })
    })
    .unwrap_or_else(|| {
        LEFT_NOTICE.store(true, Ordering::SeqCst);
        out(json!({ "busy": true, "locked": false }))
    })
}

/// The OS says the computer is about to sleep: locks now if the person chose
/// that, or as soon as a slow call that has the app is done.
#[no_mangle]
pub extern "C" fn pb_sleeping() -> *mut c_char {
    with_app_now(|app| {
        app.sleeping();
        json!({ "busy": false, "locked": !unlocked(app) })
    })
    .unwrap_or_else(|| {
        SLEEP_NOTICE.store(true, Ordering::SeqCst);
        out(json!({ "busy": true, "locked": false }))
    })
}

#[no_mangle]
pub extern "C" fn pb_sign_out() -> *mut c_char {
    with_app_waiting(|app| {
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
        // An item that asks for the master password shows its name until
        // it's given, as upstream's apps do.
        view["reprompt"] = json!(item.kept.reprompt != 0);
        view["mayUse"] = json!(app.may_use(item));
        if !app.may_use(item) {
            return view;
        }
        view["grant"] = json!(app.grant());
        view["username"] = json!(item.username);
        view["uris"] = json!(item.uris);
        view["collectionIds"] = json!(item.collection_ids);
        view["hasPassword"] = json!(item.password.is_some());
        view["hasTotp"] = json!(item.totp.is_some());
        view["hasNotes"] = json!(item.notes.is_some());
        // Each field's upstream type: 0 text, 1 hidden, 2 yes or no, 3 linked.
        view["fields"] = item
            .fields
            .iter()
            .zip(&item.kept.field_kinds)
            .map(|(f, (kind, _))| json!({ "name": f.name, "hidden": f.hidden, "type": kind, "reserved": f.reserved(), "editable": f.edited_by_the_app(*kind), "value": if f.hidden || f.reserved() { None } else { Some(f.value.as_str()) } }))
            .collect();
        // The expiration date the web vault or the extension set, if any.
        view["expires"] = json!(item
            .fields
            .iter()
            .find(|f| f.name == EXPIRES_FIELD)
            .map(|f| f.value.as_str()));
        // A card's and an identity's plain details, and which secrets they have.
        if let Some(card) = &item.card {
            view["card"] = json!({
                "cardholderName": card.cardholder_name,
                "brand": card.brand,
                "expiration": card.expiration(),
                "expMonth": card.exp_month,
                "expYear": card.exp_year,
                "hasNumber": card.number.as_ref().is_some_and(|n| !n.is_empty()),
                "hasCode": card.code.as_ref().is_some_and(|c| !c.is_empty()),
            });
        }
        view["attachments"] = item
            .attachments
            .iter()
            .map(|a| json!({ "id": a.id, "name": a.name, "size": a.size }))
            .collect();
        if let Some(key) = &item.ssh_key {
            view["sshKey"] = json!({
                "publicKey": key.public_key,
                "fingerprint": key.fingerprint,
                "hasPrivateKey": key.private_key.as_ref().is_some_and(|k| !k.is_empty()),
            });
        }
        if let Some(identity) = &item.identity {
            let has = |s: &Option<Zeroizing<String>>| s.as_ref().is_some_and(|s| !s.is_empty());
            view["identity"] = json!({
                "fullName": Some(identity.full_name()).filter(|n| !n.is_empty()),
                "title": identity.title,
                "firstName": identity.first_name,
                "middleName": identity.middle_name,
                "lastName": identity.last_name,
                "address1": identity.address1,
                "address2": identity.address2,
                "address3": identity.address3,
                "city": identity.city,
                "state": identity.state,
                "postalCode": identity.postal_code,
                "country": identity.country,
                "company": identity.company,
                "email": identity.email,
                "phone": identity.phone,
                "username": identity.username,
                "address": identity.address(),
                "hasSsn": has(&identity.ssn),
                "hasPassportNumber": has(&identity.passport_number),
                "hasLicenseNumber": has(&identity.license_number),
            });
        }
        // The app edits logins, notes, cards, and identities; the web vault edits SSH keys.
        view["editable"] = json!(matches!(
            item.kind,
            ItemKind::Login | ItemKind::SecureNote | ItemKind::Card | ItemKind::Identity
        ));
        view
    })
}

/// Saves one of an item's attachments to the Downloads folder (docs/core.md,
/// "Attachments"): `{ok, path}`, or the words to show.
///
/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_save_attachment(
    id: *const c_char,
    attachment_id: *const c_char,
) -> *mut c_char {
    let (id, attachment_id) = (text(id), text(attachment_id));
    with_app_waiting(|app| match app.save_attachment(&id, &attachment_id) {
        Ok(path) => json!({ "ok": true, "path": path.to_string_lossy() }),
        Err(e) => failed(e),
    })
}

/// Saves one of an item's attachments in `dir` (an iPhone's Documents folder,
/// which Files shows): `{ok, path}`, or the words to show.
///
/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_save_attachment_in(
    id: *const c_char,
    attachment_id: *const c_char,
    dir: *const c_char,
) -> *mut c_char {
    let (id, attachment_id, dir) = (text(id), text(attachment_id), text(dir));
    with_app_waiting(|app| {
        match app.save_attachment_in(&id, &attachment_id, std::path::Path::new(&dir)) {
            Ok(path) => json!({ "ok": true, "path": path.to_string_lossy() }),
            Err(e) => failed(e),
        }
    })
}

/// Writes an attachment into a file Android opened for it (android.rs):
/// `{ok}`, or the words to show. Slow: off the main thread.
#[cfg(target_os = "android")]
pub(crate) fn write_attachment_to(id: &str, attachment_id: &str, fd: i32) -> Value {
    app_value(|app| done(app.write_attachment_to(id, attachment_id, fd)))
}

/// Checks the master password for an item that asks for it again (docs/core.md,
/// "Asking for the master password again"): `{ok}`, or the words to show.
///
/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_confirm_master_password(
    id: *const c_char,
    password: *const c_char,
) -> *mut c_char {
    let id = text(id);
    let password = Zeroizing::new(text(password));
    with_app_waiting(|app| done(app.confirm_master_password(&id, &password)))
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
        if !app.may_use(item) {
            return Value::Null;
        }
        let secret = match what.as_str() {
            "password" => item.password.as_ref().map(|p| p.as_str()),
            "notes" => item.notes.as_ref().map(|n| n.as_str()),
            "totp" => item.totp.as_ref().map(|t| t.as_str()),
            // A card's or an identity's secrets.
            what if SECRET_DETAILS.contains(&what) => {
                return json!(item.detail(what).as_deref().map(String::as_str));
            }
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
            .filter(|i| app.may_use(i))
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
    with_app_waiting(|app| {
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
        // A card's or an identity's details, by upstream's names.
        let part = |kind: &str, key: &str| {
            d[kind][key]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let card = d["card"].is_object().then(|| Card {
            cardholder_name: part("card", "cardholderName"),
            brand: part("card", "brand"),
            exp_month: part("card", "expMonth"),
            exp_year: part("card", "expYear"),
            number: part("card", "number").map(Zeroizing::new),
            code: part("card", "code").map(Zeroizing::new),
        });
        let identity = d["identity"].is_object().then(|| Identity {
            title: part("identity", "title"),
            first_name: part("identity", "firstName"),
            middle_name: part("identity", "middleName"),
            last_name: part("identity", "lastName"),
            address1: part("identity", "address1"),
            address2: part("identity", "address2"),
            address3: part("identity", "address3"),
            city: part("identity", "city"),
            state: part("identity", "state"),
            postal_code: part("identity", "postalCode"),
            country: part("identity", "country"),
            company: part("identity", "company"),
            email: part("identity", "email"),
            phone: part("identity", "phone"),
            username: part("identity", "username"),
            ssn: part("identity", "ssn").map(Zeroizing::new),
            passport_number: part("identity", "passportNumber").map(Zeroizing::new),
            license_number: part("identity", "licenseNumber").map(Zeroizing::new),
        });
        let draft = ItemDraft {
            note: d["note"].as_bool().unwrap_or(false),
            card,
            identity,
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
        // An item that asks for the master password saves only from an
        // editor opened while it was given (its `grant`), whose fields hold
        // the item's secrets rather than blanks.
        let asks = app
            .vault()
            .and_then(|v| v.item(&id))
            .is_some_and(|item| item.kept.reprompt != 0);
        if asks && d["grant"].as_u64() != Some(app.grant()) {
            return failed(Error::MasterPasswordFirst);
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
    with_app_waiting(|app| done(app.trash(&id)))
}

/// A new password or passphrase (`generator::Options`, as JSON; missing
/// choices take the defaults, and null takes the last choices), held to the
/// organizations' policy. The choices become the next ones on this device.
/// `{ok, password, options}`, with the choices it was made with.
///
/// # Safety
/// `options` is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_generate(options: *const c_char) -> *mut c_char {
    let given = text(options);
    let options = match given.trim() {
        "" | "null" => None,
        given => match serde_json::from_str::<generator::Options>(given) {
            Ok(options) => Some(options),
            Err(_) => return out(failed(Error::Damaged("those choices"))),
        },
    };
    with_app(|app| {
        let options = options.unwrap_or_else(|| app.generator_options());
        match app.generate(&options) {
            Ok((p, held)) => json!({ "ok": true, "password": p.as_str(), "options": held }),
            Err(e) => failed(e),
        }
    })
}

/// The generator's last choices on this device (`generator::Options`), held
/// to the organizations' policy.
#[no_mangle]
pub extern "C" fn pb_generator_options() -> *mut c_char {
    with_app(|app| json!(app.generator_options()))
}

/// The organizations' "Password generator" policy (docs/core.md, "New
/// passwords"): `kind` (the only kind allowed, or null), `minLength`,
/// `uppercase`, `lowercase`, `digits`, `symbols`, `minDigits`,
/// `minSymbols`, `minWords`, `capitalize`, and `includeNumber`. Nothing set
/// when no organization sets it.
#[no_mangle]
pub extern "C" fn pb_generator_policy() -> *mut c_char {
    with_app(|app| json!(app.generator_policy()))
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
