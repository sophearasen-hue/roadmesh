import Foundation
import RoadMeshKit

/// The edge server (docs/SERVER.md), fixed at build time through Info.plist keys set from
/// `Flutter/Server.xcconfig`: `RoadMeshServerHost`, `RoadMeshServerPins` (base64 SPKI
/// SHA-256 pins, current + backup) and `RoadMeshCaAnchor`. Without them the bridge is off.
struct ServerConfig {
    let host: String
    let pins: [Data]
    let mqttPort: UInt16 = 8883
    let enrolPort: UInt16 = 8884

    static func fromBundle() -> ServerConfig? {
        guard let host = Bundle.main.object(forInfoDictionaryKey: "RoadMeshServerHost") as? String,
              !host.isEmpty, !host.hasPrefix("$("),
              let pinText = Bundle.main.object(forInfoDictionaryKey: "RoadMeshServerPins") as? String,
              let pins = Pinning.parse(pinText) else { return nil }
        return ServerConfig(host: host, pins: pins)
    }

    /// The production pseudonym CA trust anchor (34 bytes), if configured.
    static func caAnchor() -> Data? {
        guard let hex = Bundle.main.object(forInfoDictionaryKey: "RoadMeshCaAnchor") as? String,
              let a = Crypto.unhex(hex), a.count == 34 else { return nil }
        return a
    }
}
