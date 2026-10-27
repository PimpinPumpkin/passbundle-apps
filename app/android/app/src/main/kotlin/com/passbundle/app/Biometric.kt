package com.passbundle.app

import android.app.Activity
import android.content.Context
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricPrompt
import android.os.Build
import android.os.CancellationSignal
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import java.io.File
import java.security.GeneralSecurityException
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Unlocking with the phone's biometrics (docs/phones.md, "Unlocking with
 * biometrics"). The user key is sealed with an AES-256-GCM key in the
 * Keystore that needs the person's strong biometrics for every use, and that
 * Android voids when someone adds a fingerprint or face. The user key goes
 * between the core and the Keystore here, never through Dart.
 *
 * For tests, debug builds have a stand-in for Android's prompt that says yes,
 * with a key that needs no biometrics, as the desktop agent's test
 * authenticator does; release builds refuse it. Real prompts are checked by
 * hand on real phones.
 */
object Biometric {
    private const val ALIAS = "passbundle-biometric"
    private const val TRANSFORMATION = "AES/GCM/NoPadding"
    private const val IV_BYTES = 12
    private const val TAG_BITS = 128

    private lateinit var files: File
    private var standIn = false

    /** At start: where the sealed user key is kept (the app's own files). */
    fun init(context: Context) {
        files = context.filesDir
        standIn = debuggable(context) && standInMark.exists()
    }

    private val sealed get() = File(files, "biometric.sealed")
    private val standInMark get() = File(files, "biometric-stand-in")

    private fun debuggable(context: Context) = (context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0

    /** Tests only: the stand-in for Android's prompt, kept across the app's processes. False in a release build. */
    fun useStandIn(context: Context, on: Boolean): Boolean {
        if (!debuggable(context)) return false
        remove()
        standIn = on
        if (on) standInMark.writeText("") else standInMark.delete()
        return true
    }

    /** "fingerprint", "face unlock", or "biometrics": what the screens call it. */
    fun method(context: Context): String {
        val packages = context.packageManager
        return when {
            packages.hasSystemFeature(PackageManager.FEATURE_FINGERPRINT) -> "fingerprint"
            packages.hasSystemFeature(PackageManager.FEATURE_FACE) -> "face unlock"
            else -> "biometrics"
        }
    }

    /** "setUp", "notSetUp", or "unavailable" (no strong biometrics enrolled on this phone). */
    fun status(context: Context): String {
        if (standIn) return if (sealed.exists()) "setUp" else "notSetUp"
        val manager = context.getSystemService(BiometricManager::class.java)
        val can =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                manager.canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG)
            } else {
                @Suppress("DEPRECATION")
                manager.canAuthenticate()
            }
        return when {
            can != BiometricManager.BIOMETRIC_SUCCESS -> "unavailable"
            sealed.exists() -> "setUp"
            else -> "notSetUp"
        }
    }

    private fun keystore() = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    private fun newKey(context: Context): SecretKey {
        val spec = { strongBox: Boolean ->
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .apply {
                    if (standIn) return@apply
                    setUserAuthenticationRequired(true)
                    setInvalidatedByBiometricEnrollment(true)
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                        setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG)
                    }
                    if (strongBox) setIsStrongBoxBacked(true)
                }
                .build()
        }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        // In StrongBox, the phone's separate security chip, where it has one.
        if (context.packageManager.hasSystemFeature(PackageManager.FEATURE_STRONGBOX_KEYSTORE)) {
            try {
                generator.init(spec(true))
                return generator.generateKey()
            } catch (e: StrongBoxUnavailableException) {
                // Fall back to the Keystore's own hardware.
            } catch (e: GeneralSecurityException) {
                // Fall back to the Keystore's own hardware.
            }
        }
        generator.init(spec(false))
        return generator.generateKey()
    }

    /** Turns it off: the key and the sealed user key go. The core calls this when signing out. */
    @JvmStatic
    fun remove() {
        if (::files.isInitialized) sealed.delete()
        try {
            keystore().deleteEntry(ALIAS)
        } catch (e: GeneralSecurityException) {
            // Nothing to delete.
        }
    }

    private fun prompt(
        activity: Activity,
        title: String,
        cipher: Cipher,
        done: (Cipher?, String?) -> Unit,
    ) {
        if (standIn) return done(cipher, null)
        val executor = activity.mainExecutor
        val builder =
            BiometricPrompt.Builder(activity)
                .setTitle(title)
                .setNegativeButton("Use master password", executor) { _, _ -> done(null, null) }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            builder.setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG)
        }
        builder.build().authenticate(
            BiometricPrompt.CryptoObject(cipher),
            CancellationSignal(),
            executor,
            object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    done(result.cryptoObject?.cipher, null)
                }

                override fun onAuthenticationError(code: Int, message: CharSequence) {
                    val cancelled =
                        code == BiometricPrompt.BIOMETRIC_ERROR_USER_CANCELED ||
                            code == BiometricPrompt.BIOMETRIC_ERROR_CANCELED
                    done(null, if (cancelled) null else message.toString())
                }
            },
        )
    }

    /**
     * Turns it on, while unlocked: asks for the biometric, then seals the user
     * key. `done(on, error)`: on is false if the person cancelled.
     */
    fun enroll(activity: Activity, done: (Boolean, String?) -> Unit) {
        val cipher =
            try {
                remove()
                Cipher.getInstance(TRANSFORMATION).apply { init(Cipher.ENCRYPT_MODE, newKey(activity)) }
            } catch (e: GeneralSecurityException) {
                return done(false, "Couldn't set up unlocking with ${method(activity)}.")
            }
        prompt(activity, "Turn on unlocking PassBundle with ${method(activity)}", cipher) { authed, error ->
            if (authed == null) {
                remove()
                return@prompt done(false, error)
            }
            val key = Core.userKey() ?: return@prompt done(false, "Unlock PassBundle first.")
            try {
                sealed.writeBytes(authed.iv + authed.doFinal(key))
                done(true, null)
            } catch (e: GeneralSecurityException) {
                remove()
                done(false, "Couldn't set up unlocking with ${method(activity)}.")
            } finally {
                key.fill(0)
            }
        }
    }

    /**
     * Unlocks: asks for the biometric, opens the user key, and hands it to the
     * core. `done(unlocked, error)`: unlocked is false if the person cancelled.
     * While unlocked, the core takes the key as the person confirming it's
     * them (approving a sign-in from another device), and the prompt says
     * [title].
     */
    fun unlock(activity: Activity, title: String = "Unlock PassBundle", done: (Boolean, String?) -> Unit) {
        val bytes = runCatching { sealed.readBytes() }.getOrNull()
        val key = runCatching { keystore().getKey(ALIAS, null) as? SecretKey }.getOrNull()
        if (bytes == null || bytes.size <= IV_BYTES || key == null) {
            remove()
            return done(false, "Unlocking with ${method(activity)} is off. Use your master password.")
        }
        val cipher =
            try {
                Cipher.getInstance(TRANSFORMATION).apply {
                    init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(TAG_BITS, bytes, 0, IV_BYTES))
                }
            } catch (e: KeyPermanentlyInvalidatedException) {
                // Someone added a fingerprint or face since: Android voided the key.
                remove()
                return done(
                    false,
                    "A new ${method(activity)} was added to this phone, so unlocking with it is off. " +
                        "Use your master password, then turn it on again.",
                )
            } catch (e: GeneralSecurityException) {
                return done(false, "Couldn't unlock with ${method(activity)}.")
            }
        prompt(activity, title, cipher) { authed, error ->
            if (authed == null) return@prompt done(false, error)
            val userKey =
                try {
                    authed.doFinal(bytes, IV_BYTES, bytes.size - IV_BYTES)
                } catch (e: GeneralSecurityException) {
                    return@prompt done(false, "Couldn't unlock with ${method(activity)}.")
                }
            val why =
                try {
                    Core.unlockWithKey(userKey)
                } finally {
                    userKey.fill(0)
                }
            when (why) {
                "" -> done(true, null)
                null -> done(false, "Couldn't unlock with ${method(activity)}.")
                else -> done(false, why)
            }
        }
    }
}
