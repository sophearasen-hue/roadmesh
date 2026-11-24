package com.roadmesh.android

import android.Manifest
import android.annotation.SuppressLint
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.BatteryManager
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import com.roadmesh.core.DecisionInfo
import com.roadmesh.core.OwnFix
import com.roadmesh.core.PeerInfo
import com.roadmesh.core.RoadInfo
import com.roadmesh.core.TripInfo
import com.roadmesh.core.EventInfo
import com.roadmesh.core.LinkEvent
import com.roadmesh.core.RxResult
import com.roadmesh.core.SafetyEngine
import kotlin.math.cos
import kotlin.math.sin

/**
 * The drive session: a foreground service of type connectedDevice + location, so warnings
 * keep working with the screen off (targetSdk 36 rules). Every 100 ms it hands the latest
 * position to the Rust safety engine, broadcasts the signed message, and publishes the
 * decision and the peers to the Flutter UI. All safety decisions are made in Rust.
 */
class DriveService : Service() {
    private val handler = Handler(Looper.getMainLooper())
    private lateinit var keys: KeystoreKeys
    private lateinit var enrolment: Enrolment
    private lateinit var radio: BleRadio
    private var engine: SafetyEngine? = null
    private var location: Location? = null
    private var prevLocation: Location? = null
    private var vehicleType: UByte = 1u
    private var lastPseudonym = ""
    private var ticks = 0L
    private val fieldLog = FieldLog()
    private var bridge: CellularBridge? = null
    /** AUDIT S-12: a rooted phone receives and warns but sends nothing. */
    private val receiveOnly: Boolean by lazy { DeviceTrust.compromised() }

    private val listener = LocationListener { loc ->
        prevLocation = location
        location = loc
        if (engine == null) start(loc)
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val title = intent?.getStringExtra(EXTRA_TITLE) ?: ""
        val text = intent?.getStringExtra(EXTRA_TEXT) ?: ""
        vehicleType = (intent?.getIntExtra(EXTRA_VEHICLE, 1) ?: 1).toUByte()
        startForeground(NOTIFICATION_ID, notification(title, text),
            ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE or ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION)
        keys = KeystoreKeys(this)
        enrolment = Enrolment(this, keys)
        radio = BleRadio(this) { frame, rssi -> onFrame(frame, rssi) }
        requestLocation()
        instance = this
        return START_STICKY
    }

    @SuppressLint("MissingPermission") // started only after the permissions are granted
    private fun requestLocation() {
        if (checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) != android.content.pm.PackageManager.PERMISSION_GRANTED) return
        val lm = getSystemService(LocationManager::class.java)
        lm.requestLocationUpdates(LocationManager.GPS_PROVIDER, 100L, 0f, listener, Looper.getMainLooper())
    }

    private fun start(first: Location) {
        val anchors = listOfNotNull(
            Crypto.unhex(BuildConfig.CA_ANCHOR)?.takeIf { it.size == 34 },
            Crypto.unhex(BuildConfig.DEV_CA_ANCHOR)?.takeIf { it.size == 34 }, // debug builds only
        )
        val e = SafetyEngine(first.latitude, first.longitude, anchors, keys, false)
        enrolment.storedCertificates()?.let { runCatching { e.addCertificates(it, now()) } }
        engine = e
        startBridge(e)
        radio.startScan()
        handler.post(tick)
    }

    private fun now(): Double = System.currentTimeMillis() / 1000.0

    /** The latest fix carried forward to `t` with its speed and course (fixes come at 1–10 Hz). */
    private fun fixAt(t: Double): OwnFix? {
        val loc = location ?: return null
        val age = (t - loc.time / 1000.0).coerceIn(0.0, 1.0)
        val speed = if (loc.hasSpeed()) loc.speed.toDouble() else 0.0
        val course = if (loc.hasBearing()) loc.bearing.toDouble() else 0.0
        val d = speed * age
        val rad = Math.toRadians(course)
        val lat = loc.latitude + d * cos(rad) / 111_320.0
        val lon = loc.longitude + d * sin(rad) / (111_320.0 * cos(Math.toRadians(loc.latitude)))
        val prev = prevLocation
        val dt = prev?.let { (loc.time - it.time) / 1000.0 } ?: 0.0
        val accel = if (prev != null && dt > 0.05 && prev.hasSpeed()) (speed - prev.speed) / dt else 0.0
        val yaw = if (prev != null && dt > 0.05 && prev.hasBearing() && loc.hasBearing() && speed > 2.0) {
            -(((course - prev.bearing + 540.0) % 360.0) - 180.0) / dt // compass clockwise → counter-clockwise
        } else 0.0
        return OwnFix(lat, lon, course, speed, accel.coerceIn(-12.0, 12.0), yaw,
            if (loc.hasAccuracy()) loc.accuracy.toDouble() else 10.0, vehicleType)
    }

    private val road = RoadInfo(null, false) // speed limits and junctions arrive with the map (Step 8)

    private val tick = object : Runnable {
        override fun run() {
            val e = engine ?: return
            val t = now()
            fixAt(t)?.let { fix ->
                val frames = e.transmit(t, fix, road)
                if (!receiveOnly) {
                    frames.firstOrNull()?.let { radio.broadcast(it) }
                    frames.forEach { bridge?.upload(it) } // hazard events only
                }
                val p = Crypto.hex(e.pseudonym(t))
                if (p != lastPseudonym) {
                    if (lastPseudonym.isNotEmpty()) radio.renewAddress()
                    lastPseudonym = p
                }
                val d = e.decide(t, fix, road)
                val peers = e.peers(t, fix)
                ticks++
                if (ticks % 10 == 0L) {
                    bridge?.followPosition(fix.lat, fix.lon)
                    e.takeReports().forEach { bridge?.report(it) }
                    fieldLog.everySecond(peers.associate { Crypto.hex(it.pseudonym) to it.distanceM })
                    radio.pruneLinks()
                }
                if (ticks % 600 == 0L) fieldLog.everyMinute(battery())
                if (ticks % 2 == 0L && listeners.isNotEmpty()) {
                    val u = Update(d, peers, fix, road, e.trip(), e.events(t, 40u))
                    listeners.forEach { it(u) }
                }
            }
            handler.postDelayed(this, 100)
        }
    }

    /** The 4G link (docs/SERVER.md), if this build is configured for a server. */
    private fun startBridge(e: SafetyEngine) {
        val config = ServerConfig.fromBuild() ?: return
        val b = CellularBridge(config, {
            enrolment.deviceCertificate()?.let { PinnedTls.DeviceKeyManager(KeystoreKeys.DEVICE_ALIAS, it) }
        }) { ev -> handler.post { onCellular(ev) } }
        bridge = b
        b.start()
        // no certificates or no device certificate yet: enrol online (Play Integrity)
        if (e.validCertificates(now()) < 5u || enrolment.deviceCertificate() == null) {
            Thread({
                runCatching { enrolment.enrolOnline(config) }.onSuccess { batch ->
                    handler.post { runCatching { engine?.addCertificates(batch, now()) } }
                }
            }, "roadmesh-enrol").start()
        }
    }

    /** From the server: checked by the engine like any radio frame. */
    private fun onCellular(ev: LinkEvent) {
        val e = engine ?: return
        when (ev) {
            is LinkEvent.HazardEvent -> {
                val t = now()
                fixAt(t)?.let { e.receive(ev.frame, t, it) }
            }
            is LinkEvent.RevocationList -> runCatching { e.loadRevocationList(ev.list) }
            else -> {}
        }
    }

    private fun onFrame(frame: ByteArray, rssi: Int) {
        val e = engine ?: return
        val t = now()
        val fix = fixAt(t) ?: return
        val r = e.receive(frame, t, fix)
        if (r is RxResult.Accepted) {
            fieldLog.onAccepted(Crypto.hex(r.pseudonym), rssi)
            if (!receiveOnly) bridge?.upload(frame) // events heard by radio reach phones out of range
        }
    }

    private fun battery(): Int =
        getSystemService(BatteryManager::class.java).getIntProperty(BatteryManager.BATTERY_PROPERTY_CAPACITY)

    private fun notification(title: String, text: String): Notification {
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CHANNEL, title, NotificationManager.IMPORTANCE_LOW))
        return Notification.Builder(this, CHANNEL)
            .setContentTitle(title)
            .setContentText(text)
            .setSmallIcon(android.R.drawable.stat_sys_data_bluetooth)
            .setOngoing(true)
            .build()
    }

    override fun onDestroy() {
        handler.removeCallbacks(tick)
        getSystemService(LocationManager::class.java).removeUpdates(listener)
        if (::radio.isInitialized) radio.stop()
        bridge?.stop()
        bridge = null
        engine?.close()
        engine = null
        instance = null
        super.onDestroy()
    }

    /** Radio facts for the status screen. */
    fun status(): Map<String, Any> = mapOf(
        "canSend" to (radio.canSend && !receiveOnly),
        "receiveOnly" to receiveOnly,
        "codedPhy" to radio.codedPhy,
        "maxDataLength" to radio.maxDataLength,
        "certificates" to (engine?.validCertificates(now())?.toInt() ?: 0),
        "running" to (engine != null),
    )

    /** "Delete my data" on this phone (trip, behaviour, peers heard, field log). */
    fun deleteMyData() {
        engine?.deleteMyData()
        fieldLog.clear()
        bridge?.deleteMyData()
    }

    fun createCertificateRequest(): String = enrolment.createRequest().absolutePath

    fun importCertificates(): Int {
        val batch = enrolment.importCertificates()
        return engine?.addCertificates(batch, now())?.toInt() ?: 0
    }

    fun exportFieldLog(): String {
        val f = java.io.File(getExternalFilesDir(null) ?: filesDir, "roadmesh-field-log.csv")
        f.writeText(fieldLog.csv())
        return f.absolutePath
    }

    companion object {
        const val EXTRA_TITLE = "title"
        const val EXTRA_TEXT = "text"
        const val EXTRA_VEHICLE = "vehicleType"
        private const val CHANNEL = "roadmesh_drive"
        private const val NOTIFICATION_ID = 7

        /** The running service (one per process). */
        @Volatile
        var instance: DriveService? = null

        /** UI listeners (5 Hz). */
        val listeners = java.util.concurrent.CopyOnWriteArrayList<(Update) -> Unit>()
    }
}

/** One UI update: what the Rust engine decided and what it knows. The UI decides nothing. */
data class Update(
    val decision: DecisionInfo,
    val peers: List<PeerInfo>,
    val fix: OwnFix,
    val road: RoadInfo,
    val trip: TripInfo,
    val events: List<EventInfo>,
)
