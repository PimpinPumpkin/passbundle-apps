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

    /** Where checking asks: GitHub's API, and nothing else. */
    const val API_HOST = "api.github.com"

    /** Where a release's files are: PassBundle's own releases, and nowhere else. */
    const val DOWNLOADS = "https://github.com/PimpinPumpkin/passbundle-apps/releases/download/"

    /** The hosts a download may end at: github.com, and where it hands the file. */
    val DOWNLOAD_HOSTS = setOf("github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com")

    /**
     * Whether [url] is a file on PassBundle's own releases:
     * [DOWNLOADS]`<tag>/<file name>`, the tag and the name plain (letters,
     * digits, dots, hyphens, underscores, and plus signs, never "." or "..").
     */
    fun isReleaseDownload(url: String): Boolean {
        if (!url.startsWith(DOWNLOADS)) return false
        val parts = url.removePrefix(DOWNLOADS).split('/')
        return parts.size == 2 &&
            parts.all { part ->
                part.isNotEmpty() && part != "." && part != ".." && part.all { it in 'a'..'z' || it in 'A'..'Z' || it in '0'..'9' || it in "._-+" }
            }
    }

    /** Whether a check ended at GitHub's API. */
    fun isApiHost(host: String?): Boolean = host?.lowercase() == API_HOST

    /** Whether a download (after GitHub's redirects) ended at one of [DOWNLOAD_HOSTS]. */
    fun isDownloadHost(host: String?): Boolean = host?.lowercase() in DOWNLOAD_HOSTS

    /** Whether a downloaded APK's versionName is the version its release's tag names. */
    fun isSameVersion(apkVersionName: String?, version: String): Boolean = apkVersionName != null && apkVersionName == version

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
     * The Mac's releases (macos-v, which count their own builds) are never
     * marked latest, but are in the list, and never taken for Android's.
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
