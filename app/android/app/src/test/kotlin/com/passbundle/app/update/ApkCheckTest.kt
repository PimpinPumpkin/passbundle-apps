package com.passbundle.app.update

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test
import java.io.File

// Ported from Vela's updater tests (app/vela/update/ApkCheckTest.kt).
class ApkCheckTest {
    private fun file(bytes: ByteArray) = File.createTempFile("passbundle", ".apk").apply { deleteOnExit(); writeBytes(bytes) }

    private val apk = "PK".toByteArray() + ByteArray(1000) { (it % 7).toByte() }

    @Test fun theRightFilePasses() {
        val f = file(apk)
        assertNull(ApkCheck.problem(f, apk.size.toLong(), ApkCheck.sha256(f)))
        assertNull("no size published: the checksum alone", ApkCheck.problem(f, 0, ApkCheck.sha256(f)))
    }

    @Test fun aShortDownloadIsCaughtBySize() {
        val good = ApkCheck.sha256(file(apk))
        assertNotNull(ApkCheck.problem(file(apk.copyOf(600)), apk.size.toLong(), good))
    }

    @Test fun aChangedByteIsCaughtByTheChecksum() {
        val good = ApkCheck.sha256(file(apk))
        val bad = apk.copyOf().also { it[500] = 99 }
        assertNotNull(ApkCheck.problem(file(bad), apk.size.toLong(), good))
    }

    @Test fun anErrorPageIsNotAnApk() {
        val page = "<html>rate limited</html>".toByteArray()
        assertNotNull(ApkCheck.problem(file(page), 0, ApkCheck.sha256(file(page))))
    }

    @Test fun theHashIsTheOneSha256sumWrites() {
        // printf abc | sha256sum
        assertEquals("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad", ApkCheck.sha256(file("abc".toByteArray())))
    }

    @Test fun githubsDigestFieldIsRead() {
        val hex = "774e7cb3a4b7f8c899e862e70d8c899b11e1e84d1fb74d90554817dcc06dc00c"
        assertEquals(hex, ApkCheck.digestOf("sha256:$hex"))
        assertEquals(hex, ApkCheck.digestOf("SHA256:${hex.uppercase()}"))
        assertNull(ApkCheck.digestOf(""))
        assertNull(ApkCheck.digestOf(null))
        assertNull(ApkCheck.digestOf("md5:abc"))
    }

    @Test fun sha256sumsIsRead() {
        val a = "a".repeat(64)
        val b = "B".repeat(64)
        val text = "$a  passbundle-0.1.12-arm64-v8a.apk\n$b *passbundle-0.1.12-x86_64.apk\r\nnot a line\n\n"
        assertEquals(
            mapOf("passbundle-0.1.12-arm64-v8a.apk" to a, "passbundle-0.1.12-x86_64.apk" to b.lowercase()),
            ApkCheck.sums(text),
        )
        assertEquals(emptyMap<String, String>(), ApkCheck.sums("<html>Not Found</html>"))
        assertNull("a short hash isn't one", ApkCheck.sums("${"a".repeat(63)}  x.apk")["x.apk"])
    }
}
