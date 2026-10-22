package com.passbundle.autofilltest

import android.app.Activity
import android.os.Bundle
import android.text.InputType
import android.view.View
import android.view.ViewStructure
import android.view.autofill.AutofillManager
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView

/**
 * A plain sign-in form, as apps have: a username, a password, and a button.
 * Signing in says who signed in and how long the password was (never the
 * password), for the autofill test to read off the screen, and tells
 * Android's autofill the sign-in is done, so it offers to save a new one.
 */
open class SignInActivity : Activity() {
    /** The website this screen tells Android's autofill it shows, if any. */
    open val claimsDomain: String? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val domain = claimsDomain
        val username =
            // Says it's the website on the field itself, which Android always
            // puts in what autofill sees.
            object : EditText(this) {
                override fun onProvideAutofillStructure(structure: ViewStructure, flags: Int) {
                    super.onProvideAutofillStructure(structure, flags)
                    domain?.let { structure.setWebDomain(it) }
                }
            }.apply {
                id = R.id.username
                hint = "Username"
                setAutofillHints(View.AUTOFILL_HINT_USERNAME)
                // A blinking cursor keeps the screen from ever settling, which the test waits for.
                isCursorVisible = false
            }
        val password =
            EditText(this).apply {
                id = R.id.password
                hint = "Password"
                inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
                setAutofillHints(View.AUTOFILL_HINT_PASSWORD)
                isCursorVisible = false
            }
        val result = TextView(this).apply { id = R.id.result }
        val signIn =
            Button(this).apply {
                id = R.id.sign_in
                text = "Sign in"
                setOnClickListener {
                    result.text = "Signed in as ${username.text}, password ${password.text.length} characters"
                    getSystemService(AutofillManager::class.java)?.commit()
                }
            }
        setContentView(
            LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                setPadding(48, 160, 48, 48)
                // Nothing focused at the start, so Android's autofill list opens
                // when the test taps the username, as for someone, not before.
                isFocusableInTouchMode = true
                requestFocus()
                addView(username)
                addView(password)
                addView(signIn)
                addView(result)
            },
        )
    }
}

/**
 * The same form, on a screen that tells Android's autofill it's a bank's
 * website, as any app can. PassBundle mustn't offer that website's logins
 * here until the person trusts this app as a browser (it opens web links, so
 * it's offered).
 */
class ClaimsWebsiteActivity : SignInActivity() {
    override val claimsDomain = "bank.example.test"
}
