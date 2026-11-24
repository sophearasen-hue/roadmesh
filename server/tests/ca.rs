//! Certificate authority tests: issuance limits, reports and revocation
//! (AUDIT S-04, S-05, S-06). Every key is generated from the OS random source at
//! run time; nothing is hard-coded.

#![allow(clippy::panic)]

use std::collections::HashMap;

use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};
use p256::elliptic_curve::Generate;
use roadmesh_core::auth::{AuthError, Signer, Verifier};
use roadmesh_core::config::{Config, Security};
use roadmesh_core::pki::{
    CertDigest, CertVerifier, Certificate, CrlUpdate, KeyStore, MisbehaviourReport, PUBLIC_KEY_LEN,
    PseudonymSigner, ReportReason, SIG_LEN,
};
use roadmesh_core::rsm::{self, EventCode, Frame, MsgType, Rsm, VehicleType};
use roadmesh_server::ca::{
    Attestation, CaError, CaKey, CertAuthority, IssueError, IssueRequest, RejectAll, ReportError,
    ReportOutcome,
};

const T0: u32 = 1_800_000_000;
const WEEK: u32 = 7 * 86_400;

fn raw(k: &SigningKey, msg: &[u8]) -> [u8; SIG_LEN] {
    let s: Signature = k.sign(msg);
    let mut o = [0u8; SIG_LEN];
    o.copy_from_slice(&s.to_bytes());
    o
}

fn pubkey(k: &SigningKey) -> [u8; PUBLIC_KEY_LEN] {
    let mut o = [0u8; PUBLIC_KEY_LEN];
    o.copy_from_slice(k.verifying_key().to_sec1_point(true).as_bytes());
    o
}

/// Test-only CA key. Production uses a KMS/HSM key (Step 9).
struct SoftCa(SigningKey);
impl CaKey for SoftCa {
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

/// Test-only: every device is genuine.
struct AcceptAll;
impl Attestation for AcceptAll {
    fn verify(&self, _: &[u8; PUBLIC_KEY_LEN], _challenge: &[u8], _: &[u8]) -> bool {
        true
    }
}

#[derive(Default)]
struct SoftKeys(HashMap<CertDigest, SigningKey>);
impl KeyStore for SoftKeys {
    fn sign(&mut self, d: &CertDigest, m: &[u8]) -> Option<[u8; SIG_LEN]> {
        self.0.get(d).map(|k| raw(k, m))
    }
}

/// A phone: device key plus pseudonym keys "in hardware".
struct Phone {
    device: SigningKey,
    pending: Vec<SigningKey>,
}

impl Phone {
    fn new() -> Self {
        Self {
            device: SigningKey::generate(),
            pending: Vec::new(),
        }
    }
    fn request(&mut self, n: usize, at: u32) -> IssueRequest {
        self.pending = (0..n).map(|_| SigningKey::generate()).collect();
        let mut r = IssueRequest {
            device_key: pubkey(&self.device),
            attestation: b"evidence".to_vec(),
            pseudonym_keys: self.pending.iter().map(pubkey).collect(),
            requested_at: at,
            signature: [0; SIG_LEN],
        };
        r.signature = raw(&self.device, &r.to_be_signed());
        r
    }
    fn signer(&mut self, certs: Vec<Certificate>) -> PseudonymSigner<SoftKeys> {
        let mut keys = SoftKeys::default();
        for (c, k) in certs.iter().zip(self.pending.drain(..)) {
            keys.0.insert(c.digest(), k);
        }
        PseudonymSigner::new(certs, keys, &Security::v0())
    }
}

type Ca = CertAuthority<SoftCa, AcceptAll>;

fn ca() -> Ca {
    CertAuthority::new(SoftCa(SigningKey::generate()), AcceptAll, Config::spec_v0())
}

fn enrol(ca: &mut Ca, n: usize) -> PseudonymSigner<SoftKeys> {
    let mut p = Phone::new();
    let req = p.request(n, T0);
    let certs = ca.issue(&req, T0).unwrap_or_else(|e| panic!("{e:?}"));
    p.signer(certs)
}

fn frame(s: &mut PseudonymSigner<SoftKeys>, now: f64, msg_id: u16, speed: f64) -> Frame {
    let id = s.current_temp_id(now).unwrap_or_else(|| panic!("id"));
    let body = Rsm {
        msg_type: MsgType::Heartbeat,
        temp_id: id,
        msg_id,
        timestamp_ms: rsm::ms_of_hour(now),
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
    .encode();
    let auth = s.sign(now, false, &body).unwrap_or_else(|| panic!("sig"));
    Frame {
        last_hop: None,
        body,
        auth,
    }
}

fn report(
    reporter: &mut PseudonymSigner<SoftKeys>,
    evidence: Vec<Frame>,
    reason: ReportReason,
    now: f64,
) -> Vec<u8> {
    MisbehaviourReport::build(reporter, now, reason, evidence)
        .and_then(|r| r.encode())
        .unwrap_or_else(|| panic!("report"))
}

fn now() -> f64 {
    f64::from(T0) + 10.0
}

#[test]
fn issues_up_to_20_certificates_per_week() {
    let mut ca = ca();
    let mut p = Phone::new();
    let certs = ca
        .issue(&p.request(20, T0), T0)
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(certs.len(), 20);
    let anchor = ca.trust_anchor();
    let mut v = CertVerifier::new(&[anchor], &Security::v0()).unwrap_or_else(|_| panic!("anchor"));
    for c in &certs {
        assert_eq!(c.valid_until - c.valid_from, WEEK);
        assert!(v.check_certificate(c, f64::from(T0)).is_ok());
    }
    let mut ids: Vec<_> = certs.iter().map(|c| c.temp_id()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 20); // unlinkable: a different key and pseudonym each
    // the 21st in the same week is refused
    assert_eq!(
        ca.issue(&p.request(1, T0 + 3600), T0 + 3600),
        Err(IssueError::QuotaExceeded { remaining: 0 })
    );
    // a week later the window has moved on
    assert!(
        ca.issue(&p.request(20, T0 + WEEK + 1), T0 + WEEK + 1)
            .is_ok()
    );
}

#[test]
fn partial_quota_is_reported() {
    let mut ca = ca();
    let mut p = Phone::new();
    assert!(ca.issue(&p.request(15, T0), T0).is_ok());
    assert_eq!(
        ca.issue(&p.request(10, T0 + 1), T0 + 1),
        Err(IssueError::QuotaExceeded { remaining: 5 })
    );
}

#[test]
fn issuance_rejects_bad_requests() {
    let mut ca = ca();
    let mut p = Phone::new();
    assert_eq!(ca.issue(&p.request(0, T0), T0), Err(IssueError::BadRequest));
    assert_eq!(
        ca.issue(&p.request(21, T0), T0),
        Err(IssueError::BadRequest)
    );
    let mut bad = p.request(2, T0);
    bad.signature[3] ^= 1;
    assert_eq!(ca.issue(&bad, T0), Err(IssueError::BadSignature));
    // someone else's device signature
    let mut stolen = p.request(2, T0);
    stolen.device_key = pubkey(&SigningKey::generate());
    assert_eq!(ca.issue(&stolen, T0), Err(IssueError::BadSignature));
    // an old request replayed
    assert_eq!(
        ca.issue(&p.request(2, T0), T0 + 3600),
        Err(IssueError::StaleRequest)
    );
    // the same pseudonym key twice, or the device key as a pseudonym key
    let mut dup = p.request(2, T0);
    dup.pseudonym_keys[1] = dup.pseudonym_keys[0];
    dup.signature = raw(&p.device, &dup.to_be_signed());
    assert_eq!(ca.issue(&dup, T0), Err(IssueError::ReusedKey));
    let mut own = p.request(1, T0);
    own.pseudonym_keys[0] = own.device_key;
    own.signature = raw(&p.device, &own.to_be_signed());
    assert_eq!(ca.issue(&own, T0), Err(IssueError::ReusedKey));
    // a key already certified for another request
    let first = p.request(1, T0);
    assert!(ca.issue(&first, T0).is_ok());
    let mut again = p.request(1, T0);
    again.pseudonym_keys = first.pseudonym_keys.clone();
    again.signature = raw(&p.device, &again.to_be_signed());
    assert_eq!(ca.issue(&again, T0), Err(IssueError::ReusedKey));
}

#[test]
fn production_attestation_rejects_everything_until_step_9() {
    let mut ca = CertAuthority::new(SoftCa(SigningKey::generate()), RejectAll, Config::spec_v0());
    let mut p = Phone::new();
    assert_eq!(
        ca.issue(&p.request(1, T0), T0),
        Err(IssueError::AttestationFailed)
    );
}

#[test]
fn revocation_needs_three_independent_devices() {
    let mut ca = ca();
    let mut evil = enrol(&mut ca, 3);
    let t = now();
    let proof = vec![frame(&mut evil, t, 1, 95.0)]; // 342 km/h
    let evil_cert = evil.certificate_at(t).unwrap_or_else(|| panic!("cert"));

    // one phone with three pseudonyms is still ONE reporter
    let mut a = enrol(&mut ca, 3);
    for k in 0..3 {
        let at = t + 300.0 * f64::from(k); // a different pseudonym each time
        let out = ca.report(
            &report(&mut a, proof.clone(), ReportReason::ImplausibleSpeed, at),
            T0 + 20,
        );
        assert_eq!(out, Ok(ReportOutcome::Counted(1)));
    }
    let mut b = enrol(&mut ca, 1);
    assert_eq!(
        ca.report(
            &report(&mut b, proof.clone(), ReportReason::ImplausibleSpeed, t),
            T0 + 20
        ),
        Ok(ReportOutcome::Counted(2))
    );
    assert!(!ca.is_revoked(&evil_cert.digest()));
    let mut c = enrol(&mut ca, 1);
    assert_eq!(
        ca.report(
            &report(&mut c, proof.clone(), ReportReason::ImplausibleSpeed, t),
            T0 + 20
        ),
        Ok(ReportOutcome::Revoked)
    );
    // ALL of the device's certificates are revoked, not just the reported one
    for cert in evil.certificates() {
        assert!(ca.is_revoked(&cert.digest()));
    }
    let mut d = enrol(&mut ca, 1);
    assert_eq!(
        ca.report(
            &report(&mut d, proof, ReportReason::ImplausibleSpeed, t),
            T0 + 20
        ),
        Ok(ReportOutcome::AlreadyRevoked)
    );
}

#[test]
fn revoked_device_is_refused_and_phones_reject_it() {
    let mut ca = ca();
    let mut evil_phone = Phone::new();
    let certs = ca
        .issue(&evil_phone.request(2, T0), T0)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let mut evil = evil_phone.signer(certs);
    let t = now();
    let good = frame(&mut evil, t, 1, 10.0);
    let proof = vec![frame(&mut evil, t + 0.1, 2, 95.0)];

    let mut phone = CertVerifier::new(&[ca.trust_anchor()], &Security::v0())
        .unwrap_or_else(|_| panic!("anchor"));
    assert_eq!(phone.verify(t, &good.body, &good.auth), Ok(()));

    for _ in 0..3 {
        let mut r = enrol(&mut ca, 1);
        assert!(
            ca.report(
                &report(&mut r, proof.clone(), ReportReason::ImplausibleSpeed, t),
                T0 + 20
            )
            .is_ok()
        );
    }
    let crl = ca
        .revocation_list(T0 + 30, T0 + 86_400)
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(phone.load_revocation_list(&crl), Ok(CrlUpdate::Updated(1)));
    assert_eq!(
        phone.verify(t, &good.body, &good.auth),
        Err(AuthError::RevokedCertificate)
    );
    // and it gets no new certificates
    assert_eq!(
        ca.issue(&evil_phone.request(1, T0 + 40), T0 + 40),
        Err(IssueError::Banned)
    );
}

#[test]
fn reports_without_real_evidence_do_not_count() {
    let mut ca = ca();
    let mut honest = enrol(&mut ca, 1);
    let mut a = enrol(&mut ca, 1);
    let t = now();
    // a normal message is no evidence
    let normal = vec![frame(&mut honest, t, 1, 10.0)];
    assert_eq!(
        ca.report(
            &report(&mut a, normal.clone(), ReportReason::ImplausibleSpeed, t),
            T0 + 20
        ),
        Err(ReportError::Unconfirmed)
    );
    // edited evidence: the signature no longer matches
    let mut forged = frame(&mut honest, t, 2, 10.0);
    let mut r = Rsm::decode(&forged.body).unwrap_or_else(|_| panic!("decode"));
    r.speed = 95.0;
    forged.body = r.encode();
    assert_eq!(
        ca.report(
            &report(&mut a, vec![forged], ReportReason::ImplausibleSpeed, t),
            T0 + 20
        ),
        Err(ReportError::BadEvidence)
    );
    // reporting yourself
    let mut evil = enrol(&mut ca, 1);
    let mine = vec![frame(&mut evil, t, 1, 95.0)];
    assert_eq!(
        ca.report(
            &report(&mut evil, mine, ReportReason::ImplausibleSpeed, t),
            T0 + 20
        ),
        Err(ReportError::SelfReport)
    );
    // garbage
    assert_eq!(ca.report(&[1, 2, 3], T0), Err(ReportError::Malformed));
}

#[test]
fn reports_from_unknown_reporters_are_ignored() {
    let mut ca = ca();
    let mut other_ca = self::ca();
    let mut evil = enrol(&mut ca, 1);
    let mut outsider = enrol(&mut other_ca, 1); // certificate from a different CA
    let t = now();
    let proof = vec![frame(&mut evil, t, 1, 95.0)];
    assert_eq!(
        ca.report(
            &report(
                &mut outsider,
                proof.clone(),
                ReportReason::ImplausibleSpeed,
                t
            ),
            T0 + 20
        ),
        Err(ReportError::UnknownReporter)
    );
    // a tampered report signature
    let mut a = enrol(&mut ca, 1);
    let mut bytes = report(&mut a, proof, ReportReason::ImplausibleSpeed, t);
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    assert_eq!(ca.report(&bytes, T0 + 20), Err(ReportError::BadSignature));
}

#[test]
fn reporters_are_rate_limited() {
    let mut ca = ca();
    let mut a = enrol(&mut ca, 1);
    let t = now();
    let limit = Security::v0().reports_per_device_per_day;
    for i in 0..limit {
        let mut evil = enrol(&mut ca, 1);
        let proof = vec![frame(&mut evil, t, 1, 95.0)];
        assert!(
            ca.report(
                &report(&mut a, proof, ReportReason::ImplausibleSpeed, t),
                T0 + 20
            )
            .is_ok(),
            "report {i}"
        );
    }
    let mut evil = enrol(&mut ca, 1);
    let proof = vec![frame(&mut evil, t, 1, 95.0)];
    assert_eq!(
        ca.report(
            &report(&mut a, proof, ReportReason::ImplausibleSpeed, t),
            T0 + 20
        ),
        Err(ReportError::RateLimited)
    );
}

#[test]
fn revocation_list_drops_expired_certificates() {
    let mut ca = ca();
    let mut evil_phone = Phone::new();
    let certs = ca
        .issue(&evil_phone.request(1, T0), T0)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let d = certs[0].digest();
    let id = ca.device_of(&d).unwrap_or_else(|| panic!("device"));
    ca.revoke_device(&id);
    let anchor = [ca.trust_anchor()];
    let fresh = ca
        .revocation_list(T0, T0 + 1)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let later = ca
        .revocation_list(T0 + WEEK + 1, T0 + WEEK + 2)
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(later.len() < fresh.len()); // expired entries are not carried forever
    let mut v = CertVerifier::new(&anchor, &Security::v0()).unwrap_or_else(|_| panic!("anchor"));
    assert!(v.load_revocation_list(&fresh).is_ok());
    assert!(v.is_revoked(&d));
}
