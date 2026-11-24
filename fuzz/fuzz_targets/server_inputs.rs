//! Edge server inputs from untrusted phones (Step 9): attestation evidence (CBOR,
//! X.509, JWE/JWS, JSON), uploaded frames through the hazard gate, and device
//! certificates. Never panics; random bytes never pass.
#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use p256::ecdsa::SigningKey;
use p256::elliptic_curve::Generate;
use roadmesh_core::config::Config;
use roadmesh_core::pki::{PUBLIC_KEY_LEN, SIG_LEN};
use roadmesh_server::attest::{APPLE_ROOT_PEM, AppleConfig, DeviceAttestation, PlayConfig, pem_to_der};
use roadmesh_server::ca::{Attestation, CaError, CaKey, CertAuthority, RejectAll};
use roadmesh_server::devca::device_key_of;
use roadmesh_server::hazard::{HazardGate, Upload};

struct Key(SigningKey);
impl CaKey for Key {
    fn key_id(&self) -> u8 {
        1
    }
    fn public_key(&self) -> [u8; PUBLIC_KEY_LEN] {
        let mut o = [0u8; PUBLIC_KEY_LEN];
        o.copy_from_slice(self.0.verifying_key().to_sec1_point(true).as_bytes());
        o
    }
    fn sign(&self, _: &[u8]) -> Result<[u8; SIG_LEN], CaError> {
        Err(CaError::Unavailable)
    }
}

fn attestation() -> &'static DeviceAttestation {
    static A: OnceLock<DeviceAttestation> = OnceLock::new();
    A.get_or_init(|| {
        let k = SigningKey::generate();
        DeviceAttestation::new(
            Some(AppleConfig {
                app_id: "ABCDE12345.com.roadmesh.app".into(),
                root_der: pem_to_der(APPLE_ROOT_PEM).unwrap_or_default(),
                production: true,
            }),
            Some(PlayConfig {
                package: "com.roadmesh.app".into(),
                cert_digests: vec!["x".into()],
                decryption_key: [7; 32],
                verification_key: *k.verifying_key(),
                max_age_ms: 600_000,
            }),
            Box::new(|| 1_790_000_000_000),
        )
    })
}

fuzz_target!(|data: &[u8]| {
    let key = [2u8; PUBLIC_KEY_LEN];
    assert!(!attestation().verify(&key, b"challenge", data));
    let _ = device_key_of(data);
    let ca = CertAuthority::new(Key(SigningKey::generate()), RejectAll, Config::spec_v0());
    let mut gate = HazardGate::new(Config::spec_v0());
    assert!(matches!(gate.check(&ca, data, 1_790_000_000.0), Upload::Rejected(_)));
});
