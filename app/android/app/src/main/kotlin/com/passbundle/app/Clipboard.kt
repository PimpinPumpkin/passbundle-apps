package com.passbundle.app

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Handler
import android.os.Looper
import android.os.PersistableBundle
import java.util.UUID

/**
 * Copying an item's value from the core straight to the clipboard, never
 * through Dart (docs/phones.md, "Copying"). Secrets are marked sensitive, so
 * the keyboard's clipboard preview and Android's copy notice don't show them,
 * and cleared after 30 seconds unless something else was copied since.
 */
object Clipboard {
    private const val CLEAR_AFTER_MS = 30_000L
    // ClipDescription.EXTRA_IS_SENSITIVE, from Android 13; earlier versions ignore it.
    private const val IS_SENSITIVE = "android.content.extra.IS_SENSITIVE"
    // Our mark on each copy, to know it's still ours when clearing.
    private const val MARK = "com.passbundle.app.copy"

    private val main = Handler(Looper.getMainLooper())
    private var latest: String? = null

    /** False when there's nothing to copy, or the vault is locked or busy. */
    fun copy(context: Context, id: String, what: String): Boolean {
        val bytes = Core.value(id, what) ?: return false
        val text = try {
            String(bytes, Charsets.UTF_8)
        } finally {
            bytes.fill(0)
        }
        val secret = what != "username"
        val mark = UUID.randomUUID().toString()
        val clip = ClipData.newPlainText("PassBundle", text)
        clip.description.extras = PersistableBundle().apply {
            putString(MARK, mark)
            if (secret) putBoolean(IS_SENSITIVE, true)
        }
        val manager = context.getSystemService(ClipboardManager::class.java)
        manager.setPrimaryClip(clip)
        latest = mark
        if (secret) main.postDelayed({ clearIfOurs(manager, mark) }, CLEAR_AFTER_MS)
        return true
    }

    private fun clearIfOurs(manager: ClipboardManager, mark: String) {
        // A later copy of ours replaced it, and clears itself if it's a secret.
        if (latest != mark) return
        latest = null
        // Android hides the clipboard from apps in the background, so there
        // it can't tell what's on it: it clears anyway, as 30 seconds is the
        // promise. In front, it leaves something someone else copied since.
        val now = manager.primaryClipDescription
        if (now == null || now.extras?.getString(MARK) == mark) manager.clearPrimaryClip()
    }
}
