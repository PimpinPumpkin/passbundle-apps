package com.passbundle.app

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.security.GeneralSecurityException
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * The refresh token's key (docs/phones.md, "Kept where"): AES-256-GCM in the
 * Android Keystore, for this app only, never leaving it. The core calls these
 * (core/src/android.rs) and keeps what `seal` returns in the app's own files.
 *
 * And a second key of the same kind, for PINs alone (`sealPin`, `openPin`):
 * what an account's PIN opens is kept only sealed with it, so a copy of the
 * app's files has nothing to guess a PIN against (DESIGN.md in the meta
 * repo, "Unlock with a PIN"). Neither key asks for biometrics.
 */
object Keystore {
    private const val ALIAS = "passbundle-refresh-token"
    private const val PIN_ALIAS = "passbundle-pin"
    private const val TRANSFORMATION = "AES/GCM/NoPadding"
    private const val IV_BYTES = 12
    private const val TAG_BITS = 128

    @Synchronized
    private fun key(alias: String = ALIAS): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(alias, null) as? SecretKey)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        generator.init(
            KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return generator.generateKey()
    }

    /** The IV, then the ciphertext and its tag. */
    @JvmStatic
    fun seal(plain: ByteArray): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, key())
        return cipher.iv + cipher.doFinal(plain)
    }

    /** What `seal` sealed; null if it doesn't open (the key was removed, or the file changed). */
    @JvmStatic
    fun open(sealed: ByteArray): ByteArray? =
        try {
            if (sealed.size <= IV_BYTES) return null
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(TAG_BITS, sealed, 0, IV_BYTES))
            cipher.doFinal(sealed, IV_BYTES, sealed.size - IV_BYTES)
        } catch (e: GeneralSecurityException) {
            null
        }

    /**
     * Seals an account's PIN record with the PIN key: the IV, then the
     * ciphertext and its tag. `context` (a label and the account's id on this
     * device, from the core) is authenticated with it, so one account's
     * doesn't open as another's.
     */
    @JvmStatic
    fun sealPin(context: ByteArray, plain: ByteArray): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, key(PIN_ALIAS))
        cipher.updateAAD(context)
        return cipher.iv + cipher.doFinal(plain)
    }

    /**
     * What `sealPin` sealed with the same context; null if it doesn't open
     * (the key was removed or isn't this phone's, another account's context,
     * or the file changed).
     */
    @JvmStatic
    fun openPin(context: ByteArray, sealed: ByteArray): ByteArray? =
        try {
            if (sealed.size <= IV_BYTES) return null
            val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
            // Opening never makes the key: without it there's nothing to open.
            val key = store.getKey(PIN_ALIAS, null) as? SecretKey ?: return null
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(TAG_BITS, sealed, 0, IV_BYTES))
            cipher.updateAAD(context)
            cipher.doFinal(sealed, IV_BYTES, sealed.size - IV_BYTES)
        } catch (e: GeneralSecurityException) {
            null
        }
}
