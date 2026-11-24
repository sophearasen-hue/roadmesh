import Foundation

/// What the CarPlay Driving Task screen shows (SPEC §9), built in Flutter with the
/// English/Khmer strings and sent over the `roadmesh/carplay` channel. Parsed here so the
/// CarPlay scene only draws it.
public struct CarPlayContent: Equatable {
    public struct Row: Equatable {
        public let text: String
        public let detail: String
        /// "safe", "caution" or "risky".
        public let band: String
    }

    public let title: String
    public let alert: String
    /// "ok", "warning" or "danger".
    public let alertLevel: String
    public let speed: String
    public let overLimit: Bool
    public let nearestTitle: String
    public let rows: [Row]
    public let muted: Bool
    public let muteLabel: String
    public let mutedNote: String

    /// Nearest vehicles shown (SPEC §9: 3 to 5).
    public static let maxRows = 5

    /// Parse the channel's map; nil if a field is missing or has the wrong type.
    public init?(_ m: [String: Any]) {
        guard let title = m["title"] as? String, let alert = m["alert"] as? String,
              let level = m["alertLevel"] as? String, let speed = m["speed"] as? String,
              let over = m["overLimit"] as? Bool, let nearest = m["nearestTitle"] as? String,
              let rows = m["rows"] as? [[String: Any]], let muted = m["muted"] as? Bool,
              let muteLabel = m["muteLabel"] as? String, let note = m["mutedNote"] as? String else { return nil }
        self.title = title
        self.alert = alert
        alertLevel = ["ok", "warning", "danger"].contains(level) ? level : "ok"
        self.speed = speed
        overLimit = over
        nearestTitle = nearest
        self.rows = rows.prefix(Self.maxRows).compactMap { r in
            guard let t = r["text"] as? String, let d = r["detail"] as? String, let b = r["band"] as? String else {
                return nil
            }
            return Row(text: t, detail: d, band: b)
        }
        self.muted = muted
        self.muteLabel = muteLabel
        mutedNote = note
    }
}
