package com.passbundle.app

import android.app.assist.AssistStructure
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.view.View
import android.view.autofill.AutofillId

/**
 * The fields PassBundle fills on another app's screen, or a browser's page,
 * as Android's autofill describes them (docs/phones.md, "Autofill"), each
 * with what it's for (Roles.kt): a login's username and password, a one-time
 * code, a card's parts, an identity's. And what's asking: the app's package,
 * and the page's domain when a browser gives one.
 */
class Fields(
    val screen: Roles.Screen<AutofillId>,
    val packageName: String,
    val webDomain: String?,
) {
    val username: AutofillId? get() = screen.username
    val password: AutofillId? get() = screen.password

    /** The first field that asks for a one-time code. */
    val code: AutofillId? get() = screen.code

    /** Every field PassBundle might fill here, once each. */
    val ids: Array<AutofillId> get() = screen.ids.toTypedArray()

    /** The fields a kind of item fills here. */
    fun of(kind: Kind): List<Roles.Found<AutofillId>> = screen.of(kind)

    /** Every extra `from` reads is set here, so nothing added to the intent later stands in for one. */
    fun into(intent: Intent): Intent =
        intent
            .putParcelableArrayListExtra(FOUND, ArrayList(screen.found.map(::bundle)))
            .putExtra(PACKAGE, packageName)
            .putExtra(DOMAIN, webDomain)

    companion object {
        private const val FOUND = "com.passbundle.app.found"
        private const val PACKAGE = "com.passbundle.app.package"
        private const val DOMAIN = "com.passbundle.app.domain"

        private fun bundle(found: Roles.Found<AutofillId>): Bundle =
            Bundle().apply {
                putParcelable("id", found.id)
                putString("role", found.role.name)
                putString("format", found.format)
                putStringArrayList("options", found.options?.let { ArrayList(it) })
            }

        @Suppress("DEPRECATION")
        private fun found(bundle: Bundle): Roles.Found<AutofillId>? {
            val id =
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    bundle.getParcelable("id", AutofillId::class.java)
                } else {
                    bundle.getParcelable("id")
                } ?: return null
            val role = runCatching { Role.valueOf(bundle.getString("role") ?: "") }.getOrNull() ?: return null
            return Roles.Found(id, role, bundle.getString("format"), bundle.getStringArrayList("options"))
        }

        fun from(intent: Intent): Fields {
            @Suppress("DEPRECATION")
            val bundles =
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    intent.getParcelableArrayListExtra(FOUND, Bundle::class.java)
                } else {
                    intent.getParcelableArrayListExtra(FOUND)
                }
            val found = bundles.orEmpty().mapNotNull { runCatching { found(it) }.getOrNull() }
            return Fields(Roles.Screen(found), intent.getStringExtra(PACKAGE) ?: "", intent.getStringExtra(DOMAIN))
        }

        /** What's typed in the username and password fields, for saving. */
        fun values(structure: AssistStructure, fields: Fields): Pair<String?, String?> {
            var username: String? = null
            var password: String? = null

            fun visit(node: AssistStructure.ViewNode) {
                val text = node.autofillValue?.takeIf { it.isText }?.textValue?.toString()
                if (node.autofillId == fields.username) username = text
                if (node.autofillId == fields.password) password = text
                for (i in 0 until node.childCount) visit(node.getChildAt(i))
            }
            for (i in 0 until structure.windowNodeCount) visit(structure.getWindowNodeAt(i).rootViewNode)
            return username to password
        }

        /**
         * What each field PassBundle found holds, for saving a card or an
         * address (Saving.kt): a text field's text, and a list's chosen
         * option as the list shows it. A field with nothing isn't there.
         */
        fun read(structure: AssistStructure, fields: Fields): Map<AutofillId, String> {
            val wanted = fields.ids.toSet()
            val found = HashMap<AutofillId, String>()

            fun visit(node: AssistStructure.ViewNode) {
                val id = node.autofillId
                val value = node.autofillValue
                if (id != null && value != null && id in wanted) {
                    val text =
                        when {
                            value.isText -> value.textValue.toString()
                            value.isList -> node.autofillOptions?.getOrNull(value.listValue)?.toString()
                            else -> null
                        }
                    text?.let { found[id] = it }
                }
                for (i in 0 until node.childCount) visit(node.getChildAt(i))
            }
            for (i in 0 until structure.windowNodeCount) visit(structure.getWindowNodeAt(i).rootViewNode)
            return found
        }

        /** The screen's fields, each described as Roles.kt reads them, and what each is for. */
        fun find(structure: AssistStructure): Fields {
            val described = mutableListOf<Roles.Field<AutofillId>>()
            var domain: String? = null

            fun visit(node: AssistStructure.ViewNode) {
                node.webDomain?.takeIf { it.isNotBlank() && domain == null }?.let { domain = it }
                val id = node.autofillId
                val list = node.autofillType == View.AUTOFILL_TYPE_LIST
                if (id != null && (node.autofillType == View.AUTOFILL_TYPE_TEXT || list)) {
                    // A web page's attributes, as Chromium gives them: one with no
                    // value (`required`, say) comes as null.
                    val html =
                        node.htmlInfo?.attributes.orEmpty()
                            .mapNotNull { attribute ->
                                val name: String? = attribute.first
                                val value: String? = attribute.second
                                name?.let { it.lowercase() to value.orEmpty() }
                            }
                            .toMap()
                    described +=
                        Roles.Field(
                            id = id,
                            hints = node.autofillHints.orEmpty().toList(),
                            html = html,
                            inputType = node.inputType,
                            idEntry = node.idEntry,
                            hint = node.hint,
                            maxLength = node.maxTextLength.takeIf { it > 0 },
                            options = if (list) node.autofillOptions.orEmpty().map { it.toString() } else null,
                        )
                }
                for (i in 0 until node.childCount) visit(node.getChildAt(i))
            }
            for (i in 0 until structure.windowNodeCount) visit(structure.getWindowNodeAt(i).rootViewNode)
            return Fields(Roles.screen(described), structure.activityComponent.packageName, domain)
        }
    }
}
