package com.passbundle.app

import android.app.PendingIntent
import android.content.Intent
import android.os.CancellationSignal
import android.service.autofill.AutofillService
import android.service.autofill.FillCallback
import android.service.autofill.FillRequest
import android.service.autofill.SaveCallback
import android.service.autofill.SaveRequest
import org.json.JSONObject

/**
 * PassBundle as Android's autofill service (docs/phones.md, "Autofill"). It
 * runs in the app's own process, so it shares the unlocked core with the app:
 * when the app is locked, so is autofill.
 */
class PassBundleAutofill : AutofillService() {
    override fun onFillRequest(request: FillRequest, cancellationSignal: CancellationSignal, callback: FillCallback) {
        val structure = request.fillContexts.lastOrNull()?.structure ?: return callback.onSuccess(null)
        val fields = Fields.find(structure)
        // Never PassBundle's own screens.
        if (fields.packageName == packageName) return callback.onSuccess(null)
        callback.onSuccess(Responses.forFields(this, fields))
    }

    companion object {
        /** How long a sign-in to save waits for the app's screens. */
        private const val SAVE_WAITS_MS = 10 * 60 * 1000L

        private var waiting: Map<String, String>? = null
        private var waitingSince = 0L

        /**
         * The sign-in to save, once, if it's still waiting (the "pendingSave"
         * channel method). The screens take it unlocked, so the core says then
         * whether it's a new password for a login it has: the one login for
         * the same app or site with the same username, by the rules autofill
         * fills with. That login's `id` comes with it, to update it.
         */
        @Synchronized
        fun takeSave(): Map<String, String>? {
            val save = waiting?.takeIf { System.currentTimeMillis() - waitingSince < SAVE_WAITS_MS }
            waiting = null
            save ?: return null
            val answer = runCatching { JSONObject(Core.fillable(save["package"] ?: "", save["domain"] ?: "") ?: "{}") }.getOrNull()
            val items = answer?.optJSONArray("items") ?: return save
            val same =
                (0 until items.length())
                    .map { items.getJSONObject(it) }
                    .filter { !it.isNull("username") && it.optString("username") == save["username"] }
            return if (same.size == 1) save + ("id" to same[0].getString("id")) else save
        }

        @Synchronized
        private fun offer(save: Map<String, String>) {
            waiting = save
            waitingSince = System.currentTimeMillis()
        }
    }

    /**
     * Someone signed in with something new and chose "Save to PassBundle": the
     * app opens its editor with it filled in, to save once they've looked
     * (docs/phones.md, "Autofill"). The sign-in stays in this process, where
     * the app's screens take it; the intent only opens the app, so no other
     * app can hand the editor a sign-in. Android 14 before its March 2024
     * update can stop the app from opening itself here: then it waits for the
     * person to open PassBundle.
     */
    override fun onSaveRequest(request: SaveRequest, callback: SaveCallback) {
        val structure = request.fillContexts.lastOrNull()?.structure ?: return callback.onSuccess()
        val fields = Fields.find(structure)
        val (username, password) = Fields.values(structure, fields)
        if (password.isNullOrEmpty()) return callback.onSuccess()
        // A page's domain only from a browser (Browsers.kt); another app's sign-in is saved for the app.
        val domain = fields.webDomain?.takeIf { Browsers.trusts(this, fields.packageName) }
        val name = domain ?: Browsers.label(this, fields.packageName)
        offer(
            mapOf(
                "name" to name,
                "username" to (username ?: ""),
                "password" to password,
                "uri" to (domain?.let { "https://$it" } ?: "androidapp://${fields.packageName}"),
                // Where it was signed in, to find the login it updates.
                "package" to fields.packageName,
                "domain" to (domain ?: ""),
            ),
        )
        val open = Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        callback.onSuccess(
            PendingIntent.getActivity(this, 0, open, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
                .intentSender,
        )
    }
}
