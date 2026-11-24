package com.roadmesh.android

import java.math.BigInteger
import java.security.interfaces.ECPublicKey

/** Encoding helpers between Android's crypto formats and the core's (SPEC §10). */
object Crypto {
    /** ECDSA signature: ASN.1 DER `SEQUENCE { r INTEGER, s INTEGER }` → raw r‖s (64 bytes). */
    fun derToRaw(der: ByteArray): ByteArray? {
        var i = 0
        fun byte(): Int? = if (i < der.size) der[i++].toInt() and 0xff else null
        fun length(): Int? {
            val first = byte() ?: return null
            if (first < 0x80) return first
            val n = first and 0x7f
            if (n == 0 || n > 2) return null
            var len = 0
            repeat(n) { len = (len shl 8) or (byte() ?: return null) }
            return len
        }
        fun integer(): ByteArray? {
            if (byte() != 0x02) return null
            val len = length() ?: return null
            if (len <= 0 || len > 33 || i + len > der.size) return null
            val v = der.copyOfRange(i, i + len)
            i += len
            val trimmed = v.dropWhile { it == 0.toByte() }.toByteArray()
            if (trimmed.size > 32) return null
            return ByteArray(32 - trimmed.size) + trimmed
        }
        if (byte() != 0x30) return null
        val total = length() ?: return null
        if (total != der.size - i) return null
        val r = integer() ?: return null
        val s = integer() ?: return null
        if (i != der.size) return null
        return r + s
    }

    /** P-256 public key → SEC1 compressed point (33 bytes). */
    fun compress(key: ECPublicKey): ByteArray = compress(key.w.affineX, key.w.affineY)

    /** Affine coordinates → SEC1 compressed point (33 bytes). */
    fun compress(x: BigInteger, y: BigInteger): ByteArray {
        val prefix: Byte = if (y.testBit(0)) 0x03 else 0x02
        return byteArrayOf(prefix) + fixed32(x)
    }

    private fun fixed32(v: BigInteger): ByteArray {
        val b = v.toByteArray().dropWhile { it == 0.toByte() }.toByteArray()
        require(b.size <= 32) { "coordinate too long" }
        return ByteArray(32 - b.size) + b
    }

    /** Lowercase hex, for storage keys (never log key material or pseudonyms). */
    fun hex(b: ByteArray): String = b.joinToString("") { "%02x".format(it) }

    /** Parse lowercase/uppercase hex; null if malformed. */
    fun unhex(s: String): ByteArray? {
        if (s.length % 2 != 0) return null
        return try {
            ByteArray(s.length / 2) { s.substring(2 * it, 2 * it + 2).toInt(16).toByte() }
        } catch (e: NumberFormatException) {
            null
        }
    }
}
