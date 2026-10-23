package com.passbundle.app.update

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

// After Vela's release tag tests (app/vela/update/ReleaseTagsTest.kt).
class ReleasesTest {
    @Test fun `tags give their version and build`() {
        assertEquals("0.1.12", Releases.versionOf("android-v0.1.12"))
        assertEquals(12, Releases.buildOfTag("android-v0.1.12"))
        assertEquals(240, Releases.buildOfTag("android-v0.2.240"))
        assertNull(Releases.versionOf("v0.1.12"))
        assertNull(Releases.buildOfTag("android-v0.1.12-rc"))
        assertNull(Releases.buildOfTag("desktop-v0.1.12"))
    }

    @Test fun `only a higher build is newer, whatever the version line`() {
        assertTrue(Releases.isNewer("android-v0.1.13", "0.1.12"))
        assertFalse(Releases.isNewer("android-v0.1.12", "0.1.12"))
        assertFalse(Releases.isNewer("android-v0.1.11", "0.1.12"))
        assertTrue("a new version line keeps counting builds", Releases.isNewer("android-v0.2.14", "0.1.13"))
        assertFalse(Releases.isNewer("v0.1.99", "0.1.12"))
    }

    @Test fun `a build of its own is older than any release`() {
        assertTrue(Releases.isNewer("android-v0.1.1", "0.1.0"))
        assertTrue(Releases.isNewer("android-v0.1.1", null))
        assertTrue(Releases.isNewer("android-v0.1.1", "dev"))
    }

    @Test fun `the newest Android release among others`() {
        val tags = listOf("android-v0.1.9", "desktop-v0.3.0", "android-v0.1.12", "android-v0.1.10", "canary")
        assertEquals("android-v0.1.12", Releases.newest(tags))
        assertNull(Releases.newest(listOf("desktop-v0.3.0")))
    }

    @Test fun `what's new is the notes' list`() {
        val body = "What's new since 0.1.11:\r\n\r\n- Faster sync\r\n- Settings has updates\r\n\r\nFiles: the APKs and SHA256SUMS."
        assertEquals("- Faster sync\n- Settings has updates", Releases.changes(body))
        assertEquals("Just words.", Releases.changes("  Just words.\n"))
        val long = (1..500).joinToString("\n") { "- change $it" }
        assertTrue(Releases.changes(long, max = 100).length <= 102)
        assertTrue(Releases.changes(long, max = 100).endsWith("\n…"))
    }
}
