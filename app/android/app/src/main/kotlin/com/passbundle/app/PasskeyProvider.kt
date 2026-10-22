package com.passbundle.app

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.CancellationSignal
import android.os.OutcomeReceiver
import androidx.credentials.exceptions.ClearCredentialException
import androidx.credentials.exceptions.CreateCredentialException
import androidx.credentials.exceptions.CreateCredentialUnknownException
import androidx.credentials.exceptions.GetCredentialException
import androidx.credentials.provider.AuthenticationAction
import androidx.credentials.provider.BeginCreateCredentialRequest
import androidx.credentials.provider.BeginCreateCredentialResponse
import androidx.credentials.provider.BeginCreatePublicKeyCredentialRequest
import androidx.credentials.provider.BeginGetCredentialRequest
import androidx.credentials.provider.BeginGetCredentialResponse
import androidx.credentials.provider.BeginGetPublicKeyCredentialOption
import androidx.credentials.provider.CreateEntry
import androidx.credentials.provider.CredentialEntry
import androidx.credentials.provider.CredentialProviderService
import androidx.credentials.provider.ProviderClearCredentialStateRequest
import androidx.credentials.provider.PublicKeyCredentialEntry
import org.json.JSONObject

/**
 * PassBundle as Android's passkey provider (docs/phones.md, "Passkeys"),
 * Android 14 and later, in the app's process like autofill. Android asks
 * first what PassBundle has for a site; it answers with the site's passkeys
 * by account name (only to a caller that may use them: Passkeys.kt), or
 * "Unlock PassBundle" while it's locked. Picking one, or saving a new one,
 * opens PasskeyActivity, which asks the person and has the core sign.
 */
class PasskeyProvider : CredentialProviderService() {
    override fun onBeginGetCredentialRequest(
        request: BeginGetCredentialRequest,
        cancellationSignal: CancellationSignal,
        callback: OutcomeReceiver<BeginGetCredentialResponse, GetCredentialException>,
    ) {
        // The asset links check can reach the network: off the main thread.
        Thread {
            Core.init(applicationContext)
            Core.openIfClosed(filesDir.absolutePath)
            callback.onResult(entries(this, request) ?: BeginGetCredentialResponse())
        }.start()
    }

    override fun onBeginCreateCredentialRequest(
        request: BeginCreateCredentialRequest,
        cancellationSignal: CancellationSignal,
        callback: OutcomeReceiver<BeginCreateCredentialResponse, CreateCredentialException>,
    ) {
        if (request !is BeginCreatePublicKeyCredentialRequest) {
            return callback.onError(CreateCredentialUnknownException("PassBundle saves passkeys only."))
        }
        val save =
            CreateEntry.Builder("PassBundle", pending(this, PasskeyActivity.CREATE))
                .setDescription("Save the passkey in PassBundle")
                .build()
        callback.onResult(BeginCreateCredentialResponse.Builder().addCreateEntry(save).build())
    }

    override fun onClearCredentialStateRequest(
        request: ProviderClearCredentialStateRequest,
        cancellationSignal: CancellationSignal,
        callback: OutcomeReceiver<Void?, ClearCredentialException>,
    ) {
        // Nothing kept about a session to clear.
        callback.onResult(null)
    }

    companion object {
        private var requests = 0

        fun pending(context: Context, mode: String, item: String? = null): PendingIntent =
            PendingIntent.getActivity(
                context,
                ++requests,
                Intent(context, PasskeyActivity::class.java)
                    .putExtra(PasskeyActivity.MODE, mode)
                    .putExtra(PasskeyActivity.ITEM, item),
                // Mutable: Android adds the request when it starts it.
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE,
            )

        /**
         * The site's passkeys for each of Android's options, to a caller that
         * may use them; "Unlock PassBundle" while it's locked. Null when
         * there's nothing to offer. Call it off the main thread.
         */
        fun entries(context: Context, request: BeginGetCredentialRequest): BeginGetCredentialResponse? {
            val info = request.callingAppInfo ?: return null
            val response = BeginGetCredentialResponse.Builder()
            val found = mutableListOf<CredentialEntry>()
            var locked = false
            for (option in request.beginGetCredentialOptions.filterIsInstance<BeginGetPublicKeyCredentialOption>()) {
                val rpId = runCatching { JSONObject(option.requestJson).getString("rpId") }.getOrNull() ?: continue
                Passkeys.caller(context, info, rpId) ?: continue
                val answer = JSONObject(Core.passkeys(rpId) ?: continue)
                if (answer.optBoolean("locked")) {
                    locked = true
                    continue
                }
                val allowed = allowedIds(option.requestJson)
                val items = answer.optJSONArray("items") ?: continue
                for (i in 0 until items.length()) {
                    val item = items.getJSONObject(i)
                    if (allowed != null && item.getString("credentialId") !in allowed) continue
                    val name = item.optString("userName").ifEmpty { item.optString("name") }
                    found +=
                        PublicKeyCredentialEntry.Builder(context, name, pending(context, PasskeyActivity.GET, item.getString("id")), option)
                            .setDisplayName(item.optString("userDisplayName").ifEmpty { item.optString("name") })
                            .build()
                }
            }
            if (locked) {
                response.addAuthenticationAction(
                    AuthenticationAction("Unlock PassBundle", pending(context, PasskeyActivity.UNLOCK)),
                )
            }
            if (found.isEmpty() && !locked) return null
            return response.setCredentialEntries(found).build()
        }

        /** The credential IDs a site allows (base64url), or null for any. */
        private fun allowedIds(requestJson: String): Set<String>? {
            val list = runCatching { JSONObject(requestJson).optJSONArray("allowCredentials") }.getOrNull() ?: return null
            if (list.length() == 0) return null
            return (0 until list.length()).mapNotNull { list.optJSONObject(it)?.optString("id")?.trimEnd('=') }.toSet()
        }
    }
}
