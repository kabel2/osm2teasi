"""Extract the OSM ways relevant for the Teasi layers from a .osm.pbf.

Output (pickle): {"ways": [(way_id, tags)], "rels": {way_id: [(route, network, ref)]},
                  "w": way index per node, "i": position in the way, "nid": node id,
                  "X", "Y": global Teasi coordinates in 360/2^28 deg units (float64)}
Teasi 32x32 cells use X = cell_x * 32768 + u, Y = cell_y * 32768 + v (CHART_FILES.md 1a).

With --filter only the ways compile_osm.py uses (road_class or line_type) are kept; needed
for large extracts (Great Britain).  The node columns are typed arrays, not lists.

usage: osm_extract.py [--filter] <in.osm.pbf> <out.pkl>
"""
import array
import pickle
import sys

import numpy as np
import osmium

from compile_osm import line_type, road_class

KEYS = ("highway", "route", "waterway", "railway", "natural", "power", "aerialway",
        "landuse", "leisure", "man_made", "boundary", "aeroway", "amenity", "place",
        "seamark:type", "barrier")
SCALE = 2 ** 28 / 360.0
FILTER = False


def wanted(tags):
    if not any(k in tags for k in KEYS):
        return False
    if not FILTER:
        return True
    t = dict(tags)
    return road_class(t) is not None or bool(line_type(t))


class H(osmium.SimpleHandler):
    def __init__(self):
        super().__init__()
        self.ways, self.rels = [], {}
        self.w, self.i, self.nid = array.array("i"), array.array("i"), array.array("q")
        self.lon, self.lat = array.array("i"), array.array("i")

    def way(self, w):
        if not wanted(w.tags):
            return
        k = len(self.ways)
        self.ways.append((w.id, dict(w.tags)))
        for j, n in enumerate(w.nodes):
            if not n.location.valid():
                continue
            self.w.append(k); self.i.append(j); self.nid.append(n.ref)
            self.lon.append(n.location.x); self.lat.append(n.location.y)

    def relation(self, r):
        t = r.tags
        if t.get("type") != "route" or t.get("route") not in ("bicycle", "mtb", "hiking", "foot", "ferry"):
            return
        v = (t.get("route"), t.get("network"), t.get("ref"))
        for m in r.members:
            if m.type == "w":
                self.rels.setdefault(m.ref, []).append(v)


if __name__ == "__main__":
    FILTER = "--filter" in sys.argv
    src, dst = [a for a in sys.argv[1:] if not a.startswith("--")]
    h = H()
    h.apply_file(src, locations=True, idx="flex_mem")
    lon = np.array(h.lon, dtype=np.int64)          # 1e-7 degrees
    lat = np.array(h.lat, dtype=np.int64)
    out = {"ways": h.ways, "rels": h.rels,
           "w": np.array(h.w, dtype=np.int32), "i": np.array(h.i, dtype=np.int32),
           "nid": np.array(h.nid, dtype=np.int64),
           "X": (lon / 1e7 + 180) * SCALE, "Y": (90 - lat / 1e7) * SCALE}
    pickle.dump(out, open(dst, "wb"), protocol=5)
    print(len(h.ways), "ways,", len(h.w), "way nodes,", len(h.rels), "ways in routes")
