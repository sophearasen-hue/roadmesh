package com.roadmesh.android

import java.security.KeyPairGenerator
import java.security.spec.ECGenParameterSpec
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class PinnedTlsTest {
    private fun spki(): ByteArray = KeyPairGenerator.getInstance("EC").run {
        initialize(ECGenParameterSpec("secp256r1"))
        generateKeyPair().public.encoded
    }

    @Test
    fun pinMatchesOnlyThePinnedKeys() {
        val server = spki()
        val backup = spki()
        val attacker = spki()
        val pins = listOf(PinnedTls.spkiSha256(server), PinnedTls.spkiSha256(backup))
        assertTrue(PinnedTls.pinMatches(server, pins))
        assertTrue(PinnedTls.pinMatches(backup, pins))
        assertFalse(PinnedTls.pinMatches(attacker, pins))
        assertFalse(PinnedTls.pinMatches(server, emptyList()))
    }
}
