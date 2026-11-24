//! Test helpers shared by the server integration tests. Every key comes from the OS
//! random source at run time; nothing is hard-coded.
#![allow(dead_code, clippy::panic)]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};
use p256::elliptic_curve::Generate;
use roadmesh_core::config::{Config, Security};
use roadmesh_core::mqtt::{self, Packet};
use roadmesh_core::pki::{
    self, CertDigest, Certificate, IssueRequestWire, KeyStore, PUBLIC_KEY_LEN, PseudonymSigner,
    SIG_LEN,
};
use roadmesh_core::rsm::{self, EventCode, Frame, MsgType, Rsm, VehicleType};
use roadmesh_server::broker::{Broker, Peer, Settings};
use roadmesh_server::ca::{Attestation, CaError, CaKey, CertAuthority, device_id};
use roadmesh_server::devca::DeviceCa;
use roadmesh_server::hazard::HazardGate;
use roadmesh_server::monitor::Monitor;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

pub const T0: u64 = 1_800_000_000;

pub fn raw(k: &SigningKey, msg: &[u8]) -> [u8; SIG_LEN] {
    let s: Signature = k.sign(msg);
    let mut o = [0u8; SIG_LEN];
    o.copy_from_slice(&s.to_bytes());
    o
}

pub fn pubkey(k: &SigningKey) -> [u8; PUBLIC_KEY_LEN] {
    let mut o = [0u8; PUBLIC_KEY_LEN];
    o.copy_from_slice(k.verifying_key().to_sec1_point(true).as_bytes());
    o
}

/// Test-only CA key (production: AWS KMS).
pub struct SoftCa(pub SigningKey, pub u8);
impl CaKey for SoftCa {
    fn key_id(&self) -> u8 {
        self.1
    }
    fn public_key(&self) -> [u8; PUBLIC_KEY_LEN] {
        pubkey(&self.0)
    }
    fn sign(&self, m: &[u8]) -> Result<[u8; SIG_LEN], CaError> {
        Ok(raw(&self.0, m))
    }
}

/// Test-only: evidence "genuine" passes.
pub struct TestAttestation;
impl Attestation for TestAttestation {
    fn verify(&self, _: &[u8; PUBLIC_KEY_LEN], _challenge: &[u8], evidence: &[u8]) -> bool {
        evidence == b"genuine"
    }
}

#[derive(Default)]
pub struct SoftKeys(pub HashMap<CertDigest, SigningKey>);
impl KeyStore for SoftKeys {
    fn sign(&mut self, d: &CertDigest, m: &[u8]) -> Option<[u8; SIG_LEN]> {
        self.0.get(d).map(|k| raw(k, m))
    }
}

pub type TestBroker = Broker<SoftCa, SoftCa, TestAttestation>;

/// A broker with a settable clock.
pub fn broker(settings: Settings) -> (Arc<TestBroker>, Arc<AtomicU64>) {
    let clock = Arc::new(AtomicU64::new(T0 * 1000));
    let c = clock.clone();
    let ca = CertAuthority::new(
        SoftCa(SigningKey::generate(), 1),
        TestAttestation,
        Config::spec_v0(),
    );
    let devca =
        DeviceCa::new(SoftCa(SigningKey::generate(), 2)).unwrap_or_else(|e| panic!("{e:?}"));
    let b = Broker::new(
        ca,
        devca,
        HazardGate::new(Config::spec_v0()),
        settings,
        Arc::new(Monitor::capturing()),
        Box::new(move || c.load(Ordering::Relaxed) as f64 / 1000.0),
    );
    (b, clock)
}

pub fn now(clock: &AtomicU64) -> f64 {
    clock.load(Ordering::Relaxed) as f64 / 1000.0
}

/// An MQTT client over an in-memory stream.
pub struct Client {
    pub io: DuplexStream,
    buf: Vec<u8>,
}

impl Client {
    pub async fn send(&mut self, p: &Packet) {
        let _ = self.io.write_all(&mqtt::encode(p)).await;
    }

    /// Next packet, or None on close / 1 s of silence.
    pub async fn recv(&mut self) -> Option<Packet> {
        let mut chunk = [0u8; 4096];
        loop {
            if let Ok(Some((p, n))) = mqtt::decode(&self.buf) {
                self.buf.drain(..n);
                return Some(p);
            }
            let r =
                tokio::time::timeout(std::time::Duration::from_secs(1), self.io.read(&mut chunk))
                    .await;
            match r {
                Ok(Ok(n)) if n > 0 => self.buf.extend_from_slice(&chunk[..n]),
                _ => return None,
            }
        }
    }

    pub async fn publish(&mut self, topic: &str, payload: Vec<u8>) {
        self.send(&Packet::Publish {
            topic: topic.into(),
            payload,
        })
        .await;
    }

    /// The next PUBLISH (skipping other packets).
    pub async fn next_publish(&mut self) -> Option<(String, Vec<u8>)> {
        loop {
            match self.recv().await? {
                Packet::Publish { topic, payload } => return Some((topic, payload)),
                _ => continue,
            }
        }
    }
}

/// Open a session to the broker as `peer`, CONNECT done.
pub async fn connect(b: &Arc<TestBroker>, peer: Peer) -> Client {
    let (a, z) = tokio::io::duplex(64 * 1024);
    tokio::spawn(b.clone().serve(z, peer));
    let mut c = Client {
        io: a,
        buf: Vec::new(),
    };
    c.send(&Packet::Connect {
        client_id: String::new(),
        keep_alive: 60,
    })
    .await;
    assert_eq!(c.recv().await, Some(Packet::ConnAck { code: 0 }));
    c
}

/// A phone: hardware keys simulated in software.
pub struct Phone {
    pub device: SigningKey,
    pending: Vec<SigningKey>,
    pub signer: Option<PseudonymSigner<SoftKeys>>,
    pub device_cert: Vec<u8>,
}

impl Phone {
    pub fn new() -> Self {
        Self {
            device: SigningKey::generate(),
            pending: Vec::new(),
            signer: None,
            device_cert: Vec::new(),
        }
    }

    pub fn id(&self) -> [u8; 32] {
        device_id(&pubkey(&self.device))
    }

    pub fn request(&mut self, n: usize, at: u32, evidence: &[u8]) -> Vec<u8> {
        self.pending = (0..n).map(|_| SigningKey::generate()).collect();
        let keys: Vec<[u8; PUBLIC_KEY_LEN]> = self.pending.iter().map(pubkey).collect();
        let tbs = pki::issue_tbs(&pubkey(&self.device), at, &keys);
        IssueRequestWire {
            device_key: pubkey(&self.device),
            requested_at: at,
            pseudonym_keys: keys,
            attestation: evidence.to_vec(),
            signature: raw(&self.device, &tbs),
        }
        .encode()
        .unwrap_or_else(|| panic!("encode"))
    }

    /// Take an enrolment reply.
    pub fn accept(&mut self, reply: &[u8]) {
        assert_eq!(reply.first(), Some(&0), "enrol refused: {reply:?}");
        let n = usize::from(u16::from_be_bytes([reply[1], reply[2]]));
        self.device_cert = reply[3..3 + n].to_vec();
        let certs: Vec<Certificate> =
            pki::decode_certificates(&reply[3 + n..]).unwrap_or_else(|e| panic!("{e:?}"));
        let mut keys = SoftKeys::default();
        for (c, k) in certs.iter().zip(self.pending.drain(..)) {
            keys.0.insert(c.digest(), k);
        }
        self.signer = Some(PseudonymSigner::new(certs, keys, &Security::v0()));
    }

    /// A signed frame at `now` at (lat, lon).
    pub fn frame(&mut self, now: f64, msg_id: u16, event: bool, lat: f64, lon: f64) -> Frame {
        use roadmesh_core::auth::Signer;
        let s = self
            .signer
            .as_mut()
            .unwrap_or_else(|| panic!("not enrolled"));
        let id = s.current_temp_id(now).unwrap_or_else(|| panic!("id"));
        let body = Rsm {
            msg_type: if event {
                MsgType::Event
            } else {
                MsgType::Heartbeat
            },
            temp_id: id,
            msg_id,
            timestamp_ms: rsm::ms_of_hour(now),
            lat_deg: lat,
            lon_deg: lon,
            speed: 12.0,
            heading_deg: 90.0,
            accel: if event { -6.0 } else { 0.0 },
            yaw_rate: 0.0,
            vehicle_type: VehicleType::Car,
            flags: 0,
            pos_accuracy_m: 3.0,
            hop_count: 0,
            ttl: if event { 5 } else { 2 },
            origin_pos_age_ms: 0,
            event: if event {
                EventCode::HardBrake
            } else {
                EventCode::None
            },
        }
        .encode();
        let auth = s.sign(now, true, &body).unwrap_or_else(|| panic!("sig"));
        Frame {
            last_hop: None,
            body,
            auth,
        }
    }
}

/// Enrol `phone` through the broker's enrolment port.
pub async fn enrol(b: &Arc<TestBroker>, clock: &AtomicU64, phone: &mut Phone, ip: u8) {
    let mut c = connect(b, Peer::Enrol([10, 0, 0, ip].into())).await;
    let req = phone.request(20, now(clock) as u32, b"genuine");
    c.publish(roadmesh_server::broker::ENROL, req).await;
    let (topic, reply) = c.next_publish().await.unwrap_or_else(|| panic!("no reply"));
    assert_eq!(topic, roadmesh_server::broker::ENROL_REPLY);
    phone.accept(&reply);
}
