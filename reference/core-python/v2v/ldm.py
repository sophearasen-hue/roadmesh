"""Layer 4 (facilities): plausibility checks and the Local Dynamic Map."""

import math

from . import config
from .messages import BSM


class LocalDynamicMap:
    """Latest accepted BSM per neighbour, with ageing-out of silent neighbours."""

    def __init__(self, ttl: float = config.LDM_TTL_S):
        self.ttl = ttl
        self.entries: dict[str, BSM] = {}
        self.last_seen: dict[str, float] = {}
        self.hops: dict[str, int] = {}           # 0 = heard directly, n = via n relays

    def check(self, msg: BSM, now: float):
        """None if plausible; "duplicate"/"stale" for copies we can drop quietly
        (they arrive all the time once messages are relayed); else a misbehavior reason."""
        if not (0.0 <= msg.speed <= config.MAX_PLAUSIBLE_SPEED):
            return f"implausible speed {msg.speed:.0f} m/s"
        if abs(msg.accel) > config.MAX_PLAUSIBLE_ACCEL:
            return f"implausible acceleration {msg.accel:.0f} m/s^2"
        if now - msg.t > config.FAR_MAX_AGE_S:
            return "stale"                       # normal for long relay chains: drop quietly
        if msg.t - now > config.MAX_FUTURE_S:
            return "timestamp in the future"
        prev = self.entries.get(msg.temp_id)
        if prev is not None:
            if msg.t <= prev.t:
                return "duplicate"
            if msg.t - prev.t < config.MIN_MSG_INTERVAL_S:
                return "message rate too high"
            implied = math.hypot(msg.x - prev.x, msg.y - prev.y) / (msg.t - prev.t)
            if implied > max(prev.speed, msg.speed) * 1.5 + 10.0:
                return f"position jump ({implied:.0f} m/s implied)"
        return None

    def update(self, msg: BSM, now: float, hops: int = 0):
        self.entries[msg.temp_id] = msg
        self.last_seen[msg.temp_id] = now
        self.hops[msg.temp_id] = hops

    def evict(self, now: float):
        for tid in [k for k, m in self.entries.items() if now - self.last_seen[k] > self.ttl]:
            del self.entries[tid]
