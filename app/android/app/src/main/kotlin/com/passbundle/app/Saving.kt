package com.passbundle.app

/**
 * What Android offers to save from a screen, and what's saved (docs/phones.md,
 * "Autofill"): a sign-in with a password, as before; or on a payment form, a
 * card (its number, with its expiry or the name on it); or on an address
 * form, an identity (two or more of a name, a street, a city, and a postal
 * code). Accepting opens PassBundle's editor filled in, as for a sign-in, and
 * nothing is saved until the person taps Save there.
 *
 * Not offered: a card the person has (the same last 4 digits, while
 * PassBundle is unlocked; the whole number is checked once they accept), an
 * address one of their identities has (its street and postal code), or a
 * kind an organization's policy removes in every unlocked account
 * (restricted_types.rs in the core). Android decides whether to ask from
 * what was typed, with a check PassBundle hands it (`Check`), so that check
 * holds no secret: never a whole card number.
 *
 * Plain functions over plain values (Roles.kt's fields), kept apart from
 * Android's classes so they're tested on the computer (src/test, SavingTest).
 */
object Saving {
    /** What a screen's save is: a sign-in, a card, or an address (an identity). */
    enum class What { LOGIN, CARD, ADDRESS }

    /**
     * Android's check of what was typed before it asks to save (its
     * `Validator`s), as plain values so it's tested here: `holds` reads
     * values as Android does, and Responses.kt makes Android's from it.
     */
    sealed class Check<T> {
        /** The field's whole value matches (Android's `RegexValidator`). */
        data class Matches<T>(val id: T, val regex: String) : Check<T>()

        /** The field's digits pass Luhn's checksum; anything else in it is skipped (`LuhnChecksumValidator`). */
        data class Luhn<T>(val id: T) : Check<T>()

        data class All<T>(val checks: List<Check<T>>) : Check<T>()

        data class AnyOf<T>(val checks: List<Check<T>>) : Check<T>()

        data class Not<T>(val check: Check<T>) : Check<T>()

        /** Whether it holds for these values (a list's chosen option as its text); a field with none fails. */
        fun holds(value: (T) -> String?): Boolean =
            when (this) {
                is Matches -> value(id)?.let { Regex(regex).matches(it) } ?: false
                is Luhn -> value(id)?.let { luhn(it.filter { c -> c in '0'..'9' }) } ?: false
                is All -> checks.all { it.holds(value) }
                is AnyOf -> checks.any { it.holds(value) }
                is Not -> !check.holds(value)
            }
    }

    /**
     * A save Android offers on a screen: what it is, the fields that must
     * have something in them for Android to ask (a sign-in's password, a
     * card's number; an address has none), the other fields it saves, and
     * its check of what was typed.
     */
    data class Offer<T>(val what: What, val required: List<T>, val optional: List<T>, val check: Check<T>?)

    /**
     * What tells a new card or address from one the person has, from every
     * unlocked account (Core.saveHints): whether one may be made in any of
     * them, each card's last 4 digits, and each identity's street (its
     * words) and postal code (its letters and digits), as the core compares
     * them. Null while PassBundle is locked: then every card and address is
     * offered, and checked once the person accepts.
     */
    data class Known(
        val mayAddCard: Boolean = true,
        val mayAddAddress: Boolean = true,
        val last4: List<String> = emptyList(),
        val addresses: List<Pair<List<String>, String>> = emptyList(),
    )

    private val CARD_EXPIRY = setOf(Role.CARD_EXPIRY, Role.CARD_EXPIRY_MONTH, Role.CARD_EXPIRY_YEAR)
    private val CARD_NAME = setOf(Role.CARD_NAME, Role.CARD_GIVEN_NAME, Role.CARD_FAMILY_NAME)

    /** An address's parts, two of which make a form an address's. */
    private val NAME = setOf(Role.FULL_NAME, Role.GIVEN_NAME, Role.FAMILY_NAME)
    private val STREET = setOf(Role.ADDRESS_LINE1, Role.STREET, Role.ADDRESS)
    private val ADDRESS_GROUPS = listOf(NAME, STREET, setOf(Role.CITY), setOf(Role.POSTAL_CODE))

    /** Anything but a letter or a digit, as the core splits a street into words. */
    private const val NOT_WORD = "[^\\p{L}\\p{N}]"
    private const val NOT_WORD_OR_LINE = "[^\\p{L}\\p{N}\\n]"

    /**
     * What Android offers to save on a screen, if anything: a sign-in where
     * there's a password; else a card where there's a card's number with its
     * expiry or the name on it; else an address where there are fields for
     * two of a name, a street, a city, and a postal code. `known` is what
     * PassBundle knows while it's unlocked, null while it's locked.
     */
    fun <T> offer(screen: Roles.Screen<T>, known: Known?): Offer<T>? {
        screen.password?.let { password ->
            return Offer(What.LOGIN, listOf(password), listOfNotNull(screen.username), null)
        }
        if (known?.mayAddCard != false) cardOffer(screen, known?.last4.orEmpty())?.let { return it }
        if (known?.mayAddAddress != false) addressOffer(screen, known?.addresses.orEmpty())?.let { return it }
        return null
    }

    private fun <T> cardOffer(screen: Roles.Screen<T>, last4: List<String>): Offer<T>? {
        val fields = screen.of(Kind.CARD)
        val number = fields.firstOrNull { it.role == Role.CARD_NUMBER && it.options == null }?.id ?: return null
        // Its expiry or the name on it, one of them typed (or chosen, for a list).
        val given =
            fields.filter { it.role in CARD_EXPIRY || it.role in CARD_NAME }.map { field ->
                val regex =
                    when {
                        field.role in CARD_NAME -> HAS_LETTER
                        field.options != null && field.role == Role.CARD_EXPIRY_MONTH -> MONTH_CHOSEN
                        else -> HAS_DIGIT
                    }
                Check.Matches(field.id, regex)
            }
        if (given.isEmpty()) return null
        val checks =
            mutableListOf<Check<T>>(
                Check.Matches(number, CARD_DIGITS),
                Check.Luhn(number),
                Check.AnyOf(given),
            )
        if (last4.isNotEmpty()) checks += Check.Not(Check.Matches(number, endsWithAny(last4)))
        val others = fields.map { it.id }.distinct().filter { it != number }
        return Offer(What.CARD, listOf(number), others, Check.All(checks))
    }

    private fun <T> addressOffer(screen: Roles.Screen<T>, addresses: List<Pair<List<String>, String>>): Offer<T>? {
        val fields = screen.of(Kind.IDENTITY).filter { it.options == null }
        // Each part the form has, with the check that something's typed in it.
        val parts =
            ADDRESS_GROUPS.mapNotNull { group ->
                val ids = fields.filter { it.role in group }.map { it.id }.distinct()
                if (ids.isEmpty()) null else Check.AnyOf<T>(ids.map { Check.Matches(it, HAS_WORD) })
            }
        if (parts.size < 2) return null
        val pairs = mutableListOf<Check<T>>()
        for (i in parts.indices) for (j in i + 1 until parts.size) pairs += Check.All(listOf(parts[i], parts[j]))
        val checks = mutableListOf<Check<T>>(Check.AnyOf(pairs))
        // Not an identity's street and postal code: the first street line, by its words.
        val street = fields.firstOrNull { it.role == Role.ADDRESS_LINE1 } ?: fields.firstOrNull { it.role == Role.STREET }
        val postalCode = fields.firstOrNull { it.role == Role.POSTAL_CODE }
        val known = addresses.filter { it.first.isNotEmpty() && it.second.isNotEmpty() }
        if (street != null && postalCode != null && known.isNotEmpty()) {
            val same =
                known.map { (words, postal) ->
                    Check.All(listOf(Check.Matches(street.id, firstLine(words)), Check.Matches(postalCode.id, sameLettersAndDigits(postal))))
                }
            checks += Check.Not(Check.AnyOf(same))
        }
        val ids = screen.of(Kind.IDENTITY).map { it.id }.distinct()
        return Offer(What.ADDRESS, emptyList(), ids, Check.All(checks))
    }

    private const val HAS_LETTER = "(?s).*\\p{L}.*"
    private const val HAS_DIGIT = "(?s).*\\d.*"
    private const val HAS_WORD = "(?s).*[\\p{L}\\p{N}].*"

    /** A month chosen from a list: a number or a month's name, never "Month" or "MM". */
    private const val MONTH_CHOSEN = "(?is).*(\\d|jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec).*"

    /** A card's number: 12 to 19 digits, with spaces or dashes between, as people type it. */
    private const val CARD_DIGITS = "[\\s-]*(?:\\d[\\s-]*){12,19}"

    /** A number whose last 4 digits are one of these: "4242" matches "4000 0000 0000 4242". */
    fun endsWithAny(last4: List<String>): String =
        "(?s).*(?:" + last4.joinToString("|") { it.toList().joinToString("\\D*") } + ")\\D*"

    /** A street whose first line is these words (lowercase, as the core has them), whatever is between them. */
    fun firstLine(words: List<String>): String =
        "(?isu)$NOT_WORD*" + words.joinToString("$NOT_WORD_OR_LINE+") { Regex.escape(it) } + "$NOT_WORD_OR_LINE*(?:\\n.*)?"

    /** A postal code with these letters and digits (lowercase, as the core has them), whatever is between them. */
    fun sameLettersAndDigits(key: String): String =
        "(?iu)$NOT_WORD*" + key.map { Regex.escape(it.toString()) }.joinToString("$NOT_WORD*") + "$NOT_WORD*"

    /** Luhn's checksum over digits; false for none. */
    fun luhn(digits: String): Boolean {
        if (digits.isEmpty()) return false
        var sum = 0
        for ((i, c) in digits.reversed().withIndex()) {
            var d = c - '0'
            if (i % 2 == 1) {
                d *= 2
                if (d > 9) d -= 9
            }
            sum += d
        }
        return sum % 10 == 0
    }

    /**
     * The card to save from what its fields held (a list's chosen option as
     * its text), by the editor's names for its parts (`kind` "card"); null
     * when it isn't one: a number of 12 to 19 digits that passes Luhn's
     * checksum, with its expiry or the name on it. Its security code only
     * from a field marked for it (Roles.kt finds no other).
     */
    fun <T> card(screen: Roles.Screen<T>, value: (T) -> String?): Map<String, String>? {
        val fields = screen.of(Kind.CARD)
        fun first(role: Role) = fields.filter { it.role == role }.firstNotNullOfOrNull { value(it.id)?.trim()?.ifEmpty { null } }
        val number = first(Role.CARD_NUMBER)?.filter { it in '0'..'9' } ?: return null
        if (number.length !in 12..19 || !luhn(number)) return null
        var month = first(Role.CARD_EXPIRY_MONTH)?.let(::month)
        var year = first(Role.CARD_EXPIRY_YEAR)?.let(::year)
        first(Role.CARD_EXPIRY)?.let(::expiry)?.let { (m, y) ->
            month = month ?: m
            year = year ?: y
        }
        val name = first(Role.CARD_NAME) ?: joined(first(Role.CARD_GIVEN_NAME), first(Role.CARD_FAMILY_NAME))
        if (month == null && year == null && name == null) return null
        val brand = brand(number)
        return listOfNotNull(
            "kind" to "card",
            "name" to (brand ?: "Card"),
            "number" to number,
            brand?.let { "brand" to it },
            month?.let { "expMonth" to it.toString() },
            year?.let { "expYear" to it },
            name?.let { "cardholderName" to it },
            first(Role.CARD_CODE)?.let { "code" to it },
        ).toMap()
    }

    /**
     * The identity to save from what an address form's fields held, by the
     * editor's names for its parts (`kind` "identity"); null unless two or
     * more of a name, a street, a city, and a postal code are there. A whole
     * name in one field is split as a card's is (Values.nameParts); a street
     * in one field, by its lines. Named by its street, else the person's name.
     */
    fun <T> identity(screen: Roles.Screen<T>, value: (T) -> String?): Map<String, String>? {
        val fields = screen.of(Kind.IDENTITY)
        fun first(role: Role) = fields.filter { it.role == role }.firstNotNullOfOrNull { value(it.id)?.trim()?.ifEmpty { null } }
        var given = first(Role.GIVEN_NAME)
        var family = first(Role.FAMILY_NAME)
        val whole = first(Role.FULL_NAME)
        if (whole != null && given == null && family == null) {
            val (first, last) = Values.nameParts(whole)
            given = first.ifEmpty { null }
            family = last.ifEmpty { null }
        }
        val lines =
            first(Role.STREET)?.lines()?.map { it.trim() }?.filter { it.isNotEmpty() }.orEmpty()
        val address1 = first(Role.ADDRESS_LINE1) ?: lines.getOrNull(0) ?: first(Role.ADDRESS)?.lines()?.map { it.trim() }?.filter { it.isNotEmpty() }?.joinToString(", ")
        val address2 = first(Role.ADDRESS_LINE2) ?: lines.getOrNull(1).takeIf { first(Role.ADDRESS_LINE1) == null }
        val address3 = first(Role.ADDRESS_LINE3) ?: lines.drop(2).joinToString(", ").ifEmpty { null }.takeIf { first(Role.ADDRESS_LINE1) == null }
        val city = first(Role.CITY)
        val postalCode = first(Role.POSTAL_CODE)
        val named = joined(given, first(Role.MIDDLE_NAME), family)
        if (listOf(named, address1, city, postalCode).count { it != null } < 2) return null
        return listOfNotNull(
            "kind" to "identity",
            "name" to (address1 ?: named ?: "Address"),
            first(Role.TITLE)?.let { "title" to it },
            given?.let { "firstName" to it },
            first(Role.MIDDLE_NAME)?.let { "middleName" to it },
            family?.let { "lastName" to it },
            first(Role.COMPANY)?.let { "company" to it },
            first(Role.EMAIL)?.let { "email" to it },
            first(Role.PHONE)?.let { "phone" to it },
            address1?.let { "address1" to it },
            address2?.let { "address2" to it },
            address3?.let { "address3" to it },
            city?.let { "city" to it },
            first(Role.STATE)?.let { "state" to it },
            postalCode?.let { "postalCode" to it },
            first(Role.COUNTRY)?.let { "country" to it },
        ).toMap()
    }

    private fun joined(vararg parts: String?): String? = parts.filterNotNull().joinToString(" ").ifEmpty { null }

    /** A month as typed or chosen: "07", "7", "July", "Jul", "07 - July"; null when it isn't one. */
    fun month(text: String): Int? {
        val words = Roles.words(text)
        Values.MONTHS.indexOfFirst { full -> words.any { it == full || it == full.take(3) } }.takeIf { it >= 0 }?.let { return it + 1 }
        return Regex("^\\D*(\\d{1,2})(?!\\d)").find(text)?.groupValues?.get(1)?.toIntOrNull()?.takeIf { it in 1..12 }
    }

    /** A year in four digits, from "2029" or "29"; null when it isn't one. */
    fun year(text: String): String? {
        val digits = text.trim()
        return when {
            Regex("20\\d\\d").matches(digits) -> digits
            Regex("\\d\\d").matches(digits) -> "20$digits"
            else -> null
        }
    }

    /** A card's expiry in one field: "07/29", "7 / 2029", "0729", "072029"; null when it can't be read. */
    fun expiry(text: String): Pair<Int, String>? {
        val parts = text.split(Regex("\\D+")).filter { it.isNotEmpty() }
        val (mm, yy) =
            when {
                parts.size == 2 -> parts[0] to parts[1]
                parts.size == 1 && parts[0].length == 4 -> parts[0].take(2) to parts[0].drop(2)
                parts.size == 1 && parts[0].length == 6 -> parts[0].take(2) to parts[0].drop(2)
                else -> return null
            }
        val month = mm.toIntOrNull()?.takeIf { it in 1..12 } ?: return null
        return month to (year(yy) ?: return null)
    }

    /**
     * A card's brand from its number, as upstream's web vault guesses it
     * (passbundle-clients, libs/common card.view.ts,
     * `getCardBrandByPatterns`), by the names the editor offers; null when
     * none fits.
     */
    fun brand(number: String): String? =
        when {
            number.startsWith("4") -> "Visa"
            MASTERCARD.matches(number) -> "Mastercard"
            Regex("^3[47].*").matches(number) -> "Amex"
            DISCOVER.containsMatchIn(number) -> "Discover"
            Regex("^(36|30[0-5]).*").matches(number) -> "Diners Club"
            Regex("^35(2[89]|[3-8][0-9]).*").matches(number) -> "JCB"
            else -> null
        }

    private val MASTERCARD =
        Regex("^(5[1-5][0-9]{14}|2(22[1-9][0-9]{12}|2[3-9][0-9]{13}|[3-6][0-9]{14}|7[0-1][0-9]{13}|720[0-9]{12}))$")
    private val DISCOVER = Regex("^(6011|622(12[6-9]|1[3-9][0-9]|[2-8][0-9]{2}|9[0-1][0-9]|92[0-5]|64[4-9])|65)")
}
