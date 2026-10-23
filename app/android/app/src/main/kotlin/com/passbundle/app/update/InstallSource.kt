package com.passbundle.app.update

import android.content.Context
import android.os.Build

/**
 * Who Android thinks installed PassBundle. An app store that installed it
 * keeps it up to date, with its own build and often its own signing key, so
 * the updater stays off then: an update from GitHub would fail the key check
 * or take the app away from the store. Installed any other way (a download
 * from GitHub, adb, Obtainium, or the updater itself), it's on. Adapted from
 * Vela's updater (app/vela/update/InstallSource.kt), which reads the same
 * two records.
 */
object InstallSource {
    /** App stores, by package, and what to call them. */
    private val STORES =
        mapOf(
            "com.android.vending" to "Google Play",
            "org.fdroid.fdroid" to "F-Droid",
            "org.fdroid.basic" to "F-Droid",
            "org.fdroid.fdroid.privileged" to "F-Droid",
            "com.looker.droidify" to "Droid-ify",
            "com.machiav3lli.fdroid" to "Neo Store",
            "com.aurora.store" to "Aurora Store",
            "app.accrescent.client" to "Accrescent",
            "com.amazon.venezia" to "the Amazon Appstore",
            "com.sec.android.app.samsungapps" to "the Galaxy Store",
            "com.huawei.appmarket" to "AppGallery",
            "com.xiaomi.market" to "GetApps",
        )

    /**
     * The store that installed PassBundle, by name, or null when none did.
     * Either record counts (Android 11 and later keep who performed the
     * install and who started it), as in Vela.
     */
    fun store(installing: String?, initiating: String?): String? = STORES[installing] ?: STORES[initiating]

    fun store(context: Context): String? = store(installingPackage(context), initiatingPackage(context))

    /** Who installed PassBundle, as Android records it; null when nothing claims the install. */
    private fun installingPackage(context: Context): String? =
        runCatching {
            val pm = context.packageManager
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                pm.getInstallSourceInfo(context.packageName).installingPackageName
            } else {
                @Suppress("DEPRECATION")
                pm.getInstallerPackageName(context.packageName)
            }
        }.getOrNull()

    /** Who started the install (Android 11 and later). */
    private fun initiatingPackage(context: Context): String? =
        runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                context.packageManager.getInstallSourceInfo(context.packageName).initiatingPackageName
            } else {
                null
            }
        }.getOrNull()
}
