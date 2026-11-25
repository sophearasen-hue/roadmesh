package com.roadmesh.android

import android.os.Build
import java.io.File

/**
 * Root detection (AUDIT S-12). A rooted phone could have its keys or messages tampered
 * with, so it **lowers trust** instead of blocking the driver: the phone keeps receiving
 * and warning, but sends nothing (no broadcast, no relay, no uploads). The server also
 * refuses certificates to devices that fail Play Integrity.
 */
object DeviceTrust {
    private val suPaths = listOf(
        "/system/bin/su", "/system/xbin/su", "/sbin/su", "/su/bin/su", "/system/app/Superuser.apk",
        "/data/adb/magisk", "/system/bin/.ext/su", "/system/usr/we-need-root/su", "/cache/su", "/data/local/su",
        "/data/local/xbin/su", "/data/local/bin/su",
    )

    /** True if this phone looks rooted. */
    fun compromised(): Boolean = signals(Build.TAGS, suPaths.any { File(it).exists() })

    /** The decision from its inputs (unit-tested). */
    fun signals(buildTags: String?, suFound: Boolean): Boolean =
        suFound || (buildTags?.contains("test-keys") == true)
}
