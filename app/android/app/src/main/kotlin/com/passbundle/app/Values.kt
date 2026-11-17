package com.passbundle.app

/**
 * How a card's or an identity's values go into the fields Roles.kt found
 * (docs/phones.md, "Autofill"): a card's expiry as the field writes it, a
 * month or a year chosen from a list, a state or a country chosen from a
 * list (Regions.kt), a name in parts or whole, an address in one field. Plain functions, tested on the computer (src/test,
 * ValuesTest). The item's parts come from the core one at a time (`part`,
 * by the names `Item::detail` gives them in core/src/vault.rs), only once
 * the person picks the item.
 */
object Values {
    /** A field's text for a card's or an identity's part; null leaves the field as it is. */
    fun text(role: Role, format: String?, part: (String) -> String?): String? =
        when (role) {
            // Digits only: a field that takes 16 characters has no room for the spaces a number is saved with.
            Role.CARD_NUMBER -> part("cardNumber")?.filter { it.isDigit() }
            Role.CARD_NAME -> part("cardholderName")
            Role.CARD_GIVEN_NAME -> part("cardholderName")?.let { nameParts(it).first }
            Role.CARD_FAMILY_NAME -> part("cardholderName")?.let { nameParts(it).second }
            Role.CARD_EXPIRY -> expiry(part("expMonth"), part("expYear"), format ?: "MM/YY")
            Role.CARD_EXPIRY_MONTH -> month(part("expMonth"))
            Role.CARD_EXPIRY_YEAR -> year(part("expYear"), format ?: "YYYY")
            Role.CARD_CODE -> part("cardCode")
            Role.FULL_NAME -> joined(" ", part("firstName"), part("middleName"), part("lastName"))
            Role.TITLE -> part("title")
            Role.GIVEN_NAME -> part("firstName")
            Role.MIDDLE_NAME -> part("middleName")
            Role.FAMILY_NAME -> part("lastName")
            Role.EMAIL -> part("email")
            Role.PHONE -> part("phone")
            Role.COMPANY -> part("company")
            Role.STREET -> joined(", ", part("address1"), part("address2"), part("address3"))
            Role.ADDRESS -> {
                val place = joined(" ", joined(", ", part("city"), part("state")), part("postalCode"))
                joined(", ", part("address1"), part("address2"), part("address3"), place, part("country"))
            }
            Role.ADDRESS_LINE1 -> part("address1")
            Role.ADDRESS_LINE2 -> part("address2")
            Role.ADDRESS_LINE3 -> part("address3")
            Role.CITY -> part("city")
            Role.STATE -> part("state")
            Role.POSTAL_CODE -> part("postalCode")
            Role.COUNTRY -> part("country")
            // A login's, filled from the login.
            Role.USERNAME, Role.PASSWORD, Role.CODE -> null
        }?.trim()?.ifEmpty { null }

    /**
     * Which of a list's options a card's expiry month or year chooses, or an
     * identity's state or country (Regions.kt); null leaves the list as it is.
     */
    fun option(role: Role, options: List<String>, part: (String) -> String?): Int? =
        when (role) {
            Role.CARD_EXPIRY_MONTH -> monthOption(options, part("expMonth"))
            Role.CARD_EXPIRY_YEAR -> yearOption(options, part("expYear"))
            Role.STATE -> Regions.option(options, part("state"), Regions::state)
            Role.COUNTRY -> Regions.option(options, part("country"), Regions::country)
            else -> null
        }

    /** The words that join the parts there are; null when there are none. */
    private fun joined(between: String, vararg parts: String?): String? =
        parts.mapNotNull { it?.trim()?.ifEmpty { null } }.joinToString(between).ifEmpty { null }

    /** The name on a card, in parts: all but the last word, and the last word. */
    fun nameParts(name: String): Pair<String, String> {
        val words = name.trim().split(Regex("\\s+")).filter { it.isNotEmpty() }
        if (words.size < 2) return (words.firstOrNull() ?: "") to ""
        return words.dropLast(1).joinToString(" ") to words.last()
    }

    /** A card's month as two digits ("07"), from however it's saved ("7", "07"); null when it isn't one. */
    fun month(month: String?): String? =
        month?.trim()?.toIntOrNull()?.takeIf { it in 1..12 }?.toString()?.padStart(2, '0')

    /** A card's year in four digits, or two when the format says "YY"; from "2029" or "29". */
    fun year(year: String?, format: String): String? {
        val digits = year?.trim()?.takeIf { it.length == 2 || it.length == 4 }?.takeIf { y -> y.all { it.isDigit() } } ?: return null
        val full = if (digits.length == 2) "20$digits" else digits
        return if (format == "YY") full.takeLast(2) else full
    }

    /** A card's expiry as the format writes it: "MM/YY" gives "07/29", "MM / YYYY" gives "07 / 2029". */
    fun expiry(month: String?, year: String?, format: String): String? {
        val mm = month(month) ?: return null
        val yyyy = year(year, "YYYY") ?: return null
        return format.replace("MM", mm).replace("YYYY", yyyy).replace("YY", yyyy.takeLast(2))
    }

    /** A month first, then what's between, then the year: "MM/YY", "mm / aaaa", "MMYY". */
    private val EXPIRY_SHOWN = Regex("(?i)(?<![a-z])mm(\\s*[/.\\-]?\\s*)(yyyy|yy|aaaa|aa|jjjj|jj)(?![a-z])")

    /**
     * How a field writes a card's expiry, from what it shows ("MM/YY", "MM /
     * YYYY", "MMYY") or else how long it is (4: "MMYY", 5: "MM/YY", 6:
     * "MMYYYY", 7: "MM/YYYY"); "MM/YY" when it says nothing.
     */
    fun expiryFormat(shown: String?, maxLength: Int?): String {
        EXPIRY_SHOWN.find(shown.orEmpty())?.let {
            val year = if (it.groupValues[2].length == 4) "YYYY" else "YY"
            return "MM" + it.groupValues[1] + year
        }
        return when (maxLength) {
            4 -> "MMYY"
            6 -> "MMYYYY"
            7 -> "MM/YYYY"
            else -> "MM/YY"
        }
    }

    private val TWO_DIGIT_YEAR = Regex("(?i)(?<![a-z])(yy|aa|jj)(?![a-z])")
    private val FOUR_DIGIT_YEAR = Regex("(?i)(?<![a-z])(yyyy|aaaa|jjjj)(?![a-z])")

    /** How a year field writes it: "YY" when it shows two letters or takes two characters, else "YYYY". */
    fun yearFormat(shown: String?, maxLength: Int?): String =
        when {
            FOUR_DIGIT_YEAR.containsMatchIn(shown.orEmpty()) -> "YYYY"
            TWO_DIGIT_YEAR.containsMatchIn(shown.orEmpty()) -> "YY"
            maxLength == 2 -> "YY"
            else -> "YYYY"
        }

    /** The months' English names, as lists show them and Saving.kt reads them. */
    internal val MONTHS =
        listOf("january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december")

    /**
     * The option in a month list for a card's month: by its name ("March",
     * "Mar", "03 - March"), then its number ("3", "03"). Null when none fits,
     * so the list is left as it is.
     */
    fun monthOption(options: List<String>, month: String?): Int? {
        val m = month(month)?.toInt() ?: return null
        val full = MONTHS[m - 1]
        val short = full.take(3)
        val shown = options.map { it.trim().lowercase() }
        shown.indexOfFirst { it == full || it == short || Roles.has(Roles.words(it), full) }.takeIf { it >= 0 }?.let { return it }
        val number = Regex("^0?$m(?![0-9])")
        return shown.indexOfFirst { number.containsMatchIn(it) }.takeIf { it >= 0 }
    }

    /** The option in a year list for a card's year, whether it says "2029" or "29"; null when none does. */
    fun yearOption(options: List<String>, year: String?): Int? {
        val full = year(year, "YYYY") ?: return null
        return options.indexOfFirst { it.trim() == full || it.trim() == full.takeLast(2) }.takeIf { it >= 0 }
    }
}
