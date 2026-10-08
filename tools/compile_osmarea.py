"""Compile the Teasi osmarea layer (land use, water, sea) from OSM (OSMAREA_FORMAT.md).

Input is the pickle of osm_area_extract.py.  The rules were derived by matching
Denmark_osmarea.v20210810 against denmark-220101 (OSMAREA_FORMAT.md "Aus OSM erzeugen"):

  1. tags -> array/class (area_class), areas < MIN_AREA are dropped
  2. all polygons of one class are merged (union, per group of touching polygons), every
     merged polygon gets the detail
     level hi from the shorter side of its bounding box (level), holes their own
  3. Douglas-Peucker with 4 units, clipped to the country boundary <area.poly>
  4. cut into 4096-unit blocks (8 x 8 per cell): one object per block and class
  5. sea (#OW, one object per cell): inside the boundary cell minus the land built from
     natural=coastline, outside it the sea of the original file (the Denmark extract has
     no Swedish/Norwegian/German coastline).  Tiles that only contain cells outside the
     boundary (Faroe Islands) are copied from the original unchanged.

Without an original file (other countries: pass "-") the sea comes from the worldwide
land polygons (--land=<pkl> of land_extract.py): cell minus land in every tile touching
the boundary.

usage: compile_osmarea.py [--country=N] [--land=<land.pkl>] <area.pkl> <area.poly>
                          <original osmarea | -> <out chart> [YYYYMMDD]
"""
import collections
import pickle
import sys
import time

import numpy as np
import shapely
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import connected_components
from shapely.geometry import Polygon, box
from shapely.geometry.polygon import orient

import poly
from chart import tiles
from layers import build_c, geometry_parts, iter_records, parse_c, u16enc, u16str
from writer import cli, write_chart

G = 4                              # slot area C: 4x4 cells per tile
CELL = 32768                       # units per cell (360/2^25 deg)
BLOCK = 4096                       # objects are cut into 8x8 blocks per cell
MIN_AREA = 150                     # units^2 (~120 m2), smaller OSM polygons are dropped
TOLERANCE = 4                      # Douglas-Peucker, units
LEVEL_BASE = 33                    # hi = 14 - k for shorter bbox side >= 33 * 2^k (k <= 5)
SEA_LEVEL = 9

C3, WATER = ("c3", None), ("c6", None)
LANDUSE = {
    "residential": C3, "forest": 6, "scrub": 6,
    "farmland": 8, "farmyard": 8, "allotments": 8, "orchard": 8, "vineyard": 8,
    "meadow": 7, "grass": 7, "greenfield": 7, "village_green": 7,
    "greenhouse_horticulture": 7,
    "industrial": 4, "quarry": 4, "construction": 4, "railway": 4, "landfill": 4,
    "commercial": 1, "retail": 1, "military": 2, "cemetery": 3, "recreation_ground": 5,
    "basin": WATER, "reservoir": WATER, "aquaculture": WATER,
    "plant_nursery": 0, "brownfield": 0, "animal_keeping": 0, "fishfarm": 0,
    "garages": 0, "harbour": 0, "religious": 0, "scout_camp": 0}
NATURAL = {"water": WATER, "wetland": 10, "beach": 9, "heath": 6}
LEISURE = {"park", "pitch", "playground", "garden", "sports_centre", "golf_course", "track",
           "miniature_golf", "marina", "common", "dog_park", "stadium",
           "horse_riding", "recreation_ground", "water_park", "fitness_station",
           "swimming_pool", "fishing", "sports_hall", "outdoor_seating", "slipway",
           "soccer_golf", "disc_golf_course"}
MAN_MADE = {"pier": 11, "groyne": 16, "breakwater": 17}
WATER_SKIP = {"river"}             # natural=water + water=river is not in the original
# draw order of the c5 classes in a cell (roughly as in the original)
CLASS_ORDER = [4, 3, 10, 1, 0, 9, 11, 2, 5, 17, 8, 6, 7, 16]


def area_class(t):
    """tags -> ("c3", None) | ("c5", class) | ("c6", None) | None.  landuse wins."""
    for k, table in (("landuse", LANDUSE), ("natural", NATURAL)):
        v = table.get(t.get(k))
        if v is not None:
            if v == WATER and t.get("water") in WATER_SKIP:
                return None
            return v if isinstance(v, tuple) else ("c5", v)
    if t.get("leisure") in LEISURE:
        return ("c5", 5)
    if t.get("man_made") in MAN_MADE:
        return ("c5", MAN_MADE[t["man_made"]])
    if t.get("waterway") == "dock":
        return WATER
    return None


def level(geom):
    """Detail level from which a ring is drawn (the firmware skips parts with hi above
    the current level): 14 for small, down to 9 for large polygons."""
    x0, y0, x1, y1 = geom.bounds
    m, hi = min(x1 - x0, y1 - y0), 14
    for k in range(1, 6):
        if m >= LEVEL_BASE << k:
            hi = 14 - k
    return hi


def ring_area(r):
    x, y = r[:, 0].astype(float), r[:, 1].astype(float)
    return abs(np.sum(x[:-1] * y[1:] - x[1:] * y[:-1])) / 2


def polygons(g):
    """Polygon parts of any geometry."""
    return [p for p in shapely.get_parts(g) if p.geom_type == "Polygon" and not p.is_empty] \
        if g.geom_type in ("Polygon", "MultiPolygon", "GeometryCollection") else []


def valid(p):
    return p if p.is_valid else shapely.make_valid(p)


def load(areas, coast, P):
    """-> {(array, class): [Polygon]} inside the bounding box of P.  Islets (closed
    coastline ways with place=islet) are drawn as class 11 like piers.

    The areas are taken in the order of their osmium id, not in the order the
    extractor wrote them (which is the order libosmium flushes its buffers in):
    that order decides which polygon enters a union first and so ends up in the
    output, and the Rust port can only reproduce a defined one."""
    bx0, by0, bx1, by1 = P.bounds
    groups = collections.defaultdict(list)
    for aid, t, polys in sorted(areas, key=lambda a: a[0]):
        key = area_class(t)
        if key is None:
            continue
        for outer, inners in polys:
            if ring_area(outer) < MIN_AREA:
                continue
            if outer[:, 0].max() < bx0 or outer[:, 0].min() > bx1 or \
               outer[:, 1].max() < by0 or outer[:, 1].min() > by1:
                continue
            p = Polygon(outer, [i for i in inners if ring_area(i) >= MIN_AREA])
            groups[key] += polygons(valid(p))
    for wid, a, b, r, place in coast:
        if place == "islet" and a == b and len(r) >= 4 and ring_area(r) >= MIN_AREA:
            groups[("c5", 11)] += polygons(valid(Polygon(r)))
    return groups


def dissolve(lst):
    """Union of polygons, done per group of intersecting polygons (connected components
    of the STRtree intersection graph): the same result as one union_all, but minutes
    faster for the large classes (farmland 273 s -> 17 s)."""
    tree = shapely.STRtree(lst)
    a, b = tree.query(lst, predicate="intersects")
    n, label = connected_components(coo_matrix((np.ones(len(a)), (a, b)),
                                               shape=(len(lst), len(lst))), directed=False)
    comps = collections.defaultdict(list)
    for p, l in zip(lst, label):
        comps[l].append(p)
    out = []
    for c in comps.values():
        out += polygons(c[0] if len(c) == 1 else shapely.union_all(c))
    return out


def ring_points(coords):
    """Shapely ring -> closed list of int points without repeated neighbours."""
    out = []
    for x, y in coords:
        q = (int(round(x)), int(round(y)))
        if not out or out[-1] != q:
            out.append(q)
    if out and out[0] != out[-1]:
        out.append(out[0])
    return out if len(out) >= 4 else None


def cut(p, hi, P, add):
    """Simplify, clip to P and cut into blocks; add(block, hi, ring) for every ring."""
    s = p.simplify(TOLERANCE, preserve_topology=True)
    if not P.contains(s):
        s = s.intersection(P)
    for q in polygons(s):
        x0, y0, x1, y1 = q.bounds
        for bx in range(int(x0) // BLOCK, int(x1) // BLOCK + 1):
            for by in range(int(y0) // BLOCK, int(y1) // BLOCK + 1):
                r = (bx * BLOCK, by * BLOCK, bx * BLOCK + BLOCK, by * BLOCK + BLOCK)
                c = q if r[0] <= x0 and r[1] <= y0 and x1 <= r[2] and y1 <= r[3] \
                    else shapely.clip_by_rect(q, *r)
                for piece in polygons(c):
                    piece = orient(piece, 1.0)       # outer ring positive, holes negative
                    ring = ring_points(piece.exterior.coords)
                    if ring:
                        add((bx, by), hi, ring)
                    for h in piece.interiors:
                        ring = ring_points(h.coords)
                        if ring:
                            add((bx, by), level(Polygon(h)), ring)


def land_polygons(coast):
    """natural=coastline ways -> (land, water) polygon lists.  Land is left of the way, so
    closed rings running anticlockwise (lon/lat) are land, clockwise ones water in land.
    Open chains (they end far outside Denmark) are closed with a straight line."""
    chains, ends = {}, {}
    for wid, a, b, r, place in coast:
        seq, s, e = [r], a, b
        while e in chains and e != s:
            e2, sq = chains.pop(e)
            del ends[e2]
            seq, e = seq + sq, e2
        while s in ends and s != e:
            s0 = ends.pop(s)
            e0, sq = chains.pop(s0)
            seq, s = sq + seq, s0
        chains[s] = (e, seq)
        ends[e] = s
    land, water = [], []
    for s, (e, seq) in chains.items():
        pts = np.concatenate([seq[0]] + [q[1:] for q in seq[1:]]).astype(float)
        if len(pts) < 4:
            continue
        x, y = pts[:, 0], -pts[:, 1]                     # lon/lat orientation
        a = np.sum(x[:-1] * y[1:] - x[1:] * y[:-1]) + x[-1] * y[0] - x[0] * y[-1]
        (land if a > 0 else water).extend(polygons(valid(Polygon(pts))))
    return land, water


def sea_of_cell(cb, P, land, water):
    """Cell box minus land (coastline) within P (P None: whole cell);
    land/water = (polygons, STRtree)."""
    def part(ps):
        idx = ps[1].query(cb)
        return shapely.union_all([shapely.clip_by_rect(ps[0][i], *cb.bounds) for i in idx]) \
            if len(idx) else shapely.Polygon()
    ground = shapely.difference(part(land), part(water))
    sea = shapely.difference(cb if P is None else shapely.clip_by_rect(P, *cb.bounds), ground)
    return sea.simplify(TOLERANCE, preserve_topology=True)


def original_sea(orig):
    """Sea (#OW) of the original file per cell -> {cell: geometry}; its rings are
    combined with the even-odd rule like the firmware fills them."""
    sea = {}
    for tx, ty, cx, cy, g, raw in iter_records(orig, "C"):
        for it in parse_c(raw)["c6"]:
            if u16str(it["v"][0]) != "#OW":
                continue
            geos = []
            for hi, pts in geometry_parts(it["v"][1]):
                ring = [(cx * CELL + (q & 0xFFFF), cy * CELL + (q >> 16)) for q in pts]
                if len(ring) >= 4:
                    geos.append(valid(Polygon(ring)))
            sea[(cx, cy)] = xor_all(geos)
    return sea


def xor_all(geos):
    """Even-odd combination of many rings.  Pairwise in a tree: the small islands are
    combined with each other first and only once with the large sea ring (a sequential
    XOR took 7.5 minutes for Denmark)."""
    if not geos:
        return shapely.Polygon()
    while len(geos) > 1:
        geos = [shapely.symmetric_difference(a, b) for a, b in zip(geos[::2], geos[1::2])] + \
               ([geos[-1]] if len(geos) % 2 else [])
    return geos[0]


def original_records(orig, tiles):
    """Plaintext C records of the given tiles -> {tile: {slot: raw}}"""
    out = collections.defaultdict(dict)
    for tx, ty, cx, cy, g, raw in iter_records(orig, "C"):
        if (tx, ty) in tiles:
            out[(tx, ty)][(cx % g) * g + cy % g] = raw
    return out


def packed(cell, x, y):
    return (y - cell[1] * CELL) << 16 | (x - cell[0] * CELL)


def make_object(cell, name, cls, rings, arr):
    xs = [x for hi, r in rings for x, y in r]
    ys = [y for hi, r in rings for x, y in r]
    geom = []
    for hi, r in rings:
        geom.append(hi << 16 | len(r))
        geom += [packed(cell, x, y) for x, y in r]
    lo, hi_ = packed(cell, min(xs), min(ys)), packed(cell, max(xs), max(ys))
    nm = u16enc(name)
    if arr == "c5":
        return {"s": [0, len(nm), cls, lo, hi_, len(geom), 0], "v": [nm, tuple(geom)]}
    return {"s": [0, len(nm), lo, hi_, len(geom), 0], "v": [nm, tuple(geom)]}


def record(cell, objs):
    """objs: {"c3": [...], "c5": [...], "c6": [...]} -> plaintext C record"""
    size = {"c3": 0x18, "c5": 0x1C, "c6": 0x18}
    first = next(a for a in ("c5", "c6", "c3") if objs.get(a))
    hdr = [len(objs[first]) * size[first], cell[1]] + [0] * 13
    hdr[7], hdr[11], hdr[13] = (len(objs.get(a, [])) for a in ("c3", "c5", "c6"))
    return build_c({"hdr": hdr, "c1": [], "c2": [], "c4": [],
                    "c3": objs.get("c3", []), "c5": objs.get("c5", []), "c6": objs.get("c6", [])})


def build(extract, P, orig, date, log=print, land=None, country=4):
    t0 = time.time()
    groups = load(extract["areas"], extract["coast"], P)
    log(f"{sum(map(len, groups.values()))} polygons in {len(groups)} classes ({time.time() - t0:.0f} s)")

    blocks = collections.defaultdict(list)      # (arr, cls, block) -> [(hi, ring)]
    for key, lst in sorted(groups.items(), key=str):
        merged = dissolve(lst)
        for p in merged:
            cut(p, level(p), P, lambda b, hi, r, key=key: blocks[key + (b,)].append((hi, r)))
        log(f"  {key}: {len(lst)} -> {len(merged)} merged ({time.time() - t0:.0f} s)")

    # tiles: all tiles of the original plus those touching the boundary; tiles with no
    # cell inside the boundary (Faroe Islands) are copied
    otiles = {(x, y) for x, y, s, e in tiles(orig) if s != e} if orig else set()
    x0, y0, x1, y1 = P.bounds
    ptiles = {(cx // G, cy // G) for cx in range(int(x0) // CELL, int(x1) // CELL + 1)
              for cy in range(int(y0) // CELL, int(y1) // CELL + 1)
              if P.intersects(box(cx * CELL, cy * CELL, cx * CELL + CELL, cy * CELL + CELL))}
    copied = original_records(orig, otiles - ptiles) if orig else {}

    if land is None:
        land, water = land_polygons(extract["coast"])
        clip = P
    else:                          # worldwide land polygons: sea over the whole cell
        water, clip = [], None
    land, water = (land, shapely.STRtree(land)), (water, shapely.STRtree(water))
    sea_old = original_sea(orig) if orig else {}
    log(f"sea built ({time.time() - t0:.0f} s)")

    cells = collections.defaultdict(lambda: collections.defaultdict(list))
    for (arr, cls, (bx, by)), rings in blocks.items():
        cells[(bx // 8, by // 8)][arr].append((CLASS_ORDER.index(cls) if cls is not None else 0,
                                               by % 8, bx % 8, cls, rings))
    content = []
    for tx, ty in sorted(otiles | ptiles):
        if (tx, ty) in copied:
            content.append((tx, ty, {"C": copied[(tx, ty)]}, b""))
            continue
        slots = {}
        for k in range(G * G):
            cell = (tx * G + k // G, ty * G + k % G)
            cb = box(cell[0] * CELL, cell[1] * CELL, cell[0] * CELL + CELL, cell[1] * CELL + CELL)
            sea = shapely.union(sea_of_cell(cb, clip, land, water),
                                shapely.difference(sea_old.get(cell, shapely.Polygon()), P))
            objs = {}
            for arr in ("c3", "c5", "c6"):
                objs[arr] = [make_object(cell, "", cls, rings, arr)
                             for _, _, _, cls, rings in sorted(cells[cell][arr], key=lambda o: o[:3])]
            rings = []
            for q in polygons(sea):
                q = orient(q, 1.0)
                for r in [q.exterior] + [h for h in q.interiors if Polygon(h).area >= MIN_AREA]:
                    r = ring_points(r.coords)
                    if r:
                        rings.append((SEA_LEVEL, r))
            if rings:
                objs["c6"].insert(0, make_object(cell, "#OW", None, rings, "c6"))
            if any(objs.values()):
                slots[k] = record(cell, objs)
        content.append((tx, ty, {"C": slots}, b""))
    # the original directory starts with an empty placeholder tile (0,0)
    content.insert(0, (0, 0, None, b""))
    meta = {"date": date, "type": 1, "layer": 4, "country": country}
    log(f"writing {len(content)} tiles ({time.time() - t0:.0f} s)")
    return write_chart(meta, content)


if __name__ == "__main__":
    args, opts = cli(sys.argv[1:])
    if len(args) < 4:
        sys.exit(__doc__)
    src, area, orig, dst = args[:4]
    date = (args[4] if len(args) > 4 else time.strftime("%Y%m%d")).encode()
    rings = poly.load(area)
    P = shapely.union_all([Polygon([(x / 8, y / 8) for x, y in r]) for r, hole in rings if not hole])
    P = shapely.difference(P, shapely.union_all(
        [Polygon([(x / 8, y / 8) for x, y in r]) for r, hole in rings if hole] or [Polygon()]))
    land = pickle.load(open(opts["land"], "rb")) if "land" in opts else None
    d = build(pickle.load(open(src, "rb")), P, open(orig, "rb").read() if orig != "-" else None,
              date, log=lambda *a: print(*a, flush=True), land=land,
              country=int(opts.get("country", 4)))
    open(dst, "wb").write(d)
    print(f"{len(d)} B -> {dst}")
