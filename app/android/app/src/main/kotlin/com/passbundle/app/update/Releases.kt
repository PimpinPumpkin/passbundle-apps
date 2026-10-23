package com.passbundle.app.update

/**
 * The Android releases' names (.github/workflows/release.yml): tagged
 * `android-v<major>.<minor>.<build>`, where the build number counts every
 * release and only goes up, so it alone says which release is newer, as in
 * Vela (app/vela/update/ApkChoice.kt, appReleaseTags). The app's own
 * versionName is `<major>.<minor>.<build>`.
 */
object Releases {
    const val TAG_PREFIX = "android-v"

    private val TAG = Regex("""^android-v(\d+\.\d+\.(\d+))$""")
    private val NAME = Regex("""^\d+\.\d+\.(\d+)$""")

    /** The version a release tag names ("0.1.12" from android-v0.1.12), or null for another tag. */
    fun versionOf(tag: String): String? = TAG.find(tag)?.groupValues?.get(1)

    /** The build number of a release tag, or null for another tag. */
    fun buildOfTag(tag: String): Int? = TAG.find(tag)?.groupValues?.get(2)?.toIntOrNull()

    /**
     * The build number of the running app's versionName; 0 for a build that
     * isn't a release's (a computer's own, say "0.1.0"), so any release is newer.
     */
    fun buildOfName(name: String?): Int = name?.let { NAME.find(it.trim())?.groupValues?.get(1)?.toIntOrNull() } ?: 0

    /** Whether the release tagged [tag] is newer than the app running as [runningName]. */
    fun isNewer(tag: String, runningName: String?): Boolean {
        val build = buildOfTag(tag) ?: return false
        return build > buildOfName(runningName)
    }

    /**
     * The newest Android release among [tags] (from the list of releases,
     * when releases/latest is another platform's), or null when there's none.
     */
    fun newest(tags: List<String>): String? = tags.filter { buildOfTag(it) != null }.maxByOrNull { buildOfTag(it)!! }

    /**
     * What's new, from a release's notes: its "- " lines (the commit
     * subjects the release workflow lists), or the notes as they are if it
     * has none. At most [max] characters.
     */
    fun changes(body: String, max: Int = 4000): String {
        val lines = body.lines().map { it.trimEnd('\r') }.filter { it.startsWith("- ") }
        val text = if (lines.isEmpty()) body.trim() else lines.joinToString("\n")
        return if (text.length <= max) text else text.take(max).substringBeforeLast('\n') + "\n…"
    }
}
