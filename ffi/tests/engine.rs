//! Two phones through the app-facing API: certificates from the CA, signed frames,
//! peers with speed and distance (the Step 5 "done when", in software). Keys are
//! generated at run time and stand in for the Keystore / Secure Enclave.

#![allow(clippy::panic)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};
use p256::elliptic_curve::Generate;
use roadmesh_core::config::Config;
use roadmesh_ffi::{
    HardwareKeys, OwnFix, RoadInfo, RxResult, SafetyEngine, certificate_digests,
    encode_issue_request, issue_request_to_sign,
};
use roadmesh_server::ca::{Attestation, CaError, CaKey, CertAuthority, IssueRequest};

const T0: f64 = 1_800_000_000.0;

fn raw(k: &SigningKey, m: &[u8]) -> Vec<u8> {
    let s: Signature = k.sign(m);
    s.to_bytes().to_vec()
}

fn pubkey(k: &SigningKey) -> Vec<u8> {
    k.verifying_key().to_sec1_point(true).as_bytes().to_vec()
}

struct TestCa(SigningKey);
impl CaKey for TestCa {
    fn key_id(&self) -> u8 {
        1
    }
    fn public_key(&self) -> [u8; 33] {
        let mut o = [0u8; 33];
        o.copy_from_slice(&pubkey(&self.0));
        o
    }
    fn sign(&self, m: &[u8]) -> Result<[u8; 64], CaError> {
        let mut o = [0u8; 64];
        o.copy_from_slice(&raw(&self.0, m));
        Ok(o)
    }
}
struct Genuine;
impl Attestation for Genuine {
    fn verify(&self, _: &[u8; 33], _: &[u8], _: &[u8]) -> bool {
        true
    }
}

/// Software stand-in for the phone's key store.
#[derive(Default)]
struct SoftKeys(Mutex<HashMap<Vec<u8>, SigningKey>>);
impl HardwareKeys for SoftKeys {
    fn sign(&self, cert_digest: Vec<u8>, message: Vec<u8>) -> Vec<u8> {
        let g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        g.get(&cert_digest)
            .map(|k| raw(k, &message))
            .unwrap_or_default()
    }
}

fn phone(ca: &mut CertAuthority<TestCa, Genuine>) -> Arc<SafetyEngine> {
    let keys = Arc::new(SoftKeys::default());
    let mut anchor = vec![1u8];
    anchor.extend(ca.trust_anchor().public_key);
    let engine = SafetyEngine::new(11.5564, 104.9282, vec![anchor], keys.clone(), false)
        .unwrap_or_else(|e| panic!("{e}"));
    // the app's enrolment flow: keys in "hardware", request signed by the device key
    let device = SigningKey::generate();
    let pseudo: Vec<SigningKey> = (0..3).map(|_| SigningKey::generate()).collect();
    let pubs: Vec<Vec<u8>> = pseudo.iter().map(pubkey).collect();
    let at = T0 as u32;
    let tbs =
        issue_request_to_sign(pubkey(&device), pubs.clone(), at).unwrap_or_else(|e| panic!("{e}"));
    let wire = encode_issue_request(pubkey(&device), pubs, at, vec![], raw(&device, &tbs))
        .unwrap_or_else(|e| panic!("{e}"));
    let req = IssueRequest::decode(&wire).unwrap_or_else(|e| panic!("{e:?}"));
    let certs = ca.issue(&req, at).unwrap_or_else(|e| panic!("{e:?}"));
    let batch = roadmesh_core::pki::encode_certificates(&certs);
    for (d, k) in certificate_digests(batch.clone())
        .unwrap_or_else(|e| panic!("{e}"))
        .into_iter()
        .zip(pseudo)
    {
        keys.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(d, k);
    }
    assert_eq!(
        engine
            .add_certificates(batch, T0)
            .unwrap_or_else(|e| panic!("{e}")),
        3
    );
    engine
}

fn fix(lat: f64, lon: f64, heading: f64, speed: f64) -> OwnFix {
    OwnFix {
        lat,
        lon,
        heading_deg: heading,
        speed_mps: speed,
        accel_mps2: 0.0,
        yaw_rate_dps: 0.0,
        accuracy_m: 3.0,
        vehicle_type: 1,
    }
}

#[test]
fn two_phones_see_each_other_with_speed_and_distance() {
    let mut ca = CertAuthority::new(TestCa(SigningKey::generate()), Genuine, Config::spec_v0());
    let (a, b) = (phone(&mut ca), phone(&mut ca));
    let road = RoadInfo {
        speed_limit_mps: Some(40.0 / 3.6),
        at_junction: false,
    };
    // B is 0.0009° (≈ 100 m) north of A, both riding north at 10 m/s
    let (fa, fb) = (
        fix(11.5564, 104.9282, 0.0, 10.0),
        fix(11.5573, 104.9282, 0.0, 10.0),
    );
    for k in 0..5 {
        let now = T0 + f64::from(k) * 0.1;
        for f in b.transmit(now, fb, road) {
            assert!(matches!(a.receive(f, now, fa), RxResult::Accepted { .. }));
        }
        for f in a.transmit(now, fa, road) {
            assert!(matches!(b.receive(f, now, fb), RxResult::Accepted { .. }));
        }
    }
    let peers = a.peers(T0 + 0.4, fa);
    assert_eq!(peers.len(), 1);
    let p = &peers[0];
    assert!(
        (p.distance_m - 100.0).abs() < 3.0,
        "distance {}",
        p.distance_m
    );
    assert!((p.speed_mps - 10.0).abs() < 0.1);
    assert_eq!(p.vehicle_type, 1);
    assert!(p.north_m > 90.0);
    let d = a.decide(T0 + 0.4, fa, road);
    assert!(d.alerts.iter().all(|al| al.kind != "CrossingConflict"));
}

#[test]
fn tampered_or_foreign_frames_are_rejected() {
    let mut ca = CertAuthority::new(TestCa(SigningKey::generate()), Genuine, Config::spec_v0());
    let mut other_ca =
        CertAuthority::new(TestCa(SigningKey::generate()), Genuine, Config::spec_v0());
    let (a, b) = (phone(&mut ca), phone(&mut ca));
    let outsider = phone(&mut other_ca);
    let road = RoadInfo {
        speed_limit_mps: None,
        at_junction: false,
    };
    let fa = fix(11.5564, 104.9282, 0.0, 10.0);
    let mut frame = b
        .transmit(T0, fix(11.5570, 104.9282, 0.0, 10.0), road)
        .remove(0);
    frame[25] ^= 0x10; // change a signed byte
    assert!(matches!(
        a.receive(frame, T0, fa),
        RxResult::Rejected { .. }
    ));
    let forged = outsider
        .transmit(T0, fix(11.5570, 104.9282, 0.0, 10.0), road)
        .remove(0);
    assert!(matches!(
        a.receive(forged, T0, fa),
        RxResult::Rejected { .. }
    ));
    assert!(matches!(
        a.receive(vec![1, 2, 3], T0, fa),
        RxResult::Rejected { .. }
    ));
    assert!(a.peers(T0, fa).is_empty());
}

#[test]
fn no_certificate_means_receive_only() {
    let keys = Arc::new(SoftKeys::default());
    let e = SafetyEngine::new(11.5, 104.9, vec![], keys, false).unwrap_or_else(|e| panic!("{e}"));
    let road = RoadInfo {
        speed_limit_mps: None,
        at_junction: false,
    };
    assert_eq!(e.valid_certificates(T0), 0);
    assert!(e.transmit(T0, fix(11.5, 104.9, 0.0, 5.0), road).is_empty());
    assert!(e.pseudonym(T0).is_empty());
}

#[test]
fn trip_timeline_and_delete_my_data() {
    let mut ca = CertAuthority::new(TestCa(SigningKey::generate()), Genuine, Config::spec_v0());
    let (a, b) = (phone(&mut ca), phone(&mut ca));
    let road = RoadInfo {
        speed_limit_mps: Some(40.0 / 3.6),
        at_junction: false,
    };
    let fb = fix(11.5573, 104.9282, 0.0, 10.0);
    // A rides north at 20 m/s (72 km/h) on a 40 km/h road for 20 s: one speeding episode
    for k in 0..200 {
        let now = T0 + f64::from(k) * 0.1;
        let fa = fix(11.5564 + 0.0000180 * f64::from(k), 104.9282, 0.0, 20.0);
        for f in b.transmit(now, fb, road) {
            a.receive(f, now, fa);
        }
        a.decide(now, fa, road);
    }
    let trip = a.trip();
    assert!((trip.distance_m - 398.0).abs() < 5.0, "{}", trip.distance_m);
    assert!((trip.max_speed_mps - 20.0).abs() < 1e-9);
    assert_eq!(trip.counts.speeding, 1);
    assert!(trip.score < 100);
    let ev = a.events(T0 + 20.0, 10);
    assert!(
        ev.iter()
            .any(|e| e.me && e.kind == "speeding" && e.kmh > 70.0)
    );
    assert!(ev.iter().filter(|e| e.me).all(|e| e.pseudonym.is_empty()));
    let peers = a.peers(T0 + 19.9, fix(11.5600, 104.9282, 0.0, 20.0));
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].score, 100); // a calm peer
    a.delete_my_data();
    assert!(a.events(T0 + 20.0, 10).is_empty());
    assert_eq!(a.trip().distance_m, 0.0);
    assert!(a.peers(T0 + 20.0, fb).is_empty());
}
