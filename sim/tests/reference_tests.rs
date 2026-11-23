//! Ports of the reference tests that need the simulator or the demo PKI
//! (`reference/core-python/tests/test_v2v.py`). The other reference tests live
//! next to the code they test in `core/` (geometry, prediction, LDM, behaviour).

#![allow(clippy::panic)] // test code: a failed precondition should fail the test loudly

use std::cell::RefCell;
use std::rc::Rc;

use roadmesh_core::auth::{AuthError, Signer, Verifier};
use roadmesh_core::behavior::Subject;
use roadmesh_core::config::Config;
use roadmesh_core::geo::GeoOrigin;
use roadmesh_core::obu::{DriveState, Obu, OwnState, RxOutcome};
use roadmesh_core::prediction::Motion;
use roadmesh_core::rsm::{self, EventCode, Frame, MsgType, Rsm, VehicleType};
use roadmesh_sim::pki::{Ca, DemoSigner, DemoVerifier};
use roadmesh_sim::scenarios;
use roadmesh_sim::world::{Options, World};

fn ca() -> Rc<RefCell<Ca>> {
    Rc::new(RefCell::new(Ca::new(1, 3)))
}

fn body(temp_id: [u8; 4], speed: f64) -> [u8; rsm::BODY_LEN] {
    let (lat, lon) = GeoOrigin::phnom_penh().to_latlon(-40.0, -1.5);
    Rsm {
        msg_type: MsgType::Heartbeat,
        temp_id,
        msg_id: 1,
        timestamp_ms: rsm::ms_of_hour(1.0),
        lat_deg: lat,
        lon_deg: lon,
        speed,
        heading_deg: 90.0,
        accel: 0.0,
        yaw_rate: 0.0,
        vehicle_type: VehicleType::Car,
        flags: 0,
        pos_accuracy_m: 1.0,
        hop_count: 0,
        ttl: 8,
        origin_pos_age_ms: 0,
        event: EventCode::None,
    }
    .encode()
}

// ---------------------------------------------------------------- SecurityTests

#[test]
fn valid_message_accepted() {
    let ca = ca();
    let cred = ca.borrow_mut().issue(1)[0];
    let mut signer = DemoSigner::new(vec![cred], 300.0);
    let b = body(cred.cert_id, 10.0);
    let auth = signer.sign(&cred.cert_id, &rsm::signing_bytes(&b));
    assert_eq!(
        DemoVerifier::new(ca, true).verify(&cred.cert_id, &rsm::signing_bytes(&b), &auth),
        Ok(())
    );
}

#[test]
fn tampered_payload_rejected() {
    let ca = ca();
    let cred = ca.borrow_mut().issue(1)[0];
    let mut signer = DemoSigner::new(vec![cred], 300.0);
    let auth = signer.sign(
        &cred.cert_id,
        &rsm::signing_bytes(&body(cred.cert_id, 10.0)),
    );
    let tampered = body(cred.cert_id, 30.0);
    assert_eq!(
        DemoVerifier::new(ca, true).verify(&cred.cert_id, &rsm::signing_bytes(&tampered), &auth),
        Err(AuthError::BadSignature)
    );
}

#[test]
fn unknown_certificate_rejected() {
    let b = body(*b"ffff", 10.0);
    assert_eq!(
        DemoVerifier::new(ca(), true).verify(b"ffff", &rsm::signing_bytes(&b), &[0; 32]),
        Err(AuthError::UnknownCertificate)
    );
}

#[test]
fn temp_id_must_match_certificate() {
    // Signing with my key while claiming someone else's pseudonym fails verification.
    let ca = ca();
    let creds = ca.borrow_mut().issue(2);
    let (mine, victim) = (creds[0], creds[1]);
    let claimed = body(victim.cert_id, 10.0);
    let auth =
        DemoSigner::new(vec![mine], 300.0).sign(&mine.cert_id, &rsm::signing_bytes(&claimed));
    assert_eq!(
        DemoVerifier::new(ca, true).verify(&victim.cert_id, &rsm::signing_bytes(&claimed), &auth),
        Err(AuthError::BadSignature)
    );
}

#[test]
fn revocation() {
    let ca = ca();
    let cred = ca.borrow_mut().issue(1)[0];
    for reporter in ["CarA", "CarB", "CarC"] {
        ca.borrow_mut().report(&cred.cert_id, reporter);
    }
    let b = body(cred.cert_id, 10.0);
    let auth = DemoSigner::new(vec![cred], 300.0).sign(&cred.cert_id, &rsm::signing_bytes(&b));
    assert_eq!(
        DemoVerifier::new(ca, true).verify(&cred.cert_id, &rsm::signing_bytes(&b), &auth),
        Err(AuthError::RevokedCertificate)
    );
}

#[test]
fn pseudonym_rotation() {
    let creds = ca().borrow_mut().issue(3);
    let mut s = DemoSigner::new(creds, 300.0);
    assert_ne!(s.current_temp_id(0.0), s.current_temp_id(301.0));
}

// ---------------------------------------------------------------- BehaviorTests

#[test]
fn mixed_traffic_report() {
    let mut w = scenarios::mixed_traffic(Options::default());
    let s = w.run();
    assert!(s.collisions.is_empty());
    let Some(i) = w.idx("CarA") else {
        panic!("no CarA")
    };
    let Some(obu) = &w.obus[i] else {
        panic!("CarA has no app")
    };
    let cfg = obu.config().clone();
    let prof = |name: &str| {
        obu.behavior()
            .profiles()
            .iter()
            .find(|(who, _)| match who {
                Subject::Me => name == "CarA",
                Subject::Peer(id) => w.owner(id) == name,
            })
            .map(|(_, p)| *p)
    };
    let (Some(speeder), Some(tailgater), Some(jerky), Some(car_a), Some(truck)) = (
        prof("Speeder"),
        prof("Tailgater"),
        prof("Jerky"),
        prof("CarA"),
        prof("Truck"),
    ) else {
        panic!("missing profiles")
    };
    assert!(speeder.over_limit_s > 20.0);
    assert_eq!(speeder.oncoming_overtakes, 3);
    assert!(tailgater.tailgate_s > 20.0);
    assert_eq!((jerky.harsh_brakes, jerky.harsh_accels), (1, 1));
    assert_eq!(car_a.score(&cfg), 100);
    assert_eq!(truck.score(&cfg), 100);
}

// ---------------------------------------------------------------- ScenarioTests

fn run(name: &str, opts: Options) -> roadmesh_sim::world::Summary {
    let Some(mut w) = scenarios::build(name, opts) else {
        panic!("no scenario {name}")
    };
    w.run()
}

#[test]
fn blind_intersection() {
    assert!(
        !run(
            "blind_intersection",
            Options {
                v2v: false,
                ..Options::default()
            }
        )
        .collisions
        .is_empty()
    );
    let s = run("blind_intersection", Options::default());
    assert!(s.collisions.is_empty());
    assert!(s.min_gap_m.is_some_and(|g| g > 8.0));
    assert_eq!(s.min_speed_kmh["CarB"], 50.0); // the car with priority never brakes
}

#[test]
fn blind_intersection_with_heavy_loss() {
    for seed in 1..=5 {
        assert!(
            run(
                "blind_intersection",
                Options {
                    loss: 0.6,
                    seed,
                    ..Options::default()
                }
            )
            .collisions
            .is_empty(),
            "seed {seed}"
        );
    }
}

#[test]
fn emergency_brake() {
    assert!(
        !run(
            "emergency_brake",
            Options {
                v2v: false,
                ..Options::default()
            }
        )
        .collisions
        .is_empty()
    );
    for loss in [0.0, 0.3, 0.6] {
        assert!(
            run(
                "emergency_brake",
                Options {
                    loss,
                    ..Options::default()
                }
            )
            .collisions
            .is_empty(),
            "loss {loss}"
        );
    }
}

#[test]
fn ghost_attack() {
    let mut w = scenarios::ghost_attack(Options::default());
    let s = w.run();
    assert_eq!(s.min_speed_kmh["CarA"], 50.0); // no phantom braking
    assert!(w.obus[0].as_ref().is_some_and(|o| !o.blocked().is_empty())); // insider blocked at once
    assert!(s.revoked.is_empty()); // one reporter alone cannot revoke
    let off = run(
        "ghost_attack",
        Options {
            security: false,
            ..Options::default()
        },
    );
    assert!(off.min_speed_kmh["CarA"] < 40.0); // the attack works without signatures
}

#[test]
fn relay_reaches_beyond_radio_range() {
    let mut w = scenarios::highway_relay(Options::default());
    while w.t < 1.0 {
        w.step();
    }
    let (Some(a), Some(stop)) = (w.idx("CarA"), w.idx("Stop3")) else {
        panic!()
    };
    assert!(w.vehicles[stop].x - w.vehicles[a].x > 1000.0); // far beyond 300 m radio range
    let Some(obu) = &w.obus[a] else { panic!() };
    let far: Vec<String> = obu
        .ldm()
        .entries()
        .iter()
        .filter(|e| e.hops >= 2)
        .map(|e| w.owner(&e.msg.temp_id))
        .collect();
    assert!(far.contains(&"Stop3".to_owned()), "{far:?}");
    assert_eq!(
        w.decisions[a].as_ref().map(|d| d.state),
        Some(DriveState::Advise)
    );
}

#[test]
fn relay_highway_outcomes() {
    assert!(
        !run(
            "highway_relay",
            Options {
                v2v: false,
                ..Options::default()
            }
        )
        .collisions
        .is_empty()
    );
    let direct = run(
        "highway_relay",
        Options {
            relay: false,
            ..Options::default()
        },
    );
    let relay = run("highway_relay", Options::default());
    assert!(direct.collisions.is_empty());
    assert!(relay.collisions.is_empty());
    assert!(relay.max_brake_ms2["CarA"] < direct.max_brake_ms2["CarA"] - 1.0);
}

fn own_at(x: f64) -> OwnState {
    OwnState {
        motion: Motion {
            x,
            y: -1.5,
            heading: 0.0,
            speed: 13.9,
            accel: 0.0,
            yaw_rate: 0.0,
        },
        vehicle_type: VehicleType::Car,
        pos_accuracy_m: 1.0,
    }
}

#[test]
fn relay_cannot_alter_message() {
    let ca = ca();
    let cred = ca.borrow_mut().issue(1)[0];
    let signed = body(cred.cert_id, 10.0);
    let auth = DemoSigner::new(vec![cred], 300.0).sign(&cred.cert_id, &rsm::signing_bytes(&signed));
    // a relay changes the speed but keeps the originator's signature
    let mut forged = Rsm::decode(&signed).unwrap_or_else(|_| panic!("decode"));
    forged.speed = 0.0;
    forged.hop_count = 3;
    let frame = Frame {
        last_hop: Some(GeoOrigin::phnom_penh().to_latlon(500.0, 0.0)),
        body: forged.encode(),
        auth,
    };
    let mut obu = Obu::new(Config::reference(), GeoOrigin::phnom_penh(), true);
    let out = obu.receive(&frame, 1.0, &own_at(-80.0), &DemoVerifier::new(ca, true));
    assert!(matches!(
        out,
        RxOutcome::AuthFailed {
            reason: AuthError::BadSignature,
            ..
        }
    ));
    assert!(obu.ldm().get(&cred.cert_id).is_none());
    assert!(obu.pending_relays().is_empty()); // and it is not forwarded
}

#[test]
fn revocation_needs_independent_reporters() {
    let ca = ca();
    let cid = ca.borrow_mut().issue(1)[0].cert_id;
    assert!(!ca.borrow_mut().report(&cid, "CarA"));
    assert!(!ca.borrow_mut().report(&cid, "CarA")); // same reporter twice: still no
    assert!(!ca.borrow_mut().report(&cid, "CarB"));
    assert!(ca.borrow_mut().report(&cid, "CarC")); // third independent reporter
    assert!(ca.borrow().revoked.contains(&cid));
}

#[test]
fn message_flood_is_rejected() {
    let ca = ca();
    let cred = ca.borrow_mut().issue(1)[0];
    let mut signer = DemoSigner::new(vec![cred], 300.0);
    let verifier = DemoVerifier::new(ca, true);
    let mut obu = Obu::new(Config::reference(), GeoOrigin::phnom_penh(), false);
    let me = own_at(-80.0);
    for i in 0..5u16 {
        // 5 messages 10 ms apart = 100 msg/s
        let t = 1.0 + f64::from(i) * 0.01;
        let (lat, lon) = GeoOrigin::phnom_penh().to_latlon(-40.0 + f64::from(i) * 0.1, -1.5);
        let mut r = Rsm::decode(&body(cred.cert_id, 10.0)).unwrap_or_else(|_| panic!("decode"));
        (r.msg_id, r.timestamp_ms, r.lat_deg, r.lon_deg) = (i + 1, rsm::ms_of_hour(t), lat, lon);
        let b = r.encode();
        let auth = signer.sign(&cred.cert_id, &rsm::signing_bytes(&b));
        obu.receive(
            &Frame {
                last_hop: None,
                body: b,
                auth,
            },
            t,
            &me,
            &verifier,
        );
    }
    assert!(obu.blocked().contains(&cred.cert_id));
    assert!(obu.ldm().get(&cred.cert_id).is_none());
}

#[test]
fn yield_failsafe_when_target_lost() {
    let mut w: World = scenarios::blind_intersection(Options::default());
    while w.t < 2.5 {
        w.step();
    }
    let Some(a) = w.idx("CarA") else { panic!() };
    assert_eq!(
        w.decisions[a].as_ref().map(|d| d.state),
        Some(DriveState::Yield)
    );
    w.set_loss(1.0); // radio blackout
    while w.t < 4.0 {
        w.step();
    }
    assert_eq!(
        w.decisions[a].as_ref().map(|d| d.state),
        Some(DriveState::Yield)
    ); // did not drive on blind
    assert!(w.vehicles[a].accel <= Config::reference().failsafe_brake);
}
