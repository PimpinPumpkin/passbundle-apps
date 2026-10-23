package com.passbundle.app.update

import java.io.File
import java.security.MessageDigest

/**
 * Is a downloaded update the file the release says it is? Ported from Vela's
 * updater (app/vela/update/ApkCheck.kt). The signing certificate is checked
 * after this, by Updater, with Android's own reading of the APK.
 */
object ApkCheck {
    /** The most an update may be: far more than PassBundle is (about 25 MB). */
    const val MAX_BYTES = 150L shl 20

    /**
     * The most to download for a release that says its file is [size] bytes
     * (0 when it doesn't say): that size, or [MAX_BYTES]; null when the
     * release says more than [MAX_BYTES], which isn't downloaded at all.
     */
    fun limit(size: Long): Long? =
        when {
            size > MAX_BYTES -> null
            size > 0 -> size
            else -> MAX_BYTES
        }

    /** GitHub's asset `digest` ("sha256:<hex>") as bare lowercase hex, or null for anything else. */
    fun digestOf(digest: String?): String? = hex(digest?.trim()?.lowercase()?.removePrefix("sha256:"))

    /** Lowercase SHA-256 hex, or null if [text] isn't one. */
    fun hex(text: String?): String? = text?.lowercase()?.takeIf { it.length == 64 && it.all { c -> c in '0'..'9' || c in 'a'..'f' } }

    /**
     * SHA256SUMS as `sha256sum` writes it ("<hex>  <name>", or "<hex> *<name>"
     * in binary mode), as file name to lowercase hex. Lines that aren't one
     * are skipped.
     */
    fun sums(text: String): Map<String, String> =
        text.lineSequence()
            .mapNotNull { line -> Regex("""^([0-9A-Fa-f]{64}) [ *](.+)$""").find(line.trimEnd('\r'))?.groupValues }
            .associate { (_, hash, name) -> name.trim() to hash.lowercase() }

    fun sha256(file: File): String {
        val md = MessageDigest.getInstance("SHA-256")
        file.inputStream().use { s ->
            val buf = ByteArray(256 * 1024)
            while (true) {
                val n = s.read(buf)
                if (n < 0) break
                md.update(buf, 0, n)
            }
        }
        return md.digest().joinToString("") { "%02x".format(it) }
    }

    /**
     * What is wrong with [file] as the update, or null when it checks out:
     * it's a zip (an APK is one), it's [expectBytes] long when the release
     * gives a size, and its SHA-256 is [expectSha256], which is required.
     * The hash is read last: it's the slow one.
     */
    fun problem(file: File, expectBytes: Long, expectSha256: String): String? {
        if (!file.exists() || file.length() <= 4) return "empty"
        val magic = ByteArray(2).also { m -> file.inputStream().use { it.read(m) } }
        if (magic[0] != 'P'.code.toByte() || magic[1] != 'K'.code.toByte()) return "not an APK"
        if (expectBytes > 0 && file.length() != expectBytes) return "size ${file.length()}, the release says $expectBytes"
        if (sha256(file) != expectSha256) return "SHA-256 doesn't match the release's SHA256SUMS"
        return null
    }
}
