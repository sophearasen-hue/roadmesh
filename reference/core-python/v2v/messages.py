"""Layer 1 (facilities): the Basic Safety Message."""

import json
from dataclasses import asdict, dataclass


@dataclass(frozen=True)
class BSM:
    """Basic Safety Message (a subset of the SAE J2735 BSM Part I fields)."""
    temp_id: str        # rotating pseudonym = current certificate ID, never the plate
    t: float            # timestamp, s
    x: float            # position east, m (real BSM: lat/lon)
    y: float            # position north, m
    speed: float        # m/s
    heading: float      # rad, 0 = east, pi/2 = north
    yaw_rate: float     # rad/s
    accel: float        # m/s^2, negative = braking
    brake_on: bool
    length: float = 4.5
    width: float = 1.8

    def encode(self) -> bytes:
        return json.dumps(asdict(self), sort_keys=True, separators=(",", ":")).encode()

    @staticmethod
    def decode(raw: bytes) -> "BSM":
        return BSM(**json.loads(raw))
