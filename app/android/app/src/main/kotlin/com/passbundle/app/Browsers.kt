package com.passbundle.app

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.text.TextUtils
import android.util.AtomicFile
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.net.IDN
import java.security.MessageDigest

/**
 * Which apps may say what website their screen shows (docs/phones.md,
 * "Autofill"). Any app can put a website's domain on its own views, or show
 * the real site in a WebView of its own and read what fills, so a domain
 * counts only from a browser:
 *
 * - one on the list Android's Credential Manager uses for browsers
 *   (assets/browsers.json, Google's list from
 *   https://www.gstatic.com/gpm-passkeys-privileged-apps/apps.json, taken
 *   October 22, 2026; refreshed by downloading it over the file), by its
 *   package name and its release signing certificate. A certificate the list
 *   also names for a debug build anywhere doesn't count, since debug keys
 *   can be public;
 * - or one the person trusted on this device (FillActivity asks), by its
 *   package name and the certificates it's signed with when they did, so an
 *   app put in its place with another key isn't trusted. A package on the
 *   list that fails its certificate check is never offered, whatever it's
 *   called: that's an app posing as a known browser.
 *
 * Any other app fills only the logins for the app itself (androidapp://).
 */
object Browsers {
    private const val TRUSTED_FILE = "trusted-browsers.json"
    private var listed: Map<String, List<ByteArray>>? = null

    /** Whether this app's web domain counts. */
    fun trusts(context: Context, packageName: String): Boolean =
        onList(context, packageName) || trustedHere(context, packageName)

    /**
     * Whether the app may be offered trust as a browser: it opens any https
     * link (asked of the app itself, since Android leaves apps whose links it
     * hasn't verified out of an open query for a web link), and it isn't
     * using the name of a browser on the list without that browser's key.
     */
    fun couldBe(context: Context, packageName: String): Boolean {
        if (packageName in list(context)) return false
        val view =
            Intent(Intent.ACTION_VIEW, Uri.parse("https://passbundle.invalid/"))
                .addCategory(Intent.CATEGORY_BROWSABLE)
                .setPackage(packageName)
        return runCatching {
            context.packageManager.queryIntentActivities(view, PackageManager.MATCH_ALL).isNotEmpty()
        }.getOrDefault(false)
    }

    /**
     * A page's domain as it may be shown and matched: a hostname (letters,
     * digits, hyphens, and dots, as IDN's ASCII form), at most 253
     * characters. Anything else isn't a domain, and doesn't count.
     */
    fun domain(raw: String?): String? {
        val ascii = runCatching { IDN.toASCII(raw?.trim()?.trimEnd('.') ?: return null, IDN.ALLOW_UNASSIGNED) }.getOrNull()
            ?.lowercase() ?: return null
        val label = Regex("[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?")
        return ascii.takeIf { it.length in 1..253 && it.split('.').all { part -> label.matches(part) } }
    }

    /** Trusts an app as a browser on this device, by its package and the certificates it's signed with now. */
    @Synchronized
    fun trust(context: Context, packageName: String): Boolean {
        if (!couldBe(context, packageName)) return false
        val signers = signers(context, packageName)?.takeIf { it.isNotEmpty() } ?: return false
        return write(context, readTrusted(context).put(packageName, JSONArray(signers)))
    }

    /**
     * The app's name, as the launcher shows it, made safe to show: one line,
     * without characters that change how it reads, and not too long. Its
     * package name if it has none.
     */
    fun label(context: Context, packageName: String): String =
        runCatching {
            val pm = context.packageManager
            TextUtils.makeSafeForPresentation(
                pm.getApplicationLabel(pm.getApplicationInfo(packageName, 0)).toString(),
                1000,
                500f,
                // Only its first line (Android refuses this with SINGLE_LINE too).
                TextUtils.SAFE_STRING_FLAG_TRIM or TextUtils.SAFE_STRING_FLAG_FIRST_LINE,
            ).toString()
                .takeIf { it.isNotBlank() }
        }.getOrNull() ?: packageName

    private fun onList(context: Context, packageName: String): Boolean {
        val fingerprints = list(context)[packageName] ?: return false
        return fingerprints.any { hash -> hasCertificate(context, packageName, hash) }
    }

    private fun hasCertificate(context: Context, packageName: String, sha256: ByteArray): Boolean =
        runCatching {
            context.packageManager.hasSigningCertificate(packageName, sha256, PackageManager.CERT_INPUT_SHA256)
        }.getOrDefault(false)

    /**
     * Trusted here: signed now with exactly the certificates it had when the
     * person trusted it, or, for an app with one, a newer key that proves it
     * follows that one (Android's key rotation).
     */
    @Synchronized
    private fun trustedHere(context: Context, packageName: String): Boolean {
        val array = readTrusted(context).optJSONArray(packageName) ?: return false
        val kept = (0 until array.length()).map { array.getString(it) }.toSet()
        if (kept.isEmpty()) return false
        val now = signers(context, packageName)?.takeIf { it.isNotEmpty() } ?: return false
        if (now.toSet() == kept) return true
        return kept.size == 1 && now.size == 1 && hasCertificate(context, packageName, fromHex(kept.single()))
    }

    private fun file(context: Context) = AtomicFile(File(context.noBackupFilesDir, TRUSTED_FILE))

    /** What's kept; nothing if it's missing or unreadable (so nothing is trusted). */
    private fun readTrusted(context: Context): JSONObject =
        runCatching { JSONObject(String(file(context).readFully(), Charsets.UTF_8)) }.getOrDefault(JSONObject())

    private fun write(context: Context, all: JSONObject): Boolean {
        val file = file(context)
        val out = runCatching { file.startWrite() }.getOrNull() ?: return false
        return try {
            out.write(all.toString().toByteArray(Charsets.UTF_8))
            file.finishWrite(out)
            true
        } catch (_: java.io.IOException) {
            file.failWrite(out)
            false
        }
    }

    /** The SHA-256 of each certificate the app is signed with now, in hex; null when it can't be seen. */
    private fun signers(context: Context, packageName: String): List<String>? =
        runCatching {
            val info = context.packageManager.getPackageInfo(packageName, PackageManager.GET_SIGNING_CERTIFICATES)
            info.signingInfo?.apkContentsSigners?.map { signature ->
                MessageDigest.getInstance("SHA-256").digest(signature.toByteArray()).joinToString("") { "%02x".format(it) }
            }
        }.getOrNull()

    /**
     * The list's packages and their release certificates' SHA-256, leaving
     * out any certificate the list also names for a debug build.
     */
    @Synchronized
    private fun list(context: Context): Map<String, List<ByteArray>> =
        listed ?: runCatching {
            val apps = JSONObject(context.assets.open("browsers.json").bufferedReader().use { it.readText() }).getJSONArray("apps")
            val entries = (0 until apps.length()).map { apps.getJSONObject(it) }.filter { it.optString("type") == "android" }
            fun signatures(app: JSONObject) =
                app.getJSONObject("info").getJSONArray("signatures").let { s -> (0 until s.length()).map { s.getJSONObject(it) } }
            val debug =
                entries.flatMap(::signatures)
                    .filter { it.optString("build") != "release" }
                    .map { it.getString("cert_fingerprint_sha256").uppercase() }
                    .toSet()
            buildMap<String, List<ByteArray>> {
                for (app in entries) {
                    val release =
                        signatures(app)
                            .filter { it.optString("build") == "release" }
                            .map { it.getString("cert_fingerprint_sha256").uppercase() }
                            .filter { it !in debug }
                            .map(::fromHex)
                    val name = app.getJSONObject("info").getString("package_name")
                    put(name, (this[name] ?: emptyList()) + release)
                }
            }
        }.getOrDefault(emptyMap()).also { listed = it }

    private fun fromHex(text: String): ByteArray =
        text.replace(":", "").chunked(2).map { it.toInt(16).toByte() }.toByteArray()
}
