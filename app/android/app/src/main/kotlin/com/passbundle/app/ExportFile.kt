package com.passbundle.app

import android.app.Activity
import android.content.Context
import android.content.Intent
import io.flutter.plugin.common.MethodChannel
import java.io.File

/**
 * Exporting a file (app/lib/transfer.dart; DESIGN.md in the meta repo,
 * "Archive, export, and import in the apps"): the core writes the export in
 * the app's cache (cacheDir/export), which only this app can read; Android's
 * own "create document" picker asks where the person wants it, the file is
 * copied there, and it's deleted at once, whatever they chose, and whenever
 * the app starts if it was left.
 */
object ExportFile {
    const val REQUEST = 0x7a3
    private var waiting: MethodChannel.Result? = null
    private var file: File? = null

    private fun folder(context: Context) = File(context.cacheDir, "export")

    /** The folder the core writes the export in, new and empty: its path. */
    fun prepare(context: Context): String? {
        forget(context)
        val dir = folder(context)
        return if (dir.mkdirs() || dir.isDirectory) dir.absolutePath else null
    }

    /** Deletes whatever is in the folder. */
    fun forget(context: Context) {
        folder(context).deleteRecursively()
    }

    /**
     * Asks where to keep the file at [path], one the core wrote in the
     * folder, as [name]: `{ok, saved}` comes once the person has chosen.
     */
    fun export(activity: Activity, path: String, name: String, result: MethodChannel.Result) {
        // One at a time: an earlier one that never came back saved nothing.
        finish(false)
        val chosen = File(path)
        if (chosen.parentFile?.canonicalPath != folder(activity).canonicalPath || !chosen.isFile) {
            result.success(mapOf("ok" to false, "saved" to false))
            return
        }
        waiting = result
        file = chosen
        val intent =
            Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
                addCategory(Intent.CATEGORY_OPENABLE)
                type = if (name.endsWith(".csv")) "text/csv" else "application/json"
                putExtra(Intent.EXTRA_TITLE, name)
            }
        try {
            activity.startActivityForResult(intent, REQUEST)
        } catch (_: Exception) {
            finish(false)
        }
    }

    /**
     * Where the person chose, from MainActivity's onActivityResult: the file
     * copied there off the main thread, then deleted either way.
     */
    fun chosen(activity: Activity, resultCode: Int, data: Intent?) {
        val uri = data?.data
        val from = file
        if (resultCode != Activity.RESULT_OK || uri == null || from == null) {
            finish(false)
            return
        }
        Thread {
            val saved =
                try {
                    activity.contentResolver.openOutputStream(uri, "wt")?.use { out ->
                        from.inputStream().use { it.copyTo(out) }
                        true
                    } ?: false
                } catch (_: Exception) {
                    false
                }
            activity.runOnUiThread { finish(saved) }
        }.start()
    }

    private fun finish(saved: Boolean) {
        file?.delete()
        file = null
        val result = waiting
        waiting = null
        result?.success(mapOf("ok" to true, "saved" to saved))
    }
}
