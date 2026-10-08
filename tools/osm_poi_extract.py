"""Extract POI candidates (tagged nodes, tagged ways/areas) from a .osm.pbf.

Output (pickle): list of (kind, id, X, Y, tags) with kind "n" (node), "w" (way),
"r" (multipolygon relation).  X, Y = global Teasi coordinates in 360/2^28 deg units
(float).  For ways and relations X, Y is the centroid of the nodes / area; the
first node is stored as tags["@first"] = (X, Y) for ways.

With --filter only objects that compile_osmpoi.poi_type() accepts or that have
seamark:type (osmpoint) are kept; needed for large extracts (Great Britain), otherwise
every building and road ends up in the pickle.

usage: osm_poi_extract.py [--filter] <in.osm.pbf> <out.pkl>
"""
import pickle
import sys

import osmium

from compile_osmpoi import poi_type

FILTER = False

SCALE = 2 ** 28 / 360.0
KEYS = ("amenity", "shop", "tourism", "historic", "leisure", "railway", "highway",
        "public_transport", "aeroway", "aerialway", "craft", "office", "man_made",
        "natural", "building", "healthcare", "emergency", "sport", "place", "memorial",
        "seamark:type", "landuse", "waterway", "barrier")


def xy(lon, lat):
    return (lon + 180) * SCALE, (90 - lat) * SCALE


def centroid(pts):
    """Area centroid of a closed ring (shoelace), None if degenerate."""
    a = cx = cy = 0.0
    x0, y0 = pts[0]
    for (x1, y1), (x2, y2) in zip(pts, pts[1:] + pts[:1]):
        x1, y1, x2, y2 = x1 - x0, y1 - y0, x2 - x0, y2 - y0
        c = x1 * y2 - x2 * y1
        a += c; cx += (x1 + x2) * c; cy += (y1 + y2) * c
    if abs(a) < 1e-9:
        return None
    return x0 + cx / (3 * a), y0 + cy / (3 * a)


def wanted(t, kind):
    if not (any(k in t for k in KEYS) or "name" in t):
        return False
    return not FILTER or "seamark:type" in t or poi_type(dict(t), kind) is not None


class H(osmium.SimpleHandler):
    def __init__(self):
        super().__init__()
        self.out = []

    def node(self, n):
        if len(n.tags) and wanted(n.tags, "n"):
            self.out.append(("n", n.id, *xy(n.location.lon, n.location.lat), dict(n.tags)))

    def way(self, w):
        if not wanted(w.tags, "w"):
            return
        pts = [xy(n.location.lon, n.location.lat) for n in w.nodes if n.location.valid()]
        if not pts:
            return
        if len(pts) > 1 and w.nodes[0].ref == w.nodes[-1].ref:
            pts = pts[:-1]
        t = dict(w.tags)
        t["@first"] = pts[0]
        xs, ys = [p[0] for p in pts], [p[1] for p in pts]
        t["@bbox"] = ((min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2)
        t["@area"] = centroid(pts)
        self.out.append(("w", w.id, sum(p[0] for p in pts) / len(pts),
                         sum(p[1] for p in pts) / len(pts), t))

    def area(self, a):
        if not a.from_way() and wanted(a.tags, "r"):
            pts = [xy(n.lon, n.lat) for r in a.outer_rings() for n in r]
            if pts:
                self.out.append(("r", a.orig_id(), sum(p[0] for p in pts) / len(pts),
                                 sum(p[1] for p in pts) / len(pts), dict(a.tags)))


if __name__ == "__main__":
    FILTER = "--filter" in sys.argv
    src, dst = [a for a in sys.argv[1:] if not a.startswith("--")]
    h = H()
    h.apply_file(src, locations=True, idx="flex_mem")
    pickle.dump(h.out, open(dst, "wb"), protocol=5)
    print(len(h.out), "objects")
