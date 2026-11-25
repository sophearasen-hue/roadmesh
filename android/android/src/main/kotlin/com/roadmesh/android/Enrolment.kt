package com.roadmesh.android

import android.content.Context
import android.util.Base64
import com.google.android.gms.tasks.Tasks
import com.google.android.play.core.integrity.IntegrityManagerFactory
import com.google.android.play.core.integrity.IntegrityTokenRequest
import com.roadmesh.core.CellularLink
import com.roadmesh.core.LinkEvent
import com.roadmesh.core.certificateDigests
import com.roadmesh.core.parseEnrolReply
import java.security.MessageDigest
import java.util.concurrent.TimeUnit
import com.roadmesh.core.certificatePublicKeys
import com.roadmesh.core.encodeIssueRequest
import com.roadmesh.core.issueRequestToSign
import java.io.File

/**
 * Pseudonym certificates for this phone (SPEC §10): 20 hardware keys a week, one per
 * certificate, requested with the device key. Production: [enrolOnline] sends the request
 * with Play Integrity evidence to the edge server's enrolment port and receives the
 * device certificate (for mutual TLS) and the pseudonym certificates. Debug builds can
 * also exchange files with the developer CA (`scripts/devca`) via adb.
 */
class Enrolment(private val context: Context, private val keys: KeystoreKeys) {
    private val dir: File get() = context.getExternalFilesDir(null) ?: context.filesDir

    /** Request file the developer pulls with adb. */
    val requestFile: File get() = File(dir, "roadmesh-request.bin")

    /** Certificate file the developer pushes with adb (debug builds). */
    val certificateFile: File get() = File(dir, "roadmesh-certificates.bin")

    /** Certificates in use, in app-private storage (AUDIT S-20); never on shared storage. */
    private val storedFile: File get() = File(context.filesDir, "roadmesh-certificates.bin")

    /** Generate 20 new pseudonym keys and write a signed certificate request. */
    fun createRequest(count: Int = 20, nowUnix: Long = System.currentTimeMillis() / 1000): File {
        val device = keys.generate(KeystoreKeys.DEVICE_ALIAS)
        val batch = nowUnix
        val pubs = (0 until count).map { keys.generate("${KeystoreKeys.PSEUDONYM_PREFIX}${batch}_$it") }
        val at = nowUnix.toUInt()
        val tbs = issueRequestToSign(device, pubs, at)
        val sig = keys.signWith(KeystoreKeys.DEVICE_ALIAS, tbs) ?: error("device key cannot sign")
        // App Attest / Play Integrity evidence arrives in Step 9; the developer CA accepts none
        val wire = encodeIssueRequest(device, pubs, at, ByteArray(0), sig)
        requestFile.writeBytes(wire)
        pending.edit().putLong("batch", batch).putInt("count", count).apply()
        return requestFile
    }

    /** The device certificate from the last online enrolment (DER), or null. */
    fun deviceCertificate(): ByteArray? = deviceCertFile.takeIf { it.exists() }?.readBytes()

    private val deviceCertFile: File get() = File(context.filesDir, "roadmesh-device.der")

    /**
     * Enrol with the edge server (blocking; call off the main thread): new pseudonym keys,
     * Play Integrity bound to the request, pinned TLS 1.3. Returns the certificate batch.
     */
    fun enrolOnline(config: ServerConfig, count: Int = 20, nowUnix: Long = System.currentTimeMillis() / 1000): ByteArray {
        val device = keys.generate(KeystoreKeys.DEVICE_ALIAS)
        val batch = nowUnix
        val pubs = (0 until count).map { keys.generate("${KeystoreKeys.PSEUDONYM_PREFIX}${batch}_$it") }
        val at = nowUnix.toUInt()
        val tbs = issueRequestToSign(device, pubs, at)
        val evidence = byteArrayOf(PLAY_INTEGRITY) + integrityToken(config, tbs).toByteArray(Charsets.US_ASCII)
        val sig = keys.signWith(KeystoreKeys.DEVICE_ALIAS, tbs) ?: error("device key cannot sign")
        val wire = encodeIssueRequest(device, pubs, at, evidence, sig)
        pending.edit().putLong("batch", batch).putInt("count", count).apply()
        val reply = exchange(config, wire)
        val enrolled = parseEnrolReply(reply)
        deviceCertFile.writeBytes(enrolled.deviceCertificate)
        return bindCertificates(enrolled.certificates)
    }

    /** Play Integrity (classic request) with nonce = SHA-256 of the request, as the server checks. */
    private fun integrityToken(config: ServerConfig, tbs: ByteArray): String {
        val nonce = Base64.encodeToString(
            MessageDigest.getInstance("SHA-256").digest(tbs),
            Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING,
        )
        val request = IntegrityTokenRequest.builder()
            .setNonce(nonce)
            .setCloudProjectNumber(config.cloudProject)
            .build()
        val task = IntegrityManagerFactory.create(context).requestIntegrityToken(request)
        return Tasks.await(task, 30, TimeUnit.SECONDS).token()
    }

    private fun exchange(config: ServerConfig, request: ByteArray): ByteArray {
        val link = CellularLink()
        PinnedTls.connect(config.host, config.enrolPort, config.pins, null, timeoutMs = 30_000).use { s ->
            val out = s.outputStream
            out.write(link.connect(30u))
            out.write(link.enrol(request))
            out.flush()
            val buf = ByteArray(4096)
            while (true) {
                val n = s.inputStream.read(buf)
                if (n < 0) error("server closed")
                for (ev in link.onBytes(buf.copyOf(n))) {
                    when (ev) {
                        is LinkEvent.EnrolReply -> return ev.reply
                        is LinkEvent.Close, is LinkEvent.Refused -> error("server refused")
                        else -> {}
                    }
                }
            }
        }
    }

    /**
     * Read the certificates the CA returned, bind each to its hardware key (by public key),
     * and return the batch for [com.roadmesh.core.SafetyEngine.addCertificates].
     */
    fun importCertificates(): ByteArray {
        val bytes = certificateFile.readBytes()
        certificateFile.delete() // the adb copy on external storage is not kept
        return bindCertificates(bytes)
    }

    private fun bindCertificates(bytes: ByteArray): ByteArray {
        val batch = pending.getLong("batch", -1)
        val count = pending.getInt("count", 0)
        val byPublicKey = (0 until count).associate { i ->
            val alias = "${KeystoreKeys.PSEUDONYM_PREFIX}${batch}_$i"
            Crypto.hex(keys.generate(alias)) to alias
        }
        val digests = certificateDigests(bytes)
        val pubs = certificatePublicKeys(bytes)
        for ((d, p) in digests.zip(pubs)) {
            val alias = byPublicKey[Crypto.hex(p)] ?: error("certificate for a key this phone does not have")
            keys.bind(d, alias)
        }
        storedFile.writeBytes(bytes)
        return bytes
    }

    /** Previously imported certificates (kept across restarts), or null. */
    fun storedCertificates(): ByteArray? = storedFile.takeIf { it.exists() }?.readBytes()

    /** Forget the device certificate (the server forgets the device too; see docs/SERVER.md). */
    fun forgetDevice() {
        deviceCertFile.delete()
    }

    private val pending get() = context.getSharedPreferences("roadmesh_enrolment", Context.MODE_PRIVATE)

    companion object {
        /** Evidence kind byte for Play Integrity (server `attest::kind`). */
        const val PLAY_INTEGRITY: Byte = 3
    }
}
