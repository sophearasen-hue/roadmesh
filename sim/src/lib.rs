//! RoadMesh traffic simulator: drives `roadmesh-core` through scripted scenarios.
//!
//! Step 2 ports the reference simulator (`reference/core-python/v2v/world.py`,
//! `scenarios.py`) so golden tests can prove the Rust core reproduces the
//! reference outcomes. Step 4 adds the Cambodian traffic mix, GPS noise and
//! packet-loss statistics.
//!
//! Everything security-related here (demo HMAC PKI, seeded random numbers) is
//! simulator-only and must never be used by the product.

pub mod pki;
pub mod rng;
pub mod scenarios;
pub mod world;
