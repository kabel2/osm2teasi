"""Extract addresses and places from a .osm.pbf for the ta layer (TA_FORMAT.md).

Output (pickle): {"addr": [(X, Y, housenumber, street, postcode, city, place, suburb)],
                  "places": [(X, Y, place, name, tags)],
                  "interp": [(kind, [(X, Y, housenumber, street)], street)]}
X, Y = global Teasi coordinates in 360/2^28 deg units (float), for ways/areas the mean of
the outer ring nodes.  Addresses: every node, way or multipolygon with addr:housenumber and
addr:street or addr:place.  Interpolation ways (addr:interpolation=odd/even/all) with the
house numbers of their nodes.  Places: nodes and areas with place=* and name.

usage: osm_addr_extract.py <in.osm.pbf> <out.pkl>
"""
import pickle
import sys

import osmium

SCALE = 2 ** 28 / 360.0
PLACES = {"city", "town", "village", "hamlet", "suburb", "quarter", "neighbourhood",
          "locality", "isolated_dwelling", "borough", "island", "islet", "farm"}
PLACE_TAGS = ("name", "name:en", "population", "is_in", "place")


def xy(lon, lat):
    return (lon + 180) * SCALE, (90 - lat) * SCALE


def addr(t):
    return (t.get("addr:housenumber"), t.get("addr:street"), t.get("addr:postcode"),
            t.get("addr:city"), t.get("addr:place"), t.get("addr:suburb"))


class H(osmium.SimpleHandler):
    def __init__(self):
        super().__init__()
        self.addr, self.places, self.interp = [], [], []
        self.nodes = {}                      # node id -> (X, Y, housenumber) for interpolation

    def node(self, n):
        t = n.tags
        if "addr:housenumber" in t and ("addr:street" in t or "addr:place" in t):
            self.addr.append((*xy(n.location.lon, n.location.lat), *addr(t)))
        if t.get("place") in PLACES and "name" in t:
            self.places.append((*xy(n.location.lon, n.location.lat), t["place"], t["name"],
                                {k: t[k] for k in PLACE_TAGS if k in t}))

    def way(self, w):
        t = w.tags
        if "addr:interpolation" in t:
            pts = [(*xy(n.location.lon, n.location.lat), n.ref) for n in w.nodes
                   if n.location.valid()]
            if len(pts) > 1:
                self.interp.append((t["addr:interpolation"], pts, t.get("addr:street")))

    def area(self, a):
        t = a.tags
        has_addr = "addr:housenumber" in t and ("addr:street" in t or "addr:place" in t)
        is_place = t.get("place") in PLACES and "name" in t
        if not (has_addr or is_place):
            return
        pts = [xy(n.lon, n.lat) for r in a.outer_rings() for n in r]
        if not pts:
            return
        x, y = sum(p[0] for p in pts) / len(pts), sum(p[1] for p in pts) / len(pts)
        if has_addr:
            self.addr.append((x, y, *addr(t)))
        if is_place:
            self.places.append((x, y, t["place"], t["name"],
                                dict({k: t[k] for k in PLACE_TAGS if k in t}, area=True)))


class NodeNumbers(osmium.SimpleHandler):
    """Second pass: house numbers of the nodes used by interpolation ways."""
    def __init__(self, wanted):
        super().__init__()
        self.wanted, self.num = wanted, {}

    def node(self, n):
        if n.id in self.wanted and "addr:housenumber" in n.tags:
            self.num[n.id] = (n.tags["addr:housenumber"], n.tags.get("addr:street"))


if __name__ == "__main__":
    src, dst = sys.argv[1:]
    h = H()
    h.apply_file(src, locations=True, idx="flex_mem")
    wanted = {p[2] for _, pts, _ in h.interp for p in pts}
    nn = NodeNumbers(wanted)
    nn.apply_file(src)
    interp = [(kind, [(x, y, *nn.num.get(ref, (None, None))) for x, y, ref in pts], street)
              for kind, pts, street in h.interp]
    pickle.dump({"addr": h.addr, "places": h.places, "interp": interp}, open(dst, "wb"),
                protocol=5)
    print(len(h.addr), "addresses,", len(h.places), "places,", len(interp), "interpolations")
