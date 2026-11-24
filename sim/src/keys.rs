//! SIMULATOR ONLY: software stand-ins for the phones' secure hardware and the
//! CA's HSM, so the simulator runs the real certificate and signature code.
//!
//! Keys are derived from the simulator's seeded generator so runs repeat exactly.
//! That is fine for a simulator and must never be used by the product
//! (CLAUDE.md: seeded random is simulator-only). Phones generate keys in the
//! Secure Enclave / Android Keystore; the CA key lives in a KMS/HSM (Step 9).

use std::collections::HashMap;

use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};
use roadmesh_core::auth::{AuthError, Verifier};
use roadmesh_core::config::{Config, Security};
use roadmesh_core::pki::{
    CertDigest, CertVerifier, Certificate, KeyStore, PUBLIC_KEY_LEN, PseudonymSigner, SIG_LEN,
};
use roadmesh_core::rsm::BODY_LEN;
use roadmesh_server::ca::{Attestation, CaError, CaKey, CertAuthority, IssueRequest};

use crate::rng::SimRng;

/// A key from the seeded generator.
pub fn key_from(rng: &mut SimRng) -> SigningKey {
    loop {
        if let Ok(k) = SigningKey::from_slice(&rng.bytes::<32>()) {
            return k; // invalid scalars (0 or ≥ n) are astronomically rare; draw again
        }
    }
}

/// SEC1 compressed public key.
pub fn pubkey(k: &SigningKey) -> [u8; PUBLIC_KEY_LEN] {
    let mut o = [0u8; PUBLIC_KEY_LEN];
    o.copy_from_slice(k.verifying_key().to_sec1_point(true).as_bytes());
    o
}

/// ECDSA P-256 / SHA-256 signature, r‖s.
pub fn raw_sign(k: &SigningKey, msg: &[u8]) -> [u8; SIG_LEN] {
    let s: Signature = k.sign(msg);
    let mut o = [0u8; SIG_LEN];
    o.copy_from_slice(&s.to_bytes());
    o
}

/// Software key store standing in for the phone's secure hardware.
#[derive(Debug, Clone, Default)]
pub struct SoftKeys(pub HashMap<CertDigest, SigningKey>);

impl KeyStore for SoftKeys {
    fn sign(&mut self, d: &CertDigest, msg: &[u8]) -> Option<[u8; SIG_LEN]> {
        self.0.get(d).map(|k| raw_sign(k, msg))
    }
}

/// Software CA key standing in for the HSM.
#[derive(Debug, Clone)]
pub struct SimCaKey(SigningKey);

impl CaKey for SimCaKey {
    fn key_id(&self) -> u8 {
        1
    }
    fn public_key(&self) -> [u8; PUBLIC_KEY_LEN] {
        pubkey(&self.0)
    }
    fn sign(&self, m: &[u8]) -> Result<[u8; SIG_LEN], CaError> {
        Ok(raw_sign(&self.0, m))
    }
}

/// Every simulated phone is a genuine app install.
#[derive(Debug, Clone, Copy, Default)]
pub struct SimAttestation;

impl Attestation for SimAttestation {
    fn verify(&self, _: &[u8; PUBLIC_KEY_LEN], _: &[u8]) -> bool {
        true
    }
}

/// The simulator's certificate authority: the real server logic with a software key.
pub type SimCa = CertAuthority<SimCaKey, SimAttestation>;

/// New CA with a key from `rng`.
pub fn new_ca(rng: &mut SimRng, cfg: Config) -> SimCa {
    CertAuthority::new(SimCaKey(key_from(rng)), SimAttestation, cfg)
}

/// Enrol a phone: device key + `n` pseudonym keys, certificates issued by `ca` at `now`.
pub fn enrol(ca: &mut SimCa, rng: &mut SimRng, n: usize, now: u32) -> PseudonymSigner<SoftKeys> {
    enrol_with(ca, rng, n, now, &Security::v0())
}

/// [`enrol`] with given security settings (rotation, full-certificate policy).
pub fn enrol_with(
    ca: &mut SimCa,
    rng: &mut SimRng,
    n: usize,
    now: u32,
    sec: &Security,
) -> PseudonymSigner<SoftKeys> {
    let device = key_from(rng);
    let keys: Vec<SigningKey> = (0..n).map(|_| key_from(rng)).collect();
    let mut req = IssueRequest {
        device_key: pubkey(&device),
        attestation: Vec::new(),
        pseudonym_keys: keys.iter().map(pubkey).collect(),
        requested_at: now,
        signature: [0; SIG_LEN],
    };
    req.signature = raw_sign(&device, &req.to_be_signed());
    let certs = ca.issue(&req, now).unwrap_or_default();
    let mut store = SoftKeys::default();
    for (c, k) in certs.iter().zip(keys) {
        store.0.insert(c.digest(), k);
    }
    PseudonymSigner::new(certs, store, sec)
}

/// An outsider's signer: certificates from a CA key nobody trusts (forged).
pub fn rogue_signer(rng: &mut SimRng, n: usize, now: u32) -> PseudonymSigner<SoftKeys> {
    let rogue = SimCaKey(key_from(rng));
    let mut store = SoftKeys::default();
    let mut certs = Vec::new();
    for _ in 0..n {
        let k = key_from(rng);
        let mut c = Certificate {
            ca_key_id: rogue.key_id(), // claims to be the real CA
            valid_from: now,
            valid_until: now + 7 * 86_400,
            public_key: pubkey(&k),
            ca_signature: [0; SIG_LEN],
        };
        c.ca_signature = raw_sign(&rogue.0, &c.to_be_signed());
        store.0.insert(c.digest(), k);
        certs.push(c);
    }
    PseudonymSigner::new(certs, store, &Security::v0())
}

/// A phone's verifier, or none at all for the "signature checks off" comparison.
#[derive(Debug, Clone)]
pub enum SimVerifier {
    /// The real verifier.
    Real(Box<CertVerifier>),
    /// Accepts everything (comparison runs only).
    Off,
}

impl Verifier for SimVerifier {
    fn verify(&mut self, now: f64, body: &[u8; BODY_LEN], auth: &[u8]) -> Result<(), AuthError> {
        match self {
            Self::Real(v) => v.verify(now, body, auth),
            Self::Off => Ok(()),
        }
    }
}
