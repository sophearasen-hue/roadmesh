//! Shared fuzz fixtures. Keys come from the OS random source when the fuzzer
//! starts; none are hard-coded. Crashes never depend on key values: inputs that
//! reach signature checks fail them the same way for any key.

#![allow(dead_code)]

use std::collections::HashMap;

use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};
use p256::elliptic_curve::Generate;
use roadmesh_core::auth::Signer;
use roadmesh_core::config::{Config, Security};
use roadmesh_core::pki::{
    CertDigest, CertVerifier, KeyStore, PUBLIC_KEY_LEN, PseudonymSigner, SIG_LEN, TrustAnchor,
};
use roadmesh_core::rsm::{self, EventCode, Frame, MsgType, Rsm, VehicleType};
use roadmesh_server::ca::{Attestation, CaError, CaKey, CertAuthority, IssueRequest};

pub const NOW: f64 = 1_800_000_000.0;

pub fn raw(k: &SigningKey, m: &[u8]) -> [u8; SIG_LEN] {
    let s: Signature = k.sign(m);
    let mut o = [0u8; SIG_LEN];
    o.copy_from_slice(&s.to_bytes());
    o
}

pub fn pubkey(k: &SigningKey) -> [u8; PUBLIC_KEY_LEN] {
    let mut o = [0u8; PUBLIC_KEY_LEN];
    o.copy_from_slice(k.verifying_key().to_sec1_point(true).as_bytes());
    o
}

pub struct FuzzCa(pub SigningKey);
impl CaKey for FuzzCa {
    fn key_id(&self) -> u8 {
        1
    }
    fn public_key(&self) -> [u8; PUBLIC_KEY_LEN] {
        pubkey(&self.0)
    }
    fn sign(&self, m: &[u8]) -> Result<[u8; SIG_LEN], CaError> {
        Ok(raw(&self.0, m))
    }
}

pub struct Genuine;
impl Attestation for Genuine {
    fn verify(&self, _: &[u8; PUBLIC_KEY_LEN], _: &[u8]) -> bool {
        true
    }
}

#[derive(Default)]
pub struct SoftKeys(pub HashMap<CertDigest, SigningKey>);
impl KeyStore for SoftKeys {
    fn sign(&mut self, d: &CertDigest, m: &[u8]) -> Option<[u8; SIG_LEN]> {
        self.0.get(d).map(|k| raw(k, m))
    }
}

pub struct Fixture {
    pub ca: CertAuthority<FuzzCa, Genuine>,
    pub anchor: TrustAnchor,
    pub phone: PseudonymSigner<SoftKeys>,
    /// A valid, signed frame with the full certificate (mutation seed).
    pub frame: Vec<u8>,
    /// A valid misbehaviour report (mutation seed).
    pub report: Vec<u8>,
    /// A valid revocation list (mutation seed).
    pub crl: Vec<u8>,
}

fn enrol(ca: &mut CertAuthority<FuzzCa, Genuine>, now: u32) -> PseudonymSigner<SoftKeys> {
    let device = SigningKey::generate();
    let k = SigningKey::generate();
    let mut req = IssueRequest {
        device_key: pubkey(&device),
        attestation: Vec::new(),
        pseudonym_keys: vec![pubkey(&k)],
        requested_at: now,
        signature: [0; SIG_LEN],
    };
    req.signature = raw(&device, &req.to_be_signed());
    let certs = ca.issue(&req, now).unwrap_or_default();
    let mut keys = SoftKeys::default();
    if let Some(c) = certs.first() {
        keys.0.insert(c.digest(), k);
    }
    PseudonymSigner::new(certs, keys, &Security::v0())
}

pub fn body(temp_id: [u8; 4], speed: f64) -> [u8; rsm::BODY_LEN] {
    Rsm {
        msg_type: MsgType::Heartbeat,
        temp_id,
        msg_id: 1,
        timestamp_ms: rsm::ms_of_hour(NOW),
        lat_deg: 11.5564,
        lon_deg: 104.9282,
        speed,
        heading_deg: 90.0,
        accel: 0.0,
        yaw_rate: 0.0,
        vehicle_type: VehicleType::Car,
        flags: 0,
        pos_accuracy_m: 1.0,
        hop_count: 0,
        ttl: 2,
        origin_pos_age_ms: 0,
        event: EventCode::None,
    }
    .encode()
}

impl Fixture {
    pub fn new() -> Self {
        let mut ca = CertAuthority::new(FuzzCa(SigningKey::generate()), Genuine, Config::spec_v0());
        let anchor = ca.trust_anchor();
        let now = NOW as u32;
        let mut phone = enrol(&mut ca, now);
        let mut evil = enrol(&mut ca, now);
        let tid = phone.current_temp_id(NOW).unwrap_or([0; 4]);
        let b = body(tid, 10.0);
        let auth = phone.sign(NOW, false, &b).unwrap_or_default();
        let frame = Frame { last_hop: None, body: b, auth }.encode().unwrap_or_default();
        let etid = evil.current_temp_id(NOW).unwrap_or([0; 4]);
        let eb = body(etid, 95.0);
        let eauth = evil.sign(NOW, false, &eb).unwrap_or_default();
        let ev = Frame { last_hop: None, body: eb, auth: eauth };
        let report = roadmesh_core::pki::MisbehaviourReport::build(
            &mut phone,
            NOW,
            roadmesh_core::pki::ReportReason::ImplausibleSpeed,
            vec![ev],
        )
        .and_then(|r| r.encode())
        .unwrap_or_default();
        let crl = ca.revocation_list(now, now + 86_400).unwrap_or_default();
        Self { ca, anchor, phone, frame, report, crl }
    }

    pub fn verifier(&self) -> CertVerifier {
        match CertVerifier::new(&[self.anchor], &Security::v0()) {
            Ok(v) => v,
            Err(_) => std::process::abort(),
        }
    }
}

/// Seed XOR data: lets the fuzzer explore near a valid input.
pub fn mutate(seed: &[u8], data: &[u8]) -> Vec<u8> {
    let mut out = seed.to_vec();
    for (i, d) in data.iter().enumerate() {
        if i < out.len() {
            out[i] ^= d;
        } else {
            out.push(*d);
        }
    }
    out
}
