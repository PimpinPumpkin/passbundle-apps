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
// code screen offers first.
class ResponsesTest {
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
