package com.passbundle.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

// Autofill's entries as suggestions on the keyboard (Inline.kt): the list's
// own entries, in its order, as many as the keyboard takes, each drawn with
// the keyboard's spec for its place; and none when a spec needed isn't one
// PassBundle can draw.
class InlineTest {
    private fun page(id: String, autocomplete: String, type: String = "text") =
        Roles.Field(id, hints = autocomplete.split(" "), html = mapOf("type" to type, "id" to id, "name" to id))

    @Test fun `each entry gets the keyboard's spec for its place, the last spec for the rest`() {
        assertEquals(listOf(0, 0, 0), Inline.specs(3, 10, listOf(true)))
        assertEquals(listOf(0, 1, 2), Inline.specs(3, 10, listOf(true, true, true)))
        assertEquals(listOf(0, 1, 1, 1), Inline.specs(4, Int.MAX_VALUE, listOf(true, true)))
    }

    @Test fun `as many as the keyboard takes, the first in the list's order`() {
        assertEquals(listOf(0, 1), Inline.specs(5, 2, listOf(true, true, true)))
        assertEquals(listOf(0), Inline.specs(1, 5, listOf(true)))
        assertTrue(Inline.specs(0, 5, listOf(true)).isEmpty())
        assertTrue(Inline.specs(3, 0, listOf(true)).isEmpty())
    }

    @Test fun `a spec PassBundle can't draw, or none, puts no entry on the keyboard`() {
        assertTrue(Inline.specs(2, 5, emptyList()).isEmpty())
        assertTrue(Inline.specs(2, 5, listOf(false)).isEmpty())
        assertTrue(Inline.specs(2, 5, listOf(true, false)).isEmpty())
        // Only the specs that are needed count.
        assertEquals(listOf(0), Inline.specs(1, 5, listOf(true, false)))
        assertEquals(listOf(0, 1), Inline.specs(2, 2, listOf(true, true, false)))
    }

    @Test fun `the keyboard gets the list's entries, which hold no value, in the list's order`() {
        val screen =
            Roles.screen(
                listOf(
                    page("user", "username"),
                    page("pass", "current-password", type = "password"),
                    page("number", "cc-number"),
                    page("exp", "cc-exp"),
                ),
            )
        val logins = listOf(Responses.Offered("a", "acc", "Mail", username = "ana@example.test"), Responses.Offered("b", "acc", "Shop"))
        val cards = listOf(Responses.Offered("c", "acc", "Visa", subtitle = "Visa, *4242"))
        val entries = Responses.entries(screen, logins, cards, emptyList())
        val shown = Inline.specs(entries.size, 2, listOf(true))
        // The first two of the list's three: what each says, and what picking it opens.
        assertEquals(listOf("Mail", "Shop"), shown.indices.map { entries[it].title })
        assertEquals(listOf("ana@example.test", null), shown.indices.map { entries[it].subtitle })
        for (i in shown.indices) assertEquals(FillActivity.FILL, entries[i].mode)
    }
}
