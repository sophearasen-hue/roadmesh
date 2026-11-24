import Foundation
import Network

/// The 4G link to the edge server (SPEC §5 "Cellular bridge"): one MQTT session over
/// pinned TLS 1.3 with the device identity. Hazard events go up; events from phones out
/// of radio range and the revocation list come down and go to the Rust engine, which
/// checks them like radio frames. Reconnects with back-off. Never logs contents.
final class CellularBridge {
    private let config: ServerConfig
    private let identity: () -> SecIdentity?
    private let onEvent: (LinkEvent) -> Void
    private let queue = DispatchQueue(label: "roadmesh.cellular")
    private let link = CellularLink()
    private var connection: NWConnection?
    private var running = false
    private var backoff: TimeInterval = 2
    private var pingTimer: DispatchSourceTimer?
    private(set) var connected = false
    static let keepAlive: UInt16 = 60

    init(config: ServerConfig, identity: @escaping () -> SecIdentity?, onEvent: @escaping (LinkEvent) -> Void) {
        self.config = config
        self.identity = identity
        self.onEvent = onEvent
    }

    func start() {
        queue.async {
            guard !self.running else { return }
            self.running = true
            self.open()
        }
    }

    func stop() {
        queue.async {
            self.running = false
            self.close()
        }
    }

    /// A frame this phone sent or accepted: uploaded if it is a hazard event.
    func upload(_ frame: Data) { send { $0.upload(frame: frame) } }

    /// About once a second: the cells around the phone (never sent as a position).
    func follow(lat: Double, lon: Double) { send { $0.followPosition(lat: lat, lon: lon) } }

    func report(_ r: Data) { send { $0.report(report: r) } }

    func deleteMyData() { send { $0.deleteMyData() } }

    private func send(_ make: @escaping (CellularLink) -> Data) {
        queue.async {
            guard self.connected, let c = self.connection else { return }
            let bytes = make(self.link)
            if !bytes.isEmpty { c.send(content: bytes, completion: .idempotent) }
        }
    }

    private func open() {
        guard running else { return }
        guard let id = identity() else { // not enrolled yet
            queue.asyncAfter(deadline: .now() + 30) { self.open() }
            return
        }
        let c = PinnedConnection.make(config, port: config.mqttPort, identity: id, queue: queue)
        connection = c
        c.stateUpdateHandler = { [weak self] state in
            guard let self else { return }
            switch state {
            case .ready:
                c.send(content: self.link.connect(keepAliveS: Self.keepAlive), completion: .idempotent)
                self.receive(c)
                self.startPings(c)
            case .failed, .cancelled:
                self.retry()
            default:
                break
            }
        }
        c.start(queue: queue)
    }

    private func receive(_ c: NWConnection) {
        c.receive(minimumIncompleteLength: 1, maximumLength: 4096) { [weak self] data, _, done, error in
            guard let self else { return }
            if let data, !data.isEmpty {
                for ev in self.link.onBytes(data: data) {
                    switch ev {
                    case .connected:
                        self.connected = true
                        self.backoff = 2
                    case .close, .refused:
                        c.cancel()
                        return
                    default:
                        self.onEvent(ev)
                    }
                }
            }
            if done || error != nil {
                c.cancel()
                return
            }
            self.receive(c)
        }
    }

    private func startPings(_ c: NWConnection) {
        let t = DispatchSource.makeTimerSource(queue: queue)
        let every = Double(Self.keepAlive) / 2
        t.schedule(deadline: .now() + every, repeating: every)
        t.setEventHandler { [weak self] in
            guard let self else { return }
            c.send(content: self.link.ping(), completion: .idempotent)
        }
        t.resume()
        pingTimer = t
    }

    private func close() {
        connected = false
        pingTimer?.cancel()
        pingTimer = nil
        connection?.stateUpdateHandler = nil
        connection?.cancel()
        connection = nil
    }

    private func retry() {
        close()
        guard running else { return }
        let wait = backoff
        backoff = min(backoff * 2, 300)
        queue.asyncAfter(deadline: .now() + wait) { self.open() }
    }
}
