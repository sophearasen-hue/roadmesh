//! RoadMesh core for the phone apps (BUILD_PLAN Step 5).
//!
//! UniFFI generates the Kotlin (Android) and Swift (iOS) bindings from this file.
//! The safety engine runs inside the platform's background service, next to the
//! radio, so it keeps working while the screen is off; the Flutter UI only shows
//! what the service reports. All safety logic stays in `roadmesh-core`.
//!
//! Private keys never enter Rust: the platform implements [`HardwareKeys`] with
//! keys generated in the Secure Enclave / Android Keystore (AUDIT S-02).
//! Untrusted radio bytes are parsed only by the core (AUDIT S-09).

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use roadmesh_core::auth::{Signer, Verifier};
use roadmesh_core::behavior::{EventKind, Profile, SpeedLimits, Subject};
use roadmesh_core::config::Config;
use roadmesh_core::geo::{GeoOrigin, compass_deg_to_math, math_to_compass_deg};
use roadmesh_core::obu::{self, Obu, OwnState, RoadContext, RxOutcome};
use roadmesh_core::pki::{
    self, CertDigest, CertVerifier, IssueRequestWire, KeyStore, MisbehaviourReport, PUBLIC_KEY_LEN,
    PseudonymSigner, ReportReason, SIG_LEN, TrustAnchor,
};
use roadmesh_core::prediction::Motion;
use roadmesh_core::rsm::VehicleType;

uniffi::setup_scaffolding!();

mod cellular;
pub use cellular::{CellularLink, Enrolled, LinkEvent, parse_enrol_reply};

/// Errors returned to the app.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum RoadMeshError {
    /// Malformed input from the app (wrong length, bad key, ...).
    BadInput {
        /// What was wrong.
        detail: String,
    },
    /// A certificate or list failed verification.
    Rejected {
        /// Why.
        detail: String,
    },
}

impl fmt::Display for RoadMeshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadInput { detail } => write!(f, "bad input: {detail}"),
            Self::Rejected { detail } => write!(f, "rejected: {detail}"),
        }
    }
}

impl std::error::Error for RoadMeshError {}

fn bad(detail: &str) -> RoadMeshError {
    RoadMeshError::BadInput {
        detail: detail.to_owned(),
    }
}

/// The phone's secure key storage, implemented by the platform.
#[uniffi::export(with_foreign)]
pub trait HardwareKeys: Send + Sync {
    /// ECDSA P-256 / SHA-256 signature, raw r‖s (64 bytes), over `message` with the
    /// private key of the pseudonym certificate whose 8-byte digest is `cert_digest`.
    /// Return an empty vector if that key is not available.
    fn sign(&self, cert_digest: Vec<u8>, message: Vec<u8>) -> Vec<u8>;
}

#[derive(Clone)]
struct Keys(Arc<dyn HardwareKeys>);

impl fmt::Debug for Keys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HardwareKeys")
    }
}

impl KeyStore for Keys {
    fn sign(&mut self, d: &CertDigest, message: &[u8]) -> Option<[u8; SIG_LEN]> {
        <[u8; SIG_LEN]>::try_from(self.0.sign(d.to_vec(), message.to_vec()).as_slice()).ok()
    }
}

/// One position fix of this phone.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct OwnFix {
    /// Latitude (degrees).
    pub lat: f64,
    /// Longitude (degrees).
    pub lon: f64,
    /// Course over ground, compass degrees (0 = north, clockwise).
    pub heading_deg: f64,
    /// Speed (m/s).
    pub speed_mps: f64,
    /// Longitudinal acceleration (m/s², negative = braking).
    pub accel_mps2: f64,
    /// Turn rate (degrees/s, counter-clockwise positive, as the gyroscope's z axis).
    pub yaw_rate_dps: f64,
    /// Estimated horizontal accuracy (m, 68 %).
    pub accuracy_m: f64,
    /// Vehicle type code (SPEC §3): 0 car, 1 motorbike, 2 tuk-tuk, 3 truck, 4 bus, 5 pedestrian, 6 bicycle.
    pub vehicle_type: u8,
}

/// Map context for this phone.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct RoadInfo {
    /// Posted speed limit here (m/s), if known.
    pub speed_limit_mps: Option<f64>,
    /// Near a junction.
    pub at_junction: bool,
}

/// Alert level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AlertLevel {
    /// Map highlight only.
    Caution,
    /// Amber banner, single tone.
    Warning,
    /// Red banner, chime and spoken alert.
    Danger,
}

/// One alert for the driver. `kind` and `advice` are stable keys the app maps to
/// its English/Khmer strings (no user-facing text here).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AlertInfo {
    /// Alert kind key, e.g. "CrossingConflict".
    pub kind: String,
    /// Level.
    pub level: AlertLevel,
    /// Driver instruction key: "None", "EaseOff", "BrakeGently", "BrakeFirmly", "BrakeHard".
    pub advice: String,
    /// Gentlest deceleration that avoids the conflict (m/s², ≥ 0). Advice only.
    pub advised_decel_mps2: f64,
    /// Distance to the other vehicle or hazard (m).
    pub distance_m: Option<f64>,
    /// Predicted time to conflict (s).
    pub ttc_s: Option<f64>,
    /// Relays the information came through.
    pub hops: u8,
    /// The other vehicle's current pseudonym (to ring it on the map), if any.
    pub target: Option<Vec<u8>>,
}

/// The engine's decision for this cycle.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DecisionInfo {
    /// Alerts, most important first. Show `alerts[0]`.
    pub alerts: Vec<AlertInfo>,
    /// Advised acceleration limit (m/s², ≤ 0), if any. Advice only: the app never brakes.
    pub advised_accel_mps2: Option<f64>,
    /// Pseudonyms of every vehicle the advice accounts for.
    pub avoiding: Vec<Vec<u8>>,
}

/// A vehicle heard by this phone (smoothed and map-matched), relative to me.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PeerInfo {
    /// Current pseudonym (changes every few minutes; for the map only, never log it).
    pub pseudonym: Vec<u8>,
    /// Metres east of me.
    pub east_m: f64,
    /// Metres north of me.
    pub north_m: f64,
    /// Distance (m).
    pub distance_m: f64,
    /// Speed (m/s).
    pub speed_mps: f64,
    /// Reported acceleration (m/s², negative when braking).
    pub accel_mps2: f64,
    /// Compass heading (degrees).
    pub heading_deg: f64,
    /// Vehicle type code.
    pub vehicle_type: u8,
    /// 0 = heard directly, n = via n relays.
    pub hops: u8,
    /// Its reported position accuracy (m).
    pub accuracy_m: f64,
    /// Age of its latest message (ms).
    pub age_ms: u32,
    /// Driving score 0–100 observed by this phone (SPEC §7; 100 until something is seen).
    pub score: u8,
    /// Behaviour episodes observed while it was in my mesh.
    pub counts: EventCounts,
}

/// Behaviour episode counts (SPEC §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, uniffi::Record)]
pub struct EventCounts {
    /// Speeding episodes.
    pub speeding: u32,
    /// Harsh accelerations.
    pub harsh_accels: u32,
    /// Harsh braking.
    pub harsh_brakes: u32,
    /// Overtakes.
    pub overtakes: u32,
}

/// This phone's own trip.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TripInfo {
    /// Distance (m).
    pub distance_m: f64,
    /// Average speed (m/s).
    pub avg_speed_mps: f64,
    /// Top speed (m/s).
    pub max_speed_mps: f64,
    /// Driving score 0–100.
    pub score: u8,
    /// Episode counts.
    pub counts: EventCounts,
}

/// One behaviour event for the timeline. `kind` is a stable key the app maps to its
/// strings: "speeding", "hacc", "hbrake", "overtake", "tailgating".
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct EventInfo {
    /// Seconds ago.
    pub age_s: f64,
    /// Event key.
    pub kind: String,
    /// This phone's own vehicle.
    pub me: bool,
    /// The peer's pseudonym (empty for me). For display only; never log it.
    pub pseudonym: Vec<u8>,
    /// Speed at the time (km/h).
    pub kmh: f64,
}

/// What happened to a received frame (for counters; never log the contents).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum RxResult {
    /// New, verified, plausible.
    Accepted {
        /// The sender's pseudonym (for in-memory counters only; never log it).
        pseudonym: Vec<u8>,
    },
    /// Duplicate, stale, blocked or my own: dropped quietly.
    Dropped,
    /// Failed parsing, signature or plausibility checks.
    Rejected {
        /// Reason key, e.g. "BadSignature".
        reason: String,
    },
}

fn vehicle(code: u8) -> VehicleType {
    match code {
        1 => VehicleType::Motorbike,
        2 => VehicleType::TukTuk,
        3 => VehicleType::Truck,
        4 => VehicleType::Bus,
        5 => VehicleType::Pedestrian,
        6 => VehicleType::Bicycle,
        _ => VehicleType::Car,
    }
}

fn vehicle_code(v: VehicleType) -> u8 {
    v as u8
}

struct ConstLimit(Option<f64>);

impl SpeedLimits for ConstLimit {
    fn limit_at(&self, _: f64, _: f64) -> Option<f64> {
        self.0
    }
}

struct Inner {
    obu: Obu,
    signer: PseudonymSigner<Keys>,
    verifier: CertVerifier,
    origin: GeoOrigin,
    reports: Vec<Vec<u8>>,
    last_decide: Option<f64>,
}

/// The safety engine of one phone.
#[derive(uniffi::Object)]
pub struct SafetyEngine {
    inner: Mutex<Inner>,
}

impl SafetyEngine {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Inner {
    fn own(&self, fix: &OwnFix) -> OwnState {
        let (x, y) = self.origin.to_local(fix.lat, fix.lon);
        OwnState {
            motion: Motion {
                x,
                y,
                heading: compass_deg_to_math(fix.heading_deg),
                speed: fix.speed_mps.max(0.0),
                accel: fix.accel_mps2,
                yaw_rate: fix.yaw_rate_dps.to_radians(),
            },
            vehicle_type: vehicle(fix.vehicle_type),
            pos_accuracy_m: fix.accuracy_m.max(0.0),
        }
    }
}

fn anchor(bytes: &[u8]) -> Result<TrustAnchor, RoadMeshError> {
    if bytes.len() != 1 + PUBLIC_KEY_LEN {
        return Err(bad("a trust anchor is 1 key-id byte + 33 key bytes"));
    }
    let mut public_key = [0u8; PUBLIC_KEY_LEN];
    public_key.copy_from_slice(&bytes[1..]);
    Ok(TrustAnchor {
        key_id: bytes[0],
        public_key,
    })
}

#[uniffi::export]
impl SafetyEngine {
    /// New engine. `origin_*`: a point near where driving starts (the local metric
    /// frame); `trust_anchors`: the CA public keys built into the app (34 bytes each);
    /// `relay`: pass on other vehicles' messages.
    #[uniffi::constructor]
    pub fn new(
        origin_lat: f64,
        origin_lon: f64,
        trust_anchors: Vec<Vec<u8>>,
        keys: Arc<dyn HardwareKeys>,
        relay: bool,
    ) -> Result<Arc<Self>, RoadMeshError> {
        let cfg = Config::spec_v0();
        let anchors = trust_anchors
            .iter()
            .map(|a| anchor(a))
            .collect::<Result<Vec<_>, _>>()?;
        let verifier = CertVerifier::new(&anchors, &cfg.security)
            .map_err(|e| bad(&format!("trust anchor: {e:?}")))?;
        let origin = GeoOrigin::new(origin_lat, origin_lon);
        let signer = PseudonymSigner::new(Vec::new(), Keys(keys), &cfg.security);
        Ok(Arc::new(Self {
            inner: Mutex::new(Inner {
                obu: Obu::new(cfg, origin, relay),
                signer,
                verifier,
                origin,
                reports: Vec::new(),
                last_decide: None,
            }),
        }))
    }

    /// Install this phone's pseudonym certificates (a batch from the CA, 107 bytes each).
    /// Each is checked against the trust anchors. Returns how many are held.
    pub fn add_certificates(&self, batch: Vec<u8>, now: f64) -> Result<u32, RoadMeshError> {
        let certs =
            pki::decode_certificates(&batch).map_err(|e| bad(&format!("certificates: {e:?}")))?;
        let mut g = self.lock();
        for c in &certs {
            g.verifier
                .check_certificate(c, now)
                .map_err(|e| RoadMeshError::Rejected {
                    detail: format!("{e:?}"),
                })?;
        }
        g.signer.add_certificates(certs);
        Ok(g.signer.certificates().len() as u32)
    }

    /// Certificates valid at `now`. Zero means receive-only.
    pub fn valid_certificates(&self, now: f64) -> u32 {
        let g = self.lock();
        g.signer
            .certificates()
            .iter()
            .filter(|c| c.valid_at(now, 0.0))
            .count() as u32
    }

    /// Install a revocation list from the server. Returns true if it was newer.
    pub fn load_revocation_list(&self, bytes: Vec<u8>) -> Result<bool, RoadMeshError> {
        let mut g = self.lock();
        match g.verifier.load_revocation_list(&bytes) {
            Ok(pki::CrlUpdate::Updated(_)) => Ok(true),
            Ok(pki::CrlUpdate::Unchanged) => Ok(false),
            Err(e) => Err(RoadMeshError::Rejected {
                detail: format!("{e:?}"),
            }),
        }
    }

    /// Frames to broadcast now (relays, then this phone's own signed message).
    /// Empty when the phone holds no valid certificate and has nothing to relay.
    pub fn transmit(&self, now: f64, fix: OwnFix, road: RoadInfo) -> Vec<Vec<u8>> {
        let mut g = self.lock();
        let own = g.own(&fix);
        // AUDIT S-13: switch pseudonym only when stopped or at a junction
        g.signer
            .set_rotation_allowed(fix.speed_mps < 1.0 || road.at_junction);
        let ctx = RoadContext {
            speed_limit: road.speed_limit_mps,
            at_junction: road.at_junction,
        };
        let Inner { obu, signer, .. } = &mut *g;
        obu.transmit(now, &own, &ctx, signer as &mut dyn Signer)
            .iter()
            .filter_map(|f| f.encode())
            .collect()
    }

    /// Relays whose back-off (SPEC §5 rule 3, up to 60 ms) has ended. Optional: the
    /// platform may call this between transmit cycles for finer timing; `transmit`
    /// always includes them.
    pub fn due_relays(&self, now: f64) -> Vec<Vec<u8>> {
        self.lock()
            .obu
            .due_relays(now)
            .iter()
            .filter_map(|f| f.encode())
            .collect()
    }

    /// The pseudonym in use at `now` (empty if none). When it changes, the platform
    /// restarts advertising so the Bluetooth address changes with it (AUDIT S-13).
    pub fn pseudonym(&self, now: f64) -> Vec<u8> {
        let g = self.lock();
        g.signer
            .active_certificate(now)
            .map(|c| c.temp_id().to_vec())
            .unwrap_or_default()
    }

    /// Handle one received radio frame (untrusted bytes).
    pub fn receive(&self, frame: Vec<u8>, now: f64, fix: OwnFix) -> RxResult {
        let mut g = self.lock();
        let own = g.own(&fix);
        let Inner {
            obu,
            verifier,
            signer,
            reports,
            ..
        } = &mut *g;
        match obu.receive_bytes(&frame, now, &own, verifier as &mut dyn Verifier) {
            RxOutcome::Accepted { temp_id, .. } => RxResult::Accepted {
                pseudonym: temp_id.to_vec(),
            },
            RxOutcome::Dropped | RxOutcome::Own | RxOutcome::Blocked => RxResult::Dropped,
            RxOutcome::Malformed(e) => RxResult::Rejected {
                reason: format!("{e:?}"),
            },
            RxOutcome::AuthFailed { reason, .. } => RxResult::Rejected {
                reason: format!("{reason:?}"),
            },
            RxOutcome::Misbehaviour {
                temp_id,
                reason,
                report,
            } => {
                if report {
                    let evidence = obu.take_evidence(&temp_id);
                    if let Some(bytes) =
                        MisbehaviourReport::build(signer, now, ReportReason::from(reason), evidence)
                            .and_then(|r| r.encode())
                    {
                        reports.push(bytes);
                    }
                }
                RxResult::Rejected {
                    reason: format!("{:?}", ReportReason::from(reason)),
                }
            }
        }
    }

    /// Run the safety logic for this cycle.
    pub fn decide(&self, now: f64, fix: OwnFix, road: RoadInfo) -> DecisionInfo {
        let mut g = self.lock();
        let own = g.own(&fix);
        let dt = g.last_decide.map_or(0.1, |t| (now - t).clamp(0.0, 1.0));
        g.last_decide = Some(now);
        let ctx = RoadContext {
            speed_limit: road.speed_limit_mps,
            at_junction: road.at_junction,
        };
        let d = g
            .obu
            .decide(now, dt, &own, &ctx, &ConstLimit(road.speed_limit_mps));
        DecisionInfo {
            alerts: d.alerts.iter().map(alert_info).collect(),
            advised_accel_mps2: d.advised_accel,
            avoiding: d.avoiding.iter().map(|t| t.to_vec()).collect(),
        }
    }

    /// Vehicles heard, relative to me (smoothed and map-matched positions).
    pub fn peers(&self, now: f64, fix: OwnFix) -> Vec<PeerInfo> {
        let g = self.lock();
        let me = g.own(&fix).motion;
        let (view, _) = g.obu.peer_view();
        let profiles = g.obu.behavior().profiles();
        let cfg = g.obu.config();
        view.iter()
            .map(|e| {
                let m = &e.msg;
                let (east, north) = (m.motion.x - me.x, m.motion.y - me.y);
                PeerInfo {
                    pseudonym: m.temp_id.to_vec(),
                    east_m: east,
                    north_m: north,
                    distance_m: east.hypot(north),
                    speed_mps: m.motion.speed,
                    accel_mps2: m.motion.accel,
                    heading_deg: math_to_compass_deg(m.motion.heading),
                    vehicle_type: vehicle_code(m.vehicle_type),
                    hops: e.hops,
                    accuracy_m: m.pos_accuracy_m,
                    age_ms: ((now - m.t).max(0.0) * 1000.0).min(f64::from(u32::MAX)) as u32,
                    score: profiles
                        .get(&Subject::Peer(m.temp_id))
                        .map_or(100, |p| p.score(cfg)),
                    counts: profiles
                        .get(&Subject::Peer(m.temp_id))
                        .map(counts)
                        .unwrap_or_default(),
                }
            })
            .collect()
    }

    /// This phone's own trip so far.
    pub fn trip(&self) -> TripInfo {
        let g = self.lock();
        let p = g
            .obu
            .behavior()
            .profiles()
            .get(&Subject::Me)
            .copied()
            .unwrap_or(Profile {
                episode_score: 100.0,
                ..Profile::default()
            });
        TripInfo {
            distance_m: p.distance_m,
            avg_speed_mps: p.avg_speed(),
            max_speed_mps: p.max_speed,
            score: p.score(g.obu.config()),
            counts: counts(&p),
        }
    }

    /// The newest `max` behaviour events (own and peers in my mesh), newest first.
    pub fn events(&self, now: f64, max: u32) -> Vec<EventInfo> {
        let g = self.lock();
        g.obu
            .behavior()
            .events()
            .iter()
            .rev()
            .filter_map(|e| {
                let (kind, kmh) = match e.kind {
                    EventKind::Speeding { kmh, .. } => ("speeding", kmh),
                    EventKind::HarshAccel { kmh, .. } => ("hacc", kmh),
                    EventKind::HarshBrake { kmh, .. } => ("hbrake", kmh),
                    EventKind::Overtake { kmh, .. } => ("overtake", kmh),
                    EventKind::Tailgating { kmh, .. } => ("tailgating", kmh),
                    EventKind::SpeedingEnd { .. } | EventKind::TailgatingEnd { .. } => {
                        return None;
                    }
                };
                Some(EventInfo {
                    age_s: (now - e.t).max(0.0),
                    kind: kind.into(),
                    me: e.who == Subject::Me,
                    pseudonym: match e.who {
                        Subject::Me => Vec::new(),
                        Subject::Peer(id) => id.to_vec(),
                    },
                    kmh,
                })
            })
            .take(max as usize)
            .collect()
    }

    /// "Delete my data" on this phone: the trip, behaviour records, peers heard and
    /// pending reports. Certificates and blocked senders are kept.
    pub fn delete_my_data(&self) {
        let mut g = self.lock();
        g.obu.forget_all();
        g.reports.clear();
        g.last_decide = None;
    }

    /// Signed misbehaviour reports waiting to be sent to the server (taken once).
    pub fn take_reports(&self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.lock().reports)
    }
}

fn counts(p: &Profile) -> EventCounts {
    EventCounts {
        speeding: p.speeding_episodes,
        harsh_accels: p.harsh_accels,
        harsh_brakes: p.harsh_brakes,
        overtakes: p.overtakes,
    }
}

fn alert_info(a: &obu::Alert) -> AlertInfo {
    AlertInfo {
        kind: format!("{:?}", a.kind),
        level: match a.level {
            obu::Level::Caution => AlertLevel::Caution,
            obu::Level::Warning => AlertLevel::Warning,
            obu::Level::Danger => AlertLevel::Danger,
        },
        advice: format!("{:?}", a.advice),
        advised_decel_mps2: a.advised_decel,
        distance_m: a.distance_m,
        ttc_s: a.ttc_s,
        hops: a.hops,
        target: a.target.map(|t| t.to_vec()),
    }
}

fn key33(v: &[u8], what: &str) -> Result<[u8; PUBLIC_KEY_LEN], RoadMeshError> {
    <[u8; PUBLIC_KEY_LEN]>::try_from(v)
        .map_err(|_| bad(&format!("{what}: a compressed P-256 key is 33 bytes")))
}

/// The bytes the device key must sign to request certificates.
#[uniffi::export]
pub fn issue_request_to_sign(
    device_key: Vec<u8>,
    pseudonym_keys: Vec<Vec<u8>>,
    requested_at: u32,
) -> Result<Vec<u8>, RoadMeshError> {
    let dk = key33(&device_key, "device key")?;
    let keys = pseudonym_keys
        .iter()
        .map(|k| key33(k, "pseudonym key"))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(pki::issue_tbs(&dk, requested_at, &keys))
}

/// Encode a signed certificate request for the CA.
#[uniffi::export]
pub fn encode_issue_request(
    device_key: Vec<u8>,
    pseudonym_keys: Vec<Vec<u8>>,
    requested_at: u32,
    attestation: Vec<u8>,
    signature: Vec<u8>,
) -> Result<Vec<u8>, RoadMeshError> {
    let w = IssueRequestWire {
        device_key: key33(&device_key, "device key")?,
        requested_at,
        pseudonym_keys: pseudonym_keys
            .iter()
            .map(|k| key33(k, "pseudonym key"))
            .collect::<Result<Vec<_>, _>>()?,
        attestation,
        signature: <[u8; SIG_LEN]>::try_from(signature.as_slice())
            .map_err(|_| bad("signature: raw r‖s is 64 bytes"))?,
    };
    w.encode()
        .ok_or_else(|| bad("too many keys or attestation too long"))
}

/// The 8-byte digest of each certificate in a batch, in order (the platform stores
/// which hardware key belongs to which certificate).
#[uniffi::export]
pub fn certificate_digests(batch: Vec<u8>) -> Result<Vec<Vec<u8>>, RoadMeshError> {
    let certs =
        pki::decode_certificates(&batch).map_err(|e| bad(&format!("certificates: {e:?}")))?;
    Ok(certs.iter().map(|c| c.digest().to_vec()).collect())
}

/// The public key inside each certificate of a batch, in order (33 bytes each).
#[uniffi::export]
pub fn certificate_public_keys(batch: Vec<u8>) -> Result<Vec<Vec<u8>>, RoadMeshError> {
    let certs =
        pki::decode_certificates(&batch).map_err(|e| bad(&format!("certificates: {e:?}")))?;
    Ok(certs.iter().map(|c| c.public_key.to_vec()).collect())
}

/// Version of the core, for the about screen.
#[uniffi::export]
pub fn core_version() -> String {
    roadmesh_core::VERSION.to_owned()
}

/// Cut a frame into GATT notifications of at most `max_packet` bytes (MTU − 3).
/// Empty if the packet size is too small.
#[uniffi::export]
pub fn split_frame(frame: Vec<u8>, frame_no: u8, max_packet: u32) -> Vec<Vec<u8>> {
    roadmesh_core::fragment::split(&frame, frame_no, max_packet as usize).unwrap_or_default()
}

/// Reassembles notifications from one connected phone (untrusted bytes).
#[derive(uniffi::Object)]
pub struct FrameReassembler {
    inner: Mutex<roadmesh_core::fragment::Reassembler>,
}

#[uniffi::export]
impl FrameReassembler {
    /// One reassembler per link.
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(roadmesh_core::fragment::Reassembler::new()),
        })
    }

    /// Feed one notification; returns the complete frame (empty until then).
    pub fn push(&self, packet: Vec<u8>) -> Vec<u8> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(&packet)
            .unwrap_or_default()
    }
}
