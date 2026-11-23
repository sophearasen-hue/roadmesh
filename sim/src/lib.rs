//! RoadMesh traffic simulator: drives `roadmesh-core` through scripted scenarios.
//!
//! Step 2 ports the reference simulator (`reference/core-python/v2v/world.py`,
//! `scenarios.py`) so golden tests can prove the Rust core reproduces the
//! reference outcomes. Step 4 adds the Cambodian traffic mix, GPS noise and
//! packet-loss statistics.
//!
//! The simulator runs the real certificate, signature and revocation code
//! (`roadmesh_core::pki`, `roadmesh_server::ca`). Only the key storage is
//! simulated: software keys from a seeded generator ([`keys`]), which is
//! simulator-only and must never be used by the product.

pub mod keys;
pub mod rng;
pub mod scenarios;
pub mod world;
