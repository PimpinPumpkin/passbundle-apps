package com.passbundle.app

import android.app.Activity
import android.app.ActivityOptions
import android.app.PendingIntent
import android.content.ActivityNotFoundException
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.net.Uri
import android.os.Build
import android.provider.Settings
import android.view.autofill.AutofillManager
import androidx.annotation.RequiresApi

/**
 * Turning PassBundle on as Android's autofill service and, on Android 14 and
 * later, as a passkey provider, from the app's own screens (docs/phones.md,
 * "Turning it on"; autofill_offer.dart). Nothing here changes a setting:
 * Android asks the person. Autofill opens Android's own request to use
 * PassBundle (ACTION_REQUEST_SET_AUTOFILL_SERVICE, started for a result, which
 * is how Settings knows the app asking is the one named): its list of
 * services, where choosing PassBundle asks "Use PassBundle?". On Android 15
 * and later that list is the one service for passwords, passkeys, and
 * autofill, so the same choice turns on passkeys. Passkeys alone open the
 * system's list of providers (Credential Manager's settings), where the
 * person switches PassBundle on. The app asks again whether each is on when
 * it's back in front, so the answer itself isn't read.
 */
object AutofillSetup {
    /** Where "Turn on" for autofill goes, tried in order until one opens. */
    enum class Way(val id: String) {
        /** Android's own request to use PassBundle for autofill. */
        PROMPT("prompt"),

        /** Android 14 and later: the page of password, passkey, and autofill services. */
        PROVIDERS("providers"),

        /** Settings itself, where the prompt is missing (some phones' own Settings). */
        SETTINGS("settings"),
    }

    /** The ways to try, best first, on Android [sdk]. */
    fun ways(sdk: Int): List<Way> =
        if (sdk >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) listOf(Way.PROMPT, Way.PROVIDERS, Way.SETTINGS) else listOf(Way.PROMPT, Way.SETTINGS)

    /** Any request code of the app's own: Android's prompt answers with it, and the answer goes unread. */
    private const val PROMPT_REQUEST = 0x7a1

    /**
     * What the card shows (autofill_offer.dart): `apiLevel`; `autofillSupported`
     * (Android has autofill for this person) and `autofillOn` (PassBundle is the
     * service); `passkeysAvailable` (Android 14 or later, with Credential Manager)
     * and `passkeysOn` (PassBundle's provider is switched on).
     */
    fun status(context: Context): Map<String, Any> {
        val autofill = context.getSystemService(AutofillManager::class.java)?.takeIf { it.isAutofillSupported }
        val passkeys = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) passkeyProvider(context) else null
        return mapOf(
            "apiLevel" to Build.VERSION.SDK_INT,
            "autofillSupported" to (autofill != null),
            "autofillOn" to (autofill?.hasEnabledAutofillServices() == true),
            "passkeysAvailable" to (passkeys != null),
            "passkeysOn" to (passkeys == true),
        )
    }

    /** Whether PassBundle's passkey provider is on, or null without Credential Manager. Android 14 and later. */
    @RequiresApi(Build.VERSION_CODES.UPSIDE_DOWN_CAKE)
    private fun passkeyProvider(context: Context): Boolean? {
        val credentials = context.getSystemService(android.credentials.CredentialManager::class.java) ?: return null
        return try {
            credentials.isEnabledCredentialProviderService(ComponentName(context, PasskeyProvider::class.java))
        } catch (e: RuntimeException) {
            false
        }
    }

    /** Opens the first of [ways] the phone has: the [Way]'s id, or null if none opened. */
    fun turnOnAutofill(activity: Activity): String? =
        ways(Build.VERSION.SDK_INT).firstOrNull { open(activity, it) }?.id

    private fun open(activity: Activity, way: Way): Boolean =
        when (way) {
            Way.PROMPT ->
                start(
                    activity,
                    "autofill",
                    Intent(Settings.ACTION_REQUEST_SET_AUTOFILL_SERVICE, Uri.parse("package:${activity.packageName}")),
                    forResult = true,
                )
            Way.PROVIDERS -> Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE && sendProviderSettings(activity, "autofill")
            Way.SETTINGS -> start(activity, "autofill", Intent(Settings.ACTION_SETTINGS), forResult = false)
        }

    /** The system's list of passkey providers, Android 14 and later: true if it opened. */
    fun openPasskeySettings(activity: Activity): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE && sendProviderSettings(activity, "passkeys")

    private fun start(activity: Activity, what: String, intent: Intent, forResult: Boolean): Boolean {
        if (standIn) {
            opened += mapOf("for" to what, "action" to intent.action, "data" to intent.dataString, "forResult" to forResult)
            return true
        }
        return try {
            if (forResult) activity.startActivityForResult(intent, PROMPT_REQUEST) else activity.startActivity(intent)
            true
        } catch (e: ActivityNotFoundException) {
            false
        } catch (e: SecurityException) {
            false
        }
    }

    /**
     * Credential Manager's settings page (androidx.credentials'
     * createSettingsPendingIntent), sent while the app is on the screen.
     * Android 14 and later.
     */
    @RequiresApi(Build.VERSION_CODES.UPSIDE_DOWN_CAKE)
    private fun sendProviderSettings(activity: Activity, what: String): Boolean {
        val settings =
            try {
                androidx.credentials.CredentialManager.create(activity).createSettingsPendingIntent()
            } catch (e: RuntimeException) {
                return false
            }
        if (standIn) {
            opened += mapOf("for" to what, "pendingIntent" to true, "creator" to settings.creatorPackage, "activity" to settings.isActivity)
            return true
        }
        // The app is in front, sending its own PendingIntent: from Android 14
        // the sender says so, or Android may count it as an app in the
        // background opening a screen.
        val options =
            ActivityOptions.makeBasic().apply {
                @Suppress("DEPRECATION")
                setPendingIntentBackgroundActivityStartMode(ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOWED)
            }
        return try {
            settings.send(activity, 0, null, null, null, null, options.toBundle())
            true
        } catch (e: PendingIntent.CanceledException) {
            false
        }
    }

    /** Tests only: what would have opened, in order, while the stand-in is on. */
    private val opened = mutableListOf<Map<String, Any?>>()
    private var standIn = false

    /**
     * Tests only: records what "Turn on" would open instead of opening it, so
     * a test checks the intent without Android's real prompt (as Biometric.kt
     * stands in for the biometric prompt). False in a release build.
     */
    fun useStandIn(context: Context, on: Boolean): Boolean {
        if ((context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) == 0) return false
        standIn = on
        opened.clear()
        return true
    }

    /** Tests only: what the stand-in recorded. */
    fun openedByStandIn(): List<Map<String, Any?>> = opened.toList()
}
