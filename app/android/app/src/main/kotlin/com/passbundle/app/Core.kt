package com.passbundle.app

import android.content.Context

/** The core (Rust), for the few calls Kotlin makes itself (core/src/android.rs). */
object Core {
    init {
        System.loadLibrary("passbundle_core")
    }

    /** Before anything reaches the network: Android's own certificate checks need the app's context. */
    @JvmStatic external fun init(context: Context)

    /**
     * An item's value for the clipboard, as UTF-8 to zero after use; null
     * while the vault is locked or busy, or if there's nothing to copy.
     */
    @JvmStatic external fun value(id: String, what: String): ByteArray?

    /** The unlocked vault's user key, to seal for biometric unlock; zero it after use. Null while locked. */
    @JvmStatic external fun userKey(): ByteArray?

    /** Opens the kept vault with the user key biometric unlock opened: "" if it opened, else the words to show. */
    @JvmStatic external fun unlockWithKey(key: ByteArray): String?

    /** Opens the core on the app's own folder unless it's open (autofill can start the app without its screens). */
    @JvmStatic external fun openIfClosed(dir: String)

    /** The logins for an app or a page, as JSON: names and usernames only (Responses.kt). */
    @JvmStatic external fun fillable(packageName: String, webDomain: String): String?
}
