package com.passbundle.autofilltest

import android.app.Activity
import android.os.Bundle
import android.text.InputType
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView

/**
 * A plain sign-in form, as apps have: a username, a password, and a button.
 * Signing in says who signed in and how long the password was (never the
 * password), for the autofill test to read off the screen.
 */
class SignInActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val username =
            EditText(this).apply {
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
