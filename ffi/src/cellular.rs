//! The phone's side of the cellular bridge (SPEC §5, BUILD_PLAN Step 9): the MQTT
//! session to the edge server. The platform owns the TLS 1.3 socket (with certificate
//! pinning and the hardware device key); this object turns intentions into bytes to
//! send and the bytes received into events. Every byte from the network is parsed
//! here, in Rust (CLAUDE.md security rules).
//!
//! Only hazard events are uploaded: the server drops position heartbeats and keeps no
//! positions, so sending them would only cost data (about 7 MB an hour).

use std::sync::{Arc, Mutex, MutexGuard};

use roadmesh_core::geohash;
use roadmesh_core::mqtt::{self, MAX_PACKET, Packet, topic};
use roadmesh_core::pki;
use roadmesh_core::rsm::{Frame, MsgType, Rsm};

use crate::{RoadMeshError, bad};

/// Something the server sent.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum LinkEvent {
    /// The session is open (CONNACK accepted).
    Connected,
    /// The server refused the session (CONNACK code).
    Refused {
        /// MQTT return code.
        code: u8,
    },
    /// A hazard event from another phone: give it to `SafetyEngine::receive`, which
    /// checks it like any radio frame.
    HazardEvent {
        /// The radio frame.
        frame: Vec<u8>,
    },
    /// The signed revocation list: give it to `SafetyEngine::load_revocation_list`.
    RevocationList {
        /// List bytes.
        list: Vec<u8>,
    },
    /// Answer to a misbehaviour report (0 counted, 1 revoked, 2 already, ≥0x80 refused).
    ReportResult {
        /// Result code.
        code: u8,
    },
    /// Answer to "delete my data" (0 done).
    DeleteResult {
        /// Result code.
        code: u8,
    },
    /// Answer to a certificate request: parse with [`parse_enrol_reply`].
    EnrolReply {
        /// Reply bytes.
        reply: Vec<u8>,
    },
    /// Answer to a ping.
    Pong,
    /// Protocol error: close the socket and reconnect later.
    Close,
}

#[derive(Default)]
struct Inner {
    buf: Vec<u8>,
    cells: Vec<String>,
    crl_subscribed: bool,
    packet_id: u16,
}

impl Inner {
    fn next_id(&mut self) -> u16 {
        self.packet_id = self.packet_id.wrapping_add(1).max(1);
        self.packet_id
    }
}

/// One MQTT session to the edge server.
#[derive(uniffi::Object)]
pub struct CellularLink {
    inner: Mutex<Inner>,
}

fn publish(topic: &str, payload: Vec<u8>) -> Vec<u8> {
    mqtt::encode(&Packet::Publish {
        topic: topic.into(),
        payload,
    })
}

impl CellularLink {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[uniffi::export]
impl CellularLink {
    /// A link with no session yet.
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner::default()),
        })
    }

    /// Start a new session on a fresh socket: forget buffered bytes and cells, and
    /// return the CONNECT packet to send.
    pub fn connect(&self, keep_alive_s: u16) -> Vec<u8> {
        *self.lock() = Inner::default();
        mqtt::encode(&Packet::Connect {
            client_id: String::new(),
            keep_alive: keep_alive_s,
        })
    }

    /// Bytes read from the socket → events. After `Close`, drop the socket.
    pub fn on_bytes(&self, data: Vec<u8>) -> Vec<LinkEvent> {
        let mut g = self.lock();
        g.buf.extend_from_slice(&data);
        let mut out = Vec::new();
        loop {
            match mqtt::decode(&g.buf) {
                Ok(Some((p, used))) => {
                    g.buf.drain(..used);
                    let ev = match p {
                        Packet::ConnAck { code: 0 } => Some(LinkEvent::Connected),
                        Packet::ConnAck { code } => Some(LinkEvent::Refused { code }),
                        Packet::PingResp => Some(LinkEvent::Pong),
                        Packet::SubAck { .. } | Packet::UnsubAck { .. } => None,
                        Packet::Publish { topic: t, payload } => {
                            let first = payload.first().copied().unwrap_or(0x80);
                            match t.as_str() {
                                topic::CRL => Some(LinkEvent::RevocationList { list: payload }),
                                topic::REPORT_REPLY => {
                                    Some(LinkEvent::ReportResult { code: first })
                                }
                                topic::DELETE_REPLY => {
                                    Some(LinkEvent::DeleteResult { code: first })
                                }
                                topic::ENROL_REPLY => {
                                    Some(LinkEvent::EnrolReply { reply: payload })
                                }
                                t if t.starts_with(topic::EVENTS) => {
                                    Some(LinkEvent::HazardEvent { frame: payload })
                                }
                                _ => Some(LinkEvent::Close),
                            }
                        }
                        // a server never sends these
                        _ => Some(LinkEvent::Close),
                    };
                    if let Some(e) = ev {
                        let close = e == LinkEvent::Close;
                        out.push(e);
                        if close {
                            g.buf.clear();
                            return out;
                        }
                    }
                }
                Ok(None) if g.buf.len() < MAX_PACKET => return out,
                _ => {
                    g.buf.clear();
                    out.push(LinkEvent::Close);
                    return out;
                }
            }
        }
    }

    /// The map cells around (lat, lon): returns SUBSCRIBE/UNSUBSCRIBE packets for the
    /// change (and the revocation list on the first call), or nothing if unchanged.
    /// Call about once a second with the phone's position; it is never sent as such.
    pub fn follow_position(&self, lat: f64, lon: f64) -> Vec<u8> {
        let mut g = self.lock();
        let want = geohash::neighbourhood(lat, lon);
        if want.is_empty() {
            return Vec::new();
        }
        let add: Vec<String> = want
            .iter()
            .filter(|c| !g.cells.contains(c))
            .map(|c| format!("{}{c}", topic::EVENTS))
            .collect();
        let drop: Vec<String> = g
            .cells
            .iter()
            .filter(|c| !want.contains(c))
            .map(|c| format!("{}{c}", topic::EVENTS))
            .collect();
        let mut out = Vec::new();
        if !drop.is_empty() {
            let packet_id = g.next_id();
            out.extend(mqtt::encode(&Packet::Unsubscribe {
                packet_id,
                filters: drop,
            }));
        }
        let mut add = add;
        if !g.crl_subscribed {
            add.push(topic::CRL.into());
            g.crl_subscribed = true;
        }
        if !add.is_empty() {
            let packet_id = g.next_id();
            out.extend(mqtt::encode(&Packet::Subscribe {
                packet_id,
                filters: add,
            }));
        }
        g.cells = want;
        out
    }

    /// A frame this phone sent or heard: the PUBLISH to upload it if it is a hazard
    /// event, otherwise nothing.
    pub fn upload(&self, frame: Vec<u8>) -> Vec<u8> {
        let is_event = Frame::decode(&frame)
            .ok()
            .and_then(|f| Rsm::decode(&f.body).ok())
            .is_some_and(|r| r.msg_type == MsgType::Event);
        if is_event {
            publish(topic::UP, frame)
        } else {
            Vec::new()
        }
    }

    /// A signed misbehaviour report (from `SafetyEngine::take_reports`).
    pub fn report(&self, report: Vec<u8>) -> Vec<u8> {
        publish(topic::REPORT, report)
    }

    /// "Delete my data" on the server.
    pub fn delete_my_data(&self) -> Vec<u8> {
        publish(topic::DELETE, Vec::new())
    }

    /// A certificate request (enrolment port; from `encode_issue_request`).
    pub fn enrol(&self, request: Vec<u8>) -> Vec<u8> {
        publish(topic::ENROL, request)
    }

    /// Keep-alive ping.
    pub fn ping(&self) -> Vec<u8> {
        mqtt::encode(&Packet::PingReq)
    }
}

/// A successful enrolment.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Enrolled {
    /// X.509 device certificate (DER) for mutual TLS with the device key.
    pub device_certificate: Vec<u8>,
    /// Pseudonym certificates (107 bytes each) for `SafetyEngine::add_certificates`.
    pub certificates: Vec<u8>,
}

/// Parse the server's answer to a certificate request. Errors carry the server's code
/// (0x81 banned, 0x82 not a genuine app/device, 0x83 weekly quota, 0x84 try later).
#[uniffi::export]
pub fn parse_enrol_reply(reply: Vec<u8>) -> Result<Enrolled, RoadMeshError> {
    let (&code, rest) = reply.split_first().ok_or_else(|| bad("empty reply"))?;
    if code != 0 {
        return Err(RoadMeshError::Rejected {
            detail: format!("server code {code:#04x}"),
        });
    }
    let n = rest
        .get(..2)
        .map(|b| usize::from(u16::from_be_bytes([b[0], b[1]])))
        .ok_or_else(|| bad("short reply"))?;
    let cert = rest.get(2..2 + n).ok_or_else(|| bad("short reply"))?;
    let certs = rest.get(2 + n..).unwrap_or_default();
    pki::decode_certificates(certs).map_err(|_| bad("certificates"))?;
    Ok(Enrolled {
        device_certificate: cert.to_vec(),
        certificates: certs.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_from_a_byte_stream_split_anywhere() {
        let link = CellularLink::new();
        let mut stream = mqtt::encode(&Packet::ConnAck { code: 0 });
        stream.extend(publish("rm/v1/ev/w649ex", vec![1, 2, 3]));
        stream.extend(publish(topic::CRL, vec![9]));
        stream.extend(publish(topic::REPORT_REPLY, vec![1]));
        stream.extend(mqtt::encode(&Packet::PingResp));
        for split in 0..stream.len() {
            let mut ev = link.on_bytes(stream[..split].to_vec());
            ev.extend(link.on_bytes(stream[split..].to_vec()));
            assert_eq!(
                ev,
                vec![
                    LinkEvent::Connected,
                    LinkEvent::HazardEvent {
                        frame: vec![1, 2, 3]
                    },
                    LinkEvent::RevocationList { list: vec![9] },
                    LinkEvent::ReportResult { code: 1 },
                    LinkEvent::Pong,
                ]
            );
        }
    }

    #[test]
    fn garbage_or_unknown_topics_close() {
        let link = CellularLink::new();
        assert_eq!(link.on_bytes(vec![0x32, 1, 0]), vec![LinkEvent::Close]);
        assert_eq!(
            link.on_bytes(publish("other/topic", vec![1])),
            vec![LinkEvent::Close]
        );
        assert_eq!(
            link.on_bytes(mqtt::encode(&Packet::Connect {
                client_id: String::new(),
                keep_alive: 1
            })),
            vec![LinkEvent::Close]
        );
        // an oversized packet is refused before it arrives in full
        assert_eq!(
            link.on_bytes(vec![0x30, 0xff, 0x7f]),
            vec![LinkEvent::Close]
        );
    }

    #[test]
    fn following_the_position_subscribes_only_to_changes() {
        let link = CellularLink::new();
        let first = link.follow_position(11.5564, 104.9282);
        let Ok(Some((Packet::Subscribe { filters, .. }, n))) = mqtt::decode(&first) else {
            panic!("no subscribe")
        };
        assert_eq!((filters.len(), n), (10, first.len())); // 9 cells + revocation list
        assert!(link.follow_position(11.5564, 104.9282).is_empty());
        // one cell north: 3 cells dropped, 3 added
        let moved = link.follow_position(11.5564 + 0.0055, 104.9282);
        let Ok(Some((Packet::Unsubscribe { filters: gone, .. }, n))) = mqtt::decode(&moved) else {
            panic!("no unsubscribe")
        };
        let Ok(Some((Packet::Subscribe { filters: new, .. }, _))) = mqtt::decode(&moved[n..])
        else {
            panic!("no subscribe")
        };
        assert_eq!((gone.len(), new.len()), (3, 3));
        // a new connection starts over
        link.connect(60);
        assert!(!link.follow_position(11.5564, 104.9282).is_empty());
    }

    #[test]
    fn enrol_reply_parsing() {
        assert!(matches!(
            parse_enrol_reply(vec![0x82]),
            Err(RoadMeshError::Rejected { .. })
        ));
        assert!(parse_enrol_reply(vec![]).is_err());
        assert!(parse_enrol_reply(vec![0, 0, 9, 1]).is_err());
        assert_eq!(
            parse_enrol_reply(vec![0, 0, 2, 7, 7]).ok(),
            Some(Enrolled {
                device_certificate: vec![7, 7],
                certificates: vec![]
            })
        );
    }
}
