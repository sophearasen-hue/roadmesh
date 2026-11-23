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
| 2026-11-23 | Security envelope: signature + 8-byte certificate digest normally; full certificate at least every 1 s, on every event and for 2 s after a pseudonym change; auth field up to 176 bytes | Product owner (Step 3) | SPEC §3, §10, `core/src/pki.rs`, `rsm::MAX_AUTH_LEN` |
| 2026-11-23 | Verify every message in v1; 1-in-4 heartbeat sampling is a setting, off by default, decided in Step 5 with phone CPU measurements | Product owner (Step 3) | `Security::verify_sample_above_peers` |
| 2026-11-23 | "Not near a road" and signal-strength plausibility checks are wired in Steps 4 and 5 when their inputs exist | Product owner (Step 3) | SPEC §10 |
| 2026-11-23 | One hardware key per pseudonym plus a device key; revocation list is a signed sorted digest list, not a compact filter | Product owner (Step 3) | SPEC §10 |
| 2026-11-23 | cargo-fuzz runs on a date-pinned nightly (`nightly-2026-11-01`) used only in `fuzz/`; the product stays on stable 1.97.0 | Product owner (Step 3) | `fuzz/`, CI `fuzz` job |
| 2026-11-23 | The signature covers the sender's Unix hour, so a message replayed an hour or more later fails (the body holds only ms since the hour) | Engineering (Step 3) | SPEC §10, `pki::rsm_to_be_signed` |
| 2026-11-23 | Copies of an already-accepted (temp_id, msg_id) are dropped before the signature check (halved simulator verification work; forged copies cannot poison the cache because only verified messages enter it) | Engineering (Step 3) | `Obu::receive`, `Ldm::has_seen` |
| 2026-11-23 | Pseudonym rotation only when stopped or at a junction (AUDIT S-13) moves to Step 5 with the BLE address rotation; Step 3 rotates every 5 min | Engineering (Step 3) | `PseudonymSigner::certificate_at` |
