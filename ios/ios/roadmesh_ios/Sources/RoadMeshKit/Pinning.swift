import CryptoKit
import Foundation

/// Certificate pinning for the edge server (AUDIT S-17): the SHA-256 of the server
/// key's SubjectPublicKeyInfo must be one of the pins (the current key and a backup).
/// The server key is EC P-256 (docs/SERVER.md).
public enum Pinning {
    /// DER prefix of a P-256 SubjectPublicKeyInfo, before the 65-byte uncompressed point.
    static let p256SpkiPrefix = Data([
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
        0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
    ])

    /// SubjectPublicKeyInfo of a P-256 key from its uncompressed point (as
    /// `SecKeyCopyExternalRepresentation` returns it), or nil for other keys.
    public static func spki(p256Point point: Data) -> Data? {
        guard point.count == 65, point.first == 0x04 else { return nil }
        return p256SpkiPrefix + point
    }

    /// SHA-256 of `data`.
    public static func sha256(_ data: Data) -> Data { Data(SHA256.hash(data: data)) }

    /// True if the key's SPKI hash is one of `pins`.
    public static func matches(p256Point point: Data, pins: [Data]) -> Bool {
        guard let s = spki(p256Point: point) else { return false }
        let h = sha256(s)
        return pins.contains(h)
    }

    /// Parse "base64,base64,…": nil unless there are at least two 32-byte pins.
    public static func parse(_ s: String) -> [Data]? {
        let pins = s.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
        let decoded = pins.compactMap { Data(base64Encoded: $0) }
        guard decoded.count == pins.count, decoded.count >= 2, decoded.allSatisfy({ $0.count == 32 }) else {
            return nil
        }
        return decoded
    }
}
