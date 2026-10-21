package com.passbundle.app

import android.app.assist.AssistStructure
import android.content.Intent
import android.os.Build
import android.text.InputType
import android.view.View
import android.view.autofill.AutofillId

/**
 * The sign-in fields on another app's screen, or a browser's page, as Android's
 * autofill describes them (docs/phones.md, "Autofill"), and what's asking: the
 * app's package, and the page's domain when a browser gives one.
 */
class Fields(
    val username: AutofillId?,
    val password: AutofillId?,
    val packageName: String,
    val webDomain: String?,
) {
    val ids get() = listOfNotNull(username, password).toTypedArray()

    fun into(intent: Intent): Intent =
        intent
            .putExtra(USERNAME, username)
            .putExtra(PASSWORD, password)
            .putExtra(PACKAGE, packageName)
            .putExtra(DOMAIN, webDomain)

    companion object {
        private const val USERNAME = "com.passbundle.app.username"
        private const val PASSWORD = "com.passbundle.app.password"
        private const val PACKAGE = "com.passbundle.app.package"
        private const val DOMAIN = "com.passbundle.app.domain"

        private val PASSWORD_VARIATIONS =
            setOf(
                InputType.TYPE_TEXT_VARIATION_PASSWORD,
                InputType.TYPE_TEXT_VARIATION_WEB_PASSWORD,
                InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD,
            )
        private val USERNAME_WORDS = listOf("user", "email", "e-mail", "login", "account")

        fun from(intent: Intent): Fields {
            @Suppress("DEPRECATION")
            fun id(key: String): AutofillId? =
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    intent.getParcelableExtra(key, AutofillId::class.java)
                } else {
                    intent.getParcelableExtra(key)
                }
            return Fields(id(USERNAME), id(PASSWORD), intent.getStringExtra(PACKAGE) ?: "", intent.getStringExtra(DOMAIN))
        }

        /** The username and password fields: by the app's own hints first, then by what they look like. */
        fun find(structure: AssistStructure): Fields {
            var username: AutofillId? = null
            var password: AutofillId? = null
            var domain: String? = null
            // The text field just before the password, for a form that says nothing about its username.
            var textBeforePassword: AutofillId? = null
            var lastText: AutofillId? = null

            fun visit(node: AssistStructure.ViewNode) {
                node.webDomain?.takeIf { it.isNotBlank() && domain == null }?.let { domain = it }
                val id = node.autofillId
                if (id != null && node.autofillType == View.AUTOFILL_TYPE_TEXT) {
                    val hints = node.autofillHints.orEmpty().map { it.lowercase() }
                    val html = node.htmlInfo?.attributes.orEmpty().associate { it.first.lowercase() to it.second.lowercase() }
                    val textClass = (node.inputType and InputType.TYPE_MASK_CLASS) == InputType.TYPE_CLASS_TEXT
                    val variation = node.inputType and InputType.TYPE_MASK_VARIATION
                    val words = listOfNotNull(node.idEntry, node.hint?.toString(), html["name"], html["id"]).joinToString(" ").lowercase()
                    val isPassword =
                        hints.any { "password" in it } || html["type"] == "password" ||
                            (textClass && variation in PASSWORD_VARIATIONS)
                    val isUsername =
                        hints.any { it == "username" || "email" in it } || html["type"] == "email" ||
                            html["autocomplete"] == "username" || USERNAME_WORDS.any { it in words }
                    when {
                        isPassword -> if (password == null) {
                            password = id
                            textBeforePassword = lastText
                        }
                        isUsername -> if (username == null) username = id
                        else -> lastText = id
                    }
                }
                for (i in 0 until node.childCount) visit(node.getChildAt(i))
            }
            for (i in 0 until structure.windowNodeCount) visit(structure.getWindowNodeAt(i).rootViewNode)
            return Fields(username ?: textBeforePassword, password, structure.activityComponent.packageName, domain)
        }
    }
}
