package com.passbundle.app

import android.text.InputType

/** What a field on another app's screen is for, and so which kind of item fills it (Roles.kt). */
enum class Role(val kind: Kind) {
    USERNAME(Kind.LOGIN),
    PASSWORD(Kind.LOGIN),
    /** A one-time code: the current code of a login with a one-time code key. */
    CODE(Kind.CODE),
    CARD_NUMBER(Kind.CARD),
    CARD_NAME(Kind.CARD),
    CARD_GIVEN_NAME(Kind.CARD),
    CARD_FAMILY_NAME(Kind.CARD),
    /** The month and the year in one field ("07/29"). */
    CARD_EXPIRY(Kind.CARD),
    CARD_EXPIRY_MONTH(Kind.CARD),
    CARD_EXPIRY_YEAR(Kind.CARD),
    /** Only ever a field marked for it (Roles.kt). */
    CARD_CODE(Kind.CARD),
    FULL_NAME(Kind.IDENTITY),
    TITLE(Kind.IDENTITY),
    GIVEN_NAME(Kind.IDENTITY),
    MIDDLE_NAME(Kind.IDENTITY),
    FAMILY_NAME(Kind.IDENTITY),
    EMAIL(Kind.IDENTITY),
    PHONE(Kind.IDENTITY),
    COMPANY(Kind.IDENTITY),
    /** The whole postal address in one field. */
    ADDRESS(Kind.IDENTITY),
    /** The street lines in one field. */
    STREET(Kind.IDENTITY),
    ADDRESS_LINE1(Kind.IDENTITY),
    ADDRESS_LINE2(Kind.IDENTITY),
    ADDRESS_LINE3(Kind.IDENTITY),
    CITY(Kind.IDENTITY),
    STATE(Kind.IDENTITY),
    POSTAL_CODE(Kind.IDENTITY),
    COUNTRY(Kind.IDENTITY),
}

/** The kinds of fill: a login, a login's one-time code alone, a card, or an identity; `id` names it to FillActivity. */
enum class Kind(val id: String) {
    LOGIN("login"),
    CODE("code"),
    CARD("card"),
    IDENTITY("identity"),
    ;

    companion object {
        fun of(id: String?): Kind? = entries.firstOrNull { it.id == id }
    }
}

/**
 * What each field on another app's screen, or a browser's page, is for
 * (docs/phones.md, "Autofill"): a login's username or password, a one-time
 * code, a card's parts, or an identity's. Plain functions over plain
 * descriptions of the fields (`Field`), kept apart from Android's classes so
 * they're tested on the computer (src/test, RolesTest); Fields.kt reads the
 * descriptions from Android's.
 *
 * A field is "marked" when it says what it is: an app's autofill hints, or a
 * page's autocomplete words as a browser gives them. Otherwise its words
 * (its resource name, placeholder, and a page's id, name, and label) can
 * say, as whole words. A card's or an identity's value goes only where
 * PassBundle is sure:
 * - a password field never gets one, and nor does a field marked as
 *   something else;
 * - a card's security code goes only into a field marked for it;
 * - a card's other fields by their words count only on a screen with two
 *   or more card fields, one of them the number; an identity's, with two or
 *   more identity fields. A marked card field is enough by itself.
 * - words that fit two of an identity's parts ("City, state", "Phone or
 *   email") fit none.
 * Logins are found as before cards and identities were: a field that's both
 * a login's and an identity's (an email) is offered to each.
 */
object Roles {
    /** One field, as plain values: what Fields.kt reads from Android's description, or what a test makes up. */
    data class Field<T>(
        val id: T,
        /** The app's autofill hints, or the words of a page's `autocomplete` as a browser gives them. */
        val hints: List<String> = emptyList(),
        /** A page's attributes as a browser gives them (Chrome: name, type, label, id), names lowercase. */
        val html: Map<String, String> = emptyMap(),
        /** Android's input type for it. */
        val inputType: Int = 0,
        /** Its resource name in an app. */
        val idEntry: String? = null,
        /** What it shows while empty: an app's hint, or a page's placeholder. */
        val hint: String? = null,
        /** The most characters it takes, when it says. */
        val maxLength: Int? = null,
        /** A list's options (a spinner, or a page's select), as shown; null for a text field. */
        val options: List<String>? = null,
    )

    /** A field PassBundle fills, what it's for, and how it takes the value. */
    data class Found<T>(
        val id: T,
        val role: Role,
        /** How it writes a card's expiry, month, or year: "MM/YY", "MM / YYYY", "MMYY", "YY", "YYYY" (Values.kt). */
        val format: String? = null,
        /** A list's options, for a card's expiry month or year, or an address's state or country, in a list. */
        val options: List<String>? = null,
    )

    /** What PassBundle fills on a screen: each field it's sure about, and what for. */
    class Screen<T>(val found: List<Found<T>>) {
        val username: T? get() = found.firstOrNull { it.role == Role.USERNAME }?.id
        val password: T? get() = found.firstOrNull { it.role == Role.PASSWORD }?.id

        /** The first field that asks for a one-time code. */
        val code: T? get() = found.firstOrNull { it.role == Role.CODE }?.id

        /** The fields a kind of item fills. */
        fun of(kind: Kind): List<Found<T>> = found.filter { it.role.kind == kind }

        /** Every field PassBundle might fill here, once each. */
        val ids: List<T> get() = found.map { it.id }.distinct()
    }

    private val PASSWORD_VARIATIONS =
        setOf(
            InputType.TYPE_TEXT_VARIATION_PASSWORD,
            InputType.TYPE_TEXT_VARIATION_WEB_PASSWORD,
            InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD,
        )
    private val USERNAME_WORDS = listOf("user", "email", "e-mail", "login", "account")

    /** What each field on the screen is for. */
    fun <T> screen(fields: List<Field<T>>): Screen<T> {
        var username: T? = null
        var password: T? = null
        // The text field just before the password, for a form that says nothing about its username.
        var textBeforePassword: T? = null
        var lastText: T? = null
        var code: T? = null
        // Each card field, and whether it's marked.
        val cards = mutableListOf<Pair<Found<T>, Boolean>>()
        val identities = mutableListOf<Found<T>>()
        // Fields whose words say they're a card's security code: nothing fills them unmarked, but they're a card's.
        var cardCodeWords = 0

        for (field in fields) {
            val seen = Seen(field)
            if (field.options != null) {
                // A list: a card's expiry month or year, or an address's state
                // or country, chosen by its options (Values.option).
                val role = seen.cardMark ?: seen.cardWords.takeIf { !seen.marked }
                if (role == Role.CARD_EXPIRY_MONTH || role == Role.CARD_EXPIRY_YEAR) {
                    cards += Found(field.id, role, options = field.options) to (seen.cardMark != null)
                } else {
                    val place = seen.identityMark ?: seen.identityWords.takeIf { !seen.marked }
                    if (place == Role.STATE || place == Role.COUNTRY) identities += Found(field.id, place, options = field.options)
                }
                continue
            }
            // Marked as a password, a one-time code, or a card's or an identity's part (other than an email or a phone).
            if (seen.passwordMarked) {
                if (password == null) {
                    password = field.id
                    textBeforePassword = lastText
                }
                continue
            }
            if (seen.markRole == Role.CODE) {
                if (code == null) code = field.id
                continue
            }
            val cardMark = seen.cardMark
            if (cardMark != null) {
                cards += Found(field.id, cardMark, seen.format(cardMark)) to true
                continue
            }
            val identityMark = seen.identityMark
            if (identityMark != null && identityMark != Role.EMAIL && identityMark != Role.PHONE) {
                identities += Found(field.id, identityMark)
                continue
            }
            // A marked email or phone is an identity's, and may be a login's username too.
            identityMark?.let { identities += Found(field.id, it) }
            when {
                // A field the page calls a one-time code, even one masked like a password.
                seen.codeWords -> if (code == null) code = field.id
                // A masked card security code is neither a password nor a one-time code.
                seen.typedPassword && seen.cardWords == Role.CARD_CODE -> cardCodeWords++
                seen.typedPassword -> if (password == null) {
                    password = field.id
                    textBeforePassword = lastText
                }
                seen.isUsername -> {
                    if (username == null) username = field.id
                    if (!seen.marked) {
                        seen.identityWords?.takeIf { it == Role.EMAIL || it == Role.PHONE }?.let { identities += Found(field.id, it) }
                    }
                }
                else -> {
                    val card = seen.cardWords.takeIf { !seen.marked }
                    when {
                        card == Role.CARD_CODE -> cardCodeWords++
                        card != null -> cards += Found(field.id, card, seen.format(card)) to false
                        !seen.marked -> seen.identityWords?.let { identities += Found(field.id, it) }
                    }
                    if (card != Role.CARD_CODE) lastText = field.id
                }
            }
        }

        // A card's fields by their words only beside more card fields, one of them the number; a marked one is enough.
        val sureOfCards =
            cards.any { it.second } ||
                (cards.size + cardCodeWords >= 2 && cards.any { it.first.role == Role.CARD_NUMBER })
        val cardFields = if (sureOfCards) cards.map { it.first } else emptyList()
        val identityFields = if (identities.map { it.id }.distinct().size >= 2) identities else emptyList()
        // A field PassBundle is sure is a card's or an identity's isn't a username just for being before the password.
        val theirs = (cardFields + identityFields).map { it.id }.toSet()
        val logins =
            listOfNotNull<Found<T>>(
                (username ?: textBeforePassword?.takeIf { it !in theirs })?.let { Found(it, Role.USERNAME) },
                password?.let { Found(it, Role.PASSWORD) },
                code?.let { Found(it, Role.CODE) },
            )
        return Screen(logins + cardFields + identityFields)
    }

    /** What one field says about itself. */
    private class Seen<T>(val field: Field<T>) {
        private val hints = field.hints.map { it.lowercase() }
        private val html = field.html.mapValues { it.value.lowercase() }

        /** The autofill names it's marked with: the app's hints, and the page's autocomplete words. */
        private val marks: List<String> =
            (field.hints + listOfNotNull(field.html["autocomplete"]))
                .flatMap { it.lowercase().trim().split(SPACES) }
                .filter { it.isNotEmpty() && it !in MARK_PREFIXES && !it.startsWith("section-") }

        val passwordMarked = hints.any { "password" in it } || marks.any { "password" in it }

        /** What its marks say it is, of what PassBundle fills. */
        val markRole: Role? = marks.firstNotNullOfOrNull { MARKS[it] }

        /** Marked as anything: what PassBundle fills, or something else, which is never guessed. */
        val marked = markRole != null || marks.any { it in OTHER_MARKS }

        val cardMark: Role? = markRole?.takeIf { it.kind == Kind.CARD }
        val identityMark: Role? = markRole?.takeIf { it.kind == Kind.IDENTITY }

        private val textClass = (field.inputType and InputType.TYPE_MASK_CLASS) == InputType.TYPE_CLASS_TEXT
        private val variation = field.inputType and InputType.TYPE_MASK_VARIATION

        val typedPassword = html["type"] == "password" || (textClass && variation in PASSWORD_VARIATIONS)

        // As logins were found before cards and identities: by parts of the field's own names.
        private val loginWords = listOfNotNull(field.idEntry, field.hint, html["name"], html["id"]).joinToString(" ").lowercase()
        val isUsername =
            hints.any { it == "username" || "email" in it } || html["type"] == "email" ||
                html["autocomplete"] == "username" || USERNAME_WORDS.any { it in loginWords }

        /** Its words, a list for each place they come from. */
        private val sources: List<List<String>> =
            listOfNotNull(
                field.idEntry,
                field.hint,
                field.html["name"],
                field.html["id"],
                field.html["label"],
                field.html["placeholder"],
                field.html["aria-label"],
            ).map(::words).filter { it.isNotEmpty() }

        private fun says(phrases: List<String>) = sources.any { words -> phrases.any { has(words, it) } }

        /** Its words call it a one-time code, and nothing says it's a password or a PIN. */
        val codeWords: Boolean by lazy(LazyThreadSafetyMode.NONE) {
            !marked && (says(CODE_EVEN_WITH_SECRET_WORDS) || (says(CODE_PHRASES) && !says(SECRET_WORDS) && !says(NOT_CODES)))
        }

        /** The card part its words say, if any. */
        val cardWords: Role? by lazy(LazyThreadSafetyMode.NONE) { if (says(NOT_CARDS)) null else cardRole() }

        private fun cardRole(): Role? {
            val month = says(MONTH_WORDS)
            val year = says(YEAR_WORDS)
            val expiring = says(EXPIRY_WORDS) || says(CARD_WORDS)
            return when {
                says(NOT_EXPIRY) && (month || year || says(EXPIRY_WORDS)) -> null
                month && year -> Role.CARD_EXPIRY
                month && expiring -> Role.CARD_EXPIRY_MONTH
                year && expiring -> Role.CARD_EXPIRY_YEAR
                says(EXPIRY_PHRASES) -> Role.CARD_EXPIRY
                says(CARD_CODE_PHRASES) -> Role.CARD_CODE
                says(CARD_NAME_PHRASES) -> Role.CARD_NAME
                says(CARD_NUMBER_PHRASES) -> Role.CARD_NUMBER
                else -> null
            }
        }

        /** The identity part its words say, when they say only one; else, its type (an email or a phone). */
        val identityWords: Role? by lazy(LazyThreadSafetyMode.NONE) {
            run {
                if (says(NOT_IDENTITIES)) return@run null
                val found =
                    IDENTITY_WORDS.mapNotNull { group ->
                        if (says(group.not)) null else group.roles.firstNotNullOfOrNull { (role, phrases) -> role.takeIf { says(phrases) } }
                    }.toMutableSet()
                // "Name" by itself is the person's whole name.
                if (sources.any { it == listOf("name") }) found += Role.FULL_NAME
                if (found.isEmpty()) {
                    if (html["type"] == "email" || (textClass && variation == InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS)) found += Role.EMAIL
                    if (html["type"] == "tel" || (field.inputType and InputType.TYPE_MASK_CLASS) == InputType.TYPE_CLASS_PHONE) found += Role.PHONE
                }
                found.singleOrNull()
            }
        }

        /** How a card's expiry or year field writes it, from what it shows and how long it is (Values.kt). */
        fun format(role: Role): String? {
            val shown = listOfNotNull(field.hint, field.html["placeholder"], field.html["label"]).joinToString(" ")
            val length = field.maxLength ?: field.html["maxlength"]?.trim()?.toIntOrNull()
            return when (role) {
                Role.CARD_EXPIRY -> Values.expiryFormat(shown, length)
                Role.CARD_EXPIRY_YEAR -> Values.yearFormat(shown, length)
                else -> null
            }
        }
    }

    /**
     * Whether what was typed into a sign-in's password field looks like a
     * one-time code rather than a password: 4 to 8 digits with no username
     * (a code field that says nothing about itself, masked like a
     * password). Android doesn't offer to save it. A username with a
     * numeric PIN still saves.
     */
    fun looksLikeACode(username: String?, password: String): Boolean =
        username.isNullOrBlank() && password.length in 4..8 && password.all { it in '0'..'9' }

    /** A field's words: split at anything but letters and digits, and where camel case starts a word ("cardNumber"), lowercase. */
    fun words(text: String): List<String> =
        text.replace(CAMEL, "$1 $2")
            .lowercase()
            .split(NOT_WORDS)
            .filter { it.isNotEmpty() }

    private val SPACES = Regex("\\s+")
    private val CAMEL = Regex("(\\p{Ll})(\\p{Lu})")
    private val NOT_WORDS = Regex("[^\\p{L}\\p{N}]+")

    /**
     * Whether a phrase ("card number") is among the words, as whole words:
     * its words in a row, or run together in the field's own ("cardnumber",
     * "cc-number", "addressLine1"), never inside a longer word ("cid" isn't
     * in "cidade").
     */
    fun has(words: List<String>, phrase: String): Boolean {
        val wanted = phrase.replace(" ", "")
        for (start in words.indices) {
            val run = StringBuilder()
            for (end in start until words.size) {
                run.append(words[end])
                if (run.length > wanted.length) break
                if (run.length == wanted.length) {
                    if (run.toString() == wanted) return true
                    break
                }
            }
        }
        return false
    }

    /** Words before a field's name in an autocomplete attribute: a section, shipping or billing, a kind of contact. */
    private val MARK_PREFIXES = setOf("shipping", "billing", "home", "work", "mobile", "fax", "pager", "webauthn", "on", "off")

    /** The marks PassBundle fills, the HTML standard's autocomplete names and Android's autofill hints (lowercase). */
    private val MARKS: Map<String, Role> =
        mapOf(
            "one-time-code" to Role.CODE,
            "smsotpcode" to Role.CODE,
            "emailotpcode" to Role.CODE,
            "2faappotpcode" to Role.CODE,
            "cc-number" to Role.CARD_NUMBER,
            "creditcardnumber" to Role.CARD_NUMBER,
            "cc-name" to Role.CARD_NAME,
            "cc-given-name" to Role.CARD_GIVEN_NAME,
            "cc-family-name" to Role.CARD_FAMILY_NAME,
            "cc-exp" to Role.CARD_EXPIRY,
            "creditcardexpirationdate" to Role.CARD_EXPIRY,
            "cc-exp-month" to Role.CARD_EXPIRY_MONTH,
            "creditcardexpirationmonth" to Role.CARD_EXPIRY_MONTH,
            "cc-exp-year" to Role.CARD_EXPIRY_YEAR,
            "creditcardexpirationyear" to Role.CARD_EXPIRY_YEAR,
            "cc-csc" to Role.CARD_CODE,
            "creditcardsecuritycode" to Role.CARD_CODE,
            "name" to Role.FULL_NAME,
            "personname" to Role.FULL_NAME,
            "honorific-prefix" to Role.TITLE,
            "personnameprefix" to Role.TITLE,
            "given-name" to Role.GIVEN_NAME,
            "persongivenname" to Role.GIVEN_NAME,
            "additional-name" to Role.MIDDLE_NAME,
            "personmiddlename" to Role.MIDDLE_NAME,
            "family-name" to Role.FAMILY_NAME,
            "personfamilyname" to Role.FAMILY_NAME,
            "email" to Role.EMAIL,
            "emailaddress" to Role.EMAIL,
            "tel" to Role.PHONE,
            "phone" to Role.PHONE,
            "phonenumber" to Role.PHONE,
            "phonenumberdevice" to Role.PHONE,
            "organization" to Role.COMPANY,
            "postaladdress" to Role.ADDRESS,
            "street-address" to Role.STREET,
            "streetaddress" to Role.STREET,
            "address-line1" to Role.ADDRESS_LINE1,
            "address-line2" to Role.ADDRESS_LINE2,
            "extendedaddress" to Role.ADDRESS_LINE2,
            "address-line3" to Role.ADDRESS_LINE3,
            "address-level2" to Role.CITY,
            "addresslocality" to Role.CITY,
            "address-level1" to Role.STATE,
            "addressregion" to Role.STATE,
            "postal-code" to Role.POSTAL_CODE,
            "postalcode" to Role.POSTAL_CODE,
            "country" to Role.COUNTRY,
            "country-name" to Role.COUNTRY,
            "addresscountry" to Role.COUNTRY,
        )

    /** Marks that say a field is something PassBundle doesn't fill: it's left alone, never guessed. */
    private val OTHER_MARKS =
        setOf(
            "nickname", "honorific-suffix", "personnamesuffix", "personmiddleinitial", "organization-title",
            "address-level3", "address-level4", "dependentlocality", "aptnumber", "extendedpostalcode",
            "cc-additional-name", "cc-type", "creditcardexpirationday", "transaction-currency", "transaction-amount",
            "language", "bday", "bday-day", "bday-month", "bday-year", "birthdatefull", "birthdateday",
            "birthdatemonth", "birthdateyear", "sex", "gender", "url", "photo", "impp", "tel-country-code",
            "tel-national", "tel-area-code", "tel-local", "tel-local-prefix", "tel-local-suffix", "tel-extension",
            "phonecountrycode", "phonenational", "promocode", "giftcardnumber", "giftcardpin", "upivirtualpaymentaddress",
            "notapplicable", "username", "newusername", "password", "newpassword",
        )

    // Words, as whole words (`has`): mostly English, with a few common others.

    /** A one-time code. */
    private val CODE_PHRASES =
        listOf(
            "otp", "totp", "otc", "2fa", "mfa", "one time code", "otp code", "verification code", "verify code",
            "authentication code", "auth code", "authenticator", "two factor", "2 factor", "two step", "2 step",
            "second factor", "login code", "sign in code", "signin code", "sms code", "digit code",
        )

    /** A one-time code, though its words say password. */
    private val CODE_EVEN_WITH_SECRET_WORDS = listOf("one time password", "one time passcode", "one time pin")

    /** A password or a PIN, which stays one. */
    private val SECRET_WORDS =
        listOf("password", "passwd", "pwd", "passcode", "pin", "pincode", "passwort", "kennwort", "contraseña", "senha", "wachtwoord")

    /** Codes that aren't a login's one-time code. */
    private val NOT_CODES =
        listOf(
            "backup", "recovery", "promo", "coupon", "voucher", "discount", "invite", "invitation", "referral",
            "gift", "zip", "postal", "country", "area",
        )

    /** Never a card's. */
    private val NOT_CARDS =
        listOf(
            "gift", "giftcard", "loyalty", "reward", "rewards", "member", "membership", "library", "promo", "coupon",
            "voucher", "discount", "club", "student", "insurance", "health", "sim", "id", "national", "identity",
            "passport", "license", "licence", "bank", "account", "iban", "routing", "phone", "mobile", "tracking",
            "order", "invoice", "reference", "ssn", "search",
        )

    /** Not a card's expiry: a start date, a birth date. */
    private val NOT_EXPIRY = listOf("from", "start", "issue", "issued", "birth", "dob", "bday", "birthday")

    private val MONTH_WORDS = listOf("month", "mm", "mo", "mon", "monat", "mes", "mois")
    private val YEAR_WORDS = listOf("year", "yy", "yyyy", "yr", "jahr", "ano", "año", "annee", "année")
    private val EXPIRY_WORDS = listOf("exp", "expiry", "expiration", "expire", "expires", "expiring", "valid", "ccexp", "expdate")
    private val CARD_WORDS = listOf("card", "cc", "credit card", "debit card")
    private val EXPIRY_PHRASES =
        listOf(
            "expiration date", "expiry date", "exp date", "expiration", "expiry", "expires", "valid thru",
            "valid through", "good thru", "mm yy", "mm yyyy", "cc exp", "card expiry", "card expiration",
        )
    private val CARD_CODE_PHRASES =
        listOf("cvc", "cvv", "cvv2", "cvc2", "csc", "cvn", "cvd", "security code", "card code", "card verification")
    private val CARD_NAME_PHRASES =
        listOf("name on card", "name on the card", "cardholder", "card holder", "holder name", "card name", "cc name", "card owner")
    private val CARD_NUMBER_PHRASES =
        listOf(
            "card number", "card num", "card no", "cc number", "cc num", "cc no", "credit card", "debit card",
            "card pan", "kartennummer", "numero de tarjeta", "número de tarjeta", "numéro de carte", "numero de carte",
            "numero carte", "número do cartão", "numero do cartao",
        )

    /** Never an identity's: someone else's, a date, a search box, a card's. */
    private val NOT_IDENTITIES =
        listOf(
            "birth", "dob", "bday", "birthday", "maiden", "mother", "father", "parent", "emergency", "guardian",
            "spouse", "referral", "referrer", "friend", "search", "query", "find", "card", "cardholder", "promo",
            "coupon",
        )

    /** Some of an identity's parts, by words: the first that fits wins, unless a word in `not` is there. */
    private class Group(val roles: List<Pair<Role, List<String>>>, val not: List<String> = emptyList())

    /**
     * An identity's parts by their words. Within a group the first that fits
     * wins ("first and last name" is a whole name); words that fit two
     * groups ("City, state") fit none.
     */
    private val IDENTITY_WORDS: List<Group> =
        listOf(
            Group(
                listOf(
                    Role.FULL_NAME to listOf("full name", "fullname", "your name", "first and last name", "contact name"),
                    Role.GIVEN_NAME to listOf("first name", "firstname", "fname", "given name", "forename", "vorname", "prenom", "prénom"),
                    Role.MIDDLE_NAME to listOf("middle name", "middlename"),
                    Role.FAMILY_NAME to listOf("last name", "lastname", "lname", "surname", "family name", "nachname", "apellido", "apellidos"),
                ),
                not =
                    listOf(
                        "user", "username", "account", "login", "company", "business", "organization", "file", "device",
                        "display", "screen", "nick", "nickname", "list", "folder", "item", "pet", "school", "bank",
                        "product", "project", "team", "group", "event", "store", "shop", "site", "domain", "host",
                        "server", "network", "wifi", "app",
                    ),
            ),
            Group(listOf(Role.EMAIL to listOf("email", "e mail", "correo", "courriel"))),
            Group(
                listOf(Role.PHONE to listOf("phone", "telephone", "tel", "mobile", "cell", "cellphone", "telefon", "telefono", "teléfono", "téléphone")),
                not = listOf("country code", "area code", "extension", "ext", "prefix", "fax", "code"),
            ),
            Group(
                listOf(Role.COMPANY to listOf("company", "organization", "organisation", "business name", "firma", "empresa", "entreprise", "employer")),
                not = listOf("id", "code", "url", "domain", "number", "size", "type", "website", "unit"),
            ),
            Group(
                listOf(
                    Role.ADDRESS_LINE3 to listOf("address 3", "address line 3", "addr 3", "street 3"),
                    Role.ADDRESS_LINE2 to listOf("address 2", "address line 2", "addr 2", "street 2", "apartment", "apt", "suite"),
                    Role.ADDRESS_LINE1 to listOf("address 1", "address line 1", "addr 1", "street 1", "street address", "street", "address line", "address"),
                ),
                not =
                    listOf(
                        "email", "e mail", "ip", "web", "url", "website", "wallet", "mac", "server", "host", "bitcoin", "btc",
                        "crypto", "domain", "network", "ipv4", "ipv6",
                    ),
            ),
            Group(listOf(Role.CITY to listOf("city", "town", "locality", "stadt", "ville", "ciudad", "cidade"))),
            Group(listOf(Role.STATE to listOf("state", "province", "bundesland", "provincia"))),
            Group(
                listOf(
                    Role.POSTAL_CODE to
                        listOf(
                            "zip", "zipcode", "zip code", "postcode", "post code", "postal code", "postal", "plz",
                            "postleitzahl", "cep", "code postal", "codigo postal", "código postal",
                        ),
                ),
            ),
            Group(
                listOf(Role.COUNTRY to listOf("country", "pais", "país", "pays")),
                not = listOf("code", "citizenship", "nationality", "issuing", "calling"),
            ),
        )
}
