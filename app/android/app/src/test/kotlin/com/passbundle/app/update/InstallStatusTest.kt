package com.passbundle.app.update

import android.content.pm.PackageInstaller
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

// A failed update says what happened and what to do, in plain words, and
// never Android's status number (InstallStatus.failure).
class InstallStatusTest {
    private val failures =
        listOf(
            PackageInstaller.STATUS_FAILURE,
            PackageInstaller.STATUS_FAILURE_ABORTED,
            PackageInstaller.STATUS_FAILURE_BLOCKED,
            PackageInstaller.STATUS_FAILURE_CONFLICT,
            PackageInstaller.STATUS_FAILURE_INCOMPATIBLE,
            PackageInstaller.STATUS_FAILURE_INVALID,
            PackageInstaller.STATUS_FAILURE_STORAGE,
            42,
        )

    @Test fun everyFailureSaysWhatToDoWithoutAStatusNumber() {
        for (status in failures) {
            val words = InstallStatus.failure(status)
            assertFalse("$status: $words", words.contains("status"))
            assertFalse("$status: $words", Regex("[0-9]").containsMatchIn(words))
            assertTrue("$status: $words", words.endsWith("."))
        }
    }

    @Test fun eachKnownFailureHasItsOwnWords() {
        assertEquals(
            "There isn't enough space on this phone for the update. Free some space, then tap Update.",
            InstallStatus.failure(PackageInstaller.STATUS_FAILURE_STORAGE),
        )
        assertEquals(
            "The update's file was damaged on the way. Tap Update to download it again.",
            InstallStatus.failure(PackageInstaller.STATUS_FAILURE_INVALID),
        )
        assertEquals("Android didn't install the update. Tap Update to try again.", InstallStatus.failure(42))
    }
}
