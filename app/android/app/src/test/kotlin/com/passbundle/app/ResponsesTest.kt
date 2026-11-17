package com.passbundle.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

// A login's second line in Android's autofill list (Responses.kt): its
// username, the site it's from, and with more than one account on the
// phone, which account's it is (DESIGN.md in the meta repo, "Several
// accounts in the apps"); a one-time code's; and which login's code a
// code screen offers first. And the list itself (`Responses.entries`):
// what FillActivity hands back after unlocking (or trusting a browser) and
// what's offered while the vault is open, the same entries either way, each
// a pick holding no value, for logins, codes, cards, and identities alike.
class ResponsesTest {
    private fun page(id: String, autocomplete: String, type: String = "text") =
        Roles.Field(id, hints = autocomplete.split(" "), html = mapOf("type" to type, "id" to id, "name" to id))

    /** A sign-in that asks for its code, beside a card's fields and an address's, on one made-up page. */
    private val everything =
        Roles.screen(
            listOf(
                page("user", "username"),
                page("pass", "current-password", type = "password"),
                page("otp", "one-time-code"),
                page("number", "cc-number"),
                page("exp", "cc-exp"),
                page("given", "given-name"),
                page("family", "family-name"),
            ),
        )

    private val withCode = Responses.Offered("a", "acc", "Mail", username = "ana@example.test", totp = true)
    private val withoutCode = Responses.Offered("b", "acc", "Shop", username = "ana@example.test")
    private val card = Responses.Offered("c", "acc", "Visa", subtitle = "Visa, *1111")
    private val identity = Responses.Offered("i", "acc", "Home", subtitle = "Ana López", reprompt = true)

    @Test fun `after unlocking, every kind is offered to pick, and nothing is filled until one is picked`() {
        val list = Responses.entries(everything, listOf(withCode, withoutCode), listOf(card), listOf(identity))
        assertEquals(listOf(Kind.LOGIN, Kind.LOGIN, Kind.CARD, Kind.IDENTITY), list.map { it.kind })
        // Each is a pick: FillActivity fills it, from its own account, only once it's picked.
        for (entry in list) {
            assertEquals(entry.toString(), FillActivity.FILL, entry.mode)
            assertEquals("acc", entry.account)
        }
        assertEquals(listOf("a", "b", "c", "i"), list.map { it.item })
        // Each on its own kind's fields; a login with a code covers the code field too.
        assertEquals(listOf("user", "pass", "otp"), list[0].ids)
        assertEquals(listOf("user", "pass"), list[1].ids)
        assertEquals(listOf("number", "exp"), list[2].ids)
        assertEquals(listOf("given", "family"), list[3].ids)
        // One that asks for the master password again says so to FillActivity.
        assertEquals(listOf(false, false, false, true), list.map { it.asks })
    }

    @Test fun `an entry in the list holds no value, only what it says and what picking it opens`() {
        assertEquals(
            setOf("title", "subtitle", "ids", "mode", "kind", "account", "item", "asks"),
            Responses.Entry::class.java.declaredFields.map { it.name }.filterNot { it.startsWith("$") }.toSet(),
        )
        val list = Responses.entries(everything, listOf(withCode), listOf(card), listOf(identity))
        assertEquals(listOf("Mail", "Visa", "Home"), list.map { it.title })
        assertEquals(listOf("ana@example.test", "Visa, *1111", "Ana López"), list.map { it.subtitle })
    }

    @Test fun `on a screen that asks only for a code, each login's code is offered to pick, the one just filled first`() {
        val codeOnly = Roles.screen(listOf(page("otp", "one-time-code")))
        val other = Responses.Offered("d", "acc", "Bank", username = "ana@bank.example.test", totp = true)
        val list = Responses.entries(codeOnly, listOf(withCode, withoutCode, other), emptyList(), emptyList()) { it.id == "d" }
        assertEquals(listOf("d", "a"), list.map { it.item })
        for (entry in list) {
            assertEquals(Kind.CODE, entry.kind)
            assertEquals(FillActivity.FILL, entry.mode)
            assertEquals(listOf("otp"), entry.ids)
        }
        assertEquals("One-time code · ana@bank.example.test", list[0].subtitle)
    }

    @Test fun `a login whose code needs Premium is offered without its code, and not on a code screen`() {
        // The core says `totp` false for a key whose code needs Premium (core/src/codes.rs).
        val needsPremium = Responses.Offered("p", "acc", "Bank", username = "ana@bank.example.test", totp = false)
        val signIn = Responses.entries(everything, listOf(needsPremium, withCode), emptyList(), emptyList())
        assertEquals(listOf("p", "a"), signIn.map { it.item })
        assertEquals(listOf("user", "pass"), signIn[0].ids)
        assertEquals(listOf("user", "pass", "otp"), signIn[1].ids)
        val codeOnly = Roles.screen(listOf(page("otp", "one-time-code")))
        assertTrue(Responses.entries(codeOnly, listOf(needsPremium), emptyList(), emptyList()).isEmpty())
        assertEquals(listOf("a"), Responses.entries(codeOnly, listOf(needsPremium, withCode), emptyList(), emptyList()).map { it.item })
    }

    @Test fun `the offer to trust a browser opens its dialog, never a fill`() {
        val signIn = Roles.screen(listOf(page("user", "username"), page("pass", "current-password", type = "password")))
        val list = Responses.entries(signIn, emptyList(), listOf(card), emptyList(), Responses.Trust("bank.example.test", "Example Browser", "acc"))
        assertEquals(1, list.size)
        val trust = list.single()
        assertEquals(FillActivity.TRUST, trust.mode)
        assertNull(trust.item)
        assertEquals("Logins for bank.example.test", trust.title)
        assertEquals(listOf("user", "pass"), trust.ids)
    }

    @Test fun `nothing to offer is an empty list, and cards and identities only where their fields are`() {
        val signIn = Roles.screen(listOf(page("user", "username"), page("pass", "current-password", type = "password")))
        assertTrue(Responses.entries(signIn, emptyList(), listOf(card), listOf(identity)).isEmpty())
        assertTrue(Responses.entries(everything, emptyList(), emptyList(), emptyList()).isEmpty())
    }

    @Test fun `a login says its username, its site, and its account`() {
        assertEquals("ana@example.test · linked.test · Work", Responses.subtitle("ana@example.test", "linked.test", "Work"))
        assertEquals("ana@example.test · Personal", Responses.subtitle("ana@example.test", null, "Personal"))
    }

    @Test fun `with one account, it says what it said before`() {
        assertEquals("ana@example.test", Responses.subtitle("ana@example.test", null, null))
        assertEquals("ana@example.test · linked.test", Responses.subtitle("ana@example.test", "linked.test", null))
        assertNull(Responses.subtitle(null, null, null))
    }

    @Test fun `a one-time code says so, then whose it is`() {
        assertEquals("One-time code · ana@example.test", Responses.codeSubtitle("ana@example.test", null, null))
        assertEquals("One-time code · ana@example.test · linked.test · Work", Responses.codeSubtitle("ana@example.test", "linked.test", "Work"))
        assertEquals("One-time code", Responses.codeSubtitle(null, null, null))
    }

    @Test fun `the login just filled comes first on the code screen after it, in the same app and page, for a few minutes`() {
        val filled = Responses.Filled("account", "item", "com.example.bank", "bank.example.test", at = 1_000_000)
        assertTrue(filled.before("com.example.bank", "bank.example.test", now = 1_000_000 + 60_000))
        assertTrue(filled.before("com.example.bank", "bank.example.test", now = 1_000_000 + 4 * 60_000))
        assertFalse(filled.before("com.example.bank", "bank.example.test", now = 1_000_000 + 6 * 60_000))
        assertFalse(filled.before("com.example.other", "bank.example.test", now = 1_000_000 + 60_000))
        assertFalse(filled.before("com.example.bank", "other.example.test", now = 1_000_000 + 60_000))
        assertFalse(filled.before("com.example.bank", null, now = 1_000_000 + 60_000))
        // A clock that went back doesn't count.
        assertFalse(filled.before("com.example.bank", "bank.example.test", now = 999_000))
    }
}
