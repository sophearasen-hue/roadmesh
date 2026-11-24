//! Device certificates for mutual TLS (AUDIT S-16): each install's hardware device key
//! gets an X.509 client certificate from the device CA at enrolment. The device CA key
//! is reached only through [`CaKey`] (AWS KMS in production, AUDIT S-19), so it never
//! sits on the server. The certificate names no person or phone: the subject is the
//! same for every device; only the key differs.

use p256::ecdsa::{Signature, VerifyingKey};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyUsagePurpose, PKCS_ECDSA_P256_SHA256, PublicKeyData, SerialNumber,
    SignatureAlgorithm, SigningKey,
};
use roadmesh_core::pki::PUBLIC_KEY_LEN;
use time::OffsetDateTime;

use crate::ca::{CaKey, device_id};

/// Device certificates last a year; the app re-enrols before expiry.
pub const DEVICE_CERT_DAYS: i64 = 365;

/// Why a device certificate could not be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevCaError {
    /// The device key is not a valid P-256 point.
    BadKey,
    /// The CA key could not sign (KMS unavailable).
    Unavailable,
}

struct Uncompressed(Vec<u8>);

impl PublicKeyData for Uncompressed {
    fn der_bytes(&self) -> &[u8] {
        &self.0
    }
    fn algorithm(&self) -> &'static SignatureAlgorithm {
        &PKCS_ECDSA_P256_SHA256
    }
}

fn uncompressed(sec1: &[u8; PUBLIC_KEY_LEN]) -> Result<Vec<u8>, DevCaError> {
    let k = VerifyingKey::from_sec1_bytes(sec1).map_err(|_| DevCaError::BadKey)?;
    Ok(k.to_sec1_point(false).as_bytes().to_vec())
}

/// rcgen signs through the CA key; the KMS returns r‖s, X.509 wants DER.
struct Signer<'a, K> {
    key: &'a K,
    public: Vec<u8>,
}

impl<K: CaKey> PublicKeyData for Signer<'_, K> {
    fn der_bytes(&self) -> &[u8] {
        &self.public
    }
    fn algorithm(&self) -> &'static SignatureAlgorithm {
        &PKCS_ECDSA_P256_SHA256
    }
}

impl<K: CaKey> SigningKey for Signer<'_, K> {
    fn sign(&self, msg: &[u8]) -> Result<Vec<u8>, rcgen::Error> {
        let raw = self
            .key
            .sign(msg)
            .map_err(|_| rcgen::Error::RemoteKeyError)?;
        let sig = Signature::from_slice(&raw).map_err(|_| rcgen::Error::RemoteKeyError)?;
        Ok(sig.to_der().as_bytes().to_vec())
    }
}

fn name(cn: &str) -> DistinguishedName {
    let mut dn = DistinguishedName::new();
    dn.push(DnType::OrganizationName, "RoadMesh");
    dn.push(DnType::CommonName, cn);
    dn
}

fn ca_params() -> CertificateParams {
    let mut p = CertificateParams::default();
    p.distinguished_name = name("RoadMesh Device CA");
    p.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    p.not_before = rcgen::date_time_ymd(2026, 1, 1);
    p.not_after = rcgen::date_time_ymd(2046, 1, 1);
    p.serial_number = Some(SerialNumber::from(vec![1]));
    p
}

/// The device CA.
pub struct DeviceCa<K> {
    key: K,
    public: Vec<u8>,
    ca_der: Vec<u8>,
}

impl<K: CaKey> DeviceCa<K> {
    /// The CA with its self-signed root certificate (made from `key`; the same key
    /// always gives an equivalent root, so servers can restart without re-pinning).
    pub fn new(key: K) -> Result<Self, DevCaError> {
        let public = uncompressed(&key.public_key())?;
        let signer = Signer {
            key: &key,
            public: public.clone(),
        };
        let ca_der = ca_params()
            .self_signed(&signer)
            .map_err(|_| DevCaError::Unavailable)?
            .der()
            .to_vec();
        Ok(Self {
            key,
            public,
            ca_der,
        })
    }

    /// The root certificate (DER): the server's client-certificate trust anchor.
    pub fn root_der(&self) -> &[u8] {
        &self.ca_der
    }

    /// A client certificate for `device_key` (SEC1 compressed), valid from a minute
    /// before `now` (Unix s) for [`DEVICE_CERT_DAYS`].
    pub fn issue(
        &self,
        device_key: &[u8; PUBLIC_KEY_LEN],
        now: i64,
    ) -> Result<Vec<u8>, DevCaError> {
        let subject = Uncompressed(uncompressed(device_key)?);
        let mut p = CertificateParams::default();
        p.distinguished_name = name("RoadMesh device");
        p.is_ca = IsCa::ExplicitNoCa;
        p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        p.use_authority_key_identifier_extension = true;
        let from = OffsetDateTime::from_unix_timestamp(now - 60).map_err(|_| DevCaError::BadKey)?;
        p.not_before = from;
        p.not_after = from + time::Duration::days(DEVICE_CERT_DAYS);
        // serial: from the device id, positive, so a re-issue keeps the same serial
        let mut serial = device_id(device_key)[..16].to_vec();
        serial[0] &= 0x7f;
        serial[0] |= 0x01;
        p.serial_number = Some(SerialNumber::from(serial));
        let signer = Signer {
            key: &self.key,
            public: self.public.clone(),
        };
        let issuer = Issuer::new(ca_params(), signer);
        Ok(p.signed_by(&subject, &issuer)
            .map_err(|_| DevCaError::Unavailable)?
            .der()
            .to_vec())
    }
}

/// The device key (SEC1 compressed) in a client certificate (DER), if it is P-256.
pub fn device_key_of(cert_der: &[u8]) -> Option<[u8; PUBLIC_KEY_LEN]> {
    let (_, c) = x509_parser::parse_x509_certificate(cert_der).ok()?;
    let point = c.public_key().subject_public_key.data.as_ref();
    let k = VerifyingKey::from_sec1_bytes(point).ok()?;
    let mut out = [0u8; PUBLIC_KEY_LEN];
    out.copy_from_slice(k.to_sec1_point(true).as_bytes());
    Some(out)
}
