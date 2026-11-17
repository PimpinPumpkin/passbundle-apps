package com.passbundle.app

import org.junit.Assert.assertEquals
import org.junit.Test

// What Android's autofill offers and how a locked account unlocks there
// (Unlocking.kt): signed out, it offers to sign in; locked without biometric
// unlock, it asks for the master password in PassBundle's own dialog, naming
// the account when there's more than one, rather than losing the fill.
class UnlockingTest {
    @Test fun `it offers what the core says there is`() {
        assertEquals(Unlocking.Offer.NOTHING, Unlocking.offer(busy = true, signedOut = false, locked = false))
        // No account on the phone: the core says locked and signed out.
        assertEquals(Unlocking.Offer.SIGN_IN, Unlocking.offer(busy = false, signedOut = true, locked = true))
        assertEquals(Unlocking.Offer.UNLOCK, Unlocking.offer(busy = false, signedOut = false, locked = true))
        assertEquals(Unlocking.Offer.LOGINS, Unlocking.offer(busy = false, signedOut = false, locked = false))
    }

    @Test fun `biometric unlock when it's on, else the master password`() {
        assertEquals(Unlocking.Way.BIOMETRIC, Unlocking.way("setUp"))
        for (other in listOf("notSetUp", "unavailable", "")) {
            assertEquals(other, Unlocking.Way.MASTER_PASSWORD, Unlocking.way(other))
        }
    }

    @Test fun `an item that asks for the master password, or doesn't fill, says what it is`() {
        assertEquals("This login asks for your master password.", Unlocking.asks(Kind.LOGIN))
        assertEquals("This login asks for your master password.", Unlocking.asks(Kind.CODE))
        assertEquals("This card asks for your master password.", Unlocking.asks(Kind.CARD))
        assertEquals("This identity asks for your master password.", Unlocking.asks(Kind.IDENTITY))
        assertEquals("That login doesn't fill here.", Unlocking.doesntFill(Kind.CODE))
        assertEquals("That card can't be filled now.", Unlocking.doesntFill(Kind.CARD))
        assertEquals("That identity can't be filled now.", Unlocking.doesntFill(Kind.IDENTITY))
    }

    @Test fun `each kind of fill has the name FillActivity hears`() {
        assertEquals(listOf("login", "code", "card", "identity"), Kind.entries.map { it.id })
        for (kind in Kind.entries) assertEquals(kind, Kind.of(kind.id))
        assertEquals(null, Kind.of("password"))
        assertEquals(null, Kind.of(null))
    }

    @Test fun `the dialog names the account when there's more than one, and says why it's asking`() {
        assertEquals("Enter your master password to fill this in.", Unlocking.message(null))
        assertEquals("Enter the master password for Work to fill this in.", Unlocking.message("Work"))
        assertEquals(
            "Unlocking with fingerprint is off. Use your master password.\n\nEnter your master password to fill this in.",
            Unlocking.message("", "Unlocking with fingerprint is off. Use your master password."),
        )
    }
}
