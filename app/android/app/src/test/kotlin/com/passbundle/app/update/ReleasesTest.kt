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
        assertNull(Releases.versionOf("macos-v0.1.12"))
        assertNull(Releases.buildOfTag("macos-v0.1.12"))
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

    @Test fun `Mac releases, with higher build numbers of their own, are never Android's`() {
        // The Mac's releases (macos-release.yml) count their own builds, and
        // come out alongside Android's: never an update for a phone.
        val tags = listOf("macos-v0.1.40", "android-v0.1.12", "macos-v0.1.41", "android-v0.1.11")
        assertEquals("android-v0.1.12", Releases.newest(tags))
        assertFalse(Releases.isNewer("macos-v0.1.41", "0.1.12"))
        assertNull(Releases.newest(listOf("macos-v0.1.40")))
    }

    @Test fun `what's new is the notes' list`() {
        val body = "What's new since 0.1.11:\r\n\r\n- Faster sync\r\n- Settings has updates\r\n\r\nFiles: the APKs and SHA256SUMS."
        assertEquals("- Faster sync\n- Settings has updates", Releases.changes(body))
        assertEquals("Just words.", Releases.changes("  Just words.\n"))
        val long = (1..500).joinToString("\n") { "- change $it" }
        assertTrue(Releases.changes(long, max = 100).length <= 102)
        assertTrue(Releases.changes(long, max = 100).endsWith("\n…"))
    }

    @Test fun `only files on PassBundle's own releases are downloaded`() {
        val base = "https://github.com/PimpinPumpkin/passbundle-apps/releases/download"
        assertTrue(Releases.isReleaseDownload("$base/android-v0.1.12/passbundle-0.1.12-arm64-v8a.apk"))
        assertTrue(Releases.isReleaseDownload("$base/android-v0.1.12/SHA256SUMS"))
        // Another repository, another host, or not HTTPS.
        assertFalse(Releases.isReleaseDownload("https://github.com/someone/passbundle-apps/releases/download/android-v0.1.12/x.apk"))
        assertFalse(Releases.isReleaseDownload("https://github.com/PimpinPumpkin/passbundle-apps-fork/releases/download/android-v0.1.12/x.apk"))
        assertFalse(Releases.isReleaseDownload("https://github.com.example.test/PimpinPumpkin/passbundle-apps/releases/download/a/b.apk"))
        assertFalse(Releases.isReleaseDownload("https://example.test/PimpinPumpkin/passbundle-apps/releases/download/a/b.apk"))
        assertFalse(Releases.isReleaseDownload("http://github.com/PimpinPumpkin/passbundle-apps/releases/download/a/b.apk"))
        // Paths that would leave the releases once a client tidies them.
        assertFalse(Releases.isReleaseDownload("$base/../../../other/repo/b.apk"))
        assertFalse(Releases.isReleaseDownload("$base/android-v0.1.12/../b.apk"))
        assertFalse(Releases.isReleaseDownload("$base/../b.apk"))
        assertFalse(Releases.isReleaseDownload("$base/%2e%2e/b.apk"))
        // Anything but a tag and a file name.
        assertFalse(Releases.isReleaseDownload("$base/android-v0.1.12/b.apk?x=1"))
        assertFalse(Releases.isReleaseDownload("$base/android-v0.1.12/sub/b.apk"))
        assertFalse(Releases.isReleaseDownload("$base/android-v0.1.12/"))
        assertFalse(Releases.isReleaseDownload("$base/b.apk"))
    }

    @Test fun `a download ends only at GitHub's own hosts`() {
        assertTrue(Releases.isDownloadHost("github.com"))
        assertTrue(Releases.isDownloadHost("objects.githubusercontent.com"))
        assertTrue(Releases.isDownloadHost("release-assets.githubusercontent.com"))
        assertTrue(Releases.isDownloadHost("GitHub.com"))
        assertFalse(Releases.isDownloadHost("api.github.com"))
        assertFalse(Releases.isDownloadHost("githubusercontent.com"))
        assertFalse(Releases.isDownloadHost("evil.objects.githubusercontent.com"))
        assertFalse(Releases.isDownloadHost("example.test"))
        assertFalse(Releases.isDownloadHost(null))
        assertTrue(Releases.isApiHost("api.github.com"))
        assertFalse(Releases.isApiHost("github.com"))
        assertFalse(Releases.isApiHost(null))
    }

    @Test fun `the APK must be the version its tag names`() {
        assertTrue(Releases.isSameVersion("0.1.12", "0.1.12"))
        assertFalse("an older build behind a newer tag", Releases.isSameVersion("0.1.11", "0.1.12"))
        assertFalse(Releases.isSameVersion("0.1.12-debug", "0.1.12"))
        assertFalse(Releases.isSameVersion(null, "0.1.12"))
    }
}
