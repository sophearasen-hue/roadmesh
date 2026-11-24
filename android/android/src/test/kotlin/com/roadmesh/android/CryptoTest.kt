package com.roadmesh.android

import java.security.KeyPairGenerator
import java.security.Signature
import java.security.interfaces.ECPublicKey
import java.security.spec.ECGenParameterSpec
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertNull

class CryptoTest {
    @Test
    fun derSignaturesBecome64RawBytesAndVerifyBack() {
        val kp = KeyPairGenerator.getInstance("EC").apply { initialize(ECGenParameterSpec("secp256r1")) }.genKeyPair()
        repeat(50) {
            val msg = "message $it".toByteArray()
            val der = Signature.getInstance("SHA256withECDSA").run { initSign(kp.private); update(msg); sign() }
            val raw = Crypto.derToRaw(der) ?: error("parse failed")
            assertEquals(64, raw.size)
            // rebuild DER from raw and verify with the JDK
            fun int(b: ByteArray): ByteArray {
                val t = b.dropWhile { x -> x == 0.toByte() }.toByteArray()
                val v = if (t.isNotEmpty() && t[0] < 0) byteArrayOf(0) + t else t
                return byteArrayOf(0x02, v.size.toByte()) + v
            }
            val body = int(raw.copyOfRange(0, 32)) + int(raw.copyOfRange(32, 64))
            val der2 = byteArrayOf(0x30, body.size.toByte()) + body
            val ok = Signature.getInstance("SHA256withECDSA").run { initVerify(kp.public); update(msg); verify(der2) }
            assert(ok)
        }
    }

    @Test
    fun malformedDerIsRejected() {
        assertNull(Crypto.derToRaw(byteArrayOf()))
        assertNull(Crypto.derToRaw(byteArrayOf(0x30, 0x02, 0x02, 0x00)))
        assertNull(Crypto.derToRaw(byteArrayOf(0x31, 0x00)))
    }

    @Test
    fun compressedKeysAre33BytesWithParityPrefix() {
        val kp = KeyPairGenerator.getInstance("EC").apply { initialize(ECGenParameterSpec("secp256r1")) }.genKeyPair()
        val c = Crypto.compress(kp.public as ECPublicKey)
        assertEquals(33, c.size)
        assert(c[0] == 0x02.toByte() || c[0] == 0x03.toByte())
    }

    @Test
    fun hexRoundTrips() {
        val b = byteArrayOf(0, 1, -1, 16)
        assertEquals("0001ff10", Crypto.hex(b))
        assertContentEquals(b, Crypto.unhex("0001ff10"))
        assertNull(Crypto.unhex("abc"))
    }
}
