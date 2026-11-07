package com.passbundle.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

// Two-step login on Android (SecurityKeys.kt): which USB interface is a
// security key's, as the core decides on Linux (core/src/security_key/mod.rs),
// and a YubiKey's one-time code from the address it gives over NFC.
class SecurityKeysTest {
    private fun bytes(vararg values: Int) = ByteArray(values.size) { values[it].toByte() }

    @Test fun `a security key's report descriptor names the FIDO usage page`() {
        // A YubiKey's FIDO interface: Usage Page (FIDO Alliance), Usage (U2F Authenticator Device), and its reports.
        assertTrue(SecurityKeys.isFido(bytes(0x06, 0xd0, 0xf1, 0x09, 0x01, 0xa1, 0x01, 0x09, 0x20, 0x15, 0x00, 0x26, 0xff, 0x00, 0xc0)))
        // A keyboard, as a YubiKey's code interface is.
        assertFalse(SecurityKeys.isFido(bytes(0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0xc0)))
        // Cut short, and a long item first: never read past the end.
        assertFalse(SecurityKeys.isFido(bytes(0x06, 0xd0)))
        assertFalse(SecurityKeys.isFido(bytes(0xfe, 0x10, 0x00)))
        assertTrue(SecurityKeys.isFido(bytes(0xfe, 0x01, 0x00, 0xaa, 0x06, 0xd0, 0xf1)))
    }

    @Test fun `a YubiKey's code is read from the end of its address`() {
        val code = "cccccbhuinjdtrbftdrldjlrgkdeubffnkhjfjifvlbr"
        assertEquals(code, SecurityKeys.codeFrom("https://my.yubico.com/yk/#$code"))
        assertEquals(code, SecurityKeys.codeFrom("https://my.yubico.com/neo/#${code.uppercase()}"))
        assertNull(SecurityKeys.codeFrom("https://my.yubico.com/yk/"))
        assertNull(SecurityKeys.codeFrom("https://example.test/#123456"))
    }
}
