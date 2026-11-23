//! 2-D geometry: vehicle outlines, overlap tests, line of sight.
//! Port of `reference/core-python/v2v/geometry.py`.

/// A point (x east, y north) in metres.
pub type Point = (f64, f64);

/// Corners of a vehicle outline (rotated rectangle) centred on (x, y).
pub fn vehicle_box(
    x: f64,
    y: f64,
    heading: f64,
    length: f64,
    width: f64,
    margin: f64,
) -> [Point; 4] {
    let (s, c) = heading.sin_cos();
    let hl = length / 2.0 + margin;
    let hw = width / 2.0 + margin;
    [(hl, hw), (hl, -hw), (-hl, -hw), (-hl, hw)]
        .map(|(dx, dy)| (x + c * dx - s * dy, y + s * dx + c * dy))
}

/// Axis-aligned rectangle (for buildings in the simulator).
pub fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Point> {
    vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
}

fn axes(poly: &[Point]) -> impl Iterator<Item = Point> + '_ {
    (0..poly.len()).map(move |i| {
        let (x1, y1) = poly[i];
        let (x2, y2) = poly[(i + 1) % poly.len()];
        (y1 - y2, x2 - x1)
    })
}

fn project(poly: &[Point], (ax, ay): Point) -> (f64, f64) {
    poly.iter()
        .map(|&(px, py)| px * ax + py * ay)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(v), hi.max(v))
        })
}

/// Separating Axis Theorem for convex polygons.
pub fn polygons_overlap(a: &[Point], b: &[Point]) -> bool {
    for axis in axes(a).chain(axes(b)) {
        let (amin, amax) = project(a, axis);
        let (bmin, bmax) = project(b, axis);
        if amax < bmin || bmax < amin {
            return false;
        }
    }
    true
}

fn orient(p: Point, q: Point, r: Point) -> f64 {
    (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0)
}

/// Proper intersection of two segments (touching endpoints do not count).
pub fn segments_intersect(p1: Point, p2: Point, q1: Point, q2: Point) -> bool {
    let d1 = orient(q1, q2, p1);
    let d2 = orient(q1, q2, p2);
    let d3 = orient(p1, p2, q1);
    let d4 = orient(p1, p2, q2);
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

/// Ray-casting point-in-polygon test.
pub fn point_in_polygon(p: Point, poly: &[Point]) -> bool {
    let (x, y) = p;
    let mut inside = false;
    for i in 0..poly.len() {
        let (x1, y1) = poly[i];
        let (x2, y2) = poly[(i + 1) % poly.len()];
        if (y1 > y) != (y2 > y) && x < (x2 - x1) * (y - y1) / (y2 - y1) + x1 {
            inside = !inside;
        }
    }
    inside
}

/// Does the segment p–q touch the polygon (an endpoint inside, or crossing an edge)?
pub fn segment_hits_polygon(p: Point, q: Point, poly: &[Point]) -> bool {
    if point_in_polygon(p, poly) || point_in_polygon(q, poly) {
        return true;
    }
    (0..poly.len()).any(|i| segments_intersect(p, q, poly[i], poly[(i + 1) % poly.len()]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    /// reference test_overlap
    #[test]
    fn overlap() {
        assert!(polygons_overlap(
            &vehicle_box(0.0, 0.0, 0.0, 4.5, 1.8, 0.0),
            &vehicle_box(3.0, 0.0, FRAC_PI_2, 4.5, 1.8, 0.0)
        ));
        assert!(!polygons_overlap(
            &vehicle_box(0.0, 0.0, 0.0, 4.5, 1.8, 0.0),
            &vehicle_box(6.0, 0.0, 0.0, 4.5, 1.8, 0.0)
        ));
    }

    /// reference test_line_of_sight
    #[test]
    fn line_of_sight() {
        assert!(segment_hits_polygon(
            (-10.0, 0.0),
            (10.0, 0.0),
            &rect(-1.0, -1.0, 1.0, 1.0)
        ));
        assert!(!segment_hits_polygon(
            (-10.0, 5.0),
            (10.0, 5.0),
            &rect(-1.0, -1.0, 1.0, 1.0)
        ));
    }
}
