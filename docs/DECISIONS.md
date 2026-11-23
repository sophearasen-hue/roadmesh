# Decisions log

Decisions taken during the build, with who decided and where they are implemented.

| Date | Decision | Decided by | Where |
|---|---|---|---|
| 2026-11-23 | Server language: Rust | Product owner | `server/` (Step 1) |
| 2026-11-23 | Golden tests: port the reference values first (`Config::reference`), then switch the product to the spec values (`Config::spec_v0`) in a separate commit | Product owner | `core/src/config.rs`, `sim/tests/golden.rs`, `sim/tests/spec_profile.rs` |
| 2026-11-23 | Relay scenario under the spec's hop limits: new event code 5 "stopped traffic" (TTL 5) | Product owner | SPEC §3, `core/src/obu.rs` |
| 2026-11-23 | Each Danger alert carries the advised braking value (information only; the simulator's test driver follows it) | Product owner | `core/src/obu.rs` `Alert::advised_decel`, `Decision::advised_accel` |
| 2026-11-23 | Every Danger/Warning alert carries a driver instruction (Ease off / Brake gently / Brake firmly / Brake hard now) | Product owner | `core/src/obu.rs` `BrakeAdvice` |
| 2026-11-23 | Imprecise tracks (> 8 m) raise a Caution "possible conflict, ease off" instead of nothing | Product owner | SPEC §6, `Config::low_confidence_caution` |
| 2026-11-23 | GPS smoothing (Kalman) and map matching move to Step 4 | Product owner | BUILD_PLAN Step 4 |
| 2026-11-23 | RSM v0 exactly as SPEC §3 lists (31 bytes); certificate digest and signature envelope in Step 3 | Product owner | `core/src/rsm.rs` |
| 2026-11-23 | Behaviour thresholds use the spec's values in the product profile | Product owner | `Config::spec_v0` |
| 2026-11-23 | Position uncertainty lengthens outlines along the path only (sideways widening caused false oncoming alarms between lanes 3.5 m apart) | Engineering (Step 2g, found in simulation) | SPEC §6 note, `Config::uncertainty_margin_factor` |
| 2026-11-23 | Overtake counted from which vehicle was behind before the crossing (fixes a reference edge case at exactly 0 m) | Engineering (Step 2e) | `core/src/behavior.rs` |
