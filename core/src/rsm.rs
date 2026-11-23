//! RoadMesh Safety Message (RSM): the fixed 31-byte binary body of SPEC §3, and
//! the radio frame that carries it.
//!
//! Untrusted bytes are parsed only here (AUDIT S-09): every read is length-checked,
//! the module contains no `unsafe` (forbidden crate-wide) and no panics; malformed
//! input returns [`DecodeError`].
//!
//! # Body layout (big-endian, 31 bytes)
//!
//! | off | bytes | field | encoding |
//! |----:|----:|---|---|
//! | 0 | 1 | version (hi 4 bits) + msg_type (lo 4) | 0 heartbeat, 1 event |
//! | 1 | 4 | temp_id | pseudonym |
//! | 5 | 2 | msg_id | counter per temp_id |
//! | 7 | 4 | timestamp | ms since the hour |
//! | 11 | 4 | lat | i32, 1e-7° |
//! | 15 | 4 | lon | i32, 1e-7° |
//! | 19 | 2 | speed | u16, 0.02 m/s |
//! | 21 | 2 | heading | u16, 0.01° compass |
//! | 23 | 1 | accel_long | i8, 0.1 m/s² |
//! | 24 | 1 | yaw_rate | i8, 0.5 °/s (counter-clockwise positive) |
//! | 25 | 1 | vehicle_type (hi 4) + flags (lo 4) | |
//! | 26 | 1 | pos_accuracy | u8, 0.25 m |
//! | 27 | 1 | hop_count (hi 4) + ttl (lo 4) | **mutable by relays** |
//! | 28 | 2 | origin_pos_age | u16 ms, **mutable by relays** |
//! | 30 | 1 | event_code | 0 = none |
//!
//! Relays change only `hop_count`, `ttl` and `origin_pos_age`. [`signing_bytes`]
//! zeroes those, so the originator's signature still verifies after relaying
//! while any change to the rest of the body breaks it.
//!
//! # Frame layout
//!
//! `[frame_version:1][flags:1][last_hop lat:4 lon:4, only if relayed][body:31][auth_len:1][auth:auth_len]`
//!
//! The frame header (last relay position) is outside the signature, like the
//! ETSI GeoNetworking forwarding header. `auth` carries the security envelope of
//! [`crate::pki`]: signature plus certificate digest (73 bytes) or full certificate (172).

/// Body length in bytes.
pub const BODY_LEN: usize = 31;
/// Current body version (SPEC §3: version 0 is frozen once the pilot starts).
pub const VERSION: u8 = 0;
/// Current frame version.
pub const FRAME_VERSION: u8 = 0;
/// Largest `auth` field accepted (bytes): kind (1) + ECDSA P-256 signature (64) +
/// full certificate (107) = 172, rounded up (SPEC §3, user decision Step 3).
pub const MAX_AUTH_LEN: usize = 176;
/// Milliseconds in one hour: timestamps wrap at this value.
pub const MS_PER_HOUR: u32 = 3_600_000;

const FLAG_RELAYED: u8 = 0x01;

/// Message type (low 4 bits of byte 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgType {
    /// Regular position/motion broadcast.
    Heartbeat = 0,
    /// Event (hard brake, crash, stopped traffic, …): wider relay, longer validity.
    Event = 1,
}

/// Vehicle type (SPEC §3). Determines the outline used for prediction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VehicleType {
    /// Car (4.5 × 1.8 m).
    Car = 0,
    /// Motorbike (2.0 × 0.8 m).
    Motorbike = 1,
    /// Tuk-tuk (3.5 × 1.6 m).
    TukTuk = 2,
    /// Truck (10 × 2.5 m).
    Truck = 3,
    /// Bus (12 × 2.5 m).
    Bus = 4,
    /// Pedestrian (0.6 × 0.6 m).
    Pedestrian = 5,
    /// Bicycle (1.8 × 0.6 m).
    Bicycle = 6,
}

impl VehicleType {
    /// (length, width) in metres.
    pub fn dims(self) -> (f64, f64) {
        match self {
            VehicleType::Car => (4.5, 1.8),
            VehicleType::Motorbike => (2.0, 0.8),
            VehicleType::TukTuk => (3.5, 1.6),
            VehicleType::Truck => (10.0, 2.5),
            VehicleType::Bus => (12.0, 2.5),
            VehicleType::Pedestrian => (0.6, 0.6),
            VehicleType::Bicycle => (1.8, 0.6),
        }
    }

    fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => VehicleType::Car,
            1 => VehicleType::Motorbike,
            2 => VehicleType::TukTuk,
            3 => VehicleType::Truck,
            4 => VehicleType::Bus,
            5 => VehicleType::Pedestrian,
            6 => VehicleType::Bicycle,
            _ => return None,
        })
    }
}

/// Event codes (SPEC §3, plus 5 "stopped traffic" added in Step 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventCode {
    /// Not an event message.
    None = 0,
    /// Crash detected.
    Crash = 1,
    /// Hard braking.
    HardBrake = 2,
    /// Driving the wrong way.
    WrongWay = 3,
    /// Other hazard.
    Hazard = 4,
    /// Stopped traffic (queue) on a travel lane.
    StoppedTraffic = 5,
}

impl EventCode {
    fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => EventCode::None,
            1 => EventCode::Crash,
            2 => EventCode::HardBrake,
            3 => EventCode::WrongWay,
            4 => EventCode::Hazard,
            5 => EventCode::StoppedTraffic,
            _ => return None,
        })
    }
}

/// Flag bits (low 4 bits of byte 25).
pub mod flags {
    /// Hard braking right now.
    pub const HARD_BRAKE: u8 = 0x1;
    /// Overtaking right now.
    pub const OVERTAKING: u8 = 0x2;
    /// Hazard lights on.
    pub const HAZARD_LIGHTS: u8 = 0x4;
    /// Emergency vehicle / emergency.
    pub const EMERGENCY: u8 = 0x8;
}

/// Pseudonym: 4 random bytes, rotated with the certificate (SPEC §10).
pub type TempId = [u8; 4];

/// A decoded RSM body with fields in natural units.
///
/// Values are what the wire can represent: encoding quantises them (see the
/// module table) and clamps out-of-range values to the field limits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rsm {
    /// Message type.
    pub msg_type: MsgType,
    /// Pseudonym.
    pub temp_id: TempId,
    /// Counter per temp_id.
    pub msg_id: u16,
    /// Milliseconds since the hour (< 3 600 000).
    pub timestamp_ms: u32,
    /// Latitude (degrees).
    pub lat_deg: f64,
    /// Longitude (degrees).
    pub lon_deg: f64,
    /// Speed (m/s, ≥ 0).
    pub speed: f64,
    /// Compass heading (degrees, [0, 360)).
    pub heading_deg: f64,
    /// Longitudinal acceleration (m/s², negative = braking).
    pub accel: f64,
    /// Yaw rate (rad/s, counter-clockwise positive).
    pub yaw_rate: f64,
    /// Vehicle type.
    pub vehicle_type: VehicleType,
    /// Flag bits, see [`flags`].
    pub flags: u8,
    /// Estimated horizontal position error (m).
    pub pos_accuracy_m: f64,
    /// Times this message has been forwarded.
    pub hop_count: u8,
    /// Forwards remaining.
    pub ttl: u8,
    /// ms since the origin sent it (relayed messages only).
    pub origin_pos_age_ms: u16,
    /// Event code.
    pub event: EventCode,
}

/// Why bytes could not be decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// Fewer bytes than the layout needs.
    TooShort,
    /// Bytes left over after the frame.
    TrailingBytes,
    /// Unknown body or frame version.
    UnsupportedVersion,
    /// Unknown message type.
    BadMsgType,
    /// Unknown vehicle type.
    BadVehicleType,
    /// Unknown event code.
    BadEventCode,
    /// Timestamp not within the hour.
    BadTimestamp,
    /// Latitude/longitude out of range.
    BadPosition,
    /// Heading ≥ 360°.
    BadHeading,
    /// Unknown frame flags.
    BadFrameFlags,
    /// `auth` longer than [`MAX_AUTH_LEN`].
    AuthTooLong,
}

fn q_round(v: f64, unit: f64, lo: f64, hi: f64) -> f64 {
    (v / unit).round().clamp(lo, hi)
}

impl Rsm {
    /// Encode to the 31-byte body.
    pub fn encode(&self) -> [u8; BODY_LEN] {
        let mut b = [0u8; BODY_LEN];
        b[0] = (VERSION << 4) | (self.msg_type as u8 & 0x0f);
        b[1..5].copy_from_slice(&self.temp_id);
        b[5..7].copy_from_slice(&self.msg_id.to_be_bytes());
        b[7..11].copy_from_slice(&(self.timestamp_ms % MS_PER_HOUR).to_be_bytes());
        let lat = q_round(self.lat_deg, 1e-7, -900_000_000.0, 900_000_000.0) as i32;
        let lon = q_round(self.lon_deg, 1e-7, -1_800_000_000.0, 1_800_000_000.0) as i32;
        b[11..15].copy_from_slice(&lat.to_be_bytes());
        b[15..19].copy_from_slice(&lon.to_be_bytes());
        let speed = q_round(self.speed, 0.02, 0.0, f64::from(u16::MAX)) as u16;
        b[19..21].copy_from_slice(&speed.to_be_bytes());
        let heading = (q_round(self.heading_deg.rem_euclid(360.0), 0.01, 0.0, 36_000.0) as u32
            % 36_000) as u16;
        b[21..23].copy_from_slice(&heading.to_be_bytes());
        b[23] = (q_round(self.accel, 0.1, -127.0, 127.0) as i8).to_be_bytes()[0];
        b[24] = (q_round(self.yaw_rate.to_degrees(), 0.5, -127.0, 127.0) as i8).to_be_bytes()[0];
        b[25] = ((self.vehicle_type as u8) << 4) | (self.flags & 0x0f);
        b[26] = q_round(self.pos_accuracy_m, 0.25, 0.0, 255.0) as u8;
        b[27] = ((self.hop_count.min(15)) << 4) | self.ttl.min(15);
        b[28..30].copy_from_slice(&self.origin_pos_age_ms.to_be_bytes());
        b[30] = self.event as u8;
        b
    }

    /// Decode a 31-byte body. Never panics.
    pub fn decode(b: &[u8]) -> Result<Self, DecodeError> {
        if b.len() < BODY_LEN {
            return Err(DecodeError::TooShort);
        }
        if b.len() > BODY_LEN {
            return Err(DecodeError::TrailingBytes);
        }
        let u16_at = |i: usize| u16::from_be_bytes([b[i], b[i + 1]]);
        let u32_at = |i: usize| u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        if b[0] >> 4 != VERSION {
            return Err(DecodeError::UnsupportedVersion);
        }
        let msg_type = match b[0] & 0x0f {
            0 => MsgType::Heartbeat,
            1 => MsgType::Event,
            _ => return Err(DecodeError::BadMsgType),
        };
        let timestamp_ms = u32_at(7);
        if timestamp_ms >= MS_PER_HOUR {
            return Err(DecodeError::BadTimestamp);
        }
        let lat_raw = u32_at(11) as i32;
        let lon_raw = u32_at(15) as i32;
        if !(-900_000_000..=900_000_000).contains(&lat_raw)
            || !(-1_800_000_000..=1_800_000_000).contains(&lon_raw)
        {
            return Err(DecodeError::BadPosition);
        }
        let heading = u16_at(21);
        if heading >= 36_000 {
            return Err(DecodeError::BadHeading);
        }
        let vehicle_type = VehicleType::from_u8(b[25] >> 4).ok_or(DecodeError::BadVehicleType)?;
        let event = EventCode::from_u8(b[30]).ok_or(DecodeError::BadEventCode)?;
        Ok(Self {
            msg_type,
            temp_id: [b[1], b[2], b[3], b[4]],
            msg_id: u16_at(5),
            timestamp_ms,
            lat_deg: f64::from(lat_raw) * 1e-7,
            lon_deg: f64::from(lon_raw) * 1e-7,
            speed: f64::from(u16_at(19)) * 0.02,
            heading_deg: f64::from(heading) * 0.01,
            accel: f64::from(i8::from_be_bytes([b[23]])) * 0.1,
            yaw_rate: (f64::from(i8::from_be_bytes([b[24]])) * 0.5).to_radians(),
            vehicle_type,
            flags: b[25] & 0x0f,
            pos_accuracy_m: f64::from(b[26]) * 0.25,
            hop_count: b[27] >> 4,
            ttl: b[27] & 0x0f,
            origin_pos_age_ms: u16_at(28),
            event,
        })
    }
}

/// The bytes covered by the originator's signature: the body with the fields
/// relays may change (hop_count, ttl, origin_pos_age) set to zero.
pub fn signing_bytes(body: &[u8; BODY_LEN]) -> [u8; BODY_LEN] {
    let mut s = *body;
    s[27] = 0;
    s[28] = 0;
    s[29] = 0;
    s
}

/// A radio frame: optional relay header + RSM body + authentication bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// Position (lat, lon degrees) of the phone that last transmitted this frame,
    /// present only when relayed (SPEC §5 rule 3). Not signed.
    pub last_hop: Option<(f64, f64)>,
    /// The RSM body exactly as signed by the originator (plus relay-mutable fields).
    pub body: [u8; BODY_LEN],
    /// Authentication bytes: the [`crate::pki::AuthEnvelope`].
    pub auth: Vec<u8>,
}

impl Frame {
    /// Serialise. Returns `None` if `auth` is longer than [`MAX_AUTH_LEN`].
    pub fn encode(&self) -> Option<Vec<u8>> {
        if self.auth.len() > MAX_AUTH_LEN {
            return None;
        }
        let mut out = Vec::with_capacity(2 + 8 + BODY_LEN + 1 + self.auth.len());
        out.push(FRAME_VERSION);
        match self.last_hop {
            Some((lat, lon)) => {
                out.push(FLAG_RELAYED);
                out.extend_from_slice(
                    &(q_round(lat, 1e-7, -900_000_000.0, 900_000_000.0) as i32).to_be_bytes(),
                );
                out.extend_from_slice(
                    &(q_round(lon, 1e-7, -1_800_000_000.0, 1_800_000_000.0) as i32).to_be_bytes(),
                );
            }
            None => out.push(0),
        }
        out.extend_from_slice(&self.body);
        out.push(self.auth.len() as u8);
        out.extend_from_slice(&self.auth);
        Some(out)
    }

    /// Parse untrusted bytes. Never panics; rejects anything not exactly one frame.
    pub fn decode(b: &[u8]) -> Result<Self, DecodeError> {
        let mut i = 0usize;
        let take = |i: &mut usize, n: usize| -> Result<&[u8], DecodeError> {
            let end = i.checked_add(n).ok_or(DecodeError::TooShort)?;
            let s = b.get(*i..end).ok_or(DecodeError::TooShort)?;
            *i = end;
            Ok(s)
        };
        let head = take(&mut i, 2)?;
        if head[0] != FRAME_VERSION {
            return Err(DecodeError::UnsupportedVersion);
        }
        let flags = head[1];
        if flags & !FLAG_RELAYED != 0 {
            return Err(DecodeError::BadFrameFlags);
        }
        let last_hop = if flags & FLAG_RELAYED != 0 {
            let p = take(&mut i, 8)?;
            let lat = i32::from_be_bytes([p[0], p[1], p[2], p[3]]);
            let lon = i32::from_be_bytes([p[4], p[5], p[6], p[7]]);
            if !(-900_000_000..=900_000_000).contains(&lat)
                || !(-1_800_000_000..=1_800_000_000).contains(&lon)
            {
                return Err(DecodeError::BadPosition);
            }
            Some((f64::from(lat) * 1e-7, f64::from(lon) * 1e-7))
        } else {
            None
        };
        let body_slice = take(&mut i, BODY_LEN)?;
        let mut body = [0u8; BODY_LEN];
        body.copy_from_slice(body_slice);
        Rsm::decode(&body)?; // validate the body now, so a Frame always holds a valid body
        let auth_len = usize::from(take(&mut i, 1)?[0]);
        if auth_len > MAX_AUTH_LEN {
            return Err(DecodeError::AuthTooLong);
        }
        let auth = take(&mut i, auth_len)?.to_vec();
        if i != b.len() {
            return Err(DecodeError::TrailingBytes);
        }
        Ok(Self {
            last_hop,
            body,
            auth,
        })
    }
}

/// Age in seconds of a timestamp (ms since the hour) relative to `now_ms`
/// (also ms since the hour), handling the hourly wrap. The result is in
/// [−1800, 1800): negative means the timestamp is in the future.
pub fn age_s(now_ms: u32, ts_ms: u32) -> f64 {
    let half = i64::from(MS_PER_HOUR / 2);
    let d = (i64::from(now_ms) - i64::from(ts_ms)).rem_euclid(i64::from(MS_PER_HOUR));
    let d = if d >= half {
        d - i64::from(MS_PER_HOUR)
    } else {
        d
    };
    d as f64 / 1000.0
}

/// Seconds of a monotonic clock → ms since the hour (the clock starts on an hour boundary).
pub fn ms_of_hour(now_s: f64) -> u32 {
    ((now_s * 1000.0).round() as i64).rem_euclid(i64::from(MS_PER_HOUR)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Rsm {
        Rsm {
            msg_type: MsgType::Heartbeat,
            temp_id: [0xde, 0xad, 0xbe, 0xef],
            msg_id: 4242,
            timestamp_ms: 3_599_999,
            lat_deg: 11.5564321,
            lon_deg: 104.9282123,
            speed: 13.88,
            heading_deg: 359.99,
            accel: -8.0,
            yaw_rate: (-12.5f64).to_radians(),
            vehicle_type: VehicleType::Motorbike,
            flags: flags::HARD_BRAKE | flags::OVERTAKING,
            pos_accuracy_m: 3.25,
            hop_count: 3,
            ttl: 2,
            origin_pos_age_ms: 250,
            event: EventCode::StoppedTraffic,
        }
    }

    #[test]
    fn body_is_31_bytes_and_round_trips_exactly_on_grid_values() {
        let m = sample();
        let b = m.encode();
        assert_eq!(b.len(), 31);
        let d = Rsm::decode(&b).ok();
        let Some(d) = d else {
            unreachable!("decode failed")
        };
        assert_eq!(d.temp_id, m.temp_id);
        assert_eq!(d.msg_id, m.msg_id);
        assert_eq!(d.timestamp_ms, m.timestamp_ms);
        assert!((d.lat_deg - m.lat_deg).abs() < 1e-9);
        assert!((d.lon_deg - m.lon_deg).abs() < 1e-9);
        assert!((d.speed - m.speed).abs() < 1e-9);
        assert!((d.heading_deg - m.heading_deg).abs() < 1e-9);
        assert!((d.accel - m.accel).abs() < 1e-9);
        assert!((d.yaw_rate - m.yaw_rate).abs() < 1e-9);
        assert_eq!(
            (
                d.vehicle_type,
                d.flags,
                d.hop_count,
                d.ttl,
                d.origin_pos_age_ms,
                d.event
            ),
            (
                m.vehicle_type,
                m.flags,
                m.hop_count,
                m.ttl,
                m.origin_pos_age_ms,
                m.event
            )
        );
        assert_eq!(d.encode(), b);
    }

    #[test]
    fn quantisation_error_is_within_half_a_unit() {
        let mut m = sample();
        m.speed = 13.8889; // 50 km/h
        m.accel = -1.2345;
        m.heading_deg = 12.3456;
        let Ok(d) = Rsm::decode(&m.encode()) else {
            unreachable!()
        };
        assert!((d.speed - m.speed).abs() <= 0.01 + 1e-12);
        assert!((d.accel - m.accel).abs() <= 0.05 + 1e-12);
        assert!((d.heading_deg - m.heading_deg).abs() <= 0.005 + 1e-12);
    }

    #[test]
    fn out_of_range_values_are_clamped_not_wrapped() {
        let mut m = sample();
        m.accel = -30.0;
        m.speed = 5000.0;
        let Ok(d) = Rsm::decode(&m.encode()) else {
            unreachable!()
        };
        assert!((d.accel - -12.7).abs() < 1e-9);
        assert!((d.speed - 1310.7).abs() < 1e-9);
    }

    #[test]
    fn signing_bytes_ignore_only_relay_fields() {
        let m = sample();
        let mut relayed = m;
        relayed.hop_count = 5;
        relayed.ttl = 0;
        relayed.origin_pos_age_ms = 999;
        assert_eq!(signing_bytes(&m.encode()), signing_bytes(&relayed.encode()));
        let mut tampered = m;
        tampered.speed = 0.0;
        assert_ne!(
            signing_bytes(&m.encode()),
            signing_bytes(&tampered.encode())
        );
    }

    #[test]
    fn decoder_rejects_bad_fields() {
        let good = sample().encode();
        assert_eq!(Rsm::decode(&good[..30]), Err(DecodeError::TooShort));
        let mut b = good;
        b[0] = 0x10;
        assert_eq!(Rsm::decode(&b), Err(DecodeError::UnsupportedVersion));
        let mut b = good;
        b[0] = 0x07;
        assert_eq!(Rsm::decode(&b), Err(DecodeError::BadMsgType));
        let mut b = good;
        b[7..11].copy_from_slice(&MS_PER_HOUR.to_be_bytes());
        assert_eq!(Rsm::decode(&b), Err(DecodeError::BadTimestamp));
        let mut b = good;
        b[21..23].copy_from_slice(&36_000u16.to_be_bytes());
        assert_eq!(Rsm::decode(&b), Err(DecodeError::BadHeading));
        let mut b = good;
        b[25] = 0xf0;
        assert_eq!(Rsm::decode(&b), Err(DecodeError::BadVehicleType));
        let mut b = good;
        b[30] = 99;
        assert_eq!(Rsm::decode(&b), Err(DecodeError::BadEventCode));
        let mut b = good;
        b[11..15].copy_from_slice(&i32::MAX.to_be_bytes());
        assert_eq!(Rsm::decode(&b), Err(DecodeError::BadPosition));
    }

    #[test]
    fn frame_round_trip_direct_and_relayed() {
        let body = sample().encode();
        for last_hop in [None, Some((11.55, 104.93))] {
            let f = Frame {
                last_hop,
                body,
                auth: vec![7; 64],
            };
            let Some(bytes) = f.encode() else {
                unreachable!()
            };
            let Ok(g) = Frame::decode(&bytes) else {
                unreachable!()
            };
            assert_eq!(g.body, f.body);
            assert_eq!(g.auth, f.auth);
            assert_eq!(g.last_hop.is_some(), f.last_hop.is_some());
        }
        assert!(
            Frame {
                last_hop: None,
                body,
                auth: vec![0; MAX_AUTH_LEN + 1]
            }
            .encode()
            .is_none()
        );
    }

    #[test]
    fn frame_decoder_rejects_truncation_trailing_and_bad_flags() {
        let Some(bytes) = (Frame {
            last_hop: Some((1.0, 2.0)),
            body: sample().encode(),
            auth: vec![1, 2, 3],
        })
        .encode() else {
            unreachable!()
        };
        for n in 0..bytes.len() {
            assert!(Frame::decode(&bytes[..n]).is_err(), "prefix {n} accepted");
        }
        let mut long = bytes.clone();
        long.push(0);
        assert_eq!(Frame::decode(&long), Err(DecodeError::TrailingBytes));
        let mut bad = bytes.clone();
        bad[1] = 0x80;
        assert_eq!(Frame::decode(&bad), Err(DecodeError::BadFrameFlags));
    }

    /// Small deterministic generator so the test needs no dependency.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 11
        }
    }

    #[test]
    fn decoder_never_panics_on_random_or_mutated_bytes() {
        let mut r = Lcg(0x5eed);
        let Some(valid) = (Frame {
            last_hop: Some((11.5, 104.9)),
            body: sample().encode(),
            auth: vec![9; 64],
        })
        .encode() else {
            unreachable!()
        };
        for _ in 0..50_000 {
            // random length, random bytes
            let n = (r.next() % 160) as usize;
            let bytes: Vec<u8> = (0..n).map(|_| r.next() as u8).collect();
            let _ = Frame::decode(&bytes);
            let _ = Rsm::decode(&bytes);
            // a valid frame with a few bytes flipped
            let mut m = valid.clone();
            for _ in 0..(1 + r.next() % 4) {
                let i = (r.next() as usize) % m.len();
                m[i] ^= r.next() as u8;
            }
            if let Ok(f) = Frame::decode(&m) {
                // whatever decodes must re-encode to the same bytes
                assert_eq!(f.encode().as_deref(), Some(m.as_slice()));
            }
        }
    }

    #[test]
    fn timestamp_age_handles_the_hourly_wrap() {
        assert_eq!(age_s(1_000, 500), 0.5);
        assert_eq!(age_s(500, 1_000), -0.5);
        assert_eq!(age_s(200, MS_PER_HOUR - 300), 0.5); // sent just before the hour
        assert_eq!(age_s(MS_PER_HOUR - 300, 200), -0.5);
        assert_eq!(ms_of_hour(3600.25), 250);
    }
}
