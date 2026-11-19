package com.passbundle.app

import android.app.Activity
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.util.Log
import org.json.JSONObject
import org.unifiedpush.android.connector.FailedReason
import org.unifiedpush.android.connector.PushService
import org.unifiedpush.android.connector.UnifiedPush
import org.unifiedpush.android.connector.data.PushEndpoint
import org.unifiedpush.android.connector.data.PushMessage

/**
 * The plain part of push to phones ([Push]): what a message says, what the
 * phone does about the core's answer, and the words. Nothing of Android's, so
 * the JVM tests run it (PushTest).
 */
object PushPlan {
    /** The one kind the server sends: a sign-in waits for the account's answer. */
    const val SIGN_IN = "sign-in"

    /** The notification's words: the prompt's own title, and where to answer. */
    const val TITLE = "Approve sign-in?"
    const val TEXT = "Open PassBundle to review it."

    private val KIND = Regex("""^\s*\{\s*"kind"\s*:\s*"([a-z-]{1,32})"\s*}\s*$""")

    /**
     * What a message says: its kind, from exactly `{"kind":"..."}` (what the
     * server sends, DESIGN.md "Push to phones"), or null for anything else.
     */
    fun kind(content: ByteArray): String? = KIND.find(String(content, Charsets.UTF_8))?.groupValues?.get(1)

    /** What the phone does once the core has asked the account's server. */
    enum class Then {
        /** Something waits: the notification shows (again, if it showed). */
        NOTIFY,

        /** Nothing waits (answered elsewhere, or gone): one showing goes. */
        CLEAR,

        /** The account's session is over: nothing shows, and its registration goes until it signs in again. */
        FORGET,

        /** The server couldn't be reached, or said something else: as it was. */
        NOTHING,
    }

    /** From the core's answer (`waitingSignIns`): whether it worked, whether the session is over, and the counts. */
    fun then(ok: Boolean, signedOut: Boolean, requests: Int, checks: Int): Then =
        when {
            signedOut -> Then.FORGET
            !ok -> Then.NOTHING
            requests + checks > 0 -> Then.NOTIFY
            else -> Then.CLEAR
        }

    /**
     * Whether an account needs registering with the distributor: not yet, or
     * with another of the server's keys than the one it registered with (the
     * server's key was replaced), or the server lost its endpoint (a push
     * service said it was gone). Never while the server doesn't push.
     */
    fun needsRegistering(enabled: Boolean, serverKey: String?, registeredWith: String?, serverHasIt: Boolean): Boolean =
        enabled && !serverKey.isNullOrEmpty() && (registeredWith != serverKey || !serverHasIt)
}

/**
 * Push to phones over UnifiedPush (DESIGN.md in the meta repo, "Push to
 * phones"; docs/phones.md, "Approving a sign-in"). When a distributor app is
 * installed (ntfy, for example), each account on the phone registers with it
 * as one of its instances, named by the account's id on this device, with
 * that account's server's VAPID key; the endpoint and keys the distributor's
 * library makes go to the account's server ([PassBundlePush.onNewEndpoint]).
 * A message says only that a sign-in waits; the core then asks the account's
 * server, as the app does while open, with the account's kept session, locked
 * or not, and the phone shows one notification in the prompt's words. No
 * distributor, nothing changes: the app asks while it's open, as before.
 *
 * What's kept, in the app's own preferences (no backups): each account's
 * server key it registered with. Nothing secret: the library keeps the keys.
 */
object Push {
    const val CHANNEL = "sign-in-requests"

    /** What a tapped notification opens with: the vault, which asks at once after unlocking. */
    const val OPEN = "com.passbundle.app.SIGN_IN_WAITING"

    private const val NOTIFICATION_TAG = "sign-in"
    private const val NOTIFICATION_ID = 1
    private const val PREFS = "push"
    private const val SERVER_PREFS = "push-server"
    private const val LOG = "PassBundle"

    /** Whether PassBundle's screen is in front (MainActivity): the vault's own prompt shows there instead. */
    @Volatile var inFront = false

    /** Each account registered with the distributor, by its id, with the server's key it registered with. */
    private fun prefs(context: Context) = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    /** Each account whose server took its endpoint, by its id. */
    private fun serverPrefs(context: Context) = context.applicationContext.getSharedPreferences(SERVER_PREFS, Context.MODE_PRIVATE)

    /** Drops what's kept about an account's registration. */
    private fun drop(context: Context, account: String) {
        prefs(context).edit().remove(account).apply()
        serverPrefs(context).edit().remove(account).apply()
    }

    private fun debuggable(context: Context) = (context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0

    private fun log(context: Context, text: String) {
        // Debug builds only, for the emulator tests' logs; never an endpoint or a key.
        if (debuggable(context)) Log.d(LOG, "push: $text")
    }

    /** The core, open on the app's own folder, from anywhere (a message comes with PassBundle closed). */
    private fun core(context: Context) {
        Core.start(context.applicationContext)
        Core.openIfClosed(context.applicationContext.filesDir.absolutePath)
    }

    private fun json(text: String?): JSONObject = runCatching { JSONObject(text ?: "{}") }.getOrDefault(JSONObject())

    /** Whether any distributor app is installed. */
    fun installed(context: Context): Boolean = UnifiedPush.getDistributors(context).isNotEmpty()

    /** The distributor in use, by the name the phone shows for it, or null for none yet. */
    fun distributorName(context: Context): String? {
        val distributor = UnifiedPush.getAckDistributor(context) ?: UnifiedPush.getSavedDistributor(context) ?: return null
        val packages = context.packageManager
        return try {
            packages.getApplicationLabel(packages.getApplicationInfo(distributor, 0)).toString()
        } catch (_: PackageManager.NameNotFoundException) {
            null
        }
    }

    /**
     * After signing in or unlocking (main.dart): each account in [unlocked]
     * registers with the distributor where it needs to ([PushPlan.needsRegistering]),
     * and each one registered that's no longer in [signedIn] is dropped. Asks
     * the servers off the main thread; picks the distributor on it (Android
     * may ask the person which, once, when several are installed).
     */
    fun sync(activity: Activity, signedIn: List<String>, unlocked: List<String>) {
        val app = activity.applicationContext
        Thread {
            val kept = prefs(app)
            for (account in kept.all.keys.filter { it !in signedIn }) {
                UnifiedPush.unregister(app, account)
                drop(app, account)
            }
            if (!installed(app)) return@Thread
            core(app)
            for (account in unlocked.filter { it.isNotEmpty() }) {
                val settings = json(Core.pushSettings(account))
                if (!settings.optBoolean("ok")) continue
                val enabled = settings.optBoolean("enabled")
                val key = settings.optString("vapidPublicKey").takeIf { it.isNotEmpty() && it != "null" }
                if (!enabled && kept.contains(account)) {
                    UnifiedPush.unregister(app, account)
                    drop(app, account)
                    continue
                }
                if (!PushPlan.needsRegistering(enabled, key, kept.getString(account, null), settings.optBoolean("registered"))) continue
                activity.runOnUiThread {
                    UnifiedPush.tryUseCurrentOrDefaultDistributor(activity) { ok ->
                        if (ok) {
                            UnifiedPush.register(app, account, null, key)
                            kept.edit().putString(account, key).apply()
                            log(app, "registered an account with the distributor")
                        }
                    }
                }
            }
        }.start()
    }

    /**
     * Signing out of an account (main.dart): its endpoint goes from its
     * server, then from the distributor ([fromDistributor] false when the
     * distributor dropped it itself). Slow: off the main thread.
     */
    fun forget(context: Context, account: String, fromDistributor: Boolean = true) {
        val app = context.applicationContext
        if (prefs(app).contains(account)) {
            core(app)
            Core.pushUnregister(account)
        }
        if (fromDistributor) UnifiedPush.unregister(app, account)
        drop(app, account)
    }

    /**
     * What Settings says: whether a distributor is installed, its name (null
     * for none chosen yet), and whether this account is registered all the
     * way: with the distributor, and its server took the endpoint.
     */
    fun status(context: Context, account: String): Map<String, Any?> =
        mapOf(
            "installed" to installed(context),
            "distributor" to distributorName(context),
            "registered" to (prefs(context).contains(account) && serverPrefs(context).getBoolean(account, false)),
        )

    /** The distributor's endpoint for an account: to its server, with the keys the library made. */
    fun endpoint(context: Context, account: String, url: String, publicKey: String, auth: String) {
        core(context)
        val answer = json(Core.pushRegister(account, url, publicKey, auth))
        val taken = answer.optBoolean("ok") && answer.optBoolean("registered")
        serverPrefs(context).edit().putBoolean(account, taken).apply()
        log(context, if (taken) "the server has the endpoint" else "the server didn't take the endpoint")
        if (answer.optBoolean("signedOut")) forget(context, account)
    }

    /**
     * A message for an account: if it opened and says a sign-in waits, the
     * core asks the account's server what's waiting, and the notification
     * shows, or goes. What happened, for the emulator test: whether it asked,
     * the counts, and whether the notification shows.
     */
    fun received(
        context: Context,
        account: String,
        content: ByteArray,
        decrypted: Boolean,
        appInFront: Boolean = inFront,
    ): Map<String, Any> {
        val app = context.applicationContext
        if (!decrypted || PushPlan.kind(content) != PushPlan.SIGN_IN || account.isEmpty()) {
            return mapOf("asked" to false, "notify" to false, "shown" to showing(app))
        }
        core(app)
        val answer = json(Core.waitingSignIns(account))
        val requests = answer.optInt("requests")
        val checks = answer.optInt("checks")
        val then = PushPlan.then(answer.optBoolean("ok"), answer.optBoolean("signedOut"), requests, checks)
        when (then) {
            // In front, the vault's own prompt shows it.
            PushPlan.Then.NOTIFY -> if (!appInFront) show(app)
            PushPlan.Then.CLEAR -> clear(app)
            PushPlan.Then.FORGET -> {
                clear(app)
                UnifiedPush.unregister(app, account)
                drop(app, account)
            }
            PushPlan.Then.NOTHING -> {}
        }
        log(app, "a message: $requests requests and $checks new devices waiting")
        val notify = then == PushPlan.Then.NOTIFY
        return mapOf(
            "asked" to true,
            "requests" to requests,
            "checks" to checks,
            "notify" to notify,
            "shown" to if (notify && !appInFront) shown(app) else showing(app),
        )
    }

    /** Whether the notification shows, once Android has had a moment to post it (a second at most). */
    private fun shown(context: Context): Boolean {
        repeat(20) {
            if (showing(context)) return true
            Thread.sleep(50)
        }
        return showing(context)
    }

    private fun channel(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java) ?: return
        if (manager.getNotificationChannel(CHANNEL) != null) return
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, "Sign-in requests", NotificationManager.IMPORTANCE_HIGH).apply {
                description = "A sign-in is waiting for you to approve it. Never which account, device, or place."
            },
        )
    }

    private fun show(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java) ?: return
        channel(context)
        val tap =
            PendingIntent.getActivity(
                context,
                0,
                Intent(context, MainActivity::class.java)
                    .setAction(OPEN)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
        val notification =
            Notification.Builder(context, CHANNEL)
                .setSmallIcon(R.drawable.reminder_mark)
                .setContentTitle(PushPlan.TITLE)
                .setContentText(PushPlan.TEXT)
                .setContentIntent(tap)
                .setAutoCancel(true)
                .setCategory(Notification.CATEGORY_REMINDER)
                .setVisibility(Notification.VISIBILITY_PUBLIC)
                // A sign-in request lasts 15 minutes at most.
                .setTimeoutAfter(15 * 60 * 1000L)
                .build()
        try {
            manager.notify(NOTIFICATION_TAG, NOTIFICATION_ID, notification)
        } catch (_: SecurityException) {
            // Notifications are off for PassBundle: the app shows the prompt when it opens.
        }
    }

    /** The distributor couldn't register an account: asked again after the next unlock. */
    fun registrationFailed(context: Context, account: String) = drop(context, account)

    /** Takes the notification away: nothing waits now, or PassBundle is open. */
    fun clear(context: Context) {
        context.getSystemService(NotificationManager::class.java)?.cancel(NOTIFICATION_TAG, NOTIFICATION_ID)
    }

    private fun showing(context: Context): Boolean =
        context.getSystemService(NotificationManager::class.java)
            ?.activeNotifications
            ?.any { it.tag == NOTIFICATION_TAG && it.id == NOTIFICATION_ID } == true

    /**
     * Tests only: debug builds hand a message to [received] as the
     * distributor would, as if PassBundle weren't in front, which the test
     * itself is (null in a release build).
     */
    fun standInMessage(context: Context, account: String, content: String, decrypted: Boolean): Map<String, Any>? =
        if (debuggable(context)) {
            received(context, account, content.toByteArray(Charsets.UTF_8), decrypted, appInFront = false)
        } else {
            null
        }
}

/**
 * What the distributor's library hands the app (UnifiedPush's connector):
 * a new endpoint for an account, a message (opened by the library with the
 * keys it keeps), and the registration ending. Each instance is an account's
 * id on this device. The network is the core's, off the main thread.
 */
class PassBundlePush : PushService() {
    override fun onNewEndpoint(endpoint: PushEndpoint, instance: String) {
        // No keys, no way to encrypt to the phone: the server would only
        // ever send what it can't, so it isn't told.
        val keys = endpoint.pubKeySet ?: return
        val app = applicationContext
        Thread { Push.endpoint(app, instance, endpoint.url, keys.pubKey, keys.auth) }.start()
    }

    override fun onMessage(message: PushMessage, instance: String) {
        val app = applicationContext
        val content = message.content.copyOf()
        val decrypted = message.decrypted
        Thread { Push.received(app, instance, content, decrypted) }.start()
    }

    override fun onRegistrationFailed(reason: FailedReason, instance: String) {
        // Asked again after the next unlock (Push.sync), as the account isn't kept as registered.
        Push.registrationFailed(applicationContext, instance)
    }

    override fun onUnregistered(instance: String) {
        val app = applicationContext
        Thread { Push.forget(app, instance, fromDistributor = false) }.start()
    }
}
