package com.passbundle.app.update

/**
 * Which APK on a release fits this phone. A release carries one APK per
 * chip type, named `passbundle-<version>-<abi>.apk` (arm64-v8a for phones,
 * x86_64 for the emulator and Chromebooks), next to SHA256SUMS. Ported from
 * Vela's updater (app/vela/update/ApkChoice.kt).
 */
object ApkChoice {
    /**
     * The asset to download from [names], trying [supportedAbis] in the
     * phone's own order (`Build.SUPPORTED_ABIS`, the best first), then an APK
     * for every chip type (one with no chip type in its name), then null
     * when there's no APK for this phone.
     */
    fun pick(names: List<String>, supportedAbis: List<String>): String? {
        val apks = names.filter { it.endsWith(".apk") }
        for (abi in supportedAbis) {
            apks.firstOrNull { it.endsWith("-$abi.apk") }?.let { return it }
        }
        return apks.firstOrNull { name -> KNOWN_ABIS.none { name.endsWith("-$it.apk") } }
    }

    /** Android's chip types, as release file names carry them. */
    private val KNOWN_ABIS = listOf("arm64-v8a", "armeabi-v7a", "armeabi", "x86_64", "x86", "riscv64")
}
