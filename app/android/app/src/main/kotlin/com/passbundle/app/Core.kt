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
}
