"""The simulated world: vehicles, human drivers, buildings, attackers and the main loop."""

import math
import random
from dataclasses import dataclass, field
from typing import Callable

from . import config
from .geometry import box, polygons_overlap, segment_hits_polygon
from .messages import BSM
from .obu import OBU
from .prediction import first_conflict, trajectory
from .radio import Channel, Frame
from .security import CertificateAuthority, Credential, Packet, Signer


@dataclass
class Maneuver:
    """Scripted driver action, e.g. the lead car slamming the brakes."""
    t0: float
    t1: float = math.inf
    accel: float | None = None
    yaw_rate: float | None = None


@dataclass
class Vehicle:
    name: str
    x: float
    y: float
    heading: float
    speed: float
    target_speed: float | None = None
    length: float = 4.5
    width: float = 1.8
    has_obu: bool = True
    color: str = "#4c8bf5"
    role: str = "car"            # car | relay (background traffic) | hazard
    respects_limit: bool = True  # human driver keeps to the posted speed limit
    overtakes: bool = False      # human driver overtakes slower traffic via the oncoming lane
    headway_s: float | None = None  # car-following time gap; None = no following model
    lane_y: float | None = None  # lateral position the driver is steering to
    base_heading: float | None = None
    maneuvers: list[Maneuver] = field(default_factory=list)
    accel: float = 0.0
    yaw_rate: float = 0.0
    crashed: bool = False
    obu: OBU | None = None
    driver: "HumanDriver | None" = None

    def __post_init__(self):
        if self.target_speed is None:
            self.target_speed = self.speed
        self.desired_speed = self.target_speed
        if self.base_heading is None:
            self.base_heading = self.heading
        self.home_y = self.y
        self.passing = False

    def footprint(self, margin: float = 0.0):
        return box(self.x, self.y, self.heading, self.length, self.width, margin)

    def active_maneuver(self, now: float):
        return next((m for m in self.maneuvers if m.t0 <= now < m.t1), None)

    def move(self, dt: float):
        if self.crashed:
            self.speed = self.accel = self.yaw_rate = 0.0
            return
        self.speed = max(0.0, self.speed + self.accel * dt)
        if self.lane_y is not None and self.speed > 0.5:
            # lane change: steer toward lane_y (lateral speed up to 1.8 m/s)
            off = self.lane_y - (self.y if abs(math.sin(self.base_heading)) < .5 else self.x)
            vy = max(-1.8, min(1.8, off / 0.8)) if abs(off) > 0.05 else 0.0
            self.heading = self.base_heading + math.atan2(vy, self.speed)
        elif self.speed > 0:
            self.heading += self.yaw_rate * dt
        self.x += self.speed * math.cos(self.heading) * dt
        self.y += self.speed * math.sin(self.heading) * dt


class HumanDriver:
    """Sees only what's in line of sight, reacts after HUMAN_REACTION_S, then brakes hard."""

    def __init__(self, reaction: float = config.HUMAN_REACTION_S):
        self.reaction = reaction
        self.hazard_since: float | None = None
        self.braking_until = -math.inf

    def accel(self, v: Vehicle, world: "World", now: float) -> float:
        ego = trajectory(v.x, v.y, v.heading, v.speed, 0.0, v.yaw_rate,
                         horizon=config.HUMAN_HAZARD_TTC_S)
        hazard = False
        for o in world.vehicles:
            if o is v or not world.line_of_sight(v, o):
                continue
            tr = trajectory(o.x, o.y, o.heading, o.speed, o.accel, o.yaw_rate,
                            horizon=config.HUMAN_HAZARD_TTC_S)
            if first_conflict(ego, (v.length, v.width), tr, (o.length, o.width)) is not None:
                hazard = True
                break
        if hazard:
            if self.hazard_since is None:
                self.hazard_since = now
            if now - self.hazard_since >= self.reaction - 1e-9:
                self.braking_until = now + 1.0
        else:
            self.hazard_since = None
        if now < self.braking_until:
            return config.MAX_BRAKE
        if v.speed < v.target_speed - 0.05:
            a = min(config.CRUISE_ACCEL, (v.target_speed - v.speed) / config.TICK)
        elif v.speed > v.target_speed + 0.05:
            a = -max(1.0, min(2.5, (v.speed - v.target_speed) / 2))
        else:
            a = 0.0
        if v.headway_s is not None:            # smooth car-following (keeps a time gap)
            lead = self._leader(v, world)
            if lead:
                o, gap = lead
                want = 3.0 + v.speed * v.headway_s
                a = min(a, max(-3.5, 0.4 * (gap - want) + 0.8 * (o.speed - v.speed)))
        return a

    @staticmethod
    def _leader(v: Vehicle, world: "World"):
        c, s = math.cos(v.base_heading), math.sin(v.base_heading)
        best = None
        for o in world.vehicles:
            if o is v or math.cos(o.heading - v.base_heading) < .9 or not world.line_of_sight(v, o):
                continue
            dx, dy = o.x - v.x, o.y - v.y
            along, lat = dx * c + dy * s, -dx * s + dy * c
            lane = (v.lane_y if v.lane_y is not None else v.home_y) - v.y
            if 0 < along < 150 and abs(lat - lane) < 1.6:
                gap = along - (v.length + o.length) / 2
                if best is None or gap < best[1]:
                    best = (o, gap)
        return best


class GhostAttacker:
    """Roadside attacker broadcasting a car that does not exist.

    credential=None  -> outsider: signs with a made-up key (forgery)
    credential=valid -> insider: holds a real certificate (e.g. extracted from a car)
    """

    def __init__(self, name: str, pos: tuple[float, float], start: tuple[float, float],
                 heading: float, speed: float, t_start: float, t_end: float = math.inf,
                 credential: Credential | None = None, rng: random.Random | None = None):
        self.name, self.pos = name, pos
        self.start, self.heading, self.speed = start, heading, speed
        self.t_start, self.t_end = t_start, t_end
        rng = rng or random.Random(0)
        self.credential = credential or Credential(f"{rng.getrandbits(32):08x}", rng.randbytes(32))

    def ghost_pose(self, now: float):
        if not (self.t_start <= now < self.t_end):
            return None
        d = self.speed * (now - self.t_start)
        return (self.start[0] + d * math.cos(self.heading),
                self.start[1] + d * math.sin(self.heading), self.heading)

    def packets(self, now: float) -> list[Frame]:
        pose = self.ghost_pose(now)
        if pose is None:
            return []
        msg = BSM(temp_id=self.credential.cert_id, t=round(now, 2), x=round(pose[0], 3),
                  y=round(pose[1], 3), speed=self.speed, heading=self.heading,
                  yaw_rate=0.0, accel=0.0, brake_on=False)
        return [Frame(Signer.sign(msg, self.credential), 0, self.pos)]


class World:
    def __init__(self, vehicles: list[Vehicle], buildings=(), v2v: bool = True,
                 security: bool = True, loss: float = 0.0, seed: int = 1,
                 attackers_factory: Callable | None = None, watch: tuple[str, str] | None = None,
                 duration: float = 12.0, verbose: bool = False, relay: bool = True,
                 radio_range: float = config.RADIO_RANGE_M, visibility: float = math.inf,
                 view: tuple[float, float, float, float] | None = None, record_every: int = 1,
                 zones: list[tuple[float, float, float, str]] | None = None):
        self.rng = random.Random(seed)
        self.t = 0.0
        self.duration = duration
        self.vehicles = vehicles
        self.buildings = list(buildings)
        self.v2v = v2v
        self.relay = relay
        self.verbose = verbose
        self.radio_range = radio_range
        self.zones = zones or []           # (x0, x1, limit km/h, road name) along the road
        self.visibility = visibility      # how far a human driver can see (fog)
        self.view = view
        self.record_every = record_every
        self._tick = 0
        self._index = {v.name: i for i, v in enumerate(vehicles)}
        self.ca = CertificateAuthority(random.Random(seed + 1000))
        self.channel = Channel(random.Random(seed + 2000), radio_range, loss)
        self.events: list[tuple[float, str, str, str]] = []
        self.frames: list[dict] = []
        self.collisions: list[tuple[float, str, str]] = []
        self.watch = watch
        self._seen = False
        self.min_gap = math.inf
        self._cert_owner: dict[str, str] = {}

        for v in vehicles:
            v.driver = HumanDriver()
            if v2v and v.has_obu:
                creds = self.ca.issue(3)
                for c in creds:
                    self._cert_owner[c.cert_id] = v.name
                v.obu = OBU(v, self.ca, Signer(creds), security=security,
                            log=self.log, describe=self.describe, relay=relay,
                            speed_limit=self.speed_limit)
        self.attackers = attackers_factory(self) if attackers_factory else []
        for a in self.attackers:
            self._cert_owner[a.credential.cert_id] = f"ghost from {a.name}"

    # ------------------------------------------------------------ helpers
    def describe(self, cert_id: str) -> str:
        owner = self._cert_owner.get(cert_id)
        return f"{owner} [{cert_id}]" if owner else cert_id

    def log(self, t: float, source: str, kind: str, text: str):
        self.events.append((round(t, 2), source, kind, text))
        if self.verbose:
            print(f"  t={t:4.1f}s  [{source}] {text}")

    def line_of_sight(self, a: Vehicle, b: Vehicle) -> bool:
        p, q = (a.x, a.y), (b.x, b.y)
        if math.hypot(a.x - b.x, a.y - b.y) > self.visibility:
            return False
        if any(segment_hits_polygon(p, q, poly) for poly in self.buildings):
            return False
        return not any(segment_hits_polygon(p, q, o.footprint())
                       for o in self.vehicles if o is not a and o is not b)

    def speed_limit(self, x: float, y: float = 0.0):
        """Posted limit (m/s) at a position, from the map. None = no limit known."""
        for x0, x1, kmh, _ in self.zones:
            if x0 <= x < x1:
                return kmh / 3.6
        return None

    def _set_targets(self, v: Vehicle):
        """Speed the human wants now: own preference, capped by the limit here and just ahead."""
        target = v.desired_speed
        if v.respects_limit and self.zones:
            look = v.speed * 6.0 * math.cos(v.base_heading)
            for x in (v.x, v.x + look):
                lim = self.speed_limit(x)
                if lim is not None:
                    target = min(target, lim)
        v.target_speed = target
        if v.overtakes:
            self._overtake_logic(v)

    def _overtake_logic(self, v: Vehicle):
        """Pull out past slower traffic when the oncoming lane is clear; return when there's room."""
        c = math.cos(v.base_heading)
        pass_y = v.home_y + 3.5 * (1 if c > 0 else -1)
        def in_lane(o, y):
            return abs(o.y - y) < 1.6
        def along(o):
            return (o.x - v.x) * c
        if not v.passing:
            leaders = [o for o in self.vehicles if o is not v and in_lane(o, v.home_y)
                       and math.cos(o.heading - v.base_heading) > .9 and 0 < along(o) < 70]
            slow = [o for o in leaders if o.speed < v.desired_speed - 2]
            oncoming = [o for o in self.vehicles if in_lane(o, pass_y)
                        and math.cos(o.heading - v.base_heading) < -.9 and -10 < along(o) < 600]
            if slow and not oncoming:
                v.passing, v.lane_y = True, pass_y
                v.target_speed = v.desired_speed
        else:
            v.target_speed = v.desired_speed          # finish the pass briskly
            # stay out until the whole slow line is passed: no gap-cutting
            blocked = [o for o in self.vehicles if o is not v and in_lane(o, v.home_y)
                       and (-20 < along(o) < 25 or (0 < along(o) < 70 and o.speed < v.speed - 2))]
            if not blocked:
                v.passing, v.lane_y = False, v.home_y

    def by_name(self, name: str) -> Vehicle:
        return next(v for v in self.vehicles if v.name == name)

    # ------------------------------------------------------------ main loop
    def step(self):
        now = self.t
        if self.v2v:
            obus = {v.name: ((v.x, v.y), v.obu) for v in self.vehicles if v.obu and not v.crashed}
            for v in self.vehicles:
                if v.obu and not v.crashed:
                    for fr in v.obu.flush_relays():   # what it heard last tick, forwarded
                        self.channel.broadcast(v.name, (v.x, v.y), fr)
                    self.channel.broadcast(v.name, (v.x, v.y), v.obu.transmit(now))
            for att in self.attackers:
                for pkt in att.packets(now):
                    self.channel.broadcast(att.name, att.pos, pkt)
            self.channel.deliver(obus, now)

        if self.watch and not self._seen:
            a, b = self.by_name(self.watch[0]), self.by_name(self.watch[1])
            if self.line_of_sight(a, b):
                self._seen = True
                gap = math.hypot(a.x - b.x, a.y - b.y)
                self.log(now, "eyes", "sight", f"{a.name} and {b.name} can finally SEE each other, {gap:.0f} m apart")

        for v in self.vehicles:
            if not v.crashed:
                self._set_targets(v)
        for v in self.vehicles:
            if v.crashed:
                continue
            cap = v.obu.decide(now) if v.obu else None
            m = v.active_maneuver(now)
            if m and m.accel is not None:
                a = m.accel                       # scripted: the driver did this on purpose
            else:
                a = v.driver.accel(v, self, now)
                if cap is not None:
                    a = min(a, cap)
            v.accel = max(config.MAX_BRAKE, min(3.0, a))
            v.yaw_rate = m.yaw_rate if (m and m.yaw_rate is not None) else 0.0

        for v in self.vehicles:
            v.move(config.TICK)

        for i, a in enumerate(self.vehicles):
            for b in self.vehicles[i + 1:]:
                if (a.crashed and b.crashed) or math.hypot(a.x - b.x, a.y - b.y) > 8:
                    continue
                if polygons_overlap(a.footprint(), b.footprint()):
                    a.crashed = b.crashed = True
                    self.collisions.append((round(now, 2), a.name, b.name))
                    self.log(now, "world", "collision", f"*** COLLISION: {a.name} hit {b.name} ***")

        if self.watch:
            a, b = self.by_name(self.watch[0]), self.by_name(self.watch[1])
            self.min_gap = min(self.min_gap, math.hypot(a.x - b.x, a.y - b.y))
        if self._tick % self.record_every == 0:
            self._record(now)
        self._tick += 1
        self.t = round(now + config.TICK, 6)

    def run(self):
        while self.t < self.duration - 1e-9:
            self.step()
        return self.summary()

    # ------------------------------------------------------------ output
    def _record(self, now: float):
        cars = []
        for v in self.vehicles:
            st = v.obu.state if v.obu else "OFF"
            ttc = v.obu.worst_ttc if v.obu else None
            cars.append([round(v.x, 2), round(v.y, 2), round(v.heading, 3), round(v.speed, 2),
                         round(v.accel, 2), st, int(v.crashed),
                         None if ttc is None else round(ttc, 1)])
        ghosts = []
        for att in self.attackers:
            pose = att.ghost_pose(now)
            if pose:
                cid = att.credential.cert_id
                believed = any(cid in v.obu.ldm.entries for v in self.vehicles if v.obu)
                ghosts.append([round(pose[0], 2), round(pose[1], 2), round(pose[2], 3), int(believed)])
        seen = None
        if self.watch:
            seen = int(self.line_of_sight(self.by_name(self.watch[0]), self.by_name(self.watch[1])))
        # What each car's app knows about every other car: "-" itself, "." unknown,
        # "0" heard directly, "n" via n relays (capped at 9).
        known = []
        for v in self.vehicles:
            if not v.obu:
                known.append(None)
                continue
            row = ["."] * len(self.vehicles)
            row[self._index[v.name]] = "-"
            for tid in v.obu.ldm.entries:
                j = self._index.get(self._cert_owner.get(tid, ""))
                if j is not None:
                    row[j] = str(min(9, v.obu.ldm.hops.get(tid, 0)))
            known.append("".join(row))
        self.frames.append({"t": round(now, 2), "c": cars, "g": ghosts, "s": seen,
                            "k": known if self.v2v else None})

    def behavior_report(self) -> dict:
        """What each car's recorder saw. Peers are pseudonyms; the simulator also knows
        which vehicle owned each pseudonym, so the demo can show names next to them."""
        out = {}
        for v in self.vehicles:
            if not v.obu or v.role != "car":
                continue
            mon = v.obu.behavior
            def name(key):
                return v.name if key == "self" else self._cert_owner.get(key, key)
            out[v.name] = {
                "events": [[t, name(k), "self" if k == "self" else k, kind, a, b,
                            name(o) if o else None] for t, k, kind, a, b, o in sorted(mon.events, key=lambda e: e[0])],
                "profiles": {name(k): {"pseudonym": k, **p.to_dict()} for k, p in mon.profiles.items()},
            }
        return out

    def summary(self) -> dict:
        return {
            "collisions": self.collisions,
            "min_gap_m": None if self.min_gap == math.inf else round(self.min_gap, 1),
            "min_speed_kmh": {v.name: round(min(f["c"][i][3] for f in self.frames) * 3.6, 1)
                              for i, v in enumerate(self.vehicles)},
            "radio": dict(self.channel.stats),
            "max_brake_ms2": {v.name: round(-min(0.0, min(f["c"][i][4] for f in self.frames)), 1)
                              for i, v in enumerate(self.vehicles) if not v.crashed},
            "revoked": sorted(self.ca.revoked),
        }
