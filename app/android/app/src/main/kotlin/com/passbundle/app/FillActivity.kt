package com.passbundle.app

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.os.Parcelable
import android.text.InputType
import android.view.KeyEvent
import android.view.WindowManager
import android.view.autofill.AutofillManager
import android.view.inputmethod.EditorInfo
import android.widget.EditText
import android.widget.Toast
import org.json.JSONObject

/**
 * What Android's autofill opens when someone picks one of PassBundle's
 * entries (Responses.kt): fills a login, taking its password from the core
 * only now, or unlocks first. It has no screen of its own; unlocking shows
 * Android's biometric prompt, or asks for the account's master password in
 * PassBundle's own dialog, here, so the fill isn't lost; either way it hands
 * back the logins to pick from, never a fill nobody picked (docs/phones.md,
 * "Autofill"). A login that asks for the master password again asks for it
 * in a dialog here first (docs/core.md, "Asking for the master password
 * again"). An app that isn't a browser PassBundle knows asks here, in
 * PassBundle's own dialog, before its web domain counts (Browsers.kt).
 * Signed out, it opens PassBundle to sign in.
 */
class FillActivity : Activity() {
    companion object {
        const val MODE = "com.passbundle.app.mode"
        /** The account to unlock, or the login's account: its id on this device. */
        const val ACCOUNT = "com.passbundle.app.account"
        const val ITEM = "com.passbundle.app.item"
        const val ASKS = "com.passbundle.app.asks"
        /** The account to unlock's name, when there's more than one on the phone. */
        const val LABEL = "com.passbundle.app.label"
        const val FILL = "fill"
        const val UNLOCK = "unlock"
        const val TRUST = "trust"
        const val SIGN_IN = "signIn"

        /** How long the fill's own check waits for checking an app's sites (AppSites.kt). */
        private const val SITES_WAIT_MS = 2000L
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Core.start(applicationContext)
        Biometric.init(applicationContext)
        Core.openIfClosed(filesDir.absolutePath)
        val fields = Fields.from(intent)
        val account = intent.getStringExtra(ACCOUNT) ?: ""
        when (intent.getStringExtra(MODE)) {
            FILL -> fill(fields, account, intent.getStringExtra(ITEM), intent.getBooleanExtra(ASKS, false))
            UNLOCK -> unlock(fields, account, intent.getStringExtra(LABEL))
            TRUST -> trust(fields)
            SIGN_IN -> signIn()
            else -> finishWith(null)
        }
    }

    /**
     * Whether the login still fills here, now: the app (and its page, if it's
     * a browser trusted now, or a site that names it) still matches it.
     * Android hands the app being filled the request to open this, which it
     * could keep and send again later, so nothing it was once offered fills
     * unless it still would be, from the same account. Checking a site may
     * fetch: off the main thread.
     */
    private fun stillMatches(fields: Fields, account: String, item: String): Boolean {
        val domain = Browsers.domain(fields.webDomain)?.takeIf { Browsers.trusts(this, fields.packageName) }
        val logins = runCatching { Responses.logins(this, fields, domain, SITES_WAIT_MS) }.getOrNull() ?: return false
        return logins.any { it.optString("id") == item && it.optString("account") == account }
    }

    private fun fill(fields: Fields, account: String, item: String?, asks: Boolean) {
        if (item == null) return finishWith(null)
        Thread {
            val matches = runCatching { stillMatches(fields, account, item) }.getOrDefault(false)
            runOnUiThread {
                when {
                    isFinishing || isDestroyed -> Unit
                    !matches -> {
                        Toast.makeText(this, "That login doesn't fill here.", Toast.LENGTH_LONG).show()
                        finishWith(null)
                    }
                    !asks -> finishWith(Responses.filled(this, fields, account, item))
                    else -> askMasterPassword(fields, account, item)
                }
            }
        }.start()
    }

    /**
     * A master password field for PassBundle's own dialogs here: hidden as
     * typed, and kept out of the keyboard's suggestions and learning.
     */
    private fun masterPasswordField() =
        EditText(this).apply {
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
            imeOptions = EditorInfo.IME_ACTION_GO or EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING
            hint = "Master password"
        }

    /**
     * Over another app, a master password goes only into PassBundle's own
     * window: out of screenshots and recordings, with nothing drawn over it
     * (Android 12 and later), and no touch counted through anything that is.
     */
    private fun keepPrivate(dialog: AlertDialog, onShow: () -> Unit = {}) {
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) window.setHideOverlayWindows(true)
        dialog.window?.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        dialog.setOnShowListener {
            dialog.window?.decorView?.filterTouchesWhenObscured = true
            dialog.getButton(AlertDialog.BUTTON_POSITIVE)?.filterTouchesWhenObscured = true
            dialog.getButton(AlertDialog.BUTTON_NEGATIVE)?.filterTouchesWhenObscured = true
            onShow()
        }
    }

    /** A login that asks for the master password again: its dialog, then it fills. */
    private fun askMasterPassword(fields: Fields, account: String, item: String) {
        val password = masterPasswordField()
        val dialog = AlertDialog.Builder(this)
            .setTitle("PassBundle")
            .setMessage("This login asks for your master password.")
            .setView(password)
            .setPositiveButton("Continue") { _, _ -> confirm(fields, account, item, password) }
            .setNegativeButton("Cancel") { _, _ -> finishWith(null) }
            .setOnCancelListener { finishWith(null) }
            .create()
        password.setOnEditorActionListener { _, action, _ ->
            if (action != EditorInfo.IME_ACTION_GO) return@setOnEditorActionListener false
            dialog.dismiss()
            confirm(fields, account, item, password)
            true
        }
        keepPrivate(dialog)
        dialog.window?.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_VISIBLE)
        dialog.show()
        password.requestFocus()
    }

    /** Checks the account's master password off the main thread (the KDF), then fills, unless the person left. */
    private fun confirm(fields: Fields, account: String, item: String, password: EditText) {
        val typed = password.text.toString()
        password.text.clear()
        Thread {
            val ok = try {
                val answer = JSONObject(Core.confirmMasterPassword(account, item, typed) ?: "{}")
                if (!answer.optBoolean("ok")) {
                    val words = answer.optString("error", "Couldn't check that.")
                    runOnUiThread { Toast.makeText(this, words, Toast.LENGTH_LONG).show() }
                }
                answer.optBoolean("ok")
            } catch (_: RuntimeException) {
                false
            }
            runOnUiThread {
                when {
                    isFinishing || isDestroyed -> Unit
                    ok -> finishWith(Responses.filled(this, fields, account, item))
                    else -> finishWith(null)
                }
            }
        }.start()
    }

    /**
     * Trusts the app as a browser, if the person says so; then its page's
     * logins. The app's name and the page's domain are made safe to show
     * (Browsers.label, Browsers.domain), the package name is always there,
     * Trust waits a moment before it can be tapped (so a tap meant for the
     * app's own screen doesn't land on it), and touches through anything
     * drawn over this dialog don't count.
     */
    private fun trust(fields: Fields) {
        val domain = Browsers.domain(fields.webDomain) ?: return finishWith(null)
        if (!Browsers.couldBe(this, fields.packageName)) return finishWith(null)
        val app = Browsers.label(this, fields.packageName)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) window.setHideOverlayWindows(true)
        val dialog = AlertDialog.Builder(this)
            .setTitle("Trust $app as a browser?")
            .setMessage(
                "App: ${fields.packageName}\nSays this page is: $domain\n\n" +
                    "PassBundle fills a website's logins only in browsers, because any other app can say " +
                    "it's any website. Trust it only if it's a web browser you use.",
            )
            .setPositiveButton("Trust") { _, _ ->
                if (Browsers.trust(this, fields.packageName)) {
                    finishWith(Responses.forFields(this, fields))
                } else {
                    Toast.makeText(this, "Couldn't trust $app.", Toast.LENGTH_LONG).show()
                    finishWith(null)
                }
            }
            .setNegativeButton("Cancel") { _, _ -> finishWith(null) }
            .setOnCancelListener { finishWith(null) }
            .create()
        dialog.setOnShowListener {
            dialog.window?.decorView?.filterTouchesWhenObscured = true
            val trust = dialog.getButton(AlertDialog.BUTTON_POSITIVE)
            trust.filterTouchesWhenObscured = true
            dialog.getButton(AlertDialog.BUTTON_NEGATIVE).filterTouchesWhenObscured = true
            trust.isEnabled = false
            trust.postDelayed({ trust.isEnabled = true }, 1000)
        }
        dialog.show()
    }

    /**
     * Unlocks the account the screens show (the one autofill offered to
     * unlock), then hands back its logins to pick from: with its biometric
     * unlock when it's on, else (or when that doesn't unlock, or the person
     * would rather) with its master password, here.
     */
    private fun unlock(fields: Fields, account: String, label: String?) {
        if (account.isEmpty()) return finishWith(null)
        if (Unlocking.way(Biometric.status(this, account)) == Unlocking.Way.MASTER_PASSWORD) {
            return unlockWithMasterPassword(fields, account, label, null)
        }
        Biometric.unlock(this, account) { unlocked, error ->
            if (isFinishing || isDestroyed) return@unlock
            if (unlocked) return@unlock finishWith(Responses.forFields(this, fields))
            // "Use master password", or the biometric unlock is off now.
            unlockWithMasterPassword(fields, account, label, error)
        }
    }

    /**
     * The account's master password, in PassBundle's own dialog over the app
     * being filled (`keepPrivate`). The core unlocks that account with it,
     * off the main thread (the KDF), as the lock screen does; then the
     * logins to pick from. A wrong one says so and asks again; Cancel fills
     * nothing.
     */
    private fun unlockWithMasterPassword(fields: Fields, account: String, label: String?, why: String?) {
        val password = masterPasswordField()
        val dialog = AlertDialog.Builder(this)
            .setTitle("Unlock PassBundle")
            .setMessage(Unlocking.message(label, why))
            .setView(password)
            // Its listener is set once it shows, so a wrong password keeps it open.
            .setPositiveButton("Unlock", null)
            .setNegativeButton("Cancel") { _, _ -> finishWith(null) }
            .setOnCancelListener { finishWith(null) }
            .create()
        var unlocking = false
        fun submit() {
            if (unlocking) return
            val typed = password.text.toString()
            password.text.clear()
            if (typed.isEmpty()) return
            unlocking = true
            val button = dialog.getButton(AlertDialog.BUTTON_POSITIVE)
            button?.isEnabled = false
            password.isEnabled = false
            Thread {
                val answer = runCatching { JSONObject(Core.unlockWithMasterPassword(account, typed) ?: "{}") }.getOrNull()
                runOnUiThread {
                    if (isFinishing || isDestroyed) return@runOnUiThread
                    if (answer?.optBoolean("ok") == true) {
                        dialog.dismiss()
                        return@runOnUiThread finishWith(Responses.forFields(this, fields))
                    }
                    // What went wrong, said in the dialog itself, and asked again.
                    unlocking = false
                    button?.isEnabled = true
                    password.isEnabled = true
                    val wrong = answer?.optString("error")?.takeIf { it.isNotEmpty() } ?: "Couldn't unlock PassBundle. Try again."
                    dialog.setMessage(Unlocking.message(label, wrong))
                    password.requestFocus()
                }
            }.start()
        }
        // The keyboard's Go, or Enter on a keyboard of keys.
        password.setOnEditorActionListener { _, action, event ->
            val enter = event?.keyCode == KeyEvent.KEYCODE_ENTER
            if (action != EditorInfo.IME_ACTION_GO && !enter) return@setOnEditorActionListener false
            if (!enter || event?.action == KeyEvent.ACTION_DOWN) submit()
            true
        }
        keepPrivate(dialog) {
            dialog.getButton(AlertDialog.BUTTON_POSITIVE)?.setOnClickListener { submit() }
        }
        dialog.window?.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_VISIBLE)
        dialog.show()
        password.requestFocus()
    }

    /** Signed out: there's nothing to fill from until the person signs in, in PassBundle itself. */
    private fun signIn() {
        startActivity(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        Toast.makeText(this, "Sign in to PassBundle, then fill this in.", Toast.LENGTH_LONG).show()
        finishWith(null)
    }

    private fun finishWith(result: Parcelable?) {
        if (result == null) {
            setResult(RESULT_CANCELED)
        } else {
            setResult(RESULT_OK, Intent().putExtra(AutofillManager.EXTRA_AUTHENTICATION_RESULT, result))
        }
        finish()
        // With "Immediately" (docs/phones.md, "Locking"): a login's values
        // handed back lock the vault now, and so does an unlock with nothing
        // to pick from; a list to pick from (after unlocking, or trusting a
        // browser) keeps it open for the pick, a minute at most. A fill that
        // didn't happen leaves that wait as it is, for another pick.
        when (intent.getStringExtra(MODE)) {
            FILL -> if (result != null) Core.answered()
            UNLOCK -> if (result != null) Core.offered() else Core.answered()
            TRUST -> if (result != null) Core.offered()
            SIGN_IN -> Unit
        }
    }
}
