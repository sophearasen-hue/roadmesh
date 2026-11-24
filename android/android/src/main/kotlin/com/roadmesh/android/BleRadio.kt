package com.roadmesh.android

import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertisingSet
import android.bluetooth.le.AdvertisingSetCallback
import android.bluetooth.le.AdvertisingSetParameters
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context

import android.os.Build
import android.os.ParcelUuid
import com.roadmesh.core.FrameReassembler
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

/**
 * Bluetooth LE transport (SPEC §4, BUILD_PLAN Step 5). Each safety message travels in one
 * non-connectable extended advertisement as service data under the RoadMesh UUID; one
 * long-running filtered scan hears everyone else. iPhones, which cannot broadcast long
 * messages, are connected to briefly and push their frames as GATT notifications. No pairing.
 *
 * Bytes in and out are opaque here: the Rust core encodes, signs, parses and verifies
 * (AUDIT S-09). This class never logs payloads.
 */
@SuppressLint("MissingPermission") // DriveService starts only after the permissions are granted
class BleRadio(private val context: Context, private val onFrame: (ByteArray, Int) -> Unit) {
    private val adapter: BluetoothAdapter? =
        context.getSystemService(BluetoothManager::class.java)?.adapter

    /** Extended advertising is required to send (frames are up to 218 bytes). */
    val canSend: Boolean
        get() = adapter?.isLeExtendedAdvertisingSupported == true &&
            (adapter.leMaximumAdvertisingDataLength) >= MAX_FRAME + AD_OVERHEAD

    /** Coded PHY (long range) is used when the phone supports it. */
    val codedPhy: Boolean get() = adapter?.isLeCodedPhySupported == true

    /** Largest advertising payload the controller accepts. */
    val maxDataLength: Int get() = adapter?.leMaximumAdvertisingDataLength ?: 0

    private var set: AdvertisingSet? = null
    private var starting = false
    private var pending: ByteArray? = null

    private val advCallback = object : AdvertisingSetCallback() {
        override fun onAdvertisingSetStarted(s: AdvertisingSet?, txPower: Int, status: Int) {
            starting = false
            if (status == ADVERTISE_SUCCESS && s != null) {
                set = s
                pending?.let { s.setAdvertisingData(data(it)) }
            }
        }

        override fun onAdvertisingSetStopped(s: AdvertisingSet?) {
            if (s == set) set = null
        }
    }

    private val scanCallback = object : ScanCallback() {
        override fun onScanResult(callbackType: Int, result: ScanResult) {
            val record = result.scanRecord ?: return
            val bytes = record.getServiceData(SERVICE)
            if (bytes != null) {
                if (bytes.size <= MAX_FRAME) onFrame(bytes, result.rssi)
            } else if (record.serviceUuids?.contains(SERVICE) == true) {
                // an iPhone: its frames come as GATT notifications (Step 6, option A)
                link(result.device, result.rssi)
            }
        }

        override fun onBatchScanResults(results: MutableList<ScanResult>) {
            results.forEach { onScanResult(ScanSettings.CALLBACK_TYPE_ALL_MATCHES, it) }
        }
    }

    private fun data(frame: ByteArray): AdvertiseData =
        AdvertiseData.Builder()
            .setIncludeDeviceName(false)
            .setIncludeTxPowerLevel(false)
            .addServiceData(SERVICE, frame)
            .build()

    private fun parameters(): AdvertisingSetParameters {
        val b = AdvertisingSetParameters.Builder()
            .setLegacyMode(false)
            .setConnectable(false)
            .setScannable(false)
            .setInterval(AdvertisingSetParameters.INTERVAL_LOW) // 100 ms
            .setTxPowerLevel(AdvertisingSetParameters.TX_POWER_HIGH)
        if (codedPhy) {
            b.setPrimaryPhy(BluetoothDevice.PHY_LE_CODED).setSecondaryPhy(BluetoothDevice.PHY_LE_CODED)
        } else {
            b.setPrimaryPhy(BluetoothDevice.PHY_LE_1M).setSecondaryPhy(BluetoothDevice.PHY_LE_1M)
        }
        // AUDIT S-13: apps cannot choose the address type (a system API); Android advertises
        // from a random private address by default, and a restarted set gets a new one.
        return b.build()
    }

    /** Broadcast `frame` (replaces the previous one). */
    fun broadcast(frame: ByteArray) {
        val a = adapter?.bluetoothLeAdvertiser ?: return
        if (!canSend) return
        pending = frame
        val s = set
        when {
            s != null -> s.setAdvertisingData(data(frame))
            !starting -> {
                starting = true
                a.startAdvertisingSet(parameters(), data(frame), null, null, null, advCallback)
            }
        }
    }

    /**
     * New pseudonym: stop and restart advertising so the Bluetooth address changes with it
     * (AUDIT S-13; to be confirmed with a BLE sniffer in the device test).
     */
    fun renewAddress() {
        set?.let { adapter?.bluetoothLeAdvertiser?.stopAdvertisingSet(advCallback) }
        set = null
        starting = false
        pending?.let { broadcast(it) }
    }

    // ------------------------------------------------------------ links to iPhones

    private class Link(val gatt: BluetoothGatt, val parts: FrameReassembler, @Volatile var rssi: Int, @Volatile var heard: Long)

    private val links = ConcurrentHashMap<String, Link>()

    private val gattCallback = object : BluetoothGattCallback() {
        override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
            if (newState == BluetoothProfile.STATE_CONNECTED) {
                gatt.requestMtu(MTU)
            } else if (newState == BluetoothProfile.STATE_DISCONNECTED) {
                links.remove(gatt.device.address)
                gatt.close()
            }
        }

        override fun onMtuChanged(gatt: BluetoothGatt, mtu: Int, status: Int) {
            gatt.discoverServices()
        }

        override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
            val ch = gatt.getService(SERVICE.uuid)?.getCharacteristic(FRAMES) ?: return gatt.disconnect()
            gatt.setCharacteristicNotification(ch, true)
            val cccd = ch.getDescriptor(CCCD) ?: return
            if (Build.VERSION.SDK_INT >= 33) {
                gatt.writeDescriptor(cccd, BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE)
            } else {
                @Suppress("DEPRECATION")
                cccd.value = BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
                @Suppress("DEPRECATION")
                gatt.writeDescriptor(cccd)
            }
        }

        override fun onCharacteristicChanged(gatt: BluetoothGatt, ch: BluetoothGattCharacteristic, value: ByteArray) {
            packet(gatt, value)
        }

        @Deprecated("Android 12 and older")
        override fun onCharacteristicChanged(gatt: BluetoothGatt, ch: BluetoothGattCharacteristic) {
            @Suppress("DEPRECATION")
            ch.value?.let { packet(gatt, it) }
        }
    }

    private fun packet(gatt: BluetoothGatt, value: ByteArray) {
        val l = links[gatt.device.address] ?: return
        val frame = l.parts.push(value)
        if (frame.isNotEmpty()) onFrame(frame, l.rssi)
    }

    private fun link(device: BluetoothDevice, rssi: Int) {
        val now = System.currentTimeMillis()
        links[device.address]?.let {
            it.rssi = rssi
            it.heard = now
            return
        }
        if (links.size >= MAX_LINKS) return
        val gatt = device.connectGatt(context, false, gattCallback, BluetoothDevice.TRANSPORT_LE) ?: return
        links[device.address] = Link(gatt, FrameReassembler(), rssi, now)
    }

    /** Drop links to iPhones not seen for 5 s (call once a second). */
    fun pruneLinks() {
        val cutoff = System.currentTimeMillis() - 5000
        for ((addr, l) in links) {
            if (l.heard < cutoff) {
                l.gatt.disconnect()
                links.remove(addr)
            }
        }
    }

    /** Start hearing other phones. */
    fun startScan() {
        val scanner = adapter?.bluetoothLeScanner ?: return
        val android = ScanFilter.Builder().setServiceData(SERVICE, ByteArray(0), ByteArray(0)).build()
        val iphone = ScanFilter.Builder().setServiceUuid(SERVICE).build()
        val settings = ScanSettings.Builder()
            .setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY)
            .setLegacy(false)
            .setPhy(ScanSettings.PHY_LE_ALL_SUPPORTED)
            .setCallbackType(ScanSettings.CALLBACK_TYPE_ALL_MATCHES)
            .setReportDelay(0)
            .build()
        scanner.startScan(listOf(android, iphone), settings, scanCallback)
    }

    /** Stop everything. */
    fun stop() {
        adapter?.bluetoothLeScanner?.stopScan(scanCallback)
        links.values.forEach { it.gatt.disconnect() }
        links.clear()
        adapter?.bluetoothLeAdvertiser?.stopAdvertisingSet(advCallback)
        set = null
        starting = false
        pending = null
    }

    companion object {
        /** RoadMesh service UUID (128-bit, unregistered on purpose: no 16-bit SIG number). */
        val SERVICE: ParcelUuid = ParcelUuid(UUID.fromString("4b6e7a10-2d3c-4e5f-8a9b-0c1d2e3f4a5b"))

        /** Largest frame: 2 header + 8 relay position + 31 body + 1 + 176 auth (SPEC §3). */
        const val MAX_FRAME = 218

        /** AD structure length + type + 128-bit UUID. */
        const val AD_OVERHEAD = 18

        /** GATT characteristic an iPhone notifies its frames on. */
        val FRAMES: UUID = UUID.fromString("4b6e7a11-2d3c-4e5f-8a9b-0c1d2e3f4a5b")

        /** Client Characteristic Configuration descriptor. */
        val CCCD: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")

        /** Requested ATT MTU: a 218-byte frame then fits in one notification. */
        const val MTU = 247

        /** At most this many iPhones connected at once. */
        const val MAX_LINKS = 6
    }
}
