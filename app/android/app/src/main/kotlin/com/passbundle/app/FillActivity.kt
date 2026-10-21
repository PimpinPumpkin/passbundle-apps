package com.passbundle.app

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.os.Parcelable
import android.view.autofill.AutofillManager
import android.widget.Toast

/**
 * What Android's autofill opens when someone picks one of PassBundle's
 * entries (Responses.kt): fills a login, taking its password from the core
 * only now, or unlocks first. It has no screen of its own; unlocking shows
 * Android's biometric prompt, or opens PassBundle to unlock with the master
 * password.
 */
class FillActivity : Activity() {
    companion object {
        const val MODE = "com.passbundle.app.mode"
        const val ITEM = "com.passbundle.app.item"
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
            FILL -> finishWith(intent.getStringExtra(ITEM)?.let { Responses.filled(this, fields, it) })
            UNLOCK -> unlock(fields)
            else -> finishWith(null)
        }
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
