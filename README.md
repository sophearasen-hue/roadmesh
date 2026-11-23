# RoadMesh

Peer-to-peer road safety for every vehicle — cars, motorbikes, tuk-tuks — on any phone, in English and ខ្មែរ.

## What is in this folder

| Path | What it is | Status |
|---|---|---|
| `reference/phone-prototype/index.html` | The approved phone app design: Drive, Peers, Behaviour, Settings, CarPlay layout, live mesh simulation | Reference (open in a browser) |
| `reference/core-python/` | The safety engine built with Claude Code: prediction, right of way, alerts, relay, attack defences, behaviour recorder, 26 tests | Reference (security fixes applied, see GAP_REPORT §5) |
| `reference/console-app/index.html` | Scenario console: 5 scenarios, V2V on/off, in-car view, behaviour | Reference (open in a browser) |
| `docs/SPEC.md` | Technical specification v0.1 (Markdown export; the two diagrams are not included) | Reference for every step |
| `docs/GAP_REPORT.md` | How the two references compare + security findings | Read first |
| `docs/BUILD_PLAN.md` | 15 steps from here to App Store and Google Play, with the prompt for each | Follow in order |
| `docs/AUDIT.md` | 44 design checks, 27 security controls, 20 store checks | Used at every step |
| `CLAUDE.md` | Rules Claude Code reads at the start of every session | Keep at the top level |
| `SECURITY.md` | How to report a vulnerability | Publish with the app |
| `core/` | Rust safety core (pure logic, no I/O): RSM codec, LDM, prediction, alerts, behaviour | Step 2: reference port + SPEC v0.1 profile |
| `sim/` | Rust simulator: the 5 reference scenarios, golden tests, spec-profile tests | Step 2 (traffic mix in Step 4) |
| `server/` | Rust edge server | Skeleton (Step 9) |
| `docs/DECISIONS.md` | Decisions taken during the build | Updated each step |
| `app/` | Flutter app, strings in `lib/l10n/app_en.arb` / `app_km.arb` | Skeleton (Step 8) |
| `ios/`, `android/` | Swift / Kotlin radio, keys, CarPlay | Placeholders (Steps 5, 6, 10) |
| `.github/workflows/ci.yml` | CI: Rust fmt/clippy/test, cargo audit, Flutter analyze/test, gitleaks, reference guard + tests | Runs on every push |
| `scripts/check-reference.sh` | Fails if anything under `reference/` changes (checksums in `docs/reference.sha256`) | Runs in CI |

## Quick start

```bash
# run the reference engine and its tests
cd reference/core-python
python -m unittest discover -s tests -t .
python -m v2v compare blind_intersection
python -m v2v run ghost_attack --no-security

# Rust workspace and Flutter app (what CI runs)
cd ../..
cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked
(cd app && flutter pub get && flutter gen-l10n && flutter analyze --fatal-infos && flutter test)
scripts/check-reference.sh
python3 scripts/golden_from_reference.py --check   # golden outcomes still match the reference

# run a scenario through the Rust core (reference profile, or --spec for SPEC v0.1)
cargo run -q --release -p roadmesh-sim -- blind_intersection --spec

# then start building with Claude Code from the top folder
claude
```

First prompt for Claude Code: *"Read CLAUDE.md and docs/BUILD_PLAN.md. Start Step 1 in plan mode."*
