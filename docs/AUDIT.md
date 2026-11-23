# RoadMesh — Audit checklist for Claude Code

Version 0.1 · 2026-11-22
Use this file to check the codebase against (A) the approved prototype, (B) security requirements,
and (C) App Store / Google Play launch requirements. Produce the report described in section D.

Reference files:
- `docs/SPEC.md` — technical specification (sections 1–13)
- `reference/phone-prototype/index.html` — the approved phone prototype (screens)
- `reference/core-python/` — the behaviour reference engine (logic + tests)

Status values: **PASS** (implemented and tested) · **PARTIAL** (exists but incomplete or untested) · **MISSING** · **N/A in MVP**

---

## A. Prototype parity — every item must be PASS before the fleet pilot

### A1. Drive screen
| ID | Requirement | How to verify |
|---|---|---|
| P-01 | Live map centred on the user, smooth follow | Run on device; map follows while driving |
| P-02 | Every peer in the mesh drawn as a heading arrow, coloured by risk band (Safe green ≥85, Caution amber 65–84, Risky red <65) | Widget test with fake peers of each band |
| P-03 | Speed label (km/h) next to each peer in Nearby view | Widget test |
| P-04 | Radio range circle around the user | Visual check |
| P-05 | Mesh links drawn from relay parent to child, colour by hop count | Widget test with a 3-hop fake mesh |
| P-06 | Nearby view (~500 m wide) ↔ Network view (whole mesh) toggle | Tap test |
| P-07 | Legend: You, Safe, Caution, Risky, Not in mesh | Visual check, both languages |
| P-08 | Alert banner: green "All clear" / amber Warning / red pulsing Danger; alert target ringed on map | Core test + widget test per alert type |
| P-09 | HUD: own speed, speed-limit sign (pulses when over), peers connected, mesh reach (m/km), trip score, current road name | Widget test |
| P-10 | Tap a peer on the map opens Peer detail | Integration test |
| P-11 | Driver controls in prototype (speed +/−, hard brake, hazard demo) exist ONLY in a debug/demo build flag, never in the release build | Check build flavours; release build must not contain them |

### A2. Peers screen
| ID | Requirement | How to verify |
|---|---|---|
| P-12 | Title "N vehicles in your mesh" + subtitle on direct vs relayed | Widget test |
| P-13 | List sorted by distance: type icon, type name, temp_id, distance, link (Direct / relayed n times / cellular), road name, risk badge + score, speed | Widget test with fake data |
| P-14 | Empty state suggests a longer-range radio or more hops | Widget test |
| P-15 | Peer detail: speed, acceleration (m/s²), distance, link, road, latency (ms), 2-minute speed graph with dashed limit line, event-count chips, event list | Widget test |
| P-16 | Screen locked above 10 km/h (shows Drive instead) | Integration test with mocked speed |

### A3. Behaviour screen
| ID | Requirement | How to verify |
|---|---|---|
| P-17 | Trip score ring (0–100) coloured by band + band name | Widget test |
| P-18 | Distance, average speed, top speed, total events | Core test of trip stats |
| P-19 | Event chips: Speeding, Harsh acceleration, Harsh braking, Overtake with counts | Widget test |
| P-20 | Own 2-minute speed graph | Widget test |
| P-21 | "Drivers around you": counts of risky / caution / safe peers | Widget test |
| P-22 | Event timeline with filter All / Me / Others; each row: time ago, event tag, who, speed, road | Widget test |

### A4. Settings screen
| ID | Requirement | How to verify |
|---|---|---|
| P-23 | Language English / ខ្មែរ, applied instantly, remembered | Integration test |
| P-24 | Radio options shown with range + latency description (BLE; Wi-Fi Aware where supported; long-range dongle pairing) | Visual check |
| P-25 | Max relay hops setting is a DEBUG setting only; release uses spec section 5 TTL values | Check build flavours |
| P-26 | Privacy statement (same text as prototype) in both languages | String check |
| P-27 | "Delete my data" removes phone + server data | Integration test against staging server |
| P-28 | Motorbike mode (audio + vibration alerts, screen may be off) | Device test |

### A5. Core logic (must match prototype behaviour, then the stricter spec rules)
| ID | Requirement | How to verify |
|---|---|---|
| P-29 | Crossing-conflict alert with side (left/right) and seconds to conflict | Unit test: blind-junction scenario |
| P-30 | Hard braking ahead (0–200 m, same road + direction) with distance | Unit test |
| P-31 | Vehicle overtaking you | Unit test |
| P-32 | Fast vehicle approaching from behind with its speed | Unit test |
| P-33 | Own speeding alert (limit + 5 km/h) | Unit test |
| P-34 | Right-of-way rule: only the yielding vehicle gets Danger; no symmetric deadlock | Unit test: two equal vehicles meet |
| P-35 | Hysteresis: no alert flicker (enter/exit thresholds per spec section 8) | Unit test: oscillating input |
| P-36 | Behaviour events and penalties exactly per spec section 7; once per episode; recovery 1.2 pts/min | Unit tests per event |
| P-37 | Peer events recorded ONLY while the peer is in the user's mesh | Unit test |
| P-38 | Multi-hop: mesh reach grows with hops; duplicates never relayed | Simulation test |
| P-39 | All thresholds live in `core/src/config.rs` | Code search |

### A6. Localisation
| ID | Requirement | How to verify |
|---|---|---|
| P-40 | Every prototype string exists in `app_en.arb` and `app_km.arb` (no hard-coded text) | Script: compare keys; grep for string literals in widgets |
| P-41 | Khmer font bundled (Kantumruy Pro or Noto Sans Khmer); Khmer renders correctly at 11–40 pt | Device screenshots |
| P-42 | No text overflow in Khmer on a 360-px-wide phone | Golden tests in both languages |
| P-43 | Road names localised | Data check |
| P-44 | Every Khmer string marked reviewed by a native speaker before release | ARB metadata check |

---

## B. Security — highest priority

Principle: **assume every radio message, every phone and every network is hostile.**
Target standard: OWASP MASVS (Mobile Application Security Verification Standard), levels L2 + R
(resilience), plus the radio-specific controls below. Nothing ships with a MISSING item in this section.

### B1. Threat model
| Threat | What an attacker does | Impact | Required controls |
|---|---|---|---|
| Spoofed messages | Broadcasts fake "hard brake" or fake vehicles from a laptop/radio | Phantom alerts, panic braking, crashes | S-01, S-02, S-06 |
| Replay | Records real messages, re-broadcasts them later or elsewhere | Fake vehicles, confusion | S-03 |
| Sybil | One device pretends to be many vehicles | Fake traffic jam, fake hazards | S-04, S-06 |
| Relay flooding / DoS | Floods the channel or abuses relay to amplify | Real warnings lost | S-07, S-08 |
| Malformed packets | Crafted bytes crash the parser or run code | App crash or takeover | S-09, S-10 |
| Stolen app keys | Reverse-engineers the app to extract signing keys | Signs fake messages at scale | S-02, S-11, S-12 |
| Location tracking | Logs IDs/radio addresses to follow a person | Privacy harm, stalking | S-13, S-14, S-15 |
| Server compromise | Breaks into edge/cloud servers | Mass fake events, data leak | S-16 → S-21 |
| Man-in-the-middle | Intercepts phone ↔ server traffic | Fake events, data theft | S-17 |
| Account / store takeover | Steals developer or signing credentials | Malicious app update to all users | S-22, S-23 |
| Supply chain | Malicious dependency | Backdoor in every install | S-24 |

### B2. Controls
| ID | Control | Requirement | Verify by |
|---|---|---|---|
| S-01 | Signed messages | Every RSM signed with ECDSA P-256 (spec §10); unsigned or invalid → dropped silently. **The prototype's shared HMAC key must NOT exist in production code.** | Unit tests; grep for any hard-coded key |
| S-02 | Hardware-backed keys | Private keys generated and kept in Secure Enclave (iOS) / Android Keystore with StrongBox where available; non-exportable | Code review; device test |
| S-03 | Anti-replay | Reject if timestamp > 1 s old (heartbeat) / > 10 s (event), or (temp_id, msg_id) already seen; GPS-disciplined clock with skew check | Unit tests |
| S-04 | Certificate issuance limits | Server issues max 20 pseudonym certs/device/week; device proves it is genuine with Apple App Attest / Google Play Integrity before receiving certs | Server tests |
| S-05 | Revocation | Server publishes a revocation list (compact filter); phones refresh it daily and on each connection | Integration test |
| S-06 | Misbehaviour detection | Plausibility checks from spec §10 (speed, acceleration, position jumps, off-road, signal strength vs claimed distance); reports to server; repeated reports → revocation | Unit + simulation tests |
| S-07 | Relay limits | TTL, duplicate cache, back-off, direction filter, 1.5 km cut-off (spec §5); a relay never increases TTL | Unit tests |
| S-08 | Rate limits | Ignore any temp_id sending > 12 msg/s; cap total processed messages/s; drop lowest-priority first | Fuzz/load test |
| S-09 | Memory-safe parsing | RSM parsing only in Rust `core`; fixed-length checks before reading; no `unsafe` in the parser | Code review |
| S-10 | Fuzzing | `cargo-fuzz` targets for the RSM decoder and relay logic; 24 h fuzz run with zero crashes before each release | CI job |
| S-11 | No secrets in the app | No API keys, private keys or server passwords in the app binary or repo | Secret scanner (e.g. gitleaks) in CI |
| S-12 | Tamper resistance | Release builds obfuscated (R8 on Android; Swift symbols stripped); jailbreak/root detection **lowers trust** (receive-only, no relaying) rather than blocking | Build config review |
| S-13 | Radio privacy | BLE random resolvable addresses; temp_id and BLE address rotate together every 5 min, and only when the vehicle is stopped or at a junction, to break tracking | Device test with a BLE sniffer |
| S-14 | Data minimisation | No names, phones, plates broadcast or stored; own raw GPS trail deleted after 24 h; peer data deleted at track expiry + 10 min | Code review + unit tests |
| S-15 | No location in logs | Logs and crash reports never contain coordinates, temp_ids or keys | Grep + log review |
| S-16 | Server authentication | Every phone ↔ server call authenticated with device certificate (mutual TLS) + App Attest / Play Integrity token | Integration test |
| S-17 | Transport security | TLS 1.3 only; certificate pinning in the app (with a backup pin); no plain HTTP anywhere | Config review; MITM proxy test must fail |
| S-18 | Server hardening | Least-privilege cloud roles; private networks for databases; WAF + rate limiting on public endpoints; automatic OS/container patching | Infrastructure review |
| S-19 | CA key protection | Certificate-authority signing key in a cloud HSM/KMS, never on a normal server; 2-person rule for CA changes | Infrastructure review |
| S-20 | Data at rest | Server databases encrypted; phone database encrypted (SQLCipher or platform-encrypted storage) | Config review |
| S-21 | Monitoring | Alerts on abnormal certificate requests, misbehaviour-report spikes, API errors; audit logs kept 1 year | Dashboard review |
| S-22 | Developer accounts | Apple, Google, GitHub, cloud: hardware-key 2FA for every member; minimum people with admin | Account review |
| S-23 | App signing keys | Google Play App Signing enabled; upload key and Apple distribution certificate stored in a password manager/HSM, not on laptops | Account review |
| S-24 | Dependencies | Pinned versions + lockfiles; `cargo audit`, `flutter pub outdated`, Dependabot; SBOM generated per release | CI job |
| S-25 | Security testing | Static analysis (MobSF) per release; independent penetration test (app + radio + server) before public launch | Report on file |
| S-26 | Responsible disclosure | `SECURITY.md` + security contact email published; fix critical issues within 7 days | File exists |
| S-27 | Incident response | Written plan: who decides, how to revoke certs fleet-wide, how to push a forced update | Document exists |

---

## C. Store launch readiness

### C1. Apple App Store
| ID | Requirement | Notes |
|---|---|---|
| L-01 | Apple Developer Program account (organisation preferred) | Needed for TestFlight and release |
| L-02 | Privacy manifest `PrivacyInfo.xcprivacy` in the app and every SDK, with approved reasons for required-reason APIs | App Store Connect rejects uploads without it |
| L-03 | Purpose strings: `NSBluetoothAlwaysUsageDescription`, `NSLocationWhenInUseUsageDescription`, `NSLocationAlwaysAndWhenInUseUsageDescription` (only if background location is truly needed), `NSMotionUsageDescription` — in English and Khmer | Clear, specific wording |
| L-04 | Background modes declared only if used: `bluetooth-central`, `bluetooth-peripheral`, `location` | Reviewers check each is justified |
| L-05 | App Privacy ("nutrition label") answers match real data handling | Must match section B |
| L-06 | CarPlay Driving Task entitlement requested and granted | Apply early; app must work without it |
| L-07 | Safety disclaimer + no interaction while driving except glanceable alerts | App Review guidelines on driving safety |
| L-08 | TestFlight beta with real drivers before submission | |
| L-09 | Privacy policy URL + support URL; screenshots in English and Khmer | |

### C2. Google Play
| ID | Requirement | Notes |
|---|---|---|
| L-10 | `targetSdkVersion` 36 (Android 16) or higher | Required for new apps and updates from 31 Aug 2026 |
| L-11 | Foreground service types declared in manifest (`connectedDevice` for BLE, `location` if used) + matching `FOREGROUND_SERVICE_*` permissions + Play Console FGS declaration with a demo video | Required when targeting Android 14+ |
| L-12 | Precise / background location declaration in Play Console with justification | Location is core to safety function |
| L-13 | Data safety form matches real data handling | Must match section B |
| L-14 | Bluetooth permissions: `BLUETOOTH_SCAN` (with `neverForLocation` only if true), `BLUETOOTH_ADVERTISE`, `BLUETOOTH_CONNECT` | |
| L-15 | Closed testing track before production (new personal accounts must run a closed test before applying for production access; an organisation account avoids most of this) | Plan 2–3 weeks |
| L-16 | Play App Signing, R8 obfuscation, release AAB | |
| L-17 | Privacy policy URL; store listing + screenshots in English and Khmer | |

### C3. Both stores
| ID | Requirement |
|---|---|
| L-18 | Crash-free sessions ≥ 99.5% in beta |
| L-19 | Battery test: < 8%/hour in Drive mode on a low-end Android phone |
| L-20 | Legal review: terms ("driver assistance, not a guarantee"), privacy policy, Cambodian data rules, TRC approval for any radio dongle |

---

## D. Report format (produce this)

Write `docs/AUDIT_REPORT.md` with:

1. **Summary table:** counts of PASS / PARTIAL / MISSING per section (A, B, C).
2. **Full table:** `ID | Status | Evidence (file:line or test name) | Gap | Fix plan (hours)`.
3. **Top 10 risks** ordered by severity, security items first.
4. **Fix plan** as small tasks, each with the ID(s) it closes, in this order:
   all B items marked MISSING → A5 core logic → A1–A4 screens → A6 localisation → C items.

Rules for the audit:
- Do not mark PASS without a test or a file reference as evidence.
- Do not change code during the audit. Report first; fix only after the plan is approved.
