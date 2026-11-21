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
 * and the page's domain when a browser gives one. On a web page, also every
 * field Roles.kt read (`described`) and the page's own description of its
 * fields (`page`), for the item's fill rules (FillRules.kt); a page whose
 * description didn't come through whole (`pageLost`) fills nothing, rather
 * than a field its rules keep empty.
 */
class Fields(
    val screen: Roles.Screen<AutofillId>,
    val packageName: String,
    val webDomain: String?,
    val described: List<Roles.Field<AutofillId>> = emptyList(),
    val page: FillRules.Page? = null,
    val pageLost: Boolean = false,
) {
    val username: AutofillId? get() = screen.username
    val password: AutofillId? get() = screen.password

    /** The first field that asks for a one-time code. */
    val code: AutofillId? get() = screen.code

    /** Every field PassBundle might fill here, once each. */
    val ids: Array<AutofillId> get() = screen.ids.toTypedArray()

    /** The fields a kind of item fills here. */
    fun of(kind: Kind): List<Roles.Found<AutofillId>> = screen.of(kind)

    /**
     * Every extra `from` reads is set here, so nothing added to the intent
     * later stands in for one: a page's description too, or that there's
     * none.
     */
    fun into(intent: Intent): Intent =
        intent
            .putParcelableArrayListExtra(FOUND, ArrayList(screen.found.map(::bundle)))
            .putExtra(PACKAGE, packageName)
            .putExtra(DOMAIN, webDomain)
            .putParcelableArrayListExtra(DESCRIBED, page?.let { ArrayList(described.map(::fieldBundle)) })
            .putExtra(PAGE, page?.let(::pageBundle))

    companion object {
        private const val FOUND = "com.passbundle.app.found"
        private const val PACKAGE = "com.passbundle.app.package"
        private const val DOMAIN = "com.passbundle.app.domain"
        private const val DESCRIBED = "com.passbundle.app.described"
        private const val PAGE = "com.passbundle.app.page"

        private fun bundle(found: Roles.Found<AutofillId>): Bundle =
            Bundle().apply {
                putParcelable("id", found.id)
                putString("role", found.role.name)
                putString("format", found.format)
                putStringArrayList("options", found.options?.let { ArrayList(it) })
            }

        @Suppress("DEPRECATION")
        private fun id(bundle: Bundle): AutofillId? =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                bundle.getParcelable("id", AutofillId::class.java)
            } else {
                bundle.getParcelable("id")
            }

        private fun found(bundle: Bundle): Roles.Found<AutofillId>? {
            val id = id(bundle) ?: return null
            val role = runCatching { Role.valueOf(bundle.getString("role") ?: "") }.getOrNull() ?: return null
            return Roles.Found(id, role, bundle.getString("format"), bundle.getStringArrayList("options"))
        }

        /** A field as Roles.kt reads it, for the fill rules' matching of what they leave. */
        private fun fieldBundle(field: Roles.Field<AutofillId>): Bundle =
            Bundle().apply {
                putParcelable("id", field.id)
                putStringArrayList("hints", ArrayList(field.hints))
                putBundle("html", Bundle().apply { for ((name, value) in field.html) putString(name, value) })
                putInt("inputType", field.inputType)
                putString("idEntry", field.idEntry)
                putString("hint", field.hint)
                putInt("maxLength", field.maxLength ?: 0)
                putStringArrayList("options", field.options?.let { ArrayList(it) })
            }

        private fun described(bundle: Bundle): Roles.Field<AutofillId>? {
            val id = id(bundle) ?: return null
            val html = bundle.getBundle("html") ?: return null
            return Roles.Field(
                id = id,
                hints = bundle.getStringArrayList("hints").orEmpty(),
                html = html.keySet().associateWith { html.getString(it).orEmpty() },
                inputType = bundle.getInt("inputType"),
                idEntry = bundle.getString("idEntry"),
                hint = bundle.getString("hint"),
                maxLength = bundle.getInt("maxLength").takeIf { it > 0 },
                options = bundle.getStringArrayList("options"),
            )
        }

        /** A page's description of its fields, for the fill rules. A form without a name is kept as "". */
        private fun pageBundle(page: FillRules.Page): Bundle =
            Bundle().apply {
                putString("scheme", page.scheme)
                putStringArrayList("forms", ArrayList(page.forms.map { it.orEmpty() }))
                putParcelableArrayList(
                    "fields",
                    ArrayList(
                        page.fields.map { f ->
                            Bundle().apply {
                                putInt("at", f.at)
                                putString("tag", f.tag)
                                putString("type", f.type)
                                putString("id", f.id)
                                putString("name", f.name)
                                putString("label", f.label)
                                putString("ariaLabel", f.ariaLabel)
                                putString("placeholder", f.placeholder)
                                putString("autocomplete", f.autocomplete)
                                putBoolean("visible", f.visible)
                                putBoolean("list", f.list)
                                putInt("maxLength", f.maxLength ?: 0)
                                putInt("form", f.form ?: -1)
                            }
                        },
                    ),
                )
            }

        private fun page(bundle: Bundle): FillRules.Page? {
            @Suppress("DEPRECATION")
            val fields =
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    bundle.getParcelableArrayList("fields", Bundle::class.java)
                } else {
                    bundle.getParcelableArrayList("fields")
                } ?: return null
            val forms = bundle.getStringArrayList("forms") ?: return null
            return FillRules.Page(
                scheme = bundle.getString("scheme"),
                fields =
                    fields.map { f ->
                        FillRules.PageField(
                            at = f.getInt("at", -1),
                            tag = f.getString("tag"),
                            type = f.getString("type"),
                            id = f.getString("id"),
                            name = f.getString("name"),
                            label = f.getString("label"),
                            ariaLabel = f.getString("ariaLabel"),
                            placeholder = f.getString("placeholder"),
                            autocomplete = f.getString("autocomplete"),
                            visible = f.getBoolean("visible"),
                            list = f.getBoolean("list"),
                            maxLength = f.getInt("maxLength").takeIf { it > 0 },
                            form = f.getInt("form", -1).takeIf { it >= 0 },
                        )
                    },
                forms = forms.map { it?.ifEmpty { null } },
            )
        }

        fun from(intent: Intent): Fields {
            @Suppress("DEPRECATION")
            fun bundles(name: String): List<Bundle>? =
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    intent.getParcelableArrayListExtra(name, Bundle::class.java)
                } else {
                    intent.getParcelableArrayListExtra(name)
                }
            val found = bundles(FOUND).orEmpty().mapNotNull { runCatching { found(it) }.getOrNull() }
            // A page's fields name the screen's by their place, so both come through whole, or neither counts.
            val described = bundles(DESCRIBED)?.map { runCatching { described(it) }.getOrNull() }
            @Suppress("DEPRECATION")
            val pageBundle =
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    intent.getParcelableExtra(PAGE, Bundle::class.java)
                } else {
                    intent.getParcelableExtra(PAGE)
                }
            val page = pageBundle?.let { runCatching { page(it) }.getOrNull() }
            val whole =
                described != null && page != null && described.all { it != null } &&
                    page.fields.all { it.at in described.indices }
            return Fields(
                Roles.Screen(found),
                intent.getStringExtra(PACKAGE) ?: "",
                intent.getStringExtra(DOMAIN),
                described = if (whole) described.orEmpty().filterNotNull() else emptyList(),
                page = if (whole) page else null,
                pageLost = !whole && (described != null || pageBundle != null),
            )
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

        /**
         * The screen's fields, each described as Roles.kt reads them, and what
         * each is for; and on a web page, each of its fields as the page
         * describes it (its element, its attributes, the form it's in), for
         * the fill rules, and the page's scheme with its domain.
         */
        fun find(structure: AssistStructure): Fields {
            val described = mutableListOf<Roles.Field<AutofillId>>()
            val pageFields = mutableListOf<FillRules.PageField>()
            val forms = mutableListOf<String?>()
            var domain: String? = null
            var scheme: String? = null

            fun visit(node: AssistStructure.ViewNode, form: Int?) {
                node.webDomain?.takeIf { it.isNotBlank() && domain == null }?.let {
                    domain = it
                    scheme = node.webScheme
                }
                // A web page's attributes, as Chromium gives them: one with no
                // value (`required`, say) comes as null.
                val info = node.htmlInfo
                val html =
                    info?.attributes.orEmpty()
                        .mapNotNull { attribute ->
                            val name: String? = attribute.first
                            val value: String? = attribute.second
                            name?.let { it.lowercase() to value.orEmpty() }
                        }
                        .toMap()
                var inForm = form
                if (info?.tag.equals("form", ignoreCase = true)) {
                    inForm = forms.size
                    forms += html["name"]?.ifEmpty { null }
                }
                val id = node.autofillId
                val list = node.autofillType == View.AUTOFILL_TYPE_LIST
                if (id != null && (node.autofillType == View.AUTOFILL_TYPE_TEXT || list)) {
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
                    if (info != null) {
                        pageFields +=
                            FillRules.PageField(
                                at = described.size - 1,
                                tag = info.tag,
                                type = html["type"],
                                id = html["id"],
                                name = html["name"],
                                label = html["label"],
                                ariaLabel = html["aria-label"],
                                // Chromium gives a field's placeholder as its hint.
                                placeholder = html["placeholder"] ?: node.hint,
                                autocomplete = html["autocomplete"],
                                visible = node.visibility == View.VISIBLE,
                                list = list,
                                maxLength = node.maxTextLength.takeIf { it > 0 } ?: html["maxlength"]?.trim()?.toIntOrNull(),
                                form = inForm,
                            )
                    }
                }
                for (i in 0 until node.childCount) visit(node.getChildAt(i), inForm)
            }
            for (i in 0 until structure.windowNodeCount) visit(structure.getWindowNodeAt(i).rootViewNode, null)
            // Fill rules are for a website's own fields: a page a browser says it's showing.
            val page = FillRules.Page(scheme, pageFields, forms).takeIf { domain != null && pageFields.isNotEmpty() }
            return Fields(
                Roles.screen(described),
                structure.activityComponent.packageName,
                domain,
                described = if (page != null) described else emptyList(),
                page = page,
            )
        }
    }
}
