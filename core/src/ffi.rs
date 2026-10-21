//! The prototype's bridge: plain C functions for Flutter's dart:ffi. The real
//! app uses flutter_rust_bridge (docs/core.md, "The bridge to the app"), with
//! the same rule: no key crosses, and a secret crosses only when the person
//! reveals it. Strings in and out are UTF-8 (JSON where structured); free
//! every returned string with `pb_free`.

use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::kdf::Prelogin;
use crate::totp::Totp;
use crate::vault::{Item, ItemKind, Sync, Vault};

static VAULT: Mutex<Option<Vault>> = Mutex::new(None);

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

/// Runs a call, turning a panic into an error answer instead of unwinding into Dart.
fn guarded(call: impl FnOnce() -> Value) -> *mut c_char {
    out(catch_unwind(AssertUnwindSafe(call))
        .unwrap_or_else(|_| json!({ "ok": false, "error": "The core stopped unexpectedly." })))
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
        "folder": item.folder_id.as_ref().and_then(|f| vault.folders.get(f)),
        "organization": item.organization_id.as_ref().and_then(|o| vault.organizations.get(o)),
    })
}

#[no_mangle]
pub extern "C" fn pb_version() -> *mut c_char {
    out(json!(crate::version()))
}

/// Unlocks a sync (as the server sent it) with the prelogin answer and the master password.
///
/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_unlock(
    sync_json: *const c_char,
    prelogin_json: *const c_char,
    password: *const c_char,
) -> *mut c_char {
    let (sync_json, prelogin_json) = (text(sync_json), text(prelogin_json));
    let password = zeroize::Zeroizing::new(text(password));
    guarded(|| {
        let started = Instant::now();
        let opened = Sync::from_json(&sync_json).and_then(|sync| {
            let prelogin: Prelogin = serde_json::from_str(&prelogin_json).map_err(|e| {
                crate::Error::Malformed(format!("a prelogin answer the core can't read ({e})"))
            })?;
            Vault::unlock(&sync, &prelogin, &password)
        });
        match opened {
            Ok(vault) => {
                let answer = json!({
                    "ok": true,
                    "ms": started.elapsed().as_millis() as u64,
                    "email": vault.email,
                    "count": vault.items.len(),
                    "damaged": vault.damaged.len(),
                });
                *VAULT.lock().unwrap() = Some(vault);
                answer
            }
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        }
    })
}

/// Items matching a search (empty for all): what lists show, no secrets.
///
/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_list(query: *const c_char) -> *mut c_char {
    let query = text(query);
    guarded(|| match &*VAULT.lock().unwrap() {
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
    guarded(|| {
        let guard = VAULT.lock().unwrap();
        let Some(item) = guard.as_ref().and_then(|v| v.item(&id)) else {
            return Value::Null;
        };
        let mut view = summary(guard.as_ref().unwrap(), item);
        view["username"] = json!(item.username);
        view["uris"] = json!(item.uris);
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

/// A secret, while the person reveals it: "password", "notes", or "field:<index>".
///
/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_reveal(id: *const c_char, what: *const c_char) -> *mut c_char {
    let (id, what) = (text(id), text(what));
    guarded(|| {
        let guard = VAULT.lock().unwrap();
        let Some(item) = guard.as_ref().and_then(|v| v.item(&id)) else {
            return Value::Null;
        };
        let secret = match what.as_str() {
            "password" => item.password.as_ref().map(|p| p.as_str()),
            "notes" => item.notes.as_ref().map(|n| n.as_str()),
            other => other
                .strip_prefix("field:")
                .and_then(|i| i.parse::<usize>().ok())
                .and_then(|i| item.fields.get(i))
                .map(|f| f.value.as_str()),
        };
        json!(secret)
    })
}

/// The item's current one-time code and the seconds it has left.
///
/// # Safety
/// Every pointer is null or a NUL-terminated UTF-8 string, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn pb_totp(id: *const c_char) -> *mut c_char {
    let id = text(id);
    guarded(|| {
        let guard = VAULT.lock().unwrap();
        let Some(secret) = guard
            .as_ref()
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

/// Locks: drops every key and everything decrypted.
#[no_mangle]
pub extern "C" fn pb_lock() {
    if let Ok(mut vault) = VAULT.lock() {
        *vault = None;
    }
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
