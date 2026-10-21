//! The Android app's calls into the core (docs/phones.md, "Talking to the
//! OS"), from its Kotlin through JNI (`com.passbundle.app.Core`). Kotlin
//! calls in; the core never calls Kotlin, so no thread is attached to the JVM.

use jni::errors::ThrowRuntimeExAndDefault;
use jni::jni_mangle;
use jni::objects::{JByteArray, JClass, JObject, JString};
use jni::EnvUnowned;

/// At start, before anything reaches the network: the core checks
/// certificates with Android's own verifier, which needs the app's context.
#[jni_mangle("com.passbundle.app.Core")]
pub fn init<'caller>(
    mut unowned_env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    context: JObject<'caller>,
) {
    unowned_env
        .with_env(|env| rustls_platform_verifier::android::init_with_env(env, context))
        .resolve::<ThrowRuntimeExAndDefault>();
}

/// An item's value for the clipboard ("username", "password", "totp",
/// "notes", or "field:<index>"), as UTF-8 bytes Kotlin zeroes after use; null
/// while locked or busy, or if there's nothing to copy. It goes from here to
/// the clipboard without passing through Dart.
#[jni_mangle("com.passbundle.app.Core")]
pub fn value<'caller>(
    mut unowned_env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    id: JString<'caller>,
    what: JString<'caller>,
) -> JByteArray<'caller> {
    unowned_env
        .with_env(|env| -> jni::errors::Result<JByteArray<'caller>> {
            let id = id.try_to_string(env)?;
            let what = what.try_to_string(env)?;
            match crate::ffi::value_for_copy(&id, &what) {
                Some(value) => env.byte_array_from_slice(value.as_bytes()),
                None => Ok(JByteArray::default()),
            }
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}
