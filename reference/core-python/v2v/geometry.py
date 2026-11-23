"""2-D geometry helpers: vehicle boxes, overlap tests, line of sight."""

import math

Point = tuple[float, float]
Polygon = list[Point]


def box(x: float, y: float, heading: float, length: float, width: float,
        margin: float = 0.0) -> Polygon:
    """Corners of a vehicle footprint (rotated rectangle) centred on (x, y)."""
    c, s = math.cos(heading), math.sin(heading)
    hl, hw = length / 2 + margin, width / 2 + margin
    return [(x + c * dx - s * dy, y + s * dx + c * dy)
            for dx, dy in ((hl, hw), (hl, -hw), (-hl, -hw), (-hl, hw))]


def rect(x0: float, y0: float, x1: float, y1: float) -> Polygon:
    return [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]


def _axes(poly: Polygon):
    for i in range(len(poly)):
        (x1, y1), (x2, y2) = poly[i], poly[(i + 1) % len(poly)]
        yield (y1 - y2, x2 - x1)


def polygons_overlap(a: Polygon, b: Polygon) -> bool:
    """Separating Axis Theorem for convex polygons."""
    for ax, ay in (*_axes(a), *_axes(b)):
        pa = [px * ax + py * ay for px, py in a]
        pb = [px * ax + py * ay for px, py in b]
        if max(pa) < min(pb) or max(pb) < min(pa):
            return False
    return True


def _orient(p: Point, q: Point, r: Point) -> float:
    return (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])


def segments_intersect(p1: Point, p2: Point, q1: Point, q2: Point) -> bool:
    d1, d2 = _orient(q1, q2, p1), _orient(q1, q2, p2)
    d3, d4 = _orient(p1, p2, q1), _orient(p1, p2, q2)
    return (d1 * d2 < 0) and (d3 * d4 < 0)


def point_in_polygon(p: Point, poly: Polygon) -> bool:
    x, y = p
    inside = False
    for i in range(len(poly)):
        (x1, y1), (x2, y2) = poly[i], poly[(i + 1) % len(poly)]
        if (y1 > y) != (y2 > y) and x < (x2 - x1) * (y - y1) / (y2 - y1) + x1:
            inside = not inside
    return inside


def segment_hits_polygon(p: Point, q: Point, poly: Polygon) -> bool:
    if point_in_polygon(p, poly) or point_in_polygon(q, poly):
        return True
    return any(segments_intersect(p, q, poly[i], poly[(i + 1) % len(poly)])
               for i in range(len(poly)))
