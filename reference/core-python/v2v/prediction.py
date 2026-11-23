"""Layer 5 (application): motion prediction and conflict detection.

Model: constant turn rate and constant acceleration (CTRA), integrated numerically.
Speed never goes negative, so a braking car is predicted to stop, not reverse.
"""

import math

from . import config
from .geometry import box, polygons_overlap

Trajectory = list[tuple[float, float, float, float]]  # (t, x, y, heading)


def trajectory(x: float, y: float, heading: float, speed: float, accel: float,
               yaw_rate: float, horizon: float = config.HORIZON_S,
               step: float = config.PRED_STEP, lead_time: float = 0.0) -> Trajectory:
    """Predicted poses at step, 2*step, ... horizon.

    lead_time advances the start state first (used to bring a message that is a
    few ms/ticks old up to "now").
    """
    pts: Trajectory = []
    n_lead = int(round(lead_time / step))
    n = int(round(horizon / step))
    for i in range(1, n_lead + n + 1):
        speed = max(0.0, speed + accel * step)
        if speed > 0:
            heading += yaw_rate * step
        x += speed * math.cos(heading) * step
        y += speed * math.sin(heading) * step
        if i > n_lead:
            pts.append(((i - n_lead) * step, x, y, heading))
    return pts


def first_conflict(traj_a: Trajectory, dims_a: tuple[float, float],
                   traj_b: Trajectory, dims_b: tuple[float, float],
                   margin: float = config.SAFETY_MARGIN_M, time_buffer: float = 0.0):
    """First predicted time at which the two (inflated) footprints overlap, or None.

    time_buffer > 0 also counts A being where B was/will be within +-time_buffer
    seconds (post-encroachment time), so "missing by 0.2 s" is still a conflict.
    """
    reach = (math.hypot(*dims_a) + math.hypot(*dims_b)) / 2 + 2 * margin
    k = int(round(time_buffer / config.PRED_STEP))
    n = min(len(traj_a), len(traj_b))
    for i in range(n):
        t, ax, ay, ah = traj_a[i]
        poly_a = None
        for j in range(max(0, i - k), min(n, i + k + 1)):
            _, bx, by, bh = traj_b[j]
            if math.hypot(ax - bx, ay - by) > reach:
                continue
            poly_a = poly_a or box(ax, ay, ah, *dims_a, margin)
            if polygons_overlap(poly_a, box(bx, by, bh, *dims_b, margin)):
                return t
    return None
