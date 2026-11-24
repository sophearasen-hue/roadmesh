//! Step 4 city simulation: regression checks (the full matrix is `roadmesh-sim report`).

use roadmesh_sim::city::{City, CityOptions};

fn small(seed: u64, security: bool) -> CityOptions {
    CityOptions {
        vehicles: 40,
        duration_s: 60.0,
        gps_m: (2.0, 4.0),
        seed,
        security,
        ..CityOptions::default()
    }
}

#[test]
fn city_runs_repeat_exactly() {
    let a = City::new(small(3, false)).run();
    let b = City::new(small(3, false)).run();
    assert_eq!(
        (
            a.conflicts,
            a.missed,
            a.alerts,
            a.false_alarms,
            a.collisions,
            a.frames
        ),
        (
            b.conflicts,
            b.missed,
            b.alerts,
            b.false_alarms,
            b.collisions,
            b.frames
        )
    );
}

#[test]
fn no_honest_vehicle_is_blocked_by_gps_noise() {
    // SPEC §10 jump rule: ordinary 2–10 m GPS error is not misbehaviour
    for seed in 1..=2 {
        let s = City::new(CityOptions {
            gps_m: (2.0, 10.0),
            ..small(seed, false)
        })
        .run();
        assert_eq!(s.honest_blocked, 0, "seed {seed}");
        assert!(
            s.conflicts > 0 && s.alerts > 0,
            "seed {seed}: traffic must interact"
        );
    }
}

#[test]
fn real_signatures_in_the_city() {
    // every phone verifies every message with ECDSA P-256; nobody honest is rejected
    let s = City::new(CityOptions {
        vehicles: 20,
        duration_s: 15.0,
        ..small(1, true)
    })
    .run();
    assert_eq!(s.honest_blocked, 0);
    assert!(s.frames[1] > 0);
}
