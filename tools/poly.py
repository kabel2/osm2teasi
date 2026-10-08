"""Osmosis .poly files (e.g. download.geofabrik.de/europe/denmark.poly) and a
point-in-polygon test in Teasi units (360/2^28 deg, X east, Y south from 90N)."""

SCALE = 2 ** 28 / 360.0


def load(path):
    """-> list of (ring, is_hole); ring = [(X, Y)]"""
    rings, cur, hole = [], None, False
    lines = [l.strip() for l in open(path)][1:]
    for l in lines:
        if not l:
            continue
        if cur is None:
            if l == "END":
                break
            cur, hole = [], l.startswith("!")
        elif l == "END":
            rings.append((cur, hole))
            cur = None
        else:
            lon, lat = map(float, l.split())
            cur.append(((lon + 180) * SCALE, (90 - lat) * SCALE))
    return rings


def _inside(ring, x, y):
    c = False
    for (x1, y1), (x2, y2) in zip(ring, ring[1:] + ring[:1]):
        if (y1 > y) != (y2 > y) and x < x1 + (y - y1) * (x2 - x1) / (y2 - y1):
            c = not c
    return c


def contains(rings, x, y):
    n = sum(1 for r, hole in rings if not hole and _inside(r, x, y))
    return n > 0 and not any(hole and _inside(r, x, y) for r, hole in rings)
