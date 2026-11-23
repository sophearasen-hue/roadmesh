"""Layer 3 (radio): peer-to-peer broadcast with optional multi-hop relaying.

Every car is both a sender and a relay. A Frame carries the originator's signed
packet untouched plus a small *unsigned* forwarding header (hop count, position of
the last relay), the same split ETSI GeoNetworking uses. A relay can forward a
message but cannot change it: any edit breaks the originator's signature.

Radio is not blocked by buildings here; at 5.9 GHz real signals are attenuated by
them, which you can approximate by raising the loss rate.
"""

import math
import random
from dataclasses import dataclass

from .security import Packet


@dataclass(frozen=True)
class Frame:
    packet: Packet
    hops: int = 0                                  # 0 = straight from the originator
    last_hop: tuple[float, float] = (0.0, 0.0)     # where it was last transmitted from


class Channel:
    def __init__(self, rng: random.Random, range_m: float, loss: float = 0.0):
        self.rng = rng
        self.range_m = range_m
        self.loss = loss
        self._pending: list[tuple[str, tuple[float, float], Frame]] = []
        self.stats = {"sent": 0, "relayed": 0, "delivered": 0, "lost": 0}

    def broadcast(self, sender: str, pos: tuple[float, float], frame: Frame):
        self._pending.append((sender, pos, frame))
        self.stats["relayed" if frame.hops else "sent"] += 1

    def deliver(self, receivers: dict, now: float):
        """receivers: name -> ((x, y), obu)."""
        for sender, (sx, sy), frame in self._pending:
            for name, ((rx, ry), obu) in receivers.items():
                if name == sender or math.hypot(rx - sx, ry - sy) > self.range_m:
                    continue
                if self.rng.random() < self.loss:
                    self.stats["lost"] += 1
                    continue
                self.stats["delivered"] += 1
                obu.receive(frame, now)
        self._pending.clear()
