//! Local Dynamic Map: the latest accepted message per peer, plus the checks every
//! message passes before it is trusted (AUDIT S-03, S-06, S-08).
//! Port of `reference/core-python/v2v/ldm.py`, with the SPEC §5 duplicate cache.

use std::collections::HashMap;

use crate::config::Config;
use crate::geo::{GeoOrigin, compass_deg_to_math};
use crate::prediction::Motion;
use crate::rsm::{self, EventCode, MsgType, Rsm, TempId, VehicleType};

/// A peer message in local units, as the safety logic uses it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PeerMsg {
    /// Pseudonym.
    pub temp_id: TempId,
    /// Counter per pseudonym.
    pub msg_id: u16,
    /// Heartbeat or event.
    pub msg_type: MsgType,
    /// When the originator sent it, on this phone's clock (s). Derived in whole ms.
    pub t: f64,
    /// Same as `t` in integer milliseconds (exact comparisons).
    pub t_ms: i64,
    /// Motion in the local frame.
    pub motion: Motion,
    /// Outline length (m), from the vehicle type.
    pub length: f64,
    /// Outline width (m), from the vehicle type.
    pub width: f64,
    /// Vehicle type.
    pub vehicle_type: VehicleType,
    /// Flag bits.
    pub flags: u8,
    /// Reported position accuracy (m).
    pub pos_accuracy_m: f64,
    /// Event code.
    pub event: EventCode,
}

impl PeerMsg {
    /// Convert a decoded RSM, received at `now_s`, into local units.
    pub fn from_rsm(r: &Rsm, origin: &GeoOrigin, now_s: f64) -> Self {
        let now_ms = (now_s * 1000.0).round() as i64;
        let age_ms = (rsm::age_s(rsm::ms_of_hour(now_s), r.timestamp_ms) * 1000.0).round() as i64;
        let t_ms = now_ms - age_ms;
        let (x, y) = origin.to_local(r.lat_deg, r.lon_deg);
        let (length, width) = r.vehicle_type.dims();
        Self {
            temp_id: r.temp_id,
            msg_id: r.msg_id,
            msg_type: r.msg_type,
            t: t_ms as f64 / 1000.0,
            t_ms,
            motion: Motion {
                x,
                y,
                heading: compass_deg_to_math(r.heading_deg),
                speed: r.speed,
                accel: r.accel,
                yaw_rate: r.yaw_rate,
            },
            length,
            width,
            vehicle_type: r.vehicle_type,
            flags: r.flags,
            pos_accuracy_m: r.pos_accuracy_m,
            event: r.event,
        }
    }
}

/// Outcome of the plausibility and freshness checks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Check {
    /// New, fresh and plausible.
    Ok,
    /// Already have this or a newer message: drop quietly (normal with relaying).
    Duplicate,
    /// Too old to be useful: drop quietly (normal for long relay chains).
    Stale,
    /// Physically impossible or abusive: drop, block and report the sender.
    Misbehaviour(Misbehaviour),
}

/// Misbehaviour reasons (AUDIT S-06, S-08).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Misbehaviour {
    /// Speed outside [0, max_plausible_speed] (m/s).
    ImplausibleSpeed(f64),
    /// |acceleration| above max_plausible_accel (m/s²).
    ImplausibleAccel(f64),
    /// Timestamp too far in the future.
    FutureTimestamp,
    /// Faster than one message per `min_msg_interval_s` (flood).
    RateTooHigh,
    /// Moved faster than physically possible since its last message (implied m/s).
    PositionJump(f64),
}

/// One LDM entry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Entry {
    /// Latest accepted message.
    pub msg: PeerMsg,
    /// When it was received (s).
    pub last_seen: f64,
    /// 0 = heard directly, n = via n relays.
    pub hops: u8,
    /// Order in which this peer entered the map (ties are broken by it, like the reference).
    pub seq: u64,
}

/// The Local Dynamic Map.
#[derive(Debug, Default, Clone)]
pub struct Ldm {
    entries: HashMap<TempId, Entry>,
    seen: HashMap<(TempId, u16), f64>,
    last_seen: HashMap<TempId, f64>,
    next_seq: u64,
}

impl Ldm {
    /// Empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Run the checks on a verified message. Does not modify the map.
    pub fn check(&self, msg: &PeerMsg, now: f64, cfg: &Config) -> Check {
        let m = &msg.motion;
        if !(0.0..=cfg.max_plausible_speed).contains(&m.speed) {
            return Check::Misbehaviour(Misbehaviour::ImplausibleSpeed(m.speed));
        }
        if m.accel.abs() > cfg.max_plausible_accel {
            return Check::Misbehaviour(Misbehaviour::ImplausibleAccel(m.accel));
        }
        let max_age = if msg.msg_type == MsgType::Event {
            cfg.event_max_age_s
        } else {
            cfg.far_max_age_s
        };
        if now - msg.t > max_age + 1e-9 {
            return Check::Stale;
        }
        if msg.t - now > cfg.max_future_s + 1e-9 {
            return Check::Misbehaviour(Misbehaviour::FutureTimestamp);
        }
        if self.seen.contains_key(&(msg.temp_id, msg.msg_id)) {
            return Check::Duplicate;
        }
        if let Some(prev) = self.entries.get(&msg.temp_id).map(|e| e.msg) {
            if msg.t_ms <= prev.t_ms {
                return Check::Duplicate;
            }
            let dt = msg.t - prev.t;
            if dt < cfg.min_msg_interval_s - 1e-9 {
                return Check::Misbehaviour(Misbehaviour::RateTooHigh);
            }
            let implied = (m.x - prev.motion.x).hypot(m.y - prev.motion.y) / dt;
            if implied
                > m.speed.max(prev.motion.speed) * cfg.jump_speed_factor + cfg.jump_speed_slack
            {
                return Check::Misbehaviour(Misbehaviour::PositionJump(implied));
            }
        }
        Check::Ok
    }

    /// Store an accepted message.
    pub fn update(&mut self, msg: PeerMsg, now: f64, hops: u8) {
        self.seen.insert((msg.temp_id, msg.msg_id), now);
        self.last_seen.insert(msg.temp_id, now);
        let seq = match self.entries.get(&msg.temp_id) {
            Some(e) => e.seq,
            None => {
                self.next_seq += 1;
                self.next_seq
            }
        };
        self.entries.insert(
            msg.temp_id,
            Entry {
                msg,
                last_seen: now,
                hops,
                seq,
            },
        );
    }

    /// Drop peers silent for longer than the TTL and expire the duplicate cache.
    pub fn evict(&mut self, now: f64, cfg: &Config) {
        self.entries
            .retain(|_, e| now - e.last_seen <= cfg.ldm_ttl_s + 1e-9);
        self.seen
            .retain(|_, &mut t| now - t <= cfg.duplicate_cache_s);
        // last_seen is kept for the yield fail-safe timeout, bounded by the cache window
        let keep = cfg.yield_timeout_s.max(cfg.duplicate_cache_s);
        self.last_seen.retain(|_, &mut t| now - t <= keep + 1.0);
    }

    /// Remove a peer at once (it misbehaved).
    pub fn remove(&mut self, temp_id: &TempId) {
        self.entries.remove(temp_id);
    }

    /// The current entry for a peer.
    pub fn get(&self, temp_id: &TempId) -> Option<&Entry> {
        self.entries.get(temp_id)
    }

    /// When a peer was last heard (s), even after it left the map.
    pub fn last_seen(&self, temp_id: &TempId) -> Option<f64> {
        self.last_seen.get(temp_id).copied()
    }

    /// All current entries in the order the peers entered the map (deterministic,
    /// and the same order the reference's dict iterates in).
    pub fn entries(&self) -> Vec<&Entry> {
        let mut v: Vec<&Entry> = self.entries.values().collect();
        v.sort_by_key(|e| e.seq);
        v
    }

    /// Number of peers in the map.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True if no peers are in the map.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(t: f64, x: f64, speed: f64, accel: f64, msg_id: u16) -> PeerMsg {
        PeerMsg {
            temp_id: *b"aaaa",
            msg_id,
            msg_type: MsgType::Heartbeat,
            t,
            t_ms: (t * 1000.0).round() as i64,
            motion: Motion {
                x,
                y: 0.0,
                heading: 0.0,
                speed,
                accel,
                yaw_rate: 0.0,
            },
            length: 4.5,
            width: 1.8,
            vehicle_type: VehicleType::Car,
            flags: 0,
            pos_accuracy_m: 1.0,
            event: EventCode::None,
        }
    }

    /// reference PlausibilityTests.test_checks
    #[test]
    fn checks() {
        let cfg = Config::reference();
        let mut ldm = Ldm::new();
        assert!(matches!(
            ldm.check(&msg(0.0, 0.0, 120.0, 0.0, 1), 0.0, &cfg),
            Check::Misbehaviour(Misbehaviour::ImplausibleSpeed(_))
        ));
        assert!(matches!(
            ldm.check(&msg(0.0, 0.0, 10.0, -30.0, 1), 0.0, &cfg),
            Check::Misbehaviour(Misbehaviour::ImplausibleAccel(_))
        ));
        assert_eq!(
            ldm.check(&msg(0.0, 0.0, 10.0, 0.0, 1), 3.0, &cfg),
            Check::Stale
        );
        assert_eq!(
            ldm.check(&msg(0.0, 0.0, 10.0, 0.0, 1), 1.0, &cfg),
            Check::Ok
        ); // old but fine for relayed data
        assert_eq!(
            ldm.check(&msg(5.0, 0.0, 10.0, 0.0, 1), 1.0, &cfg),
            Check::Misbehaviour(Misbehaviour::FutureTimestamp)
        );
        ldm.update(msg(1.0, 0.0, 10.0, 0.0, 1), 1.0, 0);
        assert_eq!(
            ldm.check(&msg(1.0, 0.0, 10.0, 0.0, 2), 1.1, &cfg),
            Check::Duplicate
        );
        assert!(matches!(
            ldm.check(&msg(1.1, 50.0, 10.0, 0.0, 2), 1.1, &cfg),
            Check::Misbehaviour(Misbehaviour::PositionJump(_))
        ));
        assert_eq!(
            ldm.check(&msg(1.1, 1.0, 10.0, 0.0, 2), 1.1, &cfg),
            Check::Ok
        );
    }

    /// reference test_message_flood_is_rejected (the LDM part)
    #[test]
    fn flood_is_misbehaviour() {
        let cfg = Config::reference();
        let mut ldm = Ldm::new();
        ldm.update(msg(1.0, 0.0, 10.0, 0.0, 1), 1.0, 0);
        assert_eq!(
            ldm.check(&msg(1.01, 0.1, 10.0, 0.0, 2), 1.01, &cfg),
            Check::Misbehaviour(Misbehaviour::RateTooHigh)
        );
        assert_eq!(
            ldm.check(&msg(1.05, 0.5, 10.0, 0.0, 2), 1.05, &cfg),
            Check::Ok
        );
    }

    #[test]
    fn same_msg_id_is_a_duplicate_even_if_relayed_later() {
        let cfg = Config::reference();
        let mut ldm = Ldm::new();
        ldm.update(msg(1.0, 0.0, 10.0, 0.0, 7), 1.0, 0);
        ldm.update(msg(1.1, 1.0, 10.0, 0.0, 8), 1.1, 0);
        assert_eq!(
            ldm.check(&msg(1.0, 0.0, 10.0, 0.0, 7), 1.2, &cfg),
            Check::Duplicate
        );
    }

    /// reference PlausibilityTests.test_eviction
    #[test]
    fn eviction() {
        let cfg = Config::reference();
        let mut ldm = Ldm::new();
        ldm.update(msg(0.0, 0.0, 10.0, 0.0, 1), 0.0, 0);
        ldm.evict(0.9, &cfg);
        assert!(ldm.get(b"aaaa").is_some());
        ldm.evict(1.5, &cfg);
        assert!(ldm.get(b"aaaa").is_none());
        assert_eq!(ldm.last_seen(b"aaaa"), Some(0.0));
    }

    #[test]
    fn relayed_copy_of_the_same_message_has_identical_time() {
        let o = GeoOrigin::phnom_penh();
        let r = Rsm {
            msg_type: MsgType::Heartbeat,
            temp_id: *b"bbbb",
            msg_id: 1,
            timestamp_ms: rsm::ms_of_hour(1.2),
            lat_deg: 11.5564,
            lon_deg: 104.9282,
            speed: 10.0,
            heading_deg: 90.0,
            accel: 0.0,
            yaw_rate: 0.0,
            vehicle_type: VehicleType::Car,
            flags: 0,
            pos_accuracy_m: 1.0,
            hop_count: 0,
            ttl: 2,
            origin_pos_age_ms: 0,
            event: EventCode::None,
        };
        let direct = PeerMsg::from_rsm(&r, &o, 1.2);
        let relayed = PeerMsg::from_rsm(&r, &o, 1.3);
        assert_eq!(direct.t_ms, relayed.t_ms);
        assert_eq!(direct.t, relayed.t);
    }
}
