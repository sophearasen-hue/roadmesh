//! The cellular bridge end to end, through the app-facing API: two phones enrol with
//! the edge broker, one brakes hard 450 m from the other (out of radio range), and the
//! event reaches the other phone's safety engine through the server (BUILD_PLAN Step 9
//! "done when", in software; TLS is covered by server/tests/tls.rs).

#![allow(clippy::panic, clippy::unwrap_used)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};
use p256::elliptic_curve::Generate;
use roadmesh_core::config::Config;
use roadmesh_ffi::{
    CellularLink, HardwareKeys, LinkEvent, OwnFix, RoadInfo, RxResult, SafetyEngine,
    certificate_digests, encode_issue_request, issue_request_to_sign, parse_enrol_reply,
};
use roadmesh_server::broker::{Broker, Peer, Settings};
use roadmesh_server::ca::{Attestation, CaError, CaKey, CertAuthority, device_id};
use roadmesh_server::devca::DeviceCa;
use roadmesh_server::hazard::HazardGate;
use roadmesh_server::monitor::Monitor;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

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
        pubkey(&self.0).try_into().unwrap()
    }
    fn sign(&self, m: &[u8]) -> Result<[u8; 64], CaError> {
        Ok(raw(&self.0, m).try_into().unwrap())
    }
}
struct Genuine;
impl Attestation for Genuine {
    fn verify(&self, _: &[u8; 33], _: &[u8], ev: &[u8]) -> bool {
        ev == b"evidence"
    }
}

#[derive(Default)]
struct SoftKeys(Mutex<HashMap<Vec<u8>, SigningKey>>);
impl HardwareKeys for SoftKeys {
    fn sign(&self, cert_digest: Vec<u8>, message: Vec<u8>) -> Vec<u8> {
        let g = self.0.lock().unwrap();
        g.get(&cert_digest)
            .map(|k| raw(k, &message))
            .unwrap_or_default()
    }
}

type B = Broker<TestCa, TestCa, Genuine>;

/// The platform's socket loop, in miniature: write what the link returns, feed what
/// arrives back into it.
struct Socket {
    io: DuplexStream,
    link: Arc<CellularLink>,
}

impl Socket {
    async fn open(b: &Arc<B>, peer: Peer) -> Self {
        let (a, z) = tokio::io::duplex(1 << 16);
        tokio::spawn(b.clone().serve(z, peer));
        let mut s = Self {
            io: a,
            link: CellularLink::new(),
        };
        let c = s.link.connect(60);
        s.write(&c).await;
        assert_eq!(s.events().await, vec![LinkEvent::Connected]);
        s
    }
    async fn write(&mut self, b: &[u8]) {
        self.io.write_all(b).await.unwrap();
    }
    /// Events until 300 ms of silence.
    async fn events(&mut self) -> Vec<LinkEvent> {
        let mut out = Vec::new();
        let mut chunk = [0u8; 4096];
        while let Ok(Ok(n)) = tokio::time::timeout(
            std::time::Duration::from_millis(300),
            self.io.read(&mut chunk),
        )
        .await
        {
            if n == 0 {
                break;
            }
            out.extend(self.link.on_bytes(chunk[..n].to_vec()));
        }
        out
    }
}

struct App {
    engine: Arc<SafetyEngine>,
    device: SigningKey,
}

/// The app's online enrolment: request signed by the device key, evidence attached.
async fn enrol(b: &Arc<B>, anchor: Vec<u8>, ip: u8, now: f64) -> App {
    let keys = Arc::new(SoftKeys::default());
    let engine = SafetyEngine::new(11.5564, 104.9282, vec![anchor], keys.clone(), false).unwrap();
    let device = SigningKey::generate();
    let pseudo: Vec<SigningKey> = (0..20).map(|_| SigningKey::generate()).collect();
    let pubs: Vec<Vec<u8>> = pseudo.iter().map(pubkey).collect();
    let at = now as u32;
    let tbs = issue_request_to_sign(pubkey(&device), pubs.clone(), at).unwrap();
    let wire = encode_issue_request(
        pubkey(&device),
        pubs,
        at,
        b"evidence".to_vec(),
        raw(&device, &tbs),
    )
    .unwrap();
    let mut s = Socket::open(b, Peer::Enrol([10, 0, 0, ip].into())).await;
    let req = s.link.enrol(wire);
    s.write(&req).await;
    let ev = s.events().await;
    let Some(LinkEvent::EnrolReply { reply }) = ev.first() else {
        panic!("{ev:?}")
    };
    let enrolled = parse_enrol_reply(reply.clone()).unwrap();
    assert!(!enrolled.device_certificate.is_empty());
    let digests = certificate_digests(enrolled.certificates.clone()).unwrap();
    for (d, k) in digests.into_iter().zip(pseudo) {
        keys.0.lock().unwrap().insert(d, k);
    }
    assert_eq!(
        engine.add_certificates(enrolled.certificates, now).unwrap(),
        20
    );
    App { engine, device }
}

fn fix(lat: f64, accel: f64) -> OwnFix {
    OwnFix {
        lat,
        lon: 104.9282,
        heading_deg: 0.0,
        speed_mps: 12.0,
        accel_mps2: accel,
        yaw_rate_dps: 0.0,
        accuracy_m: 3.0,
        vehicle_type: 0,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn hard_braking_reaches_a_phone_450_m_away_through_the_server() {
    let clock = Arc::new(AtomicU64::new((T0 * 1000.0) as u64));
    let c = clock.clone();
    let ca = CertAuthority::new(TestCa(SigningKey::generate()), Genuine, Config::spec_v0());
    let mut anchor = vec![1u8];
    anchor.extend(ca.trust_anchor().public_key);
    let b = Broker::new(
        ca,
        DeviceCa::new(TestCa(SigningKey::generate())).unwrap(),
        HazardGate::new(Config::spec_v0()),
        Settings::default(),
        Arc::new(Monitor::capturing()),
        Box::new(move || c.load(Ordering::Relaxed) as f64 / 1000.0),
    );
    let now = T0;
    let a = enrol(&b, anchor.clone(), 1, now).await;
    let z = enrol(&b, anchor, 2, now).await;
    let id = |app: &App| device_id(&pubkey(&app.device).try_into().unwrap());
    let mut sa = Socket::open(&b, Peer::Device(id(&a))).await;
    let mut sz = Socket::open(&b, Peer::Device(id(&z))).await;
    let (la, lz) = (11.5564, 11.5564 + 0.004); // 450 m apart
    for (s, lat) in [(&mut sa, la), (&mut sz, lz)] {
        let sub = s.link.follow_position(lat, 104.9282);
        s.write(&sub).await;
    }
    sz.events().await; // subscription acknowledgements
    let road = RoadInfo {
        speed_limit_mps: None,
        at_junction: false,
    };
    // A cruises (heartbeats: not uploaded), then brakes hard (event: uploaded)
    let mut uploaded = 0;
    for (k, accel) in [0.0, 0.0, -6.0].into_iter().enumerate() {
        let t = now + 0.1 * k as f64;
        for frame in a.engine.transmit(t, fix(la, accel), road.clone()) {
            let up = sa.link.upload(frame);
            if !up.is_empty() {
                uploaded += 1;
                sa.write(&up).await;
            }
        }
    }
    assert_eq!(uploaded, 1);
    let ev = sz.events().await;
    let frames: Vec<Vec<u8>> = ev
        .into_iter()
        .filter_map(|e| match e {
            LinkEvent::HazardEvent { frame } => Some(frame),
            _ => None,
        })
        .collect();
    assert_eq!(frames.len(), 1);
    // the receiving phone checks it like a radio frame and knows the braking vehicle
    let r = z.engine.receive(frames[0].clone(), now + 0.3, fix(lz, 0.0));
    assert!(matches!(r, RxResult::Accepted { .. }), "{r:?}");
    assert_eq!(z.engine.peers(now + 0.3, fix(lz, 0.0)).len(), 1);
}
