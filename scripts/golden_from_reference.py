#!/usr/bin/env python3
"""Run the READ-ONLY Python reference and write the golden outcomes the Rust
core must reproduce: sim/tests/golden/reference.json.

    python3 scripts/golden_from_reference.py          # write the file
    python3 scripts/golden_from_reference.py --check  # fail if it is out of date (CI)

Nothing under reference/ is modified: the reference package is imported and its
worlds are stepped from outside, recording state transitions as they happen.
"""
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "reference", "core-python"))
sys.dont_write_bytecode = True  # keep reference/ free of __pycache__

from v2v.scenarios import SCENARIOS  # noqa: E402

OUT = os.path.join(ROOT, "sim", "tests", "golden", "reference.json")

RUNS = [
    ("blind_intersection", "off", dict(v2v=False)),
    ("blind_intersection", "on", dict(v2v=True)),
    ("emergency_brake", "off", dict(v2v=False)),
    ("emergency_brake", "on", dict(v2v=True)),
    ("ghost_attack", "off", dict(v2v=False)),
    ("ghost_attack", "on", dict(v2v=True)),
    ("ghost_attack", "nosec", dict(v2v=True, security=False)),
    ("mixed_traffic", "off", dict(v2v=False)),
    ("mixed_traffic", "on", dict(v2v=True)),
    ("highway_relay", "off", dict(v2v=False)),
    ("highway_relay", "on", dict(v2v=True)),
    ("highway_relay", "direct", dict(v2v=True, relay=False)),
]


def run(name, kw):
    w = SCENARIOS[name]["build"](**kw)
    prev = {v.name: "CRUISE" for v in w.vehicles}
    yields, hazards = [], []
    while w.t < w.duration - 1e-9:
        now = w.t
        w.step()
        for v in w.vehicles:
            if not v.obu:
                continue
            st = v.obu.state
            if st == "YIELD" and prev[v.name] != "YIELD":
                yields.append({"t": round(now, 2), "car": v.name,
                               "target": w._cert_owner.get(v.obu.yield_to, "?")})
            elif st == "ADVISE" and prev[v.name] != "ADVISE" and v.obu.hazard:
                hazards.append({"t": round(now, 2), "car": v.name,
                                "gap": round(v.obu.hazard[0], 1), "hops": v.obu.hazard[2]})
            prev[v.name] = st
    s = w.summary()
    cars = [v.name for v in w.vehicles if v.role == "car"]
    out = {
        "collisions": [{"t": t, "a": a, "b": b} for t, a, b in s["collisions"]],
        "min_speed_kmh": {n: s["min_speed_kmh"][n] for n in cars},
        "max_brake_ms2": {n: s["max_brake_ms2"][n] for n in cars if n in s["max_brake_ms2"]},
        "yields": yields,
        "hazards": hazards,
        "revoked": len(s["revoked"]),
        "blocked_by": {v.name: len(v.obu.blocked) for v in w.vehicles if v.obu},
    }
    if kw.get("v2v") and name == "mixed_traffic":
        rep = w.behavior_report()["CarA"]["profiles"]
        keys = ["over_limit_s", "harsh_brakes", "harsh_accels", "overtakes",
                "oncoming_overtakes", "overtaken", "tailgate_s", "score"]
        out["behaviour_carA"] = {n: {k: p[k] for k in keys} for n, p in sorted(rep.items())}
    return out


def main():
    data = {f"{name}/{variant}": run(name, kw) for name, variant, kw in RUNS}
    text = json.dumps(data, indent=1, sort_keys=True) + "\n"
    if "--check" in sys.argv:
        with open(OUT) as f:
            if f.read() != text:
                sys.exit("golden file is out of date: run scripts/golden_from_reference.py")
        print("golden file up to date")
        return
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w") as f:
        f.write(text)
    print(f"wrote {OUT} ({len(data)} runs)")


if __name__ == "__main__":
    main()
