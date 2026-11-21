package com.passbundle.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

// Fill rules on a website (FillRules.kt; which value goes where is the
// core's, core/src/fill_rules.rs, tested there against the meta repo's
// vectors): the page as the core reads it, each value as its field takes it,
// and Roles.kt's matching of what the rules leave, without the fields they
// keep empty.
class FillRulesTest {
    @Test fun `the page goes to the core as JSON, with the host the browser is trusted to say`() {
        val page =
            FillRules.Page(
                scheme = "https",
                fields =
                    listOf(
                        FillRules.PageField(
                            at = 3, tag = "input", type = "text", id = "i", name = "n", label = "L", ariaLabel = "A",
                            placeholder = "P", autocomplete = "email", visible = true, list = false, maxLength = 4, form = 0,
                        ),
                    ),
                forms = listOf("f"),
            )
        // The same text the core's own test reads (core/src/fill_rules/tests.rs, `a_page_comes_from_the_app_as_json`).
        assertEquals(
            """{"scheme":"https","host":"a.test","fields":[{"tag":"input","type":"text","id":"i","name":"n","label":"L","ariaLabel":"A","placeholder":"P","autocomplete":"email","visible":true,"list":false,"maxLength":4,"form":0}],"forms":["f"]}""",
            FillRules.json(page, "a.test"),
        )
    }

    @Test fun `what a page leaves out is null, and its words go as they are`() {
        val page =
            FillRules.Page(
                scheme = null,
                fields = listOf(FillRules.PageField(at = 0, label = "Say \"hi\"\\\n\tthere\u2028", visible = false, list = true)),
                forms = listOf(null),
            )
        assertEquals(
            """{"scheme":null,"host":"a.test","fields":[{"tag":null,"type":null,"id":null,"name":null,""" +
                """"label":"Say \"hi\"\\\u000a\u0009there\u2028","ariaLabel":null,"placeholder":null,"autocomplete":null,""" +
                """"visible":false,"list":true,"maxLength":null,"form":null}],"forms":[null]}""",
            FillRules.json(page, "a.test"),
        )
    }

    @Test fun `a text field takes a value as the core says`() {
        assertEquals("C-1234", FillRules.text("text", "C-1234"))
        // A card's number in digits, as Android's card fill writes it.
        assertEquals("4111111111111111", FillRules.text("digits", "4111 1111 1111 1111"))
        assertEquals("07", FillRules.text("month", "7"))
        assertEquals("12", FillRules.text("month", "12"))
        // Not a month: as it is, as the extension fills it.
        assertEquals("13", FillRules.text("month", "13"))
        assertEquals("2029", FillRules.text("year", "29"))
        assertEquals("2029", FillRules.text("year", " 2029 "))
        assertEquals("29", FillRules.text("shortYear", "2029"))
        assertEquals("29", FillRules.text("shortYear", "29"))
        // Nothing to fill, or a way the app doesn't know: the field is left as it is.
        assertNull(FillRules.text("text", ""))
        assertNull(FillRules.text("digits", "none"))
        assertNull(FillRules.text("someday", "C-1234"))
        assertNull(FillRules.text("monthOption", "7"))
    }

    @Test fun `a list takes the option that is the value, or none`() {
        assertEquals(3, FillRules.option("monthOption", listOf("--", "January", "February", "March"), "3"))
        assertEquals(1, FillRules.option("yearOption", listOf("2028", "2029"), "29"))
        assertEquals(1, FillRules.option("stateOption", listOf("Indiana", "Illinois"), "IL"))
        assertEquals(0, FillRules.option("countryOption", listOf("United States", "Canada"), "US"))
        assertEquals(1, FillRules.option("option", listOf("Chicago", " springfield "), "Springfield"))
        assertNull(FillRules.option("option", listOf("Chicago"), "Springfield"))
        assertNull(FillRules.option("monthOption", listOf("2028", "2029"), "7"))
        assertNull(FillRules.option("text", listOf("Springfield"), "Springfield"))
    }

    private fun field(id: String, type: String = "text") = Roles.Field(id, html = mapOf("type" to type, "id" to id, "name" to id))

    @Test fun `Roles kt's matching fills what the rules leave, without the fields they keep empty`() {
        val described = listOf(field("email", "email"), field("login"), field("first", "password"), field("second", "password"))
        val page = FillRules.Page("https", described.mapIndexed { at, f -> FillRules.PageField(at = at, id = f.id) })
        val all = Roles.screen(described)
        assertEquals("email" to "first", all.username to all.password)
        // The username and the password the person said never to fill: the next fields that fit get them.
        val left = FillRules.left(described, page, listOf(0, 2))
        assertEquals("login" to "second", left.username to left.password)
        // A field the page doesn't have changes nothing.
        val same = FillRules.left(described, page, listOf(9))
        assertEquals("email" to "first", same.username to same.password)
    }

    @Test fun `a page's fields name the screen's by their place, which may skip the app's own`() {
        // The browser's own address bar, say: no HTML, so no rule's.
        val described = listOf(Roles.Field("bar"), field("user", "email"), field("pass", "password"))
        val page = FillRules.Page("https", listOf(FillRules.PageField(at = 1, id = "user"), FillRules.PageField(at = 2, id = "pass")))
        // The page's second field is the screen's third.
        val left = FillRules.left(described, page, listOf(1))
        assertEquals("user" to null, left.username to left.password)
    }
}
