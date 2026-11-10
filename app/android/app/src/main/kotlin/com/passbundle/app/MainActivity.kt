package com.passbundle.app

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Build
import android.os.Bundle
import android.view.WindowManager
import com.passbundle.app.update.Updater
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

/**
 * The app's one screen, which Flutter draws. What's here is what only Android
 * does (docs/phones.md, "Talking to the OS"): the app's own folder,
 * copying straight from the core to the clipboard, turning on autofill and
 * passkeys, security keys and Duo for two-step login, and updates from GitHub.
 */
class MainActivity : FlutterActivity() {
    private var screenOff: BroadcastReceiver? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // The vault stays out of screenshots, screen recordings, and the
        // app switcher's preview (docs/phones.md, "Locking").
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        // A copy of a file to attach left by a run that stopped partway.
        Picker.forgetLeftOver(this)
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == Picker.REQUEST) Picker.picked(this, resultCode, data)
    }

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        // The phone locking (its screen going off) is the computer sleeping:
        // the app locks if the person chose that (main.dart).
        val computer = MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "passbundle/computer")
        screenOff =
            object : BroadcastReceiver() {
                override fun onReceive(context: Context, intent: Intent) {
                    computer.invokeMethod("sleeping", null)
                }
            }.also {
                val filter = IntentFilter(Intent.ACTION_SCREEN_OFF)
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    registerReceiver(it, filter, Context.RECEIVER_NOT_EXPORTED)
                } else {
                    registerReceiver(it, filter)
                }
            }
        Core.start(applicationContext)
        Biometric.init(applicationContext)
        // Two-step login (two_step.dart): security keys and YubiKey codes
        // held to the phone or plugged in (SecurityKeys.kt), and Duo's way
        // back from the browser (LinkActivity.kt).
        SecurityKeys.activity = this
        val links = MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "passbundle/links")
        Links.listener = { link -> runOnUiThread { links.invokeMethod("link", link) } }
        // How an update's download goes, and what Android's installer said (update/Updater.kt).
        val updates = MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "passbundle/updates")
        Updater.listener = { what, details -> updates.invokeMethod(what, details) }
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "passbundle/platform")
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    // Where the core keeps the vault: this app's private files.
                    "filesDir" -> result.success(filesDir.absolutePath)
                    // A sign-in to save, once: the editor opens with it (vault.dart).
                    "pendingSave" -> result.success(PassBundleAutofill.takeSave())
                    // The server an organization's MDM set (app_restrictions.xml), if any.
                    "managedServer" ->
                        result.success(
                            getSystemService(android.content.RestrictionsManager::class.java)
                                ?.applicationRestrictions
                                ?.getString("server_url")
                                ?.takeIf { it.isNotBlank() },
                        )
                    // Each call about an account names it (core.dart, Core.account).
                    "copy" -> {
                        val account = call.argument<String>("account") ?: ""
                        val id = call.argument<String>("id")
                        val what = call.argument<String>("what")
                        result.success(id != null && what != null && Clipboard.copy(this, account, id, what))
                    }
                    // Unlocking with the phone's biometrics (Biometric.kt), each account's its own.
                    "biometricStatus" ->
                        result.success(
                            mapOf(
                                "status" to Biometric.status(this, call.argument<String>("account") ?: ""),
                                "method" to Biometric.method(this),
                            ),
                        )
                    "biometricEnroll" ->
                        Biometric.enroll(this, call.argument<String>("account") ?: "") { on, error ->
                            result.success(mapOf("ok" to (error == null), "on" to on, "error" to error))
                        }
                    "biometricUnlock" ->
                        Biometric.unlock(this, call.argument<String>("account") ?: "") { unlocked, error ->
                            result.success(mapOf("ok" to (error == null), "unlocked" to unlocked, "error" to error))
                        }
                    // Confirming it's the person, to approve a sign-in from another device.
                    "biometricConfirmApproval" ->
                        Biometric.unlock(this, call.argument<String>("account") ?: "", "Approve sign-in", forApproval = true) { confirmed, error ->
                            result.success(mapOf("ok" to (error == null), "confirmed" to confirmed, "error" to error))
                        }
                    // Tests only: debug builds' stand-in for Android's prompt (false in a release build).
                    "biometricStandIn" -> result.success(Biometric.useStandIn(this, call.argument<Boolean>("on") == true))
                    "biometricRemove" -> {
                        Biometric.remove(call.argument<String>("account") ?: "")
                        result.success(mapOf("ok" to true))
                    }
                    // An attachment, into Downloads (Attachments.kt).
                    "saveAttachment" -> {
                        val account = call.argument<String>("account") ?: ""
                        val id = call.argument<String>("id")
                        val attachment = call.argument<String>("attachmentId")
                        val name = call.argument<String>("name")
                        if (id == null || attachment == null || name == null) {
                            result.success(mapOf("ok" to false, "error" to "Couldn't save that."))
                        } else {
                            Attachments.saveToDownloads(this, account, id, attachment, name) { answer -> result.success(answer) }
                        }
                    }
                    // A file to attach (Picker.kt): copied into the app's cache,
                    // up to the size the app attaches, and deleted after.
                    "pickFile" -> Picker.pick(this, call.argument<Number>("limit")?.toLong() ?: 0L, result)
                    "forgetPicked" -> {
                        Picker.forget(this)
                        result.success(null)
                    }
                    // A login's website, in the browser: only http and https.
                    "openUrl" -> {
                        val uri = call.argument<String>("url")?.let { android.net.Uri.parse(it) }
                        val web = uri != null && (uri.scheme == "https" || uri.scheme == "http")
                        result.success(
                            web &&
                                try {
                                    startActivity(
                                        android.content.Intent(android.content.Intent.ACTION_VIEW, uri)
                                            .addCategory(android.content.Intent.CATEGORY_BROWSABLE),
                                    )
                                    true
                                } catch (e: android.content.ActivityNotFoundException) {
                                    false
                                },
                        )
                    }
                    // A YubiKey's code held to the phone, while two-step login shows YubiKey.
                    "yubikeyNfc" -> {
                        val onCode: ((String) -> Unit)? =
                            if (call.argument<Boolean>("on") == true) ({ code: String -> links.invokeMethod("yubikeyOtp", code) }) else null
                        SecurityKeys.readCodes(this, onCode)
                        result.success(true)
                    }
                    // Browsers the person trusted for autofill (Browsers.kt), and stopping.
                    "trustedBrowsers" -> result.success(Browsers.trusted(this))
                    "untrustBrowser" -> {
                        call.argument<String>("package")?.let { Browsers.untrust(this, it) }
                        result.success(true)
                    }
                    // Turning on autofill and passkeys: whether each is on, and
                    // Android's own prompt or settings for them (AutofillSetup.kt).
                    "autofillStatus" -> result.success(AutofillSetup.status(this))
                    "turnOnAutofill" -> result.success(AutofillSetup.turnOnAutofill(this))
                    "openPasskeySettings" -> result.success(AutofillSetup.openPasskeySettings(this))
                    // Tests only: debug builds record what would open instead (false in a release build).
                    "autofillSetupStandIn" ->
                        result.success(AutofillSetup.useStandIn(this, call.argument<Boolean>("on") == true))
                    "autofillSetupOpened" -> result.success(AutofillSetup.openedByStandIn())
                    // Updates from PassBundle's GitHub releases (update/Updater.kt).
                    "updateStatus" -> result.success(Updater.status(this))
                    "updateCheck" -> {
                        val quiet = call.argument<Boolean>("quiet") == true
                        Thread {
                            val answer = Updater.check(applicationContext, quiet)
                            runOnUiThread { result.success(answer) }
                        }.start()
                    }
                    "updateInstall" -> Updater.install(this) { answer -> result.success(answer) }
                    "updateAuto" -> {
                        Updater.setAuto(this, call.argument<Boolean>("on") == true)
                        result.success(true)
                    }
                    else -> result.notImplemented()
                }
            }
    }

    override fun cleanUpFlutterEngine(flutterEngine: FlutterEngine) {
        screenOff?.let { unregisterReceiver(it) }
        screenOff = null
        Updater.listener = null
        Links.listener = null
        if (SecurityKeys.activity === this) SecurityKeys.activity = null
        super.cleanUpFlutterEngine(flutterEngine)
    }
}
