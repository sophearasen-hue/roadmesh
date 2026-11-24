//! Geohash map cells for the cellular bridge (SPEC §5 "Cellular bridge", §11): the
//! server pushes a hazard event to the cell it happened in; a phone subscribes to its
//! own cell and the 8 around it. Level 6 cells are about 1.2 km × 0.6 km.

/// Cell level used for event topics.
pub const LEVEL: usize = 6;

const BASE32: &[u8; 32] = b"0123456789bcdefghjkmnpqrstuvwxyz";

/// Geohash of a position, `level` characters (1–12). `None` for positions outside
/// the valid range or non-finite values.
pub fn encode(lat: f64, lon: f64, level: usize) -> Option<String> {
    if !(lat.is_finite() && lon.is_finite())
        || !(-90.0..=90.0).contains(&lat)
        || !(-180.0..=180.0).contains(&lon)
        || !(1..=12).contains(&level)
    {
        return None;
    }
    let (mut lat_r, mut lon_r) = ((-90.0, 90.0), (-180.0, 180.0));
    let mut out = String::with_capacity(level);
    let mut even = true;
    let mut bits = 0u8;
    let mut n = 0;
    while out.len() < level {
        let (r, v) = if even {
            (&mut lon_r, lon)
        } else {
            (&mut lat_r, lat)
        };
        let mid = (r.0 + r.1) / 2.0;
        bits <<= 1;
        if v >= mid {
            bits |= 1;
            r.0 = mid;
        } else {
            r.1 = mid;
        }
        even = !even;
        n += 1;
        if n == 5 {
            out.push(char::from(BASE32[usize::from(bits)]));
            bits = 0;
            n = 0;
        }
    }
    Some(out)
}

/// Cell size (degrees latitude, degrees longitude) at `level`.
pub fn cell_size(level: usize) -> (f64, f64) {
    let bits = 5 * level;
    let lon_bits = bits.div_ceil(2);
    let lat_bits = bits / 2;
    (
        180.0 / f64::from(1u32 << lat_bits),
        360.0 / f64::from(1u32 << lon_bits),
    )
}

/// The cell of a position and the 8 cells around it, level [`LEVEL`], own cell first.
/// Longitude wraps at ±180°; cells beyond the poles are left out.
pub fn neighbourhood(lat: f64, lon: f64) -> Vec<String> {
    let (dlat, dlon) = cell_size(LEVEL);
    let mut out: Vec<String> = Vec::with_capacity(9);
    for (i, j) in [
        (0, 0),
        (-1, -1),
        (-1, 0),
        (-1, 1),
        (0, -1),
        (0, 1),
        (1, -1),
        (1, 0),
        (1, 1),
    ] {
        let la = lat + f64::from(i) * dlat;
        let mut lo = lon + f64::from(j) * dlon;
        if lo > 180.0 {
            lo -= 360.0;
        }
        if lo < -180.0 {
            lo += 360.0;
        }
        if let Some(c) = encode(la, lo, LEVEL)
            && !out.contains(&c)
        {
            out.push(c);
        }
    }
    out
}

/// A well-formed cell name of level [`LEVEL`].
pub fn valid_cell(s: &str) -> bool {
    s.len() == LEVEL && s.bytes().all(|b| BASE32.contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_cells() {
        // Phnom Penh, Independence Monument
        assert_eq!(encode(11.5564, 104.9282, 6).as_deref(), Some("w649ex"));
        // the classic example from the geohash definition
        assert_eq!(
            encode(57.64911, 10.40744, 11).as_deref(),
            Some("u4pruydqqvj")
        );
        assert_eq!(encode(91.0, 0.0, 6), None);
        assert_eq!(encode(f64::NAN, 0.0, 6), None);
    }

    #[test]
    fn neighbourhood_is_nine_distinct_adjacent_cells() {
        let n = neighbourhood(11.5564, 104.9282);
        assert_eq!(n.len(), 9);
        assert_eq!(n[0], "w649ex");
        let (dlat, dlon) = cell_size(LEVEL);
        assert!((dlat - 0.0054931640625).abs() < 1e-12);
        assert!((dlon - 0.010986328125).abs() < 1e-12);
        // a point just across a cell edge is in the neighbourhood
        let near = encode(11.5564 + dlat, 104.9282 - dlon, LEVEL).unwrap_or_default();
        assert!(n.contains(&near));
        assert!(n.iter().all(|c| valid_cell(c)));
        assert!(!valid_cell("w64xpa") && !valid_cell("w64xp") && !valid_cell("+"));
    }
}
