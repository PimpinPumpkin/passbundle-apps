package com.passbundle.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

// A login's second line in Android's autofill list (Responses.kt): its
// username, the site it's from, and with more than one account on the
// phone, which account's it is (DESIGN.md in the meta repo, "Several
// accounts in the apps").
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
}
