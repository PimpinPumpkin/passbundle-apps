package com.passbundle.app

import android.app.PendingIntent
import android.app.assist.AssistStructure
import android.content.Intent
import android.os.CancellationSignal
import android.service.autofill.AutofillService
import android.service.autofill.FillCallback
import android.service.autofill.FillRequest
import android.service.autofill.SaveCallback
import android.service.autofill.SaveRequest
import org.json.JSONObject
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/**
 * PassBundle as Android's autofill service (docs/phones.md, "Autofill"). It
 * runs in the app's own process, so it shares the unlocked core with the app:
 * when the app is locked, so is autofill.
 */
class PassBundleAutofill : AutofillService() {
    override fun onFillRequest(request: FillRequest, cancellationSignal: CancellationSignal, callback: FillCallback) {
        val structure = request.fillContexts.lastOrNull()?.structure ?: return callback.onSuccess(null)
        // Whatever an app's screen holds, reading it never stops PassBundle
        // (and the unlocked vault with it): a screen it can't read gets nothing.
        val fields = runCatching { Fields.find(structure) }.getOrNull() ?: return callback.onSuccess(null)
        // Never PassBundle's own screens.
        if (fields.packageName == packageName) return callback.onSuccess(null)
        // Checking an app's sites can reach the network: off the main thread,
        // one request at a time, for a moment at most, well inside Android's
        // wait for an answer. A request Android cancelled meanwhile is skipped.
        val cancelled = AtomicBoolean(false)
        cancellationSignal.setOnCancelListener { cancelled.set(true) }
        filling.execute {
            if (cancelled.get()) return@execute
            val keyboard = Inline.keyboard(this, request)
            val response = runCatching { Responses.forFields(this, fields, SITES_WAIT_MS, keyboard) }.getOrNull()
            if (!cancelled.get()) runCatching { callback.onSuccess(response) }
        }
    }

    companion object {
        /** How long a fill waits for checking the sites an app names (AppSites.kt). */
        private const val SITES_WAIT_MS = 2000L

        /** Fill requests, one at a time. */
        private val filling = Executors.newSingleThreadExecutor()

        /** How long a sign-in (or a card, or an address) to save waits for the app's screens. */
        private const val SAVE_WAITS_MS = 10 * 60 * 1000L

        private var waiting: Map<String, String>? = null
        private var waitingSince = 0L

        /**
         * The sign-in, card, or address to save, once, if it's still waiting
         * (the "pendingSave" channel method). The screens take it unlocked,
         * so the core says then whether it's a new password for a login it
         * has: the one login for the same app or site with the same
         * username, by the rules autofill fills with, in any unlocked
         * account. That login's `id` and its `account` come with it, to
         * update it there. A card or an address (`kind`) is checked as
         * `checked` says.
         */
        @Synchronized
        fun takeSave(): Map<String, Any>? {
            val save = waiting?.takeIf { System.currentTimeMillis() - waitingSince < SAVE_WAITS_MS }
            waiting = null
            save ?: return null
            val kind = save["kind"]
            if (kind == "card" || kind == "identity") return checked(kind, save)
            val answer = runCatching { JSONObject(Core.fillable(save["package"] ?: "", save["domain"] ?: "") ?: "{}") }.getOrNull()
            val items = answer?.optJSONArray("items") ?: return save
            val same =
                (0 until items.length())
                    .map { items.getJSONObject(it) }
                    .filter { !it.isNull("username") && it.optString("username") == save["username"] }
            return if (same.size == 1) {
                save + ("id" to same[0].getString("id")) + ("account" to same[0].optString("account"))
            } else {
                save
            }
        }

        /**
         * A card or an address to save, now that PassBundle is unlocked: one
         * an unlocked account has already (the same number, or the same
         * street and postal code) isn't saved again, and the screens say so
         * by its name (`already`), with nothing else of it; otherwise it
         * comes with the unlocked accounts where one may be made (`mayAdd`),
         * since an organization's policy may remove the kind in some. While
         * the core can't say (locked, or busy), it comes as it is, and saving
         * it checks the policy (core/src/app.rs, `save`).
         */
        private fun checked(kind: String, save: Map<String, String>): Map<String, Any> {
            val answer =
                runCatching {
                    JSONObject(Core.alreadySaved(kind, save["number"] ?: "", save["address1"] ?: "", save["postalCode"] ?: "") ?: "{}")
                }.getOrNull()
            if (answer == null || answer.optBoolean("locked", true)) return save
            answer.optJSONObject("saved")?.let { return mapOf("kind" to kind, "already" to it.optString("name")) }
            val mayAdd = answer.optJSONArray("mayAdd")
            return save + ("mayAdd" to (0 until (mayAdd?.length() ?: 0)).map { mayAdd!!.getString(it) })
        }

        @Synchronized
        private fun offer(save: Map<String, String>) {
            waiting = save
            waitingSince = System.currentTimeMillis()
        }
    }

    /**
     * Someone signed in with something new, or typed a card or an address,
     * and chose "Save to PassBundle": the app opens its editor with it
     * filled in, to save once they've looked (docs/phones.md, "Autofill").
     * What was typed stays in this process, where the app's screens take
     * it; the intent only opens the app, so no other app can hand the editor
     * a sign-in, a card, or an address. Android 14 before its March 2024
     * update can stop the app from opening itself here: then it waits for
     * the person to open PassBundle.
     */
    override fun onSaveRequest(request: SaveRequest, callback: SaveCallback) {
        val structure = request.fillContexts.lastOrNull()?.structure ?: return callback.onSuccess()
        val fields = runCatching { Fields.find(structure) }.getOrNull() ?: return callback.onSuccess()
        // What Android offered to save there (Responses.kt), else what the screen holds.
        val offered = request.clientState?.getString(Responses.SAVES)
        val save = runCatching { toSave(structure, fields, offered) }.getOrNull() ?: return callback.onSuccess()
        offer(save)
        val open = Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        callback.onSuccess(
            PendingIntent.getActivity(this, 0, open, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
                .intentSender,
        )
    }

    /**
     * What to save from the screen: a sign-in where there's a password, as
     * before; else the card or the address Android offered to save
     * (Saving.kt), from what its fields hold. Null when there's nothing to
     * save.
     */
    private fun toSave(structure: AssistStructure, fields: Fields, offered: String?): Map<String, String>? {
        val what = offered?.let { name -> Saving.What.entries.firstOrNull { it.name == name } } ?: Saving.offer(fields.screen, null)?.what
        if (what == Saving.What.CARD || what == Saving.What.ADDRESS) {
            val values = Fields.read(structure, fields)
            return if (what == Saving.What.CARD) Saving.card(fields.screen) { values[it] } else Saving.identity(fields.screen) { values[it] }
        }
        val (username, password) = Fields.values(structure, fields)
        if (password.isNullOrEmpty()) return null
        // Never a one-time code as a password (DESIGN.md in the meta repo, "One-time codes never become passwords").
        if (Roles.looksLikeACode(username, password)) return null
        // A page's domain only from a browser (Browsers.kt); another app's sign-in is saved for the app.
        val domain = Browsers.domain(fields.webDomain)?.takeIf { Browsers.trusts(this, fields.packageName) }
        val name = domain ?: Browsers.label(this, fields.packageName)
        return mapOf(
            "name" to name,
            "username" to (username ?: ""),
            "password" to password,
            "uri" to (domain?.let { "https://$it" } ?: "androidapp://${fields.packageName}"),
            // Where it was signed in, to find the login it updates.
            "package" to fields.packageName,
            "domain" to (domain ?: ""),
        )
    }
}
