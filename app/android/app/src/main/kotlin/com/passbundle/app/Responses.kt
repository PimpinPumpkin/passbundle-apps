package com.passbundle.app

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.util.Log
import android.service.autofill.Dataset
import android.service.autofill.FillResponse
import android.service.autofill.SaveInfo
import android.view.autofill.AutofillValue
import android.widget.RemoteViews
import org.json.JSONObject
import java.util.concurrent.atomic.AtomicInteger

/**
 * What PassBundle offers Android's autofill (docs/phones.md, "Autofill"): the
 * matching logins by name and username, each filled only when picked (through
 * FillActivity, which takes the password from the core then), or "Unlock
 * PassBundle" while the vault is locked. A page's domain counts only from a
 * browser (Browsers.kt); another app that can open web links and gives one is
 * offered to be trusted as a browser, in PassBundle's own dialog.
 */
object Responses {
    private val requests = AtomicInteger()

    fun presentation(context: Context, title: String, subtitle: String?): RemoteViews =
        RemoteViews(context.packageName, R.layout.autofill_item).apply {
            setTextViewText(R.id.title, title)
            setTextViewText(R.id.subtitle, subtitle ?: "")
        }

    /** FillActivity, to unlock or to fill one login, for these fields. */
    fun pending(context: Context, fields: Fields, mode: String, item: String? = null, asks: Boolean = false) =
        PendingIntent.getActivity(
            context,
            requests.incrementAndGet(),
            fields.into(Intent(context, FillActivity::class.java))
                .putExtra(FillActivity.MODE, mode)
                .putExtra(FillActivity.ITEM, item)
                .putExtra(FillActivity.ASKS, asks),
            // Mutable: Android adds the screen's structure when it starts it.
            PendingIntent.FLAG_CANCEL_CURRENT or PendingIntent.FLAG_MUTABLE,
        ).intentSender

    /** Android offers "Save to PassBundle" for a sign-in with a password (PassBundleAutofill.onSaveRequest). */
    private fun saveInfo(fields: Fields): SaveInfo? {
        val password = fields.password ?: return null
        val types =
            SaveInfo.SAVE_DATA_TYPE_PASSWORD or
                (if (fields.username != null) SaveInfo.SAVE_DATA_TYPE_USERNAME else 0)
        return SaveInfo.Builder(types, arrayOf(password))
            .apply { fields.username?.let { setOptionalIds(arrayOf(it)) } }
            .build()
    }

    /**
     * The logins for these fields, from what the core says (Core.fillable),
     * or null when it says nothing (locked, or busy). For an app that isn't a
     * browser, the logins of the sites it names and that name it back
     * (AppSites.kt) come after its own, each marked with its site (`from`);
     * `waitMs` is how long to wait for checking those sites (0: only what was
     * checked lately). A page the app shows itself (a WebView: an in-app
     * browser, say) gets a site's logins only if it's that site, so a page
     * opened inside the app can't be handed the logins of the sites that
     * trust the app.
     */
    fun logins(context: Context, fields: Fields, domain: String?, waitMs: Long): List<JSONObject>? {
        val answer = JSONObject(Core.fillable(fields.packageName, domain ?: "") ?: return null)
        val items = answer.optJSONArray("items") ?: return null
        val found = LinkedHashMap<String, JSONObject>()
        for (i in 0 until items.length()) items.getJSONObject(i).let { found[it.getString("id")] = it }
        if (domain == null) {
            val page = Browsers.domain(fields.webDomain)
            for (site in AppSites.linked(context, fields.packageName, waitMs)) {
                if (page != null && page != site && !page.endsWith(".$site")) continue
                val more = runCatching { JSONObject(Core.fillable(fields.packageName, site) ?: "{}") }.getOrNull()
                val siteItems = more?.optJSONArray("items") ?: continue
                for (i in 0 until siteItems.length()) siteItems.getJSONObject(i).let { found.putIfAbsent(it.getString("id"), it.put("from", site)) }
            }
        }
        return found.values.toList()
    }

    /**
     * The response for these fields, from what the core says; null offers
     * nothing. `waitMs` as for `logins`.
     */
    fun forFields(context: Context, fields: Fields, waitMs: Long = 0): FillResponse? {
        val ids = fields.ids
        if (ids.isEmpty()) return null
        Core.openIfClosed(context.filesDir.absolutePath)
        // Only a hostname counts as a page's domain, and only from a browser.
        val claimed = Browsers.domain(fields.webDomain)
        val trusted = claimed != null && Browsers.trusts(context, fields.packageName)
        val domain = claimed.takeIf { trusted }
        val answer = JSONObject(Core.fillable(fields.packageName, domain ?: "") ?: return null)
        if (answer.optBoolean("busy") || answer.optBoolean("signedOut")) return null
        val save = saveInfo(fields)
        if (answer.optBoolean("locked")) {
            // The app's sites checked meanwhile, for once the person unlocks.
            if (domain == null) AppSites.warm(context, fields.packageName)
            return FillResponse.Builder()
                .setAuthentication(
                    ids,
                    pending(context, fields, FillActivity.UNLOCK),
                    presentation(context, "Unlock PassBundle", "to fill this in"),
                )
                .apply { save?.let { setSaveInfo(it) } }
                .build()
        }
        val items = logins(context, fields, domain, waitMs) ?: return null
        val response = FillResponse.Builder()
        save?.let { response.setSaveInfo(it) }
        val offerTrust = claimed != null && !trusted && Browsers.couldBe(context, fields.packageName)
        // Debug builds only, for the emulator tests' logs: what autofill decided, never a value.
        if ((context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0) {
            Log.d("PassBundle", "autofill ${fields.packageName}: web domain ${fields.webDomain}, trusted $trusted, offers trust $offerTrust")
        }
        // Nothing to fill: only the offer to save what the person signs in with.
        if (items.isEmpty() && !offerTrust) return save?.let { response.build() }
        for (item in items) {
            val id = item.getString("id")
            @Suppress("DEPRECATION")
            // A site's login in an app says which site it's from, as a browser's address bar would.
            val username = item.optString("username").takeIf { !item.isNull("username") && it.isNotEmpty() }
            val subtitle = listOfNotNull(username, item.optString("from").ifEmpty { null }).joinToString(" · ").ifEmpty { null }
            val dataset = Dataset.Builder(presentation(context, item.optString("name"), subtitle))
            // Placeholders: the values come from FillActivity once this one is picked.
            for (field in ids) {
                @Suppress("DEPRECATION")
                dataset.setValue(field, null)
            }
            dataset.setAuthentication(pending(context, fields, FillActivity.FILL, id, item.optBoolean("reprompt")))
            response.addDataset(dataset.build())
        }
        if (offerTrust) {
            val app = Browsers.label(context, fields.packageName)
            @Suppress("DEPRECATION")
            val dataset = Dataset.Builder(presentation(context, "Logins for $claimed", "Trust $app as a browser first"))
            for (field in ids) {
                @Suppress("DEPRECATION")
                dataset.setValue(field, null)
            }
            dataset.setAuthentication(pending(context, fields, FillActivity.TRUST))
            response.addDataset(dataset.build())
        }
        return response.build()
    }

    /** One login's values for the fields, taken from the core only now that it was picked. */
    fun filled(context: Context, fields: Fields, item: String): Dataset? {
        fun text(what: String): String? {
            val bytes = Core.value(item, what) ?: return null
            return try {
                String(bytes, Charsets.UTF_8)
            } finally {
                bytes.fill(0)
            }
        }
        @Suppress("DEPRECATION")
        val dataset = Dataset.Builder(presentation(context, "PassBundle", null))
        var any = false
        fields.username?.let { id -> text("username")?.let { @Suppress("DEPRECATION") dataset.setValue(id, AutofillValue.forText(it)); any = true } }
        fields.password?.let { id -> text("password")?.let { @Suppress("DEPRECATION") dataset.setValue(id, AutofillValue.forText(it)); any = true } }
        return if (any) dataset.build() else null
    }
}
