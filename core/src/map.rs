//! Static road map (SPEC §6 LDM layer 1) and map matching.
//!
//! The map is a set of roads (polylines in the phone's local metric frame, see
//! [`crate::geo::GeoOrigin`]) with lane counts, lane width, speed limit and
//! direction. Junctions are found where roads cross. Map matching places a track
//! on a road, a direction of travel and a lane, with a confidence in [0, 1]:
//!
//! * the SPEC §6 confidence gate: tracks under `map_min_confidence` never raise
//!   Warning or Danger (they may still raise a Caution);
//! * lane snapping: a confident track away from a junction is moved sideways to
//!   its lane centre and turned to the road direction before prediction, so GPS
//!   noise cannot merge opposite lanes 3.5 m apart into a false head-on conflict;
//! * the SPEC §10 "not near a road" plausibility check;
//! * speed limits and "at a junction" for the alerts and behaviour scoring.
//!
//! Traffic drives on the right (Cambodia). Product maps come from an
//! OpenStreetMap extract (Step 8); the simulator builds its city grid directly.

use std::f64::consts::PI;

use crate::behavior::SpeedLimits;
use crate::prediction::Motion;
use crate::rsm::flags;

/// One road: a polyline with lanes.
#[derive(Debug, Clone, PartialEq)]
pub struct Road {
    /// Identifier (unique in the map).
    pub id: u32,
    /// Centre line in local metres, at least two points.
    pub points: Vec<(f64, f64)>,
    /// Lanes in each direction (two-way) or in total (one-way).
    pub lanes: u8,
    /// Lane width (m).
    pub lane_width_m: f64,
    /// Speed limit (m/s).
    pub limit: f64,
    /// Traffic only in the direction of the polyline.
    pub one_way: bool,
}

impl Road {
    /// Half the paved width (m).
    pub fn half_width(&self) -> f64 {
        let lanes = f64::from(self.lanes.max(1));
        if self.one_way {
            lanes * self.lane_width_m / 2.0
        } else {
            lanes * self.lane_width_m
        }
    }

    /// Lateral offsets (left of the polyline direction positive) of the lane
    /// centres for traffic going forward (`true`) or backward along the polyline.
    pub fn lane_centres(&self, forward: bool) -> Vec<f64> {
        let n = self.lanes.max(1);
        let w = self.lane_width_m;
        (0..n)
            .map(|k| {
                let k = f64::from(k);
                if self.one_way {
                    (k + 0.5) * w - f64::from(n) * w / 2.0
                } else if forward {
                    -(k + 0.5) * w // right-hand traffic: forward lanes on the right
                } else {
                    (k + 0.5) * w
                }
            })
            .collect()
    }
}

/// Where a track sits on the map.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapMatch {
    /// Road id.
    pub road: u32,
    /// Distance along the road from its first point (m).
    pub along_m: f64,
    /// Signed offset from the centre line, left positive (m).
    pub lateral_m: f64,
    /// Travelling in the polyline direction.
    pub forward: bool,
    /// Lateral offset of the nearest lane centre (m).
    pub lane_centre_m: f64,
    /// Lateral offsets of the outermost lane centres for this direction (m).
    pub lane_span_m: (f64, f64),
    /// Direction of travel on the road (rad, math convention).
    pub road_heading: f64,
    /// Inside a junction area (matching is ambiguous there; no snapping).
    pub in_junction: bool,
    /// Confidence in [0, 1].
    pub confidence: f64,
}

#[derive(Debug, Clone, Copy)]
struct Projection {
    road: usize,
    along: f64,
    lateral: f64,
    dir: f64,
    foot: (f64, f64),
}

/// The static road map.
#[derive(Debug, Clone, PartialEq)]
pub struct RoadMap {
    roads: Vec<Road>,
    junctions: Vec<(f64, f64)>,
    junction_radius_m: f64,
}

fn wrap(a: f64) -> f64 {
    (a + PI).rem_euclid(2.0 * PI) - PI
}

fn seg_intersection(
    a: (f64, f64),
    b: (f64, f64),
    c: (f64, f64),
    d: (f64, f64),
) -> Option<(f64, f64)> {
    let r = (b.0 - a.0, b.1 - a.1);
    let s = (d.0 - c.0, d.1 - c.1);
    let den = r.0 * s.1 - r.1 * s.0;
    if den.abs() < 1e-12 {
        return None;
    }
    let t = ((c.0 - a.0) * s.1 - (c.1 - a.1) * s.0) / den;
    let u = ((c.0 - a.0) * r.1 - (c.1 - a.1) * r.0) / den;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some((a.0 + t * r.0, a.1 + t * r.1))
}

impl RoadMap {
    /// Build a map. Roads with fewer than two points are dropped. Junctions are
    /// the crossing points of different roads; the junction area has radius
    /// `junction_radius_m` (it should cover the paved crossing plus the turn arcs).
    pub fn new(roads: Vec<Road>, junction_radius_m: f64) -> Self {
        let roads: Vec<Road> = roads.into_iter().filter(|r| r.points.len() >= 2).collect();
        let mut junctions: Vec<(f64, f64)> = Vec::new();
        for (i, a) in roads.iter().enumerate() {
            for b in roads.iter().skip(i + 1) {
                for sa in a.points.windows(2) {
                    for sb in b.points.windows(2) {
                        if let Some(p) = seg_intersection(sa[0], sa[1], sb[0], sb[1])
                            && junctions.iter().all(|q| (q.0 - p.0).hypot(q.1 - p.1) > 1.0)
                        {
                            junctions.push(p);
                        }
                    }
                }
            }
        }
        Self {
            roads,
            junctions,
            junction_radius_m,
        }
    }

    /// The roads.
    pub fn roads(&self) -> &[Road] {
        &self.roads
    }

    /// Junction centres.
    pub fn junctions(&self) -> &[(f64, f64)] {
        &self.junctions
    }

    fn is_one_way(&self, id: u32) -> bool {
        self.roads.iter().any(|r| r.id == id && r.one_way)
    }

    /// Is this point inside a junction area?
    pub fn at_junction(&self, x: f64, y: f64) -> bool {
        self.junctions
            .iter()
            .any(|j| (j.0 - x).hypot(j.1 - y) <= self.junction_radius_m)
    }

    /// Distance to the nearest junction centre (m).
    pub fn distance_to_junction(&self, x: f64, y: f64) -> f64 {
        self.junctions
            .iter()
            .map(|j| (j.0 - x).hypot(j.1 - y))
            .fold(f64::INFINITY, f64::min)
    }

    fn projections(&self, x: f64, y: f64, radius: f64) -> Vec<Projection> {
        let mut out = Vec::new();
        for (ri, r) in self.roads.iter().enumerate() {
            let mut along0 = 0.0;
            let mut best: Option<(f64, Projection)> = None;
            for s in r.points.windows(2) {
                let (a, b) = (s[0], s[1]);
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                let len = dx.hypot(dy);
                if len < 1e-9 {
                    continue;
                }
                let (ux, uy) = (dx / len, dy / len);
                let t = ((x - a.0) * ux + (y - a.1) * uy).clamp(0.0, len);
                let foot = (a.0 + t * ux, a.1 + t * uy);
                let lateral = -(x - a.0) * uy + (y - a.1) * ux;
                let d = (x - foot.0).hypot(y - foot.1);
                if best.as_ref().is_none_or(|b| d < b.0) {
                    best = Some((
                        d,
                        Projection {
                            road: ri,
                            along: along0 + t,
                            lateral,
                            dir: uy.atan2(ux),
                            foot,
                        },
                    ));
                }
                along0 += len;
            }
            if let Some((d, p)) = best
                && d <= radius + r.half_width()
            {
                out.push(p);
            }
        }
        out
    }

    /// Distance from a point to the nearest paved road edge (0 on the road, m).
    pub fn distance_off_road(&self, x: f64, y: f64) -> f64 {
        self.projections(x, y, f64::INFINITY)
            .iter()
            .map(|p| self.outside(p, x, y))
            .fold(f64::INFINITY, f64::min)
    }

    /// How far the point lies outside the paved width of the projected road (m).
    fn outside(&self, p: &Projection, x: f64, y: f64) -> f64 {
        let d = (x - p.foot.0).hypot(y - p.foot.1);
        (d - self.roads[p.road].half_width()).max(0.0)
    }

    /// Match a track to a road, direction and lane.
    ///
    /// Position likelihood: how far outside the paved width the point lies,
    /// relative to its accuracy. Heading likelihood (outside junctions, when
    /// moving): agreement with the road direction. Inside a junction any road
    /// that contains the point is acceptable, so turning vehicles keep full
    /// confidence where the crossing conflicts happen.
    pub fn match_motion(
        &self,
        m: &Motion,
        pos_accuracy_m: f64,
        flag_bits: u8,
        heading_sigma_deg: f64,
        position_sigma_factor: f64,
    ) -> Option<MapMatch> {
        let sigma = pos_accuracy_m.max(2.0) * position_sigma_factor.max(1.0);
        let in_junction = self.at_junction(m.x, m.y);
        let mut best: Option<MapMatch> = None;
        for p in self.projections(m.x, m.y, 3.0 * sigma + 10.0) {
            let r = &self.roads[p.road];
            let out = self.outside(&p, m.x, m.y);
            let pos_like = (-0.5 * (out / sigma).powi(2)).exp();
            let fwd_err = wrap(m.heading - p.dir).abs();
            let back_err = PI - fwd_err;
            let forward = r.one_way || fwd_err <= back_err;
            let err = if forward { fwd_err } else { back_err };
            let moving = m.speed > 2.0;
            let head_like = if moving && !in_junction {
                (-0.5 * (err.to_degrees() / heading_sigma_deg).powi(2)).exp()
            } else {
                1.0
            };
            let confidence = pos_like * head_like;
            let lanes = if flag_bits & flags::OVERTAKING != 0 && !r.one_way {
                // overtaking may use the oncoming lanes
                let mut v = r.lane_centres(true);
                v.extend(r.lane_centres(false));
                v
            } else {
                r.lane_centres(forward)
            };
            let lane_centre_m = lanes
                .iter()
                .copied()
                .min_by(|a, b| (a - p.lateral).abs().total_cmp(&(b - p.lateral).abs()))
                .unwrap_or(0.0);
            let lo = lanes.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = lanes.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let road_heading = if forward { p.dir } else { wrap(p.dir + PI) };
            let cand = MapMatch {
                road: r.id,
                along_m: p.along,
                lateral_m: p.lateral,
                forward,
                lane_centre_m,
                lane_span_m: (lo, hi),
                road_heading,
                in_junction,
                confidence,
            };
            if best.as_ref().is_none_or(|b| cand.confidence > b.confidence) {
                best = Some(cand);
            }
        }
        best
    }

    /// Lane snapping: a confident match away from junctions is kept on its own side
    /// of the road (its lateral offset clamped between the lane centres of its
    /// direction), and a vehicle driving straight along the road is aligned with it
    /// and its turn rate set to zero. Within its own side the measured offset is
    /// kept: phone GPS cannot tell adjacent same-direction lanes apart, so no lane
    /// is invented (the alerts treat that case as uncertain). Returns the
    /// (possibly adjusted) motion and the match.
    pub fn snap(
        &self,
        m: &Motion,
        pos_accuracy_m: f64,
        flag_bits: u8,
        cfg: &crate::config::Config,
    ) -> (Motion, Option<MapMatch>) {
        let Some(mm) = self.match_motion(
            m,
            pos_accuracy_m,
            flag_bits,
            cfg.map_heading_sigma_deg,
            cfg.map_position_sigma_factor,
        ) else {
            return (*m, None);
        };
        if mm.in_junction || mm.confidence < cfg.map_min_confidence {
            return (*m, Some(mm));
        }
        let mut out = *m;
        let shift = mm.lateral_m.clamp(mm.lane_span_m.0, mm.lane_span_m.1) - mm.lateral_m;
        // move along the left normal of the polyline direction
        let dir = if mm.forward || self.is_one_way(mm.road) {
            mm.road_heading
        } else {
            wrap(mm.road_heading + PI)
        };
        out.x += -dir.sin() * shift;
        out.y += dir.cos() * shift;
        let err = wrap(m.heading - mm.road_heading).abs();
        if m.speed > 2.0
            && err <= cfg.map_snap_max_heading_deg.to_radians()
            && m.yaw_rate.abs() < cfg.map_snap_max_yaw_rate
        {
            // driving along the road: a small reported turn rate is sensor noise, and
            // integrated over 6 s it would bend the predicted path into the next lane
            out.heading = mm.road_heading;
            out.yaw_rate = 0.0;
        }
        (out, Some(mm))
    }
}

impl SpeedLimits for RoadMap {
    fn limit_at(&self, x: f64, y: f64) -> Option<f64> {
        self.projections(x, y, 5.0)
            .into_iter()
            .map(|p| (self.outside(&p, x, y), self.roads[p.road].limit))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, l)| l)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    /// A two-lane-each-way east-west road crossing a north-south road at (0, 0).
    fn cross() -> RoadMap {
        RoadMap::new(
            vec![
                Road {
                    id: 1,
                    points: vec![(-500.0, 0.0), (500.0, 0.0)],
                    lanes: 2,
                    lane_width_m: 3.4,
                    limit: 60.0 / 3.6,
                    one_way: false,
                },
                Road {
                    id: 2,
                    points: vec![(0.0, -500.0), (0.0, 500.0)],
                    lanes: 1,
                    lane_width_m: 3.5,
                    limit: 40.0 / 3.6,
                    one_way: false,
                },
            ],
            15.0,
        )
    }

    fn mv(x: f64, y: f64, heading: f64, speed: f64) -> Motion {
        Motion {
            x,
            y,
            heading,
            speed,
            accel: 0.0,
            yaw_rate: 0.0,
        }
    }

    #[test]
    fn finds_the_junction() {
        let m = cross();
        assert_eq!(m.junctions().len(), 1);
        assert!(m.at_junction(5.0, 5.0));
        assert!(!m.at_junction(40.0, 0.0));
    }

    #[test]
    fn matches_direction_and_lane_right_hand_traffic() {
        let m = cross();
        // eastbound on the outer right lane (y = -5.1)
        let mm = m
            .match_motion(&mv(100.0, -5.0, 0.0, 10.0), 3.0, 0, 20.0, 1.0)
            .unwrap_or_else(unreachable_match);
        assert_eq!(mm.road, 1);
        assert!(mm.forward);
        assert!((mm.lane_centre_m + 5.1).abs() < 1e-9);
        assert!(mm.confidence > 0.95);
        // westbound
        let w = m
            .match_motion(&mv(100.0, 2.0, PI, 10.0), 3.0, 0, 20.0, 1.0)
            .unwrap_or_else(unreachable_match);
        assert!(!w.forward);
        assert!((w.lane_centre_m - 1.7).abs() < 1e-9);
    }

    fn unreachable_match() -> MapMatch {
        MapMatch {
            road: 0,
            along_m: 0.0,
            lateral_m: 0.0,
            forward: true,
            lane_centre_m: 0.0,
            lane_span_m: (0.0, 0.0),
            road_heading: 0.0,
            in_junction: false,
            confidence: -1.0,
        }
    }

    #[test]
    fn noisy_position_snaps_back_into_its_own_lane() {
        let m = cross();
        let cfg = Config::spec_v0();
        // eastbound car whose GPS says it is 2.5 m into the westbound side
        let noisy = Motion {
            yaw_rate: 0.02,
            ..mv(100.0, 2.5, 0.05, 10.0)
        };
        let (s, mm) = m.snap(&noisy, 6.0, 0, &cfg);
        assert!(mm.is_some_and(|x| x.forward));
        assert!(
            (s.y + 1.7).abs() < 1e-9,
            "kept on the eastbound side, got y = {}",
            s.y
        );
        assert!(s.heading.abs() < 1e-9 && s.yaw_rate == 0.0); // aligned with the road, no drift
        // inside its own side the measured offset is kept (lanes cannot be resolved)
        let (s2, _) = m.snap(&mv(100.0, -3.0, 0.0, 10.0), 6.0, 0, &cfg);
        assert!((s2.y + 3.0).abs() < 1e-9);
    }

    #[test]
    fn overtaking_vehicle_is_not_pulled_back() {
        let m = cross();
        let cfg = Config::spec_v0();
        let (s, _) = m.snap(&mv(100.0, 1.6, 0.0, 15.0), 2.0, flags::OVERTAKING, &cfg);
        assert!(s.y > 0.0, "kept in the oncoming lane it is using");
    }

    #[test]
    fn no_snapping_in_junctions_or_when_unsure() {
        let m = cross();
        let cfg = Config::spec_v0();
        let turning = Motion {
            yaw_rate: 0.5,
            ..mv(3.0, -4.0, 0.8, 5.0)
        };
        let (s, mm) = m.snap(&turning, 3.0, 0, &cfg);
        assert_eq!(s, turning);
        assert!(mm.is_some_and(|x| x.in_junction && x.confidence > 0.9));
        // far off the road: low confidence, left alone
        let off = mv(100.0, 40.0, 0.0, 10.0);
        let (s2, mm2) = m.snap(&off, 3.0, 0, &cfg);
        assert_eq!(s2, off);
        assert!(mm2.is_none_or(|x| x.confidence < cfg.map_min_confidence));
    }

    #[test]
    fn heading_across_the_road_lowers_confidence() {
        let m = cross();
        let mm = m
            .match_motion(&mv(100.0, -5.0, PI / 2.0, 10.0), 3.0, 0, 20.0, 1.0)
            .unwrap_or_else(unreachable_match);
        assert!(mm.confidence < 0.1);
    }

    #[test]
    fn off_road_distance_and_speed_limits() {
        let m = cross();
        assert_eq!(m.distance_off_road(100.0, -3.0), 0.0);
        assert!((m.distance_off_road(100.0, 50.0) - 43.2).abs() < 1e-9); // 50 m from the 6.8 m-wide road edge
        assert!(
            m.limit_at(100.0, -3.0)
                .is_some_and(|l| (l - 60.0 / 3.6).abs() < 1e-9)
        );
        assert!(
            m.limit_at(0.0, 100.0)
                .is_some_and(|l| (l - 40.0 / 3.6).abs() < 1e-9)
        );
        assert!(m.limit_at(200.0, 200.0).is_none());
    }
}
