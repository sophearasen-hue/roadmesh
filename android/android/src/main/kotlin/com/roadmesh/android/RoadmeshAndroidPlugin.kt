package com.roadmesh.android

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Handler
import android.os.Looper
import com.roadmesh.core.AlertLevel
import com.roadmesh.core.PeerInfo
import io.flutter.embedding.engine.plugins.FlutterPlugin
import io.flutter.embedding.engine.plugins.activity.ActivityAware
import io.flutter.embedding.engine.plugins.activity.ActivityPluginBinding
import io.flutter.plugin.common.EventChannel
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import io.flutter.plugin.common.PluginRegistry

/** Flutter ↔ drive service. The UI only shows what the service reports; it decides nothing. */
class RoadmeshAndroidPlugin : FlutterPlugin, MethodChannel.MethodCallHandler, EventChannel.StreamHandler,
    ActivityAware, PluginRegistry.RequestPermissionsResultListener {
    private lateinit var context: Context
    private lateinit var methods: MethodChannel
    private lateinit var events: EventChannel
    private var activity: Activity? = null
    private var sink: EventChannel.EventSink? = null
    private var permissionResult: MethodChannel.Result? = null
    private val main = Handler(Looper.getMainLooper())

    private val listener: (Update) -> Unit = { u ->
        val m = mapOf(
            "alerts" to u.decision.alerts.map(::alert),
            "peers" to u.peers.map(::peer),
            "own" to mapOf(
                "speed" to u.fix.speedMps,
                "heading" to u.fix.headingDeg,
                "accuracy" to u.fix.accuracyM,
                "limit" to u.road.speedLimitMps,
            ),
            "trip" to mapOf(
                "distance" to u.trip.distanceM,
                "avgSpeed" to u.trip.avgSpeedMps,
                "maxSpeed" to u.trip.maxSpeedMps,
                "score" to u.trip.score.toInt(),
                "counts" to counts(u.trip.counts),
            ),
            "events" to u.events.map {
                mapOf(
                    "age" to it.ageS,
                    "kind" to it.kind,
                    "me" to it.me,
                    "id" to Crypto.hex(it.pseudonym),
                    "kmh" to it.kmh,
                )
            },
        )
        main.post { sink?.success(m) }
    }

    override fun onAttachedToEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        context = binding.applicationContext
        methods = MethodChannel(binding.binaryMessenger, "roadmesh/radio").also { it.setMethodCallHandler(this) }
        events = EventChannel(binding.binaryMessenger, "roadmesh/radio/events").also { it.setStreamHandler(this) }
    }

    override fun onDetachedFromEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        methods.setMethodCallHandler(null)
        events.setStreamHandler(null)
        DriveService.listeners.remove(listener)
    }

    override fun onMethodCall(call: MethodCall, result: MethodChannel.Result) {
        val svc = DriveService.instance
        try {
            when (call.method) {
                "requestPermissions" -> requestPermissions(result)
                "start" -> {
                    val i = Intent(context, DriveService::class.java)
                        .putExtra(DriveService.EXTRA_TITLE, call.argument<String>("title") ?: "")
                        .putExtra(DriveService.EXTRA_TEXT, call.argument<String>("text") ?: "")
                        .putExtra(DriveService.EXTRA_VEHICLE, call.argument<Int>("vehicleType") ?: 1)
                    context.startForegroundService(i)
                    result.success(null)
                }
                "stop" -> {
                    context.stopService(Intent(context, DriveService::class.java))
                    result.success(null)
                }
                "status" -> result.success(svc?.status() ?: mapOf("running" to false))
                "createCertificateRequest" -> result.success(svc?.createCertificateRequest())
                "importCertificates" -> result.success(svc?.importCertificates())
                "exportFieldLog" -> result.success(svc?.exportFieldLog())
                "alertTone" -> {
                    alertTone(call.argument<String>("level") == "danger")
                    result.success(null)
                }
                "deleteMyData" -> {
                    svc?.deleteMyData()
                    result.success(null)
                }
                else -> result.notImplemented()
            }
        } catch (e: Exception) {
            result.error("roadmesh", e.javaClass.simpleName, null) // no payloads or keys in errors
        }
    }

    /** The engine chose the alert; this only sounds it (notification stream, follows the ringer). */
    private fun alertTone(danger: Boolean) {
        runCatching {
            val tone = android.media.ToneGenerator(android.media.AudioManager.STREAM_NOTIFICATION, 100)
            tone.startTone(
                if (danger) android.media.ToneGenerator.TONE_CDMA_ALERT_CALL_GUARD else android.media.ToneGenerator.TONE_PROP_BEEP,
                if (danger) 600 else 200,
            )
            main.postDelayed({ tone.release() }, 800)
        }
        runCatching {
            val v = context.getSystemService(android.os.Vibrator::class.java)
            v?.vibrate(android.os.VibrationEffect.createOneShot(if (danger) 500L else 150L, android.os.VibrationEffect.DEFAULT_AMPLITUDE))
        }
    }

    private val needed = arrayOf(
        Manifest.permission.BLUETOOTH_SCAN,
        Manifest.permission.BLUETOOTH_ADVERTISE,
        Manifest.permission.BLUETOOTH_CONNECT,
        Manifest.permission.ACCESS_FINE_LOCATION,
        Manifest.permission.POST_NOTIFICATIONS,
    )

    private fun requestPermissions(result: MethodChannel.Result) {
        val missing = needed.filter { context.checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }
        val act = activity
        if (missing.isEmpty()) return result.success(true)
        if (act == null) return result.success(false)
        permissionResult = result
        act.requestPermissions(missing.toTypedArray(), REQUEST)
    }

    override fun onRequestPermissionsResult(code: Int, permissions: Array<out String>, grants: IntArray): Boolean {
        if (code != REQUEST) return false
        permissionResult?.success(grants.all { it == PackageManager.PERMISSION_GRANTED })
        permissionResult = null
        return true
    }

    override fun onListen(arguments: Any?, events: EventChannel.EventSink?) {
        sink = events
        DriveService.listeners.add(listener)
    }

    override fun onCancel(arguments: Any?) {
        DriveService.listeners.remove(listener)
        sink = null
    }

    override fun onAttachedToActivity(binding: ActivityPluginBinding) {
        activity = binding.activity
        binding.addRequestPermissionsResultListener(this)
    }

    override fun onDetachedFromActivityForConfigChanges() { activity = null }
    override fun onReattachedToActivityForConfigChanges(binding: ActivityPluginBinding) = onAttachedToActivity(binding)
    override fun onDetachedFromActivity() { activity = null }

    private fun alert(a: com.roadmesh.core.AlertInfo): Map<String, Any?> = mapOf(
        "kind" to a.kind,
        "level" to when (a.level) {
            AlertLevel.CAUTION -> "caution"
            AlertLevel.WARNING -> "warning"
            AlertLevel.DANGER -> "danger"
        },
        "advice" to a.advice,
        "advisedDecel" to a.advisedDecelMps2,
        "distance" to a.distanceM,
        "ttc" to a.ttcS,
        "hops" to a.hops.toInt(),
        "target" to a.target?.let { Crypto.hex(it) },
    )

    private fun counts(c: com.roadmesh.core.EventCounts): Map<String, Int> = mapOf(
        "speeding" to c.speeding.toInt(),
        "hacc" to c.harshAccels.toInt(),
        "hbrake" to c.harshBrakes.toInt(),
        "overtake" to c.overtakes.toInt(),
    )

    private fun peer(p: PeerInfo): Map<String, Any?> = mapOf(
        "id" to Crypto.hex(p.pseudonym), // display key only; the UI never logs it
        "east" to p.eastM,
        "north" to p.northM,
        "distance" to p.distanceM,
        "speed" to p.speedMps,
        "accel" to p.accelMps2,
        "heading" to p.headingDeg,
        "vehicleType" to p.vehicleType.toInt(),
        "hops" to p.hops.toInt(),
        "accuracy" to p.accuracyM,
        "ageMs" to p.ageMs.toInt(),
        "score" to p.score.toInt(),
        "counts" to counts(p.counts),
    )

    companion object {
        private const val REQUEST = 7311
    }
}
