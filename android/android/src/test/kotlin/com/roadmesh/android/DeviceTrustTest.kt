package com.roadmesh.android

import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class DeviceTrustTest {
    @Test
    fun rootSignalsLowerTrust() {
        assertFalse(DeviceTrust.signals("release-keys", suFound = false))
        assertFalse(DeviceTrust.signals(null, suFound = false))
        assertTrue(DeviceTrust.signals("test-keys", suFound = false))
        assertTrue(DeviceTrust.signals("release-keys", suFound = true))
    }
}
