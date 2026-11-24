import Foundation
import Network
import RoadMeshKit
import Security

/// TLS 1.3 to the edge server (AUDIT S-16, S-17): system certificate validation and
/// host-name check, then the SPKI pin; optionally the device identity (Secure Enclave
/// device key + device certificate) for mutual TLS.
enum PinnedConnection {
    static func make(_ config: ServerConfig, port: UInt16, identity: SecIdentity?, queue: DispatchQueue) -> NWConnection {
        let tls = NWProtocolTLS.Options()
        let sec = tls.securityProtocolOptions
        sec_protocol_options_set_min_tls_protocol_version(sec, .TLSv13)
        sec_protocol_options_set_max_tls_protocol_version(sec, .TLSv13)
        sec_protocol_options_add_tls_application_protocol(sec, "mqtt")
        sec_protocol_options_set_tls_server_name(sec, config.host)
        if let identity, let id = sec_identity_create(identity) {
            sec_protocol_options_set_local_identity(sec, id)
        }
        let host = config.host
        let pins = config.pins
        sec_protocol_options_set_verify_block(sec, { _, trustRef, complete in
            let trust = sec_trust_copy_ref(trustRef).takeRetainedValue()
            SecTrustSetPolicies(trust, SecPolicyCreateSSL(true, host as CFString))
            var error: CFError?
            guard SecTrustEvaluateWithError(trust, &error),
                  let chain = SecTrustCopyCertificateChain(trust) as? [SecCertificate],
                  let leaf = chain.first,
                  let key = SecCertificateCopyKey(leaf),
                  let point = SecKeyCopyExternalRepresentation(key, nil) as Data? else {
                complete(false)
                return
            }
            complete(Pinning.matches(p256Point: point, pins: pins))
        }, queue)
        let params = NWParameters(tls: tls, tcp: NWProtocolTCP.Options())
        return NWConnection(host: NWEndpoint.Host(config.host),
                            port: NWEndpoint.Port(rawValue: port) ?? 8883, using: params)
    }
}
