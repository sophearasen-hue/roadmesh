"""Tunable constants for the whole stack. All units SI (m, s, m/s, rad)."""

TICK = 0.1                 # 10 Hz, the standard BSM rate
RADIO_RANGE_M = 300.0      # typical DSRC / C-V2X range

# Prediction / threat assessment
HORIZON_S = 5.0            # how far ahead the OBU predicts
PRED_STEP = 0.1
SAFETY_MARGIN_M = 0.5      # vehicle boxes are inflated by this much when predicting
TTC_WARN_S = 4.0           # warn the driver below this time-to-collision
TTC_BRAKE_S = 2.0          # auto-brake for a non-yielding car below this
YIELD_PET_S = 1.0          # when yielding, pass >= 1 s after the other car (post-encroachment time)

# Braking candidates the OBU tries, gentlest first (m/s^2)
BRAKE_LEVELS = (0.0, -1.5, -3.0, -4.5, -6.0, -8.0)
MAX_BRAKE = -8.0
FAILSAFE_BRAKE = -3.0      # used when the car we yield to drops off the map
YIELD_TIMEOUT_S = 3.0      # how long a lost yield target is still respected

# Plausibility checks (misbehavior detection)
MAX_PLAUSIBLE_SPEED = 70.0     # m/s
MAX_PLAUSIBLE_ACCEL = 12.0     # m/s^2
SAFETY_MAX_AGE_S = 0.5         # collision avoidance only trusts data this fresh
FAR_MAX_AGE_S = 2.0            # relayed data for long-range awareness may be this old
MAX_FUTURE_S = 0.5             # a timestamp this far in the future is misbehavior
LDM_TTL_S = 1.0                # drop neighbours not heard from for this long
MIN_MSG_INTERVAL_S = 0.05      # one pseudonym may not send faster than 20 msg/s (flood guard)

# Peer-to-peer relaying (multi-hop, like ETSI GeoNetworking)
MAX_HOPS = 8                   # a message is forwarded at most this many times
RELAY_MIN_PROGRESS_M = 100.0   # only relay if I'm this far from the last hop (limits flooding)
SAFETY_RANGE_M = 400.0         # collision prediction only looks at cars this close

# Hazard-ahead advisory (uses long-range, relayed data)
HAZARD_RANGE_M = 3000.0
HAZARD_STOPPED_SPEED = 3.0     # m/s; slower than this counts as stopped traffic
HAZARD_ADVISORY_SPEED = 90 / 3.6
HAZARD_MEMORY_S = 8.0          # stopped traffic doesn't vanish: remember it through relay gaps
HAZARD_BUFFER_M = 30.0         # come to rest this far behind the queue
COMFORT_DECEL = 3.0            # the advisory never brakes harder than this

# Security
PSEUDONYM_ROTATE_S = 300.0     # change temp ID / certificate every 5 min
REVOKE_AFTER_REPORTS = 3       # CA revokes only after this many INDEPENDENT reporters,
                               # so one malicious app cannot get honest cars revoked.
                               # The reporting car still blocks the sender locally at once.

# Human driver model
HUMAN_REACTION_S = 1.2
HUMAN_HAZARD_TTC_S = 3.0
CRUISE_ACCEL = 1.5
