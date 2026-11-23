//! RoadMesh safety core.
//!
//! Pure logic with no I/O, shared by the iOS app, the Android app and the
//! simulator (SPEC §2, §11). The app only *warns*: this crate outputs alert
//! levels and never a brake or steering command (CLAUDE.md, GAP_REPORT F-5).
//!
//! Step 1 of `docs/BUILD_PLAN.md` creates the crate only. The port of
//! `reference/core-python` arrives in Step 2.

pub mod config;
pub mod geo;
pub mod geometry;
pub mod prediction;
pub mod rsm;

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
