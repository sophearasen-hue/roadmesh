import Foundation
import RoadMeshKit
import Security

/// Device and pseudonym keys in the Secure Enclave (AUDIT S-02): generated in hardware,
/// never exportable. The Rust core asks for signatures through `HardwareKeys`; it never
/// sees a private key. The simulator has no Secure Enclave, so it uses ordinary keychain
/// keys (debug only).
final class KeychainKeys: HardwareKeys, @unchecked Sendable {
    static let deviceTag = "com.roadmesh.device"
    static let pseudonymPrefix = "com.roadmesh.pseudo."
    private let defaults = UserDefaults(suiteName: "roadmesh.keys") ?? .standard

    /// Generate (or reuse) a P-256 key under `tag`; returns its compressed public key.
    func generate(tag: String) throws -> Data {
        let key = try find(tag: tag) ?? create(tag: tag)
        guard let pub = SecKeyCopyPublicKey(key),
              let raw = SecKeyCopyExternalRepresentation(pub, nil) as Data?,
              let compressed = Crypto.compress(uncompressed: raw) else {
            throw KeyError.publicKey
        }
        return compressed
    }

    private func create(tag: String) throws -> SecKey {
        var privateAttrs: [String: Any] = [
            kSecAttrIsPermanent as String: true,
            kSecAttrApplicationTag as String: Data(tag.utf8),
        ]
        var attrs: [String: Any] = [
            kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
            kSecAttrKeySizeInBits as String: 256,
        ]
        #if !targetEnvironment(simulator)
        guard let access = SecAccessControlCreateWithFlags(
            nil, kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly, .privateKeyUsage, nil) else {
            throw KeyError.create
        }
        privateAttrs[kSecAttrAccessControl as String] = access
        attrs[kSecAttrTokenID as String] = kSecAttrTokenIDSecureEnclave
        #endif
        attrs[kSecPrivateKeyAttrs as String] = privateAttrs
        var error: Unmanaged<CFError>?
        guard let key = SecKeyCreateRandomKey(attrs as CFDictionary, &error) else { throw KeyError.create }
        return key
    }

    private func find(tag: String) throws -> SecKey? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassKey,
            kSecAttrApplicationTag as String: Data(tag.utf8),
            kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
            kSecReturnRef as String: true,
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard status == errSecSuccess, let found = item else { return nil }
        return (found as! SecKey) // swiftlint:disable:this force_cast (CFTypeRef of kSecClassKey)
    }

    /// Raw r‖s signature over `message` with the key under `tag`, or nil.
    func sign(tag: String, message: Data) -> Data? {
        guard let key = try? find(tag: tag),
              let der = SecKeyCreateSignature(key, .ecdsaSignatureMessageX962SHA256, message as CFData, nil) as Data?
        else { return nil }
        return Crypto.derToRaw(der)
    }

    /// Remember which key belongs to which certificate (digest hex → tag).
    func bind(certDigest: Data, tag: String) {
        defaults.set(tag, forKey: "cert_" + Crypto.hex(certDigest))
    }

    func sign(certDigest: Data, message: Data) -> Data {
        guard let tag = defaults.string(forKey: "cert_" + Crypto.hex(certDigest)) else { return Data() }
        return sign(tag: tag, message: message) ?? Data()
    }

    enum KeyError: Error { case create, publicKey, sign }
}
