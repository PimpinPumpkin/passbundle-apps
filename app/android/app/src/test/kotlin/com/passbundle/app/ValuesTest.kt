package com.passbundle.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

// How a card's or an identity's values go into the fields Roles.kt found
// (Values.kt), from made-up items' parts, by the core's names for them.
class ValuesTest {
    private val card =
        mapOf(
            "cardNumber" to "4111 1111 1111 1111",
            "cardholderName" to "Ana María López",
            "expMonth" to "7",
            "expYear" to "2029",
            "cardCode" to "123",
        )
    private val identity =
        mapOf(
            "title" to "Ms",
            "firstName" to "Ana",
            "middleName" to "María",
            "lastName" to "López",
            "address1" to "1 Example Street",
            "address2" to "Apt 2",
            "city" to "Springfield",
            "state" to "IL",
            "postalCode" to "62701",
            "country" to "US",
            "email" to "ana@example.test",
            "phone" to "+1 555 0100",
        )

    private fun of(parts: Map<String, String>, role: Role, format: String? = null) = Values.text(role, format) { parts[it] }

    @Test fun `a card's parts, the number in digits only`() {
        assertEquals("4111111111111111", of(card, Role.CARD_NUMBER))
        assertEquals("Ana María López", of(card, Role.CARD_NAME))
        assertEquals("Ana María", of(card, Role.CARD_GIVEN_NAME))
        assertEquals("López", of(card, Role.CARD_FAMILY_NAME))
        assertEquals("07/29", of(card, Role.CARD_EXPIRY))
        assertEquals("07 / 2029", of(card, Role.CARD_EXPIRY, "MM / YYYY"))
        assertEquals("0729", of(card, Role.CARD_EXPIRY, "MMYY"))
        assertEquals("07", of(card, Role.CARD_EXPIRY_MONTH))
        assertEquals("2029", of(card, Role.CARD_EXPIRY_YEAR))
        assertEquals("29", of(card, Role.CARD_EXPIRY_YEAR, "YY"))
        assertEquals("123", of(card, Role.CARD_CODE))
    }

    @Test fun `what a card hasn't, it leaves empty`() {
        val bare = mapOf("cardNumber" to "", "expMonth" to "13", "expYear" to "2029")
        assertNull(of(bare, Role.CARD_NUMBER))
        assertNull(of(bare, Role.CARD_EXPIRY))
        assertNull(of(bare, Role.CARD_EXPIRY_MONTH))
        assertNull(of(bare, Role.CARD_CODE))
        assertNull(of(mapOf("expMonth" to "7"), Role.CARD_EXPIRY))
        // One word: all of it is the given name.
        assertEquals("Ana", of(mapOf("cardholderName" to "Ana"), Role.CARD_GIVEN_NAME))
        assertNull(of(mapOf("cardholderName" to "Ana"), Role.CARD_FAMILY_NAME))
    }

    @Test fun `an identity's parts, its name whole, and its address in one field`() {
        assertEquals("Ana María López", of(identity, Role.FULL_NAME))
        assertEquals("Ms", of(identity, Role.TITLE))
        assertEquals("Ana", of(identity, Role.GIVEN_NAME))
        assertEquals("María", of(identity, Role.MIDDLE_NAME))
        assertEquals("López", of(identity, Role.FAMILY_NAME))
        assertEquals("1 Example Street, Apt 2", of(identity, Role.STREET))
        assertEquals("1 Example Street, Apt 2, Springfield, IL 62701, US", of(identity, Role.ADDRESS))
        assertEquals("1 Example Street", of(identity, Role.ADDRESS_LINE1))
        assertEquals("Apt 2", of(identity, Role.ADDRESS_LINE2))
        assertNull(of(identity, Role.ADDRESS_LINE3))
        assertEquals("Springfield", of(identity, Role.CITY))
        assertEquals("IL", of(identity, Role.STATE))
        assertEquals("62701", of(identity, Role.POSTAL_CODE))
        assertEquals("US", of(identity, Role.COUNTRY))
        assertEquals("ana@example.test", of(identity, Role.EMAIL))
        assertEquals("+1 555 0100", of(identity, Role.PHONE))
        assertNull(of(identity, Role.COMPANY))
        assertEquals("Springfield 62701", of(mapOf("city" to "Springfield", "postalCode" to "62701"), Role.ADDRESS))
    }

    @Test fun `a login's fields aren't a card's or an identity's to fill`() {
        for (role in listOf(Role.USERNAME, Role.PASSWORD, Role.CODE)) {
            assertNull(role.name, Values.text(role, null) { "anything" })
        }
    }

    @Test fun `an expiry's format comes from what the field shows, or how long it is`() {
        assertEquals("MM/YY", Values.expiryFormat("MM/YY", null))
        assertEquals("MM / YYYY", Values.expiryFormat("MM / YYYY", null))
        assertEquals("MM-YY", Values.expiryFormat("Expiry (mm-yy)", null))
        assertEquals("MM/YY", Values.expiryFormat("MM/AA", null))
        assertEquals("MMYY", Values.expiryFormat("MMYY", 7))
        assertEquals("MMYY", Values.expiryFormat(null, 4))
        assertEquals("MM/YYYY", Values.expiryFormat("Expiration date", 7))
        assertEquals("MM/YY", Values.expiryFormat("", null))
        assertEquals("YY", Values.yearFormat("YY", null))
        assertEquals("YYYY", Values.yearFormat("yyyy", 2))
        assertEquals("YY", Values.yearFormat(null, 2))
        assertEquals("YYYY", Values.yearFormat("Year", null))
    }

    @Test fun `months and years however they're saved`() {
        assertEquals("07", Values.month("7"))
        assertEquals("07", Values.month(" 07 "))
        assertEquals("12", Values.month("12"))
        assertNull(Values.month("0"))
        assertNull(Values.month("July"))
        assertEquals("2029", Values.year("29", "YYYY"))
        assertEquals("29", Values.year("2029", "YY"))
        assertNull(Values.year("202", "YYYY"))
        assertNull(Values.year("20x9", "YYYY"))
    }

    @Test fun `a month list's option, by name or number, or none`() {
        val numbered = listOf("Month") + (1..12).map { it.toString().padStart(2, '0') }
        assertEquals(7, Values.monthOption(numbered, "7"))
        assertEquals(12, Values.monthOption(numbered, "12"))
        assertEquals(3, Values.monthOption(listOf("January", "February", "March", "April"), "4"))
        assertEquals(2, Values.monthOption(listOf("Jan", "Feb", "Mar"), "03"))
        assertEquals(1, Values.monthOption(listOf("--", "07 - July"), "7"))
        // A list numbered from 0 still says the name.
        assertEquals(2, Values.monthOption(listOf("0 - January", "1 - February", "2 - March"), "3"))
        assertNull(Values.monthOption(listOf("Month", "1", "2"), "7"))
        assertNull(Values.monthOption(numbered, "13"))
        assertEquals(10, Values.monthOption(numbered, "10"))
        assertEquals(1, Values.monthOption(numbered, "1"))
    }

    @Test fun `a year list's option, whether it says 2029 or 29`() {
        assertEquals(4, Values.yearOption(listOf("Year", "2026", "2027", "2028", "2029"), "2029"))
        assertEquals(1, Values.yearOption(listOf("YY", "29", "30"), "2029"))
        assertEquals(1, Values.yearOption(listOf("Year", "2029"), "29"))
        assertNull(Values.yearOption(listOf("Year", "2026"), "2029"))
        assertNull(Values.yearOption(listOf("2029"), null))
    }

    @Test fun `a list is chosen only for a card's month or year`() {
        val months = listOf("Month") + (1..12).map { it.toString() }
        assertEquals(7, Values.option(Role.CARD_EXPIRY_MONTH, months) { card[it] })
        assertEquals(1, Values.option(Role.CARD_EXPIRY_YEAR, listOf("Year", "2029")) { card[it] })
        assertNull(Values.option(Role.COUNTRY, listOf("US", "Canada")) { identity[it] })
    }
}
