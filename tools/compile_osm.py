"""Compile the Teasi osm layer (street net, names, routing graph) from OSM (OSM_FORMAT.md).

Input is the pickle of osm_extract.py.  Records built:
  D (32x32): a1 road edges, a3 unnamed lines, a4 named lines (waterways, seamark lines)
  A (4x4):   street name table referenced by a1[0]
  B (8x8):   routing graph (nodes = OSM nodes at junctions and way ends)
  C (4x4):   simplified overview lines of road classes 0-3 (+ c2/c4 copied from the original)
Tiles west of 0 deg without any new record (Faroe Islands) are copied from the original file;
their routing graph is self-contained.

Only cells touching the area polygon (+ BUFFER) are written.

B edge [2] (ascent in cm) is interpolated from the node heights of osm_heights.py or from
the elevation grid of dem_heights.py (--heights=<pkl>); without it the ascents are 0.

Other countries have no original file (pass "-"): c2/c4 stay empty and nothing is copied.
--country=N sets the header country index, --name the country name in the A records.

usage: compile_osm.py [--heights=<pkl>] [--country=N] [--name=<country>]
                      <ways.pkl> <area.poly> <original osm | -> <out> [YYYYMMDD]
"""
import collections
import gc
import math
import os
import pickle
import struct
import sys
import time
import unicodedata

import numpy as np
import shapely

import layers as L
import poly
from writer import write_chart

CELL = 32768                 # units per cell (D: 360/2^28 deg, A/C: 360/2^25)
MARGIN = 512                 # line layers store u = x + 512
BCELL = 65536                # B nodes: 65536 units (360/2^27 deg) per 8x8 cell
LEN_FACTOR = 5 * 2 ** 25 / (2 * math.pi * 6371000)   # length units per metre (~4.19)
SCALE = 2 ** 28 / 360.0
C_TOLERANCE = 32             # Douglas-Peucker tolerance of the overview lines (360/2^25)
BUFFER = 32768               # cells are kept if they touch the area polygon + BUFFER

# ---- road classes (a1[7] >> 27) and B way category (edge[0] bits 25-29) ---------------
CLASS = {"motorway": 0, "motorway_link": 0, "trunk": 1, "trunk_link": 1, "primary": 2,
         "secondary": 3, "primary_link": 3, "secondary_link": 4, "tertiary": 6,
         "tertiary_link": 6, "service": 7, "residential": 7, "unclassified": 7,
         "living_street": 7, "road": 7, "pedestrian": 8, "cycleway": 10, "footway": 11,
         "track": 12, "path": 13, "bridleway": 13, "steps": 14}
FERRY = 9
CATEGORY = {0: 18, 1: 18, 2: 18, 3: 18, 4: 18, 6: 10, 8: 16, 9: 6, 10: 16, 11: 16, 12: 5,
            13: 5, 14: 16}                       # class 7: service 6, others 10
NOT_ROUTED = {0}                                 # motorways: node ids 0, not in B

# B node[2]: left turns (crossing traffic), penalised by the router (FUN_003cf4d8: +90000).
# List of 6-bit entries (in | out << 3, edge indices of the node, 0 terminates), only at
# nodes with >= 3 car roads (class <= 7), only between car roads.
HEIGHT_K = 4                # inverse-distance interpolation from the 4 nearest known nodes

TURN_CLASS = 7
TURN_ANGLE = 35             # left turn: heading change > 35 deg (first segment, cos(lat))

PAVED = {"asphalt", "paved", "paving_stones", "concrete", "concrete:plates",
         "concrete:lanes", "chipseal"}                   # flags bits 7-8 = 1
COBBLES = {"sett", "cobblestone", "unhewn_cobblestone"}  # 2; no surface 3, others 0
RESTRICTED = {"private", "no", "destination", "customers", "forestry"}
CYCLE_ON_ROAD = {"track", "lane", "shared_lane", "separate", "opposite_lane",
                 "opposite_track", "opposite", "share_busway", "crossing"}

# ---- lines: a3 type / a4 type ---------------------------------------------------------
RAIL = {"rail", "light_rail", "subway", "narrow_gauge", "disused", "tram", "preserved",
        "miniature"}
A3_MAN_MADE = {"pier": 4, "groyne": 5, "breakwater": 6}
WATERWAY = {"stream", "river", "canal"}
SMALL_WATERWAY = {"ditch", "drain"}            # only when named


def road_class(t):
    if t.get("area") == "yes":
        return None
    if t.get("route") == "ferry" and "highway" not in t:
        return FERRY
    return CLASS.get(t.get("highway"))


def category(cls, t):
    if cls == 7:
        return 6 if t.get("highway") == "service" else 10
    return CATEGORY[cls]


def flags(t, rels):
    f = 0
    junction = t.get("junction") == "roundabout"
    f |= junction
    nets = {n for r, n, _ in rels if r in ("bicycle", "mtb")}
    f |= ("lcn" in nets) << 1 | ("rcn" in nets) << 2 | ("ncn" in nets) << 3
    f |= ("icn" in nets) << 27
    f |= any(r in ("hiking", "foot") for r, _, _ in rels) << 18
    f |= (t.get("access") in RESTRICTED) << 4
    f |= ("mtb:scale" in t) << 5
    f |= (t.get("highway") == "bridleway") << 6
    surface = t.get("surface")
    f |= (3 if surface is None else 1 if surface in PAVED else 2 if surface in COBBLES else 0) << 7
    cycle_road = t.get("cycleway") in CYCLE_ON_ROAD
    bicycle = t.get("bicycle")
    if bicycle == "no":
        b = 0
    elif bicycle == "designated" or cycle_road:
        b = 3
    elif bicycle in ("yes", "permissive"):
        b = 2
    else:
        b = 1
    f |= b << 9
    f |= (t.get("oneway") in ("yes", "true", "1") or junction) << 11
    try:
        lanes = int(t.get("lanes", "0"))
    except ValueError:
        lanes = 0
    f |= min(max(lanes, 0), 7) << 13
    f |= cycle_road << 29
    f |= (bicycle == "dismount") << 30
    f |= (t.get("foot") in ("yes", "designated", "permissive")) << 31
    return f


def passable(t, cls):
    """(forward, backward) passable for bicycles."""
    if cls == FERRY:
        return True, True
    ow = t.get("oneway")
    if t.get("oneway:bicycle") == "no" or t.get("cycleway") in ("opposite", "opposite_lane",
                                                                 "opposite_track"):
        return True, True
    if ow == "-1":
        return False, True
    if ow in ("yes", "true", "1") or t.get("junction") == "roundabout":
        return True, False
    return True, True


def multilingual(t):
    name = t.get("name")
    others = [(code, t[k]) for code, k in (("GER", "name:de"), ("ENG", "name:en"))
              if t.get(k) and t.get(k) != name]
    if not name or not others:
        return name or ""
    return "[" + "¦".join(["DAN" + name] + [c + n for c, n in others]) + "]"


def line_type(t):
    """-> ("a3", type) / ("a4", type, name) / None"""
    if t.get("railway") in RAIL:
        return ("a3", 0)
    if t.get("man_made") in A3_MAN_MADE:
        return ("a3", A3_MAN_MADE[t["man_made"]])
    if t.get("power") == "line":
        return ("a3", 10)
    if t.get("waterway") in WATERWAY or (t.get("waterway") in SMALL_WATERWAY and t.get("name")):
        return ("a4", 7, multilingual(t))
    st = t.get("seamark:type")
    if st in ("navigation_line", "recommended_track"):
        key = "navigation_line" if st == "navigation_line" else "recommended_track"
        o = t.get(f"seamark:{key}:orientation")
        try:
            o = "%d" % round(float(o)) if o else ""
        except ValueError:
            o = ""
        return ("a4", 8, o + "(leading)") if st == "navigation_line" else ("a4", 9, o + "(fixed_marks)")
    return None


# ---- geometry ---------------------------------------------------------------------------

def clip(pts, x0, y0, x1, y1):
    """Clip a polyline (list of (x, y) floats) to a closed rectangle -> list of parts."""
    parts, cur = [], []
    for (ax, ay), (bx, by) in zip(pts, pts[1:]):
        t0, t1, dx, dy = 0.0, 1.0, bx - ax, by - ay
        ok = True
        for p, q in ((-dx, ax - x0), (dx, x1 - ax), (-dy, ay - y0), (dy, y1 - ay)):
            if p == 0:
                if q < 0:
                    ok = False
                    break
            else:
                r = q / p
                if p < 0:
                    t0 = max(t0, r)
                else:
                    t1 = min(t1, r)
        if not ok or t0 > t1:
            if cur:
                parts.append(cur)
                cur = []
            continue
        a = (ax + t0 * dx, ay + t0 * dy)
        b = (ax + t1 * dx, ay + t1 * dy)
        if not cur:
            cur = [a]
        cur.append(b)
        if t1 < 1.0:
            parts.append(cur)
            cur = []
    if cur:
        parts.append(cur)
    return parts


def encode_parts(parts, ox, oy):
    """parts in global units -> geometry u32 list relative to (ox, oy) with the margin."""
    out = []
    for part in parts:
        pts = []
        for x, y in part:
            p = (int(round(y - oy)) + MARGIN) << 16 | (int(round(x - ox)) + MARGIN)
            if not pts or pts[-1] != p:
                pts.append(p)
        if len(pts) >= 2:
            out += [len(pts) << 16 | len(pts)] + pts
    return out


def cells_of(xs, ys):
    """32x32 cells whose margin-extended box the bbox of xs/ys touches."""
    cx0, cx1 = int((min(xs) - MARGIN) // CELL), int((max(xs) + MARGIN) // CELL)
    cy0, cy1 = int((min(ys) - MARGIN) // CELL), int((max(ys) + MARGIN) // CELL)
    return [(cx, cy) for cx in range(cx0, cx1 + 1) for cy in range(cy0, cy1 + 1)]


def place(pts, D, item_fn):
    """Put a polyline into every D cell it touches (clipped at the margin)."""
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    for cx, cy in cells_of(xs, ys):
        ox, oy = cx * CELL, cy * CELL
        if (min(xs) >= ox - MARGIN and max(xs) <= ox + CELL + MARGIN and
                min(ys) >= oy - MARGIN and max(ys) <= oy + CELL + MARGIN):
            parts = [pts]
        else:
            parts = clip(pts, ox - MARGIN, oy - MARGIN, ox + CELL + MARGIN, oy + CELL + MARGIN)
        geo = encode_parts(parts, ox, oy)
        if geo:
            D[(cx, cy)].append(item_fn(geo))


def heading(X, Y, a, b, forward):
    """Direction (degrees, counter-clockwise from east) in which an edge leaves its start."""
    idx = range(a, b + 1) if forward else range(b, a - 1, -1)
    it = iter(idx)
    i0 = next(it)
    x0, y0 = X[i0], Y[i0]
    f = math.cos(math.radians(90 - y0 / SCALE))
    dx = dy = 0.0
    for i in it:
        dx, dy = (X[i] - x0) * f, Y[i] - y0
        if dx or dy:
            break
    return math.degrees(math.atan2(-dy, dx))


def turns(out):
    """B node[2] from the node's outgoing edges [(words, class, heading)]."""
    car = [i for i, (_, c, _) in enumerate(out) if c <= TURN_CLASS]
    if len(car) < 3:
        return 0
    pairs = [(a, c) for a in car for c in car
             if a != c and (out[c][2] - out[a][2] - 180 + 180) % 360 - 180 > TURN_ANGLE]
    if len(pairs) > 5 or any(max(p) > 7 for p in pairs):
        return 0
    return sum((a | c << 3) << 6 * j for j, (a, c) in enumerate(pairs))


# ---- names ------------------------------------------------------------------------------

def sort_key(name):
    s = name.casefold().replace("æ", "ae").replace("ø", "o").replace("ß", "ss")
    s = "".join(c for c in unicodedata.normalize("NFKD", s) if not unicodedata.combining(c))
    return s, name


def name_table(names):
    """-> (blk5, blk6, {name: byte offset in blk5})"""
    words, woff, blk6, blk5, index = [], {}, bytearray(), [], {}
    for name in sorted(names, key=sort_key):
        ws = name.split()
        index[name] = 4 * len(blk5)
        for j, w in enumerate(ws):
            if w not in woff:
                woff[w] = len(blk6)
                enc = w.encode("utf-16le", "surrogatepass")
                blk6 += struct.pack("<H", len(enc) // 2) + enc
            blk5.append((len(ws) if j == 0 else 0) << 24 | woff[w])
    return struct.pack("<%dI" % len(blk5), *blk5), bytes(blk6), index


def build_a(cx, cy, names, country_name="Denmark"):
    blk5, blk6, index = name_table(names)
    country = L.u16enc(country_name)
    rec = {"hdr": [cx, cy, 0, 1, 0, 0, 0],
           "items": [{"s": [0, len(country), 2, 1, 0, 0, 0], "name": country,
                      "s18": [{"s": [0, 0, 0, len(set(names)), 0, 0], "str": ()}], "s10": []}],
           "blk5": blk5, "blk6": blk6}
    head = len(L.build_a(dict(rec, blk5=b"", blk6=b"")))
    rec["hdr"][5], rec["hdr"][6] = head, head + len(blk5)
    return L.build_a(rec), index


def d_header(rec):
    first = next((n * 4 * w for name, w, _ in L.D_ARRAYS if (n := len(rec[name]))), 0)
    return [first, 0, 0, len(rec["a1"]), 0, len(rec["a2"]), 0, len(rec["a3"]), 0,
            len(rec["a4"]), 0, len(rec["a5"]), 0]


# ---- main build -------------------------------------------------------------------------

def build(ext, P, orig, date, heights=None, log=print, country=4, name="Denmark"):
    """P: area polygon (shapely, 360/2^28 units); cells not touching P + BUFFER are dropped.

    Edges, graph nodes and their per-cell numbering are computed for the whole country
    with numpy; the B, D and A records are then built tile by tile, so the memory does
    not grow with the country (Great Britain).  The tag dicts of ext["ways"] are dropped
    once the way attributes are known (ext["ways"] is set to None)."""
    t0 = time.time()
    Pb = P.buffer(BUFFER)
    shapely.prepare(Pb)
    memo = {}

    def inside(cell, g):
        if (cell, g) not in memo:
            size = CELL * 32 // g
            x, y = cell[0] * size, cell[1] * size
            memo[(cell, g)] = Pb.intersects(shapely.box(x, y, x + size, y + size))
        return memo[(cell, g)]

    rels = ext["rels"]
    W, NID = ext["w"], ext["nid"]
    X, Y = ext["X"], ext["Y"]
    nw = len(ext["ways"])
    starts = np.r_[0, np.flatnonzero(W[1:] != W[:-1]) + 1]
    ends = np.r_[starts[1:], len(W)]
    wS, wE = np.full(nw, -1, np.int64), np.full(nw, -1, np.int64)
    wS[W[starts]], wE[W[starts]] = starts, ends

    # way attributes (roads: class, flags, category, passable, name; lines: type)
    cls = np.full(nw, -1, np.int8)
    fl = np.zeros(nw, np.int64)
    cat = np.zeros(nw, np.int8)
    fw, bw = np.zeros(nw, bool), np.zeros(nw, bool)
    wid = np.zeros(nw, np.int64)
    names, lines = [""] * nw, {}
    for k, (w_id, t) in enumerate(ext["ways"]):
        wid[k] = w_id
        if wS[k] < 0 or wE[k] - wS[k] < 2:
            continue
        c = road_class(t)
        if c is not None:
            cls[k], fl[k], cat[k] = c, flags(t, rels.get(w_id, [])), category(c, t)
            fw[k], bw[k] = passable(t, c)
            names[k] = t.get("name", "")
        else:
            lt = line_type(t)
            if lt:
                lines[k] = lt
    ext["ways"] = None
    gc.collect()
    log(f"{int((cls >= 0).sum())} roads, {len(lines)} lines ({time.time() - t0:.0f} s)")

    # segment lengths (haversine), cumulative per way-node array
    lon, lat = np.radians(X / SCALE - 180), np.radians(90 - Y / SCALE)
    dlat, dlon = np.diff(lat), np.diff(lon)
    h = np.sin(dlat / 2) ** 2 + np.cos(lat[:-1]) * np.cos(lat[1:]) * np.sin(dlon / 2) ** 2
    del lon, lat, dlat, dlon
    seg = 2 * 6371000 * np.arcsin(np.sqrt(h)) * LEN_FACTOR
    del h
    seg[W[1:] != W[:-1]] = 0
    cum = np.r_[0, np.cumsum(seg)]
    del seg

    # split points: nodes shared by roads, and way ends
    isroad = cls[W] >= 0
    road_idx = np.flatnonzero(isroad)
    uniq, cnt = np.unique(NID[road_idx], return_counts=True)
    split = np.isin(NID, uniq[cnt > 1])
    log(f"{int((cnt > 1).sum())} shared nodes ({time.time() - t0:.0f} s)")
    del uniq, cnt

    # ascent (cm) per way-node step, cumulative, from interpolated node heights
    cup = cdown = np.zeros(len(X) + 1)
    if heights is not None:
        H = np.zeros(len(X))
        if "grid" in heights:
            H[road_idx] = 100 * grid_sample(heights, X[road_idx], Y[road_idx])
        else:
            from scipy.spatial import cKDTree
            tree = cKDTree(np.c_[heights["X"], heights["Y"]])
            dist, near = tree.query(np.c_[X[road_idx], Y[road_idx]], k=HEIGHT_K)
            w = 1 / np.maximum(dist, 1) ** 2
            H[road_idx] = (w * heights["h"][near]).sum(1) / w.sum(1)
            del tree, dist, near, w
        dh = np.diff(H)
        del H
        dh[W[1:] != W[:-1]] = 0
        cup = np.r_[0, np.cumsum(np.maximum(dh, 0))]
        cdown = np.r_[0, np.cumsum(np.maximum(-dh, 0))]
        del dh
        log(f"heights interpolated ({time.time() - t0:.0f} s)")
    del road_idx

    # edges: consecutive cut points (way ends, split nodes) of a road way
    cutm = np.zeros(len(W), bool)
    cutm[starts], cutm[ends - 1] = True, True
    cutm &= isroad
    cutm |= isroad & split
    del split, isroad
    cp = np.flatnonzero(cutm)
    del cutm
    same = W[cp[1:]] == W[cp[:-1]]
    EA, EB = cp[:-1][same], cp[1:][same]
    del cp, same
    EK = W[EA]
    EC = cls[EK]
    routed = ~np.isin(EC, list(NOT_ROUTED))
    log(f"{len(EA)} edges ({time.time() - t0:.0f} s)")

    # graph nodes: numbered per B cell in (v, u, node id) order, starting at 1
    r = np.flatnonzero(routed)
    ends2 = np.c_[EA[r], EB[r]].ravel()
    gid, first = np.unique(NID[ends2], return_index=True)
    pos = ends2[first]
    del ends2, first
    gx, gy = X[pos] / 2, Y[pos] / 2
    del pos
    gbx, gby = (gx // BCELL).astype(np.int64), (gy // BCELL).astype(np.int64)
    # the original maps a cell onto 0..65535: floor(u * 65535/65536)
    gu = ((gx - gbx * BCELL) * (BCELL - 1) / BCELL).astype(np.int64)
    gv = ((gy - gby * BCELL) * (BCELL - 1) / BCELL).astype(np.int64)
    del gx, gy
    o = np.lexsort((gid, gu, gv, gby, gbx))
    new = np.r_[True, (gbx[o][1:] != gbx[o][:-1]) | (gby[o][1:] != gby[o][:-1])]
    ar = np.arange(len(o))
    gj = np.empty(len(o), np.int64)
    gj[o] = ar - np.maximum.accumulate(np.where(new, ar, 0)) + 1
    del o, new, ar
    bcells = {(int(a), int(b)) for a, b in np.unique(np.c_[gbx, gby], axis=0)}
    kb = {bc for bc in bcells if inside(bc, 8)}
    gkeep = np.array([(int(a), int(b)) in kb for a, b in zip(gbx, gby)], bool) \
        if len(gid) else np.zeros(0, bool)
    EU = np.zeros(len(EA), np.int64)
    EV = np.zeros(len(EA), np.int64)
    EU[r], EV[r] = np.searchsorted(gid, NID[EA[r]]), np.searchsorted(gid, NID[EB[r]])
    log(f"{len(gid)} graph nodes in {len(bcells)} B cells, {len(kb)} kept "
        f"({time.time() - t0:.0f} s)")

    # directed edges (both directions), per source node in edge order
    ok = routed & gkeep[EU] & gkeep[EV]
    far = ok & (np.maximum(np.abs(gbx[EU] - gbx[EV]), np.abs(gby[EU] - gby[EV])) > 63)
    for e in np.flatnonzero(far):
        log(f"way {wid[EK[e]]}: edge spans more than 63 B cells, not routed")
    ok &= ~far
    e_ok = np.flatnonzero(ok)
    HS = np.r_[EU[e_ok], EV[e_ok]]
    HE = np.r_[e_ok, e_ok]
    HF = np.r_[np.ones(len(e_ok), bool), np.zeros(len(e_ok), bool)]
    o = np.lexsort((~HF, HE, HS))
    HS, HE, HF = HS[o], HE[o], HF[o]
    del o, e_ok, ok, far

    def b_record(bc, nodes_idx):
        """B record of one cell; nodes_idx = graph node indices sorted by number."""
        nodes, bedges = [[0, 0, 0]], []
        for g in nodes_idx:
            out = []
            for hh in range(np.searchsorted(HS, g), np.searchsorted(HS, g, "right")):
                e, fwd = int(HE[hh]), bool(HF[hh])
                k, a, b = int(EK[e]), int(EA[e]), int(EB[e])
                c = int(cls[k])
                dst = int(EV[e] if fwd else EU[e])
                ln = min(int(round(cum[b] - cum[a])), 0xFFFFF)
                dx, dy = int(gbx[dst]) - bc[0], int(gby[dst]) - bc[1]
                w0 = ln | c << 21 | int(cat[k]) << 25 | int(fw[k] if fwd else bw[k]) << 30 | 1 << 31
                w3 = (dx + 64) << 25 | (dy + 64) << 18 | int(gj[dst])
                climb = int(round(cup[b] - cup[a] if fwd else cdown[b] - cdown[a]))
                out.append(([w0, int(fl[k]), climb, w3], c, heading(X, Y, a, b, fwd)))
            assert len(out) < 64
            nodes.append([len(bedges) | len(out) << 26, int(gv[g]) << 16 | int(gu[g]),
                          turns(out)])
            bedges += [e for e, _, _ in out]
        assert len(bedges) < 1 << 19
        return L.build_b({"hdr": [0, 0, 0, len(nodes), 0, len(bedges), 0, 0, 0, 0, 0],
                          "nodes": nodes, "edges": bedges, "b3": [], "extra": []})

    # bounding boxes of the edges and lines (for the tile selection of D)
    def bboxes(a, b):
        idx = np.c_[a, b + 1].ravel()
        pad = lambda v: np.r_[v, v[-1:]]
        return (np.minimum.reduceat(pad(X), idx)[::2], np.maximum.reduceat(pad(X), idx)[::2],
                np.minimum.reduceat(pad(Y), idx)[::2], np.maximum.reduceat(pad(Y), idx)[::2])
    ebox = bboxes(EA, EB) if len(EA) else [np.zeros(0)] * 4
    LK = np.array(sorted(lines), np.int64)
    lbox = bboxes(wS[LK], wE[LK] - 1) if len(LK) else [np.zeros(0)] * 4

    # tiles touching the area
    TS = CELL * 32
    x0, y0, x1, y1 = Pb.bounds
    tlist = [(tx, ty) for tx in range(int(x0 // TS), int(x1 // TS) + 1)
             for ty in range(int(y0 // TS), int(y1 // TS) + 1) if inside((tx, ty), 1)]
    gtx, gty = gbx // 8, gby // 8
    B, A, Drec = {}, {}, {}
    b_only = os.environ.get("B_ONLY")
    for n_t, (tx, ty) in enumerate(tlist):
        # B: routing graph
        sel = np.flatnonzero((gtx == tx) & (gty == ty) & gkeep)
        sel = sel[np.lexsort((gj[sel], gby[sel], gbx[sel]))]
        cells_b = collections.defaultdict(list)
        for g in sel:
            cells_b[(int(gbx[g]), int(gby[g]))].append(g)
        for bc, lst in cells_b.items():
            B[bc] = b_record(bc, lst)
        if b_only:
            continue

        # D: a1 items (name offsets filled in later), a3, a4
        tx0, ty0 = tx * TS - MARGIN, ty * TS - MARGIN
        tx1, ty1 = tx0 + TS + 2 * MARGIN, ty0 + TS + 2 * MARGIN

        def touching(box):
            return np.flatnonzero((box[1] >= tx0) & (box[0] <= tx1) & (box[3] >= ty0) &
                                  (box[2] <= ty1))
        D = collections.defaultdict(list)
        for e in touching(ebox):
            k, a, b = int(EK[e]), int(EA[e]), int(EB[e])
            c = int(cls[k])
            pts = list(zip(X[a:b + 1].tolist(), Y[a:b + 1].tolist()))
            if c in NOT_ROUTED:
                s_id = t_id = 0
            else:
                s_id, t_id = (int(gj[g]) if gkeep[g] else 0 for g in (EU[e], EV[e]))
            ln = min(int(round(cum[b] - cum[a])), 0xFFFFFF)
            head = (names[k], s_id, t_id, int(fl[k]), c << 27 | ln)
            place(pts, D, lambda geo, head=head: ("a1", head, geo))
        for j in touching(lbox):
            k = int(LK[j])
            lt = lines[k]
            pts = list(zip(X[wS[k]:wE[k]].tolist(), Y[wS[k]:wE[k]].tolist()))
            place(pts, D, lambda geo, lt=lt: (lt[0], lt[1:], geo))
        D = {cell: items for cell, items in D.items()
             if cell[0] // 32 == tx and cell[1] // 32 == ty and inside(cell, 32)}

        # A: name tables per 4x4 cell (all their D cells lie in this tile)
        anames = collections.defaultdict(set)
        for (cx, cy), items in D.items():
            for kind, head, _ in items:
                if kind == "a1" and head[0]:
                    anames[(cx // 8, cy // 8)].add(head[0])
        index = {}
        for ac, ns in anames.items():
            A[ac], index[ac] = build_a(ac[0], ac[1], ns, name)

        for (cx, cy), items in D.items():
            idx = index.get((cx // 8, cy // 8), {})
            rec = {"a1": [], "a2": [], "a3": [], "a4": [], "a5": []}
            for kind, head, geo in items:
                if kind == "a1":
                    nm, s_id, t_id, f, w7 = head
                    off = idx[nm] if nm else 0xFFFFFFFF
                    rec["a1"].append({"s": [off, off, 0xFFFF7FFF, 0xFFFF7FFF, s_id, t_id, f,
                                            w7, len(geo), 0], "v": [tuple(geo)]})
                elif kind == "a3":
                    rec["a3"].append({"s": [head[0], len(geo), 0], "v": [tuple(geo)]})
                else:
                    nm = L.u16enc(head[1])
                    rec["a4"].append({"s": [0, len(nm), head[0], len(geo), 0],
                                      "v": [nm, tuple(geo)]})
            rec["a1"].sort(key=lambda it: (it["s"][7] >> 27, it["s"][0]))
            rec["a3"].sort(key=lambda it: it["s"][0])
            rec["a4"].sort(key=lambda it: it["s"][2])
            rec["hdr"] = d_header(rec)
            Drec[(cx, cy)] = L.build_d(rec)
        del D
        log(f"  tile {tx},{ty} ({n_t + 1}/{len(tlist)}): {len(cells_b)} B, "
            f"{sum(1 for c in Drec if c[0] // 32 == tx and c[1] // 32 == ty)} D records "
            f"({time.time() - t0:.0f} s)")
    log(f"B built: {len(B)} records ({time.time() - t0:.0f} s)")
    if b_only:                                       # calibration: routing graph only
        pickle.dump(B, open(b_only, "wb"))
        sys.exit(0)
    log(f"A built: {len(A)} records, D built: {len(Drec)} records ({time.time() - t0:.0f} s)")

    # C: overview lines (classes 0-3) per 4x4 cell, in 360/2^25 units
    ocs = original_c(orig) if orig else {}
    Cit = collections.defaultdict(list)
    for c in (0, 1, 2, 3):
        segs = [shapely.LineString(np.c_[X[wS[k]:wE[k]] / 8, Y[wS[k]:wE[k]] / 8])
                for k in np.flatnonzero(cls == c)]
        if not segs:
            continue
        merged = shapely.line_merge(shapely.MultiLineString(segs))
        merged = shapely.simplify(merged, C_TOLERANCE, preserve_topology=False)
        lines_c = list(merged.geoms) if hasattr(merged, "geoms") else [merged]
        for ln in lines_c:
            pts = [tuple(p) for p in ln.coords]
            cs = collections.defaultdict(list)
            place(pts, cs, lambda geo: geo)
            for cell, geos in cs.items():
                if inside(cell, 4):
                    Cit[(cell, c)] += [w for geo in geos for w in geo]
    C1 = collections.defaultdict(list)
    for (cell, c), geo in sorted(Cit.items()):
        C1[cell].append({"s": [0xFFFFFFFF, c, len(geo), 0], "v": [tuple(geo)]})
    Crec = {}
    for cell in set(C1) | set(ocs):
        o = ocs.get(cell, {"c2": [], "c4": []})
        rec = {"c1": C1.get(cell, []), "c2": o["c2"], "c3": [], "c4": o["c4"], "c5": [],
               "c6": []}
        first = next((len(rec[n]) * 4 * w for n, w, _, _ in L.C_SPEC if rec[n]), 0)
        rec["hdr"] = [first, cell[1], 0, len(rec["c1"]), 0, len(rec["c2"]), 0, 0, 0,
                      len(rec["c4"]), 0, 0, 0, 0, 0]
        Crec[cell] = L.build_c(rec)
    log(f"C built: {len(Crec)} records ({time.time() - t0:.0f} s)")

    # tiles
    tiles = collections.defaultdict(lambda: {"A": {}, "B": {}, "C": {}, "D": {}})
    for area, recs, g in (("A", A, 4), ("B", B, 8), ("C", Crec, 4), ("D", Drec, 32)):
        for (cx, cy), raw in recs.items():
            tiles[(cx // g, cy // g)][area][(cx % g) * g + cy % g] = raw
    for tile, areas in (original_tiles(orig) if orig else {}).items():
        if tile not in tiles and tile[0] < 128:          # west of 0 deg: Faroe Islands
            tiles[tile] = areas
            log(f"tile {tile} copied from the original")
    out = [(x, y, tiles[(x, y)], b"") for x, y in sorted(tiles)]
    log(f"writing {len(out)} tiles ({time.time() - t0:.0f} s)")
    return write_chart({"date": date, "type": 1, "layer": 1, "country": country}, out)


def grid_sample(g, X, Y):
    """Bilinear heights (m) from an elevation grid {"grid": (rows, cols) array, north-west
    corner "lon0"/"lat0", "step" deg} at X, Y (360/2^28 units)."""
    c = ((X / SCALE - 180) - g["lon0"]) / g["step"]
    r = (g["lat0"] - (90 - Y / SCALE)) / g["step"]
    A = g["grid"]
    c = np.clip(c, 0, A.shape[1] - 1.001)
    r = np.clip(r, 0, A.shape[0] - 1.001)
    c0, r0 = c.astype(int), r.astype(int)
    fc, fr = c - c0, r - r0
    return ((A[r0, c0] * (1 - fc) + A[r0, c0 + 1] * fc) * (1 - fr)
            + (A[r0 + 1, c0] * (1 - fc) + A[r0 + 1, c0 + 1] * fc) * fr)


def original_c(orig):
    """c2/c4 (boundaries) of the original C records, by 4x4 cell."""
    out = {}
    for tx, ty, cx, cy, g, raw in L.iter_records(orig, "C"):
        r = L.parse_c(raw)
        if r["c2"] or r["c4"]:
            out[(cx, cy)] = {"c2": r["c2"], "c4": r["c4"]}
    return out


def original_tiles(orig):
    tiles = collections.defaultdict(lambda: {"A": {}, "B": {}, "C": {}, "D": {}})
    for area in "ABCD":
        g = L.AREAS[area][1]
        for tx, ty, cx, cy, _, raw in L.iter_records(orig, area):
            tiles[(tx, ty)][area][(cx % g) * g + cy % g] = raw
    return tiles


if __name__ == "__main__":
    from writer import cli
    args, opts = cli(sys.argv[1:])
    if len(args) < 4:
        sys.exit(__doc__)
    hts = pickle.load(open(opts["heights"], "rb")) if "heights" in opts else None
    ext = pickle.load(open(args[0], "rb"))
    P = shapely.union_all([shapely.Polygon(r) for r, hole in poly.load(args[1]) if not hole])
    orig = open(args[2], "rb").read() if args[2] != "-" else None
    date = (args[4] if len(args) > 4 else time.strftime("%Y%m%d")).encode()
    d = build(ext, P, orig, date, hts, log=lambda *a: print(*a, flush=True),
              country=int(opts.get("country", 4)), name=opts.get("name", "Denmark"))
    open(args[3], "wb").write(d)
    print(len(d), "B ->", args[3])
