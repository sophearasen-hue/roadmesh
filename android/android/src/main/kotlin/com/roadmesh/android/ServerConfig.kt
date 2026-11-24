package com.roadmesh.android

import android.util.Base64

/**
 * The edge server (docs/SERVER.md), fixed at build time through Gradle properties:
 * `roadmesh.serverHost`, `roadmesh.serverPins` (comma-separated base64 SPKI SHA-256
 * pins: the current key and at least one backup, AUDIT S-17), `roadmesh.caAnchor`
 * (pseudonym CA trust anchor, 68 hex chars) and `roadmesh.cloudProject` (Google Cloud
 * project number for Play Integrity). Without all of them the cellular bridge is off.
 */
data class ServerConfig(
    val host: String,
    val pins: List<ByteArray>,
    val cloudProject: Long,
    val mqttPort: Int = 8883,
    val enrolPort: Int = 8884,
) {
    companion object {
        /** From BuildConfig, or null if the bridge is not configured. */
        fun fromBuild(): ServerConfig? = parse(
            BuildConfig.SERVER_HOST, BuildConfig.SERVER_PINS, BuildConfig.CLOUD_PROJECT,
        )

        /** Parse; null unless a host, two or more valid pins and a project number are given. */
        fun parse(host: String, pins: String, cloudProject: String): ServerConfig? {
            if (host.isBlank()) return null
            val p = pins.split(',').map { it.trim() }.filter { it.isNotEmpty() }.map {
                runCatching { Base64.decode(it, Base64.DEFAULT) }.getOrNull() ?: return null
            }
            if (p.size < 2 || p.any { it.size != 32 }) return null // current + backup pin
            val project = cloudProject.toLongOrNull() ?: return null
            return ServerConfig(host.trim(), p, project)
        }
    }
}
