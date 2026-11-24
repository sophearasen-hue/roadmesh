//! The edge broker (BUILD_PLAN Step 9): MQTT sessions over TLS, topic rules, the
//! hazard gate, certificate enrolment, misbehaviour reports, revocation list and
//! "delete my data". Every byte from a phone is parsed by the core's MQTT codec and
//! frame decoders; nothing unverified is passed on.
//!
//! Topics (all others close the session):
//!
//! | Topic | Who | What |
//! |---|---|---|
//! | `rm/v1/up` | phone → server | one radio frame (own message or an event heard) |
//! | `rm/v1/ev/<cell>` | server → phones | a verified hazard event in geohash cell `<cell>` |
//! | `rm/v1/crl` | server → phones | the signed revocation list (sent on subscribe and on change) |
//! | `rm/v1/report` | phone → server | a signed misbehaviour report |
//! | `rm/v1/delete` | phone → server | "delete my data" |
//! | `rm/v1/reply/<what>` | server → that phone | the outcome of a report or delete |
//! | `rm/v1/enrol` | phone → server | certificate request (enrolment port only) |
//! | `rm/v1/enrol/reply` | server → that phone | device certificate + pseudonym certificates |
//!
//! Subscriptions must name cells exactly (no wildcards) and at most
//! [`Settings::max_cells`] of them, so no phone can watch a whole city.

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use roadmesh_core::geohash;
use roadmesh_core::mqtt::{self, MAX_PACKET, Packet};
use roadmesh_core::pki;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::ca::{
    Attestation, CaKey, CertAuthority, DeviceId, IssueError, IssueRequest, ReportOutcome,
};
use crate::devca::DeviceCa;
use crate::hazard::{HazardGate, Upload};
use crate::limits::{KeyedLimiter, TokenBucket};
use crate::monitor::{Metric, Monitor, device_ref};

pub use roadmesh_core::mqtt::topic::{
    CRL, DELETE, DELETE_REPLY, ENROL, ENROL_REPLY, EVENTS, REPORT, REPORT_REPLY, UP,
};

/// Limits.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Open sessions at most.
    pub max_sessions: usize,
    /// Publishes per second per session (own messages at 10 Hz plus events heard).
    pub publish_rate: f64,
    /// Burst of publishes.
    pub publish_burst: f64,
    /// A session that keeps exceeding the rate is closed after this many drops.
    pub max_drops: u32,
    /// Enrolment attempts per minute per source address.
    pub enrol_per_min: f64,
    /// Event cells one session may subscribe to (own cell and the 8 around it).
    pub max_cells: usize,
    /// CONNECT must arrive within this time (s).
    pub connect_timeout_s: u64,
    /// Longest keep-alive accepted (s); 0 from the phone means this.
    pub max_keep_alive_s: u16,
    /// Pushes queued per session before new ones are dropped.
    pub queue: usize,
    /// Revocation list validity (s).
    pub crl_validity_s: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_sessions: 50_000,
            publish_rate: 25.0,
            publish_burst: 50.0,
            max_drops: 200,
            enrol_per_min: 5.0,
            max_cells: 9,
            connect_timeout_s: 10,
            max_keep_alive_s: 300,
            queue: 256,
            crl_validity_s: 3600,
        }
    }
}

/// Who is on the other end, from the TLS layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Peer {
    /// A device authenticated by its client certificate (MQTT port).
    Device(DeviceId),
    /// An unauthenticated phone asking for certificates (enrolment port).
    Enrol(IpAddr),
}

struct State<K, D, A> {
    ca: CertAuthority<K, A>,
    devca: DeviceCa<D>,
    gate: HazardGate,
    subs: HashMap<String, HashSet<u64>>,
    sessions: HashMap<u64, mpsc::Sender<Arc<Vec<u8>>>>,
    crl: Arc<Vec<u8>>,
    crl_at: u32,
    crl_serial: Option<u32>,
    enrol_limit: KeyedLimiter<IpAddr>,
}

/// Shared server state.
pub struct Broker<K, D, A> {
    state: Mutex<State<K, D, A>>,
    settings: Settings,
    /// Counters and audit log.
    pub monitor: Arc<Monitor>,
    clock: Box<dyn Fn() -> f64 + Send + Sync>,
    next_id: AtomicU64,
}

fn publish(topic: &str, payload: Vec<u8>) -> Arc<Vec<u8>> {
    Arc::new(mqtt::encode(&Packet::Publish {
        topic: topic.into(),
        payload,
    }))
}

/// Answer codes in reply payloads (first byte).
pub mod code {
    /// Done.
    pub const OK: u8 = 0;
    /// Misbehaviour report: the accused device is now revoked.
    pub const REVOKED: u8 = 1;
    /// Misbehaviour report: it was already revoked.
    pub const ALREADY_REVOKED: u8 = 2;
    /// Refused (malformed, unverified, over quota, ...).
    pub const REFUSED: u8 = 0x80;
    /// Enrolment: the device is banned.
    pub const BANNED: u8 = 0x81;
    /// Enrolment: attestation failed.
    pub const NOT_GENUINE: u8 = 0x82;
    /// Enrolment: certificate quota used up for this week.
    pub const QUOTA: u8 = 0x83;
    /// Try again later (rate limit, CA unavailable).
    pub const LATER: u8 = 0x84;
}

impl<K, D, A> Broker<K, D, A>
where
    K: CaKey + Send + 'static,
    D: CaKey + Send + 'static,
    A: Attestation + Send + 'static,
{
    /// A broker around a CA and a device CA. `clock` gives Unix time (s).
    pub fn new(
        ca: CertAuthority<K, A>,
        devca: DeviceCa<D>,
        gate: HazardGate,
        settings: Settings,
        monitor: Arc<Monitor>,
        clock: Box<dyn Fn() -> f64 + Send + Sync>,
    ) -> Arc<Self> {
        let enrol_limit = KeyedLimiter::new(
            settings.enrol_per_min / 60.0,
            settings.enrol_per_min,
            100_000,
        );
        Arc::new(Self {
            state: Mutex::new(State {
                ca,
                devca,
                gate,
                subs: HashMap::new(),
                sessions: HashMap::new(),
                crl: Arc::new(Vec::new()),
                crl_at: 0,
                crl_serial: None,
                enrol_limit,
            }),
            settings,
            monitor,
            clock,
            next_id: AtomicU64::new(1),
        })
    }

    fn now(&self) -> f64 {
        (self.clock)()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State<K, D, A>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Is this device refused service? (Checked right after the TLS handshake.)
    pub fn is_banned(&self, d: &DeviceId) -> bool {
        self.lock().ca.is_banned(d)
    }

    /// The device CA's root certificate (DER), for the TLS client verifier.
    pub fn device_root(&self) -> Vec<u8> {
        self.lock().devca.root_der().to_vec()
    }

    /// The pseudonym CA's trust anchor (key id ‖ compressed key), built into the app.
    pub fn trust_anchor(&self) -> Vec<u8> {
        let a = self.lock().ca.trust_anchor();
        let mut v = vec![a.key_id];
        v.extend_from_slice(&a.public_key);
        v
    }

    /// Housekeeping, once a minute (may block on the KMS: call from a blocking
    /// thread): prune expired certificates, re-sign the revocation list when it changed
    /// or half its validity has passed and push it, write the metrics line.
    pub fn tick(&self) {
        let now = self.now();
        self.lock().ca.prune(now as u32, 7 * 86_400);
        self.refresh_crl(false);
        self.monitor.minute_tick(now as u64);
    }

    /// Re-sign and push the revocation list if needed (`force`: always).
    fn refresh_crl(&self, force: bool) {
        let now_u = self.now() as u32;
        let p = {
            let mut s = self.lock();
            let due = force
                || s.crl_serial != Some(s.ca.crl_serial())
                || now_u.saturating_sub(s.crl_at) > self.settings.crl_validity_s / 2;
            if !due {
                return;
            }
            let Ok(list) =
                s.ca.revocation_list(now_u, now_u + self.settings.crl_validity_s)
            else {
                return; // KMS unavailable: phones keep the previous list until it expires
            };
            s.crl_serial = Some(s.ca.crl_serial());
            s.crl = Arc::new(list.clone());
            s.crl_at = now_u;
            publish(CRL, list)
        };
        self.fan_out(CRL, &p);
    }

    fn fan_out(&self, topic: &str, packet: &Arc<Vec<u8>>) {
        let targets: Vec<mpsc::Sender<Arc<Vec<u8>>>> = {
            let s = self.lock();
            s.subs
                .get(topic)
                .map(|ids| {
                    ids.iter()
                        .filter_map(|id| s.sessions.get(id).cloned())
                        .collect()
                })
                .unwrap_or_default()
        };
        for t in targets {
            if t.try_send(packet.clone()).is_ok() {
                if topic != CRL {
                    self.monitor.inc(Metric::EventsPushed);
                }
            } else {
                self.monitor.inc(Metric::SlowConsumers);
            }
        }
    }

    /// Serve one session until it ends.
    pub async fn serve<S>(self: Arc<Self>, stream: S, peer: Peer)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, mut rx) = mpsc::channel::<Arc<Vec<u8>>>(self.settings.queue);
        let refused = {
            let mut s = self.lock();
            let banned = matches!(peer, Peer::Device(d) if s.ca.is_banned(&d));
            if banned || s.sessions.len() >= self.settings.max_sessions {
                true
            } else {
                s.sessions.insert(id, tx.clone());
                false
            }
        };
        if refused {
            self.monitor.inc(Metric::SessionsRefused);
            return;
        }
        let (mut rd, mut wr) = tokio::io::split(stream);
        let writer = tokio::spawn(async move {
            while let Some(p) = rx.recv().await {
                if wr.write_all(&p).await.is_err() {
                    break;
                }
            }
            let _ = wr.shutdown().await;
        });
        let _ = self.session(&mut rd, &tx, id, peer).await;
        // forget the session and its subscriptions
        {
            let mut s = self.lock();
            s.sessions.remove(&id);
            s.subs.retain(|_, ids| {
                ids.remove(&id);
                !ids.is_empty()
            });
        }
        drop(tx);
        let _ = writer.await;
    }

    async fn session<R: AsyncRead + Unpin>(
        self: &Arc<Self>,
        rd: &mut R,
        tx: &mpsc::Sender<Arc<Vec<u8>>>,
        id: u64,
        peer: Peer,
    ) -> Result<(), ()> {
        let mut buf: Vec<u8> = Vec::with_capacity(1024);
        let mut chunk = [0u8; 2048];
        // CONNECT first, within the timeout
        let connect = Duration::from_secs(self.settings.connect_timeout_s);
        let (first, used) = read_packet(rd, &mut buf, &mut chunk, connect).await?;
        buf.drain(..used);
        let Packet::Connect { keep_alive, .. } = first else {
            self.monitor.inc(Metric::SessionsRefused);
            return Err(());
        };
        tx.send(Arc::new(mqtt::encode(&Packet::ConnAck { code: 0 })))
            .await
            .map_err(|_| ())?;
        self.monitor.inc(Metric::Sessions);
        let ka = match keep_alive {
            0 => self.settings.max_keep_alive_s,
            k => k.min(self.settings.max_keep_alive_s),
        };
        let idle = Duration::from_millis(u64::from(ka) * 1500);
        let mut bucket = TokenBucket::new(
            self.settings.publish_rate,
            self.settings.publish_burst,
            self.now(),
        );
        let mut drops = 0u32;
        let mut cells: HashSet<String> = HashSet::new();
        loop {
            let (p, used) = read_packet(rd, &mut buf, &mut chunk, idle).await?;
            buf.drain(..used);
            match p {
                Packet::PingReq => {
                    tx.send(Arc::new(mqtt::encode(&Packet::PingResp)))
                        .await
                        .map_err(|_| ())?;
                }
                Packet::Disconnect => return Ok(()),
                Packet::Publish { topic, payload } => {
                    if !bucket.take(self.now()) {
                        self.monitor.inc(Metric::RateLimited);
                        drops += 1;
                        if drops > self.settings.max_drops {
                            return Err(());
                        }
                        continue;
                    }
                    let reply = if topic == UP {
                        self.on_publish(peer, &topic, payload)?
                    } else {
                        // enrolment, reports and deletes may wait on the KMS
                        let me = self.clone();
                        tokio::task::spawn_blocking(move || me.on_publish(peer, &topic, payload))
                            .await
                            .map_err(|_| ())??
                    };
                    if let Some(r) = reply {
                        tx.send(r).await.map_err(|_| ())?;
                    }
                }
                Packet::Subscribe { packet_id, filters } => {
                    let Peer::Device(_) = peer else {
                        return Err(()); // enrolment sessions only enrol
                    };
                    let mut codes = Vec::with_capacity(filters.len());
                    let mut send_crl = false;
                    for f in &filters {
                        let ok = if f == CRL {
                            send_crl = true;
                            true
                        } else if let Some(cell) = f.strip_prefix(EVENTS) {
                            let room =
                                cells.contains(cell) || cells.len() < self.settings.max_cells;
                            if geohash::valid_cell(cell) && room {
                                cells.insert(cell.to_owned());
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        };
                        if ok {
                            self.lock().subs.entry(f.clone()).or_default().insert(id);
                        }
                        codes.push(if ok { 0 } else { 0x80 });
                    }
                    tx.send(Arc::new(mqtt::encode(&Packet::SubAck { packet_id, codes })))
                        .await
                        .map_err(|_| ())?;
                    if send_crl {
                        let crl = self.lock().crl.clone();
                        if !crl.is_empty() {
                            tx.send(publish(CRL, crl.to_vec())).await.map_err(|_| ())?;
                        }
                    }
                }
                Packet::Unsubscribe { packet_id, filters } => {
                    {
                        let mut s = self.lock();
                        for f in &filters {
                            if let Some(ids) = s.subs.get_mut(f) {
                                ids.remove(&id);
                            }
                            if let Some(cell) = f.strip_prefix(EVENTS) {
                                cells.remove(cell);
                            }
                        }
                    }
                    tx.send(Arc::new(mqtt::encode(&Packet::UnsubAck { packet_id })))
                        .await
                        .map_err(|_| ())?;
                }
                // a phone never sends these
                Packet::Connect { .. }
                | Packet::ConnAck { .. }
                | Packet::SubAck { .. }
                | Packet::UnsubAck { .. }
                | Packet::PingResp => return Err(()),
            }
        }
    }

    /// Handle a publish; `Err` closes the session (topic not allowed).
    fn on_publish(
        &self,
        peer: Peer,
        topic: &str,
        payload: Vec<u8>,
    ) -> Result<Option<Arc<Vec<u8>>>, ()> {
        let now = self.now();
        let now_u = now as u32;
        match (peer, topic) {
            (Peer::Device(_), UP) => {
                self.monitor.inc(Metric::Uploads);
                let up = {
                    let mut s = self.lock();
                    let State { ca, gate, .. } = &mut *s;
                    gate.check(ca, &payload, now)
                };
                match up {
                    Upload::Event { cell, frame } => {
                        let t = format!("{EVENTS}{cell}");
                        self.fan_out(&t, &publish(&t, frame));
                    }
                    Upload::Heartbeat => {}
                    Upload::Rejected(_) => self.monitor.inc(Metric::UploadsRejected),
                }
                Ok(None)
            }
            (Peer::Device(dev), REPORT) => {
                let out = self.lock().ca.report(&payload, now_u);
                let c = match out {
                    Ok(ReportOutcome::Counted(_)) => code::OK,
                    Ok(ReportOutcome::Revoked) => code::REVOKED,
                    Ok(ReportOutcome::AlreadyRevoked) => code::ALREADY_REVOKED,
                    Err(_) => code::REFUSED,
                };
                if out.is_ok() {
                    self.monitor.inc(Metric::Reports);
                } else {
                    self.monitor.inc(Metric::ReportsRefused);
                }
                if out == Ok(ReportOutcome::Revoked) {
                    self.monitor.inc(Metric::Revocations);
                    self.monitor
                        .audit(now as u64, "revoked", &[("reporter", device_ref(&dev))]);
                    self.refresh_crl(true);
                }
                Ok(Some(publish(REPORT_REPLY, vec![c])))
            }
            (Peer::Device(dev), DELETE) => {
                self.lock().ca.forget_device(&dev);
                self.monitor.inc(Metric::Deletes);
                self.monitor
                    .audit(now as u64, "delete", &[("device", device_ref(&dev))]);
                Ok(Some(publish(DELETE_REPLY, vec![code::OK])))
            }
            (Peer::Enrol(ip), ENROL) => {
                Ok(Some(publish(ENROL_REPLY, self.enrol(ip, &payload, now))))
            }
            _ => Err(()),
        }
    }

    /// A certificate request: attestation, signatures and quota are checked by the CA.
    /// Answer: `[0][u16 len][device certificate DER][pseudonym certificates]` or `[code]`.
    fn enrol(&self, ip: IpAddr, payload: &[u8], now: f64) -> Vec<u8> {
        let now_u = now as u32;
        let mut s = self.lock();
        if !s.enrol_limit.take(&ip, now) {
            drop(s);
            self.monitor.inc(Metric::RateLimited);
            return vec![code::LATER];
        }
        let req = match IssueRequest::decode(payload) {
            Ok(r) => r,
            Err(_) => {
                drop(s);
                self.monitor.inc(Metric::EnrolRefused);
                return vec![code::REFUSED];
            }
        };
        let issued = s.ca.issue(&req, now_u);
        let dev = crate::ca::device_id(&req.device_key);
        let result = issued.and_then(|certs| {
            let der = s
                .devca
                .issue(&req.device_key, i64::from(now_u))
                .map_err(|_| IssueError::Ca(crate::ca::CaError::Unavailable))?;
            Ok((der, certs))
        });
        drop(s);
        match result {
            Ok((der, certs)) => {
                self.monitor.inc(Metric::EnrolOk);
                self.monitor.audit(
                    now as u64,
                    "enrolled",
                    &[
                        ("device", device_ref(&dev)),
                        ("certs", certs.len().to_string()),
                    ],
                );
                let mut out = vec![code::OK];
                out.extend_from_slice(&u16::try_from(der.len()).unwrap_or(0).to_be_bytes());
                out.extend_from_slice(&der);
                out.extend_from_slice(&pki::encode_certificates(&certs));
                out
            }
            Err(e) => {
                self.monitor.inc(Metric::EnrolRefused);
                self.monitor.audit(
                    now as u64,
                    "enrol_refused",
                    &[("device", device_ref(&dev)), ("why", format!("{e:?}"))],
                );
                vec![match e {
                    IssueError::Banned => code::BANNED,
                    IssueError::AttestationFailed => code::NOT_GENUINE,
                    IssueError::QuotaExceeded { .. } => code::QUOTA,
                    IssueError::Ca(_) => code::LATER,
                    _ => code::REFUSED,
                }]
            }
        }
    }
}

/// Read until one whole packet is buffered; `Err` on timeout, EOF or a bad packet.
async fn read_packet<R: AsyncRead + Unpin>(
    rd: &mut R,
    buf: &mut Vec<u8>,
    chunk: &mut [u8],
    timeout: Duration,
) -> Result<(Packet, usize), ()> {
    loop {
        match mqtt::decode(buf) {
            Ok(Some(p)) => return Ok(p),
            Ok(None) => {}
            Err(_) => return Err(()),
        }
        if buf.len() >= MAX_PACKET {
            return Err(());
        }
        let n = tokio::time::timeout(timeout, rd.read(chunk))
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}
