package com.passbundle.app

import android.app.Activity
import android.app.PendingIntent
import android.content.Intent
import android.content.pm.PackageManager
import android.hardware.usb.UsbConstants
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbDeviceConnection
import android.hardware.usb.UsbEndpoint
import android.hardware.usb.UsbInterface
import android.hardware.usb.UsbManager
import android.nfc.NfcAdapter
import android.nfc.Tag
import android.nfc.tech.IsoDep
import android.nfc.tech.Ndef
import java.io.IOException
import java.util.concurrent.TimeUnit
import java.util.concurrent.locks.ReentrantLock
import kotlin.concurrent.withLock

/**
 * Security keys for two-step login (core/src/android.rs, `ask_security_key`):
 * a key held to the back of the phone (NFC) or plugged in (USB). The core does
 * the talking, in CTAP, as it does on computers; this finds the key, asks
 * Android's permission for one on USB, and passes the core's APDUs and HID
 * reports to it, on the core's own thread. Also a YubiKey's one-time code held
 * to the phone (`readCodes`), from the address the key gives over NFC.
 */
object SecurityKeys {
    /** The app's screen while it's shown, for NFC's reader mode (MainActivity). */
    @Volatile var activity: Activity? = null

    private val lock = ReentrantLock()
    private val changed = lock.newCondition()
    private var looking = false
    private var tag: IsoDep? = null
    private var usb: Usb? = null
    private val asked = mutableSetOf<String>()

    private class Usb(
        val device: UsbDevice,
        val connection: UsbDeviceConnection,
        val iface: UsbInterface,
        val input: UsbEndpoint,
        val output: UsbEndpoint,
    )

    private val readerFlags =
        NfcAdapter.FLAG_READER_NFC_A or NfcAdapter.FLAG_READER_NFC_B or NfcAdapter.FLAG_READER_NO_PLATFORM_SOUNDS

    /** Starts looking for a key; false if the phone has neither NFC nor USB. */
    @JvmStatic fun start(): Boolean {
        val activity = activity ?: return false
        val nfc = NfcAdapter.getDefaultAdapter(activity)
        val hasUsb = activity.packageManager.hasSystemFeature(PackageManager.FEATURE_USB_HOST)
        if (nfc == null && !hasUsb) return false
        lock.withLock {
            looking = true
            asked.clear()
        }
        if (nfc != null) {
            activity.runOnUiThread {
                try {
                    nfc.enableReaderMode(activity, { found(it) }, readerFlags or NfcAdapter.FLAG_READER_SKIP_NDEF_CHECK, null)
                } catch (e: IllegalStateException) {
                    // The screen went away meanwhile.
                }
            }
        }
        return true
    }

    private fun found(found: Tag) {
        val iso = IsoDep.get(found) ?: return
        try {
            iso.connect()
            iso.timeout = 5000
        } catch (e: IOException) {
            return
        }
        lock.withLock {
            if (!looking) {
                closeQuietly(iso)
                return
            }
            tag?.let { closeQuietly(it) }
            tag = iso
            changed.signalAll()
        }
    }

    /** Waits up to [timeoutMs] for a key: 1 held to the phone, 2 plugged in, 0 none yet. */
    @JvmStatic fun waitForKey(timeoutMs: Int): Int {
        if (lock.withLock { usb == null }) openUsb()
        lock.withLock {
            if (tag == null && usb == null) changed.await(timeoutMs.toLong().coerceAtLeast(1), TimeUnit.MILLISECONDS)
            return when {
                tag?.isConnected == true -> 1
                usb != null -> 2
                else -> 0
            }
        }
    }

    /** A plugged-in key that looks like a security key, with Android's permission (asked once per wait). */
    private fun openUsb() {
        val activity = activity ?: return
        val manager = activity.getSystemService(UsbManager::class.java) ?: return
        for (device in manager.deviceList.values) {
            val iface = fidoInterface(device) ?: continue
            if (!manager.hasPermission(device)) {
                if (lock.withLock { asked.add(device.deviceName) }) {
                    val intent = Intent("com.passbundle.app.USB_PERMISSION").setPackage(activity.packageName)
                    manager.requestPermission(device, PendingIntent.getBroadcast(activity, 0, intent, PendingIntent.FLAG_IMMUTABLE))
                }
                continue
            }
            val connection = manager.openDevice(device) ?: continue
            if (!connection.claimInterface(iface, true)) {
                connection.close()
                continue
            }
            // Its report descriptor names the FIDO Alliance's usage page, as
            // the core checks on Linux (core/src/security_key/mod.rs, is_fido).
            val descriptor = ByteArray(1024)
            val length = connection.controlTransfer(0x81, 0x06, 0x2200, iface.id, descriptor, descriptor.size, 1000)
            val (input, output) = endpoints(iface) ?: (null to null)
            if (length <= 0 || !isFido(descriptor.copyOf(length)) || input == null || output == null) {
                connection.releaseInterface(iface)
                connection.close()
                continue
            }
            lock.withLock {
                usb = Usb(device, connection, iface, input, output)
                changed.signalAll()
            }
            return
        }
    }

    /** An interface that looks like a security key's: HID without a boot protocol (not a keyboard or mouse), with a 64-byte interrupt endpoint each way. */
    private fun fidoInterface(device: UsbDevice): UsbInterface? =
        (0 until device.interfaceCount)
            .map { device.getInterface(it) }
            .firstOrNull {
                it.interfaceClass == UsbConstants.USB_CLASS_HID && it.interfaceSubclass == 0 &&
                    it.interfaceProtocol == 0 && endpoints(it) != null
            }

    private fun endpoints(iface: UsbInterface): Pair<UsbEndpoint, UsbEndpoint>? {
        val all = (0 until iface.endpointCount).map { iface.getEndpoint(it) }
            .filter { it.type == UsbConstants.USB_ENDPOINT_XFER_INT && it.maxPacketSize == 64 }
        val input = all.firstOrNull { it.direction == UsbConstants.USB_DIR_IN } ?: return null
        val output = all.firstOrNull { it.direction == UsbConstants.USB_DIR_OUT } ?: return null
        return input to output
    }

    /** An APDU to the key held to the phone, and its answer; null once it's gone. */
    @JvmStatic fun nfcTransceive(apdu: ByteArray): ByteArray? {
        val iso = lock.withLock { tag } ?: return null
        return try {
            iso.transceive(apdu)
        } catch (e: IOException) {
            null
        } catch (e: SecurityException) {
            null
        } catch (e: IllegalStateException) {
            null
        }
    }

    /** A 64-byte report to the key plugged in: the report back once sent, null once it's gone. */
    @JvmStatic fun usbWrite(report: ByteArray): ByteArray? {
        val usb = lock.withLock { usb } ?: return null
        return if (usb.connection.bulkTransfer(usb.output, report, report.size, 1000) == report.size) report else null
    }

    /** The key's next 64-byte report: empty if none came in [timeoutMs], null once it's unplugged. */
    @JvmStatic fun usbRead(timeoutMs: Int): ByteArray? {
        val usb = lock.withLock { usb } ?: return null
        val report = ByteArray(64)
        val read = usb.connection.bulkTransfer(usb.input, report, report.size, timeoutMs.coerceAtLeast(1))
        if (read == 64) return report
        val manager = activity?.getSystemService(UsbManager::class.java)
        val attached = manager?.deviceList?.values?.any { it.deviceName == usb.device.deviceName } ?: false
        return if (attached) ByteArray(0) else null
    }

    /** Done with the key found: the next wait looks again. */
    @JvmStatic fun release() {
        lock.withLock {
            tag?.let { closeQuietly(it) }
            tag = null
            usb?.let {
                it.connection.releaseInterface(it.iface)
                it.connection.close()
            }
            usb = null
        }
    }

    /** Stops looking. */
    @JvmStatic fun stop() {
        release()
        lock.withLock { looking = false }
        val activity = activity ?: return
        activity.runOnUiThread {
            try {
                NfcAdapter.getDefaultAdapter(activity)?.disableReaderMode(activity)
            } catch (e: IllegalStateException) {
                // The screen went away meanwhile.
            }
        }
    }

    private fun closeQuietly(iso: IsoDep) {
        try {
            iso.close()
        } catch (e: IOException) {
            // Gone already.
        }
    }

    /**
     * A YubiKey's one-time codes held to the phone, while [onCode] is set
     * (two_step.dart, YubiKey): the key gives an address over NFC that ends in
     * its code (`https://my.yubico.com/yk/#<code>`).
     */
    fun readCodes(activity: Activity, onCode: ((String) -> Unit)?) {
        val nfc = NfcAdapter.getDefaultAdapter(activity) ?: return
        activity.runOnUiThread {
            try {
                if (onCode == null) {
                    nfc.disableReaderMode(activity)
                } else {
                    nfc.enableReaderMode(activity, { found ->
                        val message = Ndef.get(found)?.cachedNdefMessage ?: return@enableReaderMode
                        message.records.firstNotNullOfOrNull { record -> record.toUri()?.toString()?.let(::codeFrom) }
                            ?.let { code -> activity.runOnUiThread { onCode(code) } }
                    }, readerFlags, null)
                }
            } catch (e: IllegalStateException) {
                // The screen went away meanwhile.
            }
        }
    }

    /** The one-time code at the end of a YubiKey's address: 44 modhex characters after its last '#'. */
    fun codeFrom(address: String): String? {
        val code = address.substringAfterLast('#', "").trim().lowercase()
        return code.takeIf { it.length == 44 && it.all { c -> c in "cbdefghijklnrtuv" } }
    }

    /** Whether a HID report descriptor names the FIDO Alliance's usage page (0xF1D0), as core/src/security_key/mod.rs reads one. */
    fun isFido(descriptor: ByteArray): Boolean {
        var at = 0
        while (at < descriptor.size) {
            val prefix = descriptor[at].toInt() and 0xff
            if (prefix == 0xfe) {
                if (at + 1 >= descriptor.size) return false
                at += 3 + (descriptor[at + 1].toInt() and 0xff)
                continue
            }
            val size = if (prefix and 0x03 == 3) 4 else prefix and 0x03
            if (at + 1 + size > descriptor.size) return false
            if (prefix and 0xfc == 0x04) {
                var page = 0
                for (i in size - 1 downTo 0) page = (page shl 8) or (descriptor[at + 1 + i].toInt() and 0xff)
                if (page == 0xf1d0) return true
            }
            at += 1 + size
        }
        return false
    }
}
