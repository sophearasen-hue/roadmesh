//! Motion prediction and conflict detection.
//! Port of `reference/core-python/v2v/prediction.py`.
//!
//! Model: constant turn rate and constant acceleration, integrated numerically.
//! Speed never goes negative, so a braking vehicle is predicted to stop, not reverse.

use crate::geometry::{polygons_overlap, vehicle_box};

/// One predicted pose: (t, x, y, heading).
pub type Pose = (f64, f64, f64, f64);
/// Predicted poses at step, 2·step, … horizon.
pub type Trajectory = Vec<Pose>;

/// Motion state used for prediction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    /// x east (m).
    pub x: f64,
    /// y north (m).
    pub y: f64,
    /// heading (rad, 0 = east, counter-clockwise).
    pub heading: f64,
    /// speed (m/s).
    pub speed: f64,
    /// longitudinal acceleration (m/s²).
    pub accel: f64,
    /// yaw rate (rad/s).
    pub yaw_rate: f64,
}

/// Predict `horizon` seconds ahead in `step` increments, after first advancing
/// `lead_time` seconds (to bring a slightly old message up to "now").
pub fn trajectory(m: Motion, horizon: f64, step: f64, lead_time: f64) -> Trajectory {
    let n_lead = (lead_time / step).round_ties_even().max(0.0) as usize;
    let n = (horizon / step).round_ties_even().max(0.0) as usize;
    let (mut x, mut y, mut heading, mut speed) = (m.x, m.y, m.heading, m.speed);
    let mut pts = Vec::with_capacity(n);
    for i in 1..=(n_lead + n) {
        speed = (speed + m.accel * step).max(0.0);
        if speed > 0.0 {
            heading += m.yaw_rate * step;
        }
        x += speed * heading.cos() * step;
        y += speed * heading.sin() * step;
        if i > n_lead {
            pts.push(((i - n_lead) as f64 * step, x, y, heading));
        }
    }
    pts
}

/// First predicted time at which the two inflated outlines overlap, or `None`.
///
/// `time_buffer > 0` also counts A being where B was or will be within
/// ±`time_buffer` seconds (post-encroachment time), so "missing by 0.2 s" is
/// still a conflict.
pub fn first_conflict(
    a: &[Pose],
    dims_a: (f64, f64),
    b: &[Pose],
    dims_b: (f64, f64),
    margin: f64,
    time_buffer: f64,
    step: f64,
) -> Option<f64> {
    let reach = (dims_a.0.hypot(dims_a.1) + dims_b.0.hypot(dims_b.1)) / 2.0 + 2.0 * margin;
    let k = (time_buffer / step).round_ties_even().max(0.0) as usize;
    let n = a.len().min(b.len());
    for i in 0..n {
        let (t, ax, ay, ah) = a[i];
        let mut poly_a = None;
        for &(_, bx, by, bh) in &b[i.saturating_sub(k)..n.min(i + k + 1)] {
            if (ax - bx).hypot(ay - by) > reach {
                continue;
            }
            let pa =
                *poly_a.get_or_insert_with(|| vehicle_box(ax, ay, ah, dims_a.0, dims_a.1, margin));
            if polygons_overlap(&pa, &vehicle_box(bx, by, bh, dims_b.0, dims_b.1, margin)) {
                return Some(t);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    fn m(x: f64, y: f64, heading: f64, speed: f64, accel: f64, yaw_rate: f64) -> Motion {
        Motion {
            x,
            y,
            heading,
            speed,
            accel,
            yaw_rate,
        }
    }

    /// reference test_braking_car_stops
    #[test]
    fn braking_car_stops() {
        let tr = trajectory(m(0.0, 0.0, 0.0, 10.0, -5.0, 0.0), 5.0, 0.1, 0.0);
        let last = tr[tr.len() - 1];
        let earlier = tr[tr.len() - 15];
        assert!((last.1 - earlier.1).abs() < 1e-7, "stopped, not reversing");
        assert!(last.1 < 12.0);
    }

    /// reference test_turning_quarter_circle
    #[test]
    fn turning_quarter_circle() {
        let w = FRAC_PI_2 / 4.0;
        let tr = trajectory(m(0.0, 0.0, 0.0, 10.0, 0.0, w), 4.0, 0.01, 0.0);
        let last = tr[tr.len() - 1];
        assert!((last.3 - FRAC_PI_2).abs() < 0.005);
        let r = 10.0 / w;
        assert!((last.1 - r).abs() < 0.3);
        assert!((last.2 - r).abs() < 0.3);
    }

    /// reference test_crossing_conflict
    #[test]
    fn crossing_conflict() {
        let a = trajectory(m(-20.0, 0.0, 0.0, 10.0, 0.0, 0.0), 5.0, 0.1, 0.0);
        let b = trajectory(m(0.0, -20.0, FRAC_PI_2, 10.0, 0.0, 0.0), 5.0, 0.1, 0.0);
        let ttc = first_conflict(&a, (4.5, 1.8), &b, (4.5, 1.8), 0.5, 0.0, 0.1);
        assert!(matches!(ttc, Some(t) if t < 2.0));
    }

    #[test]
    fn time_buffer_catches_near_misses() {
        // B clears the crossing about 0.9 s before A's front arrives: no overlap,
        // but inside a 1 s post-encroachment buffer
        let a = trajectory(m(-35.0, 0.0, 0.0, 10.0, 0.0, 0.0), 5.0, 0.1, 0.0);
        let b = trajectory(m(0.0, -20.0, FRAC_PI_2, 10.0, 0.0, 0.0), 5.0, 0.1, 0.0);
        assert!(first_conflict(&a, (4.5, 1.8), &b, (4.5, 1.8), 0.0, 0.0, 0.1).is_none());
        assert!(first_conflict(&a, (4.5, 1.8), &b, (4.5, 1.8), 0.0, 1.0, 0.1).is_some());
    }

    #[test]
    fn lead_time_advances_the_start() {
        let base = m(0.0, 0.0, 0.0, 10.0, 0.0, 0.0);
        let tr = trajectory(base, 1.0, 0.1, 0.2);
        assert!((tr[0].1 - 3.0).abs() < 1e-9);
        assert_eq!(tr.len(), 10);
    }
}
