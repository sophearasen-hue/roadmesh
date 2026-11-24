import CarPlay
import Flutter
import RoadMeshKit
import UIKit

/// Holds the latest CarPlay content from Flutter and the mute button's way back.
final class CarPlayHub {
    static let shared = CarPlayHub()
    private(set) var content: CarPlayContent?
    private var observers: [(CarPlayContent) -> Void] = []
    var channel: FlutterMethodChannel?

    func set(_ c: CarPlayContent) {
        guard c != content else { return }
        content = c
        observers.forEach { $0(c) }
    }

    func observe(_ f: @escaping (CarPlayContent) -> Void) {
        observers = [f] // one CarPlay screen at a time
        if let c = content { f(c) }
    }

    func stopObserving() { observers = [] }

    /// The driver tapped Mute/Unmute in the car: Flutter stores it and sends new content.
    func toggleMute() {
        channel?.invokeMethod("mute", arguments: !(content?.muted ?? false))
    }
}

/// The CarPlay Driving Task scene (SPEC §9, BUILD_PLAN Step 10): one list template with
/// the current alert, own speed against the limit, the nearest vehicles and a mute
/// button. The full map stays on the phone (Apple's rules for this category).
@objc(RoadMeshCarPlaySceneDelegate)
final class CarPlaySceneDelegate: UIResponder, CPTemplateApplicationSceneDelegate {
    private var interface: CPInterfaceController?
    private var list: CPListTemplate?

    func templateApplicationScene(_ scene: CPTemplateApplicationScene,
                                  didConnect interfaceController: CPInterfaceController) {
        interface = interfaceController
        let t = CPListTemplate(title: "RoadMesh", sections: [])
        list = t
        interfaceController.setRootTemplate(t, animated: false, completion: nil)
        CarPlayHub.shared.observe { [weak self] c in
            DispatchQueue.main.async { self?.show(c) }
        }
    }

    func templateApplicationScene(_ scene: CPTemplateApplicationScene,
                                  didDisconnectInterfaceController interfaceController: CPInterfaceController) {
        CarPlayHub.shared.stopObserving()
        interface = nil
        list = nil
    }

    private func show(_ c: CarPlayContent) {
        guard let t = list else { return }
        let alertColor: UIColor = c.alertLevel == "danger" ? .systemRed : c.alertLevel == "warning" ? .systemOrange : .systemGreen
        let alert = CPListItem(text: c.alert, detailText: c.muted ? c.mutedNote : nil, image: Self.dot(alertColor))
        let speed = CPListItem(text: c.speed, detailText: nil, image: Self.dot(c.overLimit ? .systemRed : .systemBlue))
        let rows = c.rows.map { r -> CPListItem in
            let color: UIColor = r.band == "risky" ? .systemRed : r.band == "caution" ? .systemOrange : .systemGreen
            return CPListItem(text: r.text, detailText: r.detail, image: Self.dot(color))
        }
        t.updateSections([
            CPListSection(items: [alert, speed]),
            CPListSection(items: rows, header: c.nearestTitle, sectionIndexTitle: nil),
        ])
        t.trailingNavigationBarButtons = [CPBarButton(title: c.muteLabel) { _ in CarPlayHub.shared.toggleMute() }]
    }

    private static func dot(_ color: UIColor) -> UIImage {
        UIGraphicsImageRenderer(size: CGSize(width: 24, height: 24)).image { ctx in
            color.setFill()
            ctx.cgContext.fillEllipse(in: CGRect(x: 4, y: 4, width: 16, height: 16))
        }
    }
}
