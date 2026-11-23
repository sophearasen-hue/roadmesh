"""The on-board unit: ties the layers together and runs the safety applications.

    build BSM -> sign -> (radio) -> verify -> plausibility -> LDM
    -> predict every neighbour -> find conflicts -> right of way -> brake demand + HMI

Applications:
  * Intersection Movement Assist (IMA): crossing-path conflicts, yield to the right
  * Forward Collision Warning / auto-brake: car ahead on the same path
  * Electronic Emergency Brake Light (EEBL): hard braking ahead, even if not visible
  * Hazard ahead: stopped traffic far down the road, learned through relayed messages

Every OBU is also a relay: it forwards the signed messages it hears (multi-hop), so
information travels well beyond one radio range. Collision avoidance only trusts
fresh, nearby data; the hazard advisory may use older data from far away.
"""

import math
from dataclasses import dataclass
from typing import Callable

from . import config
from .behavior import BehaviorMonitor
from .ldm import LocalDynamicMap
from .messages import BSM
from .prediction import first_conflict, trajectory
from .radio import Frame
from .security import CertificateAuthority, Signer, Verifier

LogFn = Callable[[float, str, str, str], None]  # (t, source, kind, text)


@dataclass
class Threat:
    ttc: float
    msg: BSM
    traj: list

    @property
    def tid(self) -> str:
        return self.msg.temp_id


def relation(me, other: BSM) -> str:
    """How `other` relates to my path: ahead / behind / oncoming / crossing."""
    dx, dy = other.x - me.x, other.y - me.y
    along = dx * math.cos(me.heading) + dy * math.sin(me.heading)
    hdiff = math.cos(other.heading - me.heading)
    if hdiff > 0.7:
        return "ahead" if along > 0 else "behind"
    if hdiff < -0.7:
        return "oncoming"
    return "crossing"


class OBU:
    def __init__(self, vehicle, ca: CertificateAuthority, signer: Signer,
                 security: bool = True, log: LogFn | None = None,
                 describe: Callable[[str], str] = str, relay: bool = True,
                 speed_limit: Callable[[float, float], float | None] | None = None):
        self.vehicle = vehicle
        self.name = vehicle.name
        self.ca = ca
        self.signer = signer
        self.verifier = Verifier(ca, enabled=security)
        self.ldm = LocalDynamicMap()
        self.log = log or (lambda *a: None)
        self.describe = describe
        self.own_ids: set[str] = set()
        self.relay = relay
        self.behavior = BehaviorMonitor(speed_limit)   # records own + every peer's driving
        self.outbox: list[Frame] = []    # frames to forward on the next tick (100 ms per hop)
        self.hazard: tuple[float, BSM, int] | None = None   # (gap m, msg, hops)
        self._hazard_mem = None
        self.state = "CRUISE"            # CRUISE | ADVISE | WARN | YIELD | BRAKE
        self.yield_to: str | None = None
        self.worst_ttc: float | None = None
        self.hmi_text = ""
        self.hmi_level = "info"
        self._hmi_key = None
        self._rejected: set[tuple[str, str]] = set()
        self._reported: set[str] = set()
        self.blocked: set[str] = set()   # senders this car caught misbehaving: ignored locally
        self.stats = {"tx": 0, "rx": 0, "accepted": 0, "rejected": 0, "relayed": 0}

    # ------------------------------------------------------------ transmit
    def build_bsm(self, now: float) -> BSM:
        v = self.vehicle
        cred = self.signer.current(now)
        self.own_ids.add(cred.cert_id)
        return BSM(temp_id=cred.cert_id, t=round(now, 2), x=round(v.x, 3), y=round(v.y, 3),
                   speed=round(v.speed, 3), heading=round(v.heading, 5),
                   yaw_rate=round(v.yaw_rate, 5), accel=round(v.accel, 3),
                   brake_on=v.accel < -0.5, length=v.length, width=v.width)

    def transmit(self, now: float) -> Frame:
        self.stats["tx"] += 1
        pkt = self.signer.sign(self.build_bsm(now), self.signer.current(now))
        return Frame(pkt, 0, (self.vehicle.x, self.vehicle.y))

    def flush_relays(self) -> list[Frame]:
        out, self.outbox = self.outbox, []
        return out

    # ------------------------------------------------------------ receive
    def receive(self, frame: Frame, now: float):
        self.stats["rx"] += 1
        pkt = frame.packet
        msg, why = self.verifier.verify(pkt)    # always the ORIGINATOR's signature
        if msg is None:
            self._reject(pkt.cert_id, why, now, report=False)
            return
        if msg.temp_id in self.blocked:
            self.stats["rejected"] += 1
            return
        if msg.temp_id in self.own_ids:
            return
        why = self.ldm.check(msg, now)
        if why in ("duplicate", "stale"):
            return
        if why:
            self._reject(msg.temp_id, why, now, report=True)
            return
        self.stats["accepted"] += 1
        self.ldm.update(msg, now, frame.hops)
        # Forward it (store-and-forward, once per message). Only verified, plausible,
        # new messages are relayed, so forged or bogus data doesn't spread.
        if self.relay and frame.hops < config.MAX_HOPS:
            v = self.vehicle
            if math.hypot(v.x - frame.last_hop[0], v.y - frame.last_hop[1]) >= config.RELAY_MIN_PROGRESS_M:
                self.outbox.append(Frame(pkt, frame.hops + 1, (v.x, v.y)))
                self.stats["relayed"] += 1

    def _reject(self, cid: str, why: str, now: float, report: bool):
        self.stats["rejected"] += 1
        if (cid, why) not in self._rejected:  # log once per sender+reason, not 10x a second
            self._rejected.add((cid, why))
            self.log(now, self.name, "security", f"dropped message from {cid}: {why}")
        if report:
            self.blocked.add(cid)             # protect this car immediately, before any CA decision
            self.ldm.entries.pop(cid, None)
        if report and cid not in self._reported:
            self._reported.add(cid)
            self.log(now, self.name, "security", f"misbehavior report filed against {cid}")
            if self.ca.report(cid, self.name, why):
                self.log(now, "CA", "security", f"certificate {cid} revoked ({why})")

    # ------------------------------------------------------------ right of way
    def must_yield(self, other: BSM) -> bool:
        me = self.vehicle
        rel = relation(me, other)
        if rel == "ahead":
            return True                       # never hit the car in front
        if rel == "behind":
            return False
        me_turning = abs(me.yaw_rate) > 0.05
        other_turning = abs(other.yaw_rate) > 0.05
        if me_turning != other_turning:
            return me_turning                 # turning traffic yields to straight traffic
        if rel == "oncoming":
            return True                       # head-on: both brake
        dx, dy = other.x - me.x, other.y - me.y
        cross = math.cos(me.heading) * dy - math.sin(me.heading) * dx
        return cross < 0                      # crossing: yield to the car on my right

    def _has_passed(self, other: BSM) -> bool:
        me = self.vehicle
        dx, dy = me.x - other.x, me.y - other.y
        toward_me = dx * math.cos(other.heading) + dy * math.sin(other.heading)
        return toward_me < 0 and math.hypot(dx, dy) > other.length + 2

    def _target_cleared(self, target: BSM, target_traj, now: float) -> bool:
        me = self.vehicle
        if relation(me, target) == "crossing":
            return self._has_passed(target)   # hysteresis: wait until it's really gone
        # same path: clear once resuming cruise would not create a conflict
        ego = trajectory(me.x, me.y, me.heading, me.speed, config.CRUISE_ACCEL, me.yaw_rate)
        return first_conflict(ego, (me.length, me.width), target_traj,
                              (target.length, target.width)) is None

    def _gentlest_safe_accel(self, avoid: list[Threat]) -> float:
        me = self.vehicle
        for a in config.BRAKE_LEVELS:
            ego = trajectory(me.x, me.y, me.heading, me.speed, a, me.yaw_rate)
            if all(first_conflict(ego, (me.length, me.width), th.traj,
                                  (th.msg.length, th.msg.width),
                                  time_buffer=config.YIELD_PET_S) is None for th in avoid):
                return a
        return config.MAX_BRAKE

    # ------------------------------------------------------------ decide
    def decide(self, now: float):
        """Run all applications. Returns an acceleration cap (m/s^2) or None.

        The cap limits what the driver can do: 0 = no accelerating, negative = auto-brake.
        """
        self.ldm.evict(now)
        me = self.vehicle
        dims = (me.length, me.width)
        ego_const = trajectory(me.x, me.y, me.heading, me.speed, 0.0, me.yaw_rate)

        # Collision avoidance: only fresh data about cars close enough to matter.
        fresh = {tid: m for tid, m in self.ldm.entries.items()
                 if now - m.t <= config.SAFETY_MAX_AGE_S
                 and math.hypot(m.x - me.x, m.y - me.y) <= config.SAFETY_RANGE_M}
        threats: list[Threat] = []
        trajs: dict[str, list] = {}
        eebl = None
        for msg in fresh.values():
            tr = trajectory(msg.x, msg.y, msg.heading, msg.speed, msg.accel, msg.yaw_rate,
                            lead_time=max(0.0, now - msg.t))
            trajs[msg.temp_id] = tr
            ttc = first_conflict(ego_const, dims, tr, (msg.length, msg.width))
            if ttc is not None and ttc < config.TTC_WARN_S:
                threats.append(Threat(ttc, msg, tr))
            if msg.accel <= -4.0 and relation(me, msg) == "ahead":
                gap = math.hypot(msg.x - me.x, msg.y - me.y)
                if gap < 200 and (eebl is None or gap < eebl[0]):
                    eebl = (gap, msg)
        threats.sort(key=lambda th: th.ttc)
        self.worst_ttc = threats[0].ttc if threats else None
        self.hazard = self._find_hazard(now)
        if self.hazard:
            self._hazard_mem = (now, self.hazard[1], self.hazard[2])
        elif self._hazard_mem and now - self._hazard_mem[0] <= config.HAZARD_MEMORY_S:
            _, m, hops = self._hazard_mem          # relay chain gap: keep the last report
            c, s_ = math.cos(me.heading), math.sin(me.heading)
            gap = (m.x - me.x) * c + (m.y - me.y) * s_ - (me.length + m.length) / 2
            self.hazard = (gap, m, hops) if gap > 0 else None

        # Keep yielding until the target has clearly cleared (or is lost for too long).
        lost_target = False
        if self.state == "YIELD":
            target = fresh.get(self.yield_to)
            if target is None:
                if now - self.ldm.last_seen.get(self.yield_to, -1e9) > config.YIELD_TIMEOUT_S \
                        and not threats:
                    self.state, self.yield_to = "CRUISE", None
                else:
                    lost_target = True
            elif self._target_cleared(target, trajs[target.temp_id], now):
                self.state, self.yield_to = "CRUISE", None

        yield_threats = [th for th in threats if self.must_yield(th.msg)]
        critical = [th for th in threats
                    if th not in yield_threats and th.ttc < config.TTC_BRAKE_S]
        avoid = yield_threats + critical
        if self.state == "YIELD" and not lost_target and \
                all(th.tid != self.yield_to for th in avoid):
            t = fresh[self.yield_to]
            avoid.append(Threat(math.inf, t, trajs[t.temp_id]))

        cap = None
        if lost_target:
            cap = config.FAILSAFE_BRAKE           # fail safe: stop, don't drive on blind
        elif avoid:
            cap = self._gentlest_safe_accel(avoid)
            if yield_threats and self.state != "YIELD":
                self.state, self.yield_to = "YIELD", yield_threats[0].tid
            elif self.state != "YIELD":
                self.state = "BRAKE"
        elif threats:
            self.state = "WARN"
        else:
            self.state = "CRUISE"

        hz_cap = self._hazard_cap()
        if hz_cap is not None:
            cap = hz_cap if cap is None else min(cap, hz_cap)
        if self.hazard and self.state == "CRUISE" and (me.speed > 1.0 or self.hazard[0] > 15):
            self.state = "ADVISE"

        self._update_hmi(now, threats, eebl, lost_target)
        self.behavior.observe(now, config.TICK, self.build_bsm(now), self.ldm.entries)
        return cap

    # ------------------------------------------------------------ hazard ahead
    def _find_hazard(self, now: float):
        """Nearest stopped car ahead in my lane, from ANY entry (direct or relayed)."""
        me = self.vehicle
        c, s = math.cos(me.heading), math.sin(me.heading)
        best = None
        for tid, m in self.ldm.entries.items():
            if m.speed > config.HAZARD_STOPPED_SPEED or relation(me, m) != "ahead":
                continue
            dx, dy = m.x - me.x, m.y - me.y
            along, lateral = dx * c + dy * s, -dx * s + dy * c
            gap = along - (me.length + m.length) / 2
            if abs(lateral) < 3.0 and 0 < gap < config.HAZARD_RANGE_M and (best is None or gap < best[0]):
                best = (gap, m, self.ldm.hops.get(tid, 0))
        return best

    def _hazard_cap(self):
        """Slow to advisory speed, then brake just hard enough to stop behind the queue."""
        if not self.hazard:
            return None
        v = self.vehicle.speed
        if v < 0.5:
            return 0.0
        room = max(1.0, self.hazard[0] - config.HAZARD_BUFFER_M)
        need = v * v / (2 * room)                     # decel that stops exactly in time
        cap = None
        if need >= 1.0:
            cap = -min(config.COMFORT_DECEL, need * 1.1)
        if v > config.HAZARD_ADVISORY_SPEED:
            cap = min(cap if cap is not None else 0.0, -1.0)
        return cap

    # ------------------------------------------------------------ HMI
    def _update_hmi(self, now, threats, eebl, lost_target):
        th = threats[0] if threats else None
        if self.state == "YIELD":
            who = self.describe(self.yield_to)
            if lost_target:
                level, text = "warning", f"Lost contact with {who} - holding brake (fail-safe)"
            else:
                tt = next((t for t in threats if t.tid == self.yield_to), None)
                rel = relation(self.vehicle, self.ldm.entries[self.yield_to]) if self.yield_to in self.ldm.entries else "lost"
                why = {"ahead": "slow/stopped vehicle ahead", "crossing": "hidden car from the right",
                       "oncoming": "oncoming car"}.get(rel, "conflict")
                level = "warning"
                text = (f"BRAKING: {why} {who}" +
                        (f", conflict in {tt.ttc:.1f} s" if tt else " - waiting for it to clear"))
            key = ("YIELD", self.yield_to, lost_target)
        elif self.state == "BRAKE":
            level, text = "warning", f"EMERGENCY BRAKE: {self.describe(th.tid)} is not yielding ({th.ttc:.1f} s)"
            key = ("BRAKE", th.tid)
        elif self.state == "WARN":
            rel = relation(self.vehicle, th.msg)
            level = "caution"
            text = (f"CAUTION: {self.describe(th.tid)} {rel}, TTC {th.ttc:.1f} s"
                    + (" - you have priority" if rel == "crossing" else ""))
            key = ("WARN", th.tid)
        elif self.state == "ADVISE":
            gap, m, hops = self.hazard
            how = "heard directly" if hops == 0 else f"relayed by {hops} car{'s' if hops > 1 else ''}"
            level = "caution"
            text = f"STOPPED TRAFFIC {gap:,.0f} m ahead ({self.describe(m.temp_id)}), {how} - slowing down"
            key = ("ADVISE", m.temp_id, int(gap // 250))
        elif eebl:
            level, text = "caution", f"HARD BRAKING {eebl[0]:.0f} m ahead by {self.describe(eebl[1].temp_id)}"
            key = ("EEBL", eebl[1].temp_id)
        else:
            level, text, key = "info", "Clear.", ("CRUISE",)
        self.hmi_level, self.hmi_text = level, text
        if key != self._hmi_key:
            self._hmi_key = key
            self.log(now, self.name, f"hmi:{level}",
                     f"{text}  (speed {self.vehicle.speed * 3.6:.0f} km/h)")
