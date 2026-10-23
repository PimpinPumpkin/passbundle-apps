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
use crate::templates;
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

/// An attachment: the app says where its file is (checking the item may be
/// used), then the download, opening it, and `then` happen without the app,
/// so a long download doesn't hold up its other calls (locking, the screens'
/// reads). On a thread of the core's own, as `with_app_waiting`.
fn attachment_then(
    id: &str,
    attachment_id: &str,
    then: impl FnOnce(crate::app::AttachmentFile) -> Value + Send,
) -> Value {
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let mut to_fetch = None;
                let asked = app_value(|app| match app.attachment_to_fetch(id, attachment_id) {
                    Ok(file) => {
                        to_fetch = Some(file);
                        json!({ "ok": true })
                    }
                    Err(e) => failed(e),
                });
                match to_fetch {
                    Some(file) => catch_unwind(AssertUnwindSafe(|| then(file))).unwrap_or_else(
                        |_| json!({ "ok": false, "error": "The core stopped unexpectedly." }),
                    ),
                    None => asked,
                }
            })
            .join()
            .unwrap_or_else(|_| json!({ "ok": false, "error": "The core stopped unexpectedly." }))
    })
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
    // Test builds (the test-hooks feature) can keep the refresh token in
    // memory, never in the OS's store; a release always uses the OS's.
    #[cfg(feature = "test-hooks")]
    if std::env::var("PB_SECRETS").as_deref() == Ok("memory") {
        return Box::new(MemorySecretStore::default());
    }
    Box::new(OsSecretStore::new(dir))
}

fn failed(e: Error) -> Value {
    json!({
        "ok": false,
        "error": e.to_string(),
        "signedOut": e == Error::SignedOut,
        // Making an account got to the server: signing in is what's left.
        "accountMade": matches!(e, Error::AccountMade { .. }),
    })
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
        ItemKind::BankAccount => "bankAccount",
        ItemKind::DriversLicense => "driversLicense",
        ItemKind::Passport => "passport",
        ItemKind::Other(_) => "other",
    }
}

/// Where an item is, as the web vault's list shows it beside each item: an
/// organization's item by its first shared folder (or the organization), a
/// person's own by its folder, or Personal.
fn place(vault: &Vault, item: &Item) -> String {
    if let Some(org) = &item.organization_id {
        let mut folders: Vec<&String> = item
            .collection_ids
            .iter()
            .filter_map(|c| vault.collections.get(c))
            .collect();
        folders.sort_by_key(|n| n.to_lowercase());
        return folders
            .first()
            .map(|n| (*n).clone())
            .or_else(|| vault.organizations.get(org).cloned())
            .unwrap_or_default();
    }
    item.folder_id
        .as_ref()
        .and_then(|f| vault.folders.get(f))
        .cloned()
        .unwrap_or_else(|| "Personal".to_owned())
}

/// What the person may do with an item, for the screens to offer only what
/// the server allows (docs/core.md, "Folders and permissions").
fn permissions(vault: &Vault, item: &Item) -> Value {
    let grid = vault.grid(item);
    json!({
        "edit": grid.edit,
        "seePasswords": grid.see_passwords,
        "delete": grid.delete,
        "share": grid.share,
        "viewHistory": grid.view_history,
        "clearHistory": grid.view_history && vault.may_clear_history(item),
    })
}

fn summary(vault: &Vault, item: &Item) -> Value {
    let template = templates::template_of(item);
    json!({
        "id": item.id,
        "kind": kind(item.kind),
        "category": templates::category(item),
        "name": item.name,
        "subtitle": item.card.as_ref().map(|c| c.summary())
            .or_else(|| item.identity.as_ref().map(|i| i.full_name()))
            .or_else(|| item.ssh_key.as_ref().and_then(|k| k.fingerprint.clone()))
            .or_else(|| item.details.iter().find(|d| !d.secret).map(|d| d.value.as_str().to_owned()))
            .or_else(|| item.username.clone())
            .or_else(|| item.uris.first().cloned())
            .unwrap_or_default(),
        "website": item.uris.first(),
        // The host its tile's icon is for (icons.rs), for a login.
        "iconHost": (item.kind == ItemKind::Login).then(|| item.uris.first().and_then(|u| crate::icons::host_of(u))).flatten(),
        "favorite": item.favorite,
        "folderId": item.folder_id,
        "folder": item.folder_id.as_ref().and_then(|f| vault.folders.get(f)),
        "organizationId": item.organization_id,
        "organization": item.organization_id.as_ref().and_then(|o| vault.organizations.get(o)),
        "collectionIds": item.collection_ids,
        "where": place(vault, item),
        "template": template.map(|(_, name, _)| name),
        // A custom template (a note that defines one), and a copy an MSP
        // keeps in its clients' Templates folder, which lists show only
        // while that folder is open.
        "isTemplate": templates::definition_of(item).is_some(),
        "mspCopy": templates::is_msp_copy(item),
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

/// Makes an account on the server, then signs in to it (docs/core.md,
/// "Making an account"): the same answer as `pb_sign_in`. `name` and `hint`
/// may be empty.
///
/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_sign_up(
    server: *const c_char,
    email: *const c_char,
    name: *const c_char,
    password: *const c_char,
    hint: *const c_char,
) -> *mut c_char {
    let (server, email, name) = (text(server), text(email), text(name));
    let (password, hint) = (Zeroizing::new(text(password)), Zeroizing::new(text(hint)));
    let given = |t: &str| Some(t.to_owned()).filter(|t| !t.trim().is_empty());
    let (name, hint) = (given(&name), given(&hint).map(Zeroizing::new));
    // The slow part (the master key, an RSA key pair) away from the app, so
    // nothing else waits on it; a server address that can't be used first.
    let made = crate::server::Server::new(
        &server,
        crate::server::Device::this_one(&crate::server::Device::new_identifier()),
    )
    .and_then(|_| {
        crate::signup::NewAccount::make(
            &email,
            name.as_deref(),
            &password,
            hint.as_deref().map(|h| h.as_str()),
        )
    });
    let account = match made {
        Ok(account) => account,
        Err(e) => return out(failed(e)),
    };
    with_app_waiting(|app| step(app.register(&server, &account, &password)))
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

/// Saves the generator's settings to the server now, or takes its newer ones
/// (synced_settings): the apps call it when the generator closes.
#[no_mangle]
pub extern "C" fn pb_sync_settings() -> *mut c_char {
    with_app_waiting(|app| done(app.sync_settings()))
}

#[no_mangle]
pub extern "C" fn pb_lock() -> *mut c_char {
    with_app(|app| {
        app.lock();
        state(app)
    })
}

/// An item's value for the clipboard, for the phone app's native piece
/// (android.rs, or `pb_secret` for iOS's Swift): never a secret the
/// person's shared folder hides from them (`App::value_to_copy`). None while
/// locked, while a slow call has the app, or if there's nothing to copy.
#[cfg(phone)]
pub(crate) fn value_for_copy(id: &str, what: &str) -> Option<Zeroizing<String>> {
    let guard = match APP.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return None,
    };
    guard.as_ref()?.value_to_copy(id, what).ok()
}

/// An item's value for Android's autofill to fill (android.rs): as for
/// copying, except that a secret a shared folder hides still fills, as
/// upstream's apps fill it, without the person seeing it.
#[cfg(target_os = "android")]
pub(crate) fn value_for_fill(id: &str, what: &str) -> Option<Zeroizing<String>> {
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
/// `onSystemLock`, and `signOut`; and `removeUnlockWithPin`, from their
/// "Remove Unlock with PIN" policy. All off when no organization sets them.
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

/// Where items live, as the web vault's menu has it (places.rs): the
/// person's own folders (Personal's), and each organization with its shared
/// folders, by name, each nested inside the one its name starts with, with
/// what the person can do there (`grid`; `canCreate` means new items may go
/// in it). Each has `id`, `name` (the whole name), `label` (what it's called
/// where it sits), `depth`, and `parentId`.
#[no_mangle]
pub extern "C" fn pb_places() -> *mut c_char {
    with_app(|app| {
        let Some(vault) = app.vault() else {
            return json!({ "folders": [], "organizations": [] });
        };
        let nested = |entries: Vec<(String, String)>| crate::places::nest(&entries);
        let folders: Vec<Value> = nested(
            vault
                .folders
                .iter()
                .map(|(id, name)| (id.clone(), name.clone()))
                .collect(),
        )
        .into_iter()
        .map(|n| json!({ "id": n.id, "name": n.name, "label": n.label, "depth": n.depth, "parentId": n.parent }))
        .collect();
        let mut organizations: Vec<(&String, &String)> = vault.organizations.iter().collect();
        organizations.sort_by_key(|(_, name)| name.to_lowercase());
        let organizations: Vec<Value> = organizations
            .into_iter()
            .map(|(id, name)| {
                let collections: Vec<Value> = nested(
                    vault
                        .collection_orgs
                        .iter()
                        .filter(|(_, org)| *org == id)
                        .filter_map(|(c, _)| vault.collections.get(c).map(|n| (c.clone(), n.clone())))
                        .collect(),
                )
                .into_iter()
                .map(|n| {
                    let grid = vault.collection_grids.get(&n.id).copied().unwrap_or_default();
                    json!({
                        "id": n.id, "name": n.name, "label": n.label, "depth": n.depth, "parentId": n.parent,
                        "grid": grid, "canCreate": grid.create,
                    })
                })
                .collect();
                json!({ "id": id, "name": name, "collections": collections })
            })
            .collect();
        json!({ "folders": folders, "organizations": organizations })
    })
}

/// Templates to make new items from, grouped as the web vault's New item
/// picker groups them (templates.rs): `[{name, templates: [{id, name, base,
/// category, fields: [{label, kind, value}], source, from}]}]`.
#[no_mangle]
pub extern "C" fn pb_templates() -> *mut c_char {
    with_app(|app| match app.vault() {
        Some(vault) => json!(templates::groups(vault, |item| app.may_use(item))),
        None => json!([{ "name": "Built-in", "templates": templates::built_in() }]),
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
        view["permissions"] = permissions(vault, item);
        view["created"] = json!(item.creation_date);
        view["revisionDate"] = json!(item.revision_date);
        view["passwordChanged"] = json!(item.kept.password_revision_date);
        // How strong its password is (health.rs), for someone who may see it.
        if let Some(password) = item.password.as_ref().filter(|p| {
            !p.is_empty() && item.kind == ItemKind::Login && vault.grid(item).see_passwords
        }) {
            let hints: Vec<&str> = [item.username.as_deref(), Some(item.name.as_str())]
                .into_iter()
                .flatten()
                .collect();
            view["strength"] = json!(crate::health::word(crate::health::strength(
                password, &hints
            )));
        }
        if let Some((id, name, category)) = templates::template_of(item) {
            view["template"] = json!({ "id": id, "name": name, "category": category });
        }
        if let Some(d) = templates::definition_of(item) {
            view["definition"] = json!({ "base": d.base, "from": d.from });
        }
        // A bank account's, driver's license's, or passport's values; the
        // secrets show only when revealed (`pb_reveal` with their key).
        view["details"] = item
            .details
            .iter()
            .map(|d| json!({ "key": d.key, "label": d.label, "secret": d.secret, "value": if d.secret { None } else { Some(d.value.as_str()) } }))
            .collect();
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
            .map(
                |a| json!({ "id": a.id, "name": a.name, "size": a.size, "opens": a.key.is_some() }),
            )
            .collect();
        // A login's passkey: who it's for, never its key.
        if let Some(passkey) = &item.passkey {
            view["passkey"] = json!({
                "rpId": passkey.rp_id,
                "rpName": passkey.rp_name,
                "userName": passkey.user_name,
                "created": passkey.creation_date,
                "usable": passkey.usable,
            });
        }
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
        // The app edits logins, notes, cards, and identities, where the
        // person may; the web vault edits SSH keys and the newer kinds.
        view["editable"] = json!(
            matches!(
                item.kind,
                ItemKind::Login | ItemKind::SecureNote | ItemKind::Card | ItemKind::Identity
            ) && vault.grid(item).edit
                && vault.grid(item).see_passwords
        );
        view
    })
}

/// Favorites an item or not (`favorite` "true" or "false"), then syncs:
/// `{ok}`, or the words to show.
///
/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_set_favorite(
    id: *const c_char,
    favorite: *const c_char,
) -> *mut c_char {
    let (id, favorite) = (text(id), text(favorite) == "true");
    with_app_waiting(|app| done(app.set_favorite(&id, favorite)))
}

/// How the app looks on this device: `{showIcons}`.
#[no_mangle]
pub extern "C" fn pb_appearance() -> *mut c_char {
    with_app(|app| json!(app.appearance()))
}

/// Changes how the app looks: `{showIcons}`. Turning icons off forgets the
/// ones kept.
///
/// # Safety
/// `appearance` is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_set_appearance(appearance: *const c_char) -> *mut c_char {
    let appearance = serde_json::from_str::<crate::store::Appearance>(&text(appearance));
    with_app(|app| match appearance {
        Ok(a) => done(app.set_appearance(a)),
        Err(_) => failed(Error::Damaged("those choices")),
    })
}

/// A website's icon for a login's tile (icons.rs): `{ok, path}`, the file
/// it's kept in, or `{ok, path: null}` where there's none (or icons are off,
/// or the vault is locked). From the server's icon service the first time,
/// away from the app's other calls, then from this device for a week.
///
/// # Safety
/// `host` is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_icon(host: *const c_char) -> *mut c_char {
    let host = text(host);
    // Only a host name the core would make, so it's a safe file name too.
    if crate::icons::host_of(&format!("https://{host}")).as_deref() != Some(host.as_str()) {
        return out(json!({ "ok": false, "error": "not a website's host" }));
    }
    let mut source = None;
    let _ = app_value(|app| {
        source = app.icon_source();
        Value::Null
    });
    let Some((server, folder)) = source else {
        return out(json!({ "ok": true, "path": null }));
    };
    if let Some(kept) = crate::icons::kept(&folder, &host) {
        return out(json!({ "ok": true, "path": kept.map(|p| p.to_string_lossy().into_owned()) }));
    }
    out(waiting(|| {
        let fetched = crate::server::Server::new(&server, crate::server::Device::this_one(""))
            .and_then(|s| s.icon(&host));
        match fetched {
            Ok(icon) => {
                let path = crate::icons::keep(&folder, &host, icon.as_deref());
                json!({ "ok": true, "path": path.map(|p| p.to_string_lossy().into_owned()) })
            }
            Err(e) => failed(e),
        }
    }))
}

/// The logins with something to fix (health.rs): `{ok, items: [{id, weak,
/// reused, unsecured}]}`. Judged away from the app's other calls.
#[no_mangle]
pub extern "C" fn pb_health() -> *mut c_char {
    let mut logins = None;
    let _ = app_value(|app| {
        logins = app.vault().map(crate::health::logins);
        Value::Null
    });
    let Some(logins) = logins else {
        return out(json!({ "ok": true, "items": [] }));
    };
    out(waiting(
        || json!({ "ok": true, "items": crate::health::report(&logins) }),
    ))
}

/// An item's past versions (history.rs), from the server: `{ok,
/// hidePasswords, canRestore, canClear, unreadable, versions: [{id,
/// revisionDate, savedBy, opens, restorable, changes, rows}]}`, newest
/// first, without the versions that look the same as the one before
/// (`history::distinct`). A secret's values in `rows` are null
/// (`pb_item_version` shows them).
///
/// # Safety
/// `id` is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_history(id: *const c_char) -> *mut c_char {
    let id = text(id);
    with_app_waiting(|app| match app.history(&id) {
        Ok(_) => history_view(app, &id),
        Err(e) => failed(e),
    })
}

fn history_view(app: &App, id: &str) -> Value {
    let (Some(vault), Some(history)) = (app.vault(), app.kept_history(id)) else {
        return failed(Error::Damaged("no such item"));
    };
    let Some(current) = vault.item(id) else {
        return failed(Error::Damaged("no such item"));
    };
    let grid = vault.grid(current);
    let editable = matches!(
        current.kind,
        ItemKind::Login | ItemKind::SecureNote | ItemKind::Card | ItemKind::Identity
    );
    let can_restore = editable && grid.edit && grid.see_passwords && !current.deleted;
    let opened: Vec<&Item> = history
        .versions
        .iter()
        .filter_map(|v| v.item.as_ref())
        .collect();
    let listed = crate::history::distinct(&opened, current);
    let mut opened_at = 0;
    let mut versions = Vec::new();
    for v in &history.versions {
        let saved_by = crate::history::saved_by(&v.saved_by, &vault.user_id);
        let base = json!({ "id": v.id, "revisionDate": v.revision_date, "savedBy": saved_by });
        match &v.item {
            None if history.hide_passwords => versions.push(base),
            None => {}
            Some(item) => {
                let show = listed[opened_at];
                opened_at += 1;
                if !show {
                    continue;
                }
                let rows = crate::history::rows(item, current, false);
                let mut view = base;
                view["opens"] = json!(true);
                view["changes"] = json!(rows.iter().filter(|r| r.changed).count());
                view["restorable"] = json!(
                    can_restore
                        && item.kind == current.kind
                        && item.organization_id == current.organization_id
                        && item.kept.key == current.kept.key
                );
                view["rows"] = json!(rows);
                versions.push(view);
            }
        }
    }
    let unreadable = if history.hide_passwords {
        0
    } else {
        history.versions.iter().filter(|v| v.item.is_none()).count()
    };
    json!({
        "ok": true,
        "hidePasswords": history.hide_passwords,
        "canRestore": can_restore,
        "canClear": grid.view_history && vault.may_clear_history(current),
        "unreadable": unreadable,
        "versions": versions,
    })
}

/// One version beside the item now, with its secrets when `reveal` is
/// "true" (and the person may see them): `{ok, rows}`. From the history
/// `pb_history` asked for last.
///
/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_item_version(
    id: *const c_char,
    version_id: *const c_char,
    reveal: *const c_char,
) -> *mut c_char {
    let (id, version_id, reveal) = (text(id), text(version_id), text(reveal) == "true");
    with_app(|app| {
        let (Some(vault), Some(history)) = (app.vault(), app.kept_history(&id)) else {
            return failed(Error::Damaged("no such version"));
        };
        let Some(current) = vault.item(&id) else {
            return failed(Error::Damaged("no such item"));
        };
        let Some(version) = history
            .versions
            .iter()
            .find(|v| v.id == version_id)
            .and_then(|v| v.item.as_ref())
        else {
            return failed(Error::Damaged("no such version"));
        };
        let reveal = reveal && vault.grid(current).see_passwords && app.may_use(current);
        json!({ "ok": true, "rows": crate::history::rows(version, current, reveal) })
    })
}

/// Restores one of an item's versions as a normal edit, then syncs: `{ok}`,
/// or the words to show. From the history `pb_history` asked for last.
///
/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_restore_version(
    id: *const c_char,
    version_id: *const c_char,
) -> *mut c_char {
    let (id, version_id) = (text(id), text(version_id));
    with_app_waiting(|app| done(app.restore_version(&id, &version_id)))
}

/// Clears an item's history (`version_id` empty) or deletes one version:
/// `{ok}`, or the words to show.
///
/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_delete_history(
    id: *const c_char,
    version_id: *const c_char,
) -> *mut c_char {
    let (id, version_id) = (text(id), text(version_id));
    with_app_waiting(|app| {
        done(app.delete_history(&id, Some(version_id.as_str()).filter(|v| !v.is_empty())))
    })
}

/// Makes a folder: `{ok, id}`, or the words to show.
///
/// # Safety
/// `name` is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_create_folder(name: *const c_char) -> *mut c_char {
    let name = text(name);
    with_app_waiting(|app| match app.create_folder(&name) {
        Ok(id) => json!({ "ok": true, "id": id }),
        Err(e) => failed(e),
    })
}

/// Renames a folder: `{ok}`, or the words to show.
///
/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_rename_folder(id: *const c_char, name: *const c_char) -> *mut c_char {
    let (id, name) = (text(id), text(name));
    with_app_waiting(|app| done(app.rename_folder(&id, &name)))
}

/// Deletes a folder; its items stay, in no folder: `{ok}`, or the words to show.
///
/// # Safety
/// `id` is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_delete_folder(id: *const c_char) -> *mut c_char {
    let id = text(id);
    with_app_waiting(|app| done(app.delete_folder(&id)))
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
    out(attachment_then(&id, &attachment_id, |file| {
        match file.save() {
            Ok(path) => json!({ "ok": true, "path": path.to_string_lossy() }),
            Err(e) => failed(e),
        }
    }))
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
    out(attachment_then(&id, &attachment_id, |file| {
        match file.save_in(std::path::Path::new(&dir)) {
            Ok(path) => json!({ "ok": true, "path": path.to_string_lossy() }),
            Err(e) => failed(e),
        }
    }))
}

/// Writes an attachment into a file Android opened for it (android.rs):
/// `{ok}`, or the words to show. Slow: off the main thread.
#[cfg(target_os = "android")]
pub(crate) fn write_attachment_to(id: &str, attachment_id: &str, fd: i32) -> Value {
    attachment_then(id, attachment_id, |file| done(file.write_to(fd)))
}

/// The logins with a passkey for a site (its ID), for a phone's list of
/// passkeys (docs/phones.md, "Passkeys"): `{locked}`, or `{items: [{id, name,
/// userName, userDisplayName, credentialId, userHandle, discoverable,
/// reprompt}]}`, the IDs in base64url. Never a key.
///
/// # Safety
/// `rp_id` is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_passkeys(rp_id: *const c_char) -> *mut c_char {
    out(passkeys(&text(rp_id)))
}

/// `pb_passkeys`, for Android's JNI too.
pub(crate) fn passkeys(rp_id: &str) -> Value {
    app_value(|app| {
        if app.vault().is_none() {
            return json!({ "locked": true });
        }
        let items: Vec<Value> = app
            .passkeys_for(rp_id)
            .into_iter()
            .filter_map(|item| {
                let passkey = item.passkey.as_ref()?;
                let raw = crate::passkey::credential_id(&passkey.credential_id).ok()?;
                Some(json!({
                    "id": item.id,
                    "name": item.name,
                    "userName": passkey.user_name,
                    "userDisplayName": passkey.user_display_name,
                    "credentialId": base64url(&raw),
                    "userHandle": passkey.user_handle,
                    // A site that names no credential gets only these.
                    "discoverable": passkey.discoverable,
                    "reprompt": item.kept.reprompt != 0,
                }))
            })
            .collect();
        json!({ "locked": false, "items": items })
    })
}

/// Runs a slow call on a thread of the core's own, as `with_app_waiting`.
fn waiting(call: impl FnOnce() -> Value + Send) -> Value {
    std::thread::scope(|scope| {
        scope
            .spawn(call)
            .join()
            .unwrap_or_else(|_| json!({ "ok": false, "error": "The core stopped unexpectedly." }))
    })
}

/// Signs in to a site with a login's passkey (docs/core.md, "Passkeys"):
/// `{ok, authenticatorData, signature, credentialId, userHandle}` in
/// base64url, or the words to show. `client_data_hash` is base64url (32
/// bytes); `user_verified` is "true" when the person proved who they are in
/// this sign-in. Waits on the network when the passkey counts its uses.
///
/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_passkey_assert(
    id: *const c_char,
    rp_id: *const c_char,
    client_data_hash: *const c_char,
    user_verified: *const c_char,
) -> *mut c_char {
    let (id, rp_id, hash, verified) = (
        text(id),
        text(rp_id),
        text(client_data_hash),
        text(user_verified),
    );
    out(waiting(|| {
        passkey_assert(&id, &rp_id, &hash, verified == "true")
    }))
}

/// `pb_passkey_assert`, for Android's JNI too (off the main thread).
pub(crate) fn passkey_assert(
    id: &str,
    rp_id: &str,
    client_data_hash: &str,
    user_verified: bool,
) -> Value {
    use base64::Engine;
    let hash: Option<[u8; 32]> = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(client_data_hash.trim_end_matches('='))
        .ok()
        .and_then(|h| h.try_into().ok());
    let Some(hash) = hash else {
        return failed(Error::Damaged("a client data hash that isn't 32 bytes"));
    };
    app_value(
        |app| match app.passkey_assert(id, rp_id, &hash, user_verified) {
            Ok(a) => json!({
                "ok": true,
                "authenticatorData": base64url(&a.authenticator_data),
                "signature": base64url(&a.signature),
                "credentialId": base64url(&a.credential_id),
                // None stored: left out, as WebAuthn has it, rather than empty.
                "userHandle": (!a.user_handle.is_empty()).then(|| base64url(&a.user_handle)),
            }),
            Err(e) => failed(e),
        },
    )
}

/// Makes a passkey for a site (docs/core.md, "Passkeys", "Making one"), from
/// `{rpId, rpName, userHandle, userName, userDisplayName, userVerified,
/// discoverable, login, exclude}` (IDs and the user handle in base64url;
/// `login` the item it goes in, or null for a new login): `{ok, itemId,
/// credentialId, attestationObject, authenticatorData, publicKey}` in
/// base64url, or the words to show. Waits on the network: it's saved first.
///
/// # Safety
/// `request` is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_passkey_create(request: *const c_char) -> *mut c_char {
    let request = text(request);
    out(waiting(|| passkey_create(&request)))
}

/// `pb_passkey_create`, for Android's JNI too (off the main thread).
pub(crate) fn passkey_create(request: &str) -> Value {
    use base64::Engine;
    let decode = |text: &str| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(text.trim_end_matches('='))
            .ok()
    };
    let r: Value = serde_json::from_str(request).unwrap_or(Value::Null);
    let (Some(rp_id), Some(user_handle)) = (
        r["rpId"].as_str().map(str::to_owned),
        r["userHandle"].as_str().and_then(decode),
    ) else {
        return failed(Error::Damaged(
            "a passkey request without a site or an account",
        ));
    };
    let exclude: Vec<Vec<u8>> = r["exclude"]
        .as_array()
        .map(|ids| {
            ids.iter()
                .filter_map(|id| id.as_str().and_then(decode))
                .collect()
        })
        .unwrap_or_default();
    let text_of = |key: &str| r[key].as_str().map(str::to_owned);
    let (rp_name, user_name, user_display_name, login) = (
        text_of("rpName"),
        text_of("userName"),
        text_of("userDisplayName"),
        text_of("login"),
    );
    let user_verified = r["userVerified"].as_bool() == Some(true);
    let discoverable = r["discoverable"].as_bool() != Some(false);
    app_value(|app| {
        match app.passkey_create(&crate::app::NewPasskey {
            rp_id: &rp_id,
            rp_name: rp_name.as_deref(),
            user_handle: &user_handle,
            user_name: user_name.as_deref(),
            user_display_name: user_display_name.as_deref(),
            user_verified,
            discoverable,
            login: login.as_deref(),
            exclude: &exclude,
        }) {
            Ok(made) => json!({
                "ok": true,
                "itemId": made.item_id,
                "credentialId": base64url(&made.credential_id),
                "attestationObject": base64url(&made.attestation_object),
                "authenticatorData": base64url(&made.authenticator_data),
                "publicKey": base64url(&made.public_key),
            }),
            Err(e) => {
                // The site's own "already registered" (InvalidStateError).
                let excluded = e == Error::Refused(crate::app::ALREADY_HAS_PASSKEY);
                let mut answer = failed(e);
                answer["excluded"] = json!(excluded);
                answer
            }
        }
    })
}

/// The master password as proof the person is who they are, for a passkey
/// (Android's JNI): `{ok}`, or the words to show. Slow (the KDF).
#[cfg(target_os = "android")]
pub(crate) fn check_master_password(password: &str) -> Value {
    app_value(|app| done(app.check_master_password(password)))
}

fn base64url(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
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
        // Never a secret someone may not see (their shared folder hides passwords).
        if item.is_secret(&what) && app.vault().is_some_and(|v| !v.grid(item).see_passwords) {
            return Value::Null;
        }
        if let Some(detail) = item.details.iter().find(|d| d.key == what) {
            return json!(detail.value.as_str());
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
        // Not for someone whose shared folder hides the item's passwords.
        let Some(secret) = app
            .vault()
            .and_then(|v| v.item(&id).filter(|i| v.grid(i).see_passwords))
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
/// value, type}], folderId, organizationId, collectionIds, favorite,
/// template: {id, name}}`: a field's `type` is upstream's (0 text, 1
/// hidden, 2 yes or no), and `template` names the template a new item is
/// made from. For an organization's item being edited, `collectionIds` are
/// the shared folders it's to be in.
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
                .map(|f| {
                    // Upstream's field type (0 text, 1 hidden, 2 yes or no), or `hidden`.
                    let kind =
                        f["type"]
                            .as_u64()
                            .unwrap_or(if f["hidden"].as_bool() == Some(true) {
                                1
                            } else {
                                0
                            });
                    let value = f["value"].as_str().unwrap_or_default().to_owned();
                    Field {
                        name: f["name"].as_str().unwrap_or_default().to_owned(),
                        value: Zeroizing::new(if kind == 2 && value != "true" {
                            "false".to_owned()
                        } else {
                            value
                        }),
                        hidden: kind == 1,
                        boolean: kind == 2,
                    }
                })
                .collect(),
            folder_id: string("folderId"),
            organization_id: string("organizationId"),
            favorite: d["favorite"].as_bool().unwrap_or(false),
            template: match (d["template"]["id"].as_str(), d["template"]["name"].as_str()) {
                (Some(id), Some(name)) if !id.is_empty() && !name.trim().is_empty() => {
                    Some((id.to_owned(), name.trim().to_owned()))
                }
                _ => None,
            },
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
