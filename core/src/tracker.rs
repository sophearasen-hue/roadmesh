//! Track smoothing (SPEC §6 tracking pipeline, steps 2 and 3).
//!
//! Phone GPS positions carry 2–10 m of error; the reported speed (Doppler),
//! heading, acceleration and turn rate are far more precise. Each track is a
//! Kalman filter on position whose prediction step dead-reckons with the
//! reported motion (constant turn rate and acceleration) and whose update step
//! uses the GPS fix with the sender's reported accuracy. The filter removes
//! fix-to-fix jitter; slowly drifting GPS bias is not observable from one phone's
//! messages, which is why the outlines are still lengthened by the reported
//! accuracy and why map matching snaps tracks to their lane.
//!
//! When a vehicle switches pseudonym, the new pseudonym continues the old track
//! if it appears within 3 m and 2 m/s of where the old one was predicted to be.

use std::collections::HashMap;

use crate::config::Config;
use crate::prediction::Motion;
use crate::rsm::TempId;

/// A smoothed track.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Track {
    /// Filtered position at `t` (local m).
    pub x: f64,
    /// See `x`.
    pub y: f64,
    /// Position variance per axis (m²).
    pub var: f64,
    /// Time of the last fix (s).
    pub t: f64,
    /// Last reported motion (its position is the raw fix).
    pub reported: Motion,
}

/// Position after `dt` seconds of constant turn rate and acceleration from `m`.
pub fn dead_reckon(m: &Motion, dt: f64) -> (f64, f64) {
    // never reverse: a braking vehicle stops
    let t_stop = if m.accel < 0.0 && m.speed > 0.0 {
        (m.speed / -m.accel).min(dt)
    } else {
        dt
    };
    let dist = (m.speed * t_stop + 0.5 * m.accel * t_stop * t_stop).max(0.0);
    let (x, y) = if m.yaw_rate.abs() < 1e-4 {
        (m.x + dist * m.heading.cos(), m.y + dist * m.heading.sin())
    } else {
        // arc of the same length
        let dh = m.yaw_rate * t_stop;
        let r = if t_stop > 0.0 { dist / dh } else { 0.0 };
        (
            m.x + r * ((m.heading + dh).sin() - m.heading.sin()),
            m.y - r * ((m.heading + dh).cos() - m.heading.cos()),
        )
    };
    (x, y)
}

impl Track {
    fn start(m: &Motion, pos_acc: f64, t: f64) -> Self {
        Self {
            x: m.x,
            y: m.y,
            var: pos_acc.max(1.0).powi(2),
            t,
            reported: *m,
        }
    }

    /// The filtered position carried forward to `t` with the reported motion.
    pub fn predict_to(&self, t: f64) -> (f64, f64) {
        dead_reckon(
            &Motion {
                x: self.x,
                y: self.y,
                ..self.reported
            },
            (t - self.t).max(0.0),
        )
    }

    /// The reported motion with the filtered position.
    pub fn motion(&self) -> Motion {
        Motion {
            x: self.x,
            y: self.y,
            ..self.reported
        }
    }

    fn update(&mut self, m: &Motion, pos_acc: f64, t: f64, cfg: &Config) {
        let dt = t - self.t;
        if dt <= 0.0 {
            return;
        }
        let (px, py) = self.predict_to(t);
        let var =
            self.var + cfg.track_process_noise_m2_s * dt + (cfg.track_speed_noise * dt).powi(2);
        let r = pos_acc.max(1.0).powi(2);
        let (ix, iy) = (m.x - px, m.y - py);
        if ix.hypot(iy) > cfg.track_reset_sigma * (var + r).sqrt() {
            *self = Self::start(m, pos_acc, t); // a jump the motion cannot explain: start over
            return;
        }
        let k = var / (var + r);
        self.x = px + k * ix;
        self.y = py + k * iy;
        self.var = (1.0 - k) * var;
        self.t = t;
        self.reported = *m;
    }
}

/// All tracks of one phone, by pseudonym.
#[derive(Debug, Clone, Default)]
pub struct Tracker {
    tracks: HashMap<TempId, Track>,
}

impl Tracker {
    /// No tracks.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a fix for `id` (a message already verified and checked).
    pub fn update(&mut self, id: TempId, m: &Motion, pos_acc: f64, t: f64, cfg: &Config) {
        if let Some(tr) = self.tracks.get_mut(&id) {
            tr.update(m, pos_acc, t, cfg);
            return;
        }
        // a new pseudonym: does it continue a track that just went quiet?
        let mut best: Option<(f64, TempId)> = None;
        for (old, tr) in &self.tracks {
            let gap = t - tr.t;
            if gap <= 0.0 || gap > cfg.track_link_max_gap_s {
                continue;
            }
            let (px, py) = tr.predict_to(t);
            let d = (m.x - px).hypot(m.y - py);
            if d <= cfg.track_link_max_dist_m
                && (m.speed - tr.reported.speed).abs() <= cfg.track_link_max_speed_diff
                && best.is_none_or(|b| d < b.0)
            {
                best = Some((d, *old));
            }
        }
        let track = match best.and_then(|(_, old)| self.tracks.remove(&old)) {
            Some(mut tr) => {
                tr.update(m, pos_acc, t, cfg);
                tr
            }
            None => Track::start(m, pos_acc, t),
        };
        self.tracks.insert(id, track);
    }

    /// The track of `id`.
    pub fn get(&self, id: &TempId) -> Option<&Track> {
        self.tracks.get(id)
    }

    /// Keep only the tracks for which `keep` is true (peer data leaves with the LDM entry).
    pub fn retain(&mut self, mut keep: impl FnMut(&TempId) -> bool) {
        self.tracks.retain(|id, _| keep(id));
    }

    /// Number of tracks.
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// No tracks.
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }
}

/// The phone's own position filter (same model).
#[derive(Debug, Clone, Default)]
pub struct OwnFilter {
    track: Option<Track>,
}

impl OwnFilter {
    /// Feed the phone's own fix and get the smoothed motion.
    pub fn smooth(&mut self, m: &Motion, pos_acc: f64, t: f64, cfg: &Config) -> Motion {
        match &mut self.track {
            Some(tr) if t > tr.t => tr.update(m, pos_acc, t, cfg),
            Some(_) => {}
            None => self.track = Some(Track::start(m, pos_acc, t)),
        }
        self.track.map_or(*m, |tr| tr.motion())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mv(x: f64, y: f64, speed: f64) -> Motion {
        Motion {
            x,
            y,
            heading: 0.0,
            speed,
            accel: 0.0,
            yaw_rate: 0.0,
        }
    }

    #[test]
    fn dead_reckoning_straight_turn_and_stop() {
        let (x, y) = dead_reckon(&mv(0.0, 0.0, 10.0), 1.0);
        assert!((x - 10.0).abs() < 1e-9 && y.abs() < 1e-9);
        // quarter circle of radius 10 m: 10 m/s at 1 rad/s for π/2 s
        let turn = Motion {
            yaw_rate: 1.0,
            ..mv(0.0, 0.0, 10.0)
        };
        let (x, y) = dead_reckon(&turn, std::f64::consts::FRAC_PI_2);
        assert!(
            (x - 10.0).abs() < 1e-6 && (y - 10.0).abs() < 1e-6,
            "{x} {y}"
        );
        let brake = Motion {
            accel: -5.0,
            ..mv(0.0, 0.0, 10.0)
        };
        let (x, _) = dead_reckon(&brake, 5.0);
        assert!((x - 10.0).abs() < 1e-9); // stops after 2 s and 10 m, never reverses
    }

    #[test]
    fn filter_removes_jitter() {
        let cfg = Config::spec_v0();
        let mut tr = Tracker::new();
        // true path x = 10 t; fixes jitter ±3 m alternately
        let mut worst_raw: f64 = 0.0;
        let mut worst_filtered: f64 = 0.0;
        for k in 0..100 {
            let t = f64::from(k) * 0.1;
            let jitter = if k % 2 == 0 { 3.0 } else { -3.0 };
            tr.update(*b"PEER", &mv(10.0 * t, jitter, 10.0), 3.0, t, &cfg);
            if k > 30 {
                let track =
                    tr.get(b"PEER")
                        .copied()
                        .unwrap_or(Track::start(&mv(0.0, 0.0, 0.0), 1.0, 0.0));
                worst_raw = worst_raw.max(jitter.abs());
                worst_filtered = worst_filtered
                    .max(track.y.abs())
                    .max((track.x - 10.0 * t).abs());
            }
        }
        assert!(
            worst_filtered < worst_raw / 3.0,
            "filtered {worst_filtered} raw {worst_raw}"
        );
    }

    #[test]
    fn a_jump_the_motion_cannot_explain_restarts_the_track() {
        let cfg = Config::spec_v0();
        let mut tr = Tracker::new();
        tr.update(*b"PEER", &mv(0.0, 0.0, 10.0), 2.0, 0.0, &cfg);
        tr.update(*b"PEER", &mv(1.0, 0.0, 10.0), 2.0, 0.1, &cfg);
        tr.update(*b"PEER", &mv(300.0, 0.0, 10.0), 2.0, 0.2, &cfg);
        assert_eq!(tr.get(b"PEER").map(|t| t.x), Some(300.0));
    }

    #[test]
    fn new_pseudonym_continues_the_old_track() {
        let cfg = Config::spec_v0();
        let mut tr = Tracker::new();
        for k in 0..10 {
            let t = f64::from(k) * 0.1;
            tr.update(*b"OLD1", &mv(10.0 * t, 0.0, 10.0), 3.0, t, &cfg);
        }
        let before = tr.get(b"OLD1").map(|t| t.var).unwrap_or(f64::NAN);
        tr.update(*b"NEW1", &mv(10.0, 0.5, 10.0), 3.0, 1.0, &cfg);
        assert!(tr.get(b"OLD1").is_none());
        let after = tr.get(b"NEW1").map(|t| t.var).unwrap_or(f64::NAN);
        assert!(
            after < 9.0 && after <= before + 1.0,
            "kept the filter state: {after}"
        );
        // a different vehicle far away starts its own track
        tr.update(*b"OTHR", &mv(200.0, 0.0, 10.0), 3.0, 1.0, &cfg);
        assert_eq!(tr.get(b"OTHR").map(|t| t.var), Some(9.0));
        // and a different speed is not linked either
        tr.update(*b"FAST", &mv(11.0, 0.0, 20.0), 3.0, 1.1, &cfg);
        assert!(tr.get(b"NEW1").is_some());
    }
}
