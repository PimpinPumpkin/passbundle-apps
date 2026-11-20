package com.passbundle.app

/**
 * Fill rules on a website in a browser (DESIGN.md in the meta repo,
 * "Choosing what fills where"; core/src/fill_rules.rs): an item's own
 * rules, after the person's "Never fill this field on this site" list, in
 * the browser extension's order and with its limits. The core decides which
 * of the item's values goes into which of the page's fields, and which
 * fields stay empty (Core.fillRules); this only describes the page to it, as
 * the browser describes it to Android, and fills what it says, each value
 * taken from the core once the item is picked, as every other one is
 * (Responses.filled). What the rules leave goes to Roles.kt's matching,
 * without the fields kept empty, so a value meant for one goes to the next
 * field that fits, as upstream's matching takes what the extension's rules
 * leave. A native app's screen (no web domain from a browser) keeps Roles.kt's
 * matching alone: rules are for websites. Plain functions over plain values,
 * tested on the computer (src/test, FillRulesTest); Fields.kt reads the page
 * from Android's description of the screen.
 */
object FillRules {
    /**
     * One of a page's fields, from the HTML the browser gives for it, and its
     * place among the screen's fields (`at`, in Fields.described).
     */
    data class PageField(
        val at: Int,
        val tag: String? = null,
        val type: String? = null,
        val id: String? = null,
        val name: String? = null,
        val label: String? = null,
        val ariaLabel: String? = null,
        val placeholder: String? = null,
        val autocomplete: String? = null,
        val visible: Boolean = true,
        val list: Boolean = false,
        val maxLength: Int? = null,
        /** The form it's in, by its place in `Page.forms`. */
        val form: Int? = null,
    )

    /** A browser's page: its scheme as the browser says it (null when it doesn't), its fields, and its forms' names. */
    data class Page(val scheme: String?, val fields: List<PageField>, val forms: List<String?> = emptyList())

    /**
     * One value the core says a rule fills: the field, by its place in
     * `Page.fields`; the value, by its name for Core.value; and how the field
     * takes it (`take`, core/src/fill_rules.rs, `Take`).
     */
    data class Fill(val field: Int, val what: String, val take: String)

    /** The page as the core reads it (core/src/fill_rules.rs, `Page`), on the host the browser is trusted to say. */
    fun json(page: Page, host: String): String {
        fun field(f: PageField) =
            listOf(
                "tag" to quote(f.tag),
                "type" to quote(f.type),
                "id" to quote(f.id),
                "name" to quote(f.name),
                "label" to quote(f.label),
                "ariaLabel" to quote(f.ariaLabel),
                "placeholder" to quote(f.placeholder),
                "autocomplete" to quote(f.autocomplete),
                "visible" to f.visible.toString(),
                "list" to f.list.toString(),
                "maxLength" to (f.maxLength?.toString() ?: "null"),
                "form" to (f.form?.toString() ?: "null"),
            ).joinToString(",", "{", "}") { (key, value) -> "${quote(key)}:$value" }
        return "{\"scheme\":${quote(page.scheme)},\"host\":${quote(host)}," +
            "\"fields\":${page.fields.joinToString(",", "[", "]", transform = ::field)}," +
            "\"forms\":${page.forms.joinToString(",", "[", "]", transform = ::quote)}}"
    }

    /** A JSON string, or null. */
    private fun quote(text: String?): String =
        if (text == null) {
            "null"
        } else {
            buildString {
                append('"')
                for (c in text) {
                    when {
                        c == '"' -> append("\\\"")
                        c == '\\' -> append("\\\\")
                        c < ' ' || c == '\u2028' || c == '\u2029' -> append("\\u%04x".format(c.code))
                        else -> append(c)
                    }
                }
                append('"')
            }
        }

    /**
     * A text field's text for a value, as the core says the field takes it:
     * as it is, a card's number as digits, its month as two digits, its year
     * as four, or two where the field takes only two (as the extension writes
     * them). Null leaves the field as it is.
     */
    fun text(take: String, value: String): String? =
        when (take) {
            "text" -> value
            "digits" -> value.filter { it.isDigit() }
            "month" -> Values.month(value) ?: value
            "year", "shortYear" -> {
                val trimmed = value.trim()
                val year = if (trimmed.length == 2 && trimmed.all { it.isDigit() }) "20$trimmed" else trimmed
                if (take == "shortYear") year.takeLast(2) else year
            }
            else -> null
        }?.ifEmpty { null }

    /**
     * Which of a list's options a value chooses, as the core says the list
     * takes it: a card's month or year, a state or a country by its code or
     * its name (Values.kt, Regions.kt), or else the option that is the value.
     * Null leaves the list as it is.
     */
    fun option(take: String, options: List<String>, value: String): Int? =
        when (take) {
            "monthOption" -> Values.monthOption(options, value)
            "yearOption" -> Values.yearOption(options, value)
            "stateOption" -> Regions.option(options, value, Regions::state)
            "countryOption" -> Regions.option(options, value, Regions::country)
            "option" -> options.indexOfFirst { it.trim().equals(value.trim(), ignoreCase = true) }.takeIf { it >= 0 }
            else -> null
        }

    /**
     * The screen Roles.kt's matching fills after the rules: the screen's
     * fields (`described`) without those the rules keep empty (`never`, by
     * their place in `page.fields`).
     */
    fun <T> left(described: List<Roles.Field<T>>, page: Page, never: List<Int>): Roles.Screen<T> {
        val kept = never.mapNotNull { page.fields.getOrNull(it)?.at }.toSet()
        return Roles.screen(described.filterIndexed { at, _ -> at !in kept })
    }
}
