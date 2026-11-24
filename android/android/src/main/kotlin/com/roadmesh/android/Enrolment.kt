package com.roadmesh.android

import android.content.Context
import com.roadmesh.core.certificateDigests
import com.roadmesh.core.certificatePublicKeys
import com.roadmesh.core.encodeIssueRequest
import com.roadmesh.core.issueRequestToSign
import java.io.File

/**
 * Pseudonym certificates for this phone (SPEC §10): 20 hardware keys a week, one per
 * certificate, requested with the device key. Until the server exists (Step 9), debug
 * builds exchange the request and the certificates as files with the developer CA
 * (`scripts/devca`), via `adb pull` / `adb push`.
 */
class Enrolment(private val context: Context, private val keys: KeystoreKeys) {
    private val dir: File get() = context.getExternalFilesDir(null) ?: context.filesDir

    /** Request file the developer pulls with adb. */
    val requestFile: File get() = File(dir, "roadmesh-request.bin")

    /** Certificate file the developer pushes with adb. */
    val certificateFile: File get() = File(dir, "roadmesh-certificates.bin")

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

    /**
     * Read the certificates the CA returned, bind each to its hardware key (by public key),
     * and return the batch for [com.roadmesh.core.SafetyEngine.addCertificates].
     */
    fun importCertificates(): ByteArray {
        val bytes = certificateFile.readBytes()
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
        return bytes
    }

    /** Previously imported certificates (kept across restarts), or null. */
    fun storedCertificates(): ByteArray? = certificateFile.takeIf { it.exists() }?.readBytes()

    private val pending get() = context.getSharedPreferences("roadmesh_enrolment", Context.MODE_PRIVATE)
}
