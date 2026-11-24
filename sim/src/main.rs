//! `roadmesh-sim <scenario> [--spec] [--off] [--no-relay] [--no-security] [--loss P] [--seed N]`
//!
//! `--spec` runs the product profile (SPEC v0.1); the default is the reference profile.

use roadmesh_core::config::Config;
use roadmesh_sim::city::{City, CityOptions};
use roadmesh_sim::scenarios;
use roadmesh_sim::world::{Moment, Options};

fn city(args: &[String]) {
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
    };
    let mut o = CityOptions::default();
    if let Some(v) = value("--vehicles").and_then(|v| v.parse().ok()) {
        o.vehicles = v;
    }
    if let Some(v) = value("--loss").and_then(|v| v.parse().ok()) {
        o.loss = v;
    }
    if let Some(v) = value("--seed").and_then(|v| v.parse().ok()) {
        o.seed = v;
    }
    if let Some(v) = value("--penetration").and_then(|v| v.parse().ok()) {
        o.penetration = v;
    }
    if let Some(v) = value("--duration").and_then(|v| v.parse().ok()) {
        o.duration_s = v;
    }
    if let Some((lo, hi)) = value("--gps").and_then(|v| v.split_once('-'))
        && let (Ok(lo), Ok(hi)) = (lo.parse(), hi.parse())
    {
        o.gps_m = (lo, hi);
    }
    o.security = args.iter().any(|a| a == "--security");
    if let Some(v) = value("--full-cert-interval").and_then(|v| v.parse().ok()) {
        o.config.security.full_cert_interval_s = v;
    }
    o.relay = args.iter().any(|a| a == "--relay");
    let s = City::new(o).run();
    let (mean, median) = s.lead();
    println!(
        "conflicts {} warned {} missed {} ({:.1}%, {} with a Caution) | moto {}/{} | unequipped {}",
        s.conflicts,
        s.warned,
        s.missed,
        100.0 * s.missed_rate(),
        s.missed_with_caution,
        s.moto_missed,
        s.moto_conflicts,
        s.conflicts_unequipped
    );
    println!(
        "alerts {} false {} ({:.1}/h over {:.2} h) | lead mean {:.1} s median {:.1} s | collisions {:?} | frames {:?} | honest blocked {}",
        s.alerts,
        s.false_alarms,
        s.false_alarms_per_hour(),
        s.app_hours,
        mean,
        median,
        s.collisions,
        s.frames,
        s.honest_blocked
    );
    println!(
        "missed: {} not heard by radio, {} of {} conflicts with an unannounced turn missed; within the 8 m gate {}/{}",
        s.missed_not_heard, s.turn_missed, s.turn_conflicts, s.gated_missed, s.gated_conflicts
    );
    println!(
        "false Danger per 100 km: {:.1} over {:.0} km",
        s.false_danger_per_100km(),
        s.app_km
    );
    for (k, (n, f, fd)) in &s.by_kind {
        println!("  {k:20} alerts {n:4}  false {f:4}  false Danger {fd:4}");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "report") {
        let value = |flag: &str| {
            args.iter()
                .position(|a| a == flag)
                .and_then(|i| args.get(i + 1))
        };
        let seeds = value("--seeds").and_then(|v| v.parse().ok()).unwrap_or(4);
        let duration = value("--duration")
            .and_then(|v| v.parse().ok())
            .unwrap_or(120.0);
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        let text = roadmesh_sim::report::build(seeds, duration, threads);
        match value("--out") {
            Some(path) => {
                if let Err(e) = std::fs::write(path, &text) {
                    println!("cannot write {path}: {e}");
                } else {
                    println!("wrote {path}");
                }
            }
            None => println!("{text}"),
        }
        return;
    }
    if args.first().is_some_and(|a| a == "city") {
        city(&args[1..]);
        return;
    }
    let Some(name) = args.first() else {
        println!(
            "usage: roadmesh-sim <scenario> [--off] [--no-relay] [--no-security] [--loss P] [--seed N]"
        );
        println!("scenarios: {}", scenarios::NAMES.join(", "));
        return;
    };
    let mut opts = Options {
        v2v: !args.iter().any(|a| a == "--off"),
        ..Options::default()
    };
    opts.relay = !args.iter().any(|a| a == "--no-relay");
    opts.security = !args.iter().any(|a| a == "--no-security");
    if args.iter().any(|a| a == "--spec") {
        opts.config = Config::spec_v0();
    }
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
    };
    if let Some(p) = value("--loss").and_then(|v| v.parse().ok()) {
        opts.loss = p;
    }
    if let Some(s) = value("--seed").and_then(|v| v.parse().ok()) {
        opts.seed = s;
    }
    let Some(mut w) = scenarios::build(name, opts) else {
        println!(
            "unknown scenario {name}; choose from {}",
            scenarios::NAMES.join(", ")
        );
        return;
    };
    let s = w.run();
    for m in &w.moments {
        match m {
            Moment::Yield { t, car, target } => {
                println!("  t={t:5.1}s  {car} gives way to {target}")
            }
            Moment::Hazard { t, car, gap, hops } => {
                println!("  t={t:5.1}s  {car}: stopped traffic {gap:.0} m ahead, via {hops} relays")
            }
            Moment::Sight { t, gap } => {
                println!("  t={t:5.1}s  drivers can see each other, {gap:.0} m apart")
            }
            Moment::Revoked { t, owner } => {
                println!("  t={t:5.1}s  certificate of {owner} revoked")
            }
            Moment::Alert {
                t,
                car,
                alert: Some((kind, level, advice)),
                target,
            } => {
                let about = target
                    .as_ref()
                    .map(|n| format!(" ({n})"))
                    .unwrap_or_default();
                println!("  t={t:5.1}s  {car} screen: {level:?} {kind:?} -> {advice:?}{about}")
            }
            Moment::Alert {
                t,
                car,
                alert: None,
                ..
            } => println!("  t={t:5.1}s  {car} screen: all clear"),
        }
    }
    for (t, a, b) in &s.collisions {
        println!("  t={t:5.1}s  COLLISION {a} - {b}");
    }
    let mut speeds: Vec<_> = s.min_speed_kmh.iter().collect();
    speeds.sort_by(|a, b| a.0.cmp(b.0));
    println!("  min speeds (km/h): {speeds:?}");
    println!("  radio: {:?}", s.radio);
}
