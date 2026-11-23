# RoadMesh — project instructions for Claude Code

RoadMesh is a phone-based, peer-to-peer road safety app (V2V-style) for cars, motorbikes and tuk-tuks,
in English and Khmer, for iPhone (with CarPlay) and Android.

## Read first
- `docs/SPEC.md` — technical specification (export it from the spec doc if missing)
- `docs/BUILD_PLAN.md` — the ordered steps. Work on ONE step at a time.
- `docs/GAP_REPORT.md` — what the two references do and the security findings
- `docs/AUDIT.md` — the checklist every step is measured against

## The two references (READ-ONLY — never edit anything under `reference/`)
- `reference/core-python/` — the behaviour reference. Prediction, right of way, alerts, relay,
  plausibility checks, behaviour recorder and 26 tests. Run: `cd reference/core-python && python -m unittest discover -s tests -t .`
  The Rust core must reproduce its test results and its 5 scenario outcomes (golden tests).
- `reference/phone-prototype/index.html` — the screen reference. The Flutter app must match its
  four tabs (Drive, Peers, Behaviour, Settings), its CarPlay layout and its English/Khmer strings.
- `reference/console-app/index.html` — scenario console (for demos and for its Khmer event strings).

## Production architecture (do not change without asking)
- `core/`     Rust safety core: RSM codec (binary, SPEC §3), LDM, tracker, prediction, alerts, scoring, security checks.
              Pure logic, no I/O. Runs on iOS, Android and the simulator.
- `app/`      Flutter UI via flutter_rust_bridge.
- `ios/`      Swift: CoreBluetooth transport, CarPlay Driving Task scene, Secure Enclave keys.
- `android/`  Kotlin: BLE transport, foreground service (connectedDevice + location), Android Keystore/StrongBox keys.
- `server/`   Edge relay (MQTT, TLS 1.3, mutual TLS), certificate authority (keys in KMS/HSM), hazard service.
- `sim/`      Traffic simulator driving `core/` with GPS noise and packet loss.

## Safety rules
- The app WARNS. It never commands brakes or steering. The reference's YIELD/BRAKE states become Danger alerts.
- Safety logic lives ONLY in `core/`. UI and transport code never decide alerts.
- Every alert rule and threshold has a unit test. Thresholds live in `core/src/config.rs` only.

## Security rules (highest priority — see AUDIT section B)
- Every message signed with ECDSA P-256; private keys generated in secure hardware and never exported.
- NO shared, hard-coded or test keys in production code. The reference's HMAC/CA-lookup and seeded random are simulator-only.
- Verify before you trust: signature, certificate not revoked, timestamp fresh, not a replay, physically plausible, not flooding.
- Revocation needs 3 independent reporters; the reporting phone blocks the sender locally at once (as in the reference).
- Parse untrusted bytes only in Rust, with length checks; fuzz the decoder before every release.
- TLS 1.3 + certificate pinning; App Attest / Play Integrity before issuing certificates.
- Never log coordinates, pseudonyms or keys. No names, phone numbers or plates anywhere.
- Peer data deleted at track expiry + 10 min; own raw GPS trail after 24 h.
- If a request would weaken any of these rules, stop and ask.

## Localisation
- All user-facing text in `app/lib/l10n/app_en.arb` and `app_km.arb`. No hard-coded strings.
- Mark new Khmer strings `"@@x-needs-review": true`. Bundle Kantumruy Pro; never load fonts from the network.

## Store rules
- Android targetSdkVersion 36+, foreground service types declared, Data safety form matches reality.
- iOS PrivacyInfo.xcprivacy for the app and every SDK; purpose strings in English and Khmer.

## Workflow
- Plan mode first for every non-trivial task; wait for approval.
- Small steps, tests green before moving on; commit per step; PR per BUILD_PLAN step.
- After changing `core/`, run the golden scenarios and the simulator; report missed-conflict and false-alarm rates.
