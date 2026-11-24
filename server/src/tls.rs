//! TLS for the edge server (AUDIT S-16, S-17): TLS 1.3 only (rustls is built without
//! TLS 1.2), ALPN "mqtt". The MQTT port demands a client certificate from the device
//! CA (mutual TLS); the enrolment port authenticates phones by App Attest / Play
//! Integrity instead, because they have no device certificate yet.

use std::sync::Arc;

use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig};
use rustls_pki_types::{CertificateDer, PrivateKeyDer};

/// ALPN protocol name.
pub const ALPN: &[u8] = b"mqtt";

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// The MQTT listener: TLS 1.3 + a client certificate issued by `device_root`.
pub fn mqtt_config(
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
    device_root: &[u8],
) -> Result<Arc<ServerConfig>, rustls::Error> {
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(device_root.to_vec()))?;
    let verifier = WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider())
        .build()
        .map_err(|e| rustls::Error::General(format!("client verifier: {e}")))?;
    let mut c = ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(verifier)
        .with_single_cert(chain, key)?;
    c.alpn_protocols = vec![ALPN.to_vec()];
    Ok(Arc::new(c))
}

/// The enrolment listener: TLS 1.3, no client certificate.
pub fn enrol_config(
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> Result<Arc<ServerConfig>, rustls::Error> {
    let mut c = ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_no_client_auth()
        .with_single_cert(chain, key)?;
    c.alpn_protocols = vec![ALPN.to_vec()];
    Ok(Arc::new(c))
}

/// SHA-256 of a certificate's SubjectPublicKeyInfo, the value phones pin (base64 in
/// the app config, like HPKP). `None` if the certificate does not parse.
pub fn spki_sha256(cert_der: &[u8]) -> Option<[u8; 32]> {
    use sha2::Digest;
    let (_, c) = x509_parser::parse_x509_certificate(cert_der).ok()?;
    Some(sha2::Sha256::digest(c.tbs_certificate.subject_pki.raw).into())
}
