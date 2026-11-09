package com.passbundle.app

import android.content.Context
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.net.URI
import java.security.MessageDigest
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import java.util.concurrent.Future
import java.util.concurrent.TimeUnit

/**
 * An app and its website, when both say so (docs/phones.md, "Autofill"): an
 * app that names a site in its manifest (`asset_statements`, as Android's own
 * sharing of sign-ins between apps and sites asks of apps) gets that site's
 * logins through autofill too, once the site's Digital Asset Links name the
 * app's package and signing certificate for sharing sign-ins. The core
 * checks, as for passkeys (asset_links.rs). Only sites the app itself names
 * are ever fetched, never one from the vault; an app signed by several keys
 * has none.
 */
object AppSites {
    /** At most this many of the sites an app names count. */
    private const val MOST = 5
    private const val RELATION = "delegate_permission/common.get_login_creds"

    /** Checks that may fetch: a few at a time, and one per site, app, and key, however many ask. */
    private val fetching = Executors.newFixedThreadPool(4)
    private val checking = ConcurrentHashMap<String, Future<Boolean>>()

    /**
     * The sites whose logins also fill in the app: the ones it names that name
     * it back. With `waitMs` above 0, a site not checked lately is fetched, for
     * that long at most (one that takes longer counts next time); slow, so off
     * the main thread. With 0, only what was checked lately counts.
     */
    fun linked(context: Context, packageName: String, waitMs: Long): List<String> {
        val sites = named(context, packageName).takeIf { it.isNotEmpty() } ?: return emptyList()
        val certificate = certificate(context, packageName) ?: return emptyList()
        if (waitMs <= 0) return sites.filter { check(context, it, packageName, certificate, false) }
        val checks = sites.map { site -> site to checkSoon(context, site, packageName, certificate) }
        val end = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(waitMs)
        return checks
            .filter { (_, check) -> runCatching { check.get(maxOf(0L, end - System.nanoTime()), TimeUnit.NANOSECONDS) }.getOrDefault(false) }
            .map { it.first }
    }

    /** Starts checking the app's sites, without waiting: for once the person unlocks. */
    fun warm(context: Context, packageName: String) {
        val sites = named(context, packageName).takeIf { it.isNotEmpty() } ?: return
        val certificate = certificate(context, packageName) ?: return
        for (site in sites) checkSoon(context, site, packageName, certificate)
    }

    /** The check of a site that may fetch, on the pool: the one already running, if there is one. */
    private fun checkSoon(context: Context, site: String, packageName: String, certificate: ByteArray): Future<Boolean> {
        val key = "$site ${packageName} ${certificate.joinToString("") { "%02x".format(it) }}"
        return checking.computeIfAbsent(key) {
            fetching.submit<Boolean> {
                try {
                    check(context, site, packageName, certificate, true)
                } finally {
                    checking.remove(key)
                }
            }
        }
    }

    private fun check(context: Context, site: String, packageName: String, certificate: ByteArray, mayFetch: Boolean): Boolean =
        runCatching { Core.appLinkedToSite(site, packageName, certificate, testStatements(context, site), mayFetch) }
            .getOrDefault(false)

    /**
     * The sites an app names for sharing sign-ins, from its manifest's
     * `asset_statements`: an include of a site's own asset links
     * (`https://<site>/.well-known/assetlinks.json`), or a web site named for
     * sharing sign-ins. Hostnames only, at most five.
     */
    fun named(context: Context, packageName: String): List<String> {
        val pm = context.packageManager
        val info = runCatching { pm.getApplicationInfo(packageName, PackageManager.GET_META_DATA) }.getOrNull() ?: return emptyList()
        val id = info.metaData?.getInt("asset_statements", 0) ?: 0
        if (id == 0) return emptyList()
        val text = runCatching { pm.getResourcesForApplication(info).getString(id) }.getOrNull() ?: return emptyList()
        val statements = runCatching { JSONArray(text) }.getOrNull() ?: return emptyList()
        val sites = LinkedHashSet<String>()
        for (i in 0 until statements.length()) {
            if (sites.size == MOST) break
            statements.optJSONObject(i)?.let(::site)?.let { sites += it }
        }
        return sites.toList()
    }

    /** The site one statement names, if it's one this reads: nothing else in the address. */
    private fun site(statement: JSONObject): String? {
        val include = statement.optString("include")
        val address =
            include.ifEmpty {
                val relations = statement.optJSONArray("relation") ?: return null
                if ((0 until relations.length()).none { relations.optString(it) == RELATION }) return null
                val target = statement.optJSONObject("target")?.takeIf { it.optString("namespace") == "web" } ?: return null
                target.optString("site")
            }
        val uri = runCatching { URI(address) }.getOrNull() ?: return null
        if (uri.scheme != "https" || uri.port != -1 || uri.rawUserInfo != null || uri.rawQuery != null || uri.rawFragment != null) return null
        val path = uri.rawPath.orEmpty()
        if (include.isNotEmpty() && path != "/.well-known/assetlinks.json") return null
        if (include.isEmpty() && path != "" && path != "/") return null
        return Browsers.domain(uri.host)
    }

    /** The app's one signing certificate's SHA-256; null when it has several, or can't be seen. */
    fun certificate(context: Context, packageName: String): ByteArray? {
        val info =
            runCatching { context.packageManager.getPackageInfo(packageName, PackageManager.GET_SIGNING_CERTIFICATES) }.getOrNull()
                ?: return null
        val signing = info.signingInfo ?: return null
        if (signing.hasMultipleSigners()) return null
        val signer = signing.apkContentsSigners.singleOrNull() ?: return null
        return MessageDigest.getInstance("SHA-256").digest(signer.toByteArray())
    }

    /**
     * Debug builds only: the asset links a test names for its test sites
     * (`test-asset-links.json` in the app's folder, `{site: statements}`),
     * standing in for fetching them, for passkeys too. Release builds always
     * fetch.
     */
    fun testStatements(context: Context, site: String): String? {
        if ((context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) == 0) return null
        val file = File(context.filesDir, "test-asset-links.json")
        if (!file.exists()) return null
        return runCatching { JSONObject(file.readText()).optJSONArray(site)?.toString() }.getOrNull()
    }
}
