package com.roadmesh.android

import com.roadmesh.core.CellularLink
import com.roadmesh.core.LinkEvent
import java.io.OutputStream
import java.net.SocketTimeoutException
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import javax.net.ssl.SSLSocket

/**
 * The 4G link to the edge server (SPEC §5 "Cellular bridge"): one MQTT session over
 * pinned TLS 1.3 with the device certificate. Hazard events go up; events from phones
 * out of radio range and the revocation list come down and are handed to the Rust
 * engine, which checks them like radio frames. Reconnects with back-off. Never logs
 * contents, positions or pseudonyms.
 */
class CellularBridge(
    private val config: ServerConfig,
    private val device: () -> PinnedTls.DeviceKeyManager?,
    private val onEvent: (LinkEvent) -> Unit,
) {
    private val link = CellularLink()
    private val outbox = LinkedBlockingQueue<ByteArray>(256)
    @Volatile private var running = false
    @Volatile private var socket: SSLSocket? = null
    private var thread: Thread? = null

    /** Whether a session is open (for the HUD's link type). */
    @Volatile var connected = false
        private set

    fun start() {
        if (running) return
        running = true
        thread = Thread({ loop() }, "roadmesh-cellular").apply { isDaemon = true; start() }
    }

    fun stop() {
        running = false
        runCatching { socket?.close() }
        thread?.interrupt()
    }

    /** A frame this phone sent or accepted: uploaded if it is a hazard event. */
    fun upload(frame: ByteArray) = enqueue(link.upload(frame))

    /** About once a second: subscribe to the cells around the phone (never sent as a position). */
    fun followPosition(lat: Double, lon: Double) {
        if (connected) enqueue(link.followPosition(lat, lon))
    }

    fun report(report: ByteArray) = enqueue(link.report(report))

    fun deleteMyData() = enqueue(link.deleteMyData())

    private fun enqueue(bytes: ByteArray) {
        if (bytes.isNotEmpty() && connected) outbox.offer(bytes) // full queue: drop (QoS 0)
    }

    private fun loop() {
        var backoffMs = 2_000L
        while (running) {
            val dev = device()
            if (dev == null) { // not enrolled yet
                sleep(30_000)
                continue
            }
            try {
                val s = PinnedTls.connect(config.host, config.mqttPort, config.pins, dev)
                socket = s
                s.soTimeout = 1_000
                session(s)
                backoffMs = 2_000L
            } catch (e: Exception) {
                // network down, server refused, pin mismatch: try again later
            } finally {
                connected = false
                outbox.clear()
                runCatching { socket?.close() }
                socket = null
            }
            sleep(backoffMs)
            backoffMs = (backoffMs * 2).coerceAtMost(300_000L)
        }
    }

    private fun session(s: SSLSocket) {
        val out = s.outputStream
        val input = s.inputStream
        out.write(link.connect(KEEP_ALIVE_S.toUShort()))
        out.flush()
        val buf = ByteArray(4096)
        var lastPing = System.currentTimeMillis()
        var lastRead = System.currentTimeMillis()
        while (running) {
            drain(out)
            val n = try {
                input.read(buf)
            } catch (e: SocketTimeoutException) {
                0
            }
            if (n < 0) return
            val now = System.currentTimeMillis()
            if (n > 0) {
                lastRead = now
                for (ev in link.onBytes(buf.copyOf(n))) {
                    when (ev) {
                        is LinkEvent.Connected -> connected = true
                        is LinkEvent.Close, is LinkEvent.Refused -> return
                        else -> onEvent(ev)
                    }
                }
            }
            if (now - lastPing > KEEP_ALIVE_S * 500L) {
                out.write(link.ping())
                out.flush()
                lastPing = now
            }
            if (now - lastRead > KEEP_ALIVE_S * 1_500L) return // server silent
        }
    }

    private fun drain(out: OutputStream) {
        var wrote = false
        while (true) {
            val b = outbox.poll(0, TimeUnit.MILLISECONDS) ?: break
            out.write(b)
            wrote = true
        }
        if (wrote) out.flush()
    }

    private fun sleep(ms: Long) {
        try {
            Thread.sleep(ms)
        } catch (e: InterruptedException) {
            // stopping
        }
    }

    companion object {
        const val KEEP_ALIVE_S = 60
    }
}
