import CoreBluetooth
import Foundation

/// Bluetooth LE transport on iOS (BUILD_PLAN Step 6, decision "option A").
///
/// iOS lets an app advertise only service UUIDs (about 28 bytes), so an iPhone cannot
/// broadcast a 218-byte frame the way Android does. Instead:
/// * as a **peripheral** it advertises the RoadMesh service UUID and pushes each frame as
///   GATT notifications (fragmented to the link's packet size by the Rust core) to every
///   phone that subscribes;
/// * as a **central** it scans for the RoadMesh service: Android phones put whole frames
///   in extended advertisements (service data), iPhones are connected and subscribed to.
///
/// Bytes are opaque here: the Rust core splits, reassembles, parses and verifies
/// (AUDIT S-09). Nothing is logged.
final class BleRadio: NSObject, CBPeripheralManagerDelegate, CBCentralManagerDelegate, CBPeripheralDelegate {
    static let service = CBUUID(string: "4B6E7A10-2D3C-4E5F-8A9B-0C1D2E3F4A5B")
    static let frames = CBUUID(string: "4B6E7A11-2D3C-4E5F-8A9B-0C1D2E3F4A5B")
    static let maxLinks = 6

    private let queue = DispatchQueue(label: "roadmesh.ble")
    private var peripheral: CBPeripheralManager!
    private var central: CBCentralManager!
    private var characteristic: CBMutableCharacteristic?
    private var subscribers: [CBCentral] = []
    private var frameNo: UInt8 = 0
    private var links: [UUID: (CBPeripheral, FrameReassembler, Int)] = [:]
    private var lastHeard: [UUID: Date] = [:]
    private let onFrame: (Data, Int) -> Void

    /// `onFrame(frame, rssi)` is called on the Bluetooth queue.
    init(onFrame: @escaping (Data, Int) -> Void) {
        self.onFrame = onFrame
        super.init()
        peripheral = CBPeripheralManager(delegate: self, queue: queue)
        central = CBCentralManager(delegate: self, queue: queue)
    }

    // MARK: sending (peripheral)

    func peripheralManagerDidUpdateState(_ p: CBPeripheralManager) {
        guard p.state == .poweredOn, characteristic == nil else { return }
        let c = CBMutableCharacteristic(type: Self.frames, properties: [.notify], value: nil, permissions: [])
        let s = CBMutableService(type: Self.service, primary: true)
        s.characteristics = [c]
        characteristic = c
        p.add(s)
    }

    func peripheralManager(_ p: CBPeripheralManager, didAdd service: CBService, error: Error?) {
        guard error == nil else { return }
        startAdvertising()
    }

    /// No local name: only the service UUID (privacy, AUDIT S-13).
    private func startAdvertising() {
        peripheral.startAdvertising([CBAdvertisementDataServiceUUIDsKey: [Self.service]])
    }

    func peripheralManager(_ p: CBPeripheralManager, central: CBCentral, didSubscribeTo c: CBCharacteristic) {
        if !subscribers.contains(where: { $0.identifier == central.identifier }) { subscribers.append(central) }
    }

    func peripheralManager(_ p: CBPeripheralManager, central: CBCentral, didUnsubscribeFrom c: CBCharacteristic) {
        subscribers.removeAll { $0.identifier == central.identifier }
    }

    /// Send `frame` to every subscribed phone (replaces the previous frame).
    func broadcast(_ frame: Data) {
        queue.async { [self] in
            guard let c = characteristic, !subscribers.isEmpty else { return }
            let packet = subscribers.map { $0.maximumUpdateValueLength }.min() ?? 20
            frameNo &+= 1
            for part in splitFrame(frame: frame, frameNo: frameNo, maxPacket: UInt32(packet)) {
                // queue full: drop the rest of this frame; the next one comes in 100 ms
                if !peripheral.updateValue(part, for: c, onSubscribedCentrals: nil) { break }
            }
        }
    }

    /// New pseudonym: restart advertising (iOS rotates the address itself; AUDIT S-13 is
    /// confirmed with a sniffer in the device test).
    func renewAddress() {
        queue.async { [self] in
            peripheral.stopAdvertising()
            startAdvertising()
        }
    }

    // MARK: hearing (central)

    func centralManagerDidUpdateState(_ c: CBCentralManager) {
        guard c.state == .poweredOn else { return }
        c.scanForPeripherals(withServices: [Self.service],
                             options: [CBCentralManagerScanOptionAllowDuplicatesKey: true])
    }

    func centralManager(_ c: CBCentralManager, didDiscover p: CBPeripheral,
                        advertisementData ad: [String: Any], rssi: NSNumber) {
        // an Android phone: the whole frame is in the advertisement
        if let sd = ad[CBAdvertisementDataServiceDataKey] as? [CBUUID: Data], let frame = sd[Self.service] {
            onFrame(frame, rssi.intValue)
            return
        }
        // an iPhone: connect and subscribe (a few links at most)
        lastHeard[p.identifier] = Date()
        if var link = links[p.identifier] {
            link.2 = rssi.intValue
            links[p.identifier] = link
            return
        }
        guard links.count < Self.maxLinks else { return }
        links[p.identifier] = (p, FrameReassembler(), rssi.intValue)
        p.delegate = self
        c.connect(p, options: nil)
    }

    func centralManager(_ c: CBCentralManager, didConnect p: CBPeripheral) {
        p.discoverServices([Self.service])
    }

    func centralManager(_ c: CBCentralManager, didFailToConnect p: CBPeripheral, error: Error?) {
        links[p.identifier] = nil
    }

    func centralManager(_ c: CBCentralManager, didDisconnectPeripheral p: CBPeripheral, error: Error?) {
        links[p.identifier] = nil
    }

    func peripheral(_ p: CBPeripheral, didDiscoverServices error: Error?) {
        guard let s = p.services?.first(where: { $0.uuid == Self.service }) else { return }
        p.discoverCharacteristics([Self.frames], for: s)
    }

    func peripheral(_ p: CBPeripheral, didDiscoverCharacteristicsFor s: CBService, error: Error?) {
        guard let ch = s.characteristics?.first(where: { $0.uuid == Self.frames }) else { return }
        p.setNotifyValue(true, for: ch)
    }

    func peripheral(_ p: CBPeripheral, didUpdateValueFor ch: CBCharacteristic, error: Error?) {
        guard let value = ch.value, let link = links[p.identifier] else { return }
        let frame = link.1.push(packet: value)
        if !frame.isEmpty { onFrame(frame, link.2) }
    }

    /// Drop links to iPhones not seen for 5 s (called once a second).
    func pruneLinks() {
        queue.async { [self] in
            let now = Date()
            for (id, link) in links where now.timeIntervalSince(lastHeard[id] ?? .distantPast) > 5 {
                central.cancelPeripheralConnection(link.0)
                links[id] = nil
            }
        }
    }

    func stop() {
        queue.async { [self] in
            peripheral.stopAdvertising()
            central.stopScan()
            for (_, link) in links { central.cancelPeripheralConnection(link.0) }
            links.removeAll()
            subscribers.removeAll()
        }
    }
}
