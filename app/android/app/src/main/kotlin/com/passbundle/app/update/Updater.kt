package com.passbundle.app.update

import android.app.Activity
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageInfo
import android.content.pm.PackageInstaller
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLConnection
import java.security.MessageDigest

/**
 * Updates from PassBundle's GitHub releases (README, "Android test
 * releases"), the pattern of Vela's updater (app/vela/update/SelfUpdater.kt):
 * the newest release, when it's newer than this build, offered in Settings;
 * downloaded over HTTPS on a tap, checked against the release's SHA256SUMS,
 * checked to be PassBundle signed with this app's own key, and handed to
 * Android's installer, which asks the person before it installs anything.
 *
 * It contacts api.github.com, and github.com for the files (which GitHub
 * hands to objects.githubusercontent.com or
 * release-assets.githubusercontent.com), and no other host, only to check or
 * download: when the person taps "Check for updates" or "Update", and at
 * most once a day when the app starts, if the person leaves that on. It's
 * off when an app store installed PassBundle (InstallSource), and debug
 * builds don't check by themselves (the emulator tests stay offline).
 */
object Updater {
    private const val REPO = "https://api.github.com/repos/PimpinPumpkin/passbundle-apps"
    private const val LATEST = "$REPO/releases/latest"
    private const val LIST = "$REPO/releases?per_page=30"
    private const val SUMS = "SHA256SUMS"
    private const val DAY_MS = 24 * 60 * 60 * 1000L

    /** Room left on the phone besides the download, for installing it. */
    private const val SPACE_TO_SPARE = 64L shl 20

    /** The longest a download may take, however it goes. */
    private const val DOWNLOAD_MINUTES = 15L

    /** An update the last check found: the release and this phone's file on it. */
    private class Offer(
        val version: String,
        val apkName: String,
        val apkUrl: String,
        val size: Long,
        // GitHub's own SHA-256 of the file, when the release answer has it.
        val digest: String?,
        val sumsUrl: String,
        val notes: String,
        // From a debug build's test source (testSource), which may be anywhere.
        val test: Boolean,
    )

    @Volatile private var offer: Offer? = null

    @Volatile private var downloading = false

    /** Tells Dart how a download goes and what Android's installer said (MainActivity). */
    @Volatile var listener: ((String, Map<String, Any?>) -> Unit)? = null

    private val main = Handler(Looper.getMainLooper())

    /** This build's version, the store that installed it if one did, and whether it checks once a day. */
    fun status(context: Context): Map<String, Any?> =
        mapOf(
            "version" to versionName(context),
            "store" to InstallSource.store(context),
            "auto" to prefs(context).getBoolean("auto", true),
        )

    fun setAuto(context: Context, on: Boolean) {
        prefs(context).edit().putBoolean("auto", on).apply()
    }

    /**
     * Looks for a newer release (slow: off the main thread): `{ok, available,
     * version, notes}`, or `{ok: false, error}`. [quiet] is the check when the
     * app starts, which happens at most once a day, and never in a debug
     * build or with the person's daily check off: then `{ok, available:
     * false, skipped: true}`.
     */
    fun check(context: Context, quiet: Boolean): Map<String, Any?> {
        val skipped = mapOf("ok" to true, "available" to false, "skipped" to true)
        if (InstallSource.store(context) != null) return skipped
        val prefs = prefs(context)
        val now = System.currentTimeMillis()
        if (quiet) {
            if (debuggable(context) || !prefs.getBoolean("auto", true)) return skipped
            val last = prefs.getLong("lastCheck", 0)
            if (now >= last && now - last < DAY_MS) return skipped
        }
        prefs.edit().putLong("lastCheck", now).apply()
        offer = null
        return try {
            val found = newer(context)
            offer = found
            if (found == null) {
                mapOf("ok" to true, "available" to false)
            } else {
                mapOf("ok" to true, "available" to true, "version" to found.version, "notes" to found.notes)
            }
        } catch (e: NoFile) {
            mapOf("ok" to false, "error" to e.message)
        } catch (_: Exception) {
            mapOf("ok" to false, "error" to "Couldn't check for updates. Check your connection and try again.")
        }
    }

    /**
     * Downloads the update the last check found, checks it, and hands it to
     * Android's installer, which asks the person: `{ok}` once it's handed
     * over, or `{ok: false, error}`. What the installer says comes later,
     * through [listener] ("installed"); updating, Android closes the app.
     */
    fun install(activity: Activity, done: (Map<String, Any?>) -> Unit) {
        val offer = offer ?: return done(fail("Check for updates first."))
        if (downloading) return done(fail("The update is already downloading."))
        // Android asks once whether PassBundle may install apps; its own setting, not a prompt.
        if (!activity.packageManager.canRequestPackageInstalls()) {
            runCatching {
                activity.startActivity(
                    Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${activity.packageName}")),
                )
            }
            return done(fail("Allow PassBundle to install apps in the setting Android opened, then tap Update again."))
        }
        downloading = true
        val context = activity.applicationContext
        Thread {
            val answer =
                try {
                    fetchCheckAndInstall(context, offer)
                } catch (_: Exception) {
                    fail("Couldn't download the update. Check your connection and try again.")
                } finally {
                    downloading = false
                }
            activity.runOnUiThread { done(answer) }
        }.start()
    }

    /** The newest release, if it's newer than this build and has a file for this phone. */
    private fun newer(context: Context): Offer? {
        val source = testSource(context)
        val test = source != null
        var release = getJson(context, source ?: LATEST, test) ?: return null
        // releases/latest is another platform's release: the newest Android one instead.
        if (!test && Releases.versionOf(release.optString("tag_name")) == null) {
            val list = JSONArray(getText(context, LIST, 4 shl 20, Releases::isApiHost, test) ?: return null)
            val shipped = (0 until list.length()).map { list.getJSONObject(it) }.filter { !it.optBoolean("draft") && !it.optBoolean("prerelease") }
            val tag = Releases.newest(shipped.map { it.optString("tag_name") }) ?: return null
            release = shipped.first { it.optString("tag_name") == tag }
        }
        val tag = release.optString("tag_name")
        val version = Releases.versionOf(tag) ?: return null
        if (!Releases.isNewer(tag, versionName(context))) return null
        val assets = release.optJSONArray("assets")?.let { a -> (0 until a.length()).map { a.getJSONObject(it) } } ?: emptyList()
        val name =
            ApkChoice.pick(assets.map { it.optString("name") }, Build.SUPPORTED_ABIS.toList())
                ?: throw NoFile("PassBundle $version has no download for this phone.")
        val apk = assets.first { it.optString("name") == name }
        val sums = assets.firstOrNull { it.optString("name") == SUMS } ?: throw NoFile("PassBundle $version has no checksums, so it can't be installed from here.")
        val apkUrl = apk.getString("browser_download_url")
        val sumsUrl = sums.getString("browser_download_url")
        // Only files on PassBundle's own releases (a test's may be anywhere).
        if (!test && !(Releases.isReleaseDownload(apkUrl) && Releases.isReleaseDownload(sumsUrl))) {
            throw NoFile("PassBundle $version's files aren't on PassBundle's releases, so it can't be installed from here.")
        }
        return Offer(
            version = version,
            apkName = name,
            apkUrl = apkUrl,
            size = apk.optLong("size"),
            digest = ApkCheck.digestOf(apk.optString("digest")),
            sumsUrl = sumsUrl,
            notes = Releases.changes(release.optString("body")),
            test = test,
        )
    }

    private class NoFile(message: String) : Exception(message)

    private fun fetchCheckAndInstall(context: Context, offer: Offer): Map<String, Any?> {
        // Never more than a release says, nor than any PassBundle could be.
        val limit = ApkCheck.limit(offer.size) ?: return fail("The update is larger than PassBundle accepts, so it wasn't downloaded.")
        val expected =
            ApkCheck.sums(
                getText(context, offer.sumsUrl, 64 * 1024, Releases::isDownloadHost, offer.test) ?: return fail("Couldn't download the update's checksums."),
            )[offer.apkName]
                ?: return fail("The update isn't in its release's checksums, so it wasn't installed.")
        if (offer.digest != null && offer.digest != expected) {
            return fail("The update's checksums don't agree, so it wasn't installed.")
        }
        // One update kept at a time: a good copy from before is used again
        // (Android's prompt goes away if the screen locks), anything else goes.
        // Names of our own: a release's file name never becomes a path here.
        val dir = File(context.cacheDir, "updates").apply { mkdirs() }
        val apk = File(dir, "update.apk")
        val part = File(dir, "update.apk.part")
        dir.listFiles()?.forEach { if (it != apk) it.delete() }
        if (!apk.exists() || ApkCheck.problem(apk, offer.size, expected) != null) {
            apk.delete()
            if (dir.usableSpace < limit + SPACE_TO_SPARE) return fail("There isn't enough space on this phone for the update.")
            try {
                download(context, offer, part, limit)
                if (ApkCheck.problem(part, offer.size, expected) != null) {
                    return fail("The download didn't match the release, so it wasn't installed.")
                }
                if (!part.renameTo(apk)) return fail("Couldn't keep the download.")
            } finally {
                // Whatever happened, no half download stays.
                part.delete()
            }
        }
        val problem = apkProblem(context, apk, offer) ?: commit(context, apk, expected)
        if (problem != null) {
            apk.delete()
            return fail(problem)
        }
        return mapOf("ok" to true)
    }

    /**
     * What's wrong with a checked download as an update of this app, in the
     * person's words, or null: it's PassBundle at the release's version,
     * newer than this build, and signed with exactly the certificates this
     * app is signed with. Android checks the key again when it installs; this
     * says so before it asks.
     */
    private fun apkProblem(context: Context, apk: File, offer: Offer): String? {
        val pm = context.packageManager
        val archive = archiveInfo(pm, apk) ?: return "The download isn't an app, so it wasn't installed."
        if (archive.packageName != context.packageName) return "The download isn't PassBundle, so it wasn't installed."
        if (!Releases.isSameVersion(archive.versionName, offer.version)) {
            return "The download isn't the version its release says, so it wasn't installed."
        }
        val installed = pm.getPackageInfo(context.packageName, PackageManager.GET_SIGNING_CERTIFICATES)
        if (archive.longVersionCode <= installed.longVersionCode) return "The download isn't newer than this PassBundle."
        val theirs = signers(archive)
        if (theirs.isEmpty() || theirs != signers(installed)) {
            return "The update is signed with a different key than this copy of PassBundle, so it can't replace it. " +
                "To move to it, uninstall this copy and install the update from GitHub."
        }
        return null
    }

    private fun archiveInfo(pm: PackageManager, apk: File): PackageInfo? {
        val info = pm.getPackageArchiveInfo(apk.path, PackageManager.GET_SIGNING_CERTIFICATES)
        if (info?.signingInfo != null) return info
        // Some Android versions read an archive's signers only the old way.
        @Suppress("DEPRECATION")
        return pm.getPackageArchiveInfo(apk.path, PackageManager.GET_SIGNATURES)
    }

    /** The SHA-256 of each certificate an app or an APK is signed with now. */
    private fun signers(info: PackageInfo): Set<String> {
        @Suppress("DEPRECATION")
        val signatures = (info.signingInfo?.apkContentsSigners ?: info.signatures)?.toList() ?: emptyList()
        return signatures.map { s -> MessageDigest.getInstance("SHA-256").digest(s.toByteArray()).joinToString("") { "%02x".format(it) } }.toSet()
    }

    /**
     * Hands the APK to Android's installer, which asks the person every time
     * (InstallStatus). The bytes are hashed as they go in, so what's
     * installed is what was checked: if they differ from [expected], the
     * install is called off. Null once handed over, else what went wrong.
     */
    private fun commit(context: Context, apk: File, expected: String): String? {
        val installer = context.packageManager.packageInstaller
        val params =
            PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
                setAppPackageName(context.packageName)
                setSize(apk.length())
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                    setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_REQUIRED)
                }
            }
        val id = installer.createSession(params)
        try {
            installer.openSession(id).use { session ->
                val md = MessageDigest.getInstance("SHA-256")
                session.openWrite("passbundle.apk", 0, apk.length()).use { out ->
                    apk.inputStream().use { input ->
                        val buf = ByteArray(256 * 1024)
                        while (true) {
                            val n = input.read(buf)
                            if (n < 0) break
                            md.update(buf, 0, n)
                            out.write(buf, 0, n)
                        }
                    }
                    session.fsync(out)
                }
                if (md.digest().joinToString("") { "%02x".format(it) } != expected) {
                    session.abandon()
                    return "The update changed on this phone before it was installed, so it wasn't."
                }
                val flags = PendingIntent.FLAG_UPDATE_CURRENT or (if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0)
                val status = PendingIntent.getBroadcast(context, id, Intent(context, InstallStatus::class.java), flags)
                session.commit(status.intentSender)
            }
        } catch (e: Exception) {
            runCatching { installer.abandonSession(id) }
            throw e
        }
        return null
    }

    /** Downloads the update's file into [to], at most [limit] bytes and [DOWNLOAD_MINUTES], telling Dart how far it is. */
    private fun download(context: Context, offer: Offer, to: File, limit: Long) {
        val deadline = System.nanoTime() + DOWNLOAD_MINUTES * 60_000_000_000L
        val connection = open(context, offer.apkUrl, Releases::isDownloadHost, offer.test)
        try {
            val total = connection.contentLengthLong.takeIf { it > 0 } ?: offer.size
            if (total > limit) error("longer than the release says")
            connection.getInputStream().use { input ->
                to.outputStream().use { out ->
                    val buf = ByteArray(64 * 1024)
                    var read = 0L
                    var shown = -1
                    while (true) {
                        if (System.nanoTime() > deadline) error("too slow")
                        val n = input.read(buf)
                        if (n < 0) break
                        read += n
                        if (read > limit) error("longer than the release says")
                        out.write(buf, 0, n)
                        if (total > 0) {
                            val percent = (100 * read / total).toInt()
                            if (percent != shown) {
                                shown = percent
                                tell("progress", mapOf("percent" to percent))
                            }
                        }
                    }
                }
            }
        } finally {
            (connection as? HttpURLConnection)?.disconnect()
        }
    }

    private fun getJson(context: Context, url: String, test: Boolean): JSONObject? = getText(context, url, 1 shl 20, Releases::isApiHost, test)?.let(::JSONObject)

    /** A small text file, or null when it isn't there (404, as releases/latest answers with no release yet). */
    private fun getText(context: Context, url: String, max: Int, hostAllowed: (String?) -> Boolean, test: Boolean): String? {
        val connection = open(context, url, hostAllowed, test)
        try {
            if (connection is HttpURLConnection && connection.responseCode == 404) return null
            return connection.getInputStream().use { readAtMost(it, max) }.toString(Charsets.UTF_8)
        } finally {
            (connection as? HttpURLConnection)?.disconnect()
        }
    }

    /**
     * A connection over HTTPS that ends at a host [hostAllowed] allows, after any
     * redirects (Java never follows one from HTTPS to HTTP). A debug build's
     * test source (testSource) may also read files, from anywhere.
     */
    private fun open(context: Context, url: String, hostAllowed: (String?) -> Boolean, test: Boolean): URLConnection {
        val parsed = URL(url)
        val allowed = parsed.protocol == "https" || (test && parsed.protocol == "file" && debuggable(context))
        require(allowed) { "Only HTTPS" }
        val connection = parsed.openConnection()
        connection.connectTimeout = 15_000
        connection.readTimeout = 30_000
        if (connection is HttpURLConnection) {
            connection.instanceFollowRedirects = true
            connection.setRequestProperty("User-Agent", "PassBundle")
            if (parsed.host == "api.github.com") {
                connection.setRequestProperty("Accept", "application/vnd.github+json")
                connection.setRequestProperty("X-GitHub-Api-Version", "2022-11-28")
            }
            val code = connection.responseCode
            val landed = connection.url
            check(landed.protocol == "https") { "Redirected off HTTPS" }
            check(test || hostAllowed(landed.host)) { "Redirected to another host" }
            check(code in 200..299 || code == 404) { "HTTP $code" }
        }
        return connection
    }

    private fun readAtMost(input: InputStream, max: Int): ByteArray {
        val out = java.io.ByteArrayOutputStream()
        val buf = ByteArray(16 * 1024)
        while (true) {
            val n = input.read(buf)
            if (n < 0) break
            out.write(buf, 0, n)
            check(out.size() <= max) { "Too long" }
        }
        return out.toByteArray()
    }

    /**
     * Debug builds only: where a test's release answer is
     * (`update-source` in the app's folder, holding a URL, which may be a
     * file:// one), instead of GitHub. Release builds always ask GitHub.
     */
    private fun testSource(context: Context): String? {
        if (!debuggable(context)) return null
        val file = File(context.filesDir, "update-source")
        return runCatching { file.readText().trim() }.getOrNull()?.takeIf { it.isNotEmpty() }
    }

    /** What Android's installer said about the update, for Dart (InstallStatus). */
    internal fun installed(answer: Map<String, Any?>) = tell("installed", answer)

    private fun tell(what: String, details: Map<String, Any?>) {
        main.post { listener?.invoke(what, details) }
    }

    private fun versionName(context: Context): String? = runCatching { context.packageManager.getPackageInfo(context.packageName, 0).versionName }.getOrNull()

    private fun prefs(context: Context) = context.getSharedPreferences("updates", Context.MODE_PRIVATE)

    private fun debuggable(context: Context) = (context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0

    private fun fail(error: String): Map<String, Any?> = mapOf("ok" to false, "error" to error)
}

/**
 * What Android's installer says about an update (Updater.commit): it wants
 * the person to confirm, which opens its prompt, or it's done, or it failed.
 */
class InstallStatus : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                val confirm =
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                        intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java)
                    } else {
                        @Suppress("DEPRECATION")
                        intent.getParcelableExtra(Intent.EXTRA_INTENT)
                    }
                val shown = confirm != null && runCatching { context.startActivity(confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }.isSuccess
                if (!shown) Updater.installed(mapOf("ok" to false, "error" to "Android didn't ask to install the update. Tap Update to try again."))
            }
            // Android closes the app as it updates it, so this is seldom heard.
            PackageInstaller.STATUS_SUCCESS -> Updater.installed(mapOf("ok" to true))
            PackageInstaller.STATUS_FAILURE_ABORTED -> Updater.installed(mapOf("ok" to false, "error" to "The update wasn't installed."))
            PackageInstaller.STATUS_FAILURE_STORAGE -> Updater.installed(mapOf("ok" to false, "error" to "There isn't enough space on this phone for the update."))
            else -> Updater.installed(mapOf("ok" to false, "error" to "Android didn't install the update (status $status)."))
        }
    }
}
