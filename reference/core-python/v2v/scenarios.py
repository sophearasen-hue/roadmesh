"""Built-in scenarios. Each builder returns a ready-to-run World."""

import math

from .geometry import rect
from .world import GhostAttacker, Maneuver, Vehicle, World

KMH = 1 / 3.6
CITY = [(-1e6, 1e6, 50, "city street")]
CITY_BLOCKS = [rect(-120, -120, -6, -6), rect(6, -120, 120, -6),
               rect(-120, 6, -6, 120), rect(6, 6, 120, 120)]


def blind_intersection(v2v=True, loss=0.0, security=True, relay=True, seed=1, verbose=False) -> World:
    """Car A drives east, Car B north; a building hides them until the last moment."""
    a = Vehicle("CarA", x=-80, y=-1.5, heading=0.0, speed=50 * KMH, color="#3b82f6")
    b = Vehicle("CarB", x=1.5, y=-80, heading=math.pi / 2, speed=50 * KMH, color="#f59e0b")
    return World([a, b], CITY_BLOCKS, v2v=v2v, security=security, loss=loss, seed=seed, relay=relay,
                 watch=("CarA", "CarB"), duration=12, verbose=verbose, zones=CITY)


def emergency_brake(v2v=True, loss=0.0, security=True, relay=True, seed=1, verbose=False) -> World:
    """Three cars at 90 km/h, 20 m apart. Car1 brakes hard; Car3 can't see past Car2."""
    v = 90 * KMH
    cars = [
        Vehicle("Car1", x=60, y=-1.75, heading=0.0, speed=v, color="#ef4444",
                maneuvers=[Maneuver(t0=1.5, accel=-8.0)]),
        Vehicle("Car2", x=35.5, y=-1.75, heading=0.0, speed=v, color="#f59e0b"),
        Vehicle("Car3", x=11, y=-1.75, heading=0.0, speed=v, color="#3b82f6"),
    ]
    return World(cars, [], v2v=v2v, security=security, loss=loss, seed=seed, relay=relay,
                 watch=("Car3", "Car1"), duration=8, verbose=verbose,
                 zones=[(-1e6, 1e6, 100, "national road")])


def ghost_attack(v2v=True, loss=0.0, security=True, relay=True, seed=1, verbose=False) -> World:
    """Empty intersection. Two roadside attackers broadcast fake cars at Car A.

    * outsider: plausible fake car crossing from the right, forged signature
    * insider : real (stolen) certificate, sends a car at 340 km/h -> fails plausibility,
                gets blocked by Car A at once and reported (the CA revokes after 3 reporters)
    """
    a = Vehicle("CarA", x=-80, y=-1.5, heading=0.0, speed=50 * KMH, color="#3b82f6")

    def attackers(world: World):
        insider_cert = world.ca.issue(1)[0]
        return [
            GhostAttacker("Forger", pos=(14, -20), start=(1.5, -80), heading=math.pi / 2,
                          speed=50 * KMH, t_start=0.0, t_end=9.0, rng=world.rng),
            GhostAttacker("Insider", pos=(-20, 14), start=(-1.5, 400), heading=-math.pi / 2,
                          speed=95.0, t_start=2.0, t_end=4.5, credential=insider_cert),
        ]

    return World([a], CITY_BLOCKS, v2v=v2v, security=security, loss=loss, seed=seed, relay=relay,
                 attackers_factory=attackers, duration=12, verbose=verbose, zones=CITY)


def highway_relay(v2v=True, loss=0.0, security=True, relay=True, seed=1, verbose=False) -> World:
    """Fog on a highway. Traffic has stopped 1.5 km ahead of Car A (130 km/h).

    Drivers see 110 m. The radio reaches 300 m. Oncoming cars with the app pass
    the stopped cars' messages back along the road, car to car.
    """
    cars = [Vehicle("CarA", x=0, y=-1.75, heading=0.0, speed=130 * KMH, color="#3b82f6")]
    for i, x in enumerate((1500, 1492, 1484)):               # stopped queue
        cars.append(Vehicle(f"Stop{i + 1}", x=x, y=-1.75, heading=0.0, speed=0.0,
                            target_speed=0.0, color="#ef4444", role="hazard"))
    for i in range(11):                                      # oncoming app users, every 250 m
        cars.append(Vehicle(f"Relay{i + 1}", x=250 + 250 * i, y=1.75, heading=math.pi,
                            speed=100 * KMH, color="#8b95a1", role="relay"))
    return World(cars, [], v2v=v2v, security=security, loss=loss, seed=seed, relay=relay,
                 watch=("CarA", "Stop3"), visibility=110.0, duration=64,
                 view=(-60, -40, 1620, 40), record_every=5, verbose=verbose,
                 zones=[(-1e6, 1e6, 130, "expressway")])


def mixed_traffic(v2v=True, loss=0.0, security=True, relay=True, seed=1, verbose=False) -> World:
    """Two-lane rural road through a village (40 km/h), the national road (80 km/h)
    and a town (40 km/h). Every car has the app and records everyone's driving.

    Truck: slow and steady. CarA: you, keeps to the limits. Tailgater: sits under 1 s
    behind you. Jerky: harsh braking and acceleration. Speeder: ignores limits and
    overtakes the whole line through the oncoming lane. Oncoming: two cars.
    """
    e = 0.0
    cars = [
        Vehicle("Jerky", x=330, y=-1.75, heading=e, speed=40 * KMH, target_speed=80 * KMH,
                color="#a855f7", headway_s=2.0, maneuvers=[Maneuver(t0=14, t1=15.6, accel=-6.0),
                                            Maneuver(t0=19, t1=22, accel=3.5)]),
        Vehicle("Truck", x=150, y=-1.75, heading=e, speed=40 * KMH, target_speed=60 * KMH,
                length=10.0, width=2.5, color="#a16207", headway_s=2.0),
        Vehicle("CarA", x=110, y=-1.75, heading=e, speed=40 * KMH, target_speed=80 * KMH,
                color="#3b82f6", headway_s=2.0),
        Vehicle("Tailgater", x=101, y=-1.75, heading=e, speed=40 * KMH, target_speed=80 * KMH,
                color="#f59e0b", headway_s=0.5),
        Vehicle("Speeder", x=-60, y=-1.75, heading=e, speed=70 * KMH, target_speed=115 * KMH,
                color="#ef4444", respects_limit=False, overtakes=True, headway_s=1.5),
        Vehicle("Van", x=700, y=1.75, heading=math.pi, speed=70 * KMH, color="#14b8a6", headway_s=2.0),
        Vehicle("Moto", x=2600, y=1.75, heading=math.pi, speed=80 * KMH, length=2.0,
                width=0.8, color="#64748b"),
    ]
    zones = [(-1e6, 400, 40, "village"), (400, 1500, 80, "national road"), (1500, 1e6, 40, "town")]
    return World(cars, [], v2v=v2v, security=security, loss=loss, seed=seed, relay=relay,
                 watch=("CarA", "Speeder"), duration=60, zones=zones,
                 view=(-80, -40, 1700, 40), record_every=5, verbose=verbose)


SCENARIOS = {
    "blind_intersection": {
        "build": blind_intersection,
        "title": "Blind intersection",
        "summary": "Two cars approach a corner hidden by a building. "
                   "Car B comes from Car A's right, so Car A must yield.",
    },
    "emergency_brake": {
        "build": emergency_brake,
        "title": "Emergency brake chain",
        "summary": "Car 1 brakes hard at 90 km/h. Car 3 can't see it past Car 2 - "
                   "but it hears Car 1's brake message directly (EEBL).",
    },
    "ghost_attack": {
        "build": ghost_attack,
        "title": "Ghost car attack",
        "summary": "Roadside attackers broadcast fake cars. Signatures, plausibility "
                   "checks and revocation keep Car A from braking for nothing.",
    },
    "mixed_traffic": {
        "build": mixed_traffic,
        "title": "Driving behaviour",
        "summary": "A rural road with village and national-road speed limits. Every car "
                   "records how its peers drive: speed vs the limit, harsh braking and "
                   "acceleration, tailgating and overtaking.",
    },
    "highway_relay": {
        "build": highway_relay,
        "title": "Relay through traffic",
        "summary": "Fog, 130 km/h, traffic stopped 1.5 km ahead. Oncoming cars pass the "
                   "warning back car to car, far beyond one radio's 300 m reach.",
    },
}
