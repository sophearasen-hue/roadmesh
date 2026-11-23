"""Application layer: driving-behaviour recorder.

Every OBU runs one. It watches its own car and every peer it hears (direct or
relayed) and records, per car:

  * speed: current, average, maximum, and time spent over the local speed limit
  * harsh braking and harsh acceleration
  * overtaking: who passed whom, and whether it was done in the oncoming lane
  * tailgating: following closer than 1 s

Peers are keyed by their rotating pseudonym (temp ID), never by plate or name, so
a profile only covers the few minutes one pseudonym lives. Speed limits come from
the car's map (`speed_limit(x, y)`), as they would from a digital road map.
"""

import math
from dataclasses import dataclass
from typing import Callable

from .messages import BSM

SPEED_TOLERANCE = 1.10       # flag speeding above limit + 10 %
SPEEDING_MIN_S = 2.0         # ... sustained for this long
HARSH_BRAKE = -4.0           # m/s^2
HARSH_ACCEL = 2.5            # m/s^2
TAILGATE_HEADWAY_S = 1.0     # following closer than this (time gap)
TAILGATE_MIN_SPEED = 30 / 3.6
TAILGATE_MIN_S = 2.0
PAIR_RANGE_M = 150.0
MAX_AGE_S = 1.0              # ignore data older than this
SELF = "self"


@dataclass
class Profile:
    key: str
    first_seen: float
    last_seen: float = 0.0
    samples: int = 0
    speed_sum: float = 0.0
    max_speed: float = 0.0
    over_limit_s: float = 0.0
    harsh_brakes: int = 0
    harsh_accels: int = 0
    overtakes: int = 0           # passes this car made
    overtaken: int = 0           # times this car was passed
    oncoming_overtakes: int = 0  # passes made in the oncoming lane
    tailgate_s: float = 0.0

    def score(self) -> int:
        """Toy 0-100 behaviour score. Weights are illustrative, not a standard."""
        s = (100 - 1.0 * self.over_limit_s - 8 * self.harsh_brakes - 5 * self.harsh_accels
             - 1.5 * self.tailgate_s - 4 * self.oncoming_overtakes)
        return max(0, min(100, round(s)))

    def to_dict(self) -> dict:
        avg = self.speed_sum / self.samples if self.samples else 0.0
        return {"avg_kmh": round(avg * 3.6, 1), "max_kmh": round(self.max_speed * 3.6, 1),
                "over_limit_s": round(self.over_limit_s, 1), "harsh_brakes": self.harsh_brakes,
                "harsh_accels": self.harsh_accels, "overtakes": self.overtakes,
                "oncoming_overtakes": self.oncoming_overtakes, "overtaken": self.overtaken,
                "tailgate_s": round(self.tailgate_s, 1), "score": self.score(),
                "seen_s": round(self.last_seen - self.first_seen, 1)}


class BehaviorMonitor:
    def __init__(self, speed_limit: Callable[[float, float], float | None] | None = None):
        self.speed_limit = speed_limit or (lambda x, y: None)
        self.profiles: dict[str, Profile] = {}
        # (t, subject key, type, a, b, other key)
        self.events: list[tuple[float, str, str, float, float, str | None]] = []
        self._st: dict[str, dict] = {}
        self._pair: dict[tuple[str, str], dict] = {}

    def _event(self, t, key, kind, a=0.0, b=0.0, other=None):
        self.events.append((round(t, 2), key, kind, round(a, 1), round(b, 1), other))

    # ------------------------------------------------------------ per car
    def observe(self, now: float, dt: float, me: BSM, peers: dict[str, BSM]):
        cars = {SELF: me}
        cars.update({k: m for k, m in peers.items() if now - m.t <= MAX_AGE_S})
        for key, m in cars.items():
            self._car(now, dt, key, m)
        keys = sorted(cars)
        for i, a in enumerate(keys):
            for b in keys[i + 1:]:
                self._pair_update(now, dt, a, cars[a], b, cars[b])

    def _car(self, now, dt, key, m: BSM):
        p = self.profiles.get(key)
        if p is None:
            p = self.profiles[key] = Profile(key, now)
        st0 = self._st.get(key)
        if st0 and p.samples and now - p.last_seen > MAX_AGE_S:
            # lost sight of this car: close what was open at the moment we last saw it
            if st0["over_flag"]:
                self._event(p.last_seen, key, "speeding_end", p.last_seen - st0["over_since"], st0["ep_max"] * 3.6)
            st0.update(over_since=None, over_flag=False, braking=False, accel=False)
        p.last_seen, p.samples = now, p.samples + 1
        p.speed_sum += m.speed
        p.max_speed = max(p.max_speed, m.speed)
        st = self._st.setdefault(key, {"over_since": None, "over_flag": False, "ep_max": 0.0,
                                       "braking": False, "accel": False})
        limit = self.speed_limit(m.x, m.y)
        if limit and m.speed > limit * SPEED_TOLERANCE:
            if st["over_since"] is None:
                st["over_since"], st["ep_max"] = now, 0.0
            st["ep_max"] = max(st["ep_max"], m.speed)
            if now - st["over_since"] >= SPEEDING_MIN_S:
                p.over_limit_s += dt
                if not st["over_flag"]:
                    st["over_flag"] = True
                    self._event(now, key, "speeding", m.speed * 3.6, limit * 3.6)
        else:
            if st["over_flag"]:
                self._event(now, key, "speeding_end", now - st["over_since"], st["ep_max"] * 3.6)
            st["over_since"], st["over_flag"] = None, False

        if m.accel <= HARSH_BRAKE and not st["braking"]:
            st["braking"] = True
            p.harsh_brakes += 1
            self._event(now, key, "harsh_brake", -m.accel, m.speed * 3.6)
        elif m.accel > HARSH_BRAKE / 2:
            st["braking"] = False
        if m.accel >= HARSH_ACCEL and not st["accel"]:
            st["accel"] = True
            p.harsh_accels += 1
            self._event(now, key, "harsh_accel", m.accel, m.speed * 3.6)
        elif m.accel < HARSH_ACCEL / 2:
            st["accel"] = False

    # ------------------------------------------------------------ pairs
    def _pair_update(self, now, dt, ka, a: BSM, kb, b: BSM):
        if math.cos(a.heading - b.heading) < 0.9 or math.hypot(a.x - b.x, a.y - b.y) > PAIR_RANGE_M:
            self._pair.pop((ka, kb), None)
            return
        c, s = math.cos(a.heading), math.sin(a.heading)
        dx, dy = a.x - b.x, a.y - b.y
        along = dx * c + dy * s                 # > 0: a is ahead of b
        lateral = -dx * s + dy * c
        st = self._pair.get((ka, kb))
        if st is None:
            st = self._pair[(ka, kb)] = {"along": along, "t": now, "max_lat": 0.0,
                                         "tail_since": None, "tail_flag": False, "follower": None}
        if now - st["t"] > 0.5:                 # gap in observation: restart the pass tracking
            st["max_lat"] = 0.0
        if abs(along) < 30:
            st["max_lat"] = max(st["max_lat"], abs(lateral))
        prev = st["along"]
        if prev < 0 <= along or prev > 0 >= along:
            passer, passed = (ka, kb) if along >= 0 else (kb, ka)
            pm = a if passer == ka else b
            if st["max_lat"] >= 1.5:            # side by side, not a glitch
                # oncoming lane = passer was left of the passed car (left-hand side of travel)
                pd = self.profiles[passer]
                pd.overtakes += 1
                self.profiles[passed].overtaken += 1
                left = (lateral > 0) if passer == ka else (lateral < 0)
                if left:
                    pd.oncoming_overtakes += 1
                self._event(now, passer, "overtake", pm.speed * 3.6, 1.0 if left else 0.0, passed)
            st["max_lat"] = 0.0
        st["along"], st["t"] = along, now

        # tailgating: same lane, the one behind closer than 1 s
        if abs(lateral) < 1.5:
            follower, fm, leader = (kb, b, ka) if along > 0 else (ka, a, kb)
            gap = abs(along) - (a.length + b.length) / 2
            headway = gap / fm.speed if fm.speed > 0.1 else math.inf
            if fm.speed >= TAILGATE_MIN_SPEED and headway < TAILGATE_HEADWAY_S:
                if st["tail_since"] is None or st["follower"] != follower:
                    st["tail_since"], st["follower"], st["tail_flag"] = now, follower, False
                if now - st["tail_since"] >= TAILGATE_MIN_S:
                    self.profiles[follower].tailgate_s += dt
                    if not st["tail_flag"]:
                        st["tail_flag"] = True
                        self._event(now, follower, "tailgating", headway, fm.speed * 3.6, leader)
                return
        if st["tail_flag"]:
            self._event(now, st["follower"], "tailgating_end", now - st["tail_since"], 0.0)
        st["tail_since"], st["tail_flag"], st["follower"] = None, False, None
