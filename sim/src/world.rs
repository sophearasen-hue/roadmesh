//! The simulated world: vehicles, human drivers, buildings, attackers, radio and
//! the main loop. Port of `reference/core-python/v2v/world.py`.
//!
//! **Test driver.** A vehicle with the app follows the core's advised
//! acceleration instantly (`advised_accel`), which reproduces the reference,
//! where the engine braked the car itself. This models a driver who obeys the
//! warning with no reaction delay; it lives only in the simulator. The product
//! warns and never brakes (CLAUDE.md).

use std::collections::HashMap;
use std::f64::consts::PI;

use roadmesh_core::auth::Signer;
use roadmesh_core::behavior::SpeedLimits;
use roadmesh_core::config::Config;
use roadmesh_core::geo::{GeoOrigin, math_to_compass_deg};
use roadmesh_core::geometry::{Point, polygons_overlap, segment_hits_polygon, vehicle_box};
use roadmesh_core::obu::{
    AlertKind, BrakeAdvice, Decision, DriveState, Level, Obu, OwnState, RoadContext, RxOutcome,
};
use roadmesh_core::prediction::{Motion, first_conflict, trajectory};
use roadmesh_core::rsm::{self, EventCode, Frame, MsgType, Rsm, TempId, VehicleType};

use roadmesh_core::pki::{
    CertDigest, CertVerifier, MisbehaviourReport, PseudonymSigner, ReportReason,
};
use roadmesh_server::ca::ReportOutcome;

use crate::keys::{SimCa, SimVerifier, SoftKeys, new_ca};
use crate::rng::SimRng;

/// Simulation step (s): 10 Hz, the standard message rate.
pub const TICK: f64 = 0.1;
/// Default radio range (m).
pub const RADIO_RANGE_M: f64 = 300.0;
/// Human reaction time (s).
pub const HUMAN_REACTION_S: f64 = 1.2;
/// Humans react to conflicts predicted within this (s).
pub const HUMAN_HAZARD_TTC_S: f64 = 3.0;
/// Gentle acceleration back to the desired speed (m/s²).
pub const CRUISE_ACCEL: f64 = 1.5;
/// Hardest braking a vehicle can do (m/s²).
pub const MAX_BRAKE: f64 = -8.0;
const SAFETY_MARGIN_M: f64 = 0.5;

fn round6(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

/// Scripted driver action (e.g. the lead car slamming the brakes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Maneuver {
    /// Start (s).
    pub t0: f64,
    /// End (s).
    pub t1: f64,
    /// Forced acceleration (m/s²).
    pub accel: Option<f64>,
    /// Forced yaw rate (rad/s).
    pub yaw_rate: Option<f64>,
}

/// Background role, for reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// A main car.
    Car,
    /// Background traffic relaying messages.
    Relay,
    /// Part of a hazard (stopped queue).
    Hazard,
}

/// A simulated vehicle and its driver's intentions.
#[derive(Debug, Clone)]
pub struct Vehicle {
    /// Name (for reports only; never broadcast).
    pub name: String,
    /// x east (m).
    pub x: f64,
    /// y north (m).
    pub y: f64,
    /// Heading (rad).
    pub heading: f64,
    /// Speed (m/s).
    pub speed: f64,
    /// Speed the driver wants now (m/s).
    pub target_speed: f64,
    /// Speed the driver prefers (m/s).
    pub desired_speed: f64,
    /// Length (m).
    pub length: f64,
    /// Width (m).
    pub width: f64,
    /// Type (sets the outline others predict with).
    pub vehicle_type: VehicleType,
    /// Has the app.
    pub has_obu: bool,
    /// Role.
    pub role: Role,
    /// Keeps to the posted limit.
    pub respects_limit: bool,
    /// Overtakes slower traffic through the oncoming lane.
    pub overtakes: bool,
    /// Car-following time gap (s); `None` = no following model.
    pub headway_s: Option<f64>,
    /// Lateral position being steered to.
    pub lane_y: Option<f64>,
    /// Road direction.
    pub base_heading: f64,
    /// Own lane.
    pub home_y: f64,
    /// Currently overtaking.
    pub passing: bool,
    /// Scripted actions.
    pub maneuvers: Vec<Maneuver>,
    /// Applied acceleration (m/s²).
    pub accel: f64,
    /// Applied yaw rate (rad/s).
    pub yaw_rate: f64,
    /// Crashed.
    pub crashed: bool,
}

impl Vehicle {
    /// A car with the app, at `speed` m/s.
    pub fn new(name: &str, x: f64, y: f64, heading: f64, speed: f64) -> Self {
        Self {
            name: name.to_owned(),
            x,
            y,
            heading,
            speed,
            target_speed: speed,
            desired_speed: speed,
            length: 4.5,
            width: 1.8,
            vehicle_type: VehicleType::Car,
            has_obu: true,
            role: Role::Car,
            respects_limit: true,
            overtakes: false,
            headway_s: None,
            lane_y: None,
            base_heading: heading,
            home_y: y,
            passing: false,
            maneuvers: Vec::new(),
            accel: 0.0,
            yaw_rate: 0.0,
            crashed: false,
        }
    }
    /// Preferred speed (m/s).
    pub fn wants(mut self, v: f64) -> Self {
        self.target_speed = v;
        self.desired_speed = v;
        self
    }
    /// Vehicle type (sets length and width).
    pub fn kind(mut self, t: VehicleType) -> Self {
        self.vehicle_type = t;
        (self.length, self.width) = t.dims();
        self
    }
    /// Role.
    pub fn role(mut self, r: Role) -> Self {
        self.role = r;
        self
    }
    /// Car-following gap (s).
    pub fn headway(mut self, s: f64) -> Self {
        self.headway_s = Some(s);
        self
    }
    /// Ignores limits and overtakes.
    pub fn speeder(mut self) -> Self {
        self.respects_limit = false;
        self.overtakes = true;
        self
    }
    /// Add a scripted action.
    pub fn maneuver(mut self, t0: f64, t1: f64, accel: f64) -> Self {
        self.maneuvers.push(Maneuver {
            t0,
            t1,
            accel: Some(accel),
            yaw_rate: None,
        });
        self
    }

    fn footprint(&self) -> [Point; 4] {
        vehicle_box(self.x, self.y, self.heading, self.length, self.width, 0.0)
    }

    fn motion(&self) -> Motion {
        Motion {
            x: self.x,
            y: self.y,
            heading: self.heading,
            speed: self.speed,
            accel: self.accel,
            yaw_rate: self.yaw_rate,
        }
    }

    fn own_state(&self) -> OwnState {
        OwnState {
            motion: self.motion(),
            vehicle_type: self.vehicle_type,
            pos_accuracy_m: 1.0,
        }
    }

    fn active_maneuver(&self, now: f64) -> Option<Maneuver> {
        self.maneuvers
            .iter()
            .copied()
            .find(|m| m.t0 <= now && now < m.t1)
    }

    fn step_motion(&mut self, dt: f64) {
        if self.crashed {
            self.speed = 0.0;
            self.accel = 0.0;
            self.yaw_rate = 0.0;
            return;
        }
        self.speed = (self.speed + self.accel * dt).max(0.0);
        if let Some(lane_y) = self.lane_y
            && self.speed > 0.5
        {
            // lane change: steer toward lane_y (lateral speed up to 1.8 m/s)
            let pos = if self.base_heading.sin().abs() < 0.5 {
                self.y
            } else {
                self.x
            };
            let off = lane_y - pos;
            let vy = if off.abs() > 0.05 {
                (off / 0.8).clamp(-1.8, 1.8)
            } else {
                0.0
            };
            self.heading = self.base_heading + vy.atan2(self.speed);
        } else if self.speed > 0.0 {
            self.heading += self.yaw_rate * dt;
        }
        self.x += self.speed * self.heading.cos() * dt;
        self.y += self.speed * self.heading.sin() * dt;
    }
}

#[derive(Debug, Clone)]
struct HumanDriver {
    hazard_since: Option<f64>,
    braking_until: f64,
}

/// Roadside attacker broadcasting a vehicle that does not exist.
#[derive(Debug, Clone)]
pub struct GhostAttacker {
    /// Name (reports only).
    pub name: String,
    /// Attacker's position.
    pub pos: (f64, f64),
    start: (f64, f64),
    heading: f64,
    speed: f64,
    t_start: f64,
    t_end: f64,
    /// Signer: forged certificates (untrusted CA) or a stolen genuine phone.
    pub signer: PseudonymSigner<SoftKeys>,
    msg_id: u16,
}

impl GhostAttacker {
    /// New attacker.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: &str,
        pos: (f64, f64),
        start: (f64, f64),
        heading: f64,
        speed: f64,
        t_start: f64,
        t_end: f64,
        signer: PseudonymSigner<SoftKeys>,
    ) -> Self {
        Self {
            name: name.to_owned(),
            pos,
            start,
            heading,
            speed,
            t_start,
            t_end,
            signer,
            msg_id: 0,
        }
    }

    /// Where the fake vehicle is at `now`, if active.
    pub fn ghost_pose(&self, now: f64) -> Option<(f64, f64, f64)> {
        if !(self.t_start <= now && now < self.t_end) {
            return None;
        }
        let d = self.speed * (now - self.t_start);
        Some((
            self.start.0 + d * self.heading.cos(),
            self.start.1 + d * self.heading.sin(),
            self.heading,
        ))
    }

    fn frames(&mut self, now: f64, origin: &GeoOrigin) -> Vec<Frame> {
        let Some((x, y, h)) = self.ghost_pose(now) else {
            return Vec::new();
        };
        let Some(temp_id) = self.signer.current_temp_id(now) else {
            return Vec::new();
        };
        self.msg_id = self.msg_id.wrapping_add(1);
        let (lat, lon) = origin.to_latlon(x, y);
        let body = Rsm {
            msg_type: MsgType::Heartbeat,
            temp_id,
            msg_id: self.msg_id,
            timestamp_ms: rsm::ms_of_hour(now),
            lat_deg: lat,
            lon_deg: lon,
            speed: self.speed,
            heading_deg: math_to_compass_deg(h),
            accel: 0.0,
            yaw_rate: 0.0,
            vehicle_type: VehicleType::Car,
            flags: 0,
            pos_accuracy_m: 1.0,
            hop_count: 0,
            ttl: 8,
            origin_pos_age_ms: 0,
            event: EventCode::None,
        }
        .encode();
        let Some(auth) = self.signer.sign(now, false, &body) else {
            return Vec::new();
        };
        vec![Frame {
            last_hop: None,
            body,
            auth,
        }]
    }
}

/// Radio statistics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RadioStats {
    /// Original messages sent.
    pub sent: u64,
    /// Relayed copies sent.
    pub relayed: u64,
    /// Frames delivered to a receiver.
    pub delivered: u64,
    /// Frames lost.
    pub lost: u64,
}

/// Speed zones along the road: (x0, x1, limit km/h, name).
#[derive(Debug, Clone, Default)]
pub struct Zones(pub Vec<(f64, f64, f64, &'static str)>);

impl SpeedLimits for Zones {
    fn limit_at(&self, x: f64, _y: f64) -> Option<f64> {
        self.0
            .iter()
            .find(|z| z.0 <= x && x < z.1)
            .map(|z| z.2 / 3.6)
    }
}

/// Run options.
#[derive(Debug, Clone)]
pub struct Options {
    /// Phones with the app talk to each other.
    pub v2v: bool,
    /// Signature checks on.
    pub security: bool,
    /// Packet loss probability.
    pub loss: f64,
    /// Seed.
    pub seed: u64,
    /// Phones relay each other's messages.
    pub relay: bool,
    /// Core configuration.
    pub config: Config,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            v2v: true,
            security: true,
            loss: 0.0,
            seed: 1,
            relay: true,
            config: Config::reference(),
        }
    }
}

/// A notable moment, for reports and golden tests.
#[derive(Debug, Clone, PartialEq)]
pub enum Moment {
    /// `car` started giving way to `target`.
    Yield {
        /// time (s)
        t: f64,
        /// who yields
        car: String,
        /// to whom
        target: String,
    },
    /// `car` started slowing for stopped traffic `gap` m ahead, learned via `hops` relays.
    Hazard {
        /// time (s)
        t: f64,
        /// who
        car: String,
        /// distance (m)
        gap: f64,
        /// relays
        hops: u8,
    },
    /// The watched pair can finally see each other.
    Sight {
        /// time (s)
        t: f64,
        /// distance (m)
        gap: f64,
    },
    /// The alert at the top of `car`'s screen changed.
    Alert {
        /// time (s)
        t: f64,
        /// whose screen
        car: String,
        /// new top alert, `None` = all clear
        alert: Option<(AlertKind, Level, BrakeAdvice)>,
        /// about whom
        target: Option<String>,
    },
    /// A certificate was revoked.
    Revoked {
        /// time (s)
        t: f64,
        /// owner
        owner: String,
    },
}

/// Results of a run.
#[derive(Debug, Clone, Default)]
pub struct Summary {
    /// (t, a, b) collisions.
    pub collisions: Vec<(f64, String, String)>,
    /// Closest distance of the watched pair (m).
    pub min_gap_m: Option<f64>,
    /// Lowest recorded speed per vehicle (km/h).
    pub min_speed_kmh: HashMap<String, f64>,
    /// Hardest recorded braking per uncrashed vehicle (m/s², ≥ 0).
    pub max_brake_ms2: HashMap<String, f64>,
    /// Revoked certificate owners.
    pub revoked: Vec<String>,
    /// Radio statistics.
    pub radio: RadioStats,
}

/// The simulated world.
pub struct World {
    /// Current time (s).
    pub t: f64,
    /// Run length (s).
    pub duration: f64,
    /// Vehicles.
    pub vehicles: Vec<Vehicle>,
    /// Buildings.
    pub buildings: Vec<Vec<Point>>,
    /// Options.
    pub opts: Options,
    /// Radio range (m).
    pub radio_range: f64,
    /// How far a human can see (fog) (m).
    pub visibility: f64,
    /// Speed zones.
    pub zones: Zones,
    /// Pair whose sight line and gap are tracked.
    pub watch: Option<(usize, usize)>,
    /// Record statistics every n ticks (as the reference does).
    pub record_every: u64,
    /// Attackers.
    pub attackers: Vec<GhostAttacker>,
    /// Engines (index = vehicle index).
    pub obus: Vec<Option<Obu>>,
    /// The certificate authority (real server logic, software key).
    pub ca: SimCa,
    /// Which vehicle (or attacker) owns each pseudonym. Simulator bookkeeping only.
    pub cert_owner: HashMap<TempId, String>,
    /// Same, by certificate digest.
    pub digest_owner: HashMap<CertDigest, String>,
    /// Notable moments.
    pub moments: Vec<Moment>,
    /// Last decision per vehicle.
    pub decisions: Vec<Option<Decision>>,
    /// World RNG (attacker credentials).
    pub rng: SimRng,
    origin: GeoOrigin,
    drivers: Vec<HumanDriver>,
    signers: Vec<Option<PseudonymSigner<SoftKeys>>>,
    verifiers: Vec<Option<SimVerifier>>,
    loss_rng: SimRng,
    pending: Vec<(usize, (f64, f64), Frame)>,
    tick: u64,
    seen: bool,
    min_gap: f64,
    min_speed: Vec<f64>,
    min_accel: Vec<f64>,
    radio: RadioStats,
    collisions: Vec<(f64, String, String)>,
}

impl World {
    /// Build a world. Vehicles with the app get 3 pseudonym certificates each.
    pub fn new(
        vehicles: Vec<Vehicle>,
        buildings: Vec<Vec<Point>>,
        opts: Options,
        duration: f64,
    ) -> Self {
        let mut ca_rng = SimRng::new(opts.seed + 1000);
        let mut ca = new_ca(&mut ca_rng, opts.config.clone());
        let anchor = ca.trust_anchor();
        let origin = GeoOrigin::phnom_penh();
        let mut cert_owner = HashMap::new();
        let mut digest_owner = HashMap::new();
        let mut obus = Vec::new();
        let mut signers = Vec::new();
        let mut verifiers = Vec::new();
        for v in &vehicles {
            if opts.v2v && v.has_obu {
                let signer = crate::keys::enrol(&mut ca, &mut ca_rng, 3, 0);
                for c in signer.certificates() {
                    cert_owner.insert(c.temp_id(), v.name.clone());
                    digest_owner.insert(c.digest(), v.name.clone());
                }
                obus.push(Some(Obu::new(opts.config.clone(), origin, opts.relay)));
                signers.push(Some(signer));
                verifiers.push(Some(if opts.security {
                    match CertVerifier::new(&[anchor], &opts.config.security) {
                        Ok(v) => SimVerifier::Real(Box::new(v)),
                        Err(_) => SimVerifier::Off, // unreachable: the CA key is valid
                    }
                } else {
                    SimVerifier::Off
                }));
            } else {
                obus.push(None);
                signers.push(None);
                verifiers.push(None);
            }
        }
        let n = vehicles.len();
        Self {
            t: 0.0,
            duration,
            loss_rng: SimRng::new(opts.seed + 2000),
            rng: SimRng::new(opts.seed),
            vehicles,
            buildings,
            radio_range: RADIO_RANGE_M,
            visibility: f64::INFINITY,
            zones: Zones::default(),
            watch: None,
            record_every: 1,
            attackers: Vec::new(),
            obus,
            ca,
            cert_owner,
            digest_owner,
            moments: Vec::new(),
            decisions: vec![None; n],
            origin,
            drivers: vec![
                HumanDriver {
                    hazard_since: None,
                    braking_until: f64::NEG_INFINITY
                };
                n
            ],
            signers,
            verifiers,
            opts,
            pending: Vec::new(),
            tick: 0,
            seen: false,
            min_gap: f64::INFINITY,
            min_speed: vec![f64::INFINITY; n],
            min_accel: vec![f64::INFINITY; n],
            radio: RadioStats::default(),
            collisions: Vec::new(),
        }
    }

    /// Index of a vehicle by name.
    pub fn idx(&self, name: &str) -> Option<usize> {
        self.vehicles.iter().position(|v| v.name == name)
    }

    /// Watch a pair of vehicles (sight line, closest gap).
    pub fn watch_pair(&mut self, a: &str, b: &str) {
        if let (Some(i), Some(j)) = (self.idx(a), self.idx(b)) {
            self.watch = Some((i, j));
        }
    }

    /// Name of the vehicle or attacker behind a pseudonym.
    pub fn owner(&self, id: &TempId) -> String {
        self.cert_owner
            .get(id)
            .cloned()
            .unwrap_or_else(|| format!("{:02x}{:02x}{:02x}{:02x}", id[0], id[1], id[2], id[3]))
    }

    /// Change packet loss mid-run (for the fail-safe test).
    pub fn set_loss(&mut self, loss: f64) {
        self.opts.loss = loss;
    }

    fn line_of_sight(&self, a: usize, b: usize) -> bool {
        let (va, vb) = (&self.vehicles[a], &self.vehicles[b]);
        if (va.x - vb.x).hypot(va.y - vb.y) > self.visibility {
            return false;
        }
        let (p, q) = ((va.x, va.y), (vb.x, vb.y));
        if self
            .buildings
            .iter()
            .any(|poly| segment_hits_polygon(p, q, poly))
        {
            return false;
        }
        !self
            .vehicles
            .iter()
            .enumerate()
            .any(|(k, o)| k != a && k != b && segment_hits_polygon(p, q, &o.footprint()))
    }

    // ------------------------------------------------------------ human driver
    fn leader(&self, i: usize) -> Option<(usize, f64)> {
        let v = &self.vehicles[i];
        let (s, c) = v.base_heading.sin_cos();
        let mut best: Option<(usize, f64)> = None;
        for (k, o) in self.vehicles.iter().enumerate() {
            if k == i || (o.heading - v.base_heading).cos() < 0.9 || !self.line_of_sight(i, k) {
                continue;
            }
            let (dx, dy) = (o.x - v.x, o.y - v.y);
            let along = dx * c + dy * s;
            let lat = -dx * s + dy * c;
            let lane = v.lane_y.unwrap_or(v.home_y) - v.y;
            if 0.0 < along && along < 150.0 && (lat - lane).abs() < 1.6 {
                let gap = along - (v.length + o.length) / 2.0;
                if best.is_none_or(|b| gap < b.1) {
                    best = Some((k, gap));
                }
            }
        }
        best
    }

    fn human_accel(&mut self, i: usize, now: f64) -> f64 {
        let v = self.vehicles[i].clone();
        let ego = trajectory(
            Motion {
                accel: 0.0,
                ..v.motion()
            },
            HUMAN_HAZARD_TTC_S,
            TICK,
            0.0,
        );
        let mut hazard = false;
        for k in 0..self.vehicles.len() {
            if k == i || !self.line_of_sight(i, k) {
                continue;
            }
            let o = &self.vehicles[k];
            let tr = trajectory(o.motion(), HUMAN_HAZARD_TTC_S, TICK, 0.0);
            if first_conflict(
                &ego,
                (v.length, v.width),
                &tr,
                (o.length, o.width),
                SAFETY_MARGIN_M,
                0.0,
                TICK,
            )
            .is_some()
            {
                hazard = true;
                break;
            }
        }
        let d = &mut self.drivers[i];
        if hazard {
            let since = *d.hazard_since.get_or_insert(now);
            if now - since >= HUMAN_REACTION_S - 1e-9 {
                d.braking_until = now + 1.0;
            }
        } else {
            d.hazard_since = None;
        }
        if now < d.braking_until {
            return MAX_BRAKE;
        }
        let mut a = if v.speed < v.target_speed - 0.05 {
            CRUISE_ACCEL.min((v.target_speed - v.speed) / TICK)
        } else if v.speed > v.target_speed + 0.05 {
            -(((v.speed - v.target_speed) / 2.0).clamp(1.0, 2.5))
        } else {
            0.0
        };
        if let Some(h) = v.headway_s
            && let Some((k, gap)) = self.leader(i)
        {
            let o = &self.vehicles[k];
            let want = 3.0 + v.speed * h;
            a = a.min((0.4 * (gap - want) + 0.8 * (o.speed - v.speed)).max(-3.5));
        }
        a
    }

    fn set_targets(&mut self, i: usize) {
        let v = &self.vehicles[i];
        let mut target = v.desired_speed;
        if v.respects_limit && !self.zones.0.is_empty() {
            let look = v.speed * 6.0 * v.base_heading.cos();
            for x in [v.x, v.x + look] {
                if let Some(lim) = self.zones.limit_at(x, 0.0) {
                    target = target.min(lim);
                }
            }
        }
        self.vehicles[i].target_speed = target;
        if self.vehicles[i].overtakes {
            self.overtake_logic(i);
        }
    }

    fn overtake_logic(&mut self, i: usize) {
        let v = self.vehicles[i].clone();
        let c = v.base_heading.cos();
        let pass_y = v.home_y + 3.5 * if c > 0.0 { 1.0 } else { -1.0 };
        let in_lane = |o: &Vehicle, y: f64| (o.y - y).abs() < 1.6;
        let along = |o: &Vehicle| (o.x - v.x) * c;
        let others = || {
            self.vehicles
                .iter()
                .enumerate()
                .filter(move |(k, _)| *k != i)
                .map(|(_, o)| o)
        };
        if !v.passing {
            let slow = others().any(|o| {
                in_lane(o, v.home_y)
                    && (o.heading - v.base_heading).cos() > 0.9
                    && 0.0 < along(o)
                    && along(o) < 70.0
                    && o.speed < v.desired_speed - 2.0
            });
            let oncoming = self.vehicles.iter().any(|o| {
                in_lane(o, pass_y)
                    && (o.heading - v.base_heading).cos() < -0.9
                    && -10.0 < along(o)
                    && along(o) < 600.0
            });
            if slow && !oncoming {
                let vm = &mut self.vehicles[i];
                vm.passing = true;
                vm.lane_y = Some(pass_y);
                vm.target_speed = vm.desired_speed;
            }
        } else {
            // stay out until the whole slow line is passed: no gap-cutting
            let blocked = others().any(|o| {
                in_lane(o, v.home_y)
                    && ((-20.0 < along(o) && along(o) < 25.0)
                        || (0.0 < along(o) && along(o) < 70.0 && o.speed < v.speed - 2.0))
            });
            let vm = &mut self.vehicles[i];
            vm.target_speed = vm.desired_speed; // finish the pass briskly
            if !blocked {
                vm.passing = false;
                vm.lane_y = Some(vm.home_y);
            }
        }
    }

    // ------------------------------------------------------------ main loop

    /// Advance one tick.
    pub fn step(&mut self) {
        let now = self.t;
        if self.opts.v2v {
            for i in 0..self.vehicles.len() {
                if self.vehicles[i].crashed {
                    continue;
                }
                let own = self.vehicles[i].own_state();
                let road = RoadContext {
                    speed_limit: self.zones.limit_at(own.motion.x, own.motion.y),
                    at_junction: false,
                };
                if let (Some(obu), Some(signer)) = (self.obus[i].as_mut(), self.signers[i].as_mut())
                {
                    let relays_before = obu.pending_relays().len() as u64;
                    let pos = (own.motion.x, own.motion.y);
                    for f in obu.transmit(now, &own, &road, signer) {
                        self.pending.push((i, pos, f));
                    }
                    self.radio.relayed += relays_before;
                    self.radio.sent += 1;
                }
            }
            for a in self.attackers.iter_mut() {
                for f in a.frames(now, &self.origin) {
                    self.pending.push((usize::MAX, a.pos, f));
                    self.radio.sent += 1;
                }
            }
            self.deliver(now);
        }

        if let Some((a, b)) = self.watch
            && !self.seen
            && self.line_of_sight(a, b)
        {
            self.seen = true;
            let gap = (self.vehicles[a].x - self.vehicles[b].x)
                .hypot(self.vehicles[a].y - self.vehicles[b].y);
            self.moments.push(Moment::Sight { t: now, gap });
        }

        for i in 0..self.vehicles.len() {
            if !self.vehicles[i].crashed {
                self.set_targets(i);
            }
        }
        for i in 0..self.vehicles.len() {
            if self.vehicles[i].crashed {
                continue;
            }
            let own = self.vehicles[i].own_state();
            let road = RoadContext {
                speed_limit: self.zones.limit_at(own.motion.x, own.motion.y),
                at_junction: false,
            };
            let mut cap = None;
            if let Some(obu) = self.obus[i].as_mut() {
                let prev = self.decisions[i].as_ref().map(|d| d.state);
                let prev_top = self.decisions[i]
                    .as_ref()
                    .and_then(|d| d.alerts.first().map(|a| (a.kind, a.level, a.target)));
                let d = obu.decide(now, TICK, &own, &road, &self.zones);
                cap = d.advised_accel;
                self.note_decision(i, now, prev, &d);
                let top = d.alerts.first().map(|a| (a.kind, a.level, a.target));
                if top != prev_top {
                    let car = self.vehicles[i].name.clone();
                    let alert = d.alerts.first().map(|a| (a.kind, a.level, a.advice));
                    let target = d
                        .alerts
                        .first()
                        .and_then(|a| a.target)
                        .map(|id| self.owner(&id));
                    self.moments.push(Moment::Alert {
                        t: now,
                        car,
                        alert,
                        target,
                    });
                }
                self.decisions[i] = Some(d);
            }
            let m = self.vehicles[i].active_maneuver(now);
            let a = match m.and_then(|m| m.accel) {
                Some(forced) => forced, // scripted: the driver did this on purpose
                None => {
                    let h = self.human_accel(i, now);
                    cap.map_or(h, |c| h.min(c)) // test driver follows the advice at once
                }
            };
            let v = &mut self.vehicles[i];
            v.accel = a.clamp(MAX_BRAKE, 3.0);
            v.yaw_rate = m.and_then(|m| m.yaw_rate).unwrap_or(0.0);
        }
        for v in self.vehicles.iter_mut() {
            v.step_motion(TICK);
        }
        for i in 0..self.vehicles.len() {
            for j in (i + 1)..self.vehicles.len() {
                let (a, b) = (&self.vehicles[i], &self.vehicles[j]);
                if (a.crashed && b.crashed) || (a.x - b.x).hypot(a.y - b.y) > 8.0 {
                    continue;
                }
                if polygons_overlap(&a.footprint(), &b.footprint()) {
                    self.collisions
                        .push((round6(now), a.name.clone(), b.name.clone()));
                    self.vehicles[i].crashed = true;
                    self.vehicles[j].crashed = true;
                }
            }
        }
        if let Some((a, b)) = self.watch {
            let (va, vb) = (&self.vehicles[a], &self.vehicles[b]);
            self.min_gap = self.min_gap.min((va.x - vb.x).hypot(va.y - vb.y));
        }
        if self.tick.is_multiple_of(self.record_every) {
            for (k, v) in self.vehicles.iter().enumerate() {
                // the reference records speed and accel rounded to 2 decimals
                self.min_speed[k] = self.min_speed[k].min((v.speed * 100.0).round() / 100.0);
                self.min_accel[k] = self.min_accel[k].min((v.accel * 100.0).round() / 100.0);
            }
        }
        self.tick += 1;
        self.t = round6(now + TICK);
    }

    fn deliver(&mut self, now: f64) {
        let pending = std::mem::take(&mut self.pending);
        for (sender, (sx, sy), frame) in pending {
            for r in 0..self.vehicles.len() {
                if r == sender || self.vehicles[r].crashed || self.obus[r].is_none() {
                    continue;
                }
                let own = self.vehicles[r].own_state();
                if (own.motion.x - sx).hypot(own.motion.y - sy) > self.radio_range {
                    continue;
                }
                if self.loss_rng.next_f64() < self.opts.loss {
                    self.radio.lost += 1;
                    continue;
                }
                self.radio.delivered += 1;
                let (Some(obu), Some(verifier)) =
                    (self.obus[r].as_mut(), self.verifiers[r].as_mut())
                else {
                    continue;
                };
                let out = obu.receive(&frame, now, &own, verifier);
                if let RxOutcome::Misbehaviour {
                    temp_id,
                    reason,
                    report: true,
                } = out
                {
                    // the phone files a signed report with the evidence; the CA decides
                    let evidence = obu.take_evidence(&temp_id);
                    let report = self.signers[r].as_mut().and_then(|s| {
                        MisbehaviourReport::build(s, now, ReportReason::from(reason), evidence)
                    });
                    let outcome = report
                        .and_then(|rep| rep.encode())
                        .map(|bytes| self.ca.report(&bytes, now.max(0.0) as u32));
                    if let Some(Ok(ReportOutcome::Revoked)) = outcome {
                        let owner = self.owner(&temp_id);
                        self.moments.push(Moment::Revoked { t: now, owner });
                        self.publish_revocation_list(now);
                    }
                }
            }
        }
    }

    /// Every phone fetches the new revocation list at once (in reality: daily and on connection).
    fn publish_revocation_list(&mut self, now: f64) {
        let t = now.max(0.0) as u32;
        let Ok(list) = self.ca.revocation_list(t, t + 86_400) else {
            return;
        };
        for v in self.verifiers.iter_mut().flatten() {
            if let SimVerifier::Real(v) = v {
                let _ = v.load_revocation_list(&list);
            }
        }
    }

    fn note_decision(&mut self, i: usize, now: f64, prev: Option<DriveState>, d: &Decision) {
        let car = self.vehicles[i].name.clone();
        if d.state == DriveState::Yield
            && prev != Some(DriveState::Yield)
            && let Some(id) = d.yield_to
        {
            let target = self.owner(&id);
            self.moments.push(Moment::Yield {
                t: now,
                car,
                target,
            });
        } else if d.state == DriveState::Advise
            && prev != Some(DriveState::Advise)
            && let Some(a) = d.alerts.first()
        {
            self.moments.push(Moment::Hazard {
                t: now,
                car,
                gap: a.distance_m.unwrap_or(0.0),
                hops: a.hops,
            });
        }
    }

    /// Run to the end.
    pub fn run(&mut self) -> Summary {
        while self.t < self.duration - 1e-9 {
            self.step();
        }
        self.summary()
    }

    /// Results so far.
    pub fn summary(&self) -> Summary {
        let r1 = |v: f64| (v * 10.0).round() / 10.0;
        Summary {
            collisions: self.collisions.clone(),
            min_gap_m: if self.min_gap.is_finite() {
                Some(r1(self.min_gap))
            } else {
                None
            },
            min_speed_kmh: self
                .vehicles
                .iter()
                .zip(&self.min_speed)
                .map(|(v, s)| (v.name.clone(), r1(s * 3.6)))
                .collect(),
            max_brake_ms2: self
                .vehicles
                .iter()
                .zip(&self.min_accel)
                .filter(|(v, _)| !v.crashed)
                .map(|(v, a)| (v.name.clone(), r1(-a.min(0.0))))
                .collect(),
            revoked: {
                let mut v: Vec<String> = self
                    .digest_owner
                    .iter()
                    .filter(|(d, _)| self.ca.is_revoked(d))
                    .map(|(_, o)| o.clone())
                    .collect();
                v.sort();
                v.dedup();
                v
            },
            radio: self.radio,
        }
    }
}

/// π, for scenario headings.
pub const WEST: f64 = PI;
