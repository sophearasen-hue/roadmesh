import AudioToolbox
import Flutter
import Foundation
import RoadMeshKit
import UIKit

/// Flutter ↔ drive session, on the same `roadmesh/radio` channels as Android. The UI only
/// shows what the session reports; it decides nothing.
public class RoadmeshIosPlugin: NSObject, FlutterPlugin, FlutterStreamHandler {
    private var sink: FlutterEventSink?

    public static func register(with registrar: FlutterPluginRegistrar) {
        let instance = RoadmeshIosPlugin()
        let methods = FlutterMethodChannel(name: "roadmesh/radio", binaryMessenger: registrar.messenger())
        registrar.addMethodCallDelegate(instance, channel: methods)
        FlutterEventChannel(name: "roadmesh/radio/events", binaryMessenger: registrar.messenger())
            .setStreamHandler(instance)
        // CarPlay content from Flutter; the mute button answers on the same channel
        let carplay = FlutterMethodChannel(name: "roadmesh/carplay", binaryMessenger: registrar.messenger())
        CarPlayHub.shared.channel = carplay
        carplay.setMethodCallHandler { call, result in
            if call.method == "update", let m = call.arguments as? [String: Any], let c = CarPlayContent(m) {
                CarPlayHub.shared.set(c)
            }
            result(nil)
        }
    }

    private var devAnchor: String? {
        #if DEBUG
        return Bundle.main.object(forInfoDictionaryKey: "RoadMeshDevCaAnchor") as? String
        #else
        return nil
        #endif
    }

    public func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
        let args = call.arguments as? [String: Any]
        do {
            switch call.method {
            case "requestPermissions":
                let s = DriveSession.current ?? DriveSession(vehicleType: 1, devAnchorHex: devAnchor)
                DriveSession.current = s
                s.requestPermission()
                result(true) // iOS shows its prompts; Bluetooth asks when the radio starts
            case "start":
                let type = UInt8((args?["vehicleType"] as? Int) ?? 1)
                let s = DriveSession.current ?? DriveSession(vehicleType: type, devAnchorHex: devAnchor)
                DriveSession.current = s
                s.listener = { [weak self] u in
                    let m = Self.update(u)
                    DispatchQueue.main.async { self?.sink?(m) }
                }
                s.start()
                result(nil)
            case "stop":
                DriveSession.current?.stop()
                DriveSession.current = nil
                result(nil)
            case "status":
                result(DriveSession.current?.status() ?? ["running": false])
            case "createCertificateRequest":
                result(try DriveSession.current?.createCertificateRequest())
            case "importCertificates":
                result(try DriveSession.current?.importCertificates())
            case "exportFieldLog":
                result(try DriveSession.current?.exportFieldLog())
            case "alertTone":
                // the engine chose the alert; this only sounds it (system sounds follow the ringer)
                AudioServicesPlaySystemSound((args?["level"] as? String) == "danger" ? 1005 : 1057)
                AudioServicesPlaySystemSound(kSystemSoundID_Vibrate)
                result(nil)
            case "deleteMyData":
                DriveSession.current?.deleteMyData()
                result(nil)
            default:
                result(FlutterMethodNotImplemented)
            }
        } catch {
            result(FlutterError(code: "roadmesh", message: String(describing: type(of: error)), details: nil))
        }
    }

    public func onListen(withArguments arguments: Any?, eventSink events: @escaping FlutterEventSink) -> FlutterError? {
        sink = events
        return nil
    }

    public func onCancel(withArguments arguments: Any?) -> FlutterError? {
        sink = nil
        return nil
    }

    private static func alert(_ a: AlertInfo) -> [String: Any] {
        let level: String
        switch a.level {
        case .caution: level = "caution"
        case .warning: level = "warning"
        case .danger: level = "danger"
        }
        // missing values travel as NSNull through the Flutter codec
        return ["kind": a.kind, "level": level, "advice": a.advice, "advisedDecel": a.advisedDecelMps2,
                "distance": a.distanceM.map { $0 as Any } ?? NSNull(),
                "ttc": a.ttcS.map { $0 as Any } ?? NSNull(), "hops": Int(a.hops),
                "target": a.target.map { Crypto.hex($0) as Any } ?? NSNull()]
    }

    private static func counts(_ c: EventCounts) -> [String: Any] {
        ["speeding": Int(c.speeding), "hacc": Int(c.harshAccels), "hbrake": Int(c.harshBrakes),
         "overtake": Int(c.overtakes)]
    }

    private static func update(_ u: DriveUpdate) -> [String: Any] {
        [
            "alerts": u.decision.alerts.map(alert),
            "peers": u.peers.map(peer),
            "own": ["speed": u.fix.speedMps, "heading": u.fix.headingDeg, "accuracy": u.fix.accuracyM,
                    "limit": u.road.speedLimitMps.map { $0 as Any } ?? NSNull()],
            "trip": ["distance": u.trip.distanceM, "avgSpeed": u.trip.avgSpeedMps,
                     "maxSpeed": u.trip.maxSpeedMps, "score": Int(u.trip.score),
                     "counts": counts(u.trip.counts)],
            "events": u.events.map {
                ["age": $0.ageS, "kind": $0.kind, "me": $0.me, "id": Crypto.hex($0.pseudonym), "kmh": $0.kmh]
            },
        ]
    }

    private static func peer(_ p: PeerInfo) -> [String: Any] {
        ["id": Crypto.hex(p.pseudonym), "east": p.eastM, "north": p.northM, "distance": p.distanceM,
         "speed": p.speedMps, "accel": p.accelMps2, "heading": p.headingDeg, "vehicleType": Int(p.vehicleType),
         "hops": Int(p.hops), "accuracy": p.accuracyM, "ageMs": Int(p.ageMs), "score": Int(p.score),
         "counts": counts(p.counts)]
    }
}
