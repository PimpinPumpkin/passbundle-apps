package com.passbundle.app

import android.text.InputType
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

// What Android offers to save from a screen, and what's saved (Saving.kt),
// on made-up pages and made-up cards (public test numbers) and addresses: a
// sign-in as before; a card with its expiry or its name; an address with two
// of a name, a street, a city, and a postal code. Never a card or an address
// the person has, as Android's own check of what was typed decides
// (`Check.holds` reads it as Android does), never a kind no account may make,
// and never a security code from a field that isn't marked for one.
class SavingTest {
    private val password = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD

    /** A field on a page, as Chrome describes it: `autocomplete` as hints, the rest as attributes. */
    private fun page(
        id: String,
        autocomplete: String? = null,
        type: String = "text",
        label: String? = null,
        options: List<String>? = null,
    ) = Roles.Field(
        id,
        hints = autocomplete?.split(" ").orEmpty(),
        html = listOfNotNull("type" to type, "id" to id, "name" to id, label?.let { "label" to it }).toMap(),
        inputType = if (type == "password") password else InputType.TYPE_CLASS_TEXT,
        options = options,
    )

    private fun screen(vararg fields: Roles.Field<String>) = Roles.screen(fields.toList())

    private val MONTHS =
        listOf("January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December")

    private val payment =
        screen(
            page("number", "cc-number"),
            page("exp", "cc-exp"),
            page("holder", "cc-name"),
            page("csc", "cc-csc"),
        )

    private val shipping =
        screen(
            page("given", "shipping given-name"),
            page("family", "shipping family-name"),
            page("line1", "shipping address-line1"),
            page("line2", "shipping address-line2"),
            page("city", "shipping address-level2"),
            page("state", "shipping address-level1"),
            page("zip", "shipping postal-code"),
            page("email", "email", type = "email"),
        )

    /** Android refuses an "and" or an "or" of nothing: no check here has one. */
    private fun <T> noneEmpty(check: Saving.Check<T>?) {
        when (check) {
            null, is Saving.Check.Matches, is Saving.Check.Luhn -> Unit
            is Saving.Check.Not -> noneEmpty(check.check)
            is Saving.Check.All -> {
                assertTrue(check.checks.isNotEmpty())
                check.checks.forEach { noneEmpty(it) }
            }
            is Saving.Check.AnyOf -> {
                assertTrue(check.checks.isNotEmpty())
                check.checks.forEach { noneEmpty(it) }
            }
        }
    }

    private fun offer(screen: Roles.Screen<String>, known: Saving.Known? = null) =
        Saving.offer(screen, known)?.also { noneEmpty(it.check) }

    /** Whether Android would ask, with these values typed. */
    private fun asks(offer: Saving.Offer<String>?, vararg typed: Pair<String, String>): Boolean {
        val values = typed.toMap()
        return offer != null && offer.check?.holds { values[it] } ?: true
    }

    @Test fun `a sign-in's save is as before, its password with its username`() {
        val signIn = screen(page("user", "username"), page("pass", "current-password", type = "password"))
        val save = offer(signIn)!!
        assertEquals(Saving.What.LOGIN, save.what)
        assertEquals(listOf("pass"), save.required)
        assertEquals(listOf("user"), save.optional)
        assertNull(save.check)
        // A password on the screen makes it a sign-in's, whatever else is there.
        val both = screen(page("user", "username"), page("pass", "current-password", type = "password"), page("number", "cc-number"), page("exp", "cc-exp"))
        assertEquals(Saving.What.LOGIN, offer(both)!!.what)
    }

    @Test fun `a payment form offers to save a card, its number with its expiry or its name`() {
        val save = offer(payment)!!
        assertEquals(Saving.What.CARD, save.what)
        assertEquals(listOf("number"), save.required)
        assertEquals(listOf("exp", "holder", "csc"), save.optional)
        assertTrue(asks(save, "number" to "4242 4242 4242 4242", "exp" to "12/30"))
        assertTrue(asks(save, "number" to "4242-4242-4242-4242", "holder" to "Ana López"))
        // Not a card number: too short, too long, or not passing Luhn's checksum.
        assertFalse(asks(save, "number" to "4242", "exp" to "12/30"))
        assertFalse(asks(save, "number" to "4242 4242 4242 4242 4242", "exp" to "12/30"))
        assertFalse(asks(save, "number" to "4242 4242 4242 4241", "exp" to "12/30"))
        // Nor a number alone.
        assertFalse(asks(save, "number" to "4242424242424242"))
        assertFalse(asks(save, "number" to "4242424242424242", "exp" to "", "holder" to " "))
        // Locked, nothing more is known: offered, and checked once unlocked.
        assertTrue(asks(offer(payment, null), "number" to "4111 1111 1111 1111", "holder" to "Ana López"))
    }

    @Test fun `a card the person has, by its last 4 digits, isn't offered`() {
        val save = offer(payment, Saving.Known(last4 = listOf("1111", "4444")))!!
        assertFalse(asks(save, "number" to "4111 1111 1111 1111", "exp" to "07/29"))
        assertFalse(asks(save, "number" to "5555-5555-5555-4444", "exp" to "07/29"))
        assertTrue(asks(save, "number" to "4242 4242 4242 4242", "exp" to "07/29"))
        // The check holds no whole number: only those last digits.
        assertFalse(save.check.toString().contains("4111111111111111"))
    }

    @Test fun `a number alone isn't a payment form, nor is a gift card's`() {
        assertNull(offer(screen(page("number", "cc-number"))))
        assertNull(offer(screen(page("gift", label = "Gift card number"), page("pin", label = "Expiry"))))
    }

    @Test fun `a payment form's lists, a month chosen or a year`() {
        val months = listOf("Month") + MONTHS.mapIndexed { i, m -> "%02d - %s".format(i + 1, m) }
        val years = listOf("Year", "2029", "2030")
        val lists =
            screen(
                page("number", "cc-number"),
                page("month", "cc-exp-month", options = months),
                page("year", "cc-exp-year", options = years),
            )
        val save = offer(lists)!!
        assertEquals(Saving.What.CARD, save.what)
        assertTrue(asks(save, "number" to "4242424242424242", "month" to "07 - July", "year" to "Year"))
        assertTrue(asks(save, "number" to "4242424242424242", "month" to "Month", "year" to "2029"))
        assertFalse(asks(save, "number" to "4242424242424242", "month" to "Month", "year" to "Year"))
    }

    @Test fun `where no unlocked account may make a card, a payment form's address is offered instead, or nothing`() {
        val known = Saving.Known(mayAddCard = false)
        assertNull(offer(payment, known))
        val checkout =
            screen(
                page("number", "cc-number"),
                page("exp", "cc-exp"),
                page("given", "billing given-name"),
                page("family", "billing family-name"),
                page("zip", "billing postal-code"),
            )
        assertEquals(Saving.What.CARD, offer(checkout)!!.what)
        assertEquals(Saving.What.ADDRESS, offer(checkout, known)!!.what)
        assertNull(offer(checkout, Saving.Known(mayAddCard = false, mayAddAddress = false)))
    }

    @Test fun `an address form offers to save an address with two of its name, street, city, and postal code`() {
        val save = offer(shipping)!!
        assertEquals(Saving.What.ADDRESS, save.what)
        assertTrue(save.required.isEmpty())
        assertEquals(listOf("given", "family", "line1", "line2", "city", "state", "zip", "email"), save.optional)
        assertTrue(asks(save, "given" to "Ana", "line1" to "1 Example Street"))
        assertTrue(asks(save, "city" to "Springfield", "zip" to "62701"))
        assertTrue(asks(save, "family" to "López", "zip" to "62701", "email" to "ana@example.test"))
        // One of them isn't enough, with whatever else.
        assertFalse(asks(save, "given" to "Ana", "family" to "López", "email" to "ana@example.test", "state" to "IL"))
        assertFalse(asks(save, "line1" to "1 Example Street", "line2" to "Apt 2"))
        assertFalse(asks(save, "given" to " ", "zip" to "62701"))
    }

    @Test fun `a form with a name and an email, but no address, isn't an address's`() {
        assertNull(offer(screen(page("name", "name"), page("email", "email", type = "email"), page("phone", "tel", type = "tel"))))
    }

    @Test fun `an address an identity has, by its street and postal code, isn't offered`() {
        val known = Saving.Known(addresses = listOf(listOf("1", "example", "street") to "62701", listOf("10", "rue", "de", "l", "église") to "75001"))
        val save = offer(shipping, known)!!
        assertFalse(asks(save, "given" to "Ana", "line1" to "1 Example Street", "zip" to "62701"))
        assertFalse(asks(save, "given" to "Ana", "line1" to "  1 example street. ", "zip" to " 62701"))
        assertFalse(asks(save, "line1" to "10, Rue de l'Église", "zip" to "75001", "city" to "Paris"))
        // Another street, or another postal code, is a new address.
        assertTrue(asks(save, "given" to "Ana", "line1" to "1 Example Street West", "zip" to "62701"))
        assertTrue(asks(save, "given" to "Ana", "line1" to "1 Example Street", "zip" to "62702"))
        assertTrue(asks(save, "given" to "Ana", "line1" to "2 Example Street", "zip" to "62701"))
        // A street in one field, by its first line.
        val street = screen(page("name", "name"), page("street", "street-address"), page("zip", "postal-code"))
        val inOne = offer(street, known)!!
        assertFalse(asks(inOne, "name" to "Ana López", "street" to "1 Example Street\nApt 2", "zip" to "62701"))
        assertTrue(asks(inOne, "name" to "Ana López", "street" to "Apt 2\n1 Example Street", "zip" to "62701"))
    }

    @Test fun `the checks' patterns read as the core compares`() {
        assertTrue(Regex(Saving.endsWithAny(listOf("4242"))).matches("4000 0000 0000 4242"))
        assertTrue(Regex(Saving.endsWithAny(listOf("4242"))).matches("4000-0000-0000-4242 "))
        assertFalse(Regex(Saving.endsWithAny(listOf("4242"))).matches("4000 0000 0000 4243"))
        assertFalse(Regex(Saving.endsWithAny(listOf("4242"))).matches("4000 0000 4242 0000"))
        assertTrue(Regex(Saving.sameLettersAndDigits("sw1a1aa")).matches("SW1A 1AA"))
        assertFalse(Regex(Saving.sameLettersAndDigits("62701")).matches("62701-1234"))
        assertTrue(Regex(Saving.firstLine(listOf("calle", "mayor", "5"))).matches("Calle Mayor, 5"))
    }

    @Test fun `a card is saved from what its fields held, the number's digits with its expiry, its name, and its brand`() {
        val values = mapOf("number" to " 4242 4242 4242 4242 ", "exp" to "07 / 29", "holder" to "Ana López", "csc" to "123")
        assertEquals(
            mapOf(
                "kind" to "card",
                "name" to "Visa",
                "number" to "4242424242424242",
                "brand" to "Visa",
                "expMonth" to "7",
                "expYear" to "2029",
                "cardholderName" to "Ana López",
                "code" to "123",
            ),
            Saving.card(payment) { values[it] },
        )
        // Without its expiry, its name will do, and the other way round.
        assertEquals("2030", Saving.card(payment, mapOf("number" to "5555555555554444", "exp" to "1230")::get)?.get("expYear"))
        assertEquals("Mastercard", Saving.card(payment, mapOf("number" to "5555555555554444", "exp" to "1230")::get)?.get("name"))
        assertEquals("Ana López", Saving.card(payment, mapOf("number" to "4242424242424242", "holder" to "Ana López")::get)?.get("cardholderName"))
        // Not a card: a number alone, or one that isn't a card's.
        assertNull(Saving.card(payment, mapOf("number" to "4242424242424242")::get))
        assertNull(Saving.card(payment, mapOf("number" to "4242424242424241", "exp" to "07/29")::get))
        assertNull(Saving.card(payment, mapOf("number" to "1234", "exp" to "07/29")::get))
    }

    @Test fun `a card's security code is saved only from a field marked for it`() {
        val byWords =
            screen(
                page("number", label = "Card number"),
                page("exp", label = "Expiry (MM/YY)"),
                page("cvv", label = "CVV"),
            )
        val values = mapOf("number" to "4242424242424242", "exp" to "07/29", "cvv" to "123")
        val saved = Saving.card(byWords) { values[it] }!!
        assertEquals("4242424242424242", saved["number"])
        assertFalse(saved.containsKey("code"))
        assertFalse(saved.values.contains("123"))
    }

    @Test fun `a card's month and year from lists, and its name in parts`() {
        val months = listOf("Month") + MONTHS
        val lists =
            screen(
                page("number", "cc-number"),
                page("month", "cc-exp-month", options = months),
                page("year", "cc-exp-year", options = listOf("Year", "29", "30")),
                page("given", "cc-given-name"),
                page("family", "cc-family-name"),
            )
        val values = mapOf("number" to "378282246310005", "month" to "March", "year" to "30", "given" to "Ana María", "family" to "López")
        val saved = Saving.card(lists) { values[it] }!!
        assertEquals("3", saved["expMonth"])
        assertEquals("2030", saved["expYear"])
        assertEquals("Ana María López", saved["cardholderName"])
        assertEquals("Amex", saved["brand"])
    }

    @Test fun `an address is saved from what its fields held, named by its street`() {
        val values =
            mapOf(
                "given" to "Ana", "family" to "López", "line1" to "1 Example Street", "line2" to "Apt 2",
                "city" to "Springfield", "state" to "IL", "zip" to "62701", "email" to "ana@example.test",
            )
        assertEquals(
            mapOf(
                "kind" to "identity",
                "name" to "1 Example Street",
                "firstName" to "Ana",
                "lastName" to "López",
                "email" to "ana@example.test",
                "address1" to "1 Example Street",
                "address2" to "Apt 2",
                "city" to "Springfield",
                "state" to "IL",
                "postalCode" to "62701",
            ),
            Saving.identity(shipping) { values[it] },
        )
        // A whole name and a street in one field each.
        val oneField = screen(page("name", "name"), page("street", "street-address"), page("city", "address-level2"))
        val more = mapOf("name" to "Ana María López", "street" to "1 Example Street\nApt 2\nBuilding 3\nBack door", "city" to "Springfield")
        val saved = Saving.identity(oneField) { more[it] }!!
        assertEquals("Ana María", saved["firstName"])
        assertEquals("López", saved["lastName"])
        assertEquals("1 Example Street", saved["address1"])
        assertEquals("Apt 2", saved["address2"])
        assertEquals("Building 3, Back door", saved["address3"])
        // Two of the four, else nothing; named by the person without a street.
        assertNull(Saving.identity(shipping, mapOf("given" to "Ana", "email" to "ana@example.test")::get))
        assertEquals("Ana López", Saving.identity(shipping, mapOf("given" to "Ana", "family" to "López", "city" to "Springfield")::get)?.get("name"))
    }

    @Test fun `an address's state and country chosen from lists, never a list's prompt`() {
        val states = listOf("Select a state", "Idaho", "Illinois")
        val countries = listOf("Choose a country", "Canada", "United States")
        val lists =
            screen(
                page("line1", "address-line1"),
                page("zip", "postal-code"),
                page("state", "address-level1", options = states),
                page("country", "country", options = countries),
            )
        val chosen = mapOf("line1" to "1 Example Street", "zip" to "62701", "state" to "Illinois", "country" to "United States")
        val saved = Saving.identity(lists) { chosen[it] }!!
        assertEquals("Illinois", saved["state"])
        assertEquals("United States", saved["country"])
        val prompts = mapOf("line1" to "1 Example Street", "zip" to "62701", "state" to "Select a state", "country" to "Choose a country")
        val left = Saving.identity(lists) { prompts[it] }!!
        assertFalse(left.containsKey("state"))
        assertFalse(left.containsKey("country"))
        // A list whose first option is a place has no prompt.
        val first = screen(page("line1", "address-line1"), page("zip", "postal-code"), page("state", "address-level1", options = listOf("Idaho", "Illinois")))
        assertEquals("Idaho", Saving.identity(first, mapOf("line1" to "1 Example Street", "zip" to "62701", "state" to "Idaho")::get)?.get("state"))
    }

    @Test fun `months, years, and expiries as people type and lists show them`() {
        for ((text, month) in listOf("07" to 7, "7" to 7, "July" to 7, "Jul" to 7, "07 - July" to 7, "12" to 12, "dec" to 12)) {
            assertEquals(text, month, Saving.month(text))
        }
        for (text in listOf("Month", "MM", "13", "0", "")) assertNull(text, Saving.month(text))
        assertEquals("2029", Saving.year("2029"))
        assertEquals("2029", Saving.year("29"))
        assertNull(Saving.year("Year"))
        assertNull(Saving.year("1999"))
        for ((text, expiry) in listOf("07/29" to (7 to "2029"), "7 / 2029" to (7 to "2029"), "0729" to (7 to "2029"), "072029" to (7 to "2029"), "07-29" to (7 to "2029"))) {
            assertEquals(text, expiry, Saving.expiry(text))
        }
        for (text in listOf("13/29", "MM/YY", "729", "07/2", "")) assertNull(text, Saving.expiry(text))
    }

    @Test fun `a card's brand from its number, as the web vault guesses it`() {
        for ((number, brand) in
            listOf(
                "4242424242424242" to "Visa",
                "5555555555554444" to "Mastercard",
                "2223003122003222" to "Mastercard",
                "378282246310005" to "Amex",
                "6011111111111117" to "Discover",
                "30569309025904" to "Diners Club",
                "3530111333300000" to "JCB",
            )) {
            assertEquals(number, brand, Saving.brand(number))
        }
        assertNull(Saving.brand("9999999999999995"))
    }

    @Test fun `Luhn's checksum`() {
        assertTrue(Saving.luhn("4242424242424242"))
        assertTrue(Saving.luhn("378282246310005"))
        assertFalse(Saving.luhn("4242424242424241"))
        assertFalse(Saving.luhn(""))
    }
}
