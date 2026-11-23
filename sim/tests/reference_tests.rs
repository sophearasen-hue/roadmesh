//! Ports of the reference tests that need the simulator or the PKI
//! (`reference/core-python/tests/test_v2v.py`). The other reference tests live
//! next to the code they test in `core/` (geometry, prediction, LDM, behaviour).
//!
//! Since Step 3 the security tests run the real ECDSA P-256 certificates,
//! verifier and CA logic; only the key storage is simulated (`roadmesh_sim::keys`).

#![allow(clippy::panic)] // test code: a failed precondition should fail the test loudly

use roadmesh_core::auth::{AuthError, Signer, Verifier};
use roadmesh_core::behavior::Subject;
use roadmesh_core::config::Config;
use roadmesh_core::geo::GeoOrigin;
use roadmesh_core::obu::{DriveState, Obu, OwnState, RxOutcome};
use roadmesh_core::pki::{CertVerifier, MisbehaviourReport, PseudonymSigner, ReportReason};
use roadmesh_core::prediction::Motion;
use roadmesh_core::rsm::{self, EventCode, Frame, MsgType, Rsm, TempId, VehicleType};
use roadmesh_server::ca::ReportOutcome;
use roadmesh_sim::keys::{self, SimCa, SoftKeys};
use roadmesh_sim::rng::SimRng;
use roadmesh_sim::scenarios;
use roadmesh_sim::world::{Options, World};

struct Pki {
    ca: SimCa,
    rng: SimRng,
}

impl Pki {
    fn new() -> Self {
        let mut rng = SimRng::new(1);
        let ca = keys::new_ca(&mut rng, Config::reference());
        Self { ca, rng }
    }
    fn phone(&mut self, certs: usize) -> PseudonymSigner<SoftKeys> {
        keys::enrol(&mut self.ca, &mut self.rng, certs, 0)
    }
    fn verifier(&self) -> CertVerifier {
        CertVerifier::new(&[self.ca.trust_anchor()], &Config::reference().security)
            .unwrap_or_else(|_| panic!("anchor"))
    }
}

fn id(s: &mut PseudonymSigner<SoftKeys>, t: f64) -> TempId {
    s.current_temp_id(t)
        .unwrap_or_else(|| panic!("no certificate"))
}

fn body(temp_id: [u8; 4], speed: f64) -> [u8; rsm::BODY_LEN] {
    body_at(temp_id, speed, 1.0, 1, -40.0)
}

fn body_at(temp_id: [u8; 4], speed: f64, t: f64, msg_id: u16, x: f64) -> [u8; rsm::BODY_LEN] {
    let (lat, lon) = GeoOrigin::phnom_penh().to_latlon(x, -1.5);
    Rsm {
        msg_type: MsgType::Heartbeat,
        temp_id,
        msg_id,
        timestamp_ms: rsm::ms_of_hour(t),
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

fn sign(s: &mut PseudonymSigner<SoftKeys>, t: f64, b: &[u8; rsm::BODY_LEN]) -> Vec<u8> {
    s.sign(t, false, b).unwrap_or_else(|| panic!("cannot sign"))
}

// ---------------------------------------------------------------- SecurityTests

#[test]
fn valid_message_accepted() {
    let mut pki = Pki::new();
    let mut phone = pki.phone(1);
    let b = body(id(&mut phone, 1.0), 10.0);
    let auth = sign(&mut phone, 1.0, &b);
    assert_eq!(pki.verifier().verify(1.0, &b, &auth), Ok(()));
}

#[test]
fn tampered_payload_rejected() {
    let mut pki = Pki::new();
    let mut phone = pki.phone(1);
    let tid = id(&mut phone, 1.0);
    let auth = sign(&mut phone, 1.0, &body(tid, 10.0));
    assert_eq!(
        pki.verifier().verify(1.0, &body(tid, 30.0), &auth),
        Err(AuthError::BadSignature)
    );
}

#[test]
fn unknown_certificate_rejected() {
    // forged: a certificate from a CA key the app does not trust
    let mut pki = Pki::new();
    let mut forger = keys::rogue_signer(&mut pki.rng, 1, 0);
    let b = body(id(&mut forger, 1.0), 10.0);
    let auth = sign(&mut forger, 1.0, &b);
    assert_eq!(
        pki.verifier().verify(1.0, &b, &auth),
        Err(AuthError::BadCertificate)
    );
    // garbage auth bytes
    assert_eq!(
        pki.verifier().verify(1.0, &b, &[0; 32]),
        Err(AuthError::MalformedAuth)
    );
}

#[test]
fn temp_id_must_match_certificate() {
    // Signing with my key while claiming someone else's pseudonym fails verification.
    // (The reference reports BadSignature; the certificate binding catches it first.)
    let mut pki = Pki::new();
    let mut mine = pki.phone(1);
    let mut victim = pki.phone(1);
    let my_auth = {
        let b = body(id(&mut mine, 1.0), 10.0);
        sign(&mut mine, 1.0, &b)
    };
    let claimed = body(id(&mut victim, 1.0), 10.0);
    assert_eq!(
        pki.verifier().verify(1.0, &claimed, &my_auth),
        Err(AuthError::TempIdMismatch)
    );
    assert!(mine.sign(1.0, false, &claimed).is_none()); // my signer refuses
}

/// Three independent phones report `evil` with signed evidence; returns the last outcome.
fn report_three_times(pki: &mut Pki, evil: &mut PseudonymSigner<SoftKeys>) -> Vec<ReportOutcome> {
    let tid = id(evil, 1.0);
    let b = body(tid, 95.0); // 342 km/h
    let proof = Frame {
        last_hop: None,
        body: b,
        auth: sign(evil, 1.0, &b),
    };
    let mut out = Vec::new();
    for _ in 0..3 {
        let mut reporter = pki.phone(1);
        let r = MisbehaviourReport::build(
            &mut reporter,
            1.0,
            ReportReason::ImplausibleSpeed,
            vec![proof.clone()],
        )
        .and_then(|r| r.encode())
        .unwrap_or_else(|| panic!("report"));
        out.push(pki.ca.report(&r, 1).unwrap_or_else(|e| panic!("{e:?}")));
    }
    out
}

#[test]
fn revocation() {
    let mut pki = Pki::new();
    let mut evil = pki.phone(1);
    let tid = id(&mut evil, 1.0);
    let ok = body(tid, 10.0);
    let auth = sign(&mut evil, 1.0, &ok);
    let mut v = pki.verifier();
    assert_eq!(v.verify(1.0, &ok, &auth), Ok(()));
    report_three_times(&mut pki, &mut evil);
    let crl = pki
        .ca
        .revocation_list(2, 86_400)
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(v.load_revocation_list(&crl).is_ok());
    assert_eq!(
        v.verify(1.0, &ok, &auth),
        Err(AuthError::RevokedCertificate)
    );
}

#[test]
fn pseudonym_rotation() {
    let mut pki = Pki::new();
    let mut s = pki.phone(3);
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
    let mut pki = Pki::new();
    let mut phone = pki.phone(1);
    let tid = id(&mut phone, 1.0);
    let signed = body(tid, 10.0);
    let auth = sign(&mut phone, 1.0, &signed);
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
    let out = obu.receive(&frame, 1.0, &own_at(-80.0), &mut pki.verifier());
    assert!(matches!(
        out,
        RxOutcome::AuthFailed {
            reason: AuthError::BadSignature,
            ..
        }
    ));
    assert!(obu.ldm().get(&tid).is_none());
    assert!(obu.pending_relays().is_empty()); // and it is not forwarded
}

#[test]
fn revocation_needs_independent_reporters() {
    let mut pki = Pki::new();
    let mut evil = pki.phone(1);
    let tid = id(&mut evil, 1.0);
    let b = body(tid, 95.0);
    let proof = Frame {
        last_hop: None,
        body: b,
        auth: sign(&mut evil, 1.0, &b),
    };
    let file = |reporter: &mut PseudonymSigner<SoftKeys>, ca: &mut SimCa, t: f64| {
        let r = MisbehaviourReport::build(
            reporter,
            t,
            ReportReason::ImplausibleSpeed,
            vec![proof.clone()],
        )
        .and_then(|r| r.encode())
        .unwrap_or_else(|| panic!("report"));
        ca.report(&r, 1)
    };
    let mut car_a = pki.phone(3);
    let mut car_b = pki.phone(1);
    let mut car_c = pki.phone(1);
    assert_eq!(
        file(&mut car_a, &mut pki.ca, 1.0),
        Ok(ReportOutcome::Counted(1))
    );
    // same phone again, even under another pseudonym: still one reporter
    assert_eq!(
        file(&mut car_a, &mut pki.ca, 301.0),
        Ok(ReportOutcome::Counted(1))
    );
    assert_eq!(
        file(&mut car_b, &mut pki.ca, 1.0),
        Ok(ReportOutcome::Counted(2))
    );
    assert_eq!(
        file(&mut car_c, &mut pki.ca, 1.0),
        Ok(ReportOutcome::Revoked)
    ); // third independent reporter
    let d = evil.certificates()[0].digest();
    assert!(pki.ca.is_revoked(&d));
}

#[test]
fn message_flood_is_rejected() {
    let mut pki = Pki::new();
    let mut phone = pki.phone(1);
    let mut verifier = pki.verifier();
    let mut obu = Obu::new(Config::reference(), GeoOrigin::phnom_penh(), false);
    let me = own_at(-80.0);
    let tid = id(&mut phone, 1.0);
    for i in 0..5u16 {
        // 5 messages 10 ms apart = 100 msg/s
        let t = 1.0 + f64::from(i) * 0.01;
        let b = body_at(tid, 10.0, t, i + 1, -40.0 + f64::from(i) * 0.1);
        let auth = sign(&mut phone, t, &b);
        obu.receive(
            &Frame {
                last_hop: None,
                body: b,
                auth,
            },
            t,
            &me,
            &mut verifier,
        );
    }
    assert!(obu.blocked().contains(&tid));
    assert!(obu.ldm().get(&tid).is_none());
    // the phone kept signed proof for its report, and the CA can confirm it
    let evidence = obu.take_evidence(&tid);
    assert_eq!(evidence.len(), 2);
    let mut reporter = pki.phone(1);
    let r = MisbehaviourReport::build(&mut reporter, 1.05, ReportReason::RateTooHigh, evidence)
        .and_then(|r| r.encode())
        .unwrap_or_else(|| panic!("report"));
    assert_eq!(pki.ca.report(&r, 1), Ok(ReportOutcome::Counted(1)));
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
