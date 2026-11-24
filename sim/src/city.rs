//! Cambodian city traffic (BUILD_PLAN Step 4): the phone prototype's 6-road grid
//! and traffic mix, unsignalised junctions with blind corners, GPS noise, radio
//! range and packet loss, and a ground-truth judge that scores every phone's
//! alerts.
//!
//! * Roads, limits and mix follow `reference/phone-prototype/index.html`:
//!   2.4 × 1.6 km, 3 east-west and 3 north-south roads (40/60/40 km/h), two
//!   lanes each way; 55 % motorbikes, 30 % cars, 8 % tuk-tuks, 7 % trucks;
//!   30 % calm, 50 % normal, 20 % aggressive drivers.
//! * Junctions have no lights. Drivers look for crossing traffic they can SEE
//!   (buildings hide the corners), notice it after a reaction time, and give way
//!   to whoever arrives first. Some do not look at all.
//! * Drivers with the app also react to Warning and Danger after
//!   `app_reaction_s` and follow the advised braking (the app only advises).
//! * Ground truth: each vehicle's real future path (including the turn it has
//!   planned), with no reaction. A conflict is two outlines that would touch
//!   within 3 s.
//!
//! Simulator only: seeded randomness and, when signature checks are off, a
//! plain pseudonym source with no signatures.

use std::collections::{BTreeMap, HashSet};
use std::f64::consts::{FRAC_PI_2, PI};
use std::sync::Arc;

use roadmesh_core::auth::Signer;
use roadmesh_core::behavior::SpeedLimits;
use roadmesh_core::config::Config;
use roadmesh_core::geo::GeoOrigin;
use roadmesh_core::geometry::{Point, polygons_overlap, rect, segment_hits_polygon, vehicle_box};
use roadmesh_core::map::{Road, RoadMap};
use roadmesh_core::obu::{AlertKind, Decision, Level, Obu, OwnState, RoadContext};
use roadmesh_core::pki::{CertVerifier, PseudonymSigner};
use roadmesh_core::prediction::{Motion, Pose, first_conflict};
use roadmesh_core::rsm::{BODY_LEN, Frame, TempId, VehicleType};

use crate::keys::{SimVerifier, SoftKeys, enrol_with, new_ca};
use crate::rng::SimRng;

/// Simulation step (s).
pub const TICK: f64 = 0.1;
const WIDTH: f64 = 2400.0;
const HEIGHT: f64 = 1600.0;
const LANE_W: f64 = 3.4;
const OUTER: f64 = 1.5 * LANE_W; // 5.1 m from the centre line
const INNER: f64 = 0.5 * LANE_W; // 1.7 m
const SETBACK: f64 = 10.0; // building line from the road centre (m)
const JUNCTION_R: f64 = 18.0;
const KMH: f64 = 1.0 / 3.6;
/// Ground truth: a conflict is a predicted touch within this (s).
pub const CONFLICT_TTC_S: f64 = 3.0;
/// A conflict is warned in time if a Warning/Danger was up before the predicted
/// touch came closer than this (s).
pub const CRITICAL_TTC_S: f64 = 1.5;
/// A Warning/Danger is a false alarm unless the pair comes this close (m) within 8 s ...
pub const NEAR_MISS_MARGIN_M: f64 = 1.0;
/// ... or misses each other by less than this at a crossing point (post-encroachment time, s).
pub const NEAR_MISS_PET_S: f64 = 1.0;
const ORACLE_HORIZON_S: f64 = 8.0; // covers the 7 s crossing-conflict horizon
/// A Warning/Danger this soon after the conflict appears still counts (radio and decision delay).
pub const GRACE_S: f64 = 0.5;
/// An alert about the vehicle shown within this time still counts (the driver is reacting).
pub const ALERT_LOOKBACK_S: f64 = 1.0;

/// Driver temperament (prototype `PROF`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Temper {
    /// Slow, careful.
    Calm,
    /// Average.
    Normal,
    /// Fast, overtakes, brakes suddenly, looks less.
    Aggressive,
}

#[derive(Debug, Clone, Copy)]
struct Profile {
    speed_f: f64,
    accel: f64,
    overtake: f64,
    brake_rate: f64,
    headway_s: f64,
    approach_cap: Option<f64>,
    look_prob: f64,
    notice_s: f64,
}

fn profile(t: Temper) -> Profile {
    match t {
        Temper::Calm => Profile {
            speed_f: 0.85,
            accel: 1.2,
            overtake: 0.05,
            brake_rate: 0.002,
            headway_s: 1.4,
            approach_cap: Some(22.0 * KMH),
            look_prob: 0.98,
            notice_s: 0.7,
        },
        Temper::Normal => Profile {
            speed_f: 1.0,
            accel: 2.0,
            overtake: 0.35,
            brake_rate: 0.005,
            headway_s: 1.0,
            approach_cap: Some(32.0 * KMH),
            look_prob: 0.95,
            notice_s: 0.9,
        },
        Temper::Aggressive => Profile {
            speed_f: 1.3,
            accel: 4.2,
            overtake: 1.4,
            brake_rate: 0.02, // per second: about once every 50 s
            headway_s: 0.6,
            approach_cap: None,
            look_prob: 0.88,
            notice_s: 1.2,
        },
    }
}

fn type_speed_f(k: VehicleType) -> f64 {
    match k {
        VehicleType::Motorbike => 0.95,
        VehicleType::TukTuk => 0.7,
        VehicleType::Truck => 0.8,
        _ => 1.0,
    }
}

/// Short label for reports.
pub fn type_label(k: VehicleType) -> &'static str {
    match k {
        VehicleType::Motorbike => "motorbike",
        VehicleType::TukTuk => "tuk-tuk",
        VehicleType::Truck => "truck",
        VehicleType::Car => "car",
        _ => "other",
    }
}

/// Run options.
#[derive(Debug, Clone)]
pub struct CityOptions {
    /// Vehicles on the grid.
    pub vehicles: usize,
    /// Share of vehicles with the app (0–1).
    pub penetration: f64,
    /// Each vehicle's GPS error (RMS, m) is drawn uniformly from this range.
    pub gps_m: (f64, f64),
    /// Random packet loss (0–1).
    pub loss: f64,
    /// Seed.
    pub seed: u64,
    /// Length (s).
    pub duration_s: f64,
    /// Real ECDSA certificates and signature checks (slow with many vehicles).
    pub security: bool,
    /// Phones relay each other's messages (SPEC §5 rules in the product profile).
    pub relay: bool,
    /// Core configuration.
    pub config: Config,
    /// Radio range in open sight (m).
    pub radio_los_m: f64,
    /// Radio range around buildings (m).
    pub radio_nlos_m: f64,
    /// Drivers act on a Warning/Danger this long after it appears (s).
    pub app_reaction_s: f64,
}

impl Default for CityOptions {
    fn default() -> Self {
        Self {
            vehicles: 75,
            penetration: 1.0,
            gps_m: (2.0, 10.0),
            loss: 0.0,
            seed: 1,
            duration_s: 120.0,
            security: false,
            relay: false,
            config: Config::spec_v0(),
            radio_los_m: 150.0,
            radio_nlos_m: 60.0,
            app_reaction_s: 1.0,
        }
    }
}

// ---------------------------------------------------------------- roads

#[derive(Debug, Clone)]
struct CRoad {
    horizontal: bool,
    c: f64,
    len: f64,
    limit: f64,
    /// (position along this road, crossing road index)
    crossings: Vec<(f64, usize)>,
}

fn city_roads() -> Vec<CRoad> {
    let h = [(200.0, 40.0), (800.0, 60.0), (1400.0, 40.0)];
    let v = [(300.0, 40.0), (1200.0, 60.0), (2100.0, 40.0)];
    let mut roads: Vec<CRoad> = h
        .iter()
        .map(|&(c, l)| CRoad {
            horizontal: true,
            c,
            len: WIDTH,
            limit: l * KMH,
            crossings: Vec::new(),
        })
        .chain(v.iter().map(|&(c, l)| CRoad {
            horizontal: false,
            c,
            len: HEIGHT,
            limit: l * KMH,
            crossings: Vec::new(),
        }))
        .collect();
    let n = roads.len();
    for i in 0..n {
        for j in 0..n {
            if roads[i].horizontal != roads[j].horizontal {
                let s = roads[j].c;
                roads[i].crossings.push((s, j));
            }
        }
    }
    roads
}

/// The core map of the city grid.
pub fn city_map() -> RoadMap {
    let roads = city_roads()
        .iter()
        .enumerate()
        .map(|(i, r)| Road {
            id: i as u32,
            points: if r.horizontal {
                vec![(0.0, r.c), (r.len, r.c)]
            } else {
                vec![(r.c, 0.0), (r.c, r.len)]
            },
            lanes: 2,
            lane_width_m: LANE_W,
            limit: r.limit,
            one_way: false,
        })
        .collect();
    RoadMap::new(roads, JUNCTION_R)
}

/// Building blocks between the roads (they block sight and weaken the radio).
pub fn city_blocks() -> Vec<Vec<Point>> {
    let xs = [0.0, 300.0, 1200.0, 2100.0, WIDTH];
    let ys = [0.0, 200.0, 800.0, 1400.0, HEIGHT];
    let mut out = Vec::new();
    for w in xs.windows(2) {
        for h in ys.windows(2) {
            let x0 = if w[0] > 0.0 { w[0] + SETBACK } else { -50.0 };
            let x1 = if w[1] < WIDTH {
                w[1] - SETBACK
            } else {
                WIDTH + 50.0
            };
            let y0 = if h[0] > 0.0 { h[0] + SETBACK } else { -50.0 };
            let y1 = if h[1] < HEIGHT {
                h[1] - SETBACK
            } else {
                HEIGHT + 50.0
            };
            out.push(rect(x0, y0, x1, y1));
        }
    }
    out
}

// ---------------------------------------------------------------- vehicles

#[derive(Debug, Clone, Copy, PartialEq)]
enum Route {
    Lane {
        road: usize,
        dir: f64,
        s: f64,
        lat: f64,
        lat_target: f64,
    },
    Arc {
        cx: f64,
        cy: f64,
        r: f64,
        phi: f64,
        left: f64,
        ccw: bool,
        next: (usize, f64, f64, f64),
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct TurnPlan {
    /// Junction position along the current road.
    at_s: f64,
    /// Road index of the crossing road.
    other: usize,
    /// -1 right, 0 straight, +1 left.
    turn: i8,
    /// The driver looks for crossing traffic at this junction.
    looks: bool,
}

#[derive(Debug, Clone)]
struct GpsNoise {
    sigma: f64,
    bx: f64,
    by: f64,
    last_heading: f64,
}

/// One vehicle.
#[derive(Debug, Clone)]
pub struct CityVehicle {
    /// Type.
    pub kind: VehicleType,
    /// Driver temperament.
    pub temper: Temper,
    /// Has the app.
    pub has_app: bool,
    /// True position, motion.
    pub x: f64,
    /// See `x`.
    pub y: f64,
    /// See `x`.
    pub heading: f64,
    /// See `x`.
    pub speed: f64,
    /// See `x`.
    pub accel: f64,
    /// See `x`.
    pub yaw_rate: f64,
    /// When it crashed.
    pub crashed: Option<f64>,
    /// Removed from the road (5 s after a crash).
    pub gone: bool,
    /// GPS error RMS (m).
    pub gps_sigma_m: f64,
    route: Route,
    plan: Option<TurnPlan>,
    bias: f64,
    bias_t: f64,
    brake_t: f64,
    overtaking: Option<usize>,
    gps: GpsNoise,
    noticed: BTreeMap<usize, f64>,
    alert_since: Option<f64>,
}

impl CityVehicle {
    fn dims(&self) -> (f64, f64) {
        self.kind.dims()
    }

    fn footprint(&self) -> [Point; 4] {
        let (l, w) = self.dims();
        vehicle_box(self.x, self.y, self.heading, l, w, 0.0)
    }

    fn active(&self) -> bool {
        !self.gone
    }
}

fn dir_vec(road: &CRoad, dir: f64) -> (f64, f64) {
    if road.horizontal {
        (dir, 0.0)
    } else {
        (0.0, dir)
    }
}

fn lane_pose(road: &CRoad, dir: f64, s: f64, lat: f64) -> (f64, f64, f64) {
    if road.horizontal {
        (s, road.c + lat, if dir > 0.0 { 0.0 } else { PI })
    } else {
        (
            road.c - lat,
            s,
            if dir > 0.0 { FRAC_PI_2 } else { -FRAC_PI_2 },
        )
    }
}

fn wrap(a: f64) -> f64 {
    (a + PI).rem_euclid(2.0 * PI) - PI
}

/// Geometry of a turn at a junction: arc start position along the road, and the arc.
fn turn_arc(roads: &[CRoad], road: usize, dir: f64, plan: &TurnPlan) -> Option<(f64, Route)> {
    if plan.turn == 0 {
        return None;
    }
    let r_in = &roads[road];
    let r_out = &roads[plan.other];
    let d_in = dir_vec(r_in, dir);
    let d_out = if plan.turn < 0 {
        (d_in.1, -d_in.0)
    } else {
        (-d_in.1, d_in.0)
    };
    let dir_out = if r_out.horizontal { d_out.0 } else { d_out.1 };
    let lat_in = -dir * OUTER;
    let lat_out = -dir_out * OUTER;
    let q = if r_in.horizontal {
        (r_out.c - lat_out, r_in.c + lat_in)
    } else {
        (r_in.c - lat_in, r_out.c + lat_out)
    };
    let radius = if plan.turn < 0 { 6.0 } else { 10.0 };
    let start = (q.0 - radius * d_in.0, q.1 - radius * d_in.1);
    let centre = (start.0 + radius * d_out.0, start.1 + radius * d_out.1);
    let end = (q.0 + radius * d_out.0, q.1 + radius * d_out.1);
    let s_start = if r_in.horizontal { start.0 } else { start.1 };
    let s_end = if r_out.horizontal { end.0 } else { end.1 };
    Some((
        s_start,
        Route::Arc {
            cx: centre.0,
            cy: centre.1,
            r: radius,
            phi: (start.1 - centre.1).atan2(start.0 - centre.0),
            left: FRAC_PI_2,
            ccw: plan.turn > 0,
            next: (plan.other, dir_out, s_end, lat_out),
        },
    ))
}

fn u_turn(roads: &[CRoad], road: usize, dir: f64, s: f64) -> Route {
    let r = &roads[road];
    let (px, py, _) = lane_pose(r, dir, s, -dir * OUTER);
    let d = dir_vec(r, dir);
    let (cx, cy) = (px - OUTER * d.1, py + OUTER * d.0);
    Route::Arc {
        cx,
        cy,
        r: OUTER,
        phi: (py - cy).atan2(px - cx),
        left: PI,
        ccw: true,
        next: (road, -dir, s, dir * OUTER),
    }
}

fn arc_speed(r: f64) -> f64 {
    (2.5 * r).sqrt()
}

/// Advance a route by `dist` metres; returns the new route and pose.
fn advance(
    roads: &[CRoad],
    route: Route,
    plan: Option<&TurnPlan>,
    dist: f64,
    lat_step: f64,
) -> (Route, (f64, f64, f64)) {
    match route {
        Route::Lane {
            road,
            dir,
            s,
            lat,
            lat_target,
        } => {
            let r = &roads[road];
            let ns = s + dir * dist;
            let nlat = lat + (lat_target - lat).clamp(-lat_step, lat_step);
            // start a planned turn, or a U-turn at the end of the road
            if let Some(p) = plan
                && let Some((s_start, arc)) = turn_arc(roads, road, dir, p)
                && (ns - s_start) * dir >= 0.0
                && (s - s_start) * dir < 0.0 + 1e-9
            {
                return advance(roads, arc, None, ((ns - s_start) * dir).max(0.0), 0.0);
            }
            if (dir > 0.0 && ns >= r.len - 15.0) || (dir < 0.0 && ns <= 15.0) {
                let arc = u_turn(roads, road, dir, s);
                return advance(roads, arc, None, dist, 0.0);
            }
            let (x, y, h) = lane_pose(r, dir, ns, nlat);
            // a lane change turns the vehicle slightly
            let h = if dist > 1e-6 {
                h + (dir * (nlat - lat)).atan2(dist)
            } else {
                h
            };
            (
                Route::Lane {
                    road,
                    dir,
                    s: ns,
                    lat: nlat,
                    lat_target,
                },
                (x, y, h),
            )
        }
        Route::Arc {
            cx,
            cy,
            r,
            phi,
            left,
            ccw,
            next,
        } => {
            let dphi = dist / r;
            if dphi >= left {
                let (road, dir, s, lat) = next;
                let lane = Route::Lane {
                    road,
                    dir,
                    s,
                    lat,
                    lat_target: lat,
                };
                return advance(roads, lane, None, (dphi - left) * r, 0.0);
            }
            let nphi = if ccw { phi + dphi } else { phi - dphi };
            let h = if ccw {
                nphi + FRAC_PI_2
            } else {
                nphi - FRAC_PI_2
            };
            (
                Route::Arc {
                    cx,
                    cy,
                    r,
                    phi: nphi,
                    left: left - dphi,
                    ccw,
                    next,
                },
                (cx + r * nphi.cos(), cy + r * nphi.sin(), wrap(h)),
            )
        }
    }
}

// ---------------------------------------------------------------- identities

/// Pseudonyms without signatures, for runs with signature checks off.
#[derive(Debug, Clone)]
struct PlainSigner {
    ids: Vec<TempId>,
    rotate_s: f64,
}

impl Signer for PlainSigner {
    fn current_temp_id(&mut self, now: f64) -> Option<TempId> {
        let i = ((now / self.rotate_s).floor().max(0.0) as usize) % self.ids.len().max(1);
        self.ids.get(i).copied()
    }
    fn sign(&mut self, _: f64, _: bool, _: &[u8; BODY_LEN]) -> Option<Vec<u8>> {
        Some(Vec::new())
    }
}

#[derive(Debug, Clone)]
enum Identity {
    Plain(PlainSigner),
    Real(Box<PseudonymSigner<SoftKeys>>),
}

impl Signer for Identity {
    fn current_temp_id(&mut self, now: f64) -> Option<TempId> {
        match self {
            Self::Plain(p) => p.current_temp_id(now),
            Self::Real(p) => p.current_temp_id(now),
        }
    }
    fn sign(&mut self, now: f64, ev: bool, body: &[u8; BODY_LEN]) -> Option<Vec<u8>> {
        match self {
            Self::Plain(p) => p.sign(now, ev, body),
            Self::Real(p) => p.sign(now, ev, body),
        }
    }
}

// ---------------------------------------------------------------- metrics

#[derive(Debug, Clone)]
struct Episode {
    note: String,
    start: f64,
    unannounced_turn: bool,
    contact_est: f64,
    last_conflict: f64,
    critical: bool,
    warned_in_time: Option<f64>,
    hinted: bool,
}

#[derive(Debug, Clone)]
struct AlertEp {
    note: String,
    kind: AlertKind,
    danger: bool,
    start: f64,
    last: f64,
    confirmed: bool,
}

/// Results of one run.
#[derive(Debug, Clone, Default)]
pub struct CityStats {
    /// Conflicts between two app users.
    pub conflicts: u64,
    /// ... warned in time (Warning/Danger before the touch came within 1.5 s).
    pub warned: u64,
    /// ... missed.
    pub missed: u64,
    /// ... missed, but a Caution "possible conflict" was shown.
    pub missed_with_caution: u64,
    /// Warning times (s) of the warned conflicts.
    pub lead_times: Vec<f64>,
    /// Conflicts and misses for pairs including a motorbike.
    pub moto_conflicts: u64,
    /// See `moto_conflicts`.
    pub moto_missed: u64,
    /// Conflicts involving a vehicle without the app (cannot be warned).
    pub conflicts_unequipped: u64,
    /// Missed because the two phones never heard each other (radio range, buildings, loss).
    pub missed_not_heard: u64,
    /// Conflicts / missed where a driver was about to turn and had not started yet: no
    /// phone can know a turn before it begins (no indicator input).
    pub turn_conflicts: u64,
    /// See `turn_conflicts`.
    pub turn_missed: u64,
    /// Conflicts / missed where both phones' GPS is within the SPEC §6 confidence gate
    /// (8 m); above it only a Caution is allowed by design.
    pub gated_conflicts: u64,
    /// See `gated_conflicts`.
    pub gated_missed: u64,
    /// Within the gate, conflicts / missed where a driver was about to turn and had not
    /// started yet (reported separately, user decision Step 4).
    pub gated_turn_conflicts: u64,
    /// See `gated_turn_conflicts`.
    pub gated_turn_missed: u64,
    /// Why conflicts were missed, one cause each (in this order): never heard by radio,
    /// a phone's GPS beyond the confidence gate, a turn not yet started, other.
    pub miss_causes: [u64; 4],
    /// Warning/Danger alert episodes about another vehicle.
    pub alerts: u64,
    /// ... with no near miss within 6 s: false alarms.
    pub false_alarms: u64,
    /// Hours driven by app users.
    pub app_hours: f64,
    /// Collisions: both with the app, one, none.
    pub collisions: [u64; 3],
    /// Frames sent / delivered / lost.
    pub frames: [u64; 3],
    /// Vehicles at the start.
    pub vehicles: usize,
    /// Honest senders a phone blocked as misbehaving (should be 0).
    pub honest_blocked: u64,
    /// Kilometres driven by app users.
    pub app_km: f64,
    /// Alert episodes and false alarms per alert kind: (alerts, false, false Danger).
    pub by_kind: BTreeMap<String, (u64, u64, u64)>,
    /// Own messages sent (hop 0) and relayed copies sent.
    pub originals: u64,
    /// See `originals`.
    pub relays: u64,
    /// Relayed copies of events (hard braking, stopped traffic).
    pub event_relays: u64,
    /// Own events sent.
    pub events: u64,
    /// A phone relayed the same message twice (SPEC §5 rule 2: must be 0).
    pub duplicate_relays: u64,
}

impl CityStats {
    /// Add another run's results.
    pub fn merge(&mut self, o: &CityStats) {
        self.conflicts += o.conflicts;
        self.warned += o.warned;
        self.missed += o.missed;
        self.missed_with_caution += o.missed_with_caution;
        self.lead_times.extend(&o.lead_times);
        self.moto_conflicts += o.moto_conflicts;
        self.moto_missed += o.moto_missed;
        self.conflicts_unequipped += o.conflicts_unequipped;
        self.missed_not_heard += o.missed_not_heard;
        self.turn_conflicts += o.turn_conflicts;
        self.turn_missed += o.turn_missed;
        self.gated_conflicts += o.gated_conflicts;
        self.gated_missed += o.gated_missed;
        self.gated_turn_conflicts += o.gated_turn_conflicts;
        self.gated_turn_missed += o.gated_turn_missed;
        for k in 0..4 {
            self.miss_causes[k] += o.miss_causes[k];
        }
        self.alerts += o.alerts;
        self.false_alarms += o.false_alarms;
        self.app_hours += o.app_hours;
        for k in 0..3 {
            self.collisions[k] += o.collisions[k];
            self.frames[k] += o.frames[k];
        }
        self.vehicles += o.vehicles;
        self.honest_blocked += o.honest_blocked;
        self.app_km += o.app_km;
        self.originals += o.originals;
        self.relays += o.relays;
        self.event_relays += o.event_relays;
        self.events += o.events;
        self.duplicate_relays += o.duplicate_relays;
        for (k, v) in &o.by_kind {
            let e = self.by_kind.entry(k.clone()).or_default();
            e.0 += v.0;
            e.1 += v.1;
            e.2 += v.2;
        }
    }

    /// False Danger alerts per 100 km driven (SPEC §8 budget: under 1).
    pub fn false_danger_per_100km(&self) -> f64 {
        if self.app_km <= 0.0 {
            0.0
        } else {
            self.by_kind.values().map(|v| v.2).sum::<u64>() as f64 * 100.0 / self.app_km
        }
    }

    /// The Step 4 target metric: missed share for pairs within the GPS gate, leaving out
    /// turns that had not started (0–1).
    pub fn target_missed_rate(&self) -> f64 {
        let n = self.gated_conflicts - self.gated_turn_conflicts;
        if n == 0 {
            0.0
        } else {
            (self.gated_missed - self.gated_turn_missed) as f64 / n as f64
        }
    }

    /// Missed share (0–1).
    pub fn missed_rate(&self) -> f64 {
        if self.conflicts == 0 {
            0.0
        } else {
            self.missed as f64 / self.conflicts as f64
        }
    }

    /// False alarms per app-driving hour.
    pub fn false_alarms_per_hour(&self) -> f64 {
        if self.app_hours <= 0.0 {
            0.0
        } else {
            self.false_alarms as f64 / self.app_hours
        }
    }

    /// Mean and median warning time (s).
    pub fn lead(&self) -> (f64, f64) {
        if self.lead_times.is_empty() {
            return (0.0, 0.0);
        }
        let mut v = self.lead_times.clone();
        v.sort_by(f64::total_cmp);
        (v.iter().sum::<f64>() / v.len() as f64, v[v.len() / 2])
    }
}

// ---------------------------------------------------------------- the city

/// The simulated city.
#[derive(Debug)]
pub struct City {
    /// Options.
    pub opts: CityOptions,
    /// Vehicles.
    pub vehicles: Vec<CityVehicle>,
    /// Time (s).
    pub t: f64,
    /// Results so far.
    pub stats: CityStats,
    roads: Vec<CRoad>,
    map: Arc<RoadMap>,
    blocks: Vec<Vec<Point>>,
    obus: Vec<Option<Obu>>,
    ids: Vec<Option<Identity>>,
    verifiers: Vec<Option<SimVerifier>>,
    owner: BTreeMap<TempId, usize>,
    decisions: Vec<Option<Decision>>,
    rng: SimRng,
    noise_rng: SimRng,
    radio_rng: SimRng,
    episodes: BTreeMap<(usize, usize), Episode>,
    alert_eps: BTreeMap<(usize, usize), AlertEp>,
    last_near: BTreeMap<(usize, usize), f64>,
    /// (phone, vehicle) → (start, last): a Warning/Danger was up and its braking advice
    /// accounted for that vehicle.
    advising: BTreeMap<(usize, usize), (f64, f64)>,
    pending_own: Vec<Option<OwnState>>,
    /// (phone, sender pseudonym, message id) of every relayed copy sent.
    relayed: HashSet<(usize, [u8; 4], u16)>,
}

fn debug() -> bool {
    std::env::var_os("CITY_DEBUG").is_some()
}

fn gauss(rng: &mut SimRng) -> f64 {
    let u1 = rng.next_f64().max(1e-12);
    let u2 = rng.next_f64();
    (-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos()
}

fn pick<T: Copy>(rng: &mut SimRng, options: &[(T, f64)]) -> T {
    let r = rng.next_f64();
    let mut acc = 0.0;
    for &(v, p) in options {
        acc += p;
        if r < acc {
            return v;
        }
    }
    options[options.len() - 1].0
}

fn key(i: usize, j: usize) -> (usize, usize) {
    if i < j { (i, j) } else { (j, i) }
}

impl City {
    /// Build a city.
    pub fn new(opts: CityOptions) -> Self {
        let roads = city_roads();
        let map = Arc::new(city_map());
        let blocks = city_blocks();
        let mut rng = SimRng::new(opts.seed);
        let noise_rng = SimRng::new(opts.seed + 7);
        let radio_rng = SimRng::new(opts.seed + 13);
        let mut ca_rng = SimRng::new(opts.seed + 1000);
        let mut ca = new_ca(&mut ca_rng, opts.config.clone());
        let mut id_rng = SimRng::new(opts.seed + 500);
        let anchor = ca.trust_anchor();
        let mut vehicles: Vec<CityVehicle> = Vec::new();
        let mut tries = 0;
        while vehicles.len() < opts.vehicles && tries < opts.vehicles * 50 {
            tries += 1;
            let road = (rng.next_f64() * roads.len() as f64) as usize % roads.len();
            let r = &roads[road];
            let dir = if rng.next_f64() < 0.5 { 1.0 } else { -1.0 };
            let s = 40.0 + rng.next_f64() * (r.len - 80.0);
            if r.crossings.iter().any(|(cs, _)| (cs - s).abs() < 30.0) {
                continue;
            }
            let lat = -dir * OUTER;
            let (x, y, h) = lane_pose(r, dir, s, lat);
            if vehicles.iter().any(|v| (v.x - x).hypot(v.y - y) < 14.0) {
                continue;
            }
            let kind = pick(
                &mut rng,
                &[
                    (VehicleType::Motorbike, 0.55),
                    (VehicleType::Car, 0.30),
                    (VehicleType::TukTuk, 0.08),
                    (VehicleType::Truck, 0.07),
                ],
            );
            let temper = pick(
                &mut rng,
                &[
                    (Temper::Calm, 0.3),
                    (Temper::Normal, 0.5),
                    (Temper::Aggressive, 0.2),
                ],
            );
            let has_app = rng.next_f64() < opts.penetration;
            let sigma = opts.gps_m.0 + rng.next_f64() * (opts.gps_m.1 - opts.gps_m.0);
            let target = r.limit * profile(temper).speed_f * type_speed_f(kind);
            let speed = target * (0.6 + 0.4 * rng.next_f64());
            let bsd = sigma * (0.9f64 / 2.0).sqrt();
            let gps = GpsNoise {
                sigma,
                bx: bsd * gauss(&mut rng),
                by: bsd * gauss(&mut rng),
                last_heading: h,
            };
            vehicles.push(CityVehicle {
                kind,
                temper,
                has_app,
                x,
                y,
                heading: h,
                speed,
                accel: 0.0,
                yaw_rate: 0.0,
                crashed: None,
                gone: false,
                gps_sigma_m: sigma,
                route: Route::Lane {
                    road,
                    dir,
                    s,
                    lat,
                    lat_target: lat,
                },
                plan: None,
                bias: 0.0,
                bias_t: 5.0 + 10.0 * rng.next_f64(),
                brake_t: 0.0,
                overtaking: None,
                gps,
                noticed: BTreeMap::new(),
                alert_since: None,
            });
        }
        let mut obus = Vec::new();
        let mut ids = Vec::new();
        let mut verifiers = Vec::new();
        let mut owner = BTreeMap::new();
        let origin = GeoOrigin::phnom_penh();
        for (i, v) in vehicles.iter().enumerate() {
            if !v.has_app {
                obus.push(None);
                ids.push(None);
                verifiers.push(None);
                continue;
            }
            let mut obu = Obu::new(opts.config.clone(), origin, opts.relay);
            obu.set_map(map.clone());
            obus.push(Some(obu));
            if opts.security {
                let s = enrol_with(&mut ca, &mut ca_rng, 3, 0, &opts.config.security);
                for c in s.certificates() {
                    owner.insert(c.temp_id(), i);
                }
                ids.push(Some(Identity::Real(Box::new(s))));
                verifiers.push(Some(
                    match CertVerifier::new(&[anchor], &opts.config.security) {
                        Ok(v) => SimVerifier::Real(Box::new(v)),
                        Err(_) => SimVerifier::Off,
                    },
                ));
            } else {
                let p = PlainSigner {
                    ids: (0..3).map(|_| id_rng.bytes::<4>()).collect(),
                    rotate_s: opts.config.security.pseudonym_rotate_s,
                };
                for id in &p.ids {
                    owner.insert(*id, i);
                }
                ids.push(Some(Identity::Plain(p)));
                verifiers.push(Some(SimVerifier::Off));
            }
        }
        let n = vehicles.len();
        let stats = CityStats {
            vehicles: n,
            ..CityStats::default()
        };
        Self {
            opts,
            vehicles,
            t: 0.0,
            stats,
            roads,
            map,
            blocks,
            obus,
            ids,
            verifiers,
            owner,
            decisions: vec![None; n],
            rng,
            noise_rng,
            radio_rng,
            episodes: BTreeMap::new(),
            alert_eps: BTreeMap::new(),
            last_near: BTreeMap::new(),
            advising: BTreeMap::new(),
            pending_own: Vec::new(),
            relayed: HashSet::new(),
        }
    }

    fn line_of_sight(&self, a: (f64, f64), b: (f64, f64)) -> bool {
        !self.blocks.iter().any(|p| segment_hits_polygon(a, b, p))
    }

    /// Run to the end and return the results.
    pub fn run(mut self) -> CityStats {
        while self.t < self.opts.duration_s - 1e-9 {
            self.step();
        }
        self.finish();
        self.stats
    }

    // ---------------------------------------------------------- one tick

    /// One 100 ms step.
    pub fn step(&mut self) {
        let now = self.t;
        self.radio(now);
        self.phones(now);
        self.drive(now);
        self.collisions(now);
        self.judge(now);
        for v in &self.vehicles {
            if v.has_app && v.active() {
                self.stats.app_hours += TICK / 3600.0;
                self.stats.app_km += v.speed * TICK / 1000.0;
            }
        }
        self.t = ((now + TICK) * 1e6).round() / 1e6;
    }

    fn noisy_own(&mut self, i: usize) -> OwnState {
        let v = &mut self.vehicles[i];
        let g = &mut v.gps;
        let tau = 30.0;
        let a = (-TICK / tau).exp();
        let bsd = g.sigma * (0.9f64 / 2.0).sqrt();
        let wsd = g.sigma * (0.1f64 / 2.0).sqrt();
        let q = bsd * (1.0 - a * a).sqrt();
        g.bx = a * g.bx + q * gauss(&mut self.noise_rng);
        g.by = a * g.by + q * gauss(&mut self.noise_rng);
        let x = v.x + g.bx + wsd * gauss(&mut self.noise_rng);
        let y = v.y + g.by + wsd * gauss(&mut self.noise_rng);
        let heading = if v.speed > 1.0 {
            g.last_heading = v.heading + 1.5f64.to_radians() * gauss(&mut self.noise_rng);
            g.last_heading
        } else {
            g.last_heading
        };
        OwnState {
            motion: Motion {
                x,
                y,
                heading,
                speed: (v.speed + 0.15 * gauss(&mut self.noise_rng)).max(0.0),
                accel: v.accel + 0.2 * gauss(&mut self.noise_rng),
                yaw_rate: v.yaw_rate + 0.01 * gauss(&mut self.noise_rng),
            },
            vehicle_type: v.kind,
            pos_accuracy_m: g.sigma,
        }
    }

    // several per-vehicle vectors are indexed together
    #[allow(clippy::needless_range_loop)]
    fn radio(&mut self, now: f64) {
        let n = self.vehicles.len();
        let mut own_states: Vec<Option<OwnState>> = vec![None; n];
        for i in 0..n {
            if self.obus[i].is_some() && self.vehicles[i].active() {
                own_states[i] = Some(self.noisy_own(i));
            }
        }
        // transmit
        let mut frames: Vec<(usize, Frame)> = Vec::new();
        for i in 0..n {
            let (Some(own), Some(obu), Some(id)) = (
                own_states[i].as_ref(),
                self.obus[i].as_mut(),
                self.ids[i].as_mut(),
            ) else {
                continue;
            };
            let road = RoadContext {
                speed_limit: self.map.limit_at(own.motion.x, own.motion.y),
                at_junction: self.map.distance_to_junction(own.motion.x, own.motion.y) < 150.0,
            };
            for f in obu.transmit(now, own, &road, id) {
                if let Ok(m) = roadmesh_core::rsm::Rsm::decode(&f.body) {
                    let event = m.msg_type == roadmesh_core::rsm::MsgType::Event;
                    if m.hop_count == 0 {
                        self.stats.originals += 1;
                        self.stats.events += u64::from(event);
                    } else {
                        self.stats.relays += 1;
                        self.stats.event_relays += u64::from(event);
                        if !self.relayed.insert((i, m.temp_id, m.msg_id)) {
                            self.stats.duplicate_relays += 1;
                        }
                    }
                }
                frames.push((i, f));
            }
        }
        // deliver
        for (sender, f) in frames {
            self.stats.frames[0] += 1;
            let sp = (self.vehicles[sender].x, self.vehicles[sender].y);
            for r in 0..n {
                if r == sender || own_states[r].is_none() {
                    continue;
                }
                let rp = (self.vehicles[r].x, self.vehicles[r].y);
                let d = (sp.0 - rp.0).hypot(sp.1 - rp.1);
                if d > self.opts.radio_los_m {
                    continue;
                }
                if d > self.opts.radio_nlos_m && !self.line_of_sight(sp, rp) {
                    continue;
                }
                if self.radio_rng.next_f64() < self.opts.loss {
                    self.stats.frames[2] += 1;
                    continue;
                }
                self.stats.frames[1] += 1;
                let (Some(own), Some(obu), Some(ver)) = (
                    own_states[r].as_ref(),
                    self.obus[r].as_mut(),
                    self.verifiers[r].as_mut(),
                ) else {
                    continue;
                };
                let out = obu.receive(&f, now, own, ver);
                if debug()
                    && let roadmesh_core::obu::RxOutcome::Misbehaviour {
                        reason,
                        report: true,
                        ..
                    } = out
                {
                    let v = &self.vehicles[sender];
                    eprintln!(
                        "BLOCKED t={now:.1} sender #{sender} {} v{:.1} a{:.1} crashed={} gps{:.1}: {reason:?}",
                        type_label(v.kind),
                        v.speed,
                        v.accel,
                        v.crashed.is_some(),
                        v.gps_sigma_m
                    );
                }
            }
        }
        self.pending_own = own_states;
    }

    fn phones(&mut self, now: f64) {
        let n = self.vehicles.len();
        let states = std::mem::take(&mut self.pending_own);
        for i in 0..n {
            let (Some(own), Some(obu)) = (
                states.get(i).and_then(|s| s.as_ref()),
                self.obus[i].as_mut(),
            ) else {
                continue;
            };
            let road = RoadContext {
                speed_limit: self.map.limit_at(own.motion.x, own.motion.y),
                at_junction: self.map.distance_to_junction(own.motion.x, own.motion.y) < 150.0,
            };
            let d = obu.decide(now, TICK, own, &road, self.map.as_ref());
            self.decisions[i] = Some(d);
        }
    }

    // ---------------------------------------------------------- drivers

    fn drive(&mut self, now: f64) {
        let n = self.vehicles.len();
        let mut next: Vec<CityVehicle> = Vec::with_capacity(n);
        for i in 0..n {
            let mut v = self.vehicles[i].clone();
            if v.active() && v.crashed.is_none() {
                v.accel = self.driver(i, &mut v, now);
            }
            next.push(v);
        }
        for v in next.iter_mut() {
            if v.gone {
                continue;
            }
            if let Some(tc) = v.crashed {
                v.speed = 0.0;
                v.accel = 0.0;
                v.yaw_rate = 0.0;
                if now - tc >= 5.0 {
                    v.gone = true;
                }
                continue;
            }
            let v0 = v.speed;
            let v1 = (v0 + v.accel * TICK).max(0.0);
            let dist = 0.5 * (v0 + v1) * TICK;
            let was_lane = matches!(v.route, Route::Lane { .. });
            let (route, (x, y, h)) =
                advance(&self.roads, v.route, v.plan.as_ref(), dist, 1.5 * TICK);
            if was_lane && matches!(route, Route::Arc { .. }) {
                v.plan = None; // the turn (or U-turn) is under way
            }
            v.yaw_rate = wrap(h - v.heading) / TICK;
            v.route = route;
            v.x = x;
            v.y = y;
            v.heading = h;
            v.accel = (v1 - v0) / TICK;
            v.speed = v1;
        }
        self.vehicles = next;
    }

    /// The driver of vehicle `i` (state in `v`): returns the acceleration.
    fn driver(&mut self, i: usize, v: &mut CityVehicle, now: f64) -> f64 {
        let p = profile(v.temper);
        v.bias_t -= TICK;
        if v.bias_t <= 0.0 {
            let r = self.rng.next_f64();
            v.bias = if v.temper == Temper::Aggressive {
                -1.0 + 7.0 * r
            } else {
                -2.0 + 4.0 * r
            };
            v.bias_t = 8.0 + 12.0 * self.rng.next_f64();
        }
        if v.brake_t <= 0.0 && self.rng.next_f64() < p.brake_rate * TICK {
            v.brake_t = 0.8 + 0.7 * self.rng.next_f64(); // sudden braking, for no visible reason
        }
        let limit = match v.route {
            Route::Lane { road, .. } => self.roads[road].limit,
            Route::Arc { next, r, .. } => self.roads[next.0].limit.min(arc_speed(r)),
        };
        let mut desired = (limit * p.speed_f * type_speed_f(v.kind) + v.bias).max(1.0);
        if let Route::Arc { r, .. } = v.route {
            desired = desired.min(arc_speed(r));
        }
        if let Route::Lane {
            road,
            dir,
            s,
            lat_target,
            ..
        } = v.route
        {
            let r = self.roads[road].clone();
            // the next junction ahead
            let ahead = r
                .crossings
                .iter()
                .map(|&(cs, other)| (cs, other, (cs - s) * dir))
                .filter(|x| x.2 > -JUNCTION_R)
                .min_by(|a, b| a.2.total_cmp(&b.2));
            if let Some((cs, other, d)) = ahead {
                if v.plan.is_none_or(|pl| (pl.at_s - cs).abs() > 1.0) && d < 60.0 && d > 15.0 {
                    let turn = pick(&mut self.rng, &[(0i8, 0.7), (-1, 0.15), (1, 0.15)]);
                    let looks = self.rng.next_f64() < p.look_prob;
                    v.plan = Some(TurnPlan {
                        at_s: cs,
                        other,
                        turn,
                        looks,
                    });
                }
                if d > 0.0
                    && d < 40.0
                    && let Some(cap) = p.approach_cap
                {
                    desired = desired.min(cap);
                }
                if let Some(pl) = v.plan
                    && (pl.at_s - cs).abs() < 1.0
                {
                    if let Some((s_start, Route::Arc { r: rad, .. })) =
                        turn_arc(&self.roads, road, dir, &pl)
                    {
                        let ds = (s_start - s) * dir;
                        if ds > -1.0 {
                            desired =
                                desired.min((arc_speed(rad).powi(2) + 4.0 * ds.max(0.0)).sqrt());
                        }
                    }
                    if pl.looks
                        && d > 12.0
                        && d < 55.0
                        && self.must_give_way(i, v, road, dir, cs, d, pl.turn, now)
                    {
                        desired = desired.min((6.0 * (d - 14.0).max(0.0)).sqrt());
                    }
                }
            }
            // slow for the U-turn at the end of the road
            let d_end = if dir > 0.0 {
                r.len - 15.0 - s
            } else {
                s - 15.0
            };
            desired = desired.min((arc_speed(OUTER).powi(2) + 4.0 * d_end.max(0.0)).sqrt());
            // overtaking in the inner lane (never close to a junction or the road end)
            let near_junction = ahead.is_some_and(|a| a.2 < 80.0) || d_end < 80.0;
            if let Some(o) = v.overtaking {
                let done = match self.vehicles[o].route {
                    Route::Lane {
                        s: so,
                        road: ro,
                        dir: dro,
                        ..
                    } => ro != road || dro != dir || (s - so) * dir > 12.0,
                    Route::Arc { .. } => true,
                };
                if done || near_junction {
                    v.overtaking = None;
                }
            }
            let target = if v.overtaking.is_some() {
                -dir * INNER
            } else {
                -dir * OUTER
            };
            if (target - lat_target).abs() > 1e-9
                && let Route::Lane { lat_target: lt, .. } = &mut v.route
            {
                *lt = target;
            }
            if v.overtaking.is_none()
                && !near_junction
                && let Some((li, gap, lv)) = self.leader(i, v)
                && gap < (3.0 + v.speed * p.headway_s) * 1.7
                && lv < desired - 1.5
                && self.rng.next_f64() < p.overtake * TICK
                && self.inner_lane_clear(i, road, dir, s)
            {
                v.overtaking = Some(li);
            }
        }
        // car following (anything ahead in my path, including crossing traffic and wrecks)
        if let Some((_, gap, lv)) = self.leader(i, v) {
            let safe = 3.0 + v.speed * p.headway_s;
            if gap < safe {
                desired = desired.min(lv - (safe - gap) * 0.45);
            }
            if gap < 1.5 {
                desired = 0.0;
            }
        }
        let mut a = if v.brake_t > 0.0 {
            v.brake_t -= TICK;
            -6.5
        } else {
            ((desired - v.speed) / 1.5).clamp(-7.0, p.accel)
        };
        // the app: act on Warning/Danger after the reaction time, following the advice
        if v.has_app {
            let d = self.decisions[i].as_ref();
            let serious = d.is_some_and(|d| d.alerts.iter().any(|al| al.level >= Level::Warning));
            if serious {
                let since = *v.alert_since.get_or_insert(now);
                if now - since >= self.opts.app_reaction_s - 1e-9 {
                    let advised = d.and_then(|d| d.advised_accel).unwrap_or(0.0);
                    a = a.min(advised);
                }
            } else {
                v.alert_since = None;
            }
        }
        a
    }

    /// The closest vehicle ahead in my path: (index, gap m, its speed along my heading).
    fn leader(&self, i: usize, v: &CityVehicle) -> Option<(usize, f64, f64)> {
        let (s, c) = v.heading.sin_cos();
        let (l, w) = v.dims();
        let mut best: Option<(usize, f64, f64)> = None;
        for (j, o) in self.vehicles.iter().enumerate() {
            if j == i || o.gone {
                continue;
            }
            let (dx, dy) = (o.x - v.x, o.y - v.y);
            let along = dx * c + dy * s;
            if along <= 0.0 || along > 100.0 {
                continue;
            }
            let lat = -dx * s + dy * c;
            let (ol, ow) = o.dims();
            let dh = wrap(o.heading - v.heading).abs();
            if lat.abs() > (w + ow) / 2.0 + 0.4 || (dh > PI / 3.0 && along > 25.0) {
                continue;
            }
            let gap = along - (l + ol) / 2.0;
            if best.is_none_or(|b| gap < b.1) {
                best = Some((j, gap, o.speed * dh.cos().max(0.0)));
            }
        }
        best
    }

    fn inner_lane_clear(&self, i: usize, road: usize, dir: f64, s: f64) -> bool {
        self.vehicles.iter().enumerate().all(|(j, o)| {
            j == i
                || o.gone
                || !matches!(o.route, Route::Lane { road: r, dir: d, s: so, lat, .. }
                    if r == road && d == dir && (lat - (-dir * INNER)).abs() < 1.6 && (so - s).abs() < 30.0)
        })
    }

    /// Human judgement at a junction: is there crossing traffic I can see, have
    /// noticed, and that will get there first?
    #[allow(clippy::too_many_arguments)]
    fn must_give_way(
        &self,
        i: usize,
        v: &mut CityVehicle,
        road: usize,
        dir: f64,
        cs: f64,
        d: f64,
        turn: i8,
        now: f64,
    ) -> bool {
        let p = profile(v.temper);
        let (jx, jy, _) = lane_pose(&self.roads[road], dir, cs, 0.0);
        let tta_me = d / v.speed.max(1.5);
        let mut give_way = false;
        let mut seen_now = Vec::new();
        for (j, o) in self.vehicles.iter().enumerate() {
            if j == i || o.gone {
                continue;
            }
            let d_o = match o.route {
                Route::Lane {
                    road: ro,
                    dir: dro,
                    s: so,
                    ..
                } => {
                    let r_o = &self.roads[ro];
                    if ro == road {
                        if turn > 0 && dro == -dir {
                            Some((cs - so) * dro) // oncoming traffic when I turn left
                        } else {
                            None
                        }
                    } else if r_o.horizontal != self.roads[road].horizontal {
                        let s_j = self.roads[road].c;
                        if (r_o.c - if self.roads[road].horizontal { jx } else { jy }).abs() < 1.0 {
                            Some((s_j - so) * dro)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                Route::Arc { .. } => ((o.x - jx).hypot(o.y - jy) < 25.0).then_some(0.0),
            };
            let Some(d_o) = d_o else { continue };
            if !(-6.0..70.0).contains(&d_o) {
                continue;
            }
            let dist = (o.x - v.x).hypot(o.y - v.y);
            if dist > 120.0 || !self.line_of_sight((v.x, v.y), (o.x, o.y)) {
                continue;
            }
            seen_now.push(j);
            let first = *v.noticed.entry(j).or_insert(now);
            if now - first < p.notice_s {
                continue;
            }
            let tta_o = d_o.max(0.0) / o.speed.max(1.5);
            let blocking = o.speed < 1.0 && d_o.abs() < 12.0;
            if blocking || ((tta_o - tta_me).abs() < 2.5 && tta_o <= tta_me + 0.5) {
                give_way = true;
            }
        }
        v.noticed.retain(|j, _| seen_now.contains(j));
        give_way
    }

    // ---------------------------------------------------------- crashes

    fn collisions(&mut self, now: f64) {
        let n = self.vehicles.len();
        for i in 0..n {
            for j in (i + 1)..n {
                let (a, b) = (&self.vehicles[i], &self.vehicles[j]);
                if a.gone || b.gone || (a.crashed.is_some() && b.crashed.is_some()) {
                    continue;
                }
                if (a.x - b.x).hypot(a.y - b.y) > 15.0 {
                    continue;
                }
                if polygons_overlap(&a.footprint(), &b.footprint()) {
                    let apps = usize::from(a.has_app) + usize::from(b.has_app);
                    self.stats.collisions[2 - apps] += 1;
                    for k in [i, j] {
                        self.vehicles[k].crashed.get_or_insert(now);
                    }
                    if let Some(ep) = self.episodes.get_mut(&key(i, j)) {
                        ep.critical = true;
                    }
                }
            }
        }
    }

    // ---------------------------------------------------------- ground truth

    /// The vehicle's real future path with no reaction: its route (including the
    /// planned turn) at its current acceleration.
    fn future_path(&self, v: &CityVehicle) -> Vec<Pose> {
        let n = (ORACLE_HORIZON_S / TICK).round() as usize;
        let mut out = Vec::with_capacity(n);
        let (mut route, mut plan, mut speed) = (v.route, v.plan, v.speed);
        let moving = v.crashed.is_none();
        let (mut x, mut y, mut h) = (v.x, v.y, v.heading);
        // like standard time-to-collision: braking continues, speeding up does not
        let accel = v.accel.min(0.0);
        for k in 1..=n {
            if moving {
                let s1 = (speed + accel * TICK).max(0.0);
                let dist = 0.5 * (speed + s1) * TICK;
                speed = s1;
                let was_lane = matches!(route, Route::Lane { .. });
                let (r2, pose) = advance(&self.roads, route, plan.as_ref(), dist, 1.5 * TICK);
                if was_lane && matches!(r2, Route::Arc { .. }) {
                    plan = None;
                }
                route = r2;
                (x, y, h) = pose;
            }
            out.push((k as f64 * TICK, x, y, h));
        }
        out
    }

    fn alerting(&self, i: usize, j: usize) -> Option<f64> {
        let direct = self
            .alert_eps
            .get(&(i, j))
            .filter(|e| self.t - e.last < ALERT_LOOKBACK_S)
            .map(|e| e.start);
        let advice = self
            .advising
            .get(&(i, j))
            .filter(|e| self.t - e.1 < ALERT_LOOKBACK_S)
            .map(|e| e.0);
        direct.into_iter().chain(advice).reduce(f64::min)
    }

    fn judge(&mut self, now: f64) {
        let n = self.vehicles.len();
        if let Ok(tr) = std::env::var("CITY_TRACE") {
            let ids: Vec<usize> = tr.split(',').filter_map(|x| x.parse().ok()).collect();
            if let [i, j, t0, t1] = ids[..]
                && (t0 as f64) <= now
                && now <= t1 as f64
            {
                let view = self.obus[i]
                    .as_ref()
                    .map(|o| o.peer_view())
                    .unwrap_or_default();
                let seen: Vec<String> = view
                    .0
                    .iter()
                    .filter(|e| self.owner.get(&e.msg.temp_id) == Some(&j))
                    .map(|e| {
                        format!(
                            "seen ({:.1},{:.1}) h{:.2} v{:.1} acc{:.1} t{:.1} conf{:?}",
                            e.msg.motion.x,
                            e.msg.motion.y,
                            e.msg.motion.heading,
                            e.msg.motion.speed,
                            e.msg.pos_accuracy_m,
                            e.msg.t,
                            view.1
                                .get(&e.msg.temp_id)
                                .map(|c| (c * 100.0).round() / 100.0)
                        )
                    })
                    .collect();
                let (a, b) = (&self.vehicles[i], &self.vehicles[j]);
                eprintln!(
                    "TRACE {} | A true ({:.1},{:.1}) h{:.2} | B true ({:.1},{:.1}) h{:.2} | A sees B: {:?}",
                    self.describe(i, j, f64::NAN),
                    a.x,
                    a.y,
                    a.heading,
                    b.x,
                    b.y,
                    b.heading,
                    seen
                );
            }
        }
        // phones' alerts, by the vehicle they are about
        let mut cautions = Vec::new();
        for i in 0..n {
            let Some(d) = &self.decisions[i] else {
                continue;
            };
            if d.alerts.iter().any(|al| al.level >= Level::Warning) {
                for id in &d.avoiding {
                    if let Some(&j) = self.owner.get(id)
                        && j != i
                    {
                        let e = self.advising.entry((i, j)).or_insert((now, now));
                        e.1 = now;
                    }
                }
            }
            for al in &d.alerts {
                let Some(j) = al.target.and_then(|t| self.owner.get(&t).copied()) else {
                    continue;
                };
                if j == i {
                    continue;
                }
                if al.level >= Level::Warning {
                    if debug()
                        && al.kind == AlertKind::Oncoming
                        && !self.alert_eps.contains_key(&(i, j))
                    {
                        let (view, conf) = self.obus[i]
                            .as_ref()
                            .map(|o| o.peer_view())
                            .unwrap_or_default();
                        if let Some(t) = al.target
                            && let Some(e) = view.iter().find(|e| e.msg.temp_id == t)
                        {
                            let b = &self.vehicles[j];
                            eprintln!(
                                "ONCOMING-DBG me true ({:.1},{:.1}) | peer true ({:.1},{:.1} h{:.2}) seen ({:.1},{:.1} h{:.2}) acc {:.1} conf {:?}",
                                self.vehicles[i].x,
                                self.vehicles[i].y,
                                b.x,
                                b.y,
                                b.heading,
                                e.msg.motion.x,
                                e.msg.motion.y,
                                e.msg.motion.heading,
                                e.msg.pos_accuracy_m,
                                conf.get(&t)
                            );
                        }
                    }
                    let near = self
                        .last_near
                        .get(&key(i, j))
                        .is_some_and(|&t| now - t <= 1.0);
                    let note = if debug() {
                        format!(
                            "{:?} {:?} {} t={now:.1}",
                            al.kind,
                            al.level,
                            self.describe(i, j, al.ttc_s.unwrap_or(f64::NAN))
                        )
                    } else {
                        String::new()
                    };
                    let e = self.alert_eps.entry((i, j)).or_insert(AlertEp {
                        note,
                        kind: al.kind,
                        danger: al.level == Level::Danger,
                        start: now,
                        last: now,
                        confirmed: near,
                    });
                    e.last = now;
                } else {
                    cautions.push(key(i, j));
                }
            }
        }
        // ground truth for every pair that could meet
        let paths: Vec<Option<Vec<Pose>>> = self
            .vehicles
            .iter()
            .map(|v| (!v.gone).then(|| self.future_path(v)))
            .collect();
        for i in 0..n {
            for j in (i + 1)..n {
                let (a, b) = (&self.vehicles[i], &self.vehicles[j]);
                let (Some(pa), Some(pb)) = (&paths[i], &paths[j]) else {
                    continue;
                };
                if a.crashed.is_some() && b.crashed.is_some() {
                    continue;
                }
                let dist = (a.x - b.x).hypot(a.y - b.y);
                if dist > (a.speed + b.speed) * ORACLE_HORIZON_S + 15.0 {
                    continue;
                }
                let Some(_) = first_conflict(
                    pa,
                    a.dims(),
                    pb,
                    b.dims(),
                    NEAR_MISS_MARGIN_M / 2.0,
                    NEAR_MISS_PET_S,
                    TICK,
                ) else {
                    continue;
                };
                self.last_near.insert((i, j), now);
                for k in [(i, j), (j, i)] {
                    if let Some(e) = self.alert_eps.get_mut(&k) {
                        e.confirmed = true;
                    }
                }
                let Some(ttc) = first_conflict(pa, a.dims(), pb, b.dims(), 0.0, 0.0, TICK) else {
                    continue;
                };
                if ttc > CONFLICT_TTC_S + 1e-9 {
                    continue;
                }
                let warned = self
                    .alerting(i, j)
                    .into_iter()
                    .chain(self.alerting(j, i))
                    .reduce(f64::min);
                let hinted = cautions.contains(&(i, j));
                let note = if debug() {
                    self.describe(i, j, ttc)
                } else {
                    String::new()
                };
                let roads = &self.roads;
                let turning_soon = |v: &CityVehicle| match v.route {
                    Route::Lane { road, dir, s, .. } => {
                        let to_end = if dir > 0.0 { roads[road].len - s } else { s };
                        v.plan.is_some_and(|p| p.turn != 0) || to_end < 40.0 // U-turn at the road end
                    }
                    Route::Arc { .. } => false,
                };
                let unannounced = turning_soon(a) || turning_soon(b);
                let ep = self.episodes.entry((i, j)).or_insert(Episode {
                    note,
                    start: now,
                    unannounced_turn: unannounced,
                    contact_est: now + ttc,
                    last_conflict: now,
                    critical: false,
                    warned_in_time: None,
                    hinted: false,
                });
                ep.last_conflict = now;
                ep.hinted |= hinted;
                if ep.warned_in_time.is_none() && (!ep.critical || now - ep.start <= GRACE_S) {
                    ep.warned_in_time = warned;
                }
                if ttc < CRITICAL_TTC_S {
                    ep.critical = true;
                }
            }
        }
        self.close(now, false);
    }

    /// Has phone `x` heard vehicle `y` (any of its pseudonyms is in the LDM)?
    fn heard(&self, x: usize, y: usize) -> bool {
        self.obus[x].as_ref().is_some_and(|o| {
            self.owner
                .iter()
                .any(|(t, &w)| w == y && o.ldm().get(t).is_some())
        })
    }

    fn describe(&self, i: usize, j: usize, ttc: f64) -> String {
        let (a, b) = (&self.vehicles[i], &self.vehicles[j]);
        let dec = |x: usize| {
            self.decisions[x].as_ref().map_or(String::from("-"), |d| {
                format!(
                    "{:?} wttc={:?} [{}]",
                    d.state,
                    d.worst_ttc.map(|t| (t * 10.0).round() / 10.0),
                    d.alerts
                        .iter()
                        .map(|a| {
                            let who = a
                                .target
                                .and_then(|t| self.owner.get(&t))
                                .map_or(String::from("-"), |w| format!("#{w}"));
                            format!("{:?}/{:?}@{who}", a.kind, a.level)
                        })
                        .collect::<Vec<_>>()
                        .join(",")
                )
            })
        };
        let acc = |v: &CityVehicle| v.accel;
        format!(
            "#{i}/#{j} {} | A: {} acc{:.1} | B: {} acc{:.1}",
            self.describe0(i, j, ttc),
            dec(i),
            acc(a),
            dec(j),
            acc(b)
        )
    }

    fn describe0(&self, i: usize, j: usize, ttc: f64) -> String {
        let r = |v: &CityVehicle| match v.route {
            Route::Lane { road, dir, lat, .. } => format!("lane r{road} d{dir:+.0} lat{lat:+.1}"),
            Route::Arc { ccw, r, .. } => format!("arc {} r{r:.0}", if ccw { "L" } else { "R" }),
        };
        let (a, b) = (&self.vehicles[i], &self.vehicles[j]);
        let known = |x: usize, y: usize| {
            self.obus[x].as_ref().is_some_and(|o| {
                self.owner
                    .iter()
                    .any(|(t, &w)| w == y && o.ldm().get(t).is_some())
            })
        };
        format!(
            "t={:.1} ttc={ttc:.1} d={:.0}m | {} {:?} {:.0}km/h [{}] in_ldm={} | {} {:?} {:.0}km/h [{}] in_ldm={} | crash={:?}/{:?}",
            self.t,
            (a.x - b.x).hypot(a.y - b.y),
            type_label(a.kind),
            a.temper,
            a.speed * 3.6,
            r(a),
            known(i, j),
            type_label(b.kind),
            b.temper,
            b.speed * 3.6,
            r(b),
            known(j, i),
            a.crashed.is_some(),
            b.crashed.is_some()
        )
    }

    fn classify(&mut self, (i, j): (usize, usize), ep: Episode) {
        let (a, b) = (&self.vehicles[i], &self.vehicles[j]);
        if !(a.has_app && b.has_app) {
            self.stats.conflicts_unequipped += 1;
            return;
        }
        let moto = a.kind == VehicleType::Motorbike || b.kind == VehicleType::Motorbike;
        self.stats.conflicts += 1;
        let gate = self.opts.config.max_alert_pos_accuracy_m;
        let in_gate = a.gps_sigma_m <= gate && b.gps_sigma_m <= gate;
        if in_gate {
            self.stats.gated_conflicts += 1;
            if ep.unannounced_turn {
                self.stats.gated_turn_conflicts += 1;
            }
        }
        if ep.unannounced_turn {
            self.stats.turn_conflicts += 1;
        }
        if moto {
            self.stats.moto_conflicts += 1;
        }
        match ep.warned_in_time {
            Some(t) => {
                self.stats.warned += 1;
                self.stats.lead_times.push((ep.contact_est - t).max(0.0));
            }
            None => {
                if debug() {
                    let gate = self.opts.config.max_alert_pos_accuracy_m;
                    let in_gate = a.gps_sigma_m <= gate && b.gps_sigma_m <= gate;
                    let tag = if !in_gate {
                        "gate"
                    } else if ep.unannounced_turn {
                        "turn"
                    } else {
                        "OTHER"
                    };
                    eprintln!(
                        "MISSED[{tag}] σ{:.0}/{:.0} {}",
                        a.gps_sigma_m, b.gps_sigma_m, ep.note
                    );
                }
                self.stats.missed += 1;
                if in_gate {
                    self.stats.gated_missed += 1;
                    if ep.unannounced_turn {
                        self.stats.gated_turn_missed += 1;
                    }
                }
                if ep.unannounced_turn {
                    self.stats.turn_missed += 1;
                }
                let not_heard = !self.heard(i, j) && !self.heard(j, i);
                if not_heard {
                    self.stats.missed_not_heard += 1;
                }
                let cause = if not_heard {
                    0
                } else if !in_gate {
                    1
                } else if ep.unannounced_turn {
                    2
                } else {
                    3
                };
                self.stats.miss_causes[cause] += 1;
                if ep.hinted {
                    self.stats.missed_with_caution += 1;
                }
                if moto {
                    self.stats.moto_missed += 1;
                }
            }
        }
    }

    fn close(&mut self, now: f64, all: bool) {
        let done: Vec<(usize, usize)> = self
            .episodes
            .iter()
            .filter(|(_, e)| all || now - e.last_conflict > 0.5)
            .map(|(k, _)| *k)
            .collect();
        for k in done {
            if let Some(mut ep) = self.episodes.remove(&k) {
                if ep.warned_in_time.is_none() && !ep.critical {
                    // the alert came within the grace time of the (short) conflict
                    let pairs = [(k.0, k.1), (k.1, k.0)];
                    let direct = pairs
                        .iter()
                        .filter_map(|p| self.alert_eps.get(p))
                        .filter(|e| {
                            e.start <= ep.last_conflict + GRACE_S && e.last >= ep.start - 1.0
                        })
                        .map(|e| e.start);
                    let advice = pairs
                        .iter()
                        .filter_map(|p| self.advising.get(p))
                        .filter(|e| e.0 <= ep.last_conflict + GRACE_S && e.1 >= ep.start - 1.0)
                        .map(|e| e.0);
                    ep.warned_in_time = direct.chain(advice).reduce(f64::min);
                }
                self.classify(k, ep);
            }
        }
        let done: Vec<(usize, usize)> = self
            .alert_eps
            .iter()
            .filter(|(_, e)| all || now - e.last > 1.0)
            .map(|(k, _)| *k)
            .collect();
        self.advising.retain(|_, e| all || now - e.1 <= 1.0);
        for k in done {
            if let Some(e) = self.alert_eps.remove(&k) {
                self.stats.alerts += 1;
                let bk = self
                    .stats
                    .by_kind
                    .entry(format!("{:?}", e.kind))
                    .or_default();
                bk.0 += 1;
                if !e.confirmed {
                    bk.1 += 1;
                    if e.danger {
                        bk.2 += 1;
                    }
                    if debug() {
                        eprintln!("FALSE {} dur={:.1}", e.note, e.last - e.start);
                    }
                    self.stats.false_alarms += 1;
                }
            }
        }
    }

    fn finish(&mut self) {
        let t = self.t;
        self.close(t, true);
        // every vehicle here is honest: any block is a false misbehaviour verdict
        self.stats.honest_blocked = self
            .obus
            .iter()
            .flatten()
            .map(|o| o.blocked().len() as u64)
            .sum();
    }
}
