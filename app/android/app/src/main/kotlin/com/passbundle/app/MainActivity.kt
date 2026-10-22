package com.passbundle.app

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Build
import android.os.Bundle
import android.view.WindowManager
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

/**
 * The app's one screen, which Flutter draws. What's here is what only Android
 * does (docs/phones.md, "Talking to the OS"): the app's own folder, and
 * copying straight from the core to the clipboard.
 */
class MainActivity : FlutterActivity() {
    private var screenOff: BroadcastReceiver? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // The vault stays out of screenshots, screen recordings, and the
        // app switcher's preview (docs/phones.md, "Locking").
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
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
        Core.init(applicationContext)
        Biometric.init(applicationContext)
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
                    "copy" -> {
                        val id = call.argument<String>("id")
                        val what = call.argument<String>("what")
                        result.success(id != null && what != null && Clipboard.copy(this, id, what))
                    }
                    // Unlocking with the phone's biometrics (Biometric.kt).
                    "biometricStatus" ->
                        result.success(mapOf("status" to Biometric.status(this), "method" to Biometric.method(this)))
                    "biometricEnroll" ->
                        Biometric.enroll(this) { on, error ->
                            result.success(mapOf("ok" to (error == null), "on" to on, "error" to error))
                        }
                    "biometricUnlock" ->
                        Biometric.unlock(this) { unlocked, error ->
                            result.success(mapOf("ok" to (error == null), "unlocked" to unlocked, "error" to error))
                        }
                    // Tests only: debug builds' stand-in for Android's prompt (false in a release build).
                    "biometricStandIn" -> result.success(Biometric.useStandIn(this, call.argument<Boolean>("on") == true))
                    "biometricRemove" -> {
                        Biometric.remove()
                        result.success(mapOf("ok" to true))
                    }
                    else -> result.notImplemented()
                }
            }
    }

    override fun cleanUpFlutterEngine(flutterEngine: FlutterEngine) {
        screenOff?.let { unregisterReceiver(it) }
        screenOff = null
        super.cleanUpFlutterEngine(flutterEngine)
    }
}
