package com.passbundle.app

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.text.InputType
import android.view.WindowManager
import android.view.inputmethod.EditorInfo
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import androidx.credentials.CreatePublicKeyCredentialRequest
import androidx.credentials.CreatePublicKeyCredentialResponse
import androidx.credentials.GetCredentialResponse
import androidx.credentials.GetPublicKeyCredentialOption
import androidx.credentials.PublicKeyCredential
import androidx.credentials.exceptions.CreateCredentialCancellationException
import androidx.credentials.exceptions.CreateCredentialUnknownException
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialUnknownException
import androidx.credentials.exceptions.domerrors.InvalidStateError
import androidx.credentials.exceptions.publickeycredential.CreatePublicKeyCredentialDomException
import androidx.credentials.provider.BeginGetCredentialResponse
import androidx.credentials.provider.PendingIntentHandler
import org.json.JSONArray
import org.json.JSONObject

/**
 * What Android opens when someone picks one of PassBundle's passkeys, saves
 * a new one in PassBundle, or unlocks it to see them (PasskeyProvider.kt;
 * docs/phones.md, "Passkeys"). It has no screen of its own beyond Android's
 * biometric prompt and a master password dialog.
 *
 * - **Signing in:** checks who's asking (Passkeys.caller), asks the person to
 *   prove who they are when the site wants it or the login asks for the
 *   master password, then has the core sign over the client data hash.
 * - **Making one:** the same checks, then where it goes (PasskeySaving.kt):
 *   in place of this account's old passkey for the site, after asking; in a
 *   login for the site the person picks; or in a new login. The core makes
 *   the passkey and saves it there before the site hears of it.
 */
class PasskeyActivity : Activity() {
    companion object {
        const val MODE = "com.passbundle.app.passkey.mode"
        const val ITEM = "com.passbundle.app.passkey.item"
        /** The passkey's account: its id on this device. */
        const val ACCOUNT = "com.passbundle.app.passkey.account"
        const val GET = "get"
        const val CREATE = "create"
        const val UNLOCK = "unlock"

        /** What `clientDataJSON` says when a browser gave the hash (Android's guidance: a placeholder). */
        private val PLACEHOLDER = "{}".toByteArray(Charsets.UTF_8)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Core.start(applicationContext)
        Biometric.init(applicationContext)
        Core.openIfClosed(filesDir.absolutePath)
        when (intent.getStringExtra(MODE)) {
            GET -> get(intent.getStringExtra(ITEM), intent.getStringExtra(ACCOUNT) ?: "")
            CREATE -> create()
            UNLOCK -> unlock()
            else -> finish()
        }
    }

    // --- Unlocking, then the site's passkeys ---

    /** The account the screens show, as the core says (`Core.passkeys`): its id on this device, and whether it's locked. */
    private fun shown(): Pair<String, Boolean> {
        val answer = runCatching { JSONObject(Core.passkeys("") ?: "{}") }.getOrDefault(JSONObject())
        return answer.optString("account") to answer.optBoolean("locked")
    }

    private fun unlock() {
        val request = PendingIntentHandler.retrieveBeginGetCredentialRequest(intent) ?: return cancel()
        val (account, _) = shown()
        if (Biometric.status(this, account) != "setUp") {
            // The master password is typed in PassBundle itself.
            startActivity(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            Toast.makeText(this, "Unlock PassBundle, then try again.", Toast.LENGTH_LONG).show()
            return cancel()
        }
        Biometric.unlock(this, account) { unlocked, error ->
            if (!unlocked) {
                error?.let { Toast.makeText(this, it, Toast.LENGTH_LONG).show() }
                return@unlock cancel()
            }
            background({ PasskeyProvider.entries(this, request) }) { response ->
                val result = Intent()
                PendingIntentHandler.setBeginGetCredentialResponse(result, response ?: BeginGetCredentialResponse())
                setResult(RESULT_OK, result)
                finish()
                // With "Immediately": open for the pick only, a minute at most,
                // or locked now with nothing to pick (docs/phones.md, "Locking").
                if (response != null) Core.offered() else Core.answered()
            }
        }
    }

    // --- Signing in ---

    private fun get(item: String?, account: String) {
        val request = PendingIntentHandler.retrieveProviderGetCredentialRequest(intent) ?: return failGet(null)
        val option = request.credentialOptions.filterIsInstance<GetPublicKeyCredentialOption>().firstOrNull() ?: return failGet(null)
        val json = runCatching { JSONObject(option.requestJson) }.getOrNull() ?: return failGet("PassBundle couldn't read the site's request.")
        val rpId = json.optString("rpId").ifEmpty { return failGet("PassBundle couldn't read the site's request.") }
        val challenge = json.optString("challenge")
        if (item == null) return failGet(null)
        background({ Passkeys.caller(this, request.callingAppInfo, rpId) }) { caller ->
            if (caller == null) return@background failGet("That site doesn't let this app use its passkeys.")
            val entry =
                JSONObject(Core.passkeys(rpId) ?: "{}").optJSONArray("items")?.let { items ->
                    (0 until items.length()).map { items.getJSONObject(it) }.firstOrNull {
                        it.optString("id") == item && it.optString("account") == account
                    }
                } ?: return@background failGet("PassBundle doesn't have that passkey anymore.")
            prove(account, item.takeIf { entry.optBoolean("reprompt") }, json.optString("userVerification", "preferred")) { verified ->
                if (verified == null) return@prove failGet(null)
                val clientData = if (caller.browser && option.clientDataHash != null) null else Passkeys.clientData("webauthn.get", challenge, caller)
                val hash = clientData?.let(Passkeys::sha256) ?: option.clientDataHash!!
                background({ JSONObject(Core.passkeyAssert(account, item, rpId, Passkeys.base64url(hash), verified) ?: "{}") }) { result ->
                    val answer = result ?: JSONObject()
                    if (!answer.optBoolean("ok")) return@background failGet(answer.optString("error", "PassBundle couldn't sign in with it."))
                    val credentialId = answer.getString("credentialId")
                    val credential =
                        JSONObject()
                            .put("id", credentialId)
                            .put("rawId", credentialId)
                            .put("type", "public-key")
                            .put("authenticatorAttachment", "platform")
                            .put(
                                "response",
                                JSONObject()
                                    .put("clientDataJSON", Passkeys.base64url(clientData ?: PLACEHOLDER))
                                    .put("authenticatorData", answer.getString("authenticatorData"))
                                    .put("signature", answer.getString("signature"))
                                    .apply { Responses.text(answer, "userHandle")?.let { put("userHandle", it) } },
                            ).put("clientExtensionResults", JSONObject())
                    val result = Intent()
                    PendingIntentHandler.setGetCredentialResponse(result, GetCredentialResponse(PublicKeyCredential(credential.toString())))
                    answer(result)
                }
            }
        }
    }

    private fun failGet(words: String?) {
        words?.let { Toast.makeText(this, it, Toast.LENGTH_LONG).show() }
        val result = Intent()
        PendingIntentHandler.setGetCredentialException(
            result,
            if (words == null) GetCredentialCancellationException() else GetCredentialUnknownException(words),
        )
        answer(result)
    }

    // --- Making one ---

    /** A site's request to make a passkey, as read from it, and who's asking. */
    private class Making(
        val caller: Passkeys.Caller,
        val asked: CreatePublicKeyCredentialRequest,
        val json: JSONObject,
        val rpId: String,
        /** The name the site gives itself, or its ID. */
        val site: String,
        val userName: String,
        val rp: JSONObject,
        val user: JSONObject,
    ) {
        /** The account unlocked for this request, before anything was asked: that was the proof. */
        var unlockedNow: String? = null

        val userVerification: String get() = json.optJSONObject("authenticatorSelection")?.optString("userVerification", "preferred") ?: "preferred"

        /**
         * What the core's calls take (Core.passkeyLogins, Core.passkeyCreate):
         * the site's ID, the account's user handle, the passkeys the site has
         * already, and the app asking, which a browser isn't.
         */
        fun request(): JSONObject {
            val exclude = json.optJSONArray("excludeCredentials") ?: JSONArray()
            return JSONObject()
                .put("rpId", rpId)
                .put("userHandle", user.optString("id"))
                .put("exclude", JSONArray((0 until exclude.length()).mapNotNull { exclude.optJSONObject(it)?.optString("id") }))
                .put("app", if (caller.browser) null else caller.packageName)
        }
    }

    private fun create() {
        val request = PendingIntentHandler.retrieveProviderCreateCredentialRequest(intent) ?: return failCreate(null)
        val asked = request.callingRequest as? CreatePublicKeyCredentialRequest ?: return failCreate(null)
        val json = runCatching { JSONObject(asked.requestJson) }.getOrNull() ?: return failCreate("PassBundle couldn't read the site's request.")
        val rp = json.optJSONObject("rp") ?: return failCreate("PassBundle couldn't read the site's request.")
        val user = json.optJSONObject("user") ?: return failCreate("PassBundle couldn't read the site's request.")
        val rpId = rp.optString("id").ifEmpty { return failCreate("PassBundle couldn't read the site's request.") }
        val algorithms = json.optJSONArray("pubKeyCredParams") ?: JSONArray()
        // None listed means ES256 and RS256 (WebAuthn's default).
        if (algorithms.length() > 0 && (0 until algorithms.length()).none { algorithms.optJSONObject(it)?.optInt("alg") == -7 }) {
            return failCreate("That site doesn't take the kind of passkey PassBundle makes.")
        }
        val userName = Responses.text(user, "name") ?: ""
        val site = Responses.text(rp, "name") ?: rpId
        background({ Passkeys.caller(this, request.callingAppInfo, rpId) }) { caller ->
            if (caller == null) return@background failCreate("That site doesn't let this app save its passkeys.")
            val making = Making(caller, asked, json, rpId, site, userName, rp, user)
            // Which logins it can go in is in the vault: with none unlocked,
            // the account the screens show unlocks first.
            unlockFirst { unlocked ->
                if (unlocked == null) return@unlockFirst failCreate(null)
                making.unlockedNow = unlocked.ifEmpty { null }
                where(making)
            }
        }
    }

    /**
     * With no account unlocked, the one the screens show unlocks first, with
     * its fingerprint or face: that's the proof the site may ask for, so it
     * isn't asked again. `done(account)` with the account unlocked now, ""
     * when one was unlocked already, or null if it can't be or the person
     * cancels (without biometric unlock, PassBundle opens to unlock there).
     */
    private fun unlockFirst(done: (String?) -> Unit) {
        val (account, locked) = shown()
        if (!locked) return done("")
        if (Biometric.status(this, account) != "setUp") {
            startActivity(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            Toast.makeText(this, "Unlock PassBundle, then try again.", Toast.LENGTH_LONG).show()
            return done(null)
        }
        Biometric.unlock(this, account) { unlocked, error ->
            error?.let { Toast.makeText(this, it, Toast.LENGTH_LONG).show() }
            done(if (unlocked) account else null)
        }
    }

    /**
     * Asks where the passkey goes, from the logins the core says it can go
     * in and the accounts a new login can go in (PasskeySaving.step): a new
     * login is offered only where the core says one can be made, and where
     * none can, the core's words say why before anything is picked.
     */
    private fun where(m: Making) {
        background({ JSONObject(Core.passkeyLogins(m.request().toString()) ?: "{}") }) { result ->
            val answer = result ?: JSONObject()
            if (answer.optBoolean("locked")) return@background failCreate("Unlock PassBundle, then try again.")
            if (!answer.optBoolean("ok")) return@background failCreate(Responses.text(answer, "error") ?: "PassBundle couldn't save the passkey.")
            val accounts = accounts(answer)
            val shown = answer.optString("account")
            when (val step = PasskeySaving.step(answer.optBoolean("excluded"), logins(answer), PasskeySaving.noNewLogin(accounts))) {
                PasskeySaving.Step.Excluded -> failCreateExcluded()
                is PasskeySaving.Step.Replace -> confirmReplace(m, step.login, back = null)
                is PasskeySaving.Step.Choose -> chooseLogin(m, step, accounts, shown)
                PasskeySaving.Step.New ->
                    chooseAccount(accounts, shown) { account, label ->
                        if (account == null) return@chooseAccount failCreate(null)
                        confirmCreate(m, account, label)
                    }
                is PasskeySaving.Step.Nowhere -> nowhere(step.why)
            }
        }
    }

    /** The accounts unlocked here (Core.passkeyLogins), the one the screens show first, each saying whether a new login can go in it. */
    private fun accounts(answer: JSONObject): List<PasskeySaving.Account> {
        val unlocked = answer.optJSONArray("accounts") ?: return emptyList()
        return (0 until unlocked.length()).mapNotNull { at ->
            val account = unlocked.optJSONObject(at) ?: return@mapNotNull null
            PasskeySaving.Account(
                id = Responses.text(account, "id") ?: return@mapNotNull null,
                label = account.optString("label"),
                newLogin = account.optBoolean("newLogin", true),
                noNewLogin = Responses.text(account, "noNewLogin"),
            )
        }
    }

    /** The logins the core listed (Core.passkeyLogins), by their parts. */
    private fun logins(answer: JSONObject): List<PasskeySaving.Login> {
        val items = answer.optJSONArray("items") ?: return emptyList()
        return (0 until items.length()).mapNotNull { at ->
            val item = items.optJSONObject(at) ?: return@mapNotNull null
            PasskeySaving.Login(
                id = Responses.text(item, "id") ?: return@mapNotNull null,
                account = Responses.text(item, "account") ?: return@mapNotNull null,
                name = Responses.text(item, "name") ?: "",
                username = Responses.text(item, "userName"),
                accountLabel = Responses.text(item, "accountLabel"),
                replaces = item.optBoolean("replaces"),
                passkeyUserName = Responses.text(item, "passkeyUserName"),
                reprompt = item.optBoolean("reprompt"),
            )
        }
    }

    /**
     * The account a new passkey's new login goes in, of those unlocked
     * that take one (PasskeySaving.newLoginIn): with more than one, a
     * choice of them (`done(null, null)` if the person cancels); else that
     * one, named when another unlocked here couldn't take it, or the one
     * the screens show ([shown]).
     */
    private fun chooseAccount(accounts: List<PasskeySaving.Account>, shown: String, done: (String?, String?) -> Unit) {
        val (choices, named) = PasskeySaving.newLoginIn(accounts)
        if (choices.size < 2) {
            return done(choices.firstOrNull()?.id ?: shown, named)
        }
        val labels = choices.map { it.label }.toTypedArray()
        val dialog =
            AlertDialog.Builder(this)
                .setTitle("Save the passkey in which account?")
                .setItems(labels) { _, which -> done(choices[which].id, labels[which]) }
                .setNegativeButton("Cancel") { _, _ -> done(null, null) }
                .setOnCancelListener { done(null, null) }
                .create()
        dialog.setOnShowListener { dialog.window?.decorView?.filterTouchesWhenObscured = true }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) window.setHideOverlayWindows(true)
        dialog.show()
    }

    /** "Save a passkey in PassBundle?" for the site, in a new login in [account] ([label] names it, with more than one), then making it. */
    private fun confirmCreate(m: Making, account: String, label: String?) {
        val dialog =
            AlertDialog.Builder(this)
                .setTitle("Save a passkey in PassBundle?")
                // The site's ID, which the passkey is bound to, first: the name is
                // whatever the site says it is.
                .setMessage(PasskeySaving.newLogin(m.rpId, m.site, m.userName, label))
                .setPositiveButton("Save") { _, _ -> save(m, account, null) }
                .setNegativeButton("Cancel") { _, _ -> failCreate(null) }
                .setOnCancelListener { failCreate(null) }
                .create()
        guard(dialog)
        dialog.show()
    }

    /**
     * "Save a passkey in PassBundle?" with the logins it can go in, each in
     * its own account, then "New login": picking a login adds the passkey to
     * it, keeping its password; one with this account's passkey for the site
     * asks to replace it first. Where no account here takes a new login,
     * there's no "New login", and the question says why.
     */
    private fun chooseLogin(m: Making, step: PasskeySaving.Step.Choose, accounts: List<PasskeySaving.Account>, shown: String) {
        val logins = step.logins
        val builder = AlertDialog.Builder(this)
        val dialog =
            builder
                .setCustomTitle(heading(builder, "Save a passkey in PassBundle?", PasskeySaving.choose(m.rpId, m.site, m.userName, step.noNewLogin)))
                .setItems(PasskeySaving.rows(logins, newLogin = step.noNewLogin == null).toTypedArray<CharSequence>()) { _, which ->
                    val login = logins.getOrNull(which)
                    when {
                        login == null -> newLogin(m, accounts, shown)
                        login.replaces -> confirmReplace(m, login) { chooseLogin(m, step, accounts, shown) }
                        else -> save(m, login.account, login)
                    }
                }.setNegativeButton("Cancel") { _, _ -> failCreate(null) }
                .setOnCancelListener { failCreate(null) }
                .create()
        guard(dialog)
        dialog.show()
    }

    /** "New login" from the list: in the account the person picks, of those unlocked that take one. */
    private fun newLogin(m: Making, accounts: List<PasskeySaving.Account>, shown: String) {
        chooseAccount(accounts, shown) { account, _ ->
            if (account == null) return@chooseAccount failCreate(null)
            save(m, account, null)
        }
    }

    /**
     * No login for the site takes the passkey, and no account unlocked here
     * takes a new login (an organization keeps the person's items out of
     * Personal and they have no shared folder to add to): the core's words
     * say why, before anything is picked, and the site hears it isn't saved.
     * The app that asked hears only that: the core's words name the
     * person's organization, which is theirs to see, not the asking app's.
     */
    private fun nowhere(why: String) {
        val notSaved = "PassBundle couldn't save the passkey."
        val dialog =
            AlertDialog.Builder(this)
                .setTitle("PassBundle can't save this passkey")
                .setMessage(why)
                .setPositiveButton("OK") { _, _ -> failCreate(notSaved, toast = false) }
                .setOnCancelListener { failCreate(notSaved, toast = false) }
                .create()
        guard(dialog)
        dialog.show()
    }

    /**
     * "Replace the passkey for <user> on <site>?": the login has this
     * account's passkey for the site, and the new one takes its place, so
     * the old one stops working at the site's next sign-in. [back] goes back
     * to the list it was picked from; without one, it's Cancel.
     */
    private fun confirmReplace(m: Making, login: PasskeySaving.Login, back: (() -> Unit)?) {
        val builder = AlertDialog.Builder(this)
        builder
            .setCustomTitle(heading(builder, PasskeySaving.replaceTitle(m.userName, login, m.rpId), null))
            .setMessage(PasskeySaving.replaceMessage(login, m.rpId))
            .setPositiveButton("Replace") { _, _ -> save(m, login.account, login) }
            .setOnCancelListener { failCreate(null) }
        if (back != null) {
            builder.setNegativeButton("Back") { _, _ -> back() }
        } else {
            builder.setNegativeButton("Cancel") { _, _ -> failCreate(null) }
        }
        val dialog = builder.create()
        guard(dialog)
        dialog.show()
    }

    /**
     * A dialog's title over more than one line (a site's ID and an account's
     * name can be long), with what it asks under it when there's a list
     * below, in the dialog's own look.
     */
    private fun heading(builder: AlertDialog.Builder, title: String, message: String?): LinearLayout {
        val context = builder.context
        val dp = resources.displayMetrics.density
        return LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            setPadding((24 * dp).toInt(), (20 * dp).toInt(), (24 * dp).toInt(), (8 * dp).toInt())
            addView(
                TextView(context).apply {
                    setTextAppearance(android.R.style.TextAppearance_Material_DialogWindowTitle)
                    text = title
                },
            )
            if (message != null) {
                addView(
                    TextView(context).apply {
                        setTextAppearance(android.R.style.TextAppearance_Material_Subhead)
                        setPadding(0, (12 * dp).toInt(), 0, 0)
                        text = message
                    },
                )
            }
        }
    }

    /**
     * Has the person prove who they are as the site wants (or as the login
     * asks, with its master password), then has the core make the passkey
     * in [login] (its account's), or in a new login in [account]. Unlocked
     * just now for this request, that was the proof.
     */
    private fun save(m: Making, account: String, login: PasskeySaving.Login?) {
        val asks = login?.reprompt == true
        if (m.unlockedNow == account && !asks) return make(m, account, login?.id, true)
        prove(account, if (asks) login?.id else null, m.userVerification) { verified ->
            if (verified == null) return@prove failCreate(null)
            make(m, account, login?.id, verified)
        }
    }

    /** The core makes the passkey, saved in [login] or a new login, then the site gets it. */
    private fun make(m: Making, account: String, login: String?, verified: Boolean) {
        val clientData =
            if (m.caller.browser && m.asked.clientDataHash != null) null else Passkeys.clientData("webauthn.create", m.json.optString("challenge"), m.caller)
        val made =
            m.request()
                .put("account", account)
                .put("login", login)
                .put("rpName", Responses.text(m.rp, "name"))
                .put("userName", m.userName.ifEmpty { null })
                .put("userDisplayName", Responses.text(m.user, "displayName"))
                .put("userVerified", verified)
                .put("discoverable", true)
        background({ JSONObject(Core.passkeyCreate(made.toString()) ?: "{}") }) { result ->
            val answer = result ?: JSONObject()
            if (answer.optBoolean("excluded")) return@background failCreateExcluded()
            if (!answer.optBoolean("ok")) return@background failCreate(answer.optString("error", "PassBundle couldn't save the passkey."))
            val credentialId = answer.getString("credentialId")
            val credential =
                JSONObject()
                    .put("id", credentialId)
                    .put("rawId", credentialId)
                    .put("type", "public-key")
                    .put("authenticatorAttachment", "platform")
                    .put(
                        "response",
                        JSONObject()
                            .put("clientDataJSON", Passkeys.base64url(clientData ?: PLACEHOLDER))
                            .put("attestationObject", answer.getString("attestationObject"))
                            .put("authenticatorData", answer.getString("authenticatorData"))
                            .put("publicKey", answer.getString("publicKey"))
                            .put("publicKeyAlgorithm", -7)
                            .put("transports", JSONArray(listOf("internal", "hybrid"))),
                    ).put("clientExtensionResults", JSONObject().put("credProps", JSONObject().put("rk", true)))
            val result = Intent()
            PendingIntentHandler.setCreateCredentialResponse(result, CreatePublicKeyCredentialResponse(credential.toString()))
            answer(result)
        }
    }

    /**
     * A dialog that asks for something, made hard to trick: touches through
     * anything drawn over it don't count, overlays are hidden (Android 12 and
     * later), and its first button waits a moment, so a tap meant for the
     * other app doesn't land on it.
     */
    private fun guard(dialog: AlertDialog) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) window.setHideOverlayWindows(true)
        dialog.setOnShowListener {
            dialog.window?.decorView?.filterTouchesWhenObscured = true
            for (which in listOf(AlertDialog.BUTTON_POSITIVE, AlertDialog.BUTTON_NEGATIVE)) {
                dialog.getButton(which)?.filterTouchesWhenObscured = true
            }
            val yes = dialog.getButton(AlertDialog.BUTTON_POSITIVE)
            yes?.isEnabled = false
            yes?.postDelayed({ yes.isEnabled = true }, 1000)
            // A list to pick from (the logins a passkey can go in) waits too.
            val list = dialog.listView
            list?.isEnabled = false
            list?.postDelayed({ list.isEnabled = true }, 1000)
        }
    }

    /** The site already has this account's passkey in PassBundle: WebAuthn's InvalidStateError, so it can say so. */
    private fun failCreateExcluded() {
        Toast.makeText(this, "PassBundle already has a passkey for this account on this site.", Toast.LENGTH_LONG).show()
        val result = Intent()
        PendingIntentHandler.setCreateCredentialException(result, CreatePublicKeyCredentialDomException(InvalidStateError()))
        answer(result)
    }

    /** The site hears no passkey was saved: [words] why, shown here too unless a dialog said them already ([toast]). */
    private fun failCreate(words: String?, toast: Boolean = true) {
        if (toast) words?.let { Toast.makeText(this, it, Toast.LENGTH_LONG).show() }
        val result = Intent()
        PendingIntentHandler.setCreateCredentialException(
            result,
            if (words == null) CreateCredentialCancellationException() else CreateCredentialUnknownException(words),
        )
        answer(result)
    }

    /**
     * Hands a passkey, or why there's none, back to Android: the site's
     * request is over, so with "Immediately" chosen, or the policy's, the
     * vault locks now (docs/phones.md, "Locking").
     */
    private fun answer(result: Intent) {
        setResult(RESULT_OK, result)
        finish()
        Core.answered()
    }

    // --- Proving who they are ---

    /**
     * Has the person prove who they are when it's wanted, for the passkey's
     * account: its master password for a login that asks for it again
     * (`reprompt`, its id), else Face ID or a fingerprint (or the master
     * password without them) when the site requires it or prefers it.
     * `done(verified)`, null if they cancelled.
     */
    private fun prove(account: String, reprompt: String?, wanted: String, done: (Boolean?) -> Unit) {
        // Locked (making one, say): unlocking is the proof, and it's needed anyway.
        val (shownAccount, locked) = shown()
        if (locked && shownAccount == account) {
            if (Biometric.status(this, account) != "setUp") {
                startActivity(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                Toast.makeText(this, "Unlock PassBundle, then try again.", Toast.LENGTH_LONG).show()
                return done(null)
            }
            return Biometric.unlock(this, account) { unlocked, error ->
                error?.let { Toast.makeText(this, it, Toast.LENGTH_LONG).show() }
                done(if (unlocked) true else null)
            }
        }
        if (reprompt != null) return masterPassword("This login asks for your master password.", account, reprompt, done)
        if (wanted == "discouraged") return done(false)
        if (Biometric.status(this, account) == "setUp") {
            return Biometric.unlock(this, account) { unlocked, error ->
                when {
                    unlocked -> done(true)
                    // "Use master password", or a key Android voided.
                    else -> {
                        error?.let { Toast.makeText(this, it, Toast.LENGTH_LONG).show() }
                        masterPassword("Use your master password.", account, null, done)
                    }
                }
            }
        }
        if (wanted == "required") return masterPassword("This site wants to know it's you.", account, null, done)
        done(false)
    }

    private fun masterPassword(message: String, account: String, item: String?, done: (Boolean?) -> Unit) {
        val password =
            EditText(this).apply {
                inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
                imeOptions = EditorInfo.IME_ACTION_GO
                hint = "Master password"
            }
        fun check() {
            val typed = password.text.toString()
            password.text.clear()
            background({
                JSONObject(
                    (if (item != null) Core.confirmMasterPassword(account, item, typed) else Core.checkMasterPassword(account, typed)) ?: "{}",
                )
            }) { result ->
                val answer = result ?: JSONObject()
                if (answer.optBoolean("ok")) return@background done(true)
                Toast.makeText(this, answer.optString("error", "That master password isn't right."), Toast.LENGTH_LONG).show()
                done(null)
            }
        }
        val dialog =
            AlertDialog.Builder(this)
                .setTitle("PassBundle")
                .setMessage(message)
                .setView(password)
                .setPositiveButton("Continue") { _, _ -> check() }
                .setNegativeButton("Cancel") { _, _ -> done(null) }
                .setOnCancelListener { done(null) }
                .create()
        password.setOnEditorActionListener { _, action, _ ->
            if (action != EditorInfo.IME_ACTION_GO) return@setOnEditorActionListener false
            dialog.dismiss()
            check()
            true
        }
        dialog.window?.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_VISIBLE)
        guard(dialog)
        dialog.show()
        password.requestFocus()
    }

    private fun cancel() {
        setResult(RESULT_CANCELED)
        finish()
    }

    /** Runs slow work (the network, the KDF) off the main thread, then `then` on it, unless the person left. */
    /** The core at work (checking, saving, signing): Back waits, so a site never misses a passkey the vault kept. */
    private var working = 0

    @Deprecated("Back while the core works is ignored; Activity has no other hook before Android 13.")
    override fun onBackPressed() {
        if (working == 0) {
            @Suppress("DEPRECATION")
            super.onBackPressed()
        }
    }

    private fun <T> background(work: () -> T, then: (T?) -> Unit) {
        working++
        Thread {
            // Anything going wrong ends as nothing, which every caller refuses.
            val result = runCatching(work).getOrNull()
            runOnUiThread {
                working--
                if (!isFinishing && !isDestroyed) then(result)
            }
        }.start()
    }
}
