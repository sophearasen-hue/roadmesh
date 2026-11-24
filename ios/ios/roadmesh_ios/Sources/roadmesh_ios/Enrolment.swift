import Foundation
import RoadMeshKit

/// Pseudonym certificates for this phone (SPEC §10): 20 Secure Enclave keys a week, one
/// per certificate, requested with the device key. Until the server exists (Step 9), debug
/// builds exchange files with the developer CA (`tools/devca`) through Xcode's
/// Devices window ("Download / Replace Container").
final class Enrolment {
    private let keys: KeychainKeys
    private let defaults = UserDefaults(suiteName: "roadmesh.enrolment") ?? .standard

    init(keys: KeychainKeys) { self.keys = keys }

    private var dir: URL { FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0] }
    var requestFile: URL { dir.appendingPathComponent("roadmesh-request.bin") }
    var certificateFile: URL { dir.appendingPathComponent("roadmesh-certificates.bin") }

    /// Generate new pseudonym keys and write a signed certificate request.
    func createRequest(count: Int = 20) throws -> URL {
        let now = UInt32(Date().timeIntervalSince1970)
        let device = try keys.generate(tag: KeychainKeys.deviceTag)
        let pubs = try (0..<count).map { try keys.generate(tag: "\(KeychainKeys.pseudonymPrefix)\(now).\($0)") }
        let tbs = try issueRequestToSign(deviceKey: device, pseudonymKeys: pubs, requestedAt: now)
        guard let sig = keys.sign(tag: KeychainKeys.deviceTag, message: tbs) else { throw KeychainKeys.KeyError.sign }
        // App Attest evidence arrives in Step 9; the developer CA accepts none
        let wire = try encodeIssueRequest(deviceKey: device, pseudonymKeys: pubs, requestedAt: now,
                                          attestation: Data(), signature: sig)
        try wire.write(to: requestFile)
        defaults.set(Int(now), forKey: "batch")
        defaults.set(count, forKey: "count")
        return requestFile
    }

    /// Bind each returned certificate to its Secure Enclave key; returns the batch.
    func importCertificates() throws -> Data {
        let bytes = try Data(contentsOf: certificateFile)
        let batch = defaults.integer(forKey: "batch")
        let count = defaults.integer(forKey: "count")
        var byKey: [String: String] = [:]
        for i in 0..<count {
            let tag = "\(KeychainKeys.pseudonymPrefix)\(batch).\(i)"
            byKey[Crypto.hex(try keys.generate(tag: tag))] = tag
        }
        for (d, p) in zip(try certificateDigests(batch: bytes), try certificatePublicKeys(batch: bytes)) {
            guard let tag = byKey[Crypto.hex(p)] else { throw KeychainKeys.KeyError.publicKey }
            keys.bind(certDigest: d, tag: tag)
        }
        return bytes
    }

    /// Previously imported certificates, if any.
    func storedCertificates() -> Data? { try? Data(contentsOf: certificateFile) }
}
