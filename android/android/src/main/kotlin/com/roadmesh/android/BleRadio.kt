package com.roadmesh.android

import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothManager
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertisingSet
import android.bluetooth.le.AdvertisingSetCallback
import android.bluetooth.le.AdvertisingSetParameters
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context

import android.os.ParcelUuid
import java.util.UUID

/**
 * Bluetooth LE transport (SPEC §4, BUILD_PLAN Step 5). Each safety message travels in one
 * non-connectable extended advertisement as service data under the RoadMesh UUID; one
 * long-running filtered scan hears everyone else. No connections, no pairing.
 *
 * Bytes in and out are opaque here: the Rust core encodes, signs, parses and verifies
 * (AUDIT S-09). This class never logs payloads.
 */
@SuppressLint("MissingPermission") // DriveService starts only after the permissions are granted
class BleRadio(context: Context, private val onFrame: (ByteArray, Int) -> Unit) {
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
            val bytes = result.scanRecord?.getServiceData(SERVICE) ?: return
            if (bytes.size <= MAX_FRAME) onFrame(bytes, result.rssi)
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

    /** Start hearing other phones. */
    fun startScan() {
        val scanner = adapter?.bluetoothLeScanner ?: return
        val filter = ScanFilter.Builder().setServiceData(SERVICE, ByteArray(0), ByteArray(0)).build()
        val settings = ScanSettings.Builder()
            .setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY)
            .setLegacy(false)
            .setPhy(ScanSettings.PHY_LE_ALL_SUPPORTED)
            .setCallbackType(ScanSettings.CALLBACK_TYPE_ALL_MATCHES)
            .setReportDelay(0)
            .build()
        scanner.startScan(listOf(filter), settings, scanCallback)
    }

    /** Stop everything. */
    fun stop() {
        adapter?.bluetoothLeScanner?.stopScan(scanCallback)
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
    }
}
