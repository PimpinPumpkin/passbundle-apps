package com.passbundle.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

// Push for sign-in requests while PassBundle is closed (Push.kt): what a
// message says, read only from exactly what the server sends; what the phone
// does with the core's answer (a notification for something waiting, none
// for nothing, the registration dropped for a session that's over, and
// nothing changed when the server can't be reached); when an account
// registers again; and the notification's words, the prompt's own, which
// name no account, device, or place.
class PushTest {
    private fun bytes(text: String) = text.toByteArray(Charsets.UTF_8)

    @Test fun `a message's kind is read only from what the server sends`() {
        assertEquals("sign-in", PushPlan.kind(bytes("""{"kind":"sign-in"}""")))
        assertEquals("sign-in", PushPlan.kind(bytes(""" { "kind" : "sign-in" } """)))
        assertEquals("sync", PushPlan.kind(bytes("""{"kind":"sync"}""")))
        // Anything more, or else, isn't one.
        assertNull(PushPlan.kind(bytes("""{"kind":"sign-in","email":"ana@example.test"}""")))
        assertNull(PushPlan.kind(bytes("""{"kind":"Sign-In"}""")))
        assertNull(PushPlan.kind(bytes("""{"kind":""}""")))
        assertNull(PushPlan.kind(bytes("""{"kind":"${"a".repeat(33)}"}""")))
        assertNull(PushPlan.kind(bytes("sign-in")))
        assertNull(PushPlan.kind(bytes("")))
        assertNull(PushPlan.kind(byteArrayOf(0xff.toByte(), 0x00)))
    }

    @Test fun `what the phone does with the core's answer`() {
        assertEquals(PushPlan.Then.NOTIFY, PushPlan.then(ok = true, signedOut = false, requests = 1, checks = 0))
        assertEquals(PushPlan.Then.NOTIFY, PushPlan.then(ok = true, signedOut = false, requests = 0, checks = 2))
        // Answered elsewhere, or gone: one showing goes.
        assertEquals(PushPlan.Then.CLEAR, PushPlan.then(ok = true, signedOut = false, requests = 0, checks = 0))
        // The session is over: nothing shows, and the registration goes.
        assertEquals(PushPlan.Then.FORGET, PushPlan.then(ok = false, signedOut = true, requests = 0, checks = 0))
        // Offline, or the server said something else: as it was.
        assertEquals(PushPlan.Then.NOTHING, PushPlan.then(ok = false, signedOut = false, requests = 0, checks = 0))
    }

    @Test fun `an account registers again when the server's key changes or it lost the endpoint`() {
        // Never registered.
        assertTrue(PushPlan.needsRegistering(enabled = true, serverKey = "BKey", registeredWith = null, serverHasIt = false))
        // Registered with this key, and the server has it: nothing to do.
        assertFalse(PushPlan.needsRegistering(enabled = true, serverKey = "BKey", registeredWith = "BKey", serverHasIt = true))
        // The server's key was replaced.
        assertTrue(PushPlan.needsRegistering(enabled = true, serverKey = "BNew", registeredWith = "BKey", serverHasIt = true))
        // A push service said the endpoint was gone, and the server dropped it.
        assertTrue(PushPlan.needsRegistering(enabled = true, serverKey = "BKey", registeredWith = "BKey", serverHasIt = false))
        // A server that doesn't push, or has no key.
        assertFalse(PushPlan.needsRegistering(enabled = false, serverKey = "BKey", registeredWith = null, serverHasIt = false))
        assertFalse(PushPlan.needsRegistering(enabled = true, serverKey = null, registeredWith = null, serverHasIt = false))
        assertFalse(PushPlan.needsRegistering(enabled = true, serverKey = "", registeredWith = null, serverHasIt = false))
    }

    @Test fun `the notification says the prompt's words, and nothing about whose or where`() {
        assertEquals("Approve sign-in?", PushPlan.TITLE)
        assertEquals("Open PassBundle to review it.", PushPlan.TEXT)
        for (text in listOf(PushPlan.TITLE, PushPlan.TEXT)) {
            assertFalse(text.contains("@"))
        }
    }
}
