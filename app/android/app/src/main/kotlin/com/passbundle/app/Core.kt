package com.passbundle.app

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ApplicationInfo
import android.os.Build
import android.util.Log

/** The core (Rust), for the few calls Kotlin makes itself (core/src/android.rs). */
object Core {
    init {
        System.loadLibrary("passbundle_core")
    }

    private var screenWatcher: BroadcastReceiver? = null

    /**
     * Before anything else of the core's, from each of the app's activities
     * and services that use it: [init], and the phone locking (its screen
     * going off) locking the vault if the person chose that, in the core
     * itself, for as long as this process runs (docs/phones.md, "Locking").
     * Autofill and passkeys can unlock the vault with biometrics while the
     * app's screens aren't running, or show it locked, so locking can't wait
     * for them (MainActivity tells the screens as well).
     */
    @JvmStatic
    @Synchronized
    fun start(context: Context) {
        init(context)
        if (screenWatcher != null) return
        val receiver =
            object : BroadcastReceiver() {
                override fun onReceive(context: Context, intent: Intent) {
                    val locked = screenOff()
                    // Debug builds only, for the emulator tests' logs.
                    if ((context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0) {
                        Log.d("PassBundle", "screen off: ${if (locked) "locks the vault" else "nothing to lock"}")
                    }
                }
            }
        val filter = IntentFilter(Intent.ACTION_SCREEN_OFF)
        val app = context.applicationContext
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            app.registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED)
        } else {
            app.registerReceiver(receiver, filter)
        }
        screenWatcher = receiver
    }

    /** Before anything reaches the network: Android's own certificate checks need the app's context. */
    @JvmStatic external fun init(context: Context)

    /** The screen went off: locks if the person chose that, or the policy says so ([start]); whether it does. */
    @JvmStatic external fun screenOff(): Boolean

    /**
     * An item's value for autofill to fill, as UTF-8 to zero after use; null
     * while the vault is locked or busy, or if there's nothing to fill.
     */
    @JvmStatic external fun value(id: String, what: String): ByteArray?

    /**
     * An item's value for the clipboard, as [value], but never a secret the
     * person's shared folder hides from them (null then too).
     */
    @JvmStatic external fun copyValue(id: String, what: String): ByteArray?

    /** The unlocked vault's user key, to seal for biometric unlock; zero it after use. Null while locked. */
    @JvmStatic external fun userKey(): ByteArray?

    /** Opens the kept vault with the user key biometric unlock opened: "" if it opened, else the words to show. */
    @JvmStatic external fun unlockWithKey(key: ByteArray): String?

    /** The same key, given back to confirm it's the person approving a sign-in, never to unlock: "" if it's the open vault's. */
    @JvmStatic external fun confirmApprovalWithKey(key: ByteArray): String?

    /** Opens the core on the app's own folder unless it's open (autofill can start the app without its screens). */
    @JvmStatic external fun openIfClosed(dir: String)

    /** Writes an attachment into a file Android opened for it: {"ok": true}, or the words to show. Slow: off the main thread. */
    @JvmStatic external fun writeAttachmentTo(id: String, attachmentId: String, fd: Int): String?

    /** The logins for an app or a page, as JSON: names and usernames only (Responses.kt). */
    @JvmStatic external fun fillable(packageName: String, webDomain: String): String?

    /** The master password, for a login that asks for it again: {"ok": true}, or the words to show. Slow: off the main thread. */
    @JvmStatic external fun confirmMasterPassword(id: String, password: String): String?

    /** The master password as proof the person is who they are, for a passkey: {"ok": true}, or the words. Slow. */
    @JvmStatic external fun checkMasterPassword(password: String): String?

    /** The logins with a passkey for a site (its ID), as JSON: names and IDs, never a key (PasskeyProvider.kt). */
    @JvmStatic external fun passkeys(rpId: String): String?

    /** Signs in with a login's passkey, the client data hash in base64url: JSON (ffi.rs). Slow: off the main thread. */
    @JvmStatic external fun passkeyAssert(id: String, rpId: String, clientDataHash: String, userVerified: Boolean): String?

    /** Makes a passkey from the request as JSON, answering JSON (ffi.rs). Slow, it's saved first: off the main thread. */
    @JvmStatic external fun passkeyCreate(request: String): String?

    /**
     * Whether a site's Digital Asset Links let an app (package and its signing
     * certificate's SHA-256) use its passkeys; `statements` stands in for
     * fetching them (debug builds' test site). Slow: off the main thread.
     */
    @JvmStatic external fun appMayUsePasskeys(rpId: String, packageName: String, certificate: ByteArray, statements: String?): Boolean

    /**
     * Whether a site's Digital Asset Links let an app (package and its signing
     * certificate's SHA-256) fill the site's logins, for a site the app names
     * (AppSites.kt); `statements` stands in for fetching them (debug builds'
     * test site). With `mayFetch` false, only what was checked lately counts;
     * true may fetch: off the main thread.
     */
    @JvmStatic external fun appLinkedToSite(site: String, packageName: String, certificate: ByteArray, statements: String?, mayFetch: Boolean): Boolean
}
