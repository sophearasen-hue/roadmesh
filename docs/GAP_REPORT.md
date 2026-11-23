# RoadMesh — Gap report: Claude Code build vs. phone prototype

Reviewed 2026-11-22. Inputs:
- `reference/core-python/` + `reference/console-app/` — what Claude Code built (Python 3, 26 tests, scenario console)
- `reference/phone-prototype/index.html` — the approved phone app prototype (Drive / Peers / Behaviour / Settings, EN + ខ្មែរ, CarPlay layout)

## 1. Verdict in one line

Claude Code built a **stronger safety engine** than the prototype, but it is a **scripted demo console, not the phone app**.
Neither is shippable to the stores yet. Keep both as references: the Python engine defines *behaviour*, the prototype defines *screens*.
The production app (Flutter + native radio + Rust core) is still to be built — see `BUILD_PLAN.md`.

## 2. What each one does better

| Area | Claude Code build | Phone prototype | Keep from |
|---|---|---|---|
| Collision prediction | Rotated-box overlap, turn rate + acceleration, post-encroachment buffer | Simple point projection | **Claude Code** |
| Braking decision | Gentlest safe braking, fail-safe yield when contact is lost | Fixed brake levels | **Claude Code** (as alert logic) |
| Right of way | Ahead / oncoming / turning / from-the-right rules | From-the-right only | **Claude Code** |
| Hazard far ahead via relay | Yes (stopped traffic 1.5 km away, 5 relays) | No | **Claude Code** |
| Attack scenarios | Forged, insider, revocation, `--no-security` comparison | None | **Claude Code** |
| Behaviour detection | Speeding (zone limits), harsh brake/accel, overtaking (incl. oncoming lane), **tailgating** | Speeding, harsh brake/accel, overtaking | **Claude Code** rules + prototype scoring bands |
| Automated tests | 26 unit/scenario tests | None | **Claude Code** |
| Phone app structure (4 tabs) | No — desktop console with Compare / In the car / Behaviour views | Yes | **Prototype** |
| Live free-running traffic | No — 5 fixed scenarios replayed | Yes — 40–120 vehicles, city grid, endless | **Prototype** |
| Vehicle types (motorbike, tuk-tuk, truck) | Cars only | Yes, Cambodian traffic mix | **Prototype** |
| Peers list + peer detail drill-down | Partial (driver cards in Behaviour view) | Yes, sorted list, detail, speed graph, events | **Prototype** |
| Mesh links drawn by hop count | No | Yes | **Prototype** |
| Settings (radio type, hops, density, privacy, language) | Language + packet loss only | Yes | **Prototype** |
| CarPlay layout | No | Yes | **Prototype** |
| Risk bands Safe / Caution / Risky with colours | Score bar only | Yes | **Prototype** |
| Trip score ring, distance, avg/top speed, event filters | Partial | Yes | **Prototype** |
| Khmer translation | Yes, very complete (incl. event log) | Yes | Both — merge string sets |

## 3. Prototype parity (AUDIT.md section A) — status of the Claude Code build

| IDs | Status | Note |
|---|---|---|
| P-01 … P-11 Drive screen | **PARTIAL** | Map + alert + speed exist in "In the car"; no phone layout, no risk colours, no mesh links, no Nearby/Network toggle |
| P-12 … P-16 Peers screen | **MISSING** | Only a short "on the app network" list |
| P-17 … P-22 Behaviour screen | **PARTIAL** | Good driver cards + speed chart; no trip ring, no event filter |
| P-23 … P-28 Settings | **MISSING** | Except language switch |
| P-29 … P-39 Core logic | **PASS (stronger)** | All alert types present except "fast vehicle from behind" and "overtaking you" as *alerts* (detected as behaviour only) |
| P-40 … P-44 Localisation | **PARTIAL** | Strings are inline in HTML/JS, not in resource files; needs native-speaker review |

## 4. Security findings in the Claude Code build

| # | Finding | Severity | Status |
|---|---|---|---|
| F-1 | **One report revoked a certificate** (`REVOKE_AFTER_REPORTS = 1`). A single malicious app could get honest cars revoked. | High | **Fixed here**: CA now needs 3 independent reporters; the reporting car blocks the sender locally at once. Tests added. |
| F-2 | **No flood guard**: a sender with a valid certificate could send 100+ messages per second. | Medium | **Fixed here**: messages from one pseudonym faster than 20/s are treated as misbehaviour. Test added. |
| F-3 | Signatures are **HMAC with keys looked up at the CA**. A real phone cannot ask a server for every peer's key, and a shared-key design can be extracted from the app. | Critical for production | **By design in the simulator** (documented). Production must use ECDSA P-256 + pseudonym certificates (AUDIT S-01, S-02). |
| F-4 | Random numbers are **seeded, non-cryptographic** (for repeatable demos). | Critical for production | Simulator only. Production uses OS secure random / hardware keys. |
| F-5 | **Auto-brake output**: the engine returns an acceleration cap that brakes the car. A phone app cannot and must not control brakes. | Product/legal | Keep in simulator. In the app, map YIELD/BRAKE states to **Danger alerts** only (spec §1 non-goals). |
| F-6 | Messages are **JSON** (~250 bytes). Too large for one Bluetooth LE advertisement. | Medium | Production uses the 48-byte binary RSM (spec §3). |
| F-7 | Relay hop count is **unsigned** (by design, like ETSI). An attacker can lower it to stop spread, or replay with hop 0. | Low | Duplicate check already prevents loops. Production adds the relay rules in spec §5 and rate limits. |
| F-8 | Peer behaviour profiles are kept by pseudonym with **no retention limit**. | Medium (privacy) | Production: delete at track expiry + 10 min (spec §10, AUDIT S-14). |

## 5. What I changed in `reference/core-python`

- `config.py`: `REVOKE_AFTER_REPORTS = 3`, new `MIN_MSG_INTERVAL_S = 0.05`
- `ldm.py`: flood guard ("message rate too high")
- `obu.py`: local block list — a car that catches a misbehaving sender ignores it immediately
- `tests/test_v2v.py`: updated revocation test; new tests `test_revocation_needs_independent_reporters`, `test_message_flood_is_rejected`
- Wording in `README.md`, `scenarios.py`, `app_template.html`; console app rebuilt
- Result: **26 of 26 tests pass**; all scenario outcomes unchanged (blind junction, brake chain, relay and attack scenarios behave as before)

## 6. Next step

Follow `BUILD_PLAN.md`. The first task ports the Python engine to the Rust core and proves it gives the same results on all 5 scenarios.
