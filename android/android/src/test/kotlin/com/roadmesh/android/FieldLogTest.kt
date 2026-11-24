package com.roadmesh.android

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse

class FieldLogTest {
    @Test
    fun deliveryPerBandAndNoIdentifiersInTheCsv() {
        val log = FieldLog()
        repeat(60) {
            repeat(9) { _ -> log.onAccepted("deadbeef", -70) } // 9 of 10 arrive
            log.everySecond(mapOf("deadbeef" to 120.0))
        }
        log.everyMinute(87)
        val lines = log.csv().trim().lines()
        assertEquals(2, lines.size)
        assertEquals("0,87,100-150,540,600,90.0,-70", lines[1])
        assertFalse(log.csv().contains("deadbeef"))
    }

    @Test
    fun aMinuteWithNobodyHeardIsStillLogged() {
        val log = FieldLog()
        log.everyMinute(50)
        assertEquals("0,50,none,0,0,,", log.csv().trim().lines()[1])
    }
}
