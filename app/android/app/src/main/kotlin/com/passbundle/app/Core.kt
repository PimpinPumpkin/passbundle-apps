package com.passbundle.app

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ApplicationInfo
import android.os.Build
import android.util.Log

/**
 * The core (Rust), for the few calls Kotlin makes itself (core/src/android.rs).
 * A call about an account names it by its id on this device (`account`),
 * as the core answered it: the account a login, a passkey, or a biometric
 * key is from (DESIGN.md in the meta repo, "Several accounts in the apps").
 */
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
     * Autofill or a passkey handed its answer back (a fill, a passkey, or
     * none): with "Immediately" chosen, or the policy's, the vault locks now,
     * as when the app's screens leave (docs/phones.md, "Locking"). Never waits.
     */
    @JvmStatic external fun answered(): Boolean

    /**
     * Autofill or a passkey unlocked the vault and handed back a list to pick
     * from: with "Immediately", it stays open for the pick only, and locks a
     * minute later if nothing is picked. Never waits.
     */
    @JvmStatic external fun offered()

    /**
     * An item's value for autofill to fill, from its account, as UTF-8 to
     * zero after use: "username", "password", "totp" (the current one-time
     * code), or a card's or an identity's part, by the core's names for
     * them (Values.kt); null while that account is locked or busy, or if
     * there's nothing to fill.
     */
    @JvmStatic external fun value(account: String, id: String, what: String): ByteArray?

    /**
     * An item's value for the clipboard, as [value], but never a secret the
     * person's shared folder hides from them (null then too).
     */
    @JvmStatic external fun copyValue(account: String, id: String, what: String): ByteArray?

    /** An unlocked account's user key, to seal for its biometric unlock; zero it after use. Null while locked. */
    @JvmStatic external fun userKey(account: String): ByteArray?

    /** Opens an account's kept vault with the user key its biometric unlock opened: "" if it opened, else the words to show. */
    @JvmStatic external fun unlockWithKey(account: String, key: ByteArray): String?

    /** The same key, given back to confirm it's the person approving a sign-in to that account, never to unlock: "" if it's its open vault's. */
    @JvmStatic external fun confirmApprovalWithKey(account: String, key: ByteArray): String?

    /**
     * Push to phones (Push.kt): the account's server's push for this device,
     * as JSON (`{ok, enabled, vapidPublicKey, registered}`), asked with the
     * account's kept session whether or not it's unlocked. Slow: off the main
     * thread.
     */
    @JvmStatic external fun pushSettings(account: String): String?

    /** Keeps this device's push endpoint on the account's server, with the keys the distributor's library made (base64url): JSON, as [pushSettings]. Slow. */
    @JvmStatic external fun pushRegister(account: String, endpoint: String, publicKey: String, auth: String): String?

    /** Removes this device's push endpoint from the account's server: `{ok}` as JSON. Slow. */
    @JvmStatic external fun pushUnregister(account: String): String?

    /**
     * How many sign-ins wait for the account's answer, for a push message
     * that says one does: `{ok, requests, checks}` as JSON, numbers only,
     * never which; `signedOut` when its session is over. Locked or not. Slow.
     */
    @JvmStatic external fun waitingSignIns(account: String): String?

    /** Opens the core on the app's own folder unless it's open (autofill can start the app without its screens). */
    @JvmStatic external fun openIfClosed(dir: String)

    /** Writes an attachment into a file Android opened for it: {"ok": true}, or the words to show. Slow: off the main thread. */
    @JvmStatic external fun writeAttachmentTo(account: String, id: String, attachmentId: String, fd: Int): String?

    /**
     * The logins for an app or a page, as JSON, from every unlocked
     * account: names and usernames only, each with its `account` and
     * whether it has a one-time code to fill (`totp`: false for a key whose
     * code needs Premium, core/src/codes.rs); or `locked`, with the
     * `account` to unlock and its `accountLabel` when there's more than
     * one; or `signedOut` (Responses.kt).
     */
    @JvmStatic external fun fillable(packageName: String, webDomain: String): String?

    /**
     * The cards and identities autofill offers on any app or page, as JSON
     * (`cards`, `identities`), from every unlocked account: names and plain
     * subtitles only, never a number or a code, each with its `account`; or
     * `locked` and `signedOut`, as [fillable] says (Responses.kt).
     */
    @JvmStatic external fun cardsAndIdentities(): String?

    /**
     * Which of an item's values its fill rules put into which of a web
     * page's fields, and which fields they and its account's "Never fill
     * this field on this site" list keep empty (FillRules.kt), as JSON:
     * `fills` (each field by its place in the page's fields, the value by
     * its name for [value], and how the field takes it) and `never`; never a
     * value. The page is JSON too (FillRules.json). `locked` while that
     * account is locked or busy, or for an item it hasn't in use.
     */
    @JvmStatic external fun fillRules(account: String, id: String, page: String): String?

    /**
     * What autofill needs before Android offers to save a card or an
     * address someone typed (Saving.kt), from every unlocked account, as
     * JSON: `card` and `identity`, each with whether one may be made in any
     * of them (`mayAdd`), and each card's `last4` and each identity's
     * `addresses` (its street's words and its postal code, as the core
     * compares them), never a whole number; or `locked`, `signedOut`, and
     * `busy`, as [fillable] says.
     */
    @JvmStatic external fun saveHints(): String?

    /**
     * Whether a card (by its number) or an address (its first street line
     * and postal code) someone asked to save is one an unlocked account has,
     * as JSON: `saved`, with its `account`, `id`, and `name`, or null; and
     * the unlocked accounts where one may be made (`mayAdd`, the shown one
     * first); or `locked`, `signedOut`, and `busy`, as [fillable] says.
     * `kind` is "card" or "identity"; what doesn't apply to it is "".
     */
    @JvmStatic external fun alreadySaved(kind: String, number: String, street: String, postalCode: String): String?

    /**
     * Unlocks an account with its master password, for autofill while it's
     * locked and there's no biometric unlock (FillActivity.kt): {"ok": true},
     * or the words to show. Slow (the KDF): off the main thread.
     */
    @JvmStatic external fun unlockWithMasterPassword(account: String, password: String): String?

    /** The master password, for a login that asks for it again: {"ok": true}, or the words to show. Slow: off the main thread. */
    @JvmStatic external fun confirmMasterPassword(account: String, id: String, password: String): String?

    /** An account's master password as proof the person is who they are, for a passkey: {"ok": true}, or the words. Slow. */
    @JvmStatic external fun checkMasterPassword(account: String, password: String): String?

    /**
     * The logins with a passkey for a site (its ID), as JSON, from the
     * account the screens show: names and IDs, each with its `account`, never
     * a key; or `locked`, with the `account` to unlock (PasskeyProvider.kt).
     */
    @JvmStatic external fun passkeys(rpId: String): String?

    /** Signs in with a login's passkey, from its account, the client data hash in base64url: JSON (ffi.rs). Slow: off the main thread. */
    @JvmStatic external fun passkeyAssert(account: String, id: String, rpId: String, clientDataHash: String, userVerified: Boolean): String?

    /**
     * Where a new passkey can go, from the request as JSON (the site's ID, the
     * user handle, the credentials the site has, and the app asking): every
     * unlocked account's logins it can go in, the one whose passkey it would
     * replace first, whether the site has one of PassBundle's already
     * (`excluded`), and each unlocked account, saying whether a new login
     * can go in it (`newLogin`) and if not, why (`noNewLogin`); or `locked`,
     * with the `account` to unlock (ffi.rs, `pb_passkey_logins`). Never a
     * key or a password.
     */
    @JvmStatic external fun passkeyLogins(request: String): String?

    /** Makes a passkey from the request as JSON (with the `account` it goes in, and the `login`, or none for a new one), answering JSON (ffi.rs). Slow, it's saved first: off the main thread. */
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
