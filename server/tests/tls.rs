//! TLS for the edge server (AUDIT S-16, S-17): TLS 1.3 only, mutual TLS on the MQTT
//! port, certificate pinning in the client, and the man-in-the-middle test from the
//! BUILD_PLAN Step 9 "done when": a proxy with a certificate from a CA the phone
//! trusts is still refused because its key does not match the pin.
#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use common::*;
use p256::ecdsa::SigningKey;
use p256::elliptic_curve::Generate;
use p256::pkcs8::EncodePrivateKey;
use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair};
use roadmesh_core::geohash;
use roadmesh_core::mqtt::{self, Packet};
use roadmesh_core::rsm::Frame;
use roadmesh_server::broker::{self, Settings};
use roadmesh_server::serve::{Port, accept_loop};
use roadmesh_server::tls::{ALPN, enrol_config, mqtt_config, spki_sha256};
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsConnector;

const HOST: &str = "edge.roadmesh.test";

/// A public-looking CA and leaf certificates for the server and an attacker.
struct Pki {
    root: CertificateDer<'static>,
    root_issuer: Issuer<'static, KeyPair>,
}

impl Pki {
    fn new() -> Self {
        let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
        p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let key = KeyPair::generate().unwrap();
        let root = p.self_signed(&key).unwrap().der().clone();
        Self {
            root,
            root_issuer: Issuer::new(p, key),
        }
    }

    fn leaf(&self) -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
        let key = KeyPair::generate().unwrap();
        let cert = CertificateParams::new(vec![HOST.to_string()])
            .unwrap()
            .signed_by(&key, &self.root_issuer)
            .unwrap();
        (
            vec![cert.der().clone()],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
        )
    }
}

/// The app's verifier: normal chain validation, then the SPKI pin (current + backup).
#[derive(Debug)]
struct Pinned {
    inner: Arc<WebPkiServerVerifier>,
    pins: Vec<[u8; 32]>,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        self.inner
            .verify_server_cert(end_entity, intermediates, server_name, ocsp, now)?;
        match spki_sha256(end_entity) {
            Some(h) if self.pins.contains(&h) => Ok(ServerCertVerified::assertion()),
            _ => Err(rustls::Error::General("certificate pin mismatch".into())),
        }
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(m, c, d)
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(m, c, d)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// A phone's TLS client: pinned to `pins`, optionally with a device certificate.
fn client(
    pki: &Pki,
    pins: Vec<[u8; 32]>,
    device: Option<(&SigningKey, &[u8])>,
    versions: &[&'static rustls::SupportedProtocolVersion],
) -> TlsConnector {
    let mut roots = RootCertStore::empty();
    roots.add(pki.root.clone()).unwrap();
    let inner = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider())
        .build()
        .unwrap();
    let b = ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(versions)
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned { inner, pins }));
    let mut cfg = match device {
        Some((key, cert)) => {
            let der = key.to_pkcs8_der().unwrap();
            b.with_client_auth_cert(
                vec![CertificateDer::from(cert.to_vec())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(der.as_bytes().to_vec())),
            )
            .unwrap()
        }
        None => b.with_no_client_auth(),
    };
    cfg.alpn_protocols = vec![ALPN.to_vec()];
    TlsConnector::from(Arc::new(cfg))
}

struct Server {
    _b: Arc<TestBroker>,
    clock: Arc<std::sync::atomic::AtomicU64>,
    mqtt: std::net::SocketAddr,
    enrol: std::net::SocketAddr,
    pin: [u8; 32],
    pki: Pki,
}

async fn start() -> Server {
    let (b, clock) = broker(Settings::default());
    // rustls checks certificate validity against the real time
    let real = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    clock.store(real, std::sync::atomic::Ordering::Relaxed);
    let pki = Pki::new();
    let (chain, key) = pki.leaf();
    let pin = spki_sha256(&chain[0]).unwrap();
    let m = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let e = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (mqtt, enrol) = (m.local_addr().unwrap(), e.local_addr().unwrap());
    let mcfg = mqtt_config(chain.clone(), key.clone_key(), &b.device_root()).unwrap();
    let ecfg = enrol_config(chain, key).unwrap();
    tokio::spawn(accept_loop(b.clone(), m, mcfg, Port::Mqtt));
    tokio::spawn(accept_loop(b.clone(), e, ecfg, Port::Enrol));
    Server {
        _b: b,
        clock,
        mqtt,
        enrol,
        pin,
        pki,
    }
}

type Tls = tokio_rustls::client::TlsStream<TcpStream>;

async fn dial(c: &TlsConnector, addr: std::net::SocketAddr) -> Result<Tls, std::io::Error> {
    let tcp = TcpStream::connect(addr).await?;
    c.connect(ServerName::try_from(HOST).unwrap(), tcp).await
}

async fn send(s: &mut Tls, p: &Packet) {
    let _ = s.write_all(&mqtt::encode(p)).await;
    let _ = s.flush().await;
}

/// Next packet within 2 s; None on close, error or silence.
async fn recv(s: &mut Tls, buf: &mut Vec<u8>) -> Option<Packet> {
    let mut chunk = [0u8; 4096];
    loop {
        if let Ok(Some((p, n))) = mqtt::decode(buf) {
            buf.drain(..n);
            return Some(p);
        }
        match tokio::time::timeout(std::time::Duration::from_secs(2), s.read(&mut chunk)).await {
            Ok(Ok(n)) if n > 0 => buf.extend_from_slice(&chunk[..n]),
            _ => return None,
        }
    }
}

async fn mqtt_connect(s: &mut Tls, buf: &mut Vec<u8>) -> Option<Packet> {
    send(
        s,
        &Packet::Connect {
            client_id: String::new(),
            keep_alive: 60,
        },
    )
    .await;
    recv(s, buf).await
}

/// Enrol over TLS and connect to the MQTT port with the new device certificate.
async fn enrolled(srv: &Server, phone: &mut Phone) -> (Tls, Vec<u8>) {
    let c = client(&srv.pki, vec![srv.pin], None, &[&rustls::version::TLS13]);
    let mut s = dial(&c, srv.enrol).await.unwrap();
    let mut buf = Vec::new();
    assert_eq!(
        mqtt_connect(&mut s, &mut buf).await,
        Some(Packet::ConnAck { code: 0 })
    );
    let req = phone.request(20, now(&srv.clock) as u32, b"genuine");
    send(
        &mut s,
        &Packet::Publish {
            topic: broker::ENROL.into(),
            payload: req,
        },
    )
    .await;
    let Some(Packet::Publish { payload, .. }) = recv(&mut s, &mut buf).await else {
        panic!("no enrol reply")
    };
    phone.accept(&payload);
    let c = client(
        &srv.pki,
        vec![srv.pin],
        Some((&phone.device, &phone.device_cert)),
        &[&rustls::version::TLS13],
    );
    let mut s = dial(&c, srv.mqtt).await.unwrap();
    let mut buf = Vec::new();
    assert_eq!(
        mqtt_connect(&mut s, &mut buf).await,
        Some(Packet::ConnAck { code: 0 })
    );
    (s, buf)
}

#[tokio::test]
async fn hazard_event_crosses_the_server_end_to_end_over_mutual_tls() {
    let srv = start().await;
    let (mut a, mut z) = (Phone::new(), Phone::new());
    let (mut up, _) = enrolled(&srv, &mut a).await;
    let (mut down, mut dbuf) = enrolled(&srv, &mut z).await;
    let pp = (11.5564, 104.9282);
    let cell = geohash::encode(pp.0, pp.1, geohash::LEVEL).unwrap();
    send(
        &mut down,
        &Packet::Subscribe {
            packet_id: 1,
            filters: vec![format!("{}{cell}", broker::EVENTS)],
        },
    )
    .await;
    assert!(matches!(
        recv(&mut down, &mut dbuf).await,
        Some(Packet::SubAck { .. })
    ));
    let ev = a.frame(now(&srv.clock), 9, true, pp.0, pp.1);
    send(
        &mut up,
        &Packet::Publish {
            topic: broker::UP.into(),
            payload: ev.encode().unwrap(),
        },
    )
    .await;
    let Some(Packet::Publish { payload, .. }) = recv(&mut down, &mut dbuf).await else {
        panic!("event not delivered")
    };
    assert_eq!(Frame::decode(&payload).unwrap().body, ev.body);
}

#[tokio::test]
async fn mqtt_port_refuses_phones_without_a_device_certificate() {
    let srv = start().await;
    let c = client(&srv.pki, vec![srv.pin], None, &[&rustls::version::TLS13]);
    // TLS 1.3: the client may finish its side before the server's alert arrives
    if let Ok(mut s) = dial(&c, srv.mqtt).await {
        assert_eq!(mqtt_connect(&mut s, &mut Vec::new()).await, None);
    }
}

#[tokio::test]
async fn mqtt_port_refuses_certificates_from_another_device_ca() {
    let srv = start().await;
    let other = start().await;
    let mut p = Phone::new();
    let _ = enrolled(&other, &mut p).await; // p's device certificate is from `other`
    let c = client(
        &srv.pki,
        vec![srv.pin],
        Some((&p.device, &p.device_cert)),
        &[&rustls::version::TLS13],
    );
    if let Ok(mut s) = dial(&c, srv.mqtt).await {
        assert_eq!(mqtt_connect(&mut s, &mut Vec::new()).await, None);
    }
}

#[tokio::test]
async fn tls_1_2_is_refused_on_both_ports() {
    let srv = start().await;
    let c = client(&srv.pki, vec![srv.pin], None, &[&rustls::version::TLS12]);
    assert!(dial(&c, srv.enrol).await.is_err());
    assert!(dial(&c, srv.mqtt).await.is_err());
}

/// A man-in-the-middle proxy: terminates TLS with its own certificate and would
/// forward to the real server.
async fn mitm(
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
    upstream: std::net::SocketAddr,
) -> std::net::SocketAddr {
    let cfg = enrol_config(chain, key).unwrap();
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        let acceptor = tokio_rustls::TlsAcceptor::from(cfg);
        while let Ok((tcp, _)) = l.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                if let Ok(mut client_side) = acceptor.accept(tcp).await
                    && let Ok(mut server_side) = TcpStream::connect(upstream).await
                {
                    let _ = tokio::io::copy_bidirectional(&mut client_side, &mut server_side).await;
                }
            });
        }
    });
    addr
}

#[tokio::test]
async fn man_in_the_middle_with_a_trusted_ca_certificate_fails_the_pin() {
    let srv = start().await;
    // the attacker holds a valid certificate for the host from a CA the phone trusts
    let (chain, key) = srv.pki.leaf();
    let proxy = mitm(chain, key, srv.enrol).await;
    let c = client(&srv.pki, vec![srv.pin], None, &[&rustls::version::TLS13]);
    let err = dial(&c, proxy)
        .await
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(err.contains("pin"), "{err}");
    // the same phone reaches the real server
    assert!(dial(&c, srv.enrol).await.is_ok());
    // with the backup pin configured (key rotation), the real server still passes
    let backup = spki_sha256(&srv.pki.leaf().0[0]).unwrap();
    let c2 = client(
        &srv.pki,
        vec![backup, srv.pin],
        None,
        &[&rustls::version::TLS13],
    );
    assert!(dial(&c2, srv.enrol).await.is_ok());
}

#[tokio::test]
async fn man_in_the_middle_with_an_untrusted_certificate_fails() {
    let srv = start().await;
    let rogue = Pki::new();
    let (chain, key) = rogue.leaf();
    let proxy = mitm(chain, key, srv.enrol).await;
    let c = client(&srv.pki, vec![srv.pin], None, &[&rustls::version::TLS13]);
    assert!(dial(&c, proxy).await.is_err());
}

#[test]
fn private_keys_are_never_printed() {
    // the device key type does not expose its scalar in Debug output
    let k = SigningKey::generate();
    let d = format!("{k:?}");
    assert!(!d.contains(&format!("{:x?}", k.to_bytes().as_slice())));
}
