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
 * - **Making one:** the same checks, then the core makes the passkey and
 *   saves it in a new login before the site hears of it.
 */
class PasskeyActivity : Activity() {
    companion object {
        const val MODE = "com.passbundle.app.passkey.mode"
        const val ITEM = "com.passbundle.app.passkey.item"
        const val GET = "get"
        const val CREATE = "create"
        const val UNLOCK = "unlock"

        /** What `clientDataJSON` says when a browser gave the hash (Android's guidance: a placeholder). */
        private val PLACEHOLDER = "{}".toByteArray(Charsets.UTF_8)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Core.init(applicationContext)
        Biometric.init(applicationContext)
        Core.openIfClosed(filesDir.absolutePath)
        when (intent.getStringExtra(MODE)) {
            GET -> get(intent.getStringExtra(ITEM))
            CREATE -> create()
            UNLOCK -> unlock()
            else -> finish()
        }
    }

    // --- Unlocking, then the site's passkeys ---

    private fun unlock() {
        val request = PendingIntentHandler.retrieveBeginGetCredentialRequest(intent) ?: return cancel()
        if (Biometric.status(this) != "setUp") {
            // The master password is typed in PassBundle itself.
            startActivity(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            Toast.makeText(this, "Unlock PassBundle, then try again.", Toast.LENGTH_LONG).show()
            return cancel()
        }
        Biometric.unlock(this) { unlocked, error ->
            if (!unlocked) {
                error?.let { Toast.makeText(this, it, Toast.LENGTH_LONG).show() }
                return@unlock cancel()
            }
            background({ PasskeyProvider.entries(this, request) }) { response ->
                val result = Intent()
                PendingIntentHandler.setBeginGetCredentialResponse(result, response ?: BeginGetCredentialResponse())
                setResult(RESULT_OK, result)
                finish()
            }
        }
    }

    // --- Signing in ---

    private fun get(item: String?) {
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
                    (0 until items.length()).map { items.getJSONObject(it) }.firstOrNull { it.optString("id") == item }
                } ?: return@background failGet("PassBundle doesn't have that passkey anymore.")
            prove(item.takeIf { entry.optBoolean("reprompt") }, json.optString("userVerification", "preferred")) { verified ->
                if (verified == null) return@prove failGet(null)
                val clientData = if (caller.browser && option.clientDataHash != null) null else Passkeys.clientData("webauthn.get", challenge, caller)
                val hash = clientData?.let(Passkeys::sha256) ?: option.clientDataHash!!
                background({ JSONObject(Core.passkeyAssert(item, rpId, Passkeys.base64url(hash), verified) ?: "{}") }) { result ->
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
                                    .apply { answer.optString("userHandle").takeIf { it.isNotEmpty() }?.let { put("userHandle", it) } },
                            ).put("clientExtensionResults", JSONObject())
                    val result = Intent()
                    PendingIntentHandler.setGetCredentialResponse(result, GetCredentialResponse(PublicKeyCredential(credential.toString())))
                    setResult(RESULT_OK, result)
                    finish()
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
        setResult(RESULT_OK, result)
        finish()
    }

    // --- Making one ---

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
        val selection = json.optJSONObject("authenticatorSelection")
        val userName = user.optString("name")
        val site = rp.optString("name").ifEmpty { rpId }
        background({ Passkeys.caller(this, request.callingAppInfo, rpId) }) { caller ->
            if (caller == null) return@background failCreate("That site doesn't let this app save its passkeys.")
            val dialog = AlertDialog.Builder(this)
                .setTitle("Save a passkey in PassBundle?")
                // The site's ID, which the passkey is bound to, first: the name is
                // whatever the site says it is.
                .setMessage(
                    "For $rpId${if (site == rpId) "" else " (says it's $site)"}" +
                        "${if (userName.isEmpty()) "" else ", as $userName"}, in a new login.",
                )
                .setPositiveButton("Save") { _, _ ->
                    prove(null, selection?.optString("userVerification", "preferred") ?: "preferred") { verified ->
                        if (verified == null) return@prove failCreate(null)
                        val clientData =
                            if (caller.browser && asked.clientDataHash != null) null else Passkeys.clientData("webauthn.create", json.optString("challenge"), caller)
                        val exclude = json.optJSONArray("excludeCredentials") ?: JSONArray()
                        val made =
                            JSONObject()
                                .put("rpId", rpId)
                                .put("rpName", rp.optString("name").ifEmpty { null })
                                .put("userHandle", user.optString("id"))
                                .put("userName", userName.ifEmpty { null })
                                .put("userDisplayName", user.optString("displayName").ifEmpty { null })
                                .put("userVerified", verified)
                                .put("discoverable", true)
                                .put("exclude", JSONArray((0 until exclude.length()).mapNotNull { exclude.optJSONObject(it)?.optString("id") }))
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
                            setResult(RESULT_OK, result)
                            finish()
                        }
                    }
                }.setNegativeButton("Cancel") { _, _ -> failCreate(null) }
                .setOnCancelListener { failCreate(null) }
                .create()
            guard(dialog)
            dialog.show()
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
        }
    }

    /** The site already has this account's passkey in PassBundle: WebAuthn's InvalidStateError, so it can say so. */
    private fun failCreateExcluded() {
        Toast.makeText(this, "PassBundle already has a passkey for this account on this site.", Toast.LENGTH_LONG).show()
        val result = Intent()
        PendingIntentHandler.setCreateCredentialException(result, CreatePublicKeyCredentialDomException(InvalidStateError()))
        setResult(RESULT_OK, result)
        finish()
    }

    private fun failCreate(words: String?) {
        words?.let { Toast.makeText(this, it, Toast.LENGTH_LONG).show() }
        val result = Intent()
        PendingIntentHandler.setCreateCredentialException(
            result,
            if (words == null) CreateCredentialCancellationException() else CreateCredentialUnknownException(words),
        )
        setResult(RESULT_OK, result)
        finish()
    }

    // --- Proving who they are ---

    /**
     * Has the person prove who they are when it's wanted: the master password
     * for a login that asks for it again (`reprompt`, its id), else Face ID or
     * a fingerprint (or the master password without them) when the site
     * requires it or prefers it. `done(verified)`, null if they cancelled.
     */
    private fun prove(reprompt: String?, wanted: String, done: (Boolean?) -> Unit) {
        // Locked (making one, say): unlocking is the proof, and it's needed anyway.
        if (JSONObject(Core.passkeys("") ?: "{}").optBoolean("locked")) {
            if (Biometric.status(this) != "setUp") {
                startActivity(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                Toast.makeText(this, "Unlock PassBundle, then try again.", Toast.LENGTH_LONG).show()
                return done(null)
            }
            return Biometric.unlock(this) { unlocked, error ->
                error?.let { Toast.makeText(this, it, Toast.LENGTH_LONG).show() }
                done(if (unlocked) true else null)
            }
        }
        if (reprompt != null) return masterPassword("This login asks for your master password.", reprompt, done)
        if (wanted == "discouraged") return done(false)
        if (Biometric.status(this) == "setUp") {
            return Biometric.unlock(this) { unlocked, error ->
                when {
                    unlocked -> done(true)
                    // "Use master password", or a key Android voided.
                    else -> {
                        error?.let { Toast.makeText(this, it, Toast.LENGTH_LONG).show() }
                        masterPassword("Use your master password.", null, done)
                    }
                }
            }
        }
        if (wanted == "required") return masterPassword("This site wants to know it's you.", null, done)
        done(false)
    }

    private fun masterPassword(message: String, item: String?, done: (Boolean?) -> Unit) {
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
                JSONObject((if (item != null) Core.confirmMasterPassword(item, typed) else Core.checkMasterPassword(typed)) ?: "{}")
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
