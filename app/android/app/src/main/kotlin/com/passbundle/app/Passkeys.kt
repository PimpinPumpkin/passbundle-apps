package com.passbundle.app

import android.content.Context
import android.content.pm.ApplicationInfo
import android.util.Base64
import androidx.credentials.provider.CallingAppInfo
import org.json.JSONObject
import java.io.File
import java.security.MessageDigest

/**
 * What PassBundle's passkey provider (PasskeyProvider.kt, PasskeyActivity.kt)
 * works out about who's asking (docs/phones.md, "Passkeys"):
 *
 * - **A browser** on Google's privileged list, checked by signing certificate
 *   (Browsers.kt), gives the page's origin and the client data hash itself,
 *   having checked the site's ID against the page.
 * - **Any other app** may use a site's passkeys only if the site's Digital
 *   Asset Links name its package and signing certificate (the core checks,
 *   asset_links.rs); then PassBundle builds the client data, with the app's
 *   origin (`android:apk-key-hash:`).
 */
object Passkeys {
    /** Who's asking, once they may: their origin, and whether they give the client data hash. */
    class Caller(val origin: String, val browser: Boolean, val packageName: String)

    fun base64url(bytes: ByteArray): String = Base64.encodeToString(bytes, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)

    fun fromBase64url(text: String): ByteArray? = runCatching { Base64.decode(text, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP) }.getOrNull()

    /**
     * Whether the caller may use the site's passkeys, and its origin. A
     * browser on the list is let through; another app only with the site's
     * asset links, which the core may fetch: call it off the main thread.
     */
    fun caller(context: Context, info: CallingAppInfo, rpId: String): Caller? {
        val origin = runCatching { info.getOrigin(Browsers.privilegedJson(context)) }.getOrNull()
        if (origin != null) return Caller(origin, true, info.packageName)
        val signers = info.signingInfo.apkContentsSigners
        // An app signed by several keys has no one origin.
        if (signers.size != 1) return null
        val hash = MessageDigest.getInstance("SHA-256").digest(signers[0].toByteArray())
        // Anything going wrong (a site's file is anyone's text) means no.
        val allowed = runCatching { Core.appMayUsePasskeys(rpId, info.packageName, hash, testStatements(context, rpId)) }.getOrDefault(false)
        return if (allowed) Caller("android:apk-key-hash:${base64url(hash)}", false, info.packageName) else null
    }

    /**
     * The client data for an app (WebAuthn's CollectedClientData), which
     * PassBundle builds since no browser did: its type, the site's challenge,
     * and the app's origin and package.
     */
    fun clientData(type: String, challenge: String, caller: Caller): ByteArray =
        JSONObject()
            .put("type", type)
            .put("challenge", challenge)
            .put("origin", caller.origin)
            .put("crossOrigin", false)
            .put("androidPackageName", caller.packageName)
            .toString()
            .toByteArray(Charsets.UTF_8)

    fun sha256(bytes: ByteArray): ByteArray = MessageDigest.getInstance("SHA-256").digest(bytes)

    /**
     * Debug builds only: the asset links a test names for its test site
     * (`passkey-test-asset-links.json` in the app's folder, `{site: statements}`),
     * standing in for fetching them. Release builds always fetch.
     */
    private fun testStatements(context: Context, rpId: String): String? {
        if ((context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) == 0) return null
        val file = File(context.filesDir, "passkey-test-asset-links.json")
        if (!file.exists()) return null
        return runCatching { JSONObject(file.readText()).optJSONArray(rpId)?.toString() }.getOrNull()
    }
}
