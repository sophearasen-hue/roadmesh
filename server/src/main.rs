//! RoadMesh edge server. Placeholder until BUILD_PLAN Step 9.
//!
//! Will provide: MQTT over TLS 1.3 with mutual TLS, certificate authority with
//! keys in a cloud KMS/HSM, App Attest / Play Integrity checks, hazard service
//! (SPEC §10, AUDIT S-16 to S-21).

fn main() {
    println!(
        "roadmesh-server {} (core {})",
        env!("CARGO_PKG_VERSION"),
        roadmesh_core::VERSION
    );
}
