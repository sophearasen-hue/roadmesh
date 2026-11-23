//! The whole receive path on a phone: decode → verify → checks → LDM → relay
//! queue → decide (AUDIT S-10). Inputs are raw radio bytes, or mutations of a
//! validly signed frame so the fuzzer also reaches the checks after the signature.
#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use roadmesh_core::behavior::NoLimits;
use roadmesh_core::config::Config;
use roadmesh_core::geo::GeoOrigin;
use roadmesh_core::obu::{Obu, OwnState, RoadContext};
use roadmesh_core::prediction::Motion;
use roadmesh_core::rsm::VehicleType;

#[path = "common.rs"]
mod common;

static FIX: OnceLock<common::Fixture> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let fix = FIX.get_or_init(common::Fixture::new);
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    let mut verifier = fix.verifier();
    let mut obu = Obu::new(Config::spec_v0(), GeoOrigin::new(11.5564, 104.9282), true);
    let own = OwnState {
        motion: Motion { x: 0.0, y: 0.0, heading: 0.0, speed: 10.0, accel: 0.0, yaw_rate: 0.0 },
        vehicle_type: VehicleType::Car,
        pos_accuracy_m: 2.0,
    };
    let frames: Vec<Vec<u8>> = if mode & 1 == 0 {
        rest.chunks(220).map(<[u8]>::to_vec).collect()
    } else {
        // the valid frame first, then a mutated copy
        vec![fix.frame.clone(), common::mutate(&fix.frame, rest)]
    };
    for (i, f) in frames.iter().enumerate() {
        let now = common::NOW + 0.1 * i as f64;
        let _ = obu.receive_bytes(f, now, &own, &mut verifier);
    }
    let _ = obu.decide(common::NOW + 1.0, 0.1, &own, &RoadContext::default(), &NoLimits);
});
