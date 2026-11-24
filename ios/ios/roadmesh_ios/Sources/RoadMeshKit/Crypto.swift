import Foundation

/// Encoding helpers between Apple's crypto formats and the core's (SPEC §10).
public enum Crypto {
    /// ECDSA signature: ASN.1 DER `SEQUENCE { r INTEGER, s INTEGER }` → raw r‖s (64 bytes).
    public static func derToRaw(_ der: Data) -> Data? {
        let b = [UInt8](der)
        var i = 0
        func byte() -> Int? {
            guard i < b.count else { return nil }
            defer { i += 1 }
            return Int(b[i])
        }
        func length() -> Int? {
            guard let first = byte() else { return nil }
            if first < 0x80 { return first }
            let n = first & 0x7f
            guard n == 1 || n == 2 else { return nil }
            var len = 0
            for _ in 0..<n {
                guard let x = byte() else { return nil }
                len = (len << 8) | x
            }
            return len
        }
        func integer() -> [UInt8]? {
            guard byte() == 0x02, let len = length(), len > 0, len <= 33, i + len <= b.count else { return nil }
            var v = Array(b[i..<(i + len)])
            i += len
            while v.first == 0 { v.removeFirst() }
            guard v.count <= 32 else { return nil }
            return [UInt8](repeating: 0, count: 32 - v.count) + v
        }
        guard byte() == 0x30, let total = length(), total == b.count - i,
              let r = integer(), let s = integer(), i == b.count else { return nil }
        return Data(r + s)
    }

    /// Uncompressed SEC1 point (0x04 ‖ X ‖ Y, 65 bytes) → compressed (33 bytes).
    public static func compress(uncompressed p: Data) -> Data? {
        let b = [UInt8](p)
        guard b.count == 65, b[0] == 0x04 else { return nil }
        let prefix: UInt8 = (b[64] & 1) == 1 ? 0x03 : 0x02
        return Data([prefix] + b[1...32])
    }

    /// Lowercase hex, for storage keys (never log key material or pseudonyms).
    public static func hex(_ d: Data) -> String {
        d.map { String(format: "%02x", $0) }.joined()
    }

    /// Parse hex; nil if malformed.
    public static func unhex(_ s: String) -> Data? {
        guard s.count % 2 == 0 else { return nil }
        var out = Data(capacity: s.count / 2)
        var idx = s.startIndex
        while idx < s.endIndex {
            let next = s.index(idx, offsetBy: 2)
            guard let v = UInt8(s[idx..<next], radix: 16) else { return nil }
            out.append(v)
            idx = next
        }
        return out
    }
}
