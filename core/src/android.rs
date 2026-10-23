//! The Android app's calls into the core, and the core's into the app
//! (docs/phones.md, "Talking to the OS"), through JNI. Kotlin calls in
//! (`com.passbundle.app.Core`); the core calls out only to the Keystore, which
//! keeps the refresh token's key (`com.passbundle.app.Keystore`).

use std::sync::OnceLock;

use jni::errors::{Result as JniResult, ThrowRuntimeExAndDefault};
use jni::jni_mangle;
#[cfg(feature = "ffi")]
use jni::objects::JString;
use jni::objects::{Global, JByteArray, JClass, JClassLoader, JObject, JValue};
use jni::strings::JNIStr;
#[cfg(feature = "ffi")]
use jni::sys::jstring;
use jni::{jni_sig, jni_str, Env, EnvUnowned, JavaVM};
use zeroize::Zeroizing;

/// The JVM and the app's class loader, which finds the app's classes from any thread.
static JVM: OnceLock<(JavaVM, Global<JClassLoader<'static>>)> = OnceLock::new();

/// At start, before anything reaches the network: Android's own certificate
/// checks need the app's context, and the Keystore needs the app's classes.
#[jni_mangle("com.passbundle.app.Core")]
pub fn init<'caller>(
    mut unowned_env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    context: JObject<'caller>,
) {
    unowned_env
        .with_env(|env| -> JniResult<()> {
            if JVM.get().is_none() {
                let loader = env
                    .call_method(
                        &context,
                        jni_str!("getClassLoader"),
                        jni_sig!(() -> JClassLoader),
                        &[],
                    )?
                    .l()?;
                let loader = env.cast_local::<JClassLoader>(loader)?;
                let _ = JVM.set((env.get_java_vm()?, env.new_global_ref(loader)?));
            }
            rustls_platform_verifier::android::init_with_env(env, context)
        })
        .resolve::<ThrowRuntimeExAndDefault>();
}

/// An item's value for autofill to fill ("username", "password", "totp",
/// or "field:<index>"), as UTF-8 bytes Kotlin zeroes after use; null while
/// locked or busy, or if there's nothing to fill. It goes from here into
/// the other app's field without passing through Dart. A secret the
/// person's shared folder hides still fills, as upstream's apps fill it.
#[cfg(feature = "ffi")]
#[jni_mangle("com.passbundle.app.Core")]
pub fn value<'caller>(
    mut unowned_env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    id: JString<'caller>,
    what: JString<'caller>,
) -> JByteArray<'caller> {
    unowned_env
        .with_env(|env| -> JniResult<JByteArray<'caller>> {
            let id = id.try_to_string(env)?;
            let what = what.try_to_string(env)?;
            match crate::ffi::value_for_fill(&id, &what) {
                Some(value) => env.byte_array_from_slice(value.as_bytes()),
                None => Ok(JByteArray::default()),
            }
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// An item's value for the clipboard ("username", "password", "totp",
/// "notes", or "field:<index>"), as `value` gives it, but never a secret
/// the person's shared folder hides from them; null then too. It goes from
/// here to the clipboard without passing through Dart.
#[cfg(feature = "ffi")]
#[jni_mangle("com.passbundle.app.Core")]
pub fn copy_value<'caller>(
    mut unowned_env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    id: JString<'caller>,
    what: JString<'caller>,
) -> JByteArray<'caller> {
    unowned_env
        .with_env(|env| -> JniResult<JByteArray<'caller>> {
            let id = id.try_to_string(env)?;
            let what = what.try_to_string(env)?;
            match crate::ffi::value_for_copy(&id, &what) {
                Some(value) => env.byte_array_from_slice(value.as_bytes()),
                None => Ok(JByteArray::default()),
            }
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// The unlocked vault's user key, for Kotlin to seal with the Keystore key
/// that needs biometrics (docs/phones.md, "Unlocking with biometrics"), as
/// bytes it zeroes after use; null while locked or busy. It goes from here to
/// the Keystore without passing through Dart.
#[cfg(feature = "ffi")]
#[jni_mangle("com.passbundle.app.Core")]
pub fn user_key<'caller>(
    mut unowned_env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
) -> JByteArray<'caller> {
    unowned_env
        .with_env(|env| -> JniResult<JByteArray<'caller>> {
            match crate::ffi::user_key_for_biometric() {
                Some(key) => env.byte_array_from_slice(&key),
                None => Ok(JByteArray::default()),
            }
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// Opens the kept vault with the user key Kotlin opened after the person's
/// biometrics: "" if it opened, else the words to show (null only if JNI
/// itself failed, which Kotlin takes as not opened). A key that no longer
/// opens it means biometric unlock is out of date, and is turned off.
#[cfg(feature = "ffi")]
#[jni_mangle("com.passbundle.app.Core")]
pub fn unlock_with_key<'caller>(
    mut unowned_env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    key: JByteArray<'caller>,
) -> jstring {
    unowned_env
        .with_env(|env| -> JniResult<jstring> {
            let key = Zeroizing::new(env.convert_byte_array(&key)?);
            match crate::ffi::unlock_with_key_for_biometric(&key) {
                Ok(()) => Ok(env.new_string("")?.into_raw()),
                Err(why) => Ok(env.new_string(why)?.into_raw()),
            }
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// Opens the core on the app's own folder unless it's open: the autofill
/// service can start the app's process without its screens (or Dart).
#[cfg(feature = "ffi")]
#[jni_mangle("com.passbundle.app.Core")]
pub fn open_if_closed<'caller>(
    mut unowned_env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    dir: JString<'caller>,
) {
    unowned_env
        .with_env(|env| -> JniResult<()> {
            crate::ffi::open_if_closed(&dir.try_to_string(env)?);
            Ok(())
        })
        .resolve::<ThrowRuntimeExAndDefault>();
}

/// The logins for an app (its package name) or a page (its domain, when a
/// browser gives one), as JSON: names and usernames only, never a password
/// (docs/phones.md, "Autofill").
#[cfg(feature = "ffi")]
#[jni_mangle("com.passbundle.app.Core")]
pub fn fillable<'caller>(
    mut unowned_env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    package: JString<'caller>,
    domain: JString<'caller>,
) -> jstring {
    unowned_env
        .with_env(|env| -> JniResult<jstring> {
            let answer =
                crate::ffi::fillable(&package.try_to_string(env)?, &domain.try_to_string(env)?);
            Ok(env.new_string(answer.to_string())?.into_raw())
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// Writes one of an item's attachments into a file Android opened for it
/// (Downloads, through MediaStore; docs/core.md, "Attachments"): `{ok}`, or
/// the words to show, as JSON. Slow (the download): off the main thread.
#[cfg(feature = "ffi")]
#[jni_mangle("com.passbundle.app.Core")]
pub fn writeAttachmentTo<'caller>(
    mut unowned_env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    id: JString<'caller>,
    attachment_id: JString<'caller>,
    fd: jni::sys::jint,
) -> jstring {
    unowned_env
        .with_env(|env| -> JniResult<jstring> {
            let answer = crate::ffi::write_attachment_to(
                &id.try_to_string(env)?,
                &attachment_id.try_to_string(env)?,
                fd,
            );
            Ok(env.new_string(answer.to_string())?.into_raw())
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// The master password, for a login that asks for it again before it fills
/// (docs/core.md, "Asking for the master password again"): `{ok}`, or the
/// words to show, as JSON. Slow (the KDF): call it off the main thread.
#[cfg(feature = "ffi")]
#[jni_mangle("com.passbundle.app.Core")]
pub fn confirmMasterPassword<'caller>(
    mut unowned_env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    id: JString<'caller>,
    password: JString<'caller>,
) -> jstring {
    unowned_env
        .with_env(|env| -> JniResult<jstring> {
            let password = zeroize::Zeroizing::new(password.try_to_string(env)?);
            let answer = crate::ffi::confirm_master_password(&id.try_to_string(env)?, &password);
            Ok(env.new_string(answer.to_string())?.into_raw())
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// The logins with a passkey for a site (its ID), as JSON: names and IDs,
/// never a key (docs/phones.md, "Passkeys").
#[cfg(feature = "ffi")]
#[jni_mangle("com.passbundle.app.Core")]
pub fn passkeys<'caller>(
    mut unowned_env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    rp_id: JString<'caller>,
) -> jstring {
    unowned_env
        .with_env(|env| -> JniResult<jstring> {
            let answer = crate::ffi::passkeys(&rp_id.try_to_string(env)?);
            Ok(env.new_string(answer.to_string())?.into_raw())
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// Signs in to a site with a login's passkey (docs/core.md, "Passkeys"), as
/// JSON; the client data hash in base64url. Slow when the passkey counts its
/// uses (it's saved first): off the main thread.
#[cfg(feature = "ffi")]
#[jni_mangle("com.passbundle.app.Core")]
pub fn passkey_assert<'caller>(
    mut unowned_env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    id: JString<'caller>,
    rp_id: JString<'caller>,
    client_data_hash: JString<'caller>,
    user_verified: jni::sys::jboolean,
) -> jstring {
    unowned_env
        .with_env(|env| -> JniResult<jstring> {
            let answer = crate::ffi::passkey_assert(
                &id.try_to_string(env)?,
                &rp_id.try_to_string(env)?,
                &client_data_hash.try_to_string(env)?,
                user_verified,
            );
            Ok(env.new_string(answer.to_string())?.into_raw())
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// Makes a passkey for a site (docs/core.md, "Passkeys", "Making one"), from
/// the request as JSON, answering JSON. Slow (it's saved first): off the main
/// thread.
#[cfg(feature = "ffi")]
#[jni_mangle("com.passbundle.app.Core")]
pub fn passkey_create<'caller>(
    mut unowned_env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    request: JString<'caller>,
) -> jstring {
    unowned_env
        .with_env(|env| -> JniResult<jstring> {
            let answer = crate::ffi::passkey_create(&request.try_to_string(env)?);
            Ok(env.new_string(answer.to_string())?.into_raw())
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// The master password, as proof the person is who they are when a passkey
/// signs or is made: `{ok}`, or the words to show, as JSON. Slow (the KDF):
/// off the main thread.
#[cfg(feature = "ffi")]
#[jni_mangle("com.passbundle.app.Core")]
pub fn check_master_password<'caller>(
    mut unowned_env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    password: JString<'caller>,
) -> jstring {
    unowned_env
        .with_env(|env| -> JniResult<jstring> {
            let password = Zeroizing::new(password.try_to_string(env)?);
            let answer = crate::ffi::check_master_password(&password);
            Ok(env.new_string(answer.to_string())?.into_raw())
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// Whether a site's Digital Asset Links let an app (its package, and its
/// signing certificate's SHA-256) use the site's passkeys (asset_links.rs).
/// `statements` stands in for fetching them (debug builds' test site); null
/// fetches. False when they can't be read. Slow (the network): off the main
/// thread.
#[jni_mangle("com.passbundle.app.Core")]
pub fn app_may_use_passkeys<'caller>(
    mut unowned_env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    rp_id: JString<'caller>,
    package: JString<'caller>,
    certificate: JByteArray<'caller>,
    statements: JString<'caller>,
) -> jni::sys::jboolean {
    unowned_env
        .with_env(|env| -> JniResult<bool> {
            let certificate: [u8; 32] = match env.convert_byte_array(&certificate)?.try_into() {
                Ok(c) => c,
                Err(_) => return Ok(false),
            };
            let statements = if statements.is_null() {
                None
            } else {
                Some(statements.try_to_string(env)?)
            };
            Ok(crate::asset_links::check(
                &rp_id.try_to_string(env)?,
                &package.try_to_string(env)?,
                &certificate,
                statements.as_deref(),
            )
            .unwrap_or(false))
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// Whether a site's Digital Asset Links let an app (its package, and its
/// signing certificate's SHA-256) fill the site's logins through autofill
/// (asset_links.rs), for a site the app names itself. `statements` stands in
/// for fetching them (debug builds' test site). With `may_fetch` false only
/// what was fetched lately counts (no network: the fill's own check); true
/// may fetch (slow: off the main thread). False when they can't be read.
#[jni_mangle("com.passbundle.app.Core")]
pub fn app_linked_to_site<'caller>(
    mut unowned_env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    site: JString<'caller>,
    package: JString<'caller>,
    certificate: JByteArray<'caller>,
    statements: JString<'caller>,
    may_fetch: jni::sys::jboolean,
) -> jni::sys::jboolean {
    unowned_env
        .with_env(|env| -> JniResult<bool> {
            let certificate: [u8; 32] = match env.convert_byte_array(&certificate)?.try_into() {
                Ok(c) => c,
                Err(_) => return Ok(false),
            };
            let (site, package) = (site.try_to_string(env)?, package.try_to_string(env)?);
            if !statements.is_null() {
                let given = statements.try_to_string(env)?;
                return Ok(crate::asset_links::allows(&given, &package, &certificate));
            }
            Ok(if may_fetch {
                crate::asset_links::check(&site, &package, &certificate, None).unwrap_or(false)
            } else {
                crate::asset_links::kept(&site, &package, &certificate)
                    .ok()
                    .flatten()
                    .unwrap_or(false)
            })
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// Turns biometric unlock off: Kotlin deletes its Keystore key and the user
/// key it sealed (signing out, or a key that's out of date).
pub(crate) fn remove_biometric() {
    let Some((vm, loader)) = JVM.get() else {
        return;
    };
    let _ = vm.attach_current_thread_for_scope(|env| -> JniResult<()> {
        let result = (|| -> JniResult<()> {
            let name = env.new_string("com.passbundle.app.Biometric")?;
            let class = loader.load_class(env, name)?;
            env.call_static_method(&class, jni_str!("remove"), jni_sig!(() -> void), &[])?
                .v()
        })();
        if result.is_err() {
            env.exception_clear();
        }
        Ok(())
    });
}

/// Calls `Keystore.<method>(byte[]): byte[]` in the app.
fn keystore(method: &'static JNIStr, input: &[u8]) -> Option<Vec<u8>> {
    let (vm, loader) = JVM.get()?;
    vm.attach_current_thread_for_scope(|env| -> JniResult<Option<Vec<u8>>> {
        match call_keystore(env, loader, method, input) {
            Ok(output) => Ok(output),
            Err(_) => {
                // A Java exception (no Keystore, a bad file) leaves nothing kept or read.
                env.exception_clear();
                Ok(None)
            }
        }
    })
    .ok()
    .flatten()
}

fn call_keystore(
    env: &mut Env,
    loader: &Global<JClassLoader<'static>>,
    method: &'static JNIStr,
    input: &[u8],
) -> JniResult<Option<Vec<u8>>> {
    let name = env.new_string("com.passbundle.app.Keystore")?;
    let class = loader.load_class(env, name)?;
    let input = env.byte_array_from_slice(input)?;
    let output = env
        .call_static_method(
            &class,
            method,
            jni_sig!((input: jbyte[]) -> jbyte[]),
            &[JValue::from(&JObject::from(input))],
        )?
        .l()?;
    if output.is_null() {
        return Ok(None);
    }
    let output = env.cast_local::<JByteArray>(output)?;
    Ok(Some(env.convert_byte_array(&output)?))
}

/// Seals with the app's Keystore key; None before the app has started the core.
pub(crate) fn seal(plain: &[u8]) -> Option<Vec<u8>> {
    keystore(jni_str!("seal"), plain)
}

/// What `seal` sealed; None if it doesn't open.
pub(crate) fn open(sealed: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
    keystore(jni_str!("open"), sealed).map(Zeroizing::new)
}
