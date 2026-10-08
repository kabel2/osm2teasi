"""Extract areas (closed ways and multipolygons) and the coastline from a .osm.pbf.

Output (pickle): {"areas": [(area_id, tags, [(outer, [inner, ...]), ...])],
                  "coast": [(way_id, first_node, last_node, ring, place)]}
Rings are numpy int32 arrays (n, 2) of global Teasi coordinates in 360/2^25 deg units
(osmarea / 4x4 cells: X = cell_x * 32768 + u).  area_id is osmium's (2 * way id or
2 * relation id + 1).  Only areas with one of KEYS are kept.

usage: osm_area_extract.py <in.osm.pbf> <out.pkl>
"""
import pickle
import sys

import numpy as np
import osmium

SCALE = 2 ** 25 / 360.0
KEYS = ("landuse", "natural", "leisure", "man_made", "waterway", "water", "military",
        "aeroway", "amenity", "tourism")


def ring(nodes):
    a = np.array([((n.lon + 180) * SCALE, (90 - n.lat) * SCALE) for n in nodes])
    return np.rint(a).astype(np.int32)


class H(osmium.SimpleHandler):
    def __init__(self):
        super().__init__()
        self.areas, self.coast = [], []

    def way(self, w):
        if w.tags.get("natural") == "coastline" and len(w.nodes) > 1:
            pts = [n.location for n in w.nodes if n.location.valid()]
            self.coast.append((w.id, w.nodes[0].ref, w.nodes[-1].ref, ring(pts),
                               w.tags.get("place")))

    def area(self, a):
        t = a.tags
        if not any(k in t for k in KEYS) or t.get("natural") == "coastline":
            return
        polys = [(ring(o), [ring(i) for i in a.inner_rings(o)]) for o in a.outer_rings()]
        if polys:
            self.areas.append((a.id, dict(t), polys))


if __name__ == "__main__":
    src, dst = sys.argv[1:]
    h = H()
    h.apply_file(src, locations=True, idx="flex_mem")
    pickle.dump({"areas": h.areas, "coast": h.coast}, open(dst, "wb"), protocol=5)
    print(len(h.areas), "areas,", len(h.coast), "coastline ways")
