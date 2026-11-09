package com.passbundle.app

import com.passbundle.app.AutofillSetup.Way
import org.junit.Assert.assertEquals
import org.junit.Test

// Where "Turn on" for autofill goes (AutofillSetup.kt): Android's own prompt
// first, on every version the app runs on.
class AutofillSetupTest {
    @Test fun `Android's own prompt comes first, then Settings`() {
        for (sdk in 29..33) {
            assertEquals("API $sdk", listOf(Way.PROMPT, Way.SETTINGS), AutofillSetup.ways(sdk))
        }
    }

    @Test fun `Android 14 and later try the page of password and passkey services before Settings`() {
        for (sdk in 34..37) {
            assertEquals("API $sdk", listOf(Way.PROMPT, Way.PROVIDERS, Way.SETTINGS), AutofillSetup.ways(sdk))
        }
    }

    @Test fun `each way has the name Dart hears`() {
        assertEquals(listOf("prompt", "providers", "settings"), Way.entries.map { it.id })
    }
}
