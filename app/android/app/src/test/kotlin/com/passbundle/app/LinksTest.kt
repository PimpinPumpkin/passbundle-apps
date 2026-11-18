package com.passbundle.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

// The links that open the app from the browser (LinkActivity.kt): Duo's way
// back, and single sign-on's redirect, exactly as the server sends them, and
// nothing else (core/src/sso.rs checks the state).
class LinksTest {
    @Test fun takesDuosAndSingleSignOnsWayBack() {
        assertTrue(Links.takes("passbundle://duo-callback?code=c&state=s"))
        assertTrue(Links.takes("com.passbundle.app:/sso?code=c&state=s"))
    }

    @Test fun dropsAnythingElse() {
        listOf(
            "com.passbundle.app:/sso",
            "com.passbundle.app:/ssox?code=c",
            "com.passbundle.app://sso?code=c",
            "com.passbundle.app:/callback?code=c",
            "com.passbundle.app.evil:/sso?code=c",
            "https://evil.example/com.passbundle.app:/sso?code=c",
            "passbundle://other?code=c",
            "com.passbundle.app:/sso?" + "c".repeat(4096),
            "",
        ).forEach { link -> assertFalse(link, Links.takes(link)) }
    }
}
