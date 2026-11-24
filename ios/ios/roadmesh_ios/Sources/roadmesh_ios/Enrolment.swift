import CryptoKit
import DeviceCheck
import Foundation
import Network
import RoadMeshKit

/// Pseudonym certificates for this phone (SPEC §10): 20 Secure Enclave keys a week, one
/// per certificate, requested with the device key. Production: `enrolOnline` sends the
/// request with App Attest evidence to the edge server's enrolment port and receives the
/// device certificate (for mutual TLS) and the pseudonym certificates. Debug builds can
/// also exchange files with the developer CA (`tools/devca`) through Xcode.
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
        Self.excludeFromBackup(certificateFile)
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

    /// AUDIT S-14/S-20: certificates stay on this phone (no iCloud backup).
    static func excludeFromBackup(_ url: URL) {
        var u = url
        var v = URLResourceValues()
        v.isExcludedFromBackup = true
        try? u.setResourceValues(v)
    }

    /// Previously imported certificates, if any.
    func storedCertificates() -> Data? { try? Data(contentsOf: certificateFile) }

    enum OnlineError: Error { case appAttestUnsupported, server, closed }

    /// Enrol with the edge server: new pseudonym keys, App Attest bound to the request,
    /// pinned TLS 1.3. Stores the device certificate and returns the certificate batch.
    func enrolOnline(_ config: ServerConfig, count: Int = 20) async throws -> Data {
        let now = UInt32(Date().timeIntervalSince1970)
        let device = try keys.generate(tag: KeychainKeys.deviceTag)
        let pubs = try (0..<count).map { try keys.generate(tag: "\(KeychainKeys.pseudonymPrefix)\(now).\($0)") }
        let tbs = try issueRequestToSign(deviceKey: device, pseudonymKeys: pubs, requestedAt: now)
        guard let sig = keys.sign(tag: KeychainKeys.deviceTag, message: tbs) else { throw KeychainKeys.KeyError.sign }
        defaults.set(Int(now), forKey: "batch")
        defaults.set(count, forKey: "count")
        var evidence = try await appAttestEvidence(challenge: tbs, fresh: false)
        var reply = try await exchange(config, request: try encodeIssueRequest(
            deviceKey: device, pseudonymKeys: pubs, requestedAt: now, attestation: evidence, signature: sig))
        if reply.first == 0x82 { // the server no longer knows our App Attest key: attest a new one
            evidence = try await appAttestEvidence(challenge: tbs, fresh: true)
            reply = try await exchange(config, request: try encodeIssueRequest(
                deviceKey: device, pseudonymKeys: pubs, requestedAt: now, attestation: evidence, signature: sig))
        }
        let enrolled = try parseEnrolReply(reply: reply)
        try keys.storeDeviceCertificate(enrolled.deviceCertificate)
        try enrolled.certificates.write(to: certificateFile, options: .completeFileProtectionUntilFirstUserAuthentication)
        Self.excludeFromBackup(certificateFile)
        return try importCertificates()
    }

    /// App Attest (server `attest::kind`): an attestation the first time a key is used
    /// (kind 1), assertions afterwards (kind 2). clientDataHash = SHA-256(request).
    private func appAttestEvidence(challenge: Data, fresh: Bool) async throws -> Data {
        let service = DCAppAttestService.shared
        guard service.isSupported else { throw OnlineError.appAttestUnsupported }
        let hash = Data(SHA256.hash(data: challenge))
        if !fresh, let keyId = defaults.string(forKey: "appAttestKey"), defaults.bool(forKey: "appAttested"),
           let raw = Data(base64Encoded: keyId) {
            let assertion = try await service.generateAssertion(keyId, clientDataHash: hash)
            return Data([2]) + raw + assertion
        }
        let keyId = try await service.generateKey()
        guard let raw = Data(base64Encoded: keyId) else { throw OnlineError.appAttestUnsupported }
        let attestation = try await service.attestKey(keyId, clientDataHash: hash)
        defaults.set(keyId, forKey: "appAttestKey")
        defaults.set(true, forKey: "appAttested")
        return Data([1]) + raw + attestation
    }

    /// One request on the enrolment port; returns the reply bytes.
    private func exchange(_ config: ServerConfig, request: Data) async throws -> Data {
        let queue = DispatchQueue(label: "roadmesh.enrol")
        let link = CellularLink()
        let c = PinnedConnection.make(config, port: config.enrolPort, identity: nil, queue: queue)
        return try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Data, Error>) in
            var finished = false
            func finish(_ r: Result<Data, Error>) {
                guard !finished else { return }
                finished = true
                c.cancel()
                cont.resume(with: r)
            }
            func receive() {
                c.receive(minimumIncompleteLength: 1, maximumLength: 8192) { data, _, done, error in
                    if let data {
                        for ev in link.onBytes(data: data) {
                            switch ev {
                            case .enrolReply(let reply): return finish(.success(reply))
                            case .close, .refused: return finish(.failure(OnlineError.server))
                            default: break
                            }
                        }
                    }
                    if done || error != nil { return finish(.failure(OnlineError.closed)) }
                    receive()
                }
            }
            c.stateUpdateHandler = { state in
                switch state {
                case .ready:
                    c.send(content: link.connect(keepAliveS: 30) + link.enrol(request: request), completion: .idempotent)
                    receive()
                case .failed, .cancelled:
                    finish(.failure(OnlineError.closed))
                default:
                    break
                }
            }
            c.start(queue: queue)
            queue.asyncAfter(deadline: .now() + 30) { finish(.failure(OnlineError.closed)) }
        }
    }
}
