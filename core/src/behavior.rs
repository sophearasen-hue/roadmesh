//! Driving-behaviour recorder (SPEC §7). Port of `reference/core-python/v2v/behavior.py`.
//!
//! Watches this vehicle and every peer it hears, and records speeding, harsh
//! braking/acceleration, overtaking and tailgating. Peers are keyed by their
//! rotating pseudonym only. Peer records are deleted after the track expires plus
//! the retention time (AUDIT S-14); only events seen while the peer was in range
//! are recorded, so a score reflects what was actually observed (AUDIT P-37).

use std::collections::HashMap;

use crate::config::{Config, ScoreModel};
use crate::prediction::Motion;
use crate::rsm::{TempId, VehicleType};

/// Posted speed limit lookup from the road map (SPEC §6 layer 1).
pub trait SpeedLimits {
    /// Limit at a local position (m/s), if known.
    fn limit_at(&self, x: f64, y: f64) -> Option<f64>;
}

/// No map: no limits known.
pub struct NoLimits;
impl SpeedLimits for NoLimits {
    fn limit_at(&self, _: f64, _: f64) -> Option<f64> {
        None
    }
}

/// Whose behaviour a record describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Subject {
    /// This phone's own vehicle.
    Me,
    /// A peer, by pseudonym.
    Peer(TempId),
}

/// One observation of a vehicle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Observation {
    /// Who.
    pub who: Subject,
    /// When it was measured (s).
    pub t: f64,
    /// Motion.
    pub motion: Motion,
    /// Outline length (m).
    pub length: f64,
    /// Vehicle type.
    pub vehicle_type: VehicleType,
}

/// Behaviour event kinds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventKind {
    /// Started speeding: (speed km/h, limit km/h).
    Speeding {
        /// km/h when flagged.
        kmh: f64,
        /// limit km/h.
        limit_kmh: f64,
    },
    /// Back under the limit: (duration s, peak km/h).
    SpeedingEnd {
        /// seconds from first over-limit sample.
        duration_s: f64,
        /// peak km/h.
        peak_kmh: f64,
    },
    /// Harsh braking.
    HarshBrake {
        /// deceleration (m/s², > 0).
        decel: f64,
        /// km/h.
        kmh: f64,
    },
    /// Harsh acceleration.
    HarshAccel {
        /// m/s².
        accel: f64,
        /// km/h.
        kmh: f64,
    },
    /// Overtook `other`.
    Overtake {
        /// km/h of the overtaking vehicle.
        kmh: f64,
        /// Done in the oncoming lane (left side).
        oncoming: bool,
        /// Who was overtaken.
        other: Subject,
    },
    /// Started tailgating `other`.
    Tailgating {
        /// time gap (s).
        headway_s: f64,
        /// km/h.
        kmh: f64,
        /// the vehicle in front.
        other: Subject,
    },
    /// Stopped tailgating.
    TailgatingEnd {
        /// seconds from the start of the close following.
        duration_s: f64,
    },
}

/// A recorded event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Event {
    /// When (s).
    pub t: f64,
    /// Who did it.
    pub who: Subject,
    /// What.
    pub kind: EventKind,
}

/// Running statistics for one vehicle.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Profile {
    /// First observation (s).
    pub first_seen: f64,
    /// Latest observation (s).
    pub last_seen: f64,
    /// Number of observations.
    pub samples: u64,
    /// Sum of speeds (for the average).
    pub speed_sum: f64,
    /// Highest speed (m/s).
    pub max_speed: f64,
    /// Time over the limit, after the grace period (s).
    pub over_limit_s: f64,
    /// Speeding episodes.
    pub speeding_episodes: u32,
    /// Harsh braking episodes.
    pub harsh_brakes: u32,
    /// Harsh acceleration episodes.
    pub harsh_accels: u32,
    /// Overtakes made.
    pub overtakes: u32,
    /// Times overtaken.
    pub overtaken: u32,
    /// Overtakes made in the oncoming lane.
    pub oncoming_overtakes: u32,
    /// Tailgating time after the grace period (s).
    pub tailgate_s: f64,
    /// Tailgating episodes.
    pub tailgate_episodes: u32,
    /// SPEC §7 episode-score state: current score.
    pub episode_score: f64,
    /// Distance covered while observed (m).
    pub distance_m: f64,
}

impl Profile {
    /// Average speed (m/s).
    pub fn avg_speed(&self) -> f64 {
        if self.samples == 0 {
            0.0
        } else {
            self.speed_sum / self.samples as f64
        }
    }

    /// 0–100 score under the configured model.
    pub fn score(&self, cfg: &Config) -> u8 {
        let s = match cfg.score_model {
            ScoreModel::ReferenceTime => {
                100.0
                    - self.over_limit_s
                    - cfg.penalty_harsh_brake * f64::from(self.harsh_brakes)
                    - cfg.penalty_harsh_accel * f64::from(self.harsh_accels)
                    - cfg.penalty_tailgating * self.tailgate_s
                    - cfg.penalty_overtake_risky * f64::from(self.oncoming_overtakes)
            }
            ScoreModel::SpecEpisodes => self.episode_score,
        };
        s.round().clamp(0.0, 100.0) as u8
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct CarState {
    over_since: Option<f64>,
    over_flag: bool,
    ep_max: f64,
    braking: bool,
    accel: bool,
}

#[derive(Debug, Clone, Copy)]
struct PairState {
    along: f64,
    t: f64,
    max_lat: f64,
    tail_since: Option<f64>,
    tail_flag: bool,
    follower: Option<Subject>,
    /// A pass seen at the crossing, counted once the passer is `overtake_complete_m`
    /// ahead: (passer, passed, oncoming lane, km/h, +1 if the passer is `a` else −1).
    pending: Option<(Subject, Subject, bool, f64, f64)>,
}

/// The recorder.
#[derive(Debug, Clone, Default)]
pub struct BehaviorMonitor {
    profiles: HashMap<Subject, Profile>,
    events: Vec<Event>,
    cars: HashMap<Subject, CarState>,
    pairs: HashMap<(Subject, Subject), PairState>,
}

impl BehaviorMonitor {
    /// Empty recorder.
    pub fn new() -> Self {
        Self::default()
    }

    /// All recorded events, oldest first.
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Profiles of every vehicle still on record.
    pub fn profiles(&self) -> &HashMap<Subject, Profile> {
        &self.profiles
    }

    fn penalise(&mut self, who: Subject, points: f64) {
        if let Some(p) = self.profiles.get_mut(&who) {
            p.episode_score = (p.episode_score - points).max(0.0);
        }
    }

    fn event(&mut self, t: f64, who: Subject, kind: EventKind) {
        self.events.push(Event { t, who, kind });
    }

    /// Record one cycle: `me` plus every peer observation (any age; old ones are skipped).
    pub fn observe(
        &mut self,
        cfg: &Config,
        now: f64,
        dt: f64,
        me: Observation,
        peers: &[Observation],
        limits: &dyn SpeedLimits,
    ) {
        let mut cars: Vec<Observation> = vec![me];
        cars.extend(
            peers
                .iter()
                .copied()
                .filter(|o| now - o.t <= cfg.behaviour_max_age_s + 1e-9),
        );
        for o in &cars {
            self.car(cfg, now, dt, o, limits);
        }
        cars.sort_by_key(|o| o.who);
        for i in 0..cars.len() {
            for j in (i + 1)..cars.len() {
                self.pair(cfg, now, dt, &cars[i], &cars[j]);
            }
        }
        // recovery and retention
        if cfg.score_model == ScoreModel::SpecEpisodes {
            let rec = cfg.score_recovery_per_min / 60.0 * dt;
            for (who, p) in self.profiles.iter_mut() {
                let st = self.cars.get(who);
                let clean = st.is_none_or(|s| !s.over_flag && !s.braking && !s.accel);
                if clean && p.last_seen >= now - 1e-9 {
                    p.episode_score = (p.episode_score + rec).min(100.0);
                }
            }
        }
        let keep = cfg.ldm_ttl_s + cfg.peer_record_retention_s;
        let expired: Vec<Subject> = self
            .profiles
            .iter()
            .filter(|(w, p)| **w != Subject::Me && now - p.last_seen > keep)
            .map(|(w, _)| *w)
            .collect();
        for w in expired {
            self.profiles.remove(&w);
            self.cars.remove(&w);
            self.pairs.retain(|(a, b), _| *a != w && *b != w);
            self.events.retain(|e| e.who != w);
        }
    }

    fn car(&mut self, cfg: &Config, now: f64, dt: f64, o: &Observation, limits: &dyn SpeedLimits) {
        let who = o.who;
        let is_new = !self.profiles.contains_key(&who);
        let prev_last = self.profiles.get(&who).map(|p| (p.samples, p.last_seen));
        if is_new {
            self.profiles.insert(
                who,
                Profile {
                    first_seen: now,
                    episode_score: 100.0,
                    ..Profile::default()
                },
            );
        }
        // lost sight of this car: close what was open at the moment we last saw it
        if let (Some((samples, last)), Some(st0)) = (prev_last, self.cars.get(&who).copied())
            && samples > 0
            && now - last > cfg.behaviour_max_age_s
        {
            if st0.over_flag
                && let Some(since) = st0.over_since
            {
                self.event(
                    last,
                    who,
                    EventKind::SpeedingEnd {
                        duration_s: last - since,
                        peak_kmh: st0.ep_max * 3.6,
                    },
                );
            }
            if let Some(s) = self.cars.get_mut(&who) {
                *s = CarState {
                    ep_max: s.ep_max,
                    ..CarState::default()
                };
            }
        }
        let m = &o.motion;
        if let Some(p) = self.profiles.get_mut(&who) {
            p.last_seen = now;
            p.samples += 1;
            p.speed_sum += m.speed;
            p.distance_m += m.speed * dt.max(0.0);
            p.max_speed = p.max_speed.max(m.speed);
        }
        let mut st = self.cars.get(&who).copied().unwrap_or_default();

        // speeding
        let limit = limits.limit_at(m.x, m.y);
        let over = match (limit, st.over_since.is_some()) {
            (Some(l), false) => l > 0.0 && m.speed > l * cfg.speeding_ratio + cfg.speeding_offset,
            (Some(l), true) => {
                l > 0.0 && m.speed > l * cfg.speeding_end_ratio + cfg.speeding_end_offset
            }
            (None, _) => false,
        };
        if over {
            if st.over_since.is_none() {
                st.over_since = Some(now);
                st.ep_max = 0.0;
            }
            st.ep_max = st.ep_max.max(m.speed);
            if st
                .over_since
                .is_some_and(|s| now - s >= cfg.speeding_min_s - 1e-9)
            {
                if let Some(p) = self.profiles.get_mut(&who) {
                    p.over_limit_s += dt;
                }
                if !st.over_flag {
                    st.over_flag = true;
                    if let Some(p) = self.profiles.get_mut(&who) {
                        p.speeding_episodes += 1;
                    }
                    self.penalise(who, cfg.penalty_speeding);
                    let l = limit.unwrap_or(0.0);
                    self.event(
                        now,
                        who,
                        EventKind::Speeding {
                            kmh: m.speed * 3.6,
                            limit_kmh: l * 3.6,
                        },
                    );
                }
            }
        } else {
            if st.over_flag
                && let Some(s) = st.over_since
            {
                self.event(
                    now,
                    who,
                    EventKind::SpeedingEnd {
                        duration_s: now - s,
                        peak_kmh: st.ep_max * 3.6,
                    },
                );
            }
            st.over_since = None;
            st.over_flag = false;
        }

        // harsh braking / acceleration
        let decel = -m.accel;
        if decel >= cfg.harsh_brake && !st.braking {
            st.braking = true;
            if let Some(p) = self.profiles.get_mut(&who) {
                p.harsh_brakes += 1;
            }
            self.penalise(who, cfg.penalty_harsh_brake);
            self.event(
                now,
                who,
                EventKind::HarshBrake {
                    decel,
                    kmh: m.speed * 3.6,
                },
            );
        } else if decel < cfg.harsh_brake_end {
            st.braking = false;
        }
        let harsh = if o.vehicle_type == VehicleType::Motorbike {
            cfg.harsh_accel_motorbike
        } else {
            cfg.harsh_accel
        };
        if m.accel >= harsh && !st.accel {
            st.accel = true;
            if let Some(p) = self.profiles.get_mut(&who) {
                p.harsh_accels += 1;
            }
            self.penalise(who, cfg.penalty_harsh_accel);
            self.event(
                now,
                who,
                EventKind::HarshAccel {
                    accel: m.accel,
                    kmh: m.speed * 3.6,
                },
            );
        } else if m.accel < cfg.harsh_accel_end {
            st.accel = false;
        }
        self.cars.insert(who, st);
    }

    #[allow(clippy::too_many_arguments)]
    fn count_overtake(
        &mut self,
        cfg: &Config,
        now: f64,
        passer: Subject,
        passed: Subject,
        left: bool,
        kmh: f64,
    ) {
        if let Some(p) = self.profiles.get_mut(&passer) {
            p.overtakes += 1;
            if left {
                p.oncoming_overtakes += 1;
            }
        }
        if let Some(p) = self.profiles.get_mut(&passed) {
            p.overtaken += 1;
        }
        self.penalise(
            passer,
            if left {
                cfg.penalty_overtake_risky
            } else {
                cfg.penalty_overtake
            },
        );
        self.event(
            now,
            passer,
            EventKind::Overtake {
                kmh,
                oncoming: left,
                other: passed,
            },
        );
    }

    fn pair(&mut self, cfg: &Config, now: f64, dt: f64, a: &Observation, b: &Observation) {
        let key = (a.who, b.who);
        let (am, bm) = (&a.motion, &b.motion);
        if (am.heading - bm.heading).cos() < 0.9
            || (am.x - bm.x).hypot(am.y - bm.y) > cfg.behaviour_pair_range_m
        {
            self.pairs.remove(&key);
            return;
        }
        let (s, c) = am.heading.sin_cos();
        let (dx, dy) = (am.x - bm.x, am.y - bm.y);
        let along = dx * c + dy * s; // > 0: a is ahead of b
        let lateral = -dx * s + dy * c;
        let mut st = *self.pairs.entry(key).or_insert(PairState {
            along,
            t: now,
            max_lat: 0.0,
            tail_since: None,
            tail_flag: false,
            follower: None,
            pending: None,
        });
        if now - st.t > 0.5 {
            st.max_lat = 0.0; // gap in observation: restart the pass tracking
        }
        if along.abs() < 30.0 {
            st.max_lat = st.max_lat.max(lateral.abs());
        }
        let prev = st.along;
        if (prev < 0.0 && along >= 0.0) || (prev > 0.0 && along <= 0.0) {
            // whoever was behind before the crossing is the one who passed (the reference
            // decided from the sign of `along`, which picks the wrong car when it is exactly 0)
            let (passer, passed, pm) = if prev < 0.0 {
                (a.who, b.who, am)
            } else {
                (b.who, a.who, bm)
            };
            if st.max_lat >= cfg.overtake_min_lateral_m {
                // oncoming lane = passer was on the left of the passed vehicle
                let left = if passer == a.who {
                    lateral > 0.0
                } else {
                    lateral < 0.0
                };
                let sign = if passer == a.who { 1.0 } else { -1.0 };
                st.pending = Some((passer, passed, left, pm.speed * 3.6, sign));
            }
            st.max_lat = 0.0;
        }
        st.along = along;
        st.t = now;
        // count the pass once the passer is clear ahead (immediately in the reference profile)
        if let Some((passer, passed, left, kmh, sign)) = st.pending {
            if sign * along >= cfg.overtake_complete_m {
                st.pending = None;
                self.count_overtake(cfg, now, passer, passed, left, kmh);
            } else if sign * along < 0.0 {
                st.pending = None; // fell back: not an overtake
            }
        }

        // tailgating: same lane, the one behind closer than the headway
        if lateral.abs() < 1.5 {
            let (follower, fm, leader) = if along > 0.0 {
                (b.who, bm, a.who)
            } else {
                (a.who, am, b.who)
            };
            let gap = along.abs() - (a.length + b.length) / 2.0;
            let headway = if fm.speed > 0.1 {
                gap / fm.speed
            } else {
                f64::INFINITY
            };
            let ongoing = st.tail_since.is_some() && st.follower == Some(follower);
            let limit = if ongoing {
                cfg.tailgate_end_headway_s
            } else {
                cfg.tailgate_headway_s
            };
            if fm.speed >= cfg.tailgate_min_speed && headway < limit {
                if !ongoing {
                    st.tail_since = Some(now);
                    st.follower = Some(follower);
                    st.tail_flag = false;
                }
                if st
                    .tail_since
                    .is_some_and(|t0| now - t0 >= cfg.tailgate_min_s - 1e-9)
                {
                    if let Some(p) = self.profiles.get_mut(&follower) {
                        p.tailgate_s += dt;
                    }
                    if !st.tail_flag {
                        st.tail_flag = true;
                        if let Some(p) = self.profiles.get_mut(&follower) {
                            p.tailgate_episodes += 1;
                        }
                        self.penalise(follower, cfg.penalty_tailgating);
                        self.event(
                            now,
                            follower,
                            EventKind::Tailgating {
                                headway_s: headway,
                                kmh: fm.speed * 3.6,
                                other: leader,
                            },
                        );
                    }
                }
                self.pairs.insert(key, st);
                return;
            }
        }
        if st.tail_flag
            && let (Some(f), Some(t0)) = (st.follower, st.tail_since)
        {
            self.event(
                now,
                f,
                EventKind::TailgatingEnd {
                    duration_s: now - t0,
                },
            );
        }
        st.tail_since = None;
        st.tail_flag = false;
        st.follower = None;
        self.pairs.insert(key, st);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(f64);
    impl SpeedLimits for Fixed {
        fn limit_at(&self, _: f64, _: f64) -> Option<f64> {
            Some(self.0)
        }
    }

    fn obs(who: Subject, t: f64, x: f64, y: f64, speed: f64, accel: f64) -> Observation {
        Observation {
            who,
            t,
            motion: Motion {
                x,
                y,
                heading: 0.0,
                speed,
                accel,
                yaw_rate: 0.0,
            },
            length: 4.5,
            vehicle_type: VehicleType::Car,
        }
    }

    /// reference test_speeding_and_harsh_events
    #[test]
    fn speeding_and_harsh_events() {
        let cfg = Config::reference();
        let mut mon = BehaviorMonitor::new();
        let p1 = Subject::Peer(*b"p1p1");
        for i in 0..40 {
            let t = f64::from(i) * 0.1;
            let peer = obs(
                p1,
                t,
                20.0 + 22.2 * t,
                0.0,
                22.2,
                if i == 30 { -5.0 } else { 0.0 },
            );
            mon.observe(
                &cfg,
                t,
                0.1,
                obs(Subject::Me, t, 10.0 * t, 0.0, 10.0, 0.0),
                &[peer],
                &Fixed(50.0 / 3.6),
            );
        }
        let kinds: Vec<&EventKind> = mon
            .events()
            .iter()
            .filter(|e| e.who == p1)
            .map(|e| &e.kind)
            .collect();
        assert!(
            kinds
                .iter()
                .any(|k| matches!(k, EventKind::Speeding { .. }))
        );
        assert!(
            kinds
                .iter()
                .any(|k| matches!(k, EventKind::HarshBrake { .. }))
        );
        assert!(mon.profiles()[&p1].over_limit_s > 1.5);
        assert_eq!(mon.profiles()[&Subject::Me].over_limit_s, 0.0);
    }

    /// reference test_overtake_in_oncoming_lane
    #[test]
    fn overtake_in_oncoming_lane() {
        let cfg = Config::reference();
        let mut mon = BehaviorMonitor::new();
        let p1 = Subject::Peer(*b"p1p1");
        for i in 0..100 {
            let t = f64::from(i) * 0.1;
            let me = obs(Subject::Me, t, 20.0 * t, -1.75, 20.0, 0.0);
            let peer = obs(p1, t, -40.0 + 28.0 * t, 1.75, 28.0, 0.0);
            mon.observe(&cfg, t, 0.1, me, &[peer], &NoLimits);
        }
        let ov: Vec<&Event> = mon
            .events()
            .iter()
            .filter(|e| matches!(e.kind, EventKind::Overtake { .. }))
            .collect();
        assert_eq!(ov.len(), 1);
        assert_eq!(ov[0].who, p1);
        assert!(matches!(
            ov[0].kind,
            EventKind::Overtake {
                oncoming: true,
                other: Subject::Me,
                ..
            }
        ));
    }

    #[test]
    fn tailgating_is_recorded_for_the_follower() {
        let cfg = Config::reference();
        let mut mon = BehaviorMonitor::new();
        let p1 = Subject::Peer(*b"p1p1");
        for i in 0..40 {
            let t = f64::from(i) * 0.1;
            // peer 10 m behind me (centre to centre) at 20 m/s: 5.5 m gap = 0.275 s
            mon.observe(
                &cfg,
                t,
                0.1,
                obs(Subject::Me, t, 20.0 * t, 0.0, 20.0, 0.0),
                &[obs(p1, t, 20.0 * t - 10.0, 0.0, 20.0, 0.0)],
                &NoLimits,
            );
        }
        assert!(mon.events().iter().any(|e| e.who == p1
            && matches!(
                e.kind,
                EventKind::Tailgating {
                    other: Subject::Me,
                    ..
                }
            )));
        assert!(mon.profiles()[&p1].tailgate_s > 1.5);
    }

    #[test]
    fn spec_overtake_counts_only_once_10_m_clear() {
        let cfg = Config::spec_v0();
        let mut mon = BehaviorMonitor::new();
        let p1 = Subject::Peer(*b"p1p1");
        let mut counted_at = None;
        for i in 0..100 {
            let t = f64::from(i) * 0.1;
            let me = obs(Subject::Me, t, 20.0 * t, -1.75, 20.0, 0.0);
            let peer = obs(p1, t, -40.0 + 28.0 * t, 1.75, 28.0, 0.0);
            mon.observe(&cfg, t, 0.1, me, &[peer], &NoLimits);
            if counted_at.is_none() && mon.profiles()[&p1].overtakes == 1 {
                counted_at = Some(8.0 * t - 40.0); // how far ahead the peer was
            }
        }
        assert!(
            counted_at.is_some_and(|d| (10.0..11.0).contains(&d)),
            "{counted_at:?}"
        );
        assert_eq!(mon.profiles()[&p1].oncoming_overtakes, 1);
        assert_eq!(mon.profiles()[&p1].score(&cfg), 96); // -4: oncoming lane
    }

    #[test]
    fn spec_speeding_needs_10_kmh_for_3_s_and_ends_below_limit_plus_5() {
        let cfg = Config::spec_v0();
        let limit = 50.0 / 3.6;
        let mut mon = BehaviorMonitor::new();
        let p1 = Subject::Peer(*b"p1p1");
        let me = |t| obs(Subject::Me, t, 0.0, 0.0, 0.0, 0.0);
        // 58 km/h: over the limit but not by 10 km/h -> nothing
        for i in 0..50 {
            let t = f64::from(i) * 0.1;
            mon.observe(
                &cfg,
                t,
                0.1,
                me(t),
                &[obs(p1, t, 0.0, 0.0, 58.0 / 3.6, 0.0)],
                &Fixed(limit),
            );
        }
        assert_eq!(mon.profiles()[&p1].speeding_episodes, 0);
        // 62 km/h for 3 s -> one episode, penalty 6
        for i in 50..90 {
            let t = f64::from(i) * 0.1;
            mon.observe(
                &cfg,
                t,
                0.1,
                me(t),
                &[obs(p1, t, 0.0, 0.0, 62.0 / 3.6, 0.0)],
                &Fixed(limit),
            );
        }
        assert_eq!(mon.profiles()[&p1].speeding_episodes, 1);
        assert_eq!(mon.profiles()[&p1].score(&cfg), 94);
        // 56 km/h: still above limit + 5 -> same episode continues (hysteresis)
        for i in 90..110 {
            let t = f64::from(i) * 0.1;
            mon.observe(
                &cfg,
                t,
                0.1,
                me(t),
                &[obs(p1, t, 0.0, 0.0, 56.0 / 3.6, 0.0)],
                &Fixed(limit),
            );
        }
        assert!(
            !mon.events()
                .iter()
                .any(|e| matches!(e.kind, EventKind::SpeedingEnd { .. }))
        );
        // 54 km/h: below limit + 5 -> episode ends
        mon.observe(
            &cfg,
            11.0,
            0.1,
            me(11.0),
            &[obs(p1, 11.0, 0.0, 0.0, 54.0 / 3.6, 0.0)],
            &Fixed(limit),
        );
        assert!(
            mon.events()
                .iter()
                .any(|e| matches!(e.kind, EventKind::SpeedingEnd { .. }))
        );
    }

    #[test]
    fn spec_harsh_thresholds_and_once_per_episode() {
        let cfg = Config::spec_v0();
        let mut mon = BehaviorMonitor::new();
        let p1 = Subject::Peer(*b"p1p1");
        let me = |t| obs(Subject::Me, t, 0.0, 0.0, 0.0, 0.0);
        let decels = [-4.0, -4.6, -4.8, -3.0, -4.6, -1.0, -4.6]; // 4.0 is below 4.5; -3.0 does not end the episode
        for (i, a) in decels.iter().enumerate() {
            let t = i as f64 * 0.1;
            mon.observe(
                &cfg,
                t,
                0.1,
                me(t),
                &[obs(p1, t, 0.0, 0.0, 10.0, *a)],
                &NoLimits,
            );
        }
        assert_eq!(mon.profiles()[&p1].harsh_brakes, 2);
        // motorbikes: 3.2 m/s² is harsh for a car, not for a motorbike
        let mut mon = BehaviorMonitor::new();
        let mut bike = obs(p1, 0.0, 0.0, 0.0, 10.0, 3.2);
        bike.vehicle_type = VehicleType::Motorbike;
        mon.observe(&cfg, 0.0, 0.1, me(0.0), &[bike], &NoLimits);
        assert_eq!(mon.profiles()[&p1].harsh_accels, 0);
        let car = obs(Subject::Peer(*b"c1c1"), 0.0, 50.0, 0.0, 10.0, 3.2);
        mon.observe(&cfg, 0.1, 0.1, me(0.1), &[car], &NoLimits);
        assert_eq!(mon.profiles()[&Subject::Peer(*b"c1c1")].harsh_accels, 1);
    }

    #[test]
    fn spec_score_recovers_1_2_points_per_clean_minute_and_caps_at_100() {
        let cfg = Config::spec_v0();
        let mut mon = BehaviorMonitor::new();
        let me = |t, a| obs(Subject::Me, t, 0.0, 0.0, 10.0, a);
        mon.observe(&cfg, 0.0, 0.1, me(0.0, -5.0), &[], &NoLimits); // harsh brake: -5
        assert_eq!(mon.profiles()[&Subject::Me].score(&cfg), 95);
        let mut t = 0.0;
        for _ in 0..600 {
            t += 0.1;
            mon.observe(&cfg, t, 0.1, me(t, 0.0), &[], &NoLimits);
        }
        let s = mon.profiles()[&Subject::Me].episode_score;
        assert!((s - 96.2).abs() < 0.05, "{s}"); // one clean minute: +1.2
        for _ in 0..6000 {
            t += 0.1;
            mon.observe(&cfg, t, 0.1, me(t, 0.0), &[], &NoLimits);
        }
        assert_eq!(mon.profiles()[&Subject::Me].score(&cfg), 100);
    }

    #[test]
    fn spec_peer_records_are_deleted_after_expiry_plus_10_minutes() {
        let cfg = Config::spec_v0();
        let mut mon = BehaviorMonitor::new();
        let p1 = Subject::Peer(*b"p1p1");
        let me = |t| obs(Subject::Me, t, 0.0, 0.0, 0.0, 0.0);
        mon.observe(
            &cfg,
            0.0,
            0.1,
            me(0.0),
            &[obs(p1, 0.0, 10.0, 0.0, 10.0, -6.0)],
            &NoLimits,
        );
        assert!(mon.profiles().contains_key(&p1));
        mon.observe(&cfg, 600.0, 0.1, me(600.0), &[], &NoLimits);
        assert!(mon.profiles().contains_key(&p1)); // 2 s expiry + 600 s not yet over
        mon.observe(&cfg, 603.0, 0.1, me(603.0), &[], &NoLimits);
        assert!(!mon.profiles().contains_key(&p1));
        assert!(mon.events().iter().all(|e| e.who != p1)); // its events are gone too
    }

    #[test]
    fn lost_contact_closes_an_open_speeding_episode_at_last_sight() {
        let cfg = Config::reference();
        let mut mon = BehaviorMonitor::new();
        let p1 = Subject::Peer(*b"p1p1");
        let me = |t| obs(Subject::Me, t, 0.0, 0.0, 0.0, 0.0);
        for i in 0..30 {
            let t = f64::from(i) * 0.1;
            mon.observe(
                &cfg,
                t,
                0.1,
                me(t),
                &[obs(p1, t, 30.0 * t, 0.0, 30.0, 0.0)],
                &Fixed(50.0 / 3.6),
            );
        }
        // silent for 5 s, then seen again under the limit
        mon.observe(
            &cfg,
            8.0,
            0.1,
            me(8.0),
            &[obs(p1, 8.0, 200.0, 0.0, 10.0, 0.0)],
            &Fixed(50.0 / 3.6),
        );
        let end = mon
            .events()
            .iter()
            .find(|e| matches!(e.kind, EventKind::SpeedingEnd { .. }));
        assert!(end.is_some_and(|e| (e.t - 2.9).abs() < 1e-9));
    }
}
