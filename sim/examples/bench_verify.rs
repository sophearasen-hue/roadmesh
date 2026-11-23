//! Measures ECDSA P-256 sign and verify time per message, for the SPEC §10
//! sampling decision (Step 5). Run: `cargo run --release -p roadmesh-sim --example bench_verify`.

use roadmesh_core::auth::{Signer, Verifier};
use roadmesh_core::config::Config;
use roadmesh_core::pki::CertVerifier;
use roadmesh_core::rsm::{self, EventCode, MsgType, Rsm, VehicleType};
use roadmesh_sim::{keys, rng::SimRng};
use std::time::Instant;
fn main() {
    let mut rng = SimRng::new(1);
    let mut ca = keys::new_ca(&mut rng, Config::spec_v0());
    let mut s = keys::enrol(&mut ca, &mut rng, 1, 0);
    let mut v = CertVerifier::new(&[ca.trust_anchor()], &Config::spec_v0().security)
        .unwrap_or_else(|_| std::process::abort());
    let id = s.current_temp_id(1.0).unwrap_or([0; 4]);
    let b = Rsm {
        msg_type: MsgType::Heartbeat,
        temp_id: id,
        msg_id: 1,
        timestamp_ms: rsm::ms_of_hour(1.0),
        lat_deg: 11.5,
        lon_deg: 104.9,
        speed: 1.0,
        heading_deg: 0.0,
        accel: 0.0,
        yaw_rate: 0.0,
        vehicle_type: VehicleType::Car,
        flags: 0,
        pos_accuracy_m: 1.0,
        hop_count: 0,
        ttl: 1,
        origin_pos_age_ms: 0,
        event: EventCode::None,
    }
    .encode();
    let a = s.sign(1.0, false, &b).unwrap_or_default();
    let n = 2000;
    let t = Instant::now();
    for _ in 0..n {
        let _ = v.verify(1.0, &b, &a);
    }
    println!(
        "verify: {:.1} us",
        t.elapsed().as_secs_f64() * 1e6 / f64::from(n)
    );
    let t = Instant::now();
    for _ in 0..n {
        let _ = s.sign(1.0, false, &b);
    }
    println!(
        "sign: {:.1} us",
        t.elapsed().as_secs_f64() * 1e6 / f64::from(n)
    );
}
