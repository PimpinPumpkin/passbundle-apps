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
