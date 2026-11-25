# Audit report: section B, security (BUILD_PLAN Step 11)

Checked against `docs/AUDIT.md` section B. Status keys:

- **PASS**: done and verified by the evidence shown.
- **PASS (code) / device test**: done in code, but the AUDIT's check needs real phones.
- **OWNER**: an action outside the repository: cloud, accounts, contracts.

The "done when" for Step 11 is "section B all PASS; MobSF no high findings". **MobSF: no high and no warning findings** (CI job `mobsf`, MobSF rules over the Android, iOS and Flutter sources). **Section B: 17 PASS; 2 done in code and waiting for a device test (S-02, S-13); 5 done in code or as a file, with the rest owner-side (S-19, S-20, S-21, S-25, S-26); 3 OWNER only (S-18, S-22, S-23).** None is MISSING. The OWNER items cannot be done from code, and are listed at the end with what you need to do.

| ID | Requirement | Status | Evidence |
|---|---|---|---|
| S-01 | Every message signed (ECDSA P-256); no shared/HMAC key in production | **PASS** | `core/src/pki.rs` (signature over domain ‖ hour ‖ body); `obu.rs` verifies every message by default (`every_message_is_verified_by_default`). gitleaks in CI; HMAC exists only in `reference/` (read-only) and the simulator. |
| S-02 | Hardware-backed, non-exportable keys | **PASS (code) / device test** | Android `KeystoreKeys.kt` (StrongBox, falling back to TEE, sign-only); iOS `KeychainKeys.swift` (Secure Enclave, `privateKeyUsage`). The core receives signatures only (`HardwareKeys`). Device check: confirm StrongBox/SE on the pilot phones. |
| S-03 | Anti-replay: freshness 1 s / 10 s, (temp_id, msg_id) cache, clock skew | **PASS** | `ldm.rs` freshness and duplicate cache; signature covers the Unix hour (`pki::rsm_to_be_signed`); `clock_skew_s` 2 s; server hazard gate repeats the checks (`server/tests/broker.rs` stale/duplicate). Phones use the OS network-synchronised clock. |
| S-04 | ≤ 20 certificates/device/week; App Attest / Play Integrity first | **PASS** | `server/src/ca.rs` quota (`server/tests/ca.rs`); `server/src/attest.rs` bound to the request (`server/tests/attest.rs`: chain, nonce, counter, one key per device, Play verdicts). |
| S-05 | Revocation list; phones refresh daily and on connection | **PASS** | Signed CRL (`ca.rs`, `CertVerifier::load_revocation_list`); server pushes on change and at least every 30 min, and on every subscribe (`broker.rs`); phones subscribe on every connection (`CellularLink::follow_position`). |
| S-06 | Plausibility checks, reports, revocation after repeated reports | **PASS** | `ldm.rs` speed/acceleration/jump/flood, `obu.rs` off-road; reports with signed evidence; 3 independent reporters (`server/tests/ca.rs`); simulator: honest vehicles blocked = 0 (`docs/SIM_REPORT.md`). The radio signal-strength check vs distance is recorded in the field log for Gate 1 tuning. |
| S-07 | Relay limits; a relay never increases TTL | **PASS** | SPEC §5 rules 1–7 in `obu.rs` (Step 7 tests); TTL only decremented; simulator: a phone never relays the same message twice. |
| S-08 | Per-sender rate limit (> 12 msg/s ignored); cap on total processed; lowest priority dropped first | **PASS** | `min_msg_interval_s` = 1/12 s (flood → misbehaviour); **new in Step 11:** `rx_budget_per_s` = 600 frames/s to signature checking, keeping events and nearby direct senders first (`overload_keeps_events_and_near_vehicles...`). City simulation at 200 vehicles: the cap is never reached by honest traffic (results identical with and without it). Server: 25/s per session, 5 enrolments/min per address. |
| S-09 | Memory-safe parsing only in Rust, length checks, no `unsafe` | **PASS** | `#![forbid(unsafe_code)]` workspace-wide (only UniFFI's generated FFI glue in `ffi/` allows it). Radio frames, envelopes, certificates, MQTT and server inputs are all parsed in Rust; Kotlin/Swift pass bytes through. |
| S-10 | Fuzzing; 24 h before each release | **PASS** | 6 cargo-fuzz targets (`rsm_frame`, `receive_pipeline`, `pki_parsers`, `fragments`, `mqtt`, `server_inputs`), 5 min each in CI; the 24 h run is a release gate (`docs/RELEASE.md`). |
| S-11 | No secrets in app or repo | **PASS** | gitleaks over history in CI. Server keys live in KMS and Secrets Manager (`docs/SERVER.md`). The app holds only public trust anchors and pins. |
| S-12 | Obfuscation; root/jailbreak lowers trust | **PASS** | **New in Step 11:** `DeviceTrust` on both platforms makes a compromised phone receive-only (still warned, sends nothing); `Flutter/Release.xcconfig` strips Swift and local symbols; Rust release builds drop debug info; R8 on Android; `--obfuscate --split-debug-info` in the release commands. |
| S-13 | Random BLE addresses; pseudonym and address rotate together, only when stopped or at a junction | **PASS (code) / device test** | Android restarts the advertising set (random address) on pseudonym change; iOS relies on its own address rotation; `PseudonymSigner::set_rotation_allowed` (stopped or junction, at most 5 min late). Device check with a BLE sniffer at Gate 1. |
| S-14 | No names/phones/plates; own trail ≤ 24 h; peer data at expiry + 10 min | **PASS** | No personal fields exist in any message or store. The phone stores no position trail at all (the behaviour recorder keeps counts and speeds, never coordinates). Peer records are pruned at LDM expiry + 10 min (`behavior.rs`). **New:** backups disabled (`allowBackup=false`, data-extraction rules; iOS certificates excluded from iCloud backup). |
| S-15 | No location, pseudonyms or keys in logs | **PASS** | No platform logging calls in the app (checked by grep); field log has counts and distance bands only (`FieldLogTest`); server audit lines carry an 8-byte device reference only (`monitor.rs`). |
| S-16 | Mutual TLS + App Attest / Play Integrity | **PASS** | `server/src/tls.rs` client-certificate verifier; `server/tests/tls.rs` (no certificate or foreign CA → refused); enrolment only after attestation. |
| S-17 | TLS 1.3 only; pinning with a backup pin; MITM test fails | **PASS** | rustls built without TLS 1.2 (test: TLS 1.2 refused); app pins (`PinnedTls.kt`, `PinnedConnection.swift`; at least 2 pins required); MITM test with a trusted-CA certificate fails the pin (`server/tests/tls.rs`). |
| S-18 | Server hardening: least privilege, private networks, WAF/rate limits, patching | **OWNER** | Design in `docs/SERVER.md` (IAM scope, NLB, security groups, Fargate platform updates); in-process rate limits done. Infrastructure review after you deploy. |
| S-19 | CA key in KMS/HSM; two-person rule | **PASS (code) / OWNER** | `server/src/kms.rs`; the server refuses to start without KMS. You create the keys and the two-person approval (`docs/SERVER.md`). |
| S-20 | Server and phone data encrypted at rest | **PASS (code) / OWNER** | Phone: no database; keys in hardware; certificates in app-private storage, iOS file protection (**new**: moved off Android shared storage). Server: state in memory today, so nothing is at rest yet. The planned DynamoDB store must use KMS encryption (`docs/SERVER.md`, known gap). |
| S-21 | Monitoring; audit logs 1 year | **PASS (code) / OWNER** | `monitor.rs` metrics and ALERT lines (tested); CloudWatch 365-day retention and alarms are set up at deployment. |
| S-22 | Developer accounts: hardware-key 2FA, few admins | **OWNER** | Apple, Google, GitHub and AWS account settings. |
| S-23 | App signing keys protected; Play App Signing | **OWNER** | Store set-up (Step 14). |
| S-24 | Pinned dependencies, lockfiles, audit, Dependabot, SBOM | **PASS** | `Cargo.lock`, `pubspec.lock`, pinned action commits; `cargo audit` in CI; **new:** Dependabot (`.github/dependabot.yml`) and CycloneDX SBOM job. |
| S-25 | MobSF per release; independent penetration test before launch | **PASS (code) / OWNER** | CI `mobsf` job: no high or warning findings (**fixed in Step 11:** StrandHogg 2.0 → `singleInstance`, `allowBackup=false`, tapjacking filter). Remaining INFO items: pinning and root detection are in code (the scanner's rules look for config files); screenshot blocking is not wanted in a driving app; SafetyNet is replaced by Play Integrity. Scan the final binaries in a MobSF server and commission the pen test before launch (`docs/RELEASE.md`). |
| S-26 | SECURITY.md and contact; critical fixes in 7 days | **PASS (file) / OWNER** | `SECURITY.md` (**new**). You set the contact address (placeholder marked). |
| S-27 | Incident-response plan | **PASS** | `docs/INCIDENT_RESPONSE.md` (**new**): roles, severities, revoke one/all, forced update, server compromise, data exposure. Names to fill in before the pilot. |

## For you (OWNER items)

1. **Deploy the server** as in `docs/SERVER.md`: KMS keys with two-person approval, NLB, security groups, CloudWatch retention and alarms (S-18, S-19, S-21). Before the pilot, add the encrypted persistent store (S-20).
2. **Secure the accounts.** Hardware security keys on the Apple, Google, GitHub and AWS accounts, with as few admins as possible (S-22).
3. **Signing keys.** Turn on Play App Signing, and keep the upload key and the Apple distribution certificate in a password manager or HSM (S-23).
4. **Publish the security contact** and fill in its placeholder in `SECURITY.md`. Fill in the roles in `docs/INCIDENT_RESPONSE.md` (S-26, S-27).
5. **Before public launch:** a MobSF scan of the final binaries and an independent penetration test (S-25).
6. **At Gate 1, on real phones:** confirm StrongBox/Secure Enclave (S-02), and that the BLE address rotates with the pseudonym (S-13), using a BLE sniffer.

# Audit report: section C, store launch readiness (BUILD_PLAN Step 14)

Everything that can be prepared in the repository is done. The rest needs your developer accounts, Apple's entitlement decision, a beta with real drivers, or lawyers. Store-ready content is in `docs/STORE.md`; privacy answers are in `docs/DATA_SAFETY.md`; policy, terms and support pages are in `docs/legal/`.

| ID | Requirement | Status | Evidence / what is left |
|---|---|---|---|
| L-01 | Apple Developer Program account | **OWNER** | Organisation account recommended (also smoother on Play, L-15). |
| L-02 | Privacy manifest in the app and every SDK | **PASS** | **New:** `app/ios/Runner/PrivacyInfo.xcprivacy` (precise location and device ID, not linked, no tracking; UserDefaults CA92.1), plus the plugin's manifest; Flutter and shared_preferences ship their own. A CI step checks the built app contains it. |
| L-03 | Purpose strings in English and Khmer | **PASS** | `Runner/{en,km}.lproj/InfoPlist.strings`. **Fixed in Step 14:** these were bundled into the test target, not the app; CI now checks the app bundle. No motion sensors, so no `NSMotionUsageDescription`. Khmer wording awaits native review (P-44). |
| L-04 | Background modes only if used | **PASS** | `bluetooth-central`, `bluetooth-peripheral`, `location`, each justified in `docs/STORE.md`. |
| L-05 | App Privacy answers match data handling | **PASS** | `docs/DATA_SAFETY.md`, checked against the code and section B. Enter it in App Store Connect. |
| L-06 | CarPlay Driving Task entitlement | **OWNER** | Request it now (`docs/CARPLAY.md`); the app works without it. |
| L-07 | Safety disclaimer; no interaction while driving | **PASS** | **New:** onboarding safety notice that must be accepted ("driver assistance, not a guarantee; never brakes or steers", tested in both languages); screens other than Drive lock above 10 km/h (P-16); CarPlay list template only. |
| L-08 | TestFlight beta with real drivers | **OWNER** | Step 12. |
| L-09 | Privacy policy and support URLs; screenshots in English and Khmer | **PASS (content) / OWNER** | `docs/legal/privacy-policy.{en,km}.md`, `support.{en,km}.md`; 8 iPhone 6.9″ screenshots (`docs/store/screenshots/ios-*`). You publish the pages on your domain and fill in the contact placeholders. |
| L-10 | targetSdk 36+ | **PASS** | `app/android/app/build.gradle.kts`: targetSdk 36. |
| L-11 | FGS types, permissions, Play Console declaration with demo video | **PASS (code) / OWNER** | Manifest declares `connectedDevice|location` with matching permissions; declaration text and video script in `docs/STORE.md`. You record the video and submit. |
| L-12 | Location declaration | **PASS (content) / OWNER** | Justification text in `docs/STORE.md`; no background-location permission requested (foreground service only). |
| L-13 | Data safety form matches | **PASS** | `docs/DATA_SAFETY.md` (Play section); backups disabled to match. |
| L-14 | Bluetooth permissions | **PASS** | SCAN, ADVERTISE, CONNECT; `neverForLocation` deliberately not set (fine location is held anyway). |
| L-15 | Closed testing track | **OWNER** | 12 testers for 14 days on a personal account; plan for it in Step 12. |
| L-16 | Play App Signing, R8, release AAB | **PASS (code) / OWNER** | R8 and shrinking on, obfuscated AAB command in `docs/STORE.md` and `docs/RELEASE.md`; you enable Play App Signing. |
| L-17 | Policy URL, listing and screenshots in English and Khmer | **PASS (content) / OWNER** | Listing texts in `docs/STORE.md`; 8 Android screenshots (`docs/store/screenshots/android-*`). Publish the policy. |
| L-18 | Crash-free sessions ≥ 99.5% in beta | **OWNER** | Measured in Step 12 (Gate 3). |
| L-19 | Battery < 8%/hour on a low-end Android | **OWNER** | Measured with the field log in Step 12 (Gate 1/3). |
| L-20 | Legal review | **OWNER** | Drafts in `docs/legal/` (terms, privacy policy) for Cambodian counsel; TRC approval only if the LoRa dongle ships (not in the MVP). |

**Summary:**
- **Done in the repository:** 8 PASS, plus 6 with all content ready and the submission left to you.
- **Owner-only:** 6 (accounts, entitlement, beta, the beta's measurements, legal).
- **Review before submission:** all Khmer texts (P-44).
