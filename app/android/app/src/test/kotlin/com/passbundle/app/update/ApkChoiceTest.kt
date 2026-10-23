package com.passbundle.app.update

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

// Ported from Vela's updater tests (app/vela/update/ApkChoiceTest.kt).
class ApkChoiceTest {
    private val release = listOf("SHA256SUMS", "passbundle-0.1.12-arm64-v8a.apk", "passbundle-0.1.12-x86_64.apk")
    private val arm64Phone = listOf("arm64-v8a", "armeabi-v7a", "armeabi")

    @Test fun `each phone gets its own chip type`() {
        assertEquals("passbundle-0.1.12-arm64-v8a.apk", ApkChoice.pick(release, arm64Phone))
        assertEquals("passbundle-0.1.12-arm64-v8a.apk", ApkChoice.pick(release, listOf("arm64-v8a")))
        assertEquals("passbundle-0.1.12-x86_64.apk", ApkChoice.pick(release, listOf("x86_64", "x86", "arm64-v8a", "armeabi-v7a")))
    }

    @Test fun `the phone's own order decides, best first`() {
        // A Chromebook that runs ARM apps too still gets its own.
        assertEquals("passbundle-0.1.12-x86_64.apk", ApkChoice.pick(release.reversed(), listOf("x86_64", "arm64-v8a")))
        // A chip type with no file of its own takes the next one the phone runs.
        assertEquals("passbundle-0.1.12-arm64-v8a.apk", ApkChoice.pick(release, listOf("riscv64", "arm64-v8a")))
    }

    @Test fun `a phone with no file for it gets nothing, not another chip's`() {
        assertNull(ApkChoice.pick(release, listOf("armeabi-v7a", "armeabi")))
        assertNull(ApkChoice.pick(release, listOf("x86")))
    }

    @Test fun `an APK for every chip type is the fallback`() {
        val withAll = release + "passbundle-0.1.12.apk"
        assertEquals("passbundle-0.1.12.apk", ApkChoice.pick(withAll, listOf("armeabi-v7a")))
        assertEquals("passbundle-0.1.12-arm64-v8a.apk", ApkChoice.pick(withAll, arm64Phone))
    }

    @Test fun `only APKs count`() {
        assertNull(ApkChoice.pick(listOf("SHA256SUMS", "notes.txt", "passbundle-0.1.12-arm64-v8a.apk.sig"), arm64Phone))
    }
}
