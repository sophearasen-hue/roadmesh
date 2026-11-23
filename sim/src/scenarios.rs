//! The five reference scenarios. Port of `reference/core-python/v2v/scenarios.py`.

use std::f64::consts::{FRAC_PI_2, PI};

use roadmesh_core::geometry::rect;
use roadmesh_core::rsm::VehicleType;

use crate::keys;
use crate::world::{GhostAttacker, Options, Role, Vehicle, World, Zones};

const KMH: f64 = 1.0 / 3.6;

/// Scenario names, in display order.
pub const NAMES: [&str; 5] = [
    "blind_intersection",
    "emergency_brake",
    "ghost_attack",
    "mixed_traffic",
    "highway_relay",
];

fn city_blocks() -> Vec<Vec<(f64, f64)>> {
    vec![
        rect(-120.0, -120.0, -6.0, -6.0),
        rect(6.0, -120.0, 120.0, -6.0),
        rect(-120.0, 6.0, -6.0, 120.0),
        rect(6.0, 6.0, 120.0, 120.0),
    ]
}

fn city() -> Zones {
    Zones(vec![(-1e6, 1e6, 50.0, "city street")])
}

/// Build a scenario by name.
pub fn build(name: &str, opts: Options) -> Option<World> {
    Some(match name {
        "blind_intersection" => blind_intersection(opts),
        "emergency_brake" => emergency_brake(opts),
        "ghost_attack" => ghost_attack(opts),
        "mixed_traffic" => mixed_traffic(opts),
        "highway_relay" => highway_relay(opts),
        _ => return None,
    })
}

/// Car A drives east, Car B north; a building hides them until the last moment.
pub fn blind_intersection(opts: Options) -> World {
    let a = Vehicle::new("CarA", -80.0, -1.5, 0.0, 50.0 * KMH);
    let b = Vehicle::new("CarB", 1.5, -80.0, FRAC_PI_2, 50.0 * KMH);
    let mut w = World::new(vec![a, b], city_blocks(), opts, 12.0);
    w.zones = city();
    w.watch_pair("CarA", "CarB");
    w
}

/// Three cars at 90 km/h, 20 m apart. Car1 brakes hard; Car3 can't see past Car2.
pub fn emergency_brake(opts: Options) -> World {
    let v = 90.0 * KMH;
    let cars = vec![
        Vehicle::new("Car1", 60.0, -1.75, 0.0, v).maneuver(1.5, f64::INFINITY, -8.0),
        Vehicle::new("Car2", 35.5, -1.75, 0.0, v),
        Vehicle::new("Car3", 11.0, -1.75, 0.0, v),
    ];
    let mut w = World::new(cars, Vec::new(), opts, 8.0);
    w.zones = Zones(vec![(-1e6, 1e6, 100.0, "national road")]);
    w.watch_pair("Car3", "Car1");
    w
}

/// Empty intersection; two roadside attackers broadcast fake cars at Car A.
/// The outsider forges signatures; the insider holds a real (stolen) certificate
/// and sends an impossible 342 km/h car, gets blocked by Car A at once and reported.
pub fn ghost_attack(opts: Options) -> World {
    let a = Vehicle::new("CarA", -80.0, -1.5, 0.0, 50.0 * KMH);
    let mut w = World::new(vec![a], city_blocks(), opts, 12.0);
    w.zones = city();
    // a stolen genuine phone (valid certificate) and an outsider with forged certificates
    let insider = keys::enrol(&mut w.ca, &mut w.rng, 1, 0);
    let forged = keys::rogue_signer(&mut w.rng, 1, 0);
    let attackers = vec![
        GhostAttacker::new(
            "Forger",
            (14.0, -20.0),
            (1.5, -80.0),
            FRAC_PI_2,
            50.0 * KMH,
            0.0,
            9.0,
            forged,
        ),
        GhostAttacker::new(
            "Insider",
            (-20.0, 14.0),
            (-1.5, 400.0),
            -FRAC_PI_2,
            95.0,
            2.0,
            4.5,
            insider,
        ),
    ];
    for at in &attackers {
        for c in at.signer.certificates() {
            let owner = format!("ghost from {}", at.name);
            w.cert_owner.insert(c.temp_id(), owner.clone());
            w.digest_owner.insert(c.digest(), owner);
        }
    }
    w.attackers = attackers;
    w
}

/// Fog, 130 km/h, traffic stopped 1.5 km ahead; oncoming app users relay the warning.
pub fn highway_relay(opts: Options) -> World {
    let mut cars = vec![Vehicle::new("CarA", 0.0, -1.75, 0.0, 130.0 * KMH)];
    for (i, x) in [1500.0, 1492.0, 1484.0].into_iter().enumerate() {
        cars.push(
            Vehicle::new(&format!("Stop{}", i + 1), x, -1.75, 0.0, 0.0)
                .wants(0.0)
                .role(Role::Hazard),
        );
    }
    for i in 0..11 {
        cars.push(
            Vehicle::new(
                &format!("Relay{}", i + 1),
                250.0 + 250.0 * f64::from(i),
                1.75,
                PI,
                100.0 * KMH,
            )
            .role(Role::Relay),
        );
    }
    let mut w = World::new(cars, Vec::new(), opts, 64.0);
    w.visibility = 110.0;
    w.record_every = 5;
    w.zones = Zones(vec![(-1e6, 1e6, 130.0, "expressway")]);
    w.watch_pair("CarA", "Stop3");
    w
}

/// Rural road: village 40, national road 80, town 40 km/h. Everyone records everyone.
pub fn mixed_traffic(opts: Options) -> World {
    let cars = vec![
        Vehicle::new("Jerky", 330.0, -1.75, 0.0, 40.0 * KMH)
            .wants(80.0 * KMH)
            .headway(2.0)
            .maneuver(14.0, 15.6, -6.0)
            .maneuver(19.0, 22.0, 3.5),
        Vehicle::new("Truck", 150.0, -1.75, 0.0, 40.0 * KMH)
            .wants(60.0 * KMH)
            .kind(VehicleType::Truck)
            .headway(2.0),
        Vehicle::new("CarA", 110.0, -1.75, 0.0, 40.0 * KMH)
            .wants(80.0 * KMH)
            .headway(2.0),
        Vehicle::new("Tailgater", 101.0, -1.75, 0.0, 40.0 * KMH)
            .wants(80.0 * KMH)
            .headway(0.5),
        Vehicle::new("Speeder", -60.0, -1.75, 0.0, 70.0 * KMH)
            .wants(115.0 * KMH)
            .speeder()
            .headway(1.5),
        Vehicle::new("Van", 700.0, 1.75, PI, 70.0 * KMH).headway(2.0),
        Vehicle::new("Moto", 2600.0, 1.75, PI, 80.0 * KMH).kind(VehicleType::Motorbike),
    ];
    let mut w = World::new(cars, Vec::new(), opts, 60.0);
    w.zones = Zones(vec![
        (-1e6, 400.0, 40.0, "village"),
        (400.0, 1500.0, 80.0, "national road"),
        (1500.0, 1e6, 40.0, "town"),
    ]);
    w.record_every = 5;
    w.watch_pair("CarA", "Speeder");
    w
}
