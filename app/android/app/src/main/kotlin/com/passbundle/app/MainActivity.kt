package com.passbundle.app

import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

/**
 * The app's one screen, which Flutter draws. What's here is what only Android
 * does (docs/phones.md, "Talking to the OS"): the app's own folder, and
 * copying straight from the core to the clipboard.
 */
class MainActivity : FlutterActivity() {
    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        Core.init(applicationContext)
        Biometric.init(applicationContext)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "passbundle/platform")
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    // Where the core keeps the vault: this app's private files.
                    "filesDir" -> result.success(filesDir.absolutePath)
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
                    "biometricRemove" -> {
                        Biometric.remove()
                        result.success(mapOf("ok" to true))
                    }
                    else -> result.notImplemented()
                }
            }
    }
}
