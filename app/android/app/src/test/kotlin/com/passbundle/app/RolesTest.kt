package com.passbundle.app

import android.text.InputType
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

// What each field on a screen is for (Roles.kt), on made-up screens: an
// app's fields as Android describes them, and a page's as Chrome does (its
// autocomplete words as autofill hints, and its attributes). Logins are found
// as before; one-time codes, cards, and identities only where PassBundle is
// sure, never a password field.
class RolesTest {
    private val text = InputType.TYPE_CLASS_TEXT
    private val password = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD

    /** A field in an app. */
    private fun app(id: String, vararg hints: String, hint: String? = null, idEntry: String? = null, inputType: Int = text) =
        Roles.Field(id, hints = hints.toList(), inputType = inputType, idEntry = idEntry, hint = hint)

    /** A field on a page, as Chrome describes it: `autocomplete` as hints, the rest as attributes. */
    private fun page(
        id: String,
        autocomplete: String? = null,
        type: String = "text",
        label: String? = null,
        placeholder: String? = null,
        options: List<String>? = null,
        maxLength: Int? = null,
    ) = Roles.Field(
        id,
        hints = autocomplete?.split(" ").orEmpty(),
        html = listOfNotNull("type" to type, "id" to id, "name" to id, label?.let { "label" to it }).toMap(),
        inputType = if (type == "password") password else text,
        hint = placeholder,
        maxLength = maxLength,
        options = options,
    )

    private fun screen(vararg fields: Roles.Field<String>) = Roles.screen(fields.toList())

    /** Each field's roles on the screen; a field PassBundle doesn't fill has none. */
    private fun roles(vararg fields: Roles.Field<String>): Map<String, List<Role>> =
        screen(*fields).found.groupBy({ it.id }, { it.role })

    @Test fun `an app's sign-in is a username and a password, as before`() {
        val found = roles(app("u", "username", hint = "Username"), app("p", "password", hint = "Password", inputType = password))
        assertEquals(mapOf("u" to listOf(Role.USERNAME), "p" to listOf(Role.PASSWORD)), found)
    }

    @Test fun `a page's sign-in, and the field before a password that says nothing`() {
        assertEquals(
            mapOf("username" to listOf(Role.USERNAME), "password" to listOf(Role.PASSWORD)),
            roles(page("username", "username", placeholder = "Email"), page("password", "current-password", type = "password")),
        )
        assertEquals(
            mapOf("who" to listOf(Role.USERNAME), "secret" to listOf(Role.PASSWORD)),
            roles(page("who"), page("secret", type = "password")),
        )
    }

    @Test fun `a sign-in's email is a login's only, since one identity field isn't an identity's form`() {
        assertEquals(
            mapOf("email" to listOf(Role.USERNAME), "password" to listOf(Role.PASSWORD)),
            roles(page("email", "email", type = "email", label = "Email"), page("password", "current-password", type = "password")),
        )
    }

    @Test fun `one-time codes by their marks`() {
        for (hint in listOf("smsOTPCode", "2faAppOTPCode", "emailOTPCode", "one-time-code")) {
            val screen = screen(app("code", hint))
            assertEquals(hint, "code", screen.code)
            assertNull(hint, screen.password)
        }
    }

    @Test fun `a code masked like a password is a code, never the password to save`() {
        for (field in listOf(
            page("otp", type = "password"),
            page("code", "one-time-code", type = "password"),
            page("verify", type = "password", label = "Verification code"),
            page("mfa_token", type = "password"),
            page("x", type = "password", label = "One-time password"),
        )) {
            val screen = screen(field)
            assertEquals(field.toString(), field.id, screen.code)
            assertNull(field.toString(), screen.password)
        }
    }

    @Test fun `a PIN, a recovery code, and other codes aren't one-time codes`() {
        assertEquals("pin", screen(page("pin", type = "password", label = "PIN")).password)
        for (field in listOf(
            page("recovery", label = "Recovery code"),
            page("backup_code", label = "Backup verification code"),
            page("zip", label = "ZIP code"),
            page("promo", label = "Promo code"),
            page("code", label = "Code"),
        )) {
            assertNull(field.toString(), screen(field).code)
        }
    }

    @Test fun `a sign-in that asks for its code on the same screen`() {
        val screen = screen(page("user", "username"), page("pass", "current-password", type = "password"), page("totp", "one-time-code"))
        assertEquals("user", screen.username)
        assertEquals("pass", screen.password)
        assertEquals("totp", screen.code)
    }

    @Test fun `a checkout's card fields by their autocomplete words`() {
        val screen =
            screen(
                page("name", "cc-name", label = "Name on card"),
                page("number", "cc-number", label = "Card number"),
                page("exp", "cc-exp", placeholder = "MM / YY"),
                // Masked, as many pages mask it: still the card's code, never a password.
                page("cvc", "cc-csc", type = "password", label = "Security code"),
            )
        assertEquals(
            listOf(
                Roles.Found("name", Role.CARD_NAME),
                Roles.Found("number", Role.CARD_NUMBER),
                Roles.Found("exp", Role.CARD_EXPIRY, format = "MM / YY"),
                Roles.Found("cvc", Role.CARD_CODE),
            ),
            screen.found,
        )
        assertNull(screen.password)
        assertNull(screen.username)
    }

    @Test fun `an app's card fields by Android's hints, a marked one by itself too`() {
        assertEquals(
            mapOf(
                "n" to listOf(Role.CARD_NUMBER),
                "m" to listOf(Role.CARD_EXPIRY_MONTH),
                "y" to listOf(Role.CARD_EXPIRY_YEAR),
                "c" to listOf(Role.CARD_CODE),
            ),
            roles(
                app("n", "creditCardNumber"),
                app("m", "creditCardExpirationMonth"),
                app("y", "creditCardExpirationYear"),
                app("c", "creditCardSecurityCode", inputType = InputType.TYPE_CLASS_NUMBER or InputType.TYPE_NUMBER_VARIATION_PASSWORD),
            ),
        )
        assertEquals(mapOf("c" to listOf(Role.CARD_CODE)), roles(page("c", "cc-csc")))
    }

    @Test fun `card fields by their words, but never the security code unmarked`() {
        val found =
            roles(
                page("cardNumber", label = "Card number"),
                page("holder", label = "Name on card"),
                page("expiry", placeholder = "MM/YY"),
                page("cvv", type = "password", label = "CVV"),
            )
        assertEquals(
            mapOf("cardNumber" to listOf(Role.CARD_NUMBER), "holder" to listOf(Role.CARD_NAME), "expiry" to listOf(Role.CARD_EXPIRY)),
            found,
        )
        // Nor is the masked CVV taken for a password.
        assertNull(screen(page("cvv", type = "password", label = "CVV")).password)
    }

    @Test fun `a lone card field by its words isn't sure, and a gift card is never one`() {
        assertTrue(roles(page("cardnumber", label = "Card number")).isEmpty())
        val gift =
            roles(
                page("giftCardNumber", label = "Gift card number"),
                page("expiry", label = "Expiration date (MM/YY)"),
                page("pin", type = "password", label = "PIN"),
            )
        assertTrue(gift.values.flatten().none { it.kind == Kind.CARD })
        assertEquals(listOf(Role.PASSWORD), gift["pin"])
    }

    @Test fun `a password field is never a card's or an identity's, whatever it's called`() {
        val screen =
            screen(
                page("cc_number", type = "password", label = "Card number"),
                page("exp", "cc-exp"),
                page("city", type = "password", label = "City"),
                page("zip", "postal-code"),
            )
        assertEquals(listOf(Role.PASSWORD), screen.found.filter { it.id == "cc_number" }.map { it.role })
        assertTrue(screen.found.none { it.id == "city" })
    }

    @Test fun `a field marked as something else is never guessed into a card`() {
        // The extension's bug: "name" in a shipping address filled with the name on the card.
        val found =
            roles(
                page("name", "shipping name", label = "Name"),
                page("ship-city", "shipping address-level2", label = "Cardholder city"),
                page("number", "cc-number"),
                page("ccname", label = "Name on card"),
            )
        assertEquals(listOf(Role.FULL_NAME), found["name"])
        assertEquals(listOf(Role.CITY), found["ship-city"])
        assertEquals(listOf(Role.CARD_NAME), found["ccname"])
        // A birthday by its marks, beside a card, isn't the card's expiry.
        assertNull(roles(page("bday", "bday-month", label = "Expiry month"), page("n", "cc-number"))["bday"])
        // Marked as a card's brand, or a part of a phone number: left alone, whatever its words say.
        val marked =
            roles(
                page("brand", "cc-type", label = "Card number"),
                page("n", "cc-number"),
                page("local", "tel-local", label = "Phone number"),
                page("given", "given-name"),
                page("family", "family-name"),
            )
        assertNull(marked["brand"])
        assertNull(marked["local"])
        assertEquals(listOf(Role.CARD_NUMBER), marked["n"])
    }

    @Test fun `a card's start date isn't its expiry`() {
        val found = roles(page("number", "cc-number"), page("from", label = "Valid from (MM/YY)"), page("to", label = "Valid thru (MM/YY)"))
        assertNull(found["from"])
        assertEquals(listOf(Role.CARD_EXPIRY), found["to"])
    }

    @Test fun `expiry lists are chosen by their options`() {
        val months = listOf("Month") + (1..12).map { it.toString().padStart(2, '0') }
        val years = listOf("Year", "2026", "2027", "2028")
        val found =
            screen(
                page("number", "cc-number"),
                page("month", "cc-exp-month", options = months),
                page("year", label = "Expiration year", options = years),
                // A country's list alone isn't an address's (one identity field isn't enough).
                page("country", "country", options = listOf("Canada", "United States")),
            ).found
        assertEquals(
            listOf(
                Roles.Found("number", Role.CARD_NUMBER),
                Roles.Found("month", Role.CARD_EXPIRY_MONTH, options = months),
                Roles.Found("year", Role.CARD_EXPIRY_YEAR, options = years),
            ),
            found,
        )
    }

    @Test fun `a year field says how it writes the year`() {
        val found =
            screen(page("number", "cc-number"), page("y2", "cc-exp-year", placeholder = "YY"), page("y4", "cc-exp-year", maxLength = 4)).found
        assertEquals("YY", found.first { it.id == "y2" }.format)
        assertEquals("YYYY", found.first { it.id == "y4" }.format)
    }

    @Test fun `an address form by its autocomplete words`() {
        assertEquals(
            mapOf(
                "given" to listOf(Role.GIVEN_NAME),
                "family" to listOf(Role.FAMILY_NAME),
                "org" to listOf(Role.COMPANY),
                "line1" to listOf(Role.ADDRESS_LINE1),
                "line2" to listOf(Role.ADDRESS_LINE2),
                "city" to listOf(Role.CITY),
                "state" to listOf(Role.STATE),
                "zip" to listOf(Role.POSTAL_CODE),
                "country" to listOf(Role.COUNTRY),
                "tel" to listOf(Role.PHONE),
                // An email field is a login's username too, as before.
                "email" to listOf(Role.USERNAME, Role.EMAIL),
            ),
            roles(
                page("given", "section-ship shipping given-name"),
                page("family", "shipping family-name"),
                page("org", "organization"),
                page("line1", "address-line1"),
                page("line2", "address-line2"),
                page("city", "address-level2"),
                page("state", "address-level1"),
                page("zip", "postal-code"),
                page("country", "country"),
                page("tel", "tel", type = "tel"),
                page("email", "email", type = "email"),
            ),
        )
    }

    @Test fun `an app's address by Android's hints`() {
        assertEquals(
            mapOf(
                "name" to listOf(Role.FULL_NAME),
                "address" to listOf(Role.ADDRESS),
                "zip" to listOf(Role.POSTAL_CODE),
                "phone" to listOf(Role.PHONE),
            ),
            roles(app("name", "personName"), app("address", "postalAddress"), app("zip", "postalCode"), app("phone", "phone")),
        )
    }

    @Test fun `an address form by its words`() {
        assertEquals(
            mapOf(
                "fname" to listOf(Role.GIVEN_NAME),
                "lname" to listOf(Role.FAMILY_NAME),
                "street" to listOf(Role.ADDRESS_LINE1),
                "apt" to listOf(Role.ADDRESS_LINE2),
                "town" to listOf(Role.CITY),
                "region" to listOf(Role.STATE),
                "postcode" to listOf(Role.POSTAL_CODE),
                "nation" to listOf(Role.COUNTRY),
                "company" to listOf(Role.COMPANY),
                "phone" to listOf(Role.PHONE),
                // Only its label says email, which logins don't read: an identity's alone.
                "mail" to listOf(Role.EMAIL),
            ),
            roles(
                page("fname", label = "First name"),
                page("lname", label = "Last name"),
                page("street", label = "Street address"),
                page("apt", label = "Apt, suite, etc. (optional)"),
                page("town", label = "City"),
                page("region", label = "State / Province"),
                page("postcode", label = "ZIP / Postal code"),
                page("nation", label = "Country/Region"),
                page("company", label = "Company (optional)"),
                page("phone", label = "Phone"),
                page("mail", label = "Email address"),
            ),
        )
        // An app's own names: its resource names and hints.
        assertEquals(
            mapOf("first" to listOf(Role.GIVEN_NAME), "addressLine2" to listOf(Role.ADDRESS_LINE2)),
            roles(app("first", idEntry = "first_name"), app("addressLine2", idEntry = "addressLine2")),
        )
    }

    @Test fun `an address's state and country lists are chosen by their options, never a phone's country code`() {
        val states = listOf("State", "Idaho", "Illinois")
        val countries = listOf("Canada", "United States")
        val found =
            screen(
                page("line1", "address-line1"),
                page("zip", "postal-code"),
                page("state", "address-level1", options = states),
                page("nation", label = "Country/Region", options = countries),
                page("calling", label = "Country code", options = listOf("+1", "+44")),
                page("title", label = "Title", options = listOf("Ms", "Mr")),
            ).found
        assertEquals(
            listOf(
                Roles.Found("line1", Role.ADDRESS_LINE1),
                Roles.Found("zip", Role.POSTAL_CODE),
                Roles.Found("state", Role.STATE, options = states),
                Roles.Found("nation", Role.COUNTRY, options = countries),
            ),
            found,
        )
    }

    @Test fun `words that fit two parts, or someone else's, fit none`() {
        val found =
            roles(
                page("cityState", label = "City, State"),
                page("birthCity", label = "City of birth"),
                page("emergency", label = "Emergency contact name"),
                page("search", label = "Search by city"),
                page("given", "given-name"),
                page("family", "family-name"),
            )
        assertEquals(setOf("given", "family"), found.keys)
        // On a sign-in, "Phone or email" is the username, and no identity's.
        assertEquals(mapOf("login" to listOf(Role.USERNAME)), roles(page("login", label = "Phone or email"), page("given", "given-name")).filterKeys { it == "login" })
    }

    @Test fun `a lone identity field isn't an identity's form`() {
        assertTrue(roles(page("city", label = "City")).isEmpty())
        assertTrue(roles(page("zip", "postal-code")).isEmpty())
    }

    @Test fun `a card's or an identity's field isn't taken for the username before a password`() {
        // Before, the last text field before the password stood in for the username: here, the family name.
        for (screen in listOf(
            screen(page("given", "given-name"), page("family", "family-name"), page("new", "new-password", type = "password")),
            screen(page("first", label = "First name"), page("last", label = "Last name"), page("new", type = "password")),
        )) {
            assertNull(screen.username)
            assertEquals("new", screen.password)
            assertEquals(2, screen.of(Kind.IDENTITY).size)
        }
    }

    @Test fun `a card number that signs in, before a password, is still the username`() {
        // Some banks sign in with the card's number: by its words alone, it isn't sure to be a card's.
        assertEquals(
            mapOf("card" to listOf(Role.USERNAME), "pin" to listOf(Role.PASSWORD)),
            roles(page("card", label = "Card number"), page("pin", type = "password", label = "Password")),
        )
    }

    @Test fun `words are whole words, run together or apart`() {
        assertEquals(listOf("card", "number", "input"), Roles.words("cardNumberInput"))
        assertEquals(listOf("cc", "exp", "month"), Roles.words("cc-exp-month"))
        assertTrue(Roles.has(Roles.words("cc-number"), "cc number"))
        assertTrue(Roles.has(Roles.words("ccnumber"), "cc number"))
        assertTrue(Roles.has(Roles.words("addressLine1"), "address line 1"))
        assertTrue(Roles.has(Roles.words("Address line 1"), "address line 1"))
        // Never inside a longer word.
        assertFalse(Roles.has(Roles.words("cidade"), "cid"))
        assertFalse(Roles.has(Roles.words("username"), "name"))
        assertFalse(Roles.has(Roles.words("cvv2"), "cvv"))
    }

    @Test fun `a number of 4 to 8 digits with no username is a code, not a password to save`() {
        assertTrue(Roles.looksLikeACode(null, "123456"))
        assertTrue(Roles.looksLikeACode("", "1234"))
        assertTrue(Roles.looksLikeACode(" ", "12345678"))
        // A username with a numeric PIN still saves, and so does a password.
        assertFalse(Roles.looksLikeACode("ana@example.test", "123456"))
        assertFalse(Roles.looksLikeACode(null, "123456789"))
        assertFalse(Roles.looksLikeACode(null, "123"))
        assertFalse(Roles.looksLikeACode(null, "12ab56"))
    }
}
