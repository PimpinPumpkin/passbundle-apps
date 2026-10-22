package com.passbundle.app

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.os.Bundle
import android.os.Parcelable
import android.text.InputType
import android.view.autofill.AutofillManager
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
 */
class FillActivity : Activity() {
    companion object {
        const val MODE = "com.passbundle.app.mode"
        const val ITEM = "com.passbundle.app.item"
        const val ASKS = "com.passbundle.app.asks"
        const val FILL = "fill"
        const val UNLOCK = "unlock"
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Core.init(applicationContext)
        Biometric.init(applicationContext)
        Core.openIfClosed(filesDir.absolutePath)
        val fields = Fields.from(intent)
        when (intent.getStringExtra(MODE)) {
            FILL -> fill(fields, intent.getStringExtra(ITEM), intent.getBooleanExtra(ASKS, false))
            UNLOCK -> unlock(fields)
            else -> finishWith(null)
        }
    }

    private fun fill(fields: Fields, item: String?, asks: Boolean) {
        if (item == null) return finishWith(null)
        if (!asks) return finishWith(Responses.filled(this, fields, item))
        val password = EditText(this).apply {
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
            hint = "Master password"
        }
        AlertDialog.Builder(this)
            .setTitle("PassBundle")
            .setMessage("This login asks for your master password.")
            .setView(password)
            .setPositiveButton("Continue") { _, _ ->
                val typed = password.text.toString()
                password.text.clear()
                // The check derives the master key: off the main thread.
                Thread {
                    val answer = JSONObject(Core.confirmMasterPassword(item, typed) ?: "{}")
                    runOnUiThread {
                        if (answer.optBoolean("ok")) {
                            finishWith(Responses.filled(this, fields, item))
                        } else {
                            Toast.makeText(this, answer.optString("error", "Couldn't check that."), Toast.LENGTH_LONG).show()
                            finishWith(null)
                        }
                    }
                }.start()
            }
            .setNegativeButton("Cancel") { _, _ -> finishWith(null) }
            .setOnCancelListener { finishWith(null) }
            .show()
    }

    private fun unlock(fields: Fields) {
        if (Biometric.status(this) != "setUp") {
            // The master password is typed in PassBundle itself.
            startActivity(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            Toast.makeText(this, "Unlock PassBundle, then fill again.", Toast.LENGTH_LONG).show()
            return finishWith(null)
        }
        Biometric.unlock(this) { unlocked, error ->
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
    }
}
