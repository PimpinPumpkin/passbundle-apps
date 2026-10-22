package com.passbundle.app

import android.app.Activity
import android.content.ContentValues
import android.provider.MediaStore
import org.json.JSONObject

/**
 * Saving an item's attachment to Downloads (docs/core.md, "Attachments"):
 * MediaStore makes the file (and picks a free name), and the core downloads,
 * opens, and writes the attachment straight into it, so it never passes
 * through Java or Dart. No permission is needed on Android 10 and later.
 */
object Attachments {
    fun saveToDownloads(
        activity: Activity,
        id: String,
        attachmentId: String,
        name: String,
        done: (Map<String, Any?>) -> Unit,
    ) {
        Thread {
            val answer = try {
                write(activity, id, attachmentId, name)
            } catch (_: Exception) {
                mapOf("ok" to false, "error" to "Couldn't save that.")
            }
            activity.runOnUiThread { done(answer) }
        }.start()
    }

    private fun write(activity: Activity, id: String, attachmentId: String, name: String): Map<String, Any?> {
        val resolver = activity.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, name)
            put(MediaStore.Downloads.IS_PENDING, 1)
        }
        val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
            ?: return mapOf("ok" to false, "error" to "Couldn't save that.")
        // Anything short of a whole file, however it fails, leaves nothing in Downloads.
        val answer = try {
            resolver.openFileDescriptor(uri, "w")?.use { file ->
                JSONObject(Core.writeAttachmentTo(id, attachmentId, file.fd) ?: "{}")
            } ?: JSONObject()
        } catch (e: Exception) {
            resolver.delete(uri, null, null)
            throw e
        }
        if (!answer.optBoolean("ok")) {
            resolver.delete(uri, null, null)
            return mapOf("ok" to false, "error" to answer.optString("error", "Couldn't save that."))
        }
        resolver.update(uri, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
        val saved = resolver.query(uri, arrayOf(MediaStore.Downloads.DISPLAY_NAME), null, null, null)?.use {
            if (it.moveToFirst()) it.getString(0) else null
        } ?: name
        return mapOf("ok" to true, "path" to "Downloads/$saved")
    }
}
