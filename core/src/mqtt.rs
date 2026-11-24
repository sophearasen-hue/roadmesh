//! MQTT 3.1.1 codec, the small subset RoadMesh uses (BUILD_PLAN Step 9): CONNECT,
//! CONNACK, PUBLISH at QoS 0, SUBSCRIBE, SUBACK, UNSUBSCRIBE, UNSUBACK, PINGREQ,
//! PINGRESP and DISCONNECT. Used by the edge server and by the phones (through the
//! FFI), so every byte from the network is parsed here, in Rust, with length checks
//! (CLAUDE.md security rules). Anything outside the subset is an error, not ignored:
//! no QoS 1/2, no retained messages, no wills, no user names or passwords (phones
//! authenticate with their device certificate in mutual TLS).
//!
//! Pure logic: no I/O. Feed bytes to [`decode`]; it returns a packet and how many
//! bytes it used, or `Ok(None)` if more bytes are needed.

/// Largest packet accepted, header included (bytes). A signed radio frame is at most
/// 218 bytes; enrolment replies carry 20 certificates (about 2.2 kB) and a device
/// certificate.
pub const MAX_PACKET: usize = 4096;

/// Longest topic name or filter (bytes).
pub const MAX_TOPIC: usize = 128;

/// Most filters in one SUBSCRIBE or UNSUBSCRIBE.
pub const MAX_FILTERS: usize = 16;

/// Longest client identifier (bytes).
pub const MAX_CLIENT_ID: usize = 64;

/// RoadMesh topics (docs/SERVER.md), shared by the edge server and the phones.
pub mod topic {
    /// Phone → server: one radio frame.
    pub const UP: &str = "rm/v1/up";
    /// Server → phones: prefix of hazard-event topics, followed by a geohash cell.
    pub const EVENTS: &str = "rm/v1/ev/";
    /// Server → phones: the signed revocation list.
    pub const CRL: &str = "rm/v1/crl";
    /// Phone → server: a misbehaviour report.
    pub const REPORT: &str = "rm/v1/report";
    /// Phone → server: delete my data.
    pub const DELETE: &str = "rm/v1/delete";
    /// Phone → server (enrolment port): certificate request.
    pub const ENROL: &str = "rm/v1/enrol";
    /// Server → phone: enrolment answer.
    pub const ENROL_REPLY: &str = "rm/v1/enrol/reply";
    /// Server → phone: report answer.
    pub const REPORT_REPLY: &str = "rm/v1/reply/report";
    /// Server → phone: delete answer.
    pub const DELETE_REPLY: &str = "rm/v1/reply/delete";
}

/// A packet of the subset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Packet {
    /// Open a session (clean session only).
    Connect {
        /// Client identifier (may be empty).
        client_id: String,
        /// Keep-alive interval (s).
        keep_alive: u16,
    },
    /// Answer to CONNECT. `code` 0 = accepted.
    ConnAck {
        /// Return code (0 accepted, 1–5 refused as in MQTT 3.1.1).
        code: u8,
    },
    /// A message at QoS 0.
    Publish {
        /// Topic name (no wildcards).
        topic: String,
        /// Payload.
        payload: Vec<u8>,
    },
    /// Subscribe to topic filters (granted at QoS 0).
    Subscribe {
        /// Packet identifier (non-zero).
        packet_id: u16,
        /// Topic filters.
        filters: Vec<String>,
    },
    /// Answer to SUBSCRIBE: one code per filter (0 = granted QoS 0, 0x80 = refused).
    SubAck {
        /// Packet identifier.
        packet_id: u16,
        /// Return codes.
        codes: Vec<u8>,
    },
    /// Unsubscribe.
    Unsubscribe {
        /// Packet identifier (non-zero).
        packet_id: u16,
        /// Topic filters.
        filters: Vec<String>,
    },
    /// Answer to UNSUBSCRIBE.
    UnsubAck {
        /// Packet identifier.
        packet_id: u16,
    },
    /// Keep-alive ping.
    PingReq,
    /// Answer to PINGREQ.
    PingResp,
    /// Close the session.
    Disconnect,
}

/// Why bytes were rejected. The connection is closed on any of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MqttError {
    /// The packet is larger than [`MAX_PACKET`].
    TooLarge,
    /// The remaining-length field is malformed.
    BadLength,
    /// Packet type or flags outside the subset (QoS 1/2, retain, will, password, ...).
    Unsupported,
    /// Fields inconsistent with the packet length, bad UTF-8, bad topic, ...
    Malformed,
}

const CONNECT: u8 = 1;
const CONNACK: u8 = 2;
const PUBLISH: u8 = 3;
const SUBSCRIBE: u8 = 8;
const SUBACK: u8 = 9;
const UNSUBSCRIBE: u8 = 10;
const UNSUBACK: u8 = 11;
const PINGREQ: u8 = 12;
const PINGRESP: u8 = 13;
const DISCONNECT: u8 = 14;

/// Decode one packet from the start of `buf`. `Ok(None)`: incomplete, read more.
pub fn decode(buf: &[u8]) -> Result<Option<(Packet, usize)>, MqttError> {
    let Some(&first) = buf.first() else {
        return Ok(None);
    };
    // remaining length: 1–4 bytes, 7 bits each
    let mut len: usize = 0;
    let mut i = 1;
    loop {
        let Some(&b) = buf.get(i) else {
            return Ok(None);
        };
        len |= usize::from(b & 0x7f) << (7 * (i - 1));
        i += 1;
        if b & 0x80 == 0 {
            break;
        }
        if i > 4 {
            return Err(MqttError::BadLength);
        }
    }
    let total = i + len;
    if total > MAX_PACKET {
        return Err(MqttError::TooLarge);
    }
    let Some(body) = buf.get(i..total) else {
        return Ok(None);
    };
    let packet = parse(first >> 4, first & 0x0f, body)?;
    Ok(Some((packet, total)))
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], MqttError> {
        let end = self.at.checked_add(n).ok_or(MqttError::Malformed)?;
        let s = self.b.get(self.at..end).ok_or(MqttError::Malformed)?;
        self.at = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, MqttError> {
        Ok(self.bytes(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, MqttError> {
        let s = self.bytes(2)?;
        Ok(u16::from_be_bytes([s[0], s[1]]))
    }

    fn string(&mut self, max: usize) -> Result<String, MqttError> {
        let n = usize::from(self.u16()?);
        if n > max {
            return Err(MqttError::Malformed);
        }
        let s = std::str::from_utf8(self.bytes(n)?).map_err(|_| MqttError::Malformed)?;
        if s.contains('\0') {
            return Err(MqttError::Malformed);
        }
        Ok(s.to_owned())
    }

    fn rest(&self) -> &'a [u8] {
        self.b.get(self.at..).unwrap_or_default()
    }

    fn done(&self) -> bool {
        self.at == self.b.len()
    }
}

fn parse(kind: u8, flags: u8, body: &[u8]) -> Result<Packet, MqttError> {
    let mut r = Reader { b: body, at: 0 };
    let expect_flags = |f: u8| {
        if flags == f {
            Ok(())
        } else {
            Err(MqttError::Unsupported)
        }
    };
    let p = match kind {
        CONNECT => {
            expect_flags(0)?;
            if r.string(4)? != "MQTT" || r.u8()? != 4 {
                return Err(MqttError::Unsupported);
            }
            // clean session only; no will, user name or password; reserved bit 0
            if r.u8()? != 0b0000_0010 {
                return Err(MqttError::Unsupported);
            }
            let keep_alive = r.u16()?;
            let client_id = r.string(MAX_CLIENT_ID)?;
            Packet::Connect {
                client_id,
                keep_alive,
            }
        }
        CONNACK => {
            expect_flags(0)?;
            if r.u8()? != 0 {
                return Err(MqttError::Unsupported); // no stored sessions
            }
            Packet::ConnAck { code: r.u8()? }
        }
        PUBLISH => {
            expect_flags(0)?; // QoS 0, no dup, no retain
            let topic = r.string(MAX_TOPIC)?;
            if !valid_topic(&topic) {
                return Err(MqttError::Malformed);
            }
            let payload = r.rest().to_vec();
            r.at = body.len();
            Packet::Publish { topic, payload }
        }
        SUBSCRIBE | UNSUBSCRIBE => {
            expect_flags(0b0010)?;
            let packet_id = r.u16()?;
            if packet_id == 0 {
                return Err(MqttError::Malformed);
            }
            let mut filters = Vec::new();
            while !r.done() {
                if filters.len() == MAX_FILTERS {
                    return Err(MqttError::Unsupported);
                }
                let f = r.string(MAX_TOPIC)?;
                if !valid_filter(&f) {
                    return Err(MqttError::Malformed);
                }
                if kind == SUBSCRIBE && r.u8()? > 2 {
                    return Err(MqttError::Malformed);
                }
                filters.push(f);
            }
            if filters.is_empty() {
                return Err(MqttError::Malformed);
            }
            if kind == SUBSCRIBE {
                Packet::Subscribe { packet_id, filters }
            } else {
                Packet::Unsubscribe { packet_id, filters }
            }
        }
        SUBACK => {
            expect_flags(0)?;
            let packet_id = r.u16()?;
            let codes = r.rest().to_vec();
            r.at = body.len();
            if codes.is_empty() || codes.len() > MAX_FILTERS {
                return Err(MqttError::Malformed);
            }
            Packet::SubAck { packet_id, codes }
        }
        UNSUBACK => {
            expect_flags(0)?;
            Packet::UnsubAck {
                packet_id: r.u16()?,
            }
        }
        PINGREQ => {
            expect_flags(0)?;
            Packet::PingReq
        }
        PINGRESP => {
            expect_flags(0)?;
            Packet::PingResp
        }
        DISCONNECT => {
            expect_flags(0)?;
            Packet::Disconnect
        }
        _ => return Err(MqttError::Unsupported),
    };
    if !r.done() {
        return Err(MqttError::Malformed);
    }
    Ok(p)
}

/// A topic name: non-empty, no wildcards.
pub fn valid_topic(t: &str) -> bool {
    !t.is_empty() && t.len() <= MAX_TOPIC && !t.contains(['+', '#', '\0'])
}

/// A topic filter: `+` only as a whole level, `#` only as the whole last level.
pub fn valid_filter(f: &str) -> bool {
    if f.is_empty() || f.len() > MAX_TOPIC || f.contains('\0') {
        return false;
    }
    let levels: Vec<&str> = f.split('/').collect();
    levels.iter().enumerate().all(|(i, l)| match *l {
        "+" => true,
        "#" => i == levels.len() - 1,
        _ => !l.contains(['+', '#']),
    })
}

/// Does `filter` match `topic`?
pub fn matches(filter: &str, topic: &str) -> bool {
    let mut t = topic.split('/');
    for f in filter.split('/') {
        match f {
            "#" => return true,
            "+" => {
                if t.next().is_none() {
                    return false;
                }
            }
            _ => {
                if t.next() != Some(f) {
                    return false;
                }
            }
        }
    }
    t.next().is_none()
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    let n = u16::try_from(b.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(&b[..usize::from(n)]);
}

/// Encode a packet. Callers keep within the limits (the encoder truncates nothing
/// silently except strings beyond 65 535 bytes, which the limits rule out).
pub fn encode(p: &Packet) -> Vec<u8> {
    let mut body = Vec::new();
    let first = match p {
        Packet::Connect {
            client_id,
            keep_alive,
        } => {
            put_str(&mut body, "MQTT");
            body.push(4);
            body.push(0b0000_0010);
            body.extend_from_slice(&keep_alive.to_be_bytes());
            put_str(&mut body, client_id);
            CONNECT << 4
        }
        Packet::ConnAck { code } => {
            body.extend_from_slice(&[0, *code]);
            CONNACK << 4
        }
        Packet::Publish { topic, payload } => {
            put_str(&mut body, topic);
            body.extend_from_slice(payload);
            PUBLISH << 4
        }
        Packet::Subscribe { packet_id, filters } => {
            body.extend_from_slice(&packet_id.to_be_bytes());
            for f in filters {
                put_str(&mut body, f);
                body.push(0);
            }
            (SUBSCRIBE << 4) | 0b0010
        }
        Packet::SubAck { packet_id, codes } => {
            body.extend_from_slice(&packet_id.to_be_bytes());
            body.extend_from_slice(codes);
            SUBACK << 4
        }
        Packet::Unsubscribe { packet_id, filters } => {
            body.extend_from_slice(&packet_id.to_be_bytes());
            for f in filters {
                put_str(&mut body, f);
            }
            (UNSUBSCRIBE << 4) | 0b0010
        }
        Packet::UnsubAck { packet_id } => {
            body.extend_from_slice(&packet_id.to_be_bytes());
            UNSUBACK << 4
        }
        Packet::PingReq => PINGREQ << 4,
        Packet::PingResp => PINGRESP << 4,
        Packet::Disconnect => DISCONNECT << 4,
    };
    let mut out = Vec::with_capacity(body.len() + 5);
    out.push(first);
    let mut len = body.len();
    loop {
        let mut b = (len % 128) as u8;
        len /= 128;
        if len > 0 {
            b |= 0x80;
        }
        out.push(b);
        if len == 0 {
            break;
        }
    }
    out.extend_from_slice(&body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(p: Packet) {
        let b = encode(&p);
        assert_eq!(decode(&b), Ok(Some((p.clone(), b.len()))), "{p:?}");
        // every strict prefix is incomplete, never an error or a different packet
        for n in 0..b.len() {
            assert_eq!(decode(&b[..n]), Ok(None), "{p:?} prefix {n}");
        }
    }

    #[test]
    fn every_packet_of_the_subset_roundtrips() {
        roundtrip(Packet::Connect {
            client_id: "rm-phone".into(),
            keep_alive: 60,
        });
        roundtrip(Packet::ConnAck { code: 0 });
        roundtrip(Packet::Publish {
            topic: "rm/v1/ev/w3gv2c".into(),
            payload: vec![7; 218],
        });
        roundtrip(Packet::Publish {
            topic: "t".into(),
            payload: vec![1; 3000], // two-byte remaining length
        });
        roundtrip(Packet::Subscribe {
            packet_id: 9,
            filters: vec!["rm/v1/ev/w3gv2c".into(), "rm/v1/ev/+".into()],
        });
        roundtrip(Packet::SubAck {
            packet_id: 9,
            codes: vec![0, 0x80],
        });
        roundtrip(Packet::Unsubscribe {
            packet_id: 10,
            filters: vec!["a/#".into()],
        });
        roundtrip(Packet::UnsubAck { packet_id: 10 });
        roundtrip(Packet::PingReq);
        roundtrip(Packet::PingResp);
        roundtrip(Packet::Disconnect);
    }

    #[test]
    fn two_packets_in_one_buffer() {
        let mut b = encode(&Packet::PingReq);
        b.extend(encode(&Packet::Disconnect));
        assert_eq!(decode(&b), Ok(Some((Packet::PingReq, 2))));
        assert_eq!(decode(&b[2..]), Ok(Some((Packet::Disconnect, 2))));
    }

    #[test]
    fn features_outside_the_subset_are_refused() {
        // PUBLISH at QoS 1, retained
        let mut b = encode(&Packet::Publish {
            topic: "a".into(),
            payload: vec![1],
        });
        b[0] |= 0b0010;
        assert_eq!(decode(&b), Err(MqttError::Unsupported));
        b[0] = (PUBLISH << 4) | 1;
        assert_eq!(decode(&b), Err(MqttError::Unsupported));
        // CONNECT with a password flag, or without clean session
        let mut c = encode(&Packet::Connect {
            client_id: String::new(),
            keep_alive: 30,
        });
        c[9] = 0b0100_0010;
        assert_eq!(decode(&c), Err(MqttError::Unsupported));
        c[9] = 0;
        assert_eq!(decode(&c), Err(MqttError::Unsupported));
        // PUBACK, PUBREC, ... and reserved types
        for kind in [4u8, 5, 6, 7, 15, 0] {
            assert_eq!(decode(&[kind << 4, 0]), Err(MqttError::Unsupported));
        }
    }

    #[test]
    fn lengths_are_checked() {
        // remaining length beyond 4 bytes
        assert_eq!(
            decode(&[PINGREQ << 4, 0x80, 0x80, 0x80, 0x80, 1]),
            Err(MqttError::BadLength)
        );
        // larger than MAX_PACKET, refused before the body arrives
        assert_eq!(
            decode(&[PUBLISH << 4, 0xff, 0x7f]),
            Err(MqttError::TooLarge)
        );
        // a string longer than its packet
        assert_eq!(
            decode(&[PUBLISH << 4, 3, 0, 9, b'a']),
            Err(MqttError::Malformed)
        );
        // PINGREQ with a body
        assert_eq!(decode(&[PINGREQ << 4, 1, 0]), Err(MqttError::Malformed));
        // SUBSCRIBE with no filters, packet id 0, or a QoS byte of 3
        assert_eq!(
            decode(&[(SUBSCRIBE << 4) | 2, 2, 0, 1]),
            Err(MqttError::Malformed)
        );
        assert_eq!(
            decode(&[(SUBSCRIBE << 4) | 2, 6, 0, 0, 0, 1, b'a', 0]),
            Err(MqttError::Malformed)
        );
        assert_eq!(
            decode(&[(SUBSCRIBE << 4) | 2, 6, 0, 1, 0, 1, b'a', 3]),
            Err(MqttError::Malformed)
        );
        // invalid UTF-8 topic
        assert_eq!(
            decode(&[PUBLISH << 4, 4, 0, 2, 0xc3, 0x28]),
            Err(MqttError::Malformed)
        );
    }

    #[test]
    fn topics_and_filters() {
        assert!(valid_topic("rm/v1/ev/w3gv2c"));
        assert!(!valid_topic("rm/+/x") && !valid_topic("rm/#") && !valid_topic(""));
        assert!(valid_filter("rm/v1/ev/+") && valid_filter("rm/#") && valid_filter("#"));
        assert!(!valid_filter("rm/#/x") && !valid_filter("rm/a+") && !valid_filter("rm/#a"));
        assert!(matches("rm/v1/ev/+", "rm/v1/ev/w3gv2c"));
        assert!(matches("rm/#", "rm/v1/ev/w3gv2c"));
        assert!(matches("rm/v1/ev/w3gv2c", "rm/v1/ev/w3gv2c"));
        assert!(!matches("rm/v1/ev/+", "rm/v1/ev/w3gv2c/x"));
        assert!(!matches("rm/v1/ev/+", "rm/v1/ev"));
        assert!(!matches("rm/v1/ev/w3gv2d", "rm/v1/ev/w3gv2c"));
    }
}
