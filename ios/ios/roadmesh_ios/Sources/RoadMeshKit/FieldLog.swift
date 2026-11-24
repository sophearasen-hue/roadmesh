import Foundation

/// Field-test logger (BUILD_PLAN Step 6, same format as Android): per minute and distance
/// band, how many of the messages a heard phone sent (10 per second) arrived, the median
/// signal strength, and the battery level. Never records coordinates or pseudonyms
/// (CLAUDE.md, AUDIT S-15): pseudonyms are only in-memory counter keys, dropped each second.
public final class FieldLog {
    public static let bands: [(String, Double)] = [
        ("0-50", 50), ("50-100", 100), ("100-150", 150), ("150-250", 250), ("250+", .greatestFiniteMagnitude),
    ]

    private struct BandStats { var received = 0; var expected = 0; var rssi: [Int] = [] }
    private let rate: Int
    private var stats: [String: BandStats] = [:]
    private var thisSecond: [String: [Int]] = [:]
    private var rows: [String] = []
    private var minute = 0
    private let lock = NSLock()

    public init(ratePerSecond: Int = 10) {
        rate = ratePerSecond
        reset()
    }

    private func reset() { stats = Dictionary(uniqueKeysWithValues: Self.bands.map { ($0.0, BandStats()) }) }

    private func band(_ d: Double) -> String { Self.bands.first { d < $0.1 }?.0 ?? "250+" }

    /// A frame from `pseudonymHex` was accepted with signal strength `rssi` (dBm).
    public func onAccepted(pseudonymHex: String, rssi: Int) {
        lock.lock(); defer { lock.unlock() }
        thisSecond[pseudonymHex, default: []].append(rssi)
    }

    /// Once a second: the peers heard (pseudonym hex → distance m).
    public func everySecond(_ peers: [String: Double]) {
        lock.lock(); defer { lock.unlock() }
        for (id, d) in peers {
            let key = band(d)
            var b = stats[key] ?? BandStats()
            let got = thisSecond[id] ?? []
            b.expected += rate
            b.received += min(got.count, rate)
            b.rssi += got
            stats[key] = b
        }
        thisSecond.removeAll()
    }

    /// Once a minute: close the minute with the battery level (%).
    public func everyMinute(batteryPct: Int) {
        lock.lock(); defer { lock.unlock() }
        var any = false
        for (label, _) in Self.bands {
            guard let s = stats[label], s.expected > 0 else { continue }
            any = true
            let pct = 100.0 * Double(s.received) / Double(s.expected)
            let sorted = s.rssi.sorted()
            let med = sorted.isEmpty ? "" : String(sorted[sorted.count / 2])
            rows.append("\(minute),\(batteryPct),\(label),\(s.received),\(s.expected),\(String(format: "%.1f", pct)),\(med)")
        }
        if !any { rows.append("\(minute),\(batteryPct),none,0,0,,") }
        reset()
        minute += 1
    }

    /// The log as CSV.
    public func csv() -> String {
        lock.lock(); defer { lock.unlock() }
        return (["minute,battery_pct,distance_band_m,received,expected,delivery_pct,rssi_median_dbm"] + rows)
            .joined(separator: "\n") + "\n"
    }
}
