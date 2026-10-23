package com.passbundle.app.update

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

// Adapted from Vela's updater tests (app/vela/update/InstallSourceTest.kt):
// there, a Play install is the car workaround; here, any store keeps
// PassBundle up to date itself, so the updater stays off.
class InstallSourceTest {
    @Test fun aStoreInstallTurnsTheUpdaterOff() {
        assertEquals("Google Play", InstallSource.store("com.android.vending", "com.android.vending"))
        assertEquals("F-Droid", InstallSource.store("org.fdroid.fdroid", "org.fdroid.fdroid"))
        assertEquals("F-Droid", InstallSource.store("org.fdroid.basic", null))
        assertEquals("Accrescent", InstallSource.store("app.accrescent.client", "app.accrescent.client"))
    }

    @Test fun eitherRecordNamingAStoreCounts() {
        assertEquals("adb install -i: the installer says Play", "Google Play", InstallSource.store("com.android.vending", "com.android.shell"))
        assertEquals("started by a store, installer not recorded", "Aurora Store", InstallSource.store(null, "com.aurora.store"))
    }

    @Test fun anyOtherInstallKeepsItOn() {
        assertNull("adb", InstallSource.store(null, null))
        assertNull("a download, through Android's installer", InstallSource.store("com.google.android.packageinstaller", "com.android.chrome"))
        assertNull("Obtainium takes the same GitHub releases", InstallSource.store("dev.imranr.obtainium", "dev.imranr.obtainium"))
        assertNull("the updater itself", InstallSource.store("com.passbundle.app", "com.passbundle.app"))
    }
}
