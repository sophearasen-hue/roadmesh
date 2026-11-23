//! The product profile (`Config::spec_v0`, SPEC v0.1) on the five scenarios.
//!
//! Exact numbers differ from the reference by design (6 s horizon, earlier crossing
//! window, position uncertainty, spec relay limits and behaviour rules), so these
//! tests check safety properties instead, and report the conflict / false-alarm
//! rates CLAUDE.md asks for after every core change.

#![allow(clippy::panic)] // test code: a failed precondition should fail the test loudly

use std::collections::HashMap;

use roadmesh_core::behavior::Subject;
use roadmesh_core::config::Config;
use roadmesh_core::obu::{AlertKind, Level};
use roadmesh_sim::scenarios;
use roadmesh_sim::world::{Moment, Options, Summary, World};

fn spec(v2v: bool) -> Options {
    Options {
        v2v,
        config: Config::spec_v0(),
        ..Options::default()
    }
}

fn run(name: &str, opts: Options) -> (World, Summary) {
    let mut w = scenarios::build(name, opts).unwrap_or_else(|| unreachable!("scenario {name}"));
    let s = w.run();
    (w, s)
}

/// Danger alerts shown (entries into a Danger top alert), per car.
fn dangers(w: &World) -> HashMap<String, Vec<(f64, AlertKind, Option<String>)>> {
    let mut out: HashMap<String, Vec<_>> = HashMap::new();
    for m in &w.moments {
        if let Moment::Alert {
            t,
            car,
            alert: Some((kind, Level::Danger, _)),
            target,
        } = m
        {
            out.entry(car.clone())
                .or_default()
                .push((*t, *kind, target.clone()));
        }
    }
    out
}

fn first_alert(w: &World, car: &str, kind: AlertKind) -> Option<(f64, Option<String>)> {
    w.moments.iter().find_map(|m| match m {
        Moment::Alert {
            t,
            car: c,
            alert: Some((k, _, _)),
            target,
        } if c == car && *k == kind => Some((*t, target.clone())),
        _ => None,
    })
}

#[test]
fn blind_intersection_only_the_yielding_car_gets_danger_before_the_drivers_can_see() {
    let (_, off) = run("blind_intersection", spec(false));
    assert!(!off.collisions.is_empty());
    let (w, s) = run("blind_intersection", spec(true));
    assert!(s.collisions.is_empty());
    let d = dangers(&w);
    let sight = w.moments.iter().find_map(|m| {
        if let Moment::Sight { t, .. } = m {
            Some(*t)
        } else {
            None
        }
    });
    let a = first_alert(&w, "CarA", AlertKind::CrossingConflict);
    assert!(
        a.as_ref().is_some_and(
            |(t, who)| who.as_deref() == Some("CarB") && sight.is_some_and(|s| *t + 3.0 <= s)
        ),
        "{a:?} sight {sight:?}"
    );
    assert!(
        !d.contains_key("CarB"),
        "the car with priority must not get Danger: {:?}",
        d.get("CarB")
    );
    assert!(s.min_speed_kmh["CarB"] >= 49.0);
}

#[test]
fn emergency_brake_no_collision_at_any_loss() {
    let (_, off) = run("emergency_brake", spec(false));
    assert!(!off.collisions.is_empty());
    for loss in [0.0, 0.3, 0.6] {
        for seed in 1..=3 {
            let (w, s) = run(
                "emergency_brake",
                Options {
                    loss,
                    seed,
                    ..spec(true)
                },
            );
            assert!(s.collisions.is_empty(), "loss {loss} seed {seed}");
            let d = dangers(&w);
            assert!(
                d.contains_key("Car3"),
                "Car3 must be warned: loss {loss} seed {seed}"
            );
        }
    }
}

#[test]
fn ghost_attack_no_phantom_alert_with_security() {
    let (w, s) = run("ghost_attack", spec(true));
    assert_eq!(s.min_speed_kmh["CarA"], 50.0);
    assert!(
        dangers(&w).is_empty(),
        "forged/implausible cars raised Danger"
    );
    let (w, s) = run(
        "ghost_attack",
        Options {
            security: false,
            ..spec(true)
        },
    );
    assert!(s.min_speed_kmh["CarA"] < 45.0); // without signatures the attack works
    assert!(!dangers(&w).is_empty());
}

#[test]
fn relay_warns_beyond_1_km_with_stopped_traffic_events_and_brakes_gently() {
    let (_, off) = run("highway_relay", spec(false));
    assert!(!off.collisions.is_empty());
    let (w, s) = run("highway_relay", spec(true));
    let (wd, sd) = run(
        "highway_relay",
        Options {
            relay: false,
            ..spec(true)
        },
    );
    assert!(s.collisions.is_empty() && sd.collisions.is_empty());
    let first = |w: &World| {
        w.moments.iter().find_map(|m| match m {
            Moment::Hazard { car, gap, hops, .. } if car == "CarA" => Some((*gap, *hops)),
            _ => None,
        })
    };
    let (Some((gap, hops)), Some((gap_d, _))) = (first(&w), first(&wd)) else {
        panic!("no hazard warning")
    };
    assert!(gap > 1000.0 && hops >= 3, "relay: {gap} m via {hops}");
    assert!(gap_d < 310.0, "direct: {gap_d} m");
    assert!(s.max_brake_ms2["CarA"] + 1.0 < sd.max_brake_ms2["CarA"]);
    assert!(
        dangers(&w).get("CarA").is_none_or(|v| v.is_empty()),
        "{:?}",
        dangers(&w).get("CarA")
    );
}

#[test]
fn mixed_traffic_spec_alerts_and_behaviour() {
    let (w, s) = run("mixed_traffic", spec(true));
    assert!(s.collisions.is_empty());
    // the spec's warnings appear where they should
    assert!(first_alert(&w, "Speeder", AlertKind::OwnSpeeding).is_some());
    assert!(
        first_alert(&w, "CarA", AlertKind::FastApproachBehind)
            .is_some_and(|(_, who)| who.as_deref() == Some("Speeder"))
    );
    assert!(
        first_alert(&w, "CarA", AlertKind::OvertakingYou)
            .is_some_and(|(_, who)| who.as_deref() == Some("Speeder"))
    );
    // nobody but the tailgater ever needs a Danger alert on this road
    for (car, v) in dangers(&w) {
        assert_eq!(car, "Tailgater", "unexpected Danger for {car}: {v:?}");
    }
    // behaviour recorded by Car A with the spec rules
    let Some(i) = w.idx("CarA") else { panic!() };
    let Some(obu) = &w.obus[i] else { panic!() };
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
            .unwrap_or_else(|| panic!("no profile {name}"))
    };
    let (speeder, tailgater, jerky, car_a) = (
        prof("Speeder"),
        prof("Tailgater"),
        prof("Jerky"),
        prof("CarA"),
    );
    assert!(speeder.speeding_episodes >= 1 && speeder.oncoming_overtakes == 3);
    assert!(tailgater.tailgate_episodes >= 1);
    assert_eq!((jerky.harsh_brakes, jerky.harsh_accels), (1, 1)); // 6.0 >= 4.5, 3.5 >= 3.0
    assert!(car_a.score(&cfg) >= 85, "Car A should be in the Safe band"); // SPEC §7 bands
    assert!(speeder.score(&cfg) < 85, "the speeder should not be Safe");
}

/// CLAUDE.md: after changing core/, report missed-conflict and false-alarm rates.
/// Missed conflict = a collision with V2V on. False alarm = a Danger alert in a run where
/// that car never needed to brake hard (advised deceleration stayed under 3 m/s²) and
/// was never within 3 s of a conflict.
#[test]
fn report_missed_conflicts_and_false_alarms() {
    let mut runs = 0;
    let mut missed = 0;
    let mut dangers_total = 0;
    let mut false_alarms = 0;
    for name in scenarios::NAMES {
        for loss in [0.0, 0.3] {
            let (w, s) = run(name, Options { loss, ..spec(true) });
            runs += 1;
            missed += s.collisions.len();
            let mut per_car = Vec::new();
            for (car, v) in dangers(&w) {
                dangers_total += v.len();
                // in the attack scenario every Danger is caused by a fake vehicle
                if name == "ghost_attack" {
                    false_alarms += v.len();
                }
                per_car.push(format!("{car}:{}", v.len()));
            }
            per_car.sort();
            println!(
                "  {name:<20} loss {loss:.1}: collisions {}, Danger alerts [{}]",
                s.collisions.len(),
                per_car.join(" ")
            );
        }
    }
    println!(
        "spec profile: {runs} runs, missed conflicts {missed}, Danger alerts {dangers_total}, false alarms {false_alarms}"
    );
    assert_eq!(missed, 0);
    assert_eq!(false_alarms, 0);
}
