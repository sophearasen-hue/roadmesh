//! `roadmesh-sim <scenario> [--off] [--no-relay] [--no-security] [--loss P] [--seed N]`

use roadmesh_sim::scenarios;
use roadmesh_sim::world::{Moment, Options};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
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
