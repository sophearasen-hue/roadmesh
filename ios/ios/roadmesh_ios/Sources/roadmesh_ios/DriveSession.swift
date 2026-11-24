import CoreLocation
import Foundation
import RoadMeshKit
import UIKit

/// The drive session (BUILD_PLAN Step 6): location at full accuracy with background
/// updates, a 100 ms loop that runs the Rust engine (transmit → decide), the radio, and
/// the field log. All safety decisions are made in Rust; this class only moves data.
final class DriveSession: NSObject, CLLocationManagerDelegate {
    static var current: DriveSession?

    /// UI listener, 5 per second: what the engine decided and knows.
    var listener: ((DriveUpdate) -> Void)?

    private let keys = KeychainKeys()
    private lazy var enrolment = Enrolment(keys: keys)
    private let location = CLLocationManager()
    private var radio: BleRadio?
    private var engine: SafetyEngine?
    private var last: CLLocation?
    private var previous: CLLocation?
    private var timer: Timer?
    private var ticks = 0
    private var lastPseudonym = Data()
    private let fieldLog = FieldLog()
    private var bridge: CellularBridge?
    private let vehicleType: UInt8
    private let devAnchor: String?

    /// `devAnchorHex`: the developer CA's trust anchor (debug builds only, from Info.plist
    /// key `RoadMeshDevCaAnchor`); release builds pass nil and trust no test CA.
    init(vehicleType: UInt8, devAnchorHex: String?) {
        self.vehicleType = vehicleType
        devAnchor = devAnchorHex
        super.init()
        location.delegate = self
        location.desiredAccuracy = kCLLocationAccuracyBestForNavigation
        location.activityType = .automotiveNavigation
        location.distanceFilter = kCLDistanceFilterNone
    }

    func requestPermission() { location.requestWhenInUseAuthorization() }

    func start() {
        UIDevice.current.isBatteryMonitoringEnabled = true
        location.allowsBackgroundLocationUpdates = true
        location.pausesLocationUpdatesAutomatically = false
        location.startUpdatingLocation()
    }

    func locationManager(_ m: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        guard let loc = locations.last else { return }
        previous = last
        last = loc
        if engine == nil { begin(at: loc) }
    }

    private func begin(at loc: CLLocation) {
        var anchors: [Data] = []
        if let a = ServerConfig.caAnchor() { anchors.append(a) }
        if let hex = devAnchor, let a = Crypto.unhex(hex), a.count == 34 { anchors.append(a) } // debug only
        guard let e = try? SafetyEngine(originLat: loc.coordinate.latitude, originLon: loc.coordinate.longitude,
                                        trustAnchors: anchors, keys: keys, relay: false) else { return }
        if let stored = enrolment.storedCertificates() { _ = try? e.addCertificates(batch: stored, now: now()) }
        engine = e
        radio = BleRadio { [weak self] frame, rssi in self?.received(frame, rssi: rssi) }
        timer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self] _ in self?.tick() }
        startBridge(e)
    }

    private func now() -> Double { Date().timeIntervalSince1970 }

    /// The latest fix carried forward to `t` with its speed and course.
    private func fix(at t: Double) -> OwnFix? {
        guard let loc = last else { return nil }
        let age = min(max(t - loc.timestamp.timeIntervalSince1970, 0), 1)
        let speed = max(loc.speed, 0)
        let course = loc.course >= 0 ? loc.course : 0
        let d = speed * age
        let rad = course * .pi / 180
        let lat = loc.coordinate.latitude + d * cos(rad) / 111_320
        let lon = loc.coordinate.longitude + d * sin(rad) / (111_320 * cos(loc.coordinate.latitude * .pi / 180))
        var accel = 0.0
        var yaw = 0.0
        if let p = previous {
            let dt = loc.timestamp.timeIntervalSince(p.timestamp)
            if dt > 0.05, p.speed >= 0 { accel = (speed - p.speed) / dt }
            if dt > 0.05, p.course >= 0, loc.course >= 0, speed > 2 {
                yaw = -((loc.course - p.course + 540).truncatingRemainder(dividingBy: 360) - 180) / dt
            }
        }
        return OwnFix(lat: lat, lon: lon, headingDeg: course, speedMps: speed,
                      accelMps2: min(max(accel, -12), 12), yawRateDps: yaw,
                      accuracyM: loc.horizontalAccuracy >= 0 ? loc.horizontalAccuracy : 10,
                      vehicleType: vehicleType)
    }

    /// Speed limits and junctions arrive with the map (Step 8).
    private let road = RoadInfo(speedLimitMps: nil, atJunction: false)

    private func tick() {
        guard let e = engine, let r = radio else { return }
        let t = now()
        guard let f = fix(at: t) else { return }
        let frames = e.transmit(now: t, fix: f, road: road)
        if let frame = frames.first { r.broadcast(frame) }
        frames.forEach { bridge?.upload($0) } // hazard events only
        let p = e.pseudonym(now: t)
        if p != lastPseudonym {
            if !lastPseudonym.isEmpty { r.renewAddress() }
            lastPseudonym = p
        }
        let d = e.decide(now: t, fix: f, road: road)
        let peers = e.peers(now: t, fix: f)
        ticks += 1
        if ticks % 10 == 0 {
            bridge?.follow(lat: f.lat, lon: f.lon)
            e.takeReports().forEach { bridge?.report($0) }
            fieldLog.everySecond(Dictionary(peers.map { (Crypto.hex($0.pseudonym), $0.distanceM) },
                                            uniquingKeysWith: { a, _ in a }))
            r.pruneLinks()
        }
        if ticks % 600 == 0 { fieldLog.everyMinute(batteryPct: Int(max(UIDevice.current.batteryLevel, 0) * 100)) }
        if ticks % 2 == 0, let l = listener {
            l(DriveUpdate(decision: d, peers: peers, fix: f, road: road,
                          trip: e.trip(), events: e.events(now: t, max: 40)))
        }
    }

    /// The 4G link (docs/SERVER.md), if this build is configured for a server.
    private func startBridge(_ e: SafetyEngine) {
        guard let config = ServerConfig.fromBundle() else { return }
        let keys = self.keys
        let b = CellularBridge(config: config, identity: { keys.deviceIdentity() }) { [weak self] ev in
            DispatchQueue.main.async { self?.onCellular(ev) }
        }
        bridge = b
        b.start()
        if e.validCertificates(now: now()) < 5 || keys.deviceIdentity() == nil {
            let enrolment = self.enrolment
            Task {
                guard let batch = try? await enrolment.enrolOnline(config) else { return }
                await MainActor.run { _ = try? self.engine?.addCertificates(batch: batch, now: self.now()) }
            }
        }
    }

    /// From the server: checked by the engine like any radio frame.
    private func onCellular(_ ev: LinkEvent) {
        guard let e = engine else { return }
        switch ev {
        case .hazardEvent(let frame):
            let t = now()
            if let f = fix(at: t) { _ = e.receive(frame: frame, now: t, fix: f) }
        case .revocationList(let list):
            _ = try? e.loadRevocationList(bytes: list)
        default:
            break
        }
    }

    private func received(_ frame: Data, rssi: Int) {
        guard let e = engine else { return }
        let t = now()
        guard let f = fix(at: t) else { return }
        if case let .accepted(pseudonym) = e.receive(frame: frame, now: t, fix: f) {
            fieldLog.onAccepted(pseudonymHex: Crypto.hex(pseudonym), rssi: rssi)
            bridge?.upload(frame) // events heard by radio reach phones out of range
        }
    }

    func stop() {
        timer?.invalidate()
        location.stopUpdatingLocation()
        radio?.stop()
        radio = nil
        bridge?.stop()
        bridge = nil
        engine = nil
    }

    func status() -> [String: Any] {
        [
            "running": engine != nil,
            "canSend": engine != nil, // iPhones send over Bluetooth links (option A)
            "codedPhy": false,
            "certificates": Int(engine?.validCertificates(now: now()) ?? 0),
        ]
    }

    /// "Delete my data" on this phone (trip, behaviour, peers heard, field log).
    func deleteMyData() {
        engine?.deleteMyData()
        fieldLog.clear()
        bridge?.deleteMyData()
    }

    func createCertificateRequest() throws -> String { try enrolment.createRequest().path }

    func importCertificates() throws -> Int {
        let batch = try enrolment.importCertificates()
        return Int(try engine?.addCertificates(batch: batch, now: now()) ?? 0)
    }

    func exportFieldLog() throws -> String {
        let url = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("roadmesh-field-log.csv")
        try fieldLog.csv().write(to: url, atomically: true, encoding: .utf8)
        return url.path
    }
}

/// One UI update. The UI decides nothing.
struct DriveUpdate {
    let decision: DecisionInfo
    let peers: [PeerInfo]
    let fix: OwnFix
    let road: RoadInfo
    let trip: TripInfo
    let events: [EventInfo]
}
