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

/// An item's value for the clipboard ("username", "password", "totp",
/// "notes", or "field:<index>"), as UTF-8 bytes Kotlin zeroes after use; null
/// while locked or busy, or if there's nothing to copy. It goes from here to
/// the clipboard without passing through Dart.
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
