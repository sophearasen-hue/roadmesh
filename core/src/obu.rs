//! The safety engine of one phone: receive → verify → check → LDM → predict →
//! right of way → alerts. Port of `reference/core-python/v2v/obu.py`.
//!
//! **The core warns, it never brakes** (CLAUDE.md, GAP_REPORT F-5). Where the
//! reference returned an acceleration cap that braked the car, this returns
//! [`Decision`]: alerts with a level, a driver instruction ([`BrakeAdvice`]) and
//! the gentlest deceleration that would avoid the conflict, as information for
//! the display and voice. Only the simulator's test driver follows that number.

use std::collections::HashSet;

use crate::auth::{AuthError, Signer, Verifier};
use crate::config::Config;
use crate::geo::{GeoOrigin, math_to_compass_deg};
use crate::ldm::{Check, Ldm, Misbehaviour, PeerMsg};
use crate::prediction::{Motion, Pose, first_conflict, trajectory};
use crate::rsm::{self, DecodeError, EventCode, Frame, MsgType, Rsm, TempId, VehicleType, flags};

/// This phone's own vehicle, from its sensors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OwnState {
    /// Position, heading, speed, acceleration, yaw rate (local frame).
    pub motion: Motion,
    /// Vehicle type (sets the outline).
    pub vehicle_type: VehicleType,
    /// Estimated position accuracy (m).
    pub pos_accuracy_m: f64,
}

impl OwnState {
    fn dims(&self) -> (f64, f64) {
        self.vehicle_type.dims()
    }
}

/// Map context for the own vehicle, from the offline road map (SPEC §6 layer 1).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RoadContext {
    /// Posted speed limit here (m/s), if known.
    pub speed_limit: Option<f64>,
    /// Within ~150 m of a junction.
    pub at_junction: bool,
}

/// Where another vehicle is relative to my path (reference `relation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// Same direction, in front.
    Ahead,
    /// Same direction, behind.
    Behind,
    /// Opposite direction.
    Oncoming,
    /// Crossing path.
    Crossing,
}

/// Reference state machine (kept so the golden tests can compare it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveState {
    /// Nothing to do.
    Cruise,
    /// Stopped traffic ahead: slowing early.
    Advise,
    /// A conflict, but I have priority.
    Warn,
    /// I must give way / slow for the vehicle ahead.
    Yield,
    /// The other vehicle is not yielding: emergency.
    Brake,
}

/// Alert level (SPEC §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Map highlight only.
    Caution,
    /// Amber banner, single tone.
    Warning,
    /// Red banner, chime and spoken alert.
    Danger,
}

/// What the alert is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertKind {
    /// Crossing traffic: I must give way.
    CrossingConflict,
    /// Crossing traffic: I have priority (Caution).
    PriorityConflict,
    /// Slow or stopped vehicle ahead in my lane: collision risk.
    VehicleAhead,
    /// Oncoming vehicle on a collision course.
    Oncoming,
    /// A vehicle that should give way is not doing so, conflict imminent.
    NotYielding,
    /// Lost radio contact with the vehicle I was giving way to (fail-safe).
    LostContact,
    /// A vehicle ahead is braking hard.
    HardBrakingAhead,
    /// Stopped traffic further ahead (possibly heard through relays).
    HazardAhead,
    /// A vehicle is overtaking me.
    OvertakingYou,
    /// A vehicle is closing fast from behind.
    FastApproachBehind,
    /// I am over the speed limit.
    OwnSpeeding,
    /// Possible conflict with a vehicle whose position is too uncertain for a full alert.
    PossibleConflict,
}

/// The instruction the driver hears and sees (user decision, Step 2): every
/// Danger or Warning alert tells the driver what to do, not only that something is wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BrakeAdvice {
    /// No action needed.
    None,
    /// Stop accelerating / ease off.
    EaseOff,
    /// Brake gently.
    BrakeGently,
    /// Brake firmly.
    BrakeFirmly,
    /// Brake hard now.
    BrakeHard,
}

/// One alert for the driver.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Alert {
    /// What it is about.
    pub kind: AlertKind,
    /// How urgent.
    pub level: Level,
    /// The other vehicle, if any.
    pub target: Option<TempId>,
    /// Predicted time to the conflict (s).
    pub ttc_s: Option<f64>,
    /// Distance to the other vehicle or hazard (m).
    pub distance_m: Option<f64>,
    /// How many relays the information came through (0 = direct).
    pub hops: u8,
    /// Relation of the other vehicle to my path.
    pub relation: Option<Relation>,
    /// Gentlest deceleration that avoids the conflict (m/s², ≥ 0). Advice only.
    pub advised_decel: f64,
    /// Driver instruction derived from `advised_decel`.
    pub advice: BrakeAdvice,
}

/// Output of one decision cycle.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    /// Alerts, most important first. The app shows `alerts[0]`.
    pub alerts: Vec<Alert>,
    /// Reference state.
    pub state: DriveState,
    /// The vehicle I am giving way to.
    pub yield_to: Option<TempId>,
    /// Smallest predicted time to conflict (s).
    pub worst_ttc: Option<f64>,
    /// Advised acceleration limit (m/s², ≤ 0): 0 = do not accelerate, negative = brake
    /// this hard. `None` = no advice. Information only; the simulator's test driver uses it.
    pub advised_accel: Option<f64>,
}

/// Result of receiving one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RxOutcome {
    /// New, verified, plausible: stored.
    Accepted {
        /// Sender pseudonym.
        temp_id: TempId,
        /// Relays it came through.
        hops: u8,
    },
    /// Bytes did not parse.
    Malformed(DecodeError),
    /// Signature / certificate failure.
    AuthFailed {
        /// Claimed sender.
        temp_id: TempId,
        /// Why.
        reason: AuthError,
    },
    /// Sender is on this phone's local block list.
    Blocked,
    /// My own message coming back.
    Own,
    /// Already have it, or too old: dropped quietly.
    Dropped,
    /// Physically impossible or abusive: dropped, sender blocked locally.
    Misbehaviour {
        /// Sender.
        temp_id: TempId,
        /// Why.
        reason: Misbehaviour,
        /// True the first time for this sender: the platform should file a report.
        report: bool,
    },
}

#[derive(Debug, Clone)]
struct Threat {
    ttc: f64,
    msg: PeerMsg,
    traj: Vec<Pose>,
}

/// The safety engine of one phone.
#[derive(Debug, Clone)]
pub struct Obu {
    cfg: Config,
    origin: GeoOrigin,
    relay: bool,
    ldm: Ldm,
    own_ids: HashSet<TempId>,
    blocked: HashSet<TempId>,
    reported: HashSet<TempId>,
    outbox: Vec<Frame>,
    msg_id: u16,
    state: DriveState,
    yield_to: Option<TempId>,
    hazard_mem: Option<(f64, PeerMsg, u8)>,
}

/// Relation of `other` to my path (reference `relation`).
pub fn relation(me: &Motion, other: &Motion) -> Relation {
    let (dx, dy) = (other.x - me.x, other.y - me.y);
    let along = dx * me.heading.cos() + dy * me.heading.sin();
    let hdiff = (other.heading - me.heading).cos();
    if hdiff > 0.7 {
        if along > 0.0 {
            Relation::Ahead
        } else {
            Relation::Behind
        }
    } else if hdiff < -0.7 {
        Relation::Oncoming
    } else {
        Relation::Crossing
    }
}

impl Obu {
    /// New engine. `relay` enables store-and-forward of other phones' messages.
    pub fn new(cfg: Config, origin: GeoOrigin, relay: bool) -> Self {
        Self {
            cfg,
            origin,
            relay,
            ldm: Ldm::new(),
            own_ids: HashSet::new(),
            blocked: HashSet::new(),
            reported: HashSet::new(),
            outbox: Vec::new(),
            msg_id: 0,
            state: DriveState::Cruise,
            yield_to: None,
            hazard_mem: None,
        }
    }

    /// The configuration in use.
    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// The Local Dynamic Map (read-only).
    pub fn ldm(&self) -> &Ldm {
        &self.ldm
    }

    /// Senders this phone has blocked.
    pub fn blocked(&self) -> &HashSet<TempId> {
        &self.blocked
    }

    /// Pseudonyms this phone has used.
    pub fn own_ids(&self) -> &HashSet<TempId> {
        &self.own_ids
    }

    /// Frames queued for forwarding (cleared by [`Obu::transmit`]).
    pub fn pending_relays(&self) -> &[Frame] {
        &self.outbox
    }

    // ------------------------------------------------------------ transmit

    /// Frames to broadcast now: queued relays (heard last cycle, 100 ms per hop),
    /// then this phone's own signed message.
    pub fn transmit(&mut self, now: f64, own: &OwnState, signer: &mut dyn Signer) -> Vec<Frame> {
        let mut out = std::mem::take(&mut self.outbox);
        let temp_id = signer.current_temp_id(now);
        self.own_ids.insert(temp_id);
        self.msg_id = self.msg_id.wrapping_add(1);
        let m = &own.motion;
        let (lat, lon) = self.origin.to_latlon(m.x, m.y);
        let r = Rsm {
            msg_type: MsgType::Heartbeat,
            temp_id,
            msg_id: self.msg_id,
            timestamp_ms: rsm::ms_of_hour(now),
            lat_deg: lat,
            lon_deg: lon,
            speed: m.speed,
            heading_deg: math_to_compass_deg(m.heading),
            accel: m.accel,
            yaw_rate: m.yaw_rate,
            vehicle_type: own.vehicle_type,
            flags: if m.accel <= -self.cfg.hard_brake_ahead_decel {
                flags::HARD_BRAKE
            } else {
                0
            },
            pos_accuracy_m: own.pos_accuracy_m,
            hop_count: 0,
            ttl: self.cfg.heartbeat_ttl,
            origin_pos_age_ms: 0,
            event: EventCode::None,
        };
        let body = r.encode();
        let auth = signer.sign(&temp_id, &rsm::signing_bytes(&body));
        out.push(Frame {
            last_hop: None,
            body,
            auth,
        });
        out
    }

    // ------------------------------------------------------------ receive

    /// Handle one received frame (untrusted bytes).
    pub fn receive_bytes(
        &mut self,
        bytes: &[u8],
        now: f64,
        own: &OwnState,
        verifier: &dyn Verifier,
    ) -> RxOutcome {
        match Frame::decode(bytes) {
            Ok(f) => self.receive(&f, now, own, verifier),
            Err(e) => RxOutcome::Malformed(e),
        }
    }

    /// Handle one received frame: verify the ORIGINATOR's signature, check, store, queue relay.
    pub fn receive(
        &mut self,
        frame: &Frame,
        now: f64,
        own: &OwnState,
        verifier: &dyn Verifier,
    ) -> RxOutcome {
        let r = match Rsm::decode(&frame.body) {
            Ok(r) => r,
            Err(e) => return RxOutcome::Malformed(e),
        };
        if let Err(reason) =
            verifier.verify(&r.temp_id, &rsm::signing_bytes(&frame.body), &frame.auth)
        {
            return RxOutcome::AuthFailed {
                temp_id: r.temp_id,
                reason,
            };
        }
        if self.blocked.contains(&r.temp_id) {
            return RxOutcome::Blocked;
        }
        if self.own_ids.contains(&r.temp_id) {
            return RxOutcome::Own;
        }
        let msg = PeerMsg::from_rsm(&r, &self.origin, now);
        match self.ldm.check(&msg, now, &self.cfg) {
            Check::Duplicate | Check::Stale => return RxOutcome::Dropped,
            Check::Misbehaviour(reason) => {
                // protect this phone at once, before any CA decision
                self.blocked.insert(r.temp_id);
                self.ldm.remove(&r.temp_id);
                let report = self.reported.insert(r.temp_id);
                return RxOutcome::Misbehaviour {
                    temp_id: r.temp_id,
                    reason,
                    report,
                };
            }
            Check::Ok => {}
        }
        self.ldm.update(msg, now, r.hop_count);
        // Store-and-forward once per message. Only verified, plausible, new messages are relayed.
        if self.relay && r.ttl > 0 {
            let (hx, hy) = match frame.last_hop {
                Some((lat, lon)) => self.origin.to_local(lat, lon),
                None => (msg.motion.x, msg.motion.y),
            };
            let me = &own.motion;
            if (me.x - hx).hypot(me.y - hy) >= self.cfg.relay_min_progress_m {
                let mut fwd = r;
                fwd.hop_count = r.hop_count.saturating_add(1).min(15);
                fwd.ttl = r.ttl - 1;
                fwd.origin_pos_age_ms = ((now - msg.t).max(0.0) * 1000.0)
                    .round()
                    .min(f64::from(u16::MAX)) as u16;
                self.outbox.push(Frame {
                    last_hop: Some(self.origin.to_latlon(me.x, me.y)),
                    body: fwd.encode(),
                    auth: frame.auth.clone(),
                });
            }
        }
        RxOutcome::Accepted {
            temp_id: r.temp_id,
            hops: r.hop_count,
        }
    }

    // ------------------------------------------------------------ right of way

    fn must_yield(&self, me: &Motion, other: &Motion) -> bool {
        match relation(me, other) {
            Relation::Ahead => true, // never hit the vehicle in front
            Relation::Behind => false,
            rel => {
                let me_turning = me.yaw_rate.abs() > 0.05;
                let other_turning = other.yaw_rate.abs() > 0.05;
                if me_turning != other_turning {
                    return me_turning; // turning traffic yields to straight traffic
                }
                if rel == Relation::Oncoming {
                    return true; // head-on: both
                }
                let (dx, dy) = (other.x - me.x, other.y - me.y);
                let cross = me.heading.cos() * dy - me.heading.sin() * dx;
                cross < 0.0 // crossing: yield to the vehicle on my right
            }
        }
    }

    fn has_passed(me: &Motion, other: &PeerMsg) -> bool {
        let (dx, dy) = (me.x - other.motion.x, me.y - other.motion.y);
        let toward_me = dx * other.motion.heading.cos() + dy * other.motion.heading.sin();
        toward_me < 0.0 && dx.hypot(dy) > other.length + 2.0
    }

    fn target_cleared(&self, own: &OwnState, target: &PeerMsg, target_traj: &[Pose]) -> bool {
        let me = &own.motion;
        if relation(me, &target.motion) == Relation::Crossing {
            return Self::has_passed(me, target); // hysteresis: wait until it is really gone
        }
        let ego = trajectory(
            Motion {
                accel: self.cfg.resume_accel,
                ..*me
            },
            self.cfg.horizon_s,
            self.cfg.pred_step_s,
            0.0,
        );
        first_conflict(
            &ego,
            own.dims(),
            target_traj,
            (target.length, target.width),
            self.cfg.safety_margin_m,
            0.0,
            self.cfg.pred_step_s,
        )
        .is_none()
    }

    fn gentlest_safe_accel(&self, own: &OwnState, avoid: &[&Threat]) -> f64 {
        let c = &self.cfg;
        for &a in &c.brake_levels {
            let ego = trajectory(
                Motion {
                    accel: a,
                    ..own.motion
                },
                c.horizon_s,
                c.pred_step_s,
                0.0,
            );
            if avoid.iter().all(|th| {
                first_conflict(
                    &ego,
                    own.dims(),
                    &th.traj,
                    (th.msg.length, th.msg.width),
                    c.safety_margin_m,
                    c.yield_pet_s,
                    c.pred_step_s,
                )
                .is_none()
            }) {
                return a;
            }
        }
        c.max_brake
    }

    fn advice_for(&self, accel: Option<f64>) -> (f64, BrakeAdvice) {
        match accel {
            None => (0.0, BrakeAdvice::None),
            Some(a) if a >= 0.0 => (0.0, BrakeAdvice::EaseOff),
            Some(a) => {
                let d = -a;
                let advice = if d <= self.cfg.advice_gentle_max {
                    BrakeAdvice::BrakeGently
                } else if d <= self.cfg.advice_firm_max {
                    BrakeAdvice::BrakeFirmly
                } else {
                    BrakeAdvice::BrakeHard
                };
                (d, advice)
            }
        }
    }

    // ------------------------------------------------------------ hazard ahead

    fn find_hazard(&self, me: &Motion, own_len: f64) -> Option<(f64, PeerMsg, u8)> {
        let (s, c) = me.heading.sin_cos();
        let mut best: Option<(f64, PeerMsg, u8)> = None;
        for e in self.ldm.entries() {
            let m = &e.msg;
            if m.motion.speed > self.cfg.hazard_stopped_speed
                || relation(me, &m.motion) != Relation::Ahead
            {
                continue;
            }
            let (dx, dy) = (m.motion.x - me.x, m.motion.y - me.y);
            let along = dx * c + dy * s;
            let lateral = -dx * s + dy * c;
            let gap = along - (own_len + m.length) / 2.0;
            if lateral.abs() < 3.0
                && gap > 0.0
                && gap < self.cfg.hazard_range_m
                && best.as_ref().is_none_or(|b| gap < b.0)
            {
                best = Some((gap, *m, e.hops));
            }
        }
        best
    }

    fn hazard_accel(&self, me: &Motion, hazard: Option<&(f64, PeerMsg, u8)>) -> Option<f64> {
        let (gap, _, _) = hazard?;
        let v = me.speed;
        if v < 0.5 {
            return Some(0.0);
        }
        let room = (gap - self.cfg.hazard_buffer_m).max(1.0);
        let need = v * v / (2.0 * room);
        let mut cap = None;
        if need >= 1.0 {
            cap = Some(-(self.cfg.comfort_decel.min(need * 1.1)));
        }
        if v > self.cfg.hazard_advisory_speed {
            cap = Some(cap.unwrap_or(0.0).min(-1.0));
        }
        cap
    }

    // ------------------------------------------------------------ decide

    /// Run all safety applications for this cycle.
    pub fn decide(&mut self, now: f64, own: &OwnState, _road: &RoadContext) -> Decision {
        let c = self.cfg.clone();
        self.ldm.evict(now, &c);
        let me = own.motion;
        let dims = own.dims();
        let ego_const = trajectory(Motion { accel: 0.0, ..me }, c.horizon_s, c.pred_step_s, 0.0);
        let now_ms = (now * 1000.0).round() as i64;
        let fresh_ms = (c.safety_max_age_s * 1000.0).round() as i64;

        // Collision avoidance: only fresh data about vehicles close enough to matter.
        let mut fresh: Vec<PeerMsg> = Vec::new();
        for e in self.ldm.entries() {
            let m = &e.msg;
            if now_ms - m.t_ms <= fresh_ms
                && (m.motion.x - me.x).hypot(m.motion.y - me.y) <= c.safety_range_m
            {
                fresh.push(*m);
            }
        }
        let mut threats: Vec<Threat> = Vec::new();
        let mut trajs: Vec<(TempId, Vec<Pose>)> = Vec::new();
        let mut eebl: Option<(f64, PeerMsg)> = None;
        for m in &fresh {
            let tr = trajectory(m.motion, c.horizon_s, c.pred_step_s, (now - m.t).max(0.0));
            if let Some(ttc) = first_conflict(
                &ego_const,
                dims,
                &tr,
                (m.length, m.width),
                c.safety_margin_m,
                0.0,
                c.pred_step_s,
            ) && ttc < c.ttc_warn_s
            {
                threats.push(Threat {
                    ttc,
                    msg: *m,
                    traj: tr.clone(),
                });
            }
            if m.motion.accel <= -c.hard_brake_ahead_decel
                && relation(&me, &m.motion) == Relation::Ahead
            {
                let gap = (m.motion.x - me.x).hypot(m.motion.y - me.y);
                if gap < c.hard_brake_ahead_range_m && eebl.as_ref().is_none_or(|e| gap < e.0) {
                    eebl = Some((gap, *m));
                }
            }
            trajs.push((m.temp_id, tr));
        }
        threats.sort_by(|a, b| a.ttc.total_cmp(&b.ttc)); // stable, like Python's sort
        let worst_ttc = threats.first().map(|t| t.ttc);

        let mut hazard = self.find_hazard(&me, dims.0);
        if let Some(h) = hazard {
            self.hazard_mem = Some((now, h.1, h.2));
        } else if let Some((seen, m, hops)) = self.hazard_mem
            && now - seen <= c.hazard_memory_s
        {
            // relay chain gap: keep the last report
            let gap = (m.motion.x - me.x) * me.heading.cos()
                + (m.motion.y - me.y) * me.heading.sin()
                - (dims.0 + m.length) / 2.0;
            hazard = if gap > 0.0 {
                Some((gap, m, hops))
            } else {
                None
            };
        }

        let traj_of = |id: &TempId| {
            trajs
                .iter()
                .find(|(t, _)| t == id)
                .map(|(_, tr)| tr.clone())
        };
        let fresh_of = |id: &TempId| fresh.iter().find(|m| &m.temp_id == id).copied();

        // Keep yielding until the target has clearly cleared (or is lost for too long).
        let mut lost_target = false;
        if self.state == DriveState::Yield
            && let Some(yid) = self.yield_to
        {
            match (fresh_of(&yid), traj_of(&yid)) {
                (Some(target), Some(ttr)) => {
                    if self.target_cleared(own, &target, &ttr) {
                        self.state = DriveState::Cruise;
                        self.yield_to = None;
                    }
                }
                _ => {
                    let last = self.ldm.last_seen(&yid).unwrap_or(f64::NEG_INFINITY);
                    if now - last > c.yield_timeout_s && threats.is_empty() {
                        self.state = DriveState::Cruise;
                        self.yield_to = None;
                    } else {
                        lost_target = true;
                    }
                }
            }
        }

        let yield_idx: Vec<usize> = (0..threats.len())
            .filter(|&i| self.must_yield(&me, &threats[i].msg.motion))
            .collect();
        let mut avoid: Vec<&Threat> = yield_idx.iter().map(|&i| &threats[i]).collect();
        for (i, th) in threats.iter().enumerate() {
            if !yield_idx.contains(&i) && th.ttc < c.ttc_brake_s {
                avoid.push(th);
            }
        }
        let held_target;
        if self.state == DriveState::Yield
            && !lost_target
            && let Some(yid) = self.yield_to
            && avoid.iter().all(|th| th.msg.temp_id != yid)
            && let (Some(m), Some(tr)) = (fresh_of(&yid), traj_of(&yid))
        {
            held_target = Threat {
                ttc: f64::INFINITY,
                msg: m,
                traj: tr,
            };
            avoid.push(&held_target);
        }

        let mut cap: Option<f64> = None;
        if lost_target {
            cap = Some(c.failsafe_brake); // fail safe: slow down, don't drive on blind
        } else if !avoid.is_empty() {
            cap = Some(self.gentlest_safe_accel(own, &avoid));
            if let Some(&i) = yield_idx.first()
                && self.state != DriveState::Yield
            {
                self.state = DriveState::Yield;
                self.yield_to = Some(threats[i].msg.temp_id);
            } else if self.state != DriveState::Yield {
                self.state = DriveState::Brake;
            }
        } else if !threats.is_empty() {
            self.state = DriveState::Warn;
        } else {
            self.state = DriveState::Cruise;
        }

        let hz_cap = self.hazard_accel(&me, hazard.as_ref());
        if let Some(h) = hz_cap {
            cap = Some(cap.map_or(h, |v| v.min(h)));
        }
        if let Some(h) = hazard.as_ref()
            && self.state == DriveState::Cruise
            && (me.speed > 1.0 || h.0 > 15.0)
        {
            self.state = DriveState::Advise;
        }

        let alerts = self.reference_alerts(
            &me,
            &threats,
            hazard.as_ref(),
            eebl.as_ref(),
            lost_target,
            cap,
            hz_cap,
        );
        Decision {
            alerts,
            state: self.state,
            yield_to: self.yield_to,
            worst_ttc,
            advised_accel: cap,
        }
    }

    /// Alerts for the reference state machine (YIELD/BRAKE → Danger, WARN → Caution,
    /// stopped traffic / hard braking ahead → Warning).
    #[allow(clippy::too_many_arguments)]
    fn reference_alerts(
        &self,
        me: &Motion,
        threats: &[Threat],
        hazard: Option<&(f64, PeerMsg, u8)>,
        eebl: Option<&(f64, PeerMsg)>,
        lost_target: bool,
        cap: Option<f64>,
        hz_cap: Option<f64>,
    ) -> Vec<Alert> {
        let (decel, advice) = self.advice_for(cap);
        let dist = |m: &PeerMsg| (m.motion.x - me.x).hypot(m.motion.y - me.y);
        let base = |kind, level| Alert {
            kind,
            level,
            target: None,
            ttc_s: None,
            distance_m: None,
            hops: 0,
            relation: None,
            advised_decel: decel,
            advice,
        };
        let mut out = Vec::new();
        match self.state {
            DriveState::Yield => {
                let target = self
                    .yield_to
                    .and_then(|id| self.ldm.get(&id).map(|e| (e.msg, e.hops)));
                let rel = target.map(|(m, _)| relation(me, &m.motion));
                let kind = if lost_target {
                    AlertKind::LostContact
                } else {
                    match rel {
                        Some(Relation::Ahead) => AlertKind::VehicleAhead,
                        Some(Relation::Oncoming) => AlertKind::Oncoming,
                        _ => AlertKind::CrossingConflict,
                    }
                };
                let mut a = base(kind, Level::Danger);
                a.target = self.yield_to;
                a.relation = rel;
                a.ttc_s = threats
                    .iter()
                    .find(|t| Some(t.msg.temp_id) == self.yield_to)
                    .map(|t| t.ttc);
                a.distance_m = target.map(|(m, _)| dist(&m));
                a.hops = target.map_or(0, |(_, h)| h);
                if advice == BrakeAdvice::None {
                    a.advice = BrakeAdvice::EaseOff;
                }
                out.push(a);
            }
            DriveState::Brake => {
                if let Some(th) = threats.first() {
                    let mut a = base(AlertKind::NotYielding, Level::Danger);
                    a.target = Some(th.msg.temp_id);
                    a.ttc_s = Some(th.ttc);
                    a.relation = Some(relation(me, &th.msg.motion));
                    a.distance_m = Some(dist(&th.msg));
                    out.push(a);
                }
            }
            DriveState::Warn => {
                if let Some(th) = threats.first() {
                    let rel = relation(me, &th.msg.motion);
                    let mut a = base(AlertKind::PriorityConflict, Level::Caution);
                    a.target = Some(th.msg.temp_id);
                    a.ttc_s = Some(th.ttc);
                    a.relation = Some(rel);
                    a.distance_m = Some(dist(&th.msg));
                    a.advised_decel = 0.0;
                    a.advice = BrakeAdvice::None;
                    out.push(a);
                }
            }
            DriveState::Advise => {
                if let Some((gap, m, hops)) = hazard {
                    let (d, adv) = self.advice_for(hz_cap.or(Some(0.0)));
                    let mut a = base(AlertKind::HazardAhead, Level::Warning);
                    a.target = Some(m.temp_id);
                    a.distance_m = Some(*gap);
                    a.hops = *hops;
                    a.relation = Some(Relation::Ahead);
                    a.advised_decel = d;
                    a.advice = adv;
                    out.push(a);
                }
            }
            DriveState::Cruise => {
                if let Some((gap, m)) = eebl {
                    let mut a = base(AlertKind::HardBrakingAhead, Level::Warning);
                    a.target = Some(m.temp_id);
                    a.distance_m = Some(*gap);
                    a.relation = Some(Relation::Ahead);
                    a.advised_decel = 0.0;
                    a.advice = BrakeAdvice::EaseOff;
                    out.push(a);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    struct NoAuth;
    impl Verifier for NoAuth {
        fn verify(&self, _: &TempId, _: &[u8; rsm::BODY_LEN], _: &[u8]) -> Result<(), AuthError> {
            Ok(())
        }
    }
    struct FixedId(TempId);
    impl Signer for FixedId {
        fn current_temp_id(&mut self, _: f64) -> TempId {
            self.0
        }
        fn sign(&mut self, _: &TempId, _: &[u8; rsm::BODY_LEN]) -> Vec<u8> {
            Vec::new()
        }
    }

    fn own(x: f64, y: f64, heading: f64, speed: f64) -> OwnState {
        OwnState {
            motion: Motion {
                x,
                y,
                heading,
                speed,
                accel: 0.0,
                yaw_rate: 0.0,
            },
            vehicle_type: VehicleType::Car,
            pos_accuracy_m: 1.0,
        }
    }

    /// Run two engines facing each other for `secs`; returns A's decisions.
    fn exchange(a: &mut Obu, sa: &OwnState, b: &mut Obu, sb: &OwnState, now: f64) {
        let fa = a.transmit(now, sa, &mut FixedId(*b"AAAA"));
        let fb = b.transmit(now, sb, &mut FixedId(*b"BBBB"));
        for f in &fb {
            a.receive(f, now, sa, &NoAuth);
        }
        for f in &fa {
            b.receive(f, now, sb, &NoAuth);
        }
    }

    #[test]
    fn car_from_the_right_makes_me_yield_with_danger_and_advice() {
        let o = GeoOrigin::phnom_penh();
        let mut a = Obu::new(Config::reference(), o, false);
        let mut b = Obu::new(Config::reference(), o, false);
        let sa = own(-30.0, -1.5, 0.0, 13.9);
        let sb = own(1.5, -30.0, FRAC_PI_2, 13.9);
        exchange(&mut a, &sa, &mut b, &sb, 0.0);
        let da = a.decide(0.0, &sa, &RoadContext::default());
        let db = b.decide(0.0, &sb, &RoadContext::default());
        assert_eq!(da.state, DriveState::Yield);
        assert_eq!(da.alerts[0].level, Level::Danger);
        assert_eq!(da.alerts[0].kind, AlertKind::CrossingConflict);
        assert!(da.alerts[0].advice >= BrakeAdvice::BrakeGently);
        assert!(da.advised_accel.is_some_and(|x| x < 0.0));
        // the car with priority is warned, not told to brake (no symmetric deadlock)
        assert_eq!(db.state, DriveState::Warn);
        assert_eq!(db.alerts[0].level, Level::Caution);
        assert_eq!(db.alerts[0].advice, BrakeAdvice::None);
    }

    #[test]
    fn own_messages_and_blocked_senders_are_ignored() {
        let o = GeoOrigin::phnom_penh();
        let mut a = Obu::new(Config::reference(), o, false);
        let sa = own(0.0, 0.0, 0.0, 10.0);
        let mine = a.transmit(0.0, &sa, &mut FixedId(*b"AAAA"));
        assert_eq!(a.receive(&mine[0], 0.0, &sa, &NoAuth), RxOutcome::Own);
    }

    #[test]
    fn impossible_speed_blocks_and_reports_once() {
        let o = GeoOrigin::phnom_penh();
        let mut a = Obu::new(Config::reference(), o, false);
        let sa = own(0.0, 0.0, 0.0, 10.0);
        let mut b = Obu::new(Config::reference(), o, false);
        let fast = own(100.0, 0.0, 0.0, 95.0);
        let f = b.transmit(0.0, &fast, &mut FixedId(*b"EVIL"));
        assert!(matches!(
            a.receive(&f[0], 0.0, &sa, &NoAuth),
            RxOutcome::Misbehaviour { report: true, .. }
        ));
        assert!(a.blocked().contains(b"EVIL"));
        let f2 = b.transmit(0.1, &own(100.0, 0.0, 0.0, 10.0), &mut FixedId(*b"EVIL"));
        assert_eq!(a.receive(&f2[0], 0.1, &sa, &NoAuth), RxOutcome::Blocked);
    }

    #[test]
    fn relay_forwards_once_with_hop_count_and_own_position() {
        let o = GeoOrigin::phnom_penh();
        let mut relay = Obu::new(Config::reference(), o, true);
        let me = own(200.0, 0.0, 0.0, 10.0); // 200 m from the sender: enough progress
        let mut b = Obu::new(Config::reference(), o, false);
        let f = b.transmit(0.0, &own(0.0, 0.0, 0.0, 10.0), &mut FixedId(*b"BBBB"));
        assert!(matches!(
            relay.receive(&f[0], 0.0, &me, &NoAuth),
            RxOutcome::Accepted { hops: 0, .. }
        ));
        assert!(matches!(
            relay.receive(&f[0], 0.0, &me, &NoAuth),
            RxOutcome::Dropped
        ));
        let out = relay.transmit(0.1, &me, &mut FixedId(*b"RRRR"));
        assert_eq!(out.len(), 2); // the relayed frame, then my own message
        let Ok(fwd) = Rsm::decode(&out[0].body) else {
            unreachable!()
        };
        assert_eq!((fwd.hop_count, fwd.ttl), (1, 7));
        assert!(out[0].last_hop.is_some());
        // the relay did not change what the originator signed
        assert_eq!(
            rsm::signing_bytes(&out[0].body),
            rsm::signing_bytes(&f[0].body)
        );
    }

    #[test]
    fn advice_wording_follows_the_deceleration() {
        let a = Obu::new(Config::reference(), GeoOrigin::phnom_penh(), false);
        assert_eq!(a.advice_for(None).1, BrakeAdvice::None);
        assert_eq!(a.advice_for(Some(0.0)).1, BrakeAdvice::EaseOff);
        assert_eq!(a.advice_for(Some(-1.5)).1, BrakeAdvice::BrakeGently);
        assert_eq!(a.advice_for(Some(-3.0)).1, BrakeAdvice::BrakeFirmly);
        assert_eq!(a.advice_for(Some(-8.0)), (8.0, BrakeAdvice::BrakeHard));
    }
}
