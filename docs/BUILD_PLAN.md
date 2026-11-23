# RoadMesh — Build plan: from references to App Store and Google Play

Give Claude Code **one step at a time**. Start each step in **plan mode**, approve the plan, then let it build.
Do not start the next step until the "Done when" check passes. Commit after every step.

Before step 1, put the doc spec in place: open the spec doc → Export → Markdown → save as `docs/SPEC.md`.

Legend: 🔒 = security gate. A step with 🔒 is not done until its AUDIT section B items are PASS.

---

## Phase 0 — Prove the core and the radio (weeks 1–6)

### Step 1 · Repository skeleton
> Read CLAUDE.md, docs/SPEC.md, docs/GAP_REPORT.md and docs/AUDIT.md. Create the production folders from SPEC section 11 (`core/`, `app/`, `ios/`, `android/`, `server/`, `sim/`) next to the existing `reference/` folder. Do not modify anything in `reference/`. Set up CI (GitHub Actions) that runs `cargo test`, `flutter test`, `flutter analyze`, gitleaks and `cargo audit` on every push.

**Done when:** CI is green on an empty skeleton.

### Step 2 · Rust safety core = port of the Python reference 🔒
> Port `reference/core-python/v2v` (messages, ldm, prediction, obu decision logic, behavior) to Rust in `core/`. Replace JSON with the 48-byte binary RSM from SPEC section 3. Keep every threshold in `core/src/config.rs`. Port all 26 Python tests, and add a golden test that runs the 5 scenarios and checks the same outcomes as the Python version (collision / no collision, who yields, minimum speeds within 1 km/h). The core must output alert levels only — never a brake command (GAP_REPORT F-5).

**Done when:** all ported tests and the 5 golden scenarios pass.

### Step 3 · Real cryptography 🔒
> Implement SPEC section 10 and AUDIT S-01 to S-06 in `core/` and `server/`: ECDSA P-256 signatures, pseudonym certificates (20 per device per week), revocation list, anti-replay, plausibility checks, flood guard, local block list and the 3-independent-reporters revocation rule from the reference. Write a fuzz target for the RSM decoder (cargo-fuzz). Remove any shared or hard-coded key.

**Done when:** signature, replay, flood and revocation tests pass; 1-hour fuzz run has zero crashes; gitleaks finds no secrets.

### Step 4 · Simulator with the Cambodian traffic mix
> Build `sim/` that drives the Rust core with the phone prototype's traffic: 40–120 vehicles, 55% motorbikes, cars, tuk-tuks, trucks, calm/normal/aggressive drivers, a city grid. Add GPS noise of 2–10 m and packet loss of 0–40%. Report missed conflicts, false alarms, mean warning lead time.

**Done when:** missed conflicts < 2% and the report is readable.

### Step 5 · Android Bluetooth LE transport 🔒
> Implement the Android transport: BLE extended advertising + scanning as a foreground service of type connectedDevice (and location), targetSdkVersion 36, random resolvable addresses rotated together with the pseudonym (AUDIT S-13). Connect it to the core through FFI.

**Done when:** two Android phones show each other with speed and distance; field test logs range and battery (Gate 1: ≥ 100 m at 90% delivery, < 8%/hour).

### Step 6 · iOS transport 🔒
> Implement the iOS transport with CoreBluetooth, keys in the Secure Enclave (AUDIT S-02), and confirm iPhone ↔ Android messaging.

**Done when:** mixed iPhone + Android phones see each other.

### Step 7 · Mesh relay
> Implement the relay rules from SPEC section 5 (TTL, duplicate cache, distance back-off, direction filter, 1.5 km cut-off, congestion control).

**Done when:** a message crosses 3 hops on real phones; no duplicate floods in the simulator.

**→ Gate 1 decision: range, battery and relay work on real roads. If not, switch the baseline to 4G + dongle before continuing.**

---

## Phase 1 — Build the app (weeks 7–16)

### Step 8 · Screens matching the phone prototype
> Build the Flutter app so it matches `reference/phone-prototype/index.html` screen by screen: Drive (map, risk colours, speed labels, mesh links, Nearby/Network, alert banner, HUD), Peers (list + detail with speed graph), Behaviour (score ring, stats, filters, timeline), Settings (language, radio, privacy, delete my data, motorbike mode). Use the Khmer strings from both references, merged into `app_en.arb` / `app_km.arb`. Bundle the Kantumruy Pro font. Lock screens other than Drive above 10 km/h. Demo controls (speed +/−, hazard demo, hop slider) exist only in the debug build.

**Done when:** AUDIT items P-01 to P-28 and P-40 to P-43 are PASS (golden screenshots in both languages).

### Step 9 · Edge server + cellular bridge 🔒
> Build `server/`: MQTT over TLS 1.3 with mutual TLS, App Attest / Play Integrity check before issuing certificates, rate limits, certificate authority keys in a cloud KMS/HSM, monitoring (AUDIT S-16 to S-21).

**Done when:** phones out of radio range still receive hazard events; a man-in-the-middle proxy test fails as expected.

### Step 10 · CarPlay Driving Task screen
> Add the CarPlay scene per SPEC section 9 (list/grid template: current alert, speed vs limit, nearest 3–5 vehicles, mute). Apply for the CarPlay Driving Task entitlement at the same time.

**Done when:** it works in Xcode's CarPlay simulator.

### Step 11 · Hardening 🔒
> Run the full AUDIT section B. Fix every MISSING item. Add R8 obfuscation, stripped Swift symbols, jailbreak/root detection that lowers trust (receive-only), encrypted local database, location-free logs, SBOM, SECURITY.md contact.

**Done when:** `docs/AUDIT_REPORT.md` shows section B all PASS; MobSF scan has no high findings.

---

## Phase 2 — Pilot (weeks 17–28)

### Step 12 · Closed beta with a fleet
- TestFlight (iOS) + Google Play closed testing track (needed for new personal accounts; an organisation account is smoother)
- 200–500 vehicles in one Phnom Penh district
- Drivers rate alerts after each trip → tune thresholds in `core/src/config.rs`

**Gate 3:** false Danger alerts < 1 per 100 km, crash-free sessions ≥ 99.5%, battery < 8%/hour.

### Step 13 · Independent penetration test 🔒
Hire an outside security firm to test the app, radio protocol and server (AUDIT S-25). Fix findings before launch.

---

## Phase 3 — Store launch (weeks 29–36)

### Step 14 · Store submission checklist (AUDIT section C)
> Prepare store submissions: iOS privacy manifest (PrivacyInfo.xcprivacy), purpose strings in English and Khmer, App Privacy answers, background modes justified; Android target API 36, foreground service declarations with demo video, location declaration, Data safety form, Play App Signing. Privacy policy and support pages in both languages. Screenshots in both languages.

**Done when:** AUDIT section C all PASS.

### Step 15 · Submit
- Apple: App Store Connect → submit for review (allow 1–2 weeks with CarPlay)
- Google: promote from closed testing to production

---

## Who does what

| Task | Claude Code | You / developer |
|---|---|---|
| Write and test code, steps 1–11, 14 | ✔ | Approve plans, review |
| Run on real phones, field tests (5–7, 12) | | ✔ |
| Apple / Google developer accounts, CarPlay entitlement | | ✔ |
| Khmer string and voice review | Drafts | ✔ native speaker |
| Penetration test, legal review, TRC radio approval | | ✔ external |
