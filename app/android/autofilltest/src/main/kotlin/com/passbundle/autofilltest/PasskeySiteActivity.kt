package com.passbundle.autofilltest

import android.app.Activity
import android.os.Bundle
import android.os.CancellationSignal
import android.util.Base64
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import androidx.credentials.CreateCredentialResponse
import androidx.credentials.CreatePublicKeyCredentialRequest
import androidx.credentials.CreatePublicKeyCredentialResponse
import androidx.credentials.CredentialManager
import androidx.credentials.CredentialManagerCallback
import androidx.credentials.GetCredentialRequest
import androidx.credentials.GetCredentialResponse
import androidx.credentials.GetPublicKeyCredentialOption
import androidx.credentials.PublicKeyCredential
import androidx.credentials.exceptions.CreateCredentialException
import androidx.credentials.exceptions.GetCredentialException
import org.json.JSONArray
import org.json.JSONObject
import java.security.KeyFactory
import java.security.MessageDigest
import java.security.PublicKey
import java.security.SecureRandom
import java.security.Signature
import java.security.spec.X509EncodedKeySpec

/**
 * A site's sign-in, as an app with passkeys has it, for the passkey test
 * (tools/android-autofill-test.sh): it makes a passkey through Android's
 * Credential Manager, then signs in with it, and checks what came back as a
 * site would: the challenge, its own origin, the site's ID, and the signature
 * with the passkey's public key. It says on screen how each went.
 */
class PasskeySiteActivity : Activity() {
    private companion object {
        const val SITE = "passkeys.example.test"
    }

    private val manager by lazy { CredentialManager.create(this) }
    private lateinit var result: TextView
    private var publicKey: PublicKey? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        result = TextView(this).apply { id = R.id.result }
        val make = Button(this).apply {
            id = R.id.make_passkey
            text = "Make a passkey"
            setOnClickListener { make() }
        }
        val signIn = Button(this).apply {
            id = R.id.sign_in_passkey
            text = "Sign in with a passkey"
            setOnClickListener { signIn() }
        }
        setContentView(
            LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                setPadding(48, 160, 48, 48)
                addView(make)
                addView(signIn)
                addView(result)
            },
        )
    }

    private fun b64(bytes: ByteArray) = Base64.encodeToString(bytes, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)

    private fun unb64(text: String) = Base64.decode(text, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)

    private fun challenge() = ByteArray(32).also { SecureRandom().nextBytes(it) }

    private fun say(words: String) = runOnUiThread { result.text = words }

    private fun make() {
        val challenge = challenge()
        val request =
            JSONObject()
                .put("rp", JSONObject().put("id", SITE).put("name", "Passkeys Example"))
                .put(
                    "user",
                    JSONObject().put("id", b64("user-1".toByteArray())).put("name", "ana@$SITE").put("displayName", "Ana"),
                ).put("challenge", b64(challenge))
                .put("pubKeyCredParams", JSONArray().put(JSONObject().put("type", "public-key").put("alg", -7)))
                .put("authenticatorSelection", JSONObject().put("residentKey", "required").put("userVerification", "preferred"))
                .put("attestation", "none")
                .put("timeout", 60000)
        manager.createCredentialAsync(
            this,
            CreatePublicKeyCredentialRequest(request.toString()),
            CancellationSignal(),
            mainExecutor,
            object : CredentialManagerCallback<CreateCredentialResponse, CreateCredentialException> {
                override fun onResult(response: CreateCredentialResponse) {
                    val json = JSONObject((response as CreatePublicKeyCredentialResponse).registrationResponseJson)
                    val answer = json.getJSONObject("response")
                    val problem = check(answer, "webauthn.create", challenge)
                    if (problem != null) return say("Making a passkey went wrong: $problem")
                    val authData = unb64(answer.getString("authenticatorData"))
                    // The attested credential data names the credential it made.
                    val idLength = ((authData[53].toInt() and 0xff) shl 8) or (authData[54].toInt() and 0xff)
                    if (!authData.copyOfRange(55, 55 + idLength).contentEquals(unb64(json.getString("rawId")))) {
                        return say("Making a passkey went wrong: the credential isn't the one attested")
                    }
                    publicKey = KeyFactory.getInstance("EC").generatePublic(X509EncodedKeySpec(unb64(answer.getString("publicKey"))))
                    say("Made a passkey for $SITE")
                }

                override fun onError(e: CreateCredentialException) = say("Making a passkey failed: ${e.type} ${e.message}")
            },
        )
    }

    private fun signIn() {
        val key = publicKey ?: return say("Make a passkey first")
        val challenge = challenge()
        val request =
            JSONObject()
                .put("challenge", b64(challenge))
                .put("rpId", SITE)
                .put("allowCredentials", JSONArray())
                .put("userVerification", "preferred")
                .put("timeout", 60000)
        manager.getCredentialAsync(
            this,
            GetCredentialRequest(listOf(GetPublicKeyCredentialOption(request.toString()))),
            CancellationSignal(),
            mainExecutor,
            object : CredentialManagerCallback<GetCredentialResponse, GetCredentialException> {
                override fun onResult(response: GetCredentialResponse) {
                    val credential = response.credential as? PublicKeyCredential ?: return say("Signing in went wrong: not a passkey")
                    val answer = JSONObject(credential.authenticationResponseJson).getJSONObject("response")
                    val problem = check(answer, "webauthn.get", challenge)
                    if (problem != null) return say("Signing in went wrong: $problem")
                    val clientData = unb64(answer.getString("clientDataJSON"))
                    val signed = unb64(answer.getString("authenticatorData")) + MessageDigest.getInstance("SHA-256").digest(clientData)
                    val verifier = Signature.getInstance("SHA256withECDSA").apply { initVerify(key) }
                    verifier.update(signed)
                    if (!verifier.verify(unb64(answer.getString("signature")))) return say("Signing in went wrong: the signature doesn't verify")
                    if (String(unb64(answer.getString("userHandle"))) != "user-1") return say("Signing in went wrong: another account")
                    say("Signed in with a passkey, signature checked")
                }

                override fun onError(e: GetCredentialException) = say("Signing in failed: ${e.type} ${e.message}")
            },
        )
    }

    /**
     * What a site checks of the client data and the authenticator data: the
     * type, its challenge, this app's own origin, and the site's ID. Null if
     * all's well, else what's wrong.
     */
    private fun check(answer: JSONObject, type: String, challenge: ByteArray): String? {
        val client = JSONObject(String(unb64(answer.getString("clientDataJSON"))))
        if (client.optString("type") != type) return "client data type ${client.optString("type")}"
        if (!unb64(client.getString("challenge")).contentEquals(challenge)) return "another challenge"
        val signer = packageManager.getPackageInfo(packageName, android.content.pm.PackageManager.GET_SIGNING_CERTIFICATES).signingInfo!!.apkContentsSigners[0]
        val origin = "android:apk-key-hash:" + b64(MessageDigest.getInstance("SHA-256").digest(signer.toByteArray()))
        if (client.optString("origin") != origin) return "origin ${client.optString("origin")}"
        val authData = unb64(answer.getString("authenticatorData"))
        if (!authData.copyOfRange(0, 32).contentEquals(MessageDigest.getInstance("SHA-256").digest(SITE.toByteArray()))) return "another site's ID"
        if ((authData[32].toInt() and 0x01) == 0) return "no user presence"
        return null
    }
}
