package com.passbundle.app

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.os.Parcelable
import android.text.InputType
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
 * Android's biometric prompt, or opens PassBundle to unlock with the master
 * password. A login that asks for the master password again asks for it in
 * a dialog here first (docs/core.md, "Asking for the master password again").
 * An app that isn't a browser PassBundle knows asks here, in PassBundle's own
 * dialog, before its web domain counts (Browsers.kt).
 */
class FillActivity : Activity() {
    companion object {
        const val MODE = "com.passbundle.app.mode"
        /** The account to unlock, or the login's account: its id on this device. */
        const val ACCOUNT = "com.passbundle.app.account"
        const val ITEM = "com.passbundle.app.item"
        const val ASKS = "com.passbundle.app.asks"
        const val FILL = "fill"
        const val UNLOCK = "unlock"
        const val TRUST = "trust"

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
            UNLOCK -> unlock(fields, account)
            TRUST -> trust(fields)
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

    /** A login that asks for the master password again: its dialog, then it fills. */
    private fun askMasterPassword(fields: Fields, account: String, item: String) {
        val password = EditText(this).apply {
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
            imeOptions = EditorInfo.IME_ACTION_GO
            hint = "Master password"
        }
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

    /** Unlocks the account the screens show (the one autofill offered to unlock), then its logins. */
    private fun unlock(fields: Fields, account: String) {
        if (Biometric.status(this, account) != "setUp") {
            // The master password is typed in PassBundle itself.
            startActivity(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            Toast.makeText(this, "Unlock PassBundle, then fill again.", Toast.LENGTH_LONG).show()
            return finishWith(null)
        }
        Biometric.unlock(this, account) { unlocked, error ->
            if (!unlocked) {
                error?.let { Toast.makeText(this, it, Toast.LENGTH_LONG).show() }
                return@unlock finishWith(null)
            }
            finishWith(Responses.forFields(this, fields))
        }
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
        }
    }
}
