package com.passbundle.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

// Where a new passkey goes on Android, and what's asked (PasskeySaving.kt):
// a site that has one of PassBundle's already is told so and offered
// nothing; one login with the account's passkey for the site is asked
// about replacing it; otherwise the site's logins and "New login"; with
// none, a new login. "New login" only where an account unlocked here takes
// one; where none does (an organization keeps the person's items out of
// Personal and they have no shared folder to add to), the core's words say
// why before anything is picked. The core decides which logins and which
// accounts (matching.rs and app.rs, `no_new_passkey_login`, and
// core/tests/passkey_logins.rs holds it to that).
class PasskeySavingTest {
    private val bank = PasskeySaving.Login("b", "work", "Bank website", username = "bank@example.test")
    private val old = PasskeySaving.Login("o", "work", "Old sign-up", username = "ana@example.test", replaces = true, passkeyUserName = "ana")
    private val home = PasskeySaving.Login("h", "home", "Bank at home", accountLabel = "Personal")

    /** The core's words for an account Contoso keeps out of Personal, with no shared folder to add to. */
    private val nowhere = "Contoso keeps items out of Personal, and you have no shared folder you can add to. Ask an admin for one, then try again."
    private val work = PasskeySaving.Account("work", "Work")
    private val keptOut = PasskeySaving.Account("home", "Personal", newLogin = false, noNewLogin = nowhere)

    @Test fun `a site that has the passkey already is told so, whatever the logins`() {
        assertEquals(PasskeySaving.Step.Excluded, PasskeySaving.step(excluded = true, logins = listOf(old, bank)))
        assertEquals(PasskeySaving.Step.Excluded, PasskeySaving.step(excluded = true, logins = emptyList()))
    }

    @Test fun `the one login with the account's passkey is asked about, and nothing else`() {
        assertEquals(PasskeySaving.Step.Replace(old), PasskeySaving.step(excluded = false, logins = listOf(old, bank, home)))
        assertEquals(PasskeySaving.Step.Replace(old), PasskeySaving.step(excluded = false, logins = listOf(old)))
    }

    @Test fun `otherwise the site's logins are offered, and with none a new login`() {
        assertEquals(PasskeySaving.Step.Choose(listOf(bank, home)), PasskeySaving.step(excluded = false, logins = listOf(bank, home)))
        // Two with the account's passkey, from before PassBundle replaced them: the person picks.
        val again = old.copy(id = "o2", name = "Older sign-up")
        assertEquals(PasskeySaving.Step.Choose(listOf(old, again, bank)), PasskeySaving.step(excluded = false, logins = listOf(old, again, bank)))
        assertEquals(PasskeySaving.Step.New, PasskeySaving.step(excluded = false, logins = emptyList()))
    }

    @Test fun `with no account that takes a new login, its logins are offered without one, and with none it says why`() {
        // The one account here keeps its items out of Personal, with no shared folder to add to.
        val why = PasskeySaving.noNewLogin(listOf(keptOut))
        assertEquals(nowhere, why)
        assertEquals(PasskeySaving.Step.Choose(listOf(bank), nowhere), PasskeySaving.step(excluded = false, logins = listOf(bank), noNewLogin = why))
        assertEquals(PasskeySaving.Step.Nowhere(nowhere), PasskeySaving.step(excluded = false, logins = emptyList(), noNewLogin = why))
        // A login with the account's passkey is still asked about, and a site that has the passkey is still told so.
        assertEquals(PasskeySaving.Step.Replace(old), PasskeySaving.step(excluded = false, logins = listOf(old, bank), noNewLogin = why))
        assertEquals(PasskeySaving.Step.Excluded, PasskeySaving.step(excluded = true, logins = emptyList(), noNewLogin = why))
        // Another account here that takes one: a new login, in it.
        assertNull(PasskeySaving.noNewLogin(listOf(keptOut, work)))
        assertEquals(PasskeySaving.Step.New, PasskeySaving.step(excluded = false, logins = emptyList(), noNewLogin = PasskeySaving.noNewLogin(listOf(keptOut, work))))
        assertNull(PasskeySaving.noNewLogin(emptyList()))
        // Without its words (never from the core), it still says something.
        assertEquals("PassBundle couldn't save the passkey.", PasskeySaving.noNewLogin(listOf(keptOut.copy(noNewLogin = null))))
    }

    @Test fun `a new login goes only in an account that takes one, named when another here doesn't`() {
        assertEquals(listOf(work) to "Work", PasskeySaving.newLoginIn(listOf(keptOut, work)))
        assertEquals(listOf(work) to null, PasskeySaving.newLoginIn(listOf(work)))
        val other = PasskeySaving.Account("other", "Other")
        assertEquals(listOf(work, other) to null, PasskeySaving.newLoginIn(listOf(work, keptOut, other)))
        assertEquals(emptyList<PasskeySaving.Account>() to null, PasskeySaving.newLoginIn(listOf(keptOut)))
    }

    @Test fun `without a new login, the list has none and the question says why`() {
        assertEquals(
            listOf("Bank website\nbank@example.test", "Bank at home\nPersonal"),
            PasskeySaving.rows(listOf(bank, home), newLogin = false),
        )
        assertEquals(
            "For example.test, as ana. Add it to one of your logins for this site. $nowhere",
            PasskeySaving.choose("example.test", "example.test", "ana", nowhere),
        )
    }

    @Test fun `each row says the login, its username, what it replaces, and its account, then New login`() {
        assertEquals(
            listOf(
                "Bank website\nbank@example.test",
                "Old sign-up\nana@example.test · Replaces its passkey",
                "Bank at home\nPersonal",
                "New login",
            ),
            PasskeySaving.rows(listOf(bank, old, home)),
        )
        assertEquals("Plain", PasskeySaving.row(PasskeySaving.Login("p", "work", "Plain")))
        assertEquals(listOf("New login"), PasskeySaving.rows(emptyList()))
    }

    @Test fun `the questions name the site's ID first, then the name it gives itself, and the account`() {
        assertEquals(
            "For example.test (says it's Example), as ana@example.test, in a new login.",
            PasskeySaving.newLogin("example.test", "Example", "ana@example.test", null),
        )
        assertEquals("For example.test, in a new login in Work.", PasskeySaving.newLogin("example.test", "example.test", "", "Work"))
        assertEquals(
            "For example.test (says it's Example), as ana. Add it to one of your logins for this site, or to a new login.",
            PasskeySaving.choose("example.test", "Example", "ana"),
        )
    }

    @Test fun `replacing says whose passkey, where, and that the old one stops working`() {
        assertEquals("Replace the passkey for ana@example.test on example.test?", PasskeySaving.replaceTitle("ana@example.test", old, "example.test"))
        // The site gave no name: the old passkey's, else "this account".
        assertEquals("Replace the passkey for ana on example.test?", PasskeySaving.replaceTitle("", old, "example.test"))
        assertEquals("Replace the passkey for this account on example.test?", PasskeySaving.replaceTitle("", bank, "example.test"))
        assertEquals(
            "Old sign-up has a passkey for this account already. The new one takes its place, and the old one stops working the next time you sign in to example.test.",
            PasskeySaving.replaceMessage(old, "example.test"),
        )
        assertEquals(
            "Bank at home (in Personal) has a passkey for this account already. The new one takes its place, and the old one stops working the next time you sign in to example.test.",
            PasskeySaving.replaceMessage(home, "example.test"),
        )
    }
}
