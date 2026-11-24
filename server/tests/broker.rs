//! Edge broker over in-memory streams (TLS is tested in tests/tls.rs): the cellular
//! bridge carries verified hazard events to phones out of radio range (BUILD_PLAN
//! Step 9 "done when"), and nothing else gets through.
#![allow(clippy::panic, clippy::unwrap_used)]

mod common;

use common::*;
use roadmesh_core::geohash;
use roadmesh_core::mqtt::Packet;
use roadmesh_core::rsm::Frame;
use roadmesh_server::broker::{self, Peer, Settings, code};
use roadmesh_server::devca::device_key_of;
use roadmesh_server::monitor::Metric;

const PP: (f64, f64) = (11.5564, 104.9282); // Phnom Penh

fn ev_topic(lat: f64, lon: f64) -> String {
    format!(
        "{}{}",
        broker::EVENTS,
        geohash::encode(lat, lon, geohash::LEVEL).unwrap()
    )
}

#[tokio::test]
async fn enrolment_gives_a_device_certificate_and_20_pseudonyms() {
    let (b, clock) = broker(Settings::default());
    let mut p = Phone::new();
    enrol(&b, &clock, &mut p, 1).await;
    assert_eq!(device_key_of(&p.device_cert), Some(pubkey(&p.device)));
    assert_eq!(p.signer.as_ref().unwrap().certificates().len(), 20);
    assert_eq!(b.monitor.get(Metric::EnrolOk), 1);
    // without genuine-app evidence: refused
    let mut fake = Phone::new();
    let mut c = connect(&b, Peer::Enrol([10, 0, 0, 2].into())).await;
    c.publish(
        broker::ENROL,
        fake.request(20, now(&clock) as u32, b"emulator"),
    )
    .await;
    let (_, reply) = c.next_publish().await.unwrap();
    assert_eq!(reply, vec![code::NOT_GENUINE]);
}

#[tokio::test]
async fn enrolment_is_rate_limited_per_address() {
    let (b, clock) = broker(Settings::default());
    let mut c = connect(&b, Peer::Enrol([10, 0, 0, 9].into())).await;
    let mut codes = Vec::new();
    for _ in 0..7 {
        let mut p = Phone::new();
        c.publish(broker::ENROL, p.request(1, now(&clock) as u32, b"genuine"))
            .await;
        codes.push(c.next_publish().await.unwrap().1[0]);
    }
    assert_eq!(codes, vec![0, 0, 0, 0, 0, code::LATER, code::LATER]);
}

#[tokio::test]
async fn hazard_events_reach_phones_out_of_radio_range_and_nothing_else_does() {
    let (b, clock) = broker(Settings::default());
    let (mut a, mut z) = (Phone::new(), Phone::new());
    enrol(&b, &clock, &mut a, 1).await;
    enrol(&b, &clock, &mut z, 2).await;
    let mut up = connect(&b, Peer::Device(a.id())).await;
    let mut down = connect(&b, Peer::Device(z.id())).await;
    // z is 450 m away (out of Bluetooth range) and subscribes to its neighbourhood
    let here = (PP.0 + 0.004, PP.1);
    let cells = geohash::neighbourhood(here.0, here.1);
    down.send(&Packet::Subscribe {
        packet_id: 1,
        filters: cells
            .iter()
            .map(|c| format!("{}{c}", broker::EVENTS))
            .collect(),
    })
    .await;
    assert_eq!(
        down.recv().await,
        Some(Packet::SubAck {
            packet_id: 1,
            codes: vec![0; 9]
        })
    );
    let t = now(&clock);
    // a heartbeat is checked and dropped; an event is pushed to its cell
    let hb = a.frame(t, 1, false, PP.0, PP.1);
    up.publish(broker::UP, hb.encode().unwrap()).await;
    let ev = a.frame(t, 2, true, PP.0, PP.1);
    assert!(cells.contains(&geohash::encode(PP.0, PP.1, 6).unwrap()));
    up.publish(broker::UP, ev.encode().unwrap()).await;
    let (topic, payload) = down.next_publish().await.unwrap();
    assert_eq!(topic, ev_topic(PP.0, PP.1));
    let got = Frame::decode(&payload).unwrap();
    assert_eq!((got.body, got.auth.clone()), (ev.body, ev.auth.clone()));
    // the same event uploaded again (by another phone that heard it): not pushed twice
    up.publish(broker::UP, ev.encode().unwrap()).await;
    // a tampered event: refused
    let mut bad = a.frame(t, 3, true, PP.0, PP.1);
    bad.body[20] ^= 1;
    up.publish(broker::UP, bad.encode().unwrap()).await;
    // an event from far away (another cell): not pushed to z
    let far = a.frame(t, 4, true, PP.0 + 0.5, PP.1);
    up.publish(broker::UP, far.encode().unwrap()).await;
    // a phone enrolled with another CA: refused
    let mut stranger = Phone::new();
    let other = broker(Settings::default());
    enrol(&other.0, &other.1, &mut stranger, 1).await;
    up.publish(
        broker::UP,
        stranger.frame(t, 1, true, PP.0, PP.1).encode().unwrap(),
    )
    .await;
    // a stale event (uploaded 30 s after it was signed; events live 10 s): refused
    let old = a.frame(t, 5, true, PP.0, PP.1);
    clock.fetch_add(30_000, std::sync::atomic::Ordering::Relaxed);
    up.publish(broker::UP, old.encode().unwrap()).await;
    assert_eq!(down.next_publish().await, None);
    assert_eq!(b.monitor.get(Metric::Uploads), 7);
    // duplicate, tampered, unknown certificate, stale
    assert_eq!(b.monitor.get(Metric::UploadsRejected), 4);
    assert_eq!(b.monitor.get(Metric::EventsPushed), 1);
}

#[tokio::test]
async fn subscriptions_cannot_watch_the_whole_city() {
    let (b, clock) = broker(Settings::default());
    let mut a = Phone::new();
    enrol(&b, &clock, &mut a, 1).await;
    let mut c = connect(&b, Peer::Device(a.id())).await;
    let mut filters = vec![
        format!("{}+", broker::EVENTS),
        "rm/#".to_string(),
        "#".to_string(),
        format!("{}w64", broker::EVENTS),
        broker::UP.to_string(),
    ];
    filters.extend(
        geohash::neighbourhood(PP.0, PP.1)
            .into_iter()
            .chain(geohash::neighbourhood(PP.0 + 0.1, PP.1))
            .map(|cell| format!("{}{cell}", broker::EVENTS)),
    );
    filters.truncate(16);
    c.send(&Packet::Subscribe {
        packet_id: 7,
        filters,
    })
    .await;
    let Some(Packet::SubAck { codes, .. }) = c.recv().await else {
        panic!("no suback")
    };
    // wildcards, partial cells and other topics refused; at most 9 cells granted
    assert_eq!(&codes[..5], &[0x80; 5]);
    assert_eq!(codes[5..].iter().filter(|c| **c == 0).count(), 9);
    // publishing to an event topic directly closes the session
    c.publish(&ev_topic(PP.0, PP.1), vec![1, 2, 3]).await;
    assert_eq!(c.recv().await, None);
}

#[tokio::test]
async fn enrolment_sessions_cannot_subscribe_or_upload() {
    let (b, _clock) = broker(Settings::default());
    let mut c = connect(&b, Peer::Enrol([10, 0, 0, 1].into())).await;
    c.publish(broker::UP, vec![0; 100]).await;
    assert_eq!(c.recv().await, None);
    let mut c = connect(&b, Peer::Enrol([10, 0, 0, 1].into())).await;
    c.send(&Packet::Subscribe {
        packet_id: 1,
        filters: vec![broker::CRL.into()],
    })
    .await;
    assert_eq!(c.recv().await, None);
}

#[tokio::test]
async fn floods_are_rate_limited() {
    let (b, clock) = broker(Settings {
        max_drops: 10,
        ..Settings::default()
    });
    let mut a = Phone::new();
    enrol(&b, &clock, &mut a, 1).await;
    let mut c = connect(&b, Peer::Device(a.id())).await;
    let f = a.frame(now(&clock), 1, false, PP.0, PP.1).encode().unwrap();
    for _ in 0..100 {
        c.publish(broker::UP, f.clone()).await;
    }
    // 50 burst + 10 tolerated drops, then the session is closed
    assert_eq!(c.recv().await, None);
    assert!(b.monitor.get(Metric::RateLimited) >= 10);
    assert!(b.monitor.get(Metric::Uploads) <= 51);
}

#[tokio::test]
async fn garbage_and_unexpected_packets_close_the_session() {
    let (b, clock) = broker(Settings::default());
    let mut a = Phone::new();
    enrol(&b, &clock, &mut a, 1).await;
    let mut c = connect(&b, Peer::Device(a.id())).await;
    use tokio::io::AsyncWriteExt;
    let _ = c.io.write_all(&[0x32, 3, 0, 1, b'a']).await; // QoS 1 publish
    assert_eq!(c.recv().await, None);
    let mut c = connect(&b, Peer::Device(a.id())).await;
    c.send(&Packet::ConnAck { code: 0 }).await;
    assert_eq!(c.recv().await, None);
    // and the broker keeps serving others
    let mut c = connect(&b, Peer::Device(a.id())).await;
    c.send(&Packet::PingReq).await;
    assert_eq!(c.recv().await, Some(Packet::PingResp));
}

#[tokio::test]
async fn delete_my_data_answers() {
    let (b, clock) = broker(Settings::default());
    let mut a = Phone::new();
    enrol(&b, &clock, &mut a, 1).await;
    let mut c = connect(&b, Peer::Device(a.id())).await;
    c.publish(broker::DELETE, Vec::new()).await;
    assert_eq!(
        c.next_publish().await,
        Some((broker::DELETE_REPLY.to_string(), vec![code::OK]))
    );
    assert_eq!(b.monitor.get(Metric::Deletes), 1);
    // the audit line names the device by its short reference only
    assert!(
        b.monitor
            .lines()
            .iter()
            .any(|l| l.contains("\"audit\":\"delete\""))
    );
}
