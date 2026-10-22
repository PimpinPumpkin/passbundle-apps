package com.passbundle.app

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
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
 *   package name and its release signing certificate;
 * - or one the person trusted on this device (FillActivity asks), by its
 *   package name and the certificates it's signed with when they did, so an
 *   app put in its place with another key isn't trusted.
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
     * Whether the app could be a browser: it opens any https link. Only these
     * are offered to be trusted, so other apps' screens don't ask. Asked of
     * the app itself, since Android leaves apps whose links it hasn't
     * verified out of an open query for a web link.
     */
    fun couldBe(context: Context, packageName: String): Boolean {
        val view =
            Intent(Intent.ACTION_VIEW, Uri.parse("https://passbundle.invalid/"))
                .addCategory(Intent.CATEGORY_BROWSABLE)
                .setPackage(packageName)
        return runCatching {
            context.packageManager.queryIntentActivities(view, PackageManager.MATCH_ALL).isNotEmpty()
        }.getOrDefault(false)
    }

    /** Trusts an app as a browser on this device, by its package and the certificates it's signed with now. */
    @Synchronized
    fun trust(context: Context, packageName: String): Boolean {
        val signers = signers(context, packageName)?.takeIf { it.isNotEmpty() } ?: return false
        val all = readTrusted(context).put(packageName, JSONArray(signers))
        val file = File(context.noBackupFilesDir, TRUSTED_FILE)
        val next = File(context.noBackupFilesDir, "$TRUSTED_FILE.new")
        next.writeText(all.toString())
        return next.renameTo(file)
    }

    /** The app's name, as the launcher shows it, or its package name. */
    fun label(context: Context, packageName: String): String =
        runCatching {
            val pm = context.packageManager
            pm.getApplicationLabel(pm.getApplicationInfo(packageName, 0)).toString()
        }.getOrDefault(packageName)

    private fun onList(context: Context, packageName: String): Boolean {
        val fingerprints = list(context)[packageName] ?: return false
        return fingerprints.any { hash ->
            runCatching {
                context.packageManager.hasSigningCertificate(packageName, hash, PackageManager.CERT_INPUT_SHA256)
            }.getOrDefault(false)
        }
    }

    @Synchronized
    private fun trustedHere(context: Context, packageName: String): Boolean {
        val kept = readTrusted(context).optJSONArray(packageName) ?: return false
        val now = signers(context, packageName)?.takeIf { it.isNotEmpty() } ?: return false
        return now.toSet() == (0 until kept.length()).map { kept.getString(it) }.toSet()
    }

    private fun readTrusted(context: Context): JSONObject =
        runCatching { JSONObject(File(context.noBackupFilesDir, TRUSTED_FILE).readText()) }.getOrDefault(JSONObject())

    /** The SHA-256 of each certificate the app is signed with now, in hex; null when it can't be seen. */
    private fun signers(context: Context, packageName: String): List<String>? =
        runCatching {
            val info = context.packageManager.getPackageInfo(packageName, PackageManager.GET_SIGNING_CERTIFICATES)
            info.signingInfo?.apkContentsSigners?.map { signature ->
                MessageDigest.getInstance("SHA-256").digest(signature.toByteArray()).joinToString("") { "%02x".format(it) }
            }
        }.getOrNull()

    /** The list's packages and their release certificates' SHA-256. */
    @Synchronized
    private fun list(context: Context): Map<String, List<ByteArray>> =
        listed ?: runCatching {
            val apps = JSONObject(context.assets.open("browsers.json").bufferedReader().use { it.readText() }).getJSONArray("apps")
            buildMap<String, List<ByteArray>> {
                for (i in 0 until apps.length()) {
                    val app = apps.getJSONObject(i)
                    if (app.optString("type") != "android") continue
                    val info = app.getJSONObject("info")
                    val signatures = info.getJSONArray("signatures")
                    val release =
                        (0 until signatures.length())
                            .map { signatures.getJSONObject(it) }
                            .filter { it.optString("build") == "release" }
                            .map { fromHex(it.getString("cert_fingerprint_sha256")) }
                    val name = info.getString("package_name")
                    put(name, (this[name] ?: emptyList()) + release)
                }
            }
        }.getOrDefault(emptyMap()).also { listed = it }

    private fun fromHex(colons: String): ByteArray =
        colons.split(":").map { it.toInt(16).toByte() }.toByteArray()
}
