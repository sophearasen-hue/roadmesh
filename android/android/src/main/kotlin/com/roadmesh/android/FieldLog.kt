package com.roadmesh.android

/**
 * Field-test logger (BUILD_PLAN Step 5, Gate 1): per minute and distance band, how many
 * of the messages a heard phone sent (10 per second) actually arrived, the median signal
 * strength, and the battery level.
 *
 * Privacy (CLAUDE.md, AUDIT S-15): it never records coordinates or pseudonyms. Pseudonyms
 * are used only as in-memory counter keys and are dropped every second.
 */
class FieldLog(private val ratePerSecond: Int = 10) {
    /** Distance bands (m): label and upper bound. */
    val bands = listOf("0-50" to 50.0, "50-100" to 100.0, "100-150" to 150.0, "150-250" to 250.0, "250+" to Double.MAX_VALUE)

    private class BandStats {
        var received = 0
        var expected = 0
        val rssi = mutableListOf<Int>()
    }

    private var stats = bands.associate { it.first to BandStats() }.toMutableMap()
    private val thisSecond = HashMap<String, MutableList<Int>>() // pseudonym hex → RSSI values
    private val rows = mutableListOf<String>()
    private var minute = 0

    /** A frame from `pseudonymHex` was accepted with signal strength `rssi` (dBm). */
    @Synchronized
    fun onAccepted(pseudonymHex: String, rssi: Int) {
        thisSecond.getOrPut(pseudonymHex) { mutableListOf() }.add(rssi)
    }

    private fun band(distanceM: Double): String = bands.first { distanceM < it.second }.first

    /** Once a second: the peers heard (pseudonym hex → distance m). */
    @Synchronized
    fun everySecond(peerDistances: Map<String, Double>) {
        for ((id, d) in peerDistances) {
            val b = stats.getValue(band(d))
            val got = thisSecond[id]
            b.expected += ratePerSecond
            b.received += minOf(got?.size ?: 0, ratePerSecond)
            got?.let { b.rssi.addAll(it) }
        }
        thisSecond.clear()
    }

    /** Once a minute: close the minute with the battery level (%). */
    @Synchronized
    fun everyMinute(batteryPct: Int) {
        for ((label, s) in stats) {
            if (s.expected == 0) continue
            val pct = 100.0 * s.received / s.expected
            val med = s.rssi.sorted().let { if (it.isEmpty()) "" else it[it.size / 2].toString() }
            rows.add("$minute,$batteryPct,$label,${s.received},${s.expected},${"%.1f".format(java.util.Locale.ROOT, pct)},$med")
        }
        if (stats.values.all { it.expected == 0 }) rows.add("$minute,$batteryPct,none,0,0,,")
        stats = bands.associate { it.first to BandStats() }.toMutableMap()
        minute++
    }

    /** Forget everything ("Delete my data"). */
    @Synchronized
    fun clear() {
        stats = bands.associate { it.first to BandStats() }.toMutableMap()
        thisSecond.clear()
        rows.clear()
        minute = 0
    }

    /** The log as CSV. */
    @Synchronized
    fun csv(): String =
        (listOf("minute,battery_pct,distance_band_m,received,expected,delivery_pct,rssi_median_dbm") + rows)
            .joinToString("\n", postfix = "\n")
}
