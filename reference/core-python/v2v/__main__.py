"""Command line:

    python -m v2v list
    python -m v2v run blind_intersection            # V2V on
    python -m v2v run blind_intersection --off      # V2V off
    python -m v2v run emergency_brake --loss 0.5    # 50 % packet loss
    python -m v2v run ghost_attack --no-security    # signatures off: watch the attack work
    python -m v2v run highway_relay --no-relay      # direct radio only, no car-to-car relaying
    python -m v2v compare blind_intersection        # on vs off, side by side
    python -m v2v behavior mixed_traffic            # driving-behaviour report from CarA's recorder
    python -m v2v build-app                         # write app/index.html with every run baked in
"""

import argparse
import json
from pathlib import Path

from .scenarios import SCENARIOS

LOSS_LEVELS = (0.0, 0.3, 0.6)

# Which two runs each scenario compares side by side ({loss} = chosen packet loss).
OFF = {"key": "off", "label": "Without V2V", "short": "V2V off", "radio": False}
ON = {"key": "on-{loss}", "label": "With V2V", "short": "V2V on", "radio": True}
COMPARE = {
    "ghost_attack": [
        {"key": "on-0-nosec", "label": "Signature checks off", "short": "Checks off", "radio": True},
        {"key": "on-{loss}", "label": "Signature checks on", "short": "Checks on", "radio": True}],
    "highway_relay": [
        {"key": "direct-{loss}", "label": "Direct radio only (300 m)", "short": "Direct only", "radio": True},
        {"key": "on-{loss}", "label": "Peer-to-peer relay", "short": "Relay", "radio": True}],
}


def _outcome_line(s: dict, names=None) -> str:
    if s["collisions"]:
        return "Result: " + ", ".join(f"COLLISION {a}-{b} at t={t}s" for t, a, b in s["collisions"])
    speeds = ", ".join(f"{k} min {v:.0f} km/h" for k, v in s["min_speed_kmh"].items()
                       if names is None or k in names)
    gap = f", closest gap {s['min_gap_m']} m" if s["min_gap_m"] is not None else ""
    return f"Result: no collision ({speeds}{gap})"


def cmd_run(args):
    sc = SCENARIOS[args.scenario]
    print(f"\n=== {sc['title']}: V2V {'OFF' if args.off else 'ON'}"
          f"{f', loss {args.loss:.0%}' if args.loss else ''}"
          f"{', SECURITY OFF' if args.no_security else ''}"
          f"{', NO RELAY' if args.no_relay and not args.off else ''} ===")
    world = sc["build"](v2v=not args.off, loss=args.loss, security=not args.no_security,
                        relay=not args.no_relay, seed=args.seed, verbose=True)
    s = world.run()
    print("  " + _outcome_line(s, [v.name for v in world.vehicles if v.role == "car"]))
    if not args.off:
        r = s["radio"]
        print(f"  Radio: {r['sent']} sent, {r['relayed']} relayed, {r['delivered']} delivered, {r['lost']} lost"
              + (f"; revoked certs: {', '.join(s['revoked'])}" if s["revoked"] else ""))


def cmd_compare(args):
    for off in (False, True):
        args.off = off
        cmd_run(args)


EVENT_TEXT = {
    "speeding": lambda a, b, o: f"speeding: {a:.0f} km/h in a {b:.0f} zone",
    "speeding_end": lambda a, b, o: f"back under the limit after {a:.0f} s (peak {b:.0f} km/h)",
    "harsh_brake": lambda a, b, o: f"harsh braking {a:.1f} m/s² at {b:.0f} km/h",
    "harsh_accel": lambda a, b, o: f"harsh acceleration {a:.1f} m/s² at {b:.0f} km/h",
    "overtake": lambda a, b, o: f"overtook {o} at {a:.0f} km/h" + (" in the oncoming lane" if b else ""),
    "tailgating": lambda a, b, o: f"tailgating {o}: {a:.1f} s gap at {b:.0f} km/h",
    "tailgating_end": lambda a, b, o: f"stopped tailgating after {a:.0f} s",
}


def cmd_behavior(args):
    sc = SCENARIOS[args.scenario]
    w = sc["build"](seed=args.seed, loss=args.loss)
    w.run()
    rep = w.behavior_report()
    who = args.car or next(iter(rep))
    if who not in rep:
        raise SystemExit(f"no recorder for {who}; choose from {', '.join(rep)}")
    r = rep[who]
    print(f"\n=== {sc['title']}: what {who}'s recorder saw ===")
    for t, name, pseudo, kind, a, b, other in r["events"]:
        print(f"  t={t:5.1f}s  {name:10s} [{pseudo}]  {EVENT_TEXT[kind](a, b, other)}")
    print(f"\n  {'car':10s} {'avg':>5s} {'max':>5s} {'over':>5s} {'brk':>4s} {'acc':>4s} {'pass':>4s} {'tail':>5s} {'score':>5s}")
    for name, p in sorted(r["profiles"].items(), key=lambda kv: kv[1]["score"]):
        print(f"  {name:10s} {p['avg_kmh']:5.0f} {p['max_kmh']:5.0f} {p['over_limit_s']:4.0f}s "
              f"{p['harsh_brakes']:4d} {p['harsh_accels']:4d} {p['overtakes']:4d} {p['tailgate_s']:4.0f}s {p['score']:5d}")
    print("  (avg/max km/h, time over the limit, harsh brakes, harsh accelerations, overtakes, time tailgating)")


def export_runs(seed: int = 1) -> dict:
    """Run every scenario/variant and collect frames + events for the web app."""
    out = {}
    for key, sc in SCENARIOS.items():
        variants = {"off": dict(v2v=False)}
        for loss in LOSS_LEVELS:
            if key == "highway_relay" and loss > 0.3:
                continue                      # long run: keep the data small
            variants[f"on-{int(loss * 100)}"] = dict(v2v=True, loss=loss)
            if key == "highway_relay":
                variants[f"direct-{int(loss * 100)}"] = dict(v2v=True, loss=loss, relay=False)
        if key == "ghost_attack":
            variants["on-0-nosec"] = dict(v2v=True, security=False)
        runs = {}
        meta = None
        for vk, kw in variants.items():
            w = sc["build"](seed=seed, **kw)
            summary = w.run()
            if meta is None:
                meta = {
                    "vehicles": [{"name": v.name, "color": v.color, "length": v.length,
                                  "width": v.width, "role": v.role, "app": v.has_obu}
                                 for v in w.vehicles],
                    "radio_range": w.radio_range,
                    "zones": [[max(x0, -1e5), min(x1, 1e5), kmh, name] for x0, x1, kmh, name in w.zones],
                    "visibility": None if w.visibility == float("inf") else w.visibility,
                    "view": w.view,
                    "compare": COMPARE.get(key, [OFF, ON]),
                    "buildings": w.buildings,
                    "attackers": [{"name": a.name, "pos": a.pos} for a in w.attackers],
                    "watch": w.watch,
                }
            runs[vk] = {"frames": w.frames, "events": w.events, "summary": summary,
                        "behavior": w.behavior_report() if kw.get("v2v") else {},
                        "outcome": _outcome_line(summary, [v.name for v in w.vehicles if v.role == "car"])}
        out[key] = {"title": sc["title"], "summary": sc["summary"], **meta, "runs": runs}
    return out


def cmd_build_app(args):
    template = Path(__file__).with_name("app_template.html").read_text()
    data = json.dumps(export_runs(args.seed), separators=(",", ":"))
    if args.split:
        # page + runs.json side by side (the page fetches the data)
        out_dir = Path(args.split)
        out_dir.mkdir(parents=True, exist_ok=True)
        (out_dir / "index.html").write_text(template)
        (out_dir / "runs.json").write_text(data)
        print(f"wrote {out_dir}/index.html and {out_dir}/runs.json ({len(data) / 1024:.0f} KB)")
        return
    # one self-contained file that opens straight from disk
    html = ('<!doctype html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n'
            '<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">\n'
            '<style>body{margin:0}[hidden]{display:none!important}</style>\n</head>\n<body>\n'
            + template.replace("/*__V2V_DATA__*/null", data) + "\n</body>\n</html>\n")
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(html)
    print(f"wrote {out} ({len(html) / 1024:.0f} KB)")


def main(argv=None):
    p = argparse.ArgumentParser(prog="python -m v2v", description="V2V OBU simulator")
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list", help="list scenarios")
    for name in ("run", "compare"):
        sp = sub.add_parser(name)
        sp.add_argument("scenario", choices=SCENARIOS)
        sp.add_argument("--loss", type=float, default=0.0, help="packet loss 0..1")
        sp.add_argument("--no-security", action="store_true", help="disable signature checks")
        sp.add_argument("--no-relay", action="store_true", help="direct radio only, no multi-hop")
        sp.add_argument("--seed", type=int, default=1)
        if name == "run":
            sp.add_argument("--off", action="store_true", help="V2V off (human drivers only)")
    bh = sub.add_parser("behavior", help="driving-behaviour report from one car's recorder")
    bh.add_argument("scenario", choices=SCENARIOS)
    bh.add_argument("--car", help="whose recorder (default: first car)")
    bh.add_argument("--loss", type=float, default=0.0)
    bh.add_argument("--seed", type=int, default=1)
    b = sub.add_parser("build-app", help="bake all runs into the web app")
    b.add_argument("--out", default="app/index.html")
    b.add_argument("--seed", type=int, default=1)
    b.add_argument("--split", metavar="DIR", help="write DIR/index.html + DIR/runs.json instead")
    args = p.parse_args(argv)

    if args.cmd == "list":
        for k, sc in SCENARIOS.items():
            print(f"{k:20s} {sc['summary']}")
    elif args.cmd == "run":
        cmd_run(args)
    elif args.cmd == "compare":
        cmd_compare(args)
    elif args.cmd == "behavior":
        cmd_behavior(args)
    else:
        cmd_build_app(args)


if __name__ == "__main__":
    main()
