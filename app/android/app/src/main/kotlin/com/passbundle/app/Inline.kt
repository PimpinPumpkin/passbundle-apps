package com.passbundle.app

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.IntentSender
import android.graphics.drawable.Icon
import android.os.Build
import android.service.autofill.Dataset
import android.service.autofill.FillRequest
import android.service.autofill.FillResponse
import android.service.autofill.InlinePresentation
import android.view.autofill.AutofillId
import android.view.autofill.AutofillManager
import android.view.inputmethod.InlineSuggestionsRequest
import android.widget.RemoteViews
import androidx.annotation.RequiresApi
import androidx.autofill.inline.UiVersions
import androidx.autofill.inline.v1.InlineSuggestionUi

/**
 * Autofill's entries as suggestions on the keyboard (docs/phones.md,
 * "Autofill"), on Android 11 and later, for a keyboard that asks for them:
 * the same entries Android's list offers, in its order (Responses.entries,
 * and "Unlock PassBundle" or "Sign in to PassBundle"), as many as the
 * keyboard takes, each drawn the way it asks. Like the list's, an entry
 * holds no value: it says what it is, and picking it opens FillActivity,
 * which fills it then. Older Android, and a keyboard that doesn't ask, get
 * Android's list as before.
 *
 * Which entries the keyboard gets is a plain decision (`specs`, tested on
 * the computer: src/test, InlineTest); drawing them needs Android's classes
 * (`Keyboard`).
 */
object Inline {
    /**
     * Which of the keyboard's presentation specs each entry is drawn with,
     * for as many entries as it takes (`most`), in order: the first spec for
     * the first entry, and so on, the last spec for the rest, as Android
     * reads them. The entries past that aren't on the keyboard. None when it
     * gives no spec, or one of those needed isn't one PassBundle can draw
     * (`fits`, a spec each, AndroidX's first inline style): a keyboard that
     * asks for another look gets no suggestions from PassBundle.
     */
    fun specs(entries: Int, most: Int, fits: List<Boolean>): List<Int> {
        if (fits.isEmpty() || most <= 0) return emptyList()
        val shown = (0 until minOf(entries, most)).map { minOf(it, fits.size - 1) }
        return if (shown.all { fits[it] }) shown else emptyList()
    }

    /**
     * The keyboard's request for suggestions, on Android 11 and later; null
     * for a keyboard that doesn't ask, or a request that can't be read (then
     * Android's list is all there is, as before).
     */
    fun keyboard(context: Context, request: FillRequest): Keyboard? {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return null
        return runCatching { request.inlineSuggestionsRequest?.let { Keyboard(context, it) } }.getOrNull()
    }

    /**
     * The same request, as Android hands it to FillActivity with the screen
     * when someone picks "Unlock PassBundle" or trusts a browser, for the
     * list it hands back.
     */
    fun keyboard(context: Context, intent: Intent): Keyboard? {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return null
        return runCatching {
            val request =
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    intent.getParcelableExtra(AutofillManager.EXTRA_INLINE_SUGGESTIONS_REQUEST, InlineSuggestionsRequest::class.java)
                } else {
                    @Suppress("DEPRECATION")
                    intent.getParcelableExtra<InlineSuggestionsRequest>(AutofillManager.EXTRA_INLINE_SUGGESTIONS_REQUEST)
                }
            request?.let { Keyboard(context, it) }
        }.getOrNull()
    }

    /** What a long press on a suggestion opens: PassBundle itself. */
    private const val ATTRIBUTION = 7001

    /**
     * A keyboard's request: how many suggestions it takes and how each is
     * drawn. A suggestion has an entry's title and second line, and
     * PassBundle's mark, never a value. Android's classes for it stay in
     * here, so nothing else needs Android 11.
     */
    @RequiresApi(Build.VERSION_CODES.R)
    class Keyboard(private val context: Context, private val request: InlineSuggestionsRequest) {
        private val presentationSpecs = request.inlinePresentationSpecs

        /** Each spec drawn as PassBundle draws (AndroidX's first inline style). */
        private val fits =
            presentationSpecs.map { spec -> UiVersions.getVersions(spec.style).contains(UiVersions.INLINE_UI_VERSION_1) }

        /** The spec for each of so many entries, as many as the keyboard takes (`Inline.specs`). */
        fun plan(entries: Int): List<Int> = Inline.specs(entries, request.maxSuggestionCount, fits)

        /**
         * Each entry's suggestion on its dataset (entries and datasets in
         * the same order), for as many as the keyboard takes. If one can't
         * be drawn, none is, rather than some.
         */
        fun suggest(datasets: List<Dataset.Builder>, entries: List<Pair<String, String?>>) {
            val suggestions =
                runCatching { plan(entries.size).mapIndexed { i, spec -> presentation(entries[i].first, entries[i].second, spec) } }
                    .getOrNull() ?: return
            for ((i, suggestion) in suggestions.withIndex()) {
                @Suppress("DEPRECATION")
                datasets[i].setInlinePresentation(suggestion)
            }
        }

        /**
         * The one entry for the whole response ("Unlock PassBundle", "Sign
         * in to PassBundle") on the keyboard too: whether it's there. If
         * not, the caller sets it for Android's list alone.
         */
        fun authenticate(
            response: FillResponse.Builder,
            ids: Array<AutofillId>,
            sender: IntentSender,
            listed: RemoteViews,
            title: String,
            subtitle: String,
        ): Boolean {
            val spec = plan(1).firstOrNull() ?: return false
            val suggestion = runCatching { presentation(title, subtitle, spec) }.getOrNull() ?: return false
            @Suppress("DEPRECATION")
            response.setAuthentication(ids, sender, listed, suggestion)
            return true
        }

        /** One entry's suggestion, drawn with the spec `plan` gave it. */
        private fun presentation(title: String, subtitle: String?, spec: Int): InlinePresentation {
            val content =
                InlineSuggestionUi.newContentBuilder(attribution())
                    .setTitle(title)
                    .apply { if (!subtitle.isNullOrEmpty()) setSubtitle(subtitle) }
                    .setStartIcon(Icon.createWithResource(context, R.drawable.autofill_mark))
                    .setContentDescription(listOfNotNull(title, subtitle?.ifEmpty { null }).joinToString(", "))
                    .build()
            return InlinePresentation(content.slice, presentationSpecs[spec], false)
        }

        private fun attribution(): PendingIntent =
            PendingIntent.getActivity(
                context,
                ATTRIBUTION,
                Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
    }
}
