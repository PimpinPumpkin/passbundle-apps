package com.passbundle.app

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import io.flutter.plugin.common.MethodChannel
import java.io.File
import java.util.UUID

/**
 * Picking a file to attach to an item (docs/phones.md, "Attachments"):
 * Android's own picker, then a copy into the app's cache (cacheDir/picked),
 * which only this app can read, for the core to read, encrypt, and send. A
 * file larger than the app attaches on a phone is never copied whole. The
 * copies are deleted once the upload ends, however it ends (`forget`, which
 * attaching.dart calls), and whenever the app starts.
 */
object Picker {
    const val REQUEST = 0x7a2
    private var waiting: MethodChannel.Result? = null
    private var limit = 0L

    /** Opens the picker: the answer comes with `picked`. */
    fun pick(activity: Activity, limit: Long, result: MethodChannel.Result) {
        // One at a time: an earlier one that never came back chose nothing.
        waiting?.success(null)
        waiting = result
        this.limit = limit
        val intent =
            Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                addCategory(Intent.CATEGORY_OPENABLE)
                type = "*/*"
            }
        try {
            activity.startActivityForResult(intent, REQUEST)
        } catch (_: Exception) {
            waiting = null
            result.success(mapOf("error" to "This phone has no app to choose a file with."))
        }
    }

    /**
     * What the person chose, from MainActivity's onActivityResult: `{path,
     * name}` of the copy, `{tooLarge: true}`, or null when they chose none.
     * The copying is off the main thread.
     */
    fun picked(activity: Activity, resultCode: Int, data: Intent?) {
        val result = waiting ?: return
        waiting = null
        val uri = data?.data
        if (resultCode != Activity.RESULT_OK || uri == null) {
            result.success(null)
            return
        }
        val limit = limit
        Thread {
            val answer =
                try {
                    copy(activity, uri, limit)
                } catch (_: Exception) {
                    forget(activity)
                    mapOf("error" to "PassBundle couldn't read that file. Check that it's still there.")
                }
            activity.runOnUiThread { result.success(answer) }
        }.start()
    }

    private fun copy(activity: Activity, uri: Uri, limit: Long): Map<String, Any?> {
        val resolver = activity.contentResolver
        var name = "file"
        var size = -1L
        resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use {
            if (it.moveToFirst()) {
                if (!it.isNull(0)) name = it.getString(0)
                if (!it.isNull(1)) size = it.getLong(1)
            }
        }
        // Said before any of it is copied, when the provider says its size.
        if (size > limit) return mapOf("tooLarge" to true)
        // A name of the app's own, so the file's name never makes the path.
        val dir = File(activity.cacheDir, "picked").apply { mkdirs() }
        val copy = File(dir, UUID.randomUUID().toString())
        var copied = 0L
        val input = resolver.openInputStream(uri) ?: throw java.io.IOException("no stream")
        val buffer = ByteArray(1 shl 16)
        try {
            input.use {
                copy.outputStream().use { output ->
                    while (true) {
                        val read = input.read(buffer)
                        if (read < 0) break
                        copied += read
                        // One the provider didn't say the size of stops here.
                        if (copied > limit) break
                        output.write(buffer, 0, read)
                    }
                }
            }
        } finally {
            buffer.fill(0)
        }
        if (copied > limit) {
            copy.delete()
            return mapOf("tooLarge" to true)
        }
        return mapOf("path" to copy.absolutePath, "name" to name)
    }

    /** Deletes every copy the picker made. */
    fun forget(context: Context) {
        File(context.cacheDir, "picked").deleteRecursively()
    }

    private var started = false

    /**
     * Deletes what a run that stopped partway left, once each time the app's
     * process starts: an upload this process runs outlives its screen.
     */
    fun forgetLeftOver(context: Context) {
        if (started) return
        started = true
        forget(context)
    }
}
