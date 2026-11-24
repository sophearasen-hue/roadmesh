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
                s.listener = { [weak self] d, peers in
                    DispatchQueue.main.async {
                        self?.sink?(["alerts": d.alerts.map(Self.alert), "peers": peers.map(Self.peer)])
                    }
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
                "ttc": a.ttcS.map { $0 as Any } ?? NSNull(), "hops": Int(a.hops)]
    }

    private static func peer(_ p: PeerInfo) -> [String: Any] {
        ["id": Crypto.hex(p.pseudonym), "east": p.eastM, "north": p.northM, "distance": p.distanceM,
         "speed": p.speedMps, "heading": p.headingDeg, "vehicleType": Int(p.vehicleType),
         "hops": Int(p.hops), "accuracy": p.accuracyM]
    }
}
