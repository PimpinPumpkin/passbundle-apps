package com.passbundle.app

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.service.autofill.Dataset
import android.service.autofill.FillResponse
import android.service.autofill.SaveInfo
import android.view.autofill.AutofillValue
import android.widget.RemoteViews
import org.json.JSONObject

/**
 * What PassBundle offers Android's autofill (docs/phones.md, "Autofill"): the
 * matching logins by name and username, each filled only when picked (through
 * FillActivity, which takes the password from the core then), or "Unlock
 * PassBundle" while the vault is locked. A page's domain counts only from a
 * browser (Browsers.kt); another app that can open web links and gives one is
 * offered to be trusted as a browser, in PassBundle's own dialog.
 */
object Responses {
    private var requests = 0

    fun presentation(context: Context, title: String, subtitle: String?): RemoteViews =
        RemoteViews(context.packageName, R.layout.autofill_item).apply {
            setTextViewText(R.id.title, title)
            setTextViewText(R.id.subtitle, subtitle ?: "")
        }

    /** FillActivity, to unlock or to fill one login, for these fields. */
    fun pending(context: Context, fields: Fields, mode: String, item: String? = null, asks: Boolean = false) =
        PendingIntent.getActivity(
            context,
            ++requests,
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

    /** The response for these fields, from what the core says (Core.fillable); null offers nothing. */
    fun forFields(context: Context, fields: Fields): FillResponse? {
        val ids = fields.ids
        if (ids.isEmpty()) return null
        Core.openIfClosed(context.filesDir.absolutePath)
        val trusted = fields.webDomain != null && Browsers.trusts(context, fields.packageName)
        val domain = fields.webDomain.takeIf { trusted }
        val answer = JSONObject(Core.fillable(fields.packageName, domain ?: "") ?: return null)
        if (answer.optBoolean("busy") || answer.optBoolean("signedOut")) return null
        val save = saveInfo(fields)
        if (answer.optBoolean("locked")) {
            return FillResponse.Builder()
                .setAuthentication(
                    ids,
                    pending(context, fields, FillActivity.UNLOCK),
                    presentation(context, "Unlock PassBundle", "to fill this in"),
                )
                .apply { save?.let { setSaveInfo(it) } }
                .build()
        }
        val items = answer.optJSONArray("items") ?: return null
        val response = FillResponse.Builder()
        save?.let { response.setSaveInfo(it) }
        val offerTrust = fields.webDomain != null && !trusted && Browsers.couldBe(context, fields.packageName)
        // Nothing to fill: only the offer to save what the person signs in with.
        if (items.length() == 0 && !offerTrust) return save?.let { response.build() }
        for (i in 0 until items.length()) {
            val item = items.getJSONObject(i)
            val id = item.getString("id")
            @Suppress("DEPRECATION")
            val dataset = Dataset.Builder(presentation(context, item.optString("name"), item.optString("username").takeIf { !item.isNull("username") }))
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
            val dataset = Dataset.Builder(presentation(context, "Logins for ${fields.webDomain}", "Trust $app as a browser first"))
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
