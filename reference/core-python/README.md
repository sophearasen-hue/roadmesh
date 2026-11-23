# V2V on-board unit + safety console

The software logic of a vehicle-to-vehicle (V2V) on-board unit (OBU), plus a web
app that replays what it does. Plain Python 3.10+, standard library only.

## Run it

```bash
python -m v2v list
python -m v2v compare blind_intersection        # V2V on vs off
python -m v2v run emergency_brake --loss 0.6    # 60 % packet loss
python -m v2v run ghost_attack --no-security    # see the attack work without signatures
python -m v2v run highway_relay --no-relay      # direct radio only, no car-to-car relaying
python -m v2v build-app                         # writes app/index.html (open it in a browser)
python -m unittest discover -s tests -t .
```

## Languages

The app has an **EN / ខ្មែរ** switch in the header. The choice is remembered in the
browser. Khmer covers the whole interface, the in-car display, map labels, narration
and the event log: the simulator writes its events in English and the app translates
each known message pattern (`KM_EV` in `v2v/app_template.html`). Khmer text uses the
Kantumruy Pro font. Translations should be reviewed by a native speaker before release.

## Scenarios

| Scenario | V2V off | V2V on |
|---|---|---|
| `blind_intersection`: building hides two cars at a corner | collision at 5.6 s | Car A yields from 3.4 s before either driver can see the other; 12.6 m gap |
| `emergency_brake`: three cars at 90 km/h, 20 m apart, lead car brakes at 8 m/s² | two rear-end collisions | all three stop, even with 60 % packet loss |
| `ghost_attack`: roadside attackers broadcast fake cars | n/a | forged ghost dropped, insider blocked locally and reported (CA revokes after 3 independent reports), no phantom braking (with signatures off: Car A brakes to 31 km/h for nothing) |
| `mixed_traffic`: rural road, village 40 / national road 80 / town 40, seven cars | n/a (no recorder without V2V) | every car's app records speeding, harsh braking/acceleration, tailgating and overtaking of all peers it hears |
| `highway_relay`: fog (110 m visibility), 130 km/h, traffic stopped 1.5 km ahead, oncoming app users every 250 m | crash at 41.5 s | direct radio only: warned at 295 m, brakes 2.7 m/s². With peer-to-peer relay: warned at 1,461 m via 5 cars, brakes at most 1.2 m/s² |

## Driving-behaviour recorder

Every OBU also runs `v2v/behavior.py`. From the messages it hears (direct or
relayed) it records, for its own car and every peer:

| What | Rule |
|---|---|
| Speed | current, average, max; **speeding** = over the local limit + 10 % for 2 s (limits come from the map: `zones` per scenario) |
| Speed up / slow down | **harsh braking** ≤ −4 m/s², **harsh acceleration** ≥ 2.5 m/s² |
| Overtaking | who passed whom, side by side ≥ 1.5 m apart; flagged when done in the oncoming lane |
| Following | **tailgating** = under 1 s behind at over 30 km/h for 2 s |

A toy 0–100 score combines them. Peers are keyed by their rotating pseudonym; a
car that goes out of range has its open episodes closed at the moment it was last
heard. `python -m v2v behavior mixed_traffic --car CarA` prints the report:

```
  car          avg   max  over  brk  acc pass  tail score
  Tailgater     51    76    0s    0    0    0   51s    23
  Speeder       84   115   37s    0    0    3    0s    51
  Jerky         61    96    1s    1    1    0    0s    86
  CarA          52    71    0s    0    0    0    0s   100
```

## Peer-to-peer relay

Every car with the app is also a relay. When it hears a new, verified message it
forwards it once (store-and-forward, up to 8 hops, only if it is at least 100 m from
the previous hop, which keeps the flood down). The originator's signature travels
with the message and the hop count sits outside it, so a relay can pass a message on
but cannot change it. Each car's map therefore covers every app user the network can
reach, not just the ones within 300 m, and the app lists them with distance and
"direct" or "via N relays".

Collision avoidance still only trusts fresh (≤ 0.5 s), nearby (≤ 400 m) data. Relayed
data is older and is used for awareness and the gentle "stopped traffic ahead" slowdown.

## Stack (`v2v/`)

| Layer | File | What it does |
|---|---|---|
| Facilities | `messages.py` | Basic Safety Message (SAE J2735 subset) |
| Security | `security.py` | toy PKI: pseudonym certificates, signing, verification, misbehavior revocation (HMAC stand-in for IEEE 1609.2 ECDSA) |
| Radio | `radio.py` | peer-to-peer broadcast within 300 m, multi-hop frames (hop count + last relay outside the signature), random packet loss |
| Facilities | `ldm.py` | plausibility checks (speed, accel, stale **and** future timestamps, position jumps, replays) and the Local Dynamic Map, with 1 s ageing-out |
| Application | `prediction.py` | turn-rate + acceleration prediction, rotated-box overlap, post-encroachment time buffer |
| Application | `obu.py` | Intersection Movement Assist, forward collision / auto-brake, Emergency Electronic Brake Light; right of way; gentlest safe braking; fail-safe yield; in-car display (HMI) |
| World | `world.py`, `scenarios.py` | vehicle physics, human driver (sees only line of sight, reacts after 1.2 s), buildings, attackers |

The OBU returns an *acceleration cap*: the driver keeps control, but cannot
accelerate while yielding and gets braked automatically when needed.

## Limits of the model

- Signatures are HMAC with keys looked up at the CA, not ECDSA certificates. The flow is the same, the cryptography is not.
- Radio ignores buildings (real 5.9 GHz signals are weakened by them; raise `--loss` to approximate).
- A plausible ghost sent with a *valid* stolen certificate would still be believed until enough reports revoke it. Real systems add sensor cross-checks (camera/radar) for that.
- Prediction assumes current acceleration and turn rate hold; no driver intent (turn signals) yet.
- Relaying is simple controlled flooding. With dense traffic it multiplies radio load (the highway run relays ~10x more frames than it originates); real stacks use smarter forwarding (e.g. contention-based) and congestion control.
- The behaviour score is illustrative; the weights are not from any standard. Recording other drivers' behaviour is personal data even under pseudonyms: a real product needs clear purpose, local-only storage or consent, and retention limits.
- Showing every app user is done with rotating pseudonyms, never names or plates. Positions of other people's cars are still personal data, so a real app needs privacy review before it shows them.
