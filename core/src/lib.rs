//! RoadMesh safety core.
//!
//! Pure logic with no I/O, shared by the iOS app, the Android app and the
//! simulator (SPEC §2, §11). The app only *warns*: this crate outputs alert
//! levels and never a brake or steering command (CLAUDE.md, GAP_REPORT F-5).
//!
//! Contents: the RSM codec, LDM, prediction, alerts and behaviour scoring
//! (Step 2, ported from `reference/core-python`), and pseudonym certificates,
//! signature checks and revocation (Step 3, [`pki`]).

pub mod auth;
pub mod behavior;
pub mod config;
pub mod fragment;
pub mod geo;
pub mod geometry;
pub mod ldm;
pub mod map;
pub mod obu;
pub mod pki;
pub mod prediction;
pub mod rsm;
pub mod tracker;

/// Version of the core, reported to the app and in test reports.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_manifest() {
        assert_eq!(VERSION, "0.1.0");
    }
}
