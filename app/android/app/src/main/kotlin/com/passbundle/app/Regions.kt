package com.passbundle.app

import java.text.Normalizer
import java.util.Locale

/**
 * An identity's state or country chosen from a list (a spinner, or a page's
 * select), by its name or its abbreviation, as a card's expiry month and year
 * are (Values.kt; docs/phones.md, "Autofill"): "IL" chooses "Illinois",
 * "Illinois" chooses "IL", and either chooses "IL - Illinois" or "Illinois
 * (IL)". States are the United States' (with DC and the territories) and
 * Canada's provinces and territories; countries are every one Java knows, by
 * their two- and three-letter codes and their names in English and in the
 * phone's language. Anything else fits only by the same words. Plain
 * functions, tested on the computer (src/test, ValuesTest).
 */
object Regions {
    /**
     * The option in a list for a state or a country (`value`, as the
     * identity has it), with the other names it goes by (`names`): the one
     * whose text is one of them, else the one with one of them beside
     * another ("IL - Illinois"). Null when none fits, or more than one, so
     * the list is left as it is.
     */
    fun option(options: List<String>, value: String?, names: (String) -> Set<String>): Int? {
        val given = value?.trim()?.ifEmpty { null } ?: return null
        val wanted = (names(given) + given).map(::key).filter { it.isNotEmpty() }.toSet()
        fun only(fits: (String) -> Boolean) = options.indices.filter { fits(options[it]) }.singleOrNull()
        return only { key(it) in wanted } ?: only { option -> option.split(PARTS).any { key(it) in wanted } }
    }

    /** Between a name and its abbreviation in one option: a dash or a bar with spaces round it, or brackets. */
    private val PARTS = Regex("\\s+[-\u2013\u2014|/]\\s+|[()\\[\\]]")

    /** A name as it's compared: lowercase, without accents, its words with one space between. */
    fun key(text: String): String =
        Normalizer.normalize(text, Normalizer.Form.NFD)
            .replace(Regex("\\p{M}+"), "")
            .lowercase(Locale.ROOT)
            .split(Regex("[^\\p{L}\\p{N}]+"))
            .filter { it.isNotEmpty() }
            .joinToString(" ")

    /** A state's or a province's abbreviation and name, from either; just itself when it's neither. */
    fun state(value: String): Set<String> {
        val wanted = key(value)
        return STATES.entries.firstOrNull { key(it.key) == wanted || key(it.value) == wanted }?.let { setOf(it.key, it.value) } ?: setOf(value)
    }

    /** A country's codes and names, from any of them; just itself when it's none. */
    fun country(value: String): Set<String> {
        val wanted = key(value)
        for (code in Locale.getISOCountries()) {
            @Suppress("DEPRECATION")
            val place = Locale("", code)
            val names =
                setOfNotNull(
                    code,
                    runCatching { place.isO3Country }.getOrNull(),
                    place.getDisplayCountry(Locale.ENGLISH),
                    place.displayCountry,
                ) + COUNTRIES[code].orEmpty()
            if (names.any { key(it) == wanted }) return names
        }
        return setOf(value)
    }

    /** What lists also call a country. */
    private val COUNTRIES =
        mapOf(
            "US" to setOf("United States of America", "U.S.", "U.S.A."),
            "GB" to setOf("UK", "U.K.", "Great Britain"),
        )

    /** The United States' states, DC, and territories, and Canada's provinces and territories: abbreviation to name. */
    private val STATES =
        mapOf(
            "AL" to "Alabama", "AK" to "Alaska", "AZ" to "Arizona", "AR" to "Arkansas", "CA" to "California",
            "CO" to "Colorado", "CT" to "Connecticut", "DE" to "Delaware", "DC" to "District of Columbia",
            "FL" to "Florida", "GA" to "Georgia", "HI" to "Hawaii", "ID" to "Idaho", "IL" to "Illinois",
            "IN" to "Indiana", "IA" to "Iowa", "KS" to "Kansas", "KY" to "Kentucky", "LA" to "Louisiana",
            "ME" to "Maine", "MD" to "Maryland", "MA" to "Massachusetts", "MI" to "Michigan", "MN" to "Minnesota",
            "MS" to "Mississippi", "MO" to "Missouri", "MT" to "Montana", "NE" to "Nebraska", "NV" to "Nevada",
            "NH" to "New Hampshire", "NJ" to "New Jersey", "NM" to "New Mexico", "NY" to "New York",
            "NC" to "North Carolina", "ND" to "North Dakota", "OH" to "Ohio", "OK" to "Oklahoma", "OR" to "Oregon",
            "PA" to "Pennsylvania", "RI" to "Rhode Island", "SC" to "South Carolina", "SD" to "South Dakota",
            "TN" to "Tennessee", "TX" to "Texas", "UT" to "Utah", "VT" to "Vermont", "VA" to "Virginia",
            "WA" to "Washington", "WV" to "West Virginia", "WI" to "Wisconsin", "WY" to "Wyoming",
            "AS" to "American Samoa", "GU" to "Guam", "MP" to "Northern Mariana Islands", "PR" to "Puerto Rico",
            "VI" to "U.S. Virgin Islands",
            "AB" to "Alberta", "BC" to "British Columbia", "MB" to "Manitoba", "NB" to "New Brunswick",
            "NL" to "Newfoundland and Labrador", "NS" to "Nova Scotia", "NT" to "Northwest Territories",
            "NU" to "Nunavut", "ON" to "Ontario", "PE" to "Prince Edward Island", "QC" to "Québec",
            "SK" to "Saskatchewan", "YT" to "Yukon",
        )
}
