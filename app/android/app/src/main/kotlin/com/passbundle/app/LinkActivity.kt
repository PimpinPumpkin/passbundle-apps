package com.passbundle.app

import android.app.Activity
import android.app.ActivityManager
import android.content.Intent
import android.os.Bundle

/**
 * Duo's way back to the app (two_step.dart): the server's Duo page opens
 * `passbundle://duo-callback?code=...&state=...`, which lands here. So does
 * single sign-on's (sso.dart): the server sends the browser to
 * `com.passbundle.app:/sso?code=...&state=...`. The link goes to the app's
 * screens, which hand it to the core, which finishes only the sign-in it
 * belongs to (core/src/two_step.rs, core/src/sso.rs); then the app comes
 * back to the front. No screen of its own.
 */
class LinkActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        intent?.dataString?.let { Links.arrived(it) }
        val tasks = getSystemService(ActivityManager::class.java)?.appTasks.orEmpty()
        val app = tasks.firstOrNull { it.taskInfo.baseActivity?.className == MainActivity::class.java.name }
        if (app != null) {
            app.moveToFront()
        } else {
            startActivity(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
        finish()
    }
}

/** Links back from the browser, for the app's screens (MainActivity's passbundle/links channel). */
object Links {
    /** Duo's way back. */
    const val DUO = "passbundle://duo-callback"

    /** Single sign-on's way back, exactly the redirect the server takes for the phones (core/src/sso.rs). */
    const val SSO = "com.passbundle.app:/sso?"

    @Volatile var listener: ((String) -> Unit)? = null

    /** The two links the app takes; anything else that opens it is dropped. */
    fun takes(link: String): Boolean = (link.startsWith(DUO) || link.startsWith(SSO)) && link.length <= 4096

    fun arrived(link: String) {
        if (takes(link)) listener?.invoke(link)
    }
}
