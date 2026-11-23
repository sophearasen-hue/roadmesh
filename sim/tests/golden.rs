//! Golden tests: the Rust core + simulator must reproduce the Python reference's
//! outcomes on all 5 scenarios (BUILD_PLAN Step 2). Expected values come from
//! `scripts/golden_from_reference.py`, which runs the read-only reference.
//!
//! Tolerances (agreed in the Step 2 plan):
//! * collisions: same pairs, time within 0.2 s
//! * who gives way to whom: same pairs in the same order, time within 0.2 s
//! * stopped-traffic warning: same car and relay count, time within 0.2 s, distance within 2 %
//! * minimum speeds of the main cars: within 1 km/h; hardest braking within 0.3 m/s²
//! * attack: same blocked / revoked counts
//! * behaviour (driving scenario, Car A's recorder): exact event counts,
//!   time over the limit within 2 s, time tailgating within 3 s, score within 3

use std::collections::HashMap;

use roadmesh_core::behavior::Subject;
use roadmesh_core::config::Config;
use roadmesh_core::obu::DriveState;
use roadmesh_sim::scenarios;
use roadmesh_sim::world::{Moment, Options, Role, World};
use serde_json::Value;

/// |a − b| ≤ tol; false if either value is missing (NaN).
fn within(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

const GOLDEN: &str = include_str!("golden/reference.json");

fn opts(variant: &str) -> Options {
    let base = Options {
        config: Config::reference(),
        ..Options::default()
    };
    match variant {
        "off" => Options { v2v: false, ..base },
        "nosec" => Options {
            security: false,
            ..base
        },
        "direct" => Options {
            relay: false,
            ..base
        },
        _ => base,
    }
}

struct Outcome {
    collisions: Vec<(f64, String, String)>,
    yields: Vec<(f64, String, String)>,
    hazards: Vec<(f64, String, f64, u8)>,
    min_speed: HashMap<String, f64>,
    max_brake: HashMap<String, f64>,
    revoked: usize,
    blocked_by: HashMap<String, usize>,
    behaviour: HashMap<String, HashMap<&'static str, f64>>,
}

fn run(w: &mut World) -> Outcome {
    let s = w.run();
    let mut yields = Vec::new();
    let mut hazards = Vec::new();
    for m in &w.moments {
        match m {
            Moment::Yield { t, car, target } => yields.push((*t, car.clone(), target.clone())),
            Moment::Hazard { t, car, gap, hops } => hazards.push((*t, car.clone(), *gap, *hops)),
            _ => {}
        }
    }
    let cars: Vec<String> = w
        .vehicles
        .iter()
        .filter(|v| v.role == Role::Car)
        .map(|v| v.name.clone())
        .collect();
    let mut behaviour = HashMap::new();
    if let Some(i) = w.idx("CarA")
        && let Some(obu) = &w.obus[i]
    {
        let cfg = obu.config().clone();
        for (who, p) in obu.behavior().profiles() {
            let name = match who {
                Subject::Me => "CarA".to_owned(),
                Subject::Peer(id) => w.owner(id),
            };
            let mut m = HashMap::new();
            m.insert("over_limit_s", p.over_limit_s);
            m.insert("harsh_brakes", f64::from(p.harsh_brakes));
            m.insert("harsh_accels", f64::from(p.harsh_accels));
            m.insert("overtakes", f64::from(p.overtakes));
            m.insert("oncoming_overtakes", f64::from(p.oncoming_overtakes));
            m.insert("overtaken", f64::from(p.overtaken));
            m.insert("tailgate_s", p.tailgate_s);
            m.insert("score", f64::from(p.score(&cfg)));
            behaviour.insert(name, m);
        }
    }
    Outcome {
        collisions: s.collisions.clone(),
        yields,
        hazards,
        min_speed: cars
            .iter()
            .map(|n| (n.clone(), s.min_speed_kmh[n]))
            .collect(),
        max_brake: cars
            .iter()
            .filter_map(|n| s.max_brake_ms2.get(n).map(|b| (n.clone(), *b)))
            .collect(),
        revoked: s.revoked.len(),
        blocked_by: w
            .vehicles
            .iter()
            .zip(&w.obus)
            .filter_map(|(v, o)| o.as_ref().map(|o| (v.name.clone(), o.blocked().len())))
            .collect(),
        behaviour,
    }
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}
fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_owned()
}

fn compare(key: &str, exp: &Value, got: &Outcome) -> Vec<String> {
    let mut errs = Vec::new();
    let mut err = |m: String| errs.push(format!("{key}: {m}"));

    let ec = exp["collisions"].as_array().cloned().unwrap_or_default();
    if ec.len() != got.collisions.len() {
        err(format!(
            "collisions {:?} vs reference {ec:?}",
            got.collisions
        ));
    } else {
        for (e, g) in ec.iter().zip(&got.collisions) {
            if (s(&e["a"]), s(&e["b"])) != (g.1.clone(), g.2.clone())
                || (f(&e["t"]) - g.0).abs() > 0.2
            {
                err(format!("collision {g:?} vs reference {e}"));
            }
        }
    }
    let ey = exp["yields"].as_array().cloned().unwrap_or_default();
    if ey.len() != got.yields.len() {
        err(format!("yields {:?} vs reference {ey:?}", got.yields));
    } else {
        for (e, g) in ey.iter().zip(&got.yields) {
            if (s(&e["car"]), s(&e["target"])) != (g.1.clone(), g.2.clone())
                || (f(&e["t"]) - g.0).abs() > 0.2
            {
                err(format!("yield {g:?} vs reference {e}"));
            }
        }
    }
    let eh = exp["hazards"].as_array().cloned().unwrap_or_default();
    if eh.len() != got.hazards.len() {
        err(format!("hazards {:?} vs reference {eh:?}", got.hazards));
    } else {
        for (e, g) in eh.iter().zip(&got.hazards) {
            let gap_ok = (f(&e["gap"]) - g.2).abs() <= 0.02 * f(&e["gap"]).max(1.0);
            if s(&e["car"]) != g.1
                || e["hops"].as_u64() != Some(u64::from(g.3))
                || (f(&e["t"]) - g.0).abs() > 0.2
                || !gap_ok
            {
                err(format!("hazard {g:?} vs reference {e}"));
            }
        }
    }
    if let Some(m) = exp["min_speed_kmh"].as_object() {
        for (name, v) in m {
            let g = got.min_speed.get(name).copied().unwrap_or(f64::NAN);
            if !within(f(v), g, 1.0) {
                err(format!("min speed {name}: {g} vs reference {v}"));
            }
        }
    }
    if let Some(m) = exp["max_brake_ms2"].as_object() {
        for (name, v) in m {
            let g = got.max_brake.get(name).copied().unwrap_or(f64::NAN);
            if !within(f(v), g, 0.3) {
                err(format!("max brake {name}: {g} vs reference {v}"));
            }
        }
    }
    if exp["revoked"].as_u64() != Some(got.revoked as u64) {
        err(format!(
            "revoked {} vs reference {}",
            got.revoked, exp["revoked"]
        ));
    }
    if let Some(m) = exp["blocked_by"].as_object() {
        for (name, v) in m {
            if v.as_u64() != got.blocked_by.get(name).map(|&n| n as u64) {
                err(format!(
                    "blocked by {name}: {:?} vs reference {v}",
                    got.blocked_by.get(name)
                ));
            }
        }
    }
    if let Some(m) = exp.get("behaviour_carA").and_then(Value::as_object) {
        for (name, prof) in m {
            let Some(g) = got.behaviour.get(name) else {
                err(format!("behaviour: {name} not recorded"));
                continue;
            };
            for (k, tol) in [
                ("harsh_brakes", 0.0),
                ("harsh_accels", 0.0),
                ("overtakes", 0.0),
                ("oncoming_overtakes", 0.0),
                ("overtaken", 0.0),
                ("over_limit_s", 2.0),
                ("tailgate_s", 3.0),
                ("score", 3.0),
            ] {
                let (e, gv) = (f(&prof[k]), g.get(k).copied().unwrap_or(f64::NAN));
                if !within(e, gv, tol + 1e-9) {
                    err(format!("behaviour {name}.{k}: {gv} vs reference {e}"));
                }
            }
        }
    }
    errs
}

#[test]
fn rust_core_reproduces_the_reference_on_all_scenarios() {
    let golden: Value = serde_json::from_str(GOLDEN).unwrap_or(Value::Null);
    let runs = golden.as_object().cloned().unwrap_or_default();
    assert_eq!(runs.len(), 12, "golden file should hold 12 runs");
    let mut errors = Vec::new();
    for (key, exp) in &runs {
        let (name, variant) = key.split_once('/').unwrap_or((key, "on"));
        let Some(mut w) = scenarios::build(name, opts(variant)) else {
            errors.push(format!("unknown scenario {key}"));
            continue;
        };
        errors.extend(compare(key, exp, &run(&mut w)));
    }
    assert!(
        errors.is_empty(),
        "golden mismatches:\n{}",
        errors.join("\n")
    );
}

#[test]
fn the_core_never_outputs_a_brake_command_only_advice() {
    // Every decision in every scenario: advised_accel is advice (≤ 0), and every
    // Danger alert carries a driver instruction (user requirement, Step 2).
    for name in scenarios::NAMES {
        let Some(mut w) = scenarios::build(name, Options::default()) else {
            continue;
        };
        while w.t < w.duration - 1e-9 {
            w.step();
            for d in w.decisions.iter().flatten() {
                assert!(
                    d.advised_accel.is_none_or(|a| a <= 0.0),
                    "{name}: positive advice"
                );
                for a in &d.alerts {
                    if a.level == roadmesh_core::obu::Level::Danger {
                        assert_ne!(
                            a.advice,
                            roadmesh_core::obu::BrakeAdvice::None,
                            "{name}: Danger without instruction"
                        );
                    }
                }
                if d.state == DriveState::Yield {
                    assert!(!d.alerts.is_empty(), "{name}: yielding without an alert");
                }
            }
        }
    }
}
