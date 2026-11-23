//! Conversion between WGS-84 latitude/longitude (used on the air, SPEC §3) and a
//! local east/north frame in metres (used by all safety logic).
//!
//! Each phone picks an origin near itself. Over the few kilometres the safety logic
//! looks at, an equirectangular projection is accurate to well under the 1e-7°
//! (≈1 cm) resolution of the message, which the tests check.

/// Mean Earth radius (m), as used by the equirectangular projection.
const EARTH_RADIUS_M: f64 = 6_371_008.8;

/// A local tangent frame: x = metres east of the origin, y = metres north.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoOrigin {
    lat0_deg: f64,
    lon0_deg: f64,
    cos_lat0: f64,
}

impl GeoOrigin {
    /// Origin at the given latitude/longitude in degrees.
    pub fn new(lat_deg: f64, lon_deg: f64) -> Self {
        Self {
            lat0_deg: lat_deg,
            lon0_deg: lon_deg,
            cos_lat0: lat_deg.to_radians().cos(),
        }
    }

    /// Phnom Penh: the default origin used by the simulator.
    pub fn phnom_penh() -> Self {
        Self::new(11.5564, 104.9282)
    }

    /// Local metres → (lat, lon) degrees.
    pub fn to_latlon(&self, x: f64, y: f64) -> (f64, f64) {
        let lat = self.lat0_deg + (y / EARTH_RADIUS_M).to_degrees();
        let lon = self.lon0_deg + (x / (EARTH_RADIUS_M * self.cos_lat0)).to_degrees();
        (lat, lon)
    }

    /// (lat, lon) degrees → local metres.
    pub fn to_local(&self, lat_deg: f64, lon_deg: f64) -> (f64, f64) {
        let y = (lat_deg - self.lat0_deg).to_radians() * EARTH_RADIUS_M;
        let x = (lon_deg - self.lon0_deg).to_radians() * EARTH_RADIUS_M * self.cos_lat0;
        (x, y)
    }
}

/// Math heading (radians, 0 = east, counter-clockwise) → compass heading (degrees,
/// 0 = north, clockwise, in [0, 360)), as carried in the RSM.
pub fn math_to_compass_deg(heading_rad: f64) -> f64 {
    (90.0 - heading_rad.to_degrees()).rem_euclid(360.0)
}

/// Compass heading (degrees) → math heading (radians, in (−π, π]).
pub fn compass_deg_to_math(compass_deg: f64) -> f64 {
    let rad = (90.0 - compass_deg).to_radians();
    // normalise to (−π, π]
    let two_pi = std::f64::consts::TAU;
    let r = rad.rem_euclid(two_pi);
    if r > std::f64::consts::PI {
        r - two_pi
    } else {
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, PI};

    #[test]
    fn round_trip_is_centimetre_accurate_over_3_km() {
        let o = GeoOrigin::phnom_penh();
        for &(x, y) in &[
            (0.0, 0.0),
            (1500.0, -1.75),
            (-3000.0, 2500.0),
            (2999.9, -2999.9),
        ] {
            let (lat, lon) = o.to_latlon(x, y);
            let (x2, y2) = o.to_local(lat, lon);
            assert!((x - x2).abs() < 1e-6 && (y - y2).abs() < 1e-6);
        }
    }

    #[test]
    fn heading_conversion() {
        assert!((math_to_compass_deg(0.0) - 90.0).abs() < 1e-9); // east
        assert!((math_to_compass_deg(FRAC_PI_2) - 0.0).abs() < 1e-9); // north
        assert!((math_to_compass_deg(PI) - 270.0).abs() < 1e-9); // west
        assert!((math_to_compass_deg(-FRAC_PI_2) - 180.0).abs() < 1e-9); // south
        for &h in &[0.0, 0.3, FRAC_PI_2, PI, -FRAC_PI_2, -0.01] {
            let back = compass_deg_to_math(math_to_compass_deg(h));
            let d = (back - h + PI).rem_euclid(2.0 * PI) - PI;
            assert!(d.abs() < 1e-9, "{h} -> {back}");
        }
    }
}
