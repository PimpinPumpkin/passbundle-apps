package com.passbundle.app

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.IntentSender
import android.content.pm.ApplicationInfo
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.service.autofill.Dataset
import android.service.autofill.FillResponse
import android.service.autofill.LuhnChecksumValidator
import android.service.autofill.RegexValidator
import android.service.autofill.SaveInfo
import android.service.autofill.Validator
import android.service.autofill.Validators
import android.view.autofill.AutofillId
import android.view.autofill.AutofillValue
import android.widget.RemoteViews
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.atomic.AtomicInteger
import java.util.regex.Pattern

/**
 * What PassBundle offers Android's autofill (docs/phones.md, "Autofill"):
 * - on a sign-in screen, the matching logins of every unlocked account, by
 *   name and username (and, with more than one account on the phone, the
 *   account's name), each filled only from its own account when picked
 *   (through FillActivity, which takes the password from the core then),
 *   with its one-time code too when the screen asks for one;
 * - on a screen that asks only for a one-time code (the one after signing
 *   in), each matching login's current code, the login just filled first;
 * - on a card's fields, every card, and on an identity's, every identity,
 *   whatever the app or the site, since they aren't for a website;
 * - or "Unlock PassBundle" while the vault is locked, or "Sign in to
 *   PassBundle" with no account on the phone (Unlocking.offer).
 * A page's domain counts only from a browser (Browsers.kt); another app that
 * can open web links and gives one is offered to be trusted as a browser, in
 * PassBundle's own dialog. A keyboard that asks for suggestions (Android 11
 * and later) gets the same entries on the keyboard (Inline.kt). And Android
 * offers to save a new sign-in, card, or address there (Saving.kt).
 */
object Responses {
    private val requests = AtomicInteger()

    fun presentation(context: Context, title: String, subtitle: String?): RemoteViews =
        RemoteViews(context.packageName, R.layout.autofill_item).apply {
            setTextViewText(R.id.title, title)
            setTextViewText(R.id.subtitle, subtitle ?: "")
        }

    /**
     * FillActivity, to unlock an account or to fill one item from its
     * account, for these fields: the account by its id on this device, as
     * the core named it, and the kind of fill (a login, a login's code
     * alone, a card, an identity). Every extra FillActivity reads is set
     * here, so nothing added to the intent later stands in for one.
     */
    fun pending(
        context: Context,
        fields: Fields,
        mode: String,
        account: String,
        item: String? = null,
        asks: Boolean = false,
        label: String? = null,
        kind: Kind = Kind.LOGIN,
    ) =
        PendingIntent.getActivity(
            context,
            requests.incrementAndGet(),
            fields.into(Intent(context, FillActivity::class.java))
                .putExtra(FillActivity.MODE, mode)
                .putExtra(FillActivity.ACCOUNT, account)
                .putExtra(FillActivity.ITEM, item)
                .putExtra(FillActivity.ASKS, asks)
                .putExtra(FillActivity.LABEL, label)
                .putExtra(FillActivity.KIND, kind.id),
            // Mutable: Android adds the screen's structure when it starts it.
            PendingIntent.FLAG_CANCEL_CURRENT or PendingIntent.FLAG_MUTABLE,
        ).intentSender

    /**
     * Android offers "Save to PassBundle" (PassBundleAutofill.onSaveRequest):
     * for a sign-in with a password, as before, or a card or an address
     * (Saving.offer), with Android's own check of what was typed.
     */
    private fun saveInfo(offer: Saving.Offer<AutofillId>): SaveInfo {
        val type =
            when (offer.what) {
                Saving.What.LOGIN ->
                    SaveInfo.SAVE_DATA_TYPE_PASSWORD or
                        (if (offer.optional.isNotEmpty()) SaveInfo.SAVE_DATA_TYPE_USERNAME else 0)
                Saving.What.CARD -> SaveInfo.SAVE_DATA_TYPE_CREDIT_CARD
                Saving.What.ADDRESS -> SaveInfo.SAVE_DATA_TYPE_ADDRESS
            }
        // An address needs no one field: two of its parts, whichever (the check).
        val builder =
            if (offer.required.isEmpty()) SaveInfo.Builder(type) else SaveInfo.Builder(type, offer.required.toTypedArray())
        if (offer.optional.isNotEmpty()) builder.setOptionalIds(offer.optional.toTypedArray())
        offer.check?.let { builder.setValidator(validator(it)) }
        return builder.build()
    }

    /** What the response offers to save (Saving.What's name), in its client state, which Android hands back with the save. */
    const val SAVES = "com.passbundle.app.saves"

    /** The offer to save on a response, and what it is, for PassBundleAutofill.onSaveRequest. */
    private fun FillResponse.Builder.offerToSave(offer: Saving.Offer<AutofillId>): FillResponse.Builder =
        setSaveInfo(saveInfo(offer)).setClientState(Bundle().apply { putString(SAVES, offer.what.name) })

    /** Android's validator for a check (Saving.Check): Android runs it on what was typed, before it asks. */
    private fun validator(check: Saving.Check<AutofillId>): Validator =
        when (check) {
            is Saving.Check.Matches -> RegexValidator(check.id, Pattern.compile(check.regex))
            is Saving.Check.Luhn -> LuhnChecksumValidator(check.id)
            is Saving.Check.All -> Validators.and(*check.checks.map(::validator).toTypedArray())
            is Saving.Check.AnyOf -> Validators.or(*check.checks.map(::validator).toTypedArray())
            is Saving.Check.Not -> Validators.not(validator(check.check))
        }

    private fun strings(array: JSONArray?): List<String> = array?.let { a -> (0 until a.length()).map { a.getString(it) } }.orEmpty()

    /**
     * What PassBundle knows for Android's offer to save a card or an
     * address on these fields (Core.saveHints): where one may be made, and
     * what tells a new one from one the person has. Null while it's locked
     * or busy, or where there's no card's or identity's field to save from:
     * then every one is offered, and checked once the person accepts.
     */
    private fun known(fields: Fields): Saving.Known? {
        if (fields.password != null || (fields.of(Kind.CARD).isEmpty() && fields.of(Kind.IDENTITY).isEmpty())) return null
        val answer = JSONObject(Core.saveHints() ?: return null)
        if (answer.optBoolean("locked", true)) return null
        val card = answer.optJSONObject("card") ?: return null
        val identity = answer.optJSONObject("identity") ?: return null
        val addresses = identity.optJSONArray("addresses")
        return Saving.Known(
            mayAddCard = card.optBoolean("mayAdd", true),
            mayAddAddress = identity.optBoolean("mayAdd", true),
            last4 = strings(card.optJSONArray("last4")),
            addresses =
                (0 until (addresses?.length() ?: 0)).map { i ->
                    val address = addresses!!.getJSONObject(i)
                    strings(address.optJSONArray("street")) to address.optString("postalCode")
                },
        )
    }

    /** A value the core gave, or null when it gave none (Android's JSON reads a null as the word "null"). */
    fun text(item: JSONObject, key: String): String? = item.optString(key).takeIf { !item.isNull(key) && it.isNotEmpty() }

    /**
     * The logins for these fields, from what the core says (Core.fillable),
     * or null when it says nothing (locked, or busy). For an app that isn't a
     * browser, the logins of the sites it names and that name it back
     * (AppSites.kt) come after its own, each marked with its site (`from`);
     * `waitMs` is how long to wait for checking those sites (0: only what was
     * checked lately). A page the app shows itself (a WebView: an in-app
     * browser, say) gets a site's logins only if it's that site, so a page
     * opened inside the app can't be handed the logins of the sites that
     * trust the app. Each says whether it has a one-time code to fill
     * (`totp`), which a key whose code needs Premium doesn't.
     */
    fun logins(context: Context, fields: Fields, domain: String?, waitMs: Long): List<JSONObject>? {
        val answer = JSONObject(Core.fillable(fields.packageName, domain ?: "") ?: return null)
        val items = answer.optJSONArray("items") ?: return null
        // By account and id: each account's items are its own.
        val found = LinkedHashMap<String, JSONObject>()
        fun key(item: JSONObject) = item.optString("account") + "/" + item.getString("id")
        for (i in 0 until items.length()) items.getJSONObject(i).let { found[key(it)] = it }
        if (domain == null) {
            val page = Browsers.domain(fields.webDomain)
            for (site in AppSites.linked(context, fields.packageName, waitMs)) {
                if (page != null && page != site && !page.endsWith(".$site")) continue
                val more = runCatching { JSONObject(Core.fillable(fields.packageName, site) ?: "{}") }.getOrNull()
                val siteItems = more?.optJSONArray("items") ?: continue
                for (i in 0 until siteItems.length()) siteItems.getJSONObject(i).let { found.putIfAbsent(key(it), it.put("from", site)) }
            }
        }
        return found.values.toList()
    }

    /**
     * The cards or the identities to offer, every one in use from every
     * unlocked account, by name, the account the screens show first
     * (Core.cardsAndIdentities); null when the core says nothing (locked,
     * or busy).
     */
    fun anywhere(kind: Kind): List<JSONObject>? {
        val answer = JSONObject(Core.cardsAndIdentities() ?: return null)
        if (answer.optBoolean("locked", true)) return null
        val list = answer.optJSONArray(if (kind == Kind.CARD) "cards" else "identities") ?: return null
        return (0 until list.length()).map { list.getJSONObject(it) }
    }

    /** A login just filled: which, where, and when, for the screen after it that asks for its one-time code. */
    data class Filled(val account: String, val item: String, val packageName: String, val webDomain: String?, val at: Long) {
        /**
         * Whether a screen that asks for a code is the one after this fill:
         * in the same app, on the same page's domain if it's a browser's,
         * within a few minutes. Its login's code comes first there.
         */
        fun before(packageName: String, webDomain: String?, now: Long): Boolean =
            packageName == this.packageName && webDomain == this.webDomain && now - at in 0 until CODE_FOLLOWS_MS
    }

    /** How long the login just filled comes first for its code. */
    private const val CODE_FOLLOWS_MS = 5 * 60 * 1000L

    @Volatile private var lastFilled: Filled? = null

    /** A login was just filled here (FillActivity). */
    fun filledLogin(fields: Fields, account: String, item: String) {
        lastFilled = Filled(account, item, fields.packageName, fields.webDomain, System.currentTimeMillis())
    }

    private fun justFilled(item: Offered, fields: Fields): Boolean {
        val last = lastFilled ?: return false
        return last.account == item.account && last.item == item.id &&
            last.before(fields.packageName, fields.webDomain, System.currentTimeMillis())
    }

    /** An item the core offers to fill, as plain values: names and plain details only, never a secret. */
    data class Offered(
        val id: String,
        val account: String,
        val name: String,
        /** A login's username. */
        val username: String? = null,
        /** The site a login in an app is from (AppSites.kt). */
        val from: String? = null,
        /** The account's name, with more than one account on the phone. */
        val accountLabel: String? = null,
        /** A card's or an identity's second line, from the core. */
        val subtitle: String? = null,
        /** It asks for the master password again before it fills. */
        val reprompt: Boolean = false,
        /** A login with a one-time code to fill: a key, where the core says its code shows (core/src/codes.rs). */
        val totp: Boolean = false,
    )

    private fun offered(item: JSONObject) =
        Offered(
            id = item.getString("id"),
            account = item.optString("account"),
            name = item.optString("name"),
            username = text(item, "username"),
            from = text(item, "from"),
            accountLabel = text(item, "accountLabel"),
            subtitle = text(item, "subtitle"),
            reprompt = item.optBoolean("reprompt"),
            totp = item.optBoolean("totp"),
        )

    /** The offer to trust an app as a browser: the page's domain it claims, the app's name, and the account the screens show. */
    data class Trust(val domain: String, val app: String, val account: String)

    /**
     * One entry in Android's list: what it says, the fields it covers, and
     * what picking it opens (FillActivity's `mode`, with the item, its
     * account, and the kind of fill). It holds no value: an entry's fields
     * get only placeholders, and their values come from FillActivity once
     * the person picks it (`filled`).
     */
    data class Entry<T>(
        val title: String,
        val subtitle: String?,
        val ids: List<T>,
        val mode: String,
        val kind: Kind,
        val account: String,
        val item: String?,
        val asks: Boolean,
    )

    /**
     * What Android's list offers on a screen, from what the core offers
     * (plain values, so it's tested on the computer: src/test,
     * ResponsesTest): every login on a sign-in screen (with the code field
     * too, for a login with a key), or each login's code on a screen that
     * asks only for one, the login just filled first; the offer to trust the
     * app as a browser; and every card and identity on their own fields.
     * Each entry is a pick: the same whether the vault was open or was just
     * unlocked from this list (FillActivity hands back this list, never a
     * fill nobody picked).
     */
    fun <T> entries(
        screen: Roles.Screen<T>,
        logins: List<Offered>,
        cards: List<Offered>,
        identities: List<Offered>,
        trust: Trust? = null,
        justFilled: (Offered) -> Boolean = { false },
    ): List<Entry<T>> {
        val found = mutableListOf<Entry<T>>()
        val loginIds = listOfNotNull(screen.username, screen.password)
        val code = screen.code
        if (loginIds.isNotEmpty()) {
            for (login in logins) {
                // A login with a one-time code fills the screen's code field too.
                val ids = loginIds + listOfNotNull(code?.takeIf { login.totp })
                // A site's login in an app says which site it's from, as a browser's address bar would.
                val subtitle = subtitle(login.username, login.from, login.accountLabel)
                found += Entry(login.name, subtitle, ids, FillActivity.FILL, Kind.LOGIN, login.account, login.id, login.reprompt)
            }
        } else if (code != null) {
            // Only a code asked for, as on the screen after signing in: the logins with one, the one just filled first.
            val (first, rest) = logins.filter { it.totp }.partition(justFilled)
            for (login in first + rest) {
                val subtitle = codeSubtitle(login.username, login.from, login.accountLabel)
                found += Entry(login.name, subtitle, listOf(code), FillActivity.FILL, Kind.CODE, login.account, login.id, login.reprompt)
            }
        }
        if (trust != null && (loginIds.isNotEmpty() || code != null)) {
            val ids = loginIds + listOfNotNull(code)
            found += Entry("Logins for ${trust.domain}", "Trust ${trust.app} as a browser first", ids, FillActivity.TRUST, Kind.LOGIN, trust.account, null, false)
        }
        // Cards and identities aren't for a website: every one, on its own kind of field.
        for ((kind, items) in listOf(Kind.CARD to cards, Kind.IDENTITY to identities)) {
            val ids = screen.of(kind).map { it.id }.distinct()
            if (ids.isEmpty()) continue
            for (item in items) {
                found += Entry(item.name, subtitle(item.subtitle, null, item.accountLabel), ids, FillActivity.FILL, kind, item.account, item.id, item.reprompt)
            }
        }
        return found
    }

    /** An entry as Android's dataset: placeholders only, and FillActivity to fill it once it's picked. */
    private fun dataset(context: Context, fields: Fields, entry: Entry<AutofillId>): Dataset.Builder {
        @Suppress("DEPRECATION")
        val dataset = Dataset.Builder(presentation(context, entry.title, entry.subtitle))
        for (field in entry.ids) {
            @Suppress("DEPRECATION")
            dataset.setValue(field, null)
        }
        dataset.setAuthentication(pending(context, fields, entry.mode, entry.account, entry.item, entry.asks, kind = entry.kind))
        return dataset
    }

    /**
     * One entry for the whole response, which opens FillActivity to unlock
     * or sign in first: in Android's list, and on the keyboard when it asks
     * (Inline.kt).
     */
    private fun FillResponse.Builder.authenticate(
        context: Context,
        ids: Array<AutofillId>,
        sender: IntentSender,
        title: String,
        subtitle: String,
        keyboard: Inline.Keyboard?,
    ): FillResponse.Builder {
        val listed = presentation(context, title, subtitle)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R && keyboard?.authenticate(this, ids, sender, listed, title, subtitle) == true) {
            return this
        }
        @Suppress("DEPRECATION")
        return setAuthentication(ids, sender, listed)
    }

    /**
     * The response for these fields, from what the core says; null offers
     * nothing. `waitMs` as for `logins`. `keyboard` is the keyboard's
     * request for suggestions, when it asks (Inline.kt).
     */
    fun forFields(context: Context, fields: Fields, waitMs: Long = 0, keyboard: Inline.Keyboard? = null): FillResponse? {
        val ids = fields.ids
        if (ids.isEmpty()) return null
        // Debug builds only, for the emulator tests' logs: what autofill decided, never a value.
        val debug = (context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0
        if (debug) Log.d("PassBundle", "autofill ${fields.packageName}: fields ${fields.screen.found.joinToString { it.role.name }}")
        Core.openIfClosed(context.filesDir.absolutePath)
        // Only a hostname counts as a page's domain, and only from a browser.
        val claimed = Browsers.domain(fields.webDomain)
        val trusted = claimed != null && Browsers.trusts(context, fields.packageName)
        val domain = claimed.takeIf { trusted }
        val answer = JSONObject(Core.fillable(fields.packageName, domain ?: "") ?: return null)
        val offer = Unlocking.offer(answer.optBoolean("busy"), answer.optBoolean("signedOut"), answer.optBoolean("locked"))
        if (offer == Unlocking.Offer.NOTHING) return null
        if (offer == Unlocking.Offer.SIGN_IN) {
            // No account here: PassBundle opens to sign in; nothing to save to yet.
            return FillResponse.Builder()
                .authenticate(
                    context,
                    ids,
                    pending(context, fields, FillActivity.SIGN_IN, ""),
                    "Sign in to PassBundle",
                    "to fill this in",
                    keyboard,
                )
                .build()
        }
        val loginIds = listOfNotNull(fields.username, fields.password)
        val code = fields.code
        // A login's fields, or a one-time code's, which comes from a login too.
        val forLogins = loginIds.isNotEmpty() || code != null
        if (offer == Unlocking.Offer.UNLOCK) {
            // The app's sites checked meanwhile, for once the person unlocks.
            if (domain == null && forLogins) AppSites.warm(context, fields.packageName)
            // Locked, a card or an address is offered to save whatever it
            // is, and checked once PassBundle is unlocked to save it.
            val save = Saving.offer(fields.screen, null)
            return FillResponse.Builder()
                .authenticate(
                    context,
                    ids,
                    pending(context, fields, FillActivity.UNLOCK, answer.optString("account"), label = text(answer, "accountLabel")),
                    "Unlock PassBundle",
                    "to fill this in",
                    keyboard,
                )
                .apply { save?.let { offerToSave(it) } }
                .build()
        }
        val save = Saving.offer(fields.screen, known(fields))
        var matching = emptyList<Offered>()
        var trust: Trust? = null
        if (forLogins) {
            matching = logins(context, fields, domain, waitMs)?.map(::offered) ?: return null
            if (claimed != null && !trusted && Browsers.couldBe(context, fields.packageName)) {
                trust = Trust(claimed, Browsers.label(context, fields.packageName), answer.optString("account"))
            }
            if (debug) Log.d("PassBundle", "autofill ${fields.packageName}: web domain ${fields.webDomain}, trusted $trusted, offers trust ${trust != null}")
        }
        fun anywhereFor(kind: Kind) = if (fields.of(kind).isEmpty()) emptyList() else anywhere(kind).orEmpty().map(::offered)
        val offers = entries(fields.screen, matching, anywhereFor(Kind.CARD), anywhereFor(Kind.IDENTITY), trust) { justFilled(it, fields) }
        val response = FillResponse.Builder()
        save?.let { response.offerToSave(it) }
        // Nothing to fill: only the offer to save what the person types.
        if (offers.isEmpty()) return save?.let { response.build() }
        val datasets = offers.map { dataset(context, fields, it) }
        // On the keyboard too, as many as it takes, in the list's order.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) keyboard?.suggest(datasets, offers.map { it.title to it.subtitle })
        for (dataset in datasets) response.addDataset(dataset.build())
        return response.build()
    }

    /**
     * An item's second line in Android's list: a login's username (a card's
     * brand, an identity's name), the site it's from (a site's login in an
     * app, as a browser's address bar would say), and with more than one
     * account on the phone, its account's name.
     */
    fun subtitle(username: String?, from: String?, account: String?): String? =
        listOfNotNull(username, from, account).joinToString(" · ").ifEmpty { null }

    /** A login's one-time code's second line: what it is, then as for the login. */
    fun codeSubtitle(username: String?, from: String?, account: String?): String =
        listOfNotNull("One-time code", username, from, account).joinToString(" · ")

    /** What an item's fill rules decide on a page (Core.fillRules): the values they fill, and the fields they keep empty. */
    private class Ruled(val fills: List<FillRules.Fill>, val never: List<Int>)

    /**
     * What the item's fill rules decide here (FillRules.kt): nothing on a
     * screen that isn't a web page in a browser PassBundle trusts, since
     * rules are for websites; null when the core can't say on a page that
     * has them to apply (locked, or busy), or the page didn't come through
     * whole, so nothing fills rather than a field they keep empty.
     */
    private fun ruled(context: Context, fields: Fields, account: String, item: String): Ruled? {
        if (fields.pageLost) return null
        val page = fields.page ?: return Ruled(emptyList(), emptyList())
        val host = Browsers.domain(fields.webDomain)?.takeIf { Browsers.trusts(context, fields.packageName) }
            ?: return Ruled(emptyList(), emptyList())
        return runCatching {
            val answer = JSONObject(Core.fillRules(account, item, FillRules.json(page, host)) ?: "{}")
            val fills = answer.getJSONArray("fills")
            val never = answer.getJSONArray("never")
            Ruled(
                (0 until fills.length()).map { i ->
                    fills.getJSONObject(i).let { FillRules.Fill(it.getInt("field"), it.getString("what"), it.getString("take")) }
                },
                (0 until never.length()).map { never.getInt(it) },
            )
        }.getOrNull()
    }

    /**
     * One item's values for the fields its kind fills, from its account,
     * taken from the core only now that it was picked: a login's username
     * and password (and its current one-time code, when the screen asks for
     * one), a code alone, or a card's or an identity's parts. On a web page,
     * the item's fill rules come first (`ruled`): the fields they keep
     * empty stay so, and Roles.kt finds the fields for the rest without
     * them; what they fill is theirs. Null when there's nothing to fill.
     */
    fun filled(context: Context, fields: Fields, account: String, item: String, kind: Kind): Dataset? {
        fun text(what: String): String? {
            val bytes = Core.value(account, item, what) ?: return null
            return try {
                String(bytes, Charsets.UTF_8)
            } finally {
                bytes.fill(0)
            }
        }
        @Suppress("DEPRECATION")
        val dataset = Dataset.Builder(presentation(context, "PassBundle", null))
        var any = false
        // The fields the rules decided: kept empty, or filled by them.
        val taken = HashSet<AutofillId>()
        fun set(id: AutofillId, value: AutofillValue) {
            @Suppress("DEPRECATION")
            dataset.setValue(id, value)
            any = true
        }
        val ruled = ruled(context, fields, account, item) ?: return null
        val page = fields.page
        // Each value once, however many fields take it.
        val values = HashMap<String, String?>()
        val part = { what: String -> values.getOrPut(what) { text(what) } }
        if (page != null) {
            ruled.never.mapNotNullTo(taken) { page.fields.getOrNull(it)?.let { f -> fields.described.getOrNull(f.at)?.id } }
            for (fill in ruled.fills) {
                val field = page.fields.getOrNull(fill.field) ?: continue
                val id = fields.described.getOrNull(field.at)?.id ?: continue
                if (id in taken) continue
                val value = part(fill.what) ?: continue
                val options = fields.described[field.at].options
                val filled =
                    if (options != null) {
                        FillRules.option(fill.take, options, value)?.let(AutofillValue::forList)
                    } else {
                        FillRules.text(fill.take, value)?.let(AutofillValue::forText)
                    }
                // A value the field doesn't take leaves it to the rest, as the extension does.
                filled?.let {
                    set(id, it)
                    taken += id
                }
            }
        }
        val screen =
            if (page != null && ruled.never.isNotEmpty()) FillRules.left(fields.described, page, ruled.never) else fields.screen
        fun free(id: AutofillId?) = id?.takeIf { it !in taken }
        when (kind) {
            Kind.LOGIN, Kind.CODE -> {
                if (kind == Kind.LOGIN) {
                    free(screen.username)?.let { id -> part("username")?.let { set(id, AutofillValue.forText(it)) } }
                    free(screen.password)?.let { id -> part("password")?.let { set(id, AutofillValue.forText(it)) } }
                }
                free(screen.code)?.let { id -> part("totp")?.let { set(id, AutofillValue.forText(it)) } }
            }
            Kind.CARD, Kind.IDENTITY -> {
                for (found in screen.of(kind)) {
                    if (found.id in taken) continue
                    val options = found.options
                    if (options != null) {
                        Values.option(found.role, options, part)?.let { set(found.id, AutofillValue.forList(it)) }
                    } else {
                        Values.text(found.role, found.format, part)?.let { set(found.id, AutofillValue.forText(it)) }
                    }
                }
            }
        }
        return if (any) dataset.build() else null
    }
}
