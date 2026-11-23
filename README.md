# RoadMesh

Peer-to-peer road safety for every vehicle — cars, motorbikes, tuk-tuks — on any phone, in English and ខ្មែរ.

## What is in this folder

| Path | What it is | Status |
|---|---|---|
| `reference/phone-prototype/index.html` | The approved phone app design: Drive, Peers, Behaviour, Settings, CarPlay layout, live mesh simulation | Reference (open in a browser) |
| `reference/core-python/` | The safety engine built with Claude Code: prediction, right of way, alerts, relay, attack defences, behaviour recorder, 26 tests | Reference (security fixes applied, see GAP_REPORT §5) |
| `reference/console-app/index.html` | Scenario console: 5 scenarios, V2V on/off, in-car view, behaviour | Reference (open in a browser) |
| `docs/SPEC.md` | Technical specification | Export from the spec doc (Export → Markdown) |
| `docs/GAP_REPORT.md` | How the two references compare + security findings | Read first |
| `docs/BUILD_PLAN.md` | 15 steps from here to App Store and Google Play, with the prompt for each | Follow in order |
| `docs/AUDIT.md` | 44 design checks, 27 security controls, 20 store checks | Used at every step |
| `CLAUDE.md` | Rules Claude Code reads at the start of every session | Keep at the top level |
| `SECURITY.md` | How to report a vulnerability | Publish with the app |

## Quick start

```bash
# run the reference engine and its tests
cd reference/core-python
python -m unittest discover -s tests -t .
python -m v2v compare blind_intersection
python -m v2v run ghost_attack --no-security

# then start building with Claude Code from the top folder
cd ../..
claude
```

First prompt for Claude Code: *"Read CLAUDE.md and docs/BUILD_PLAN.md. Start Step 1 in plan mode."*
