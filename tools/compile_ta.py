"""Compile the Teasi ta layer (address search: places, streets, house numbers) from OSM.

Format: TA_FORMAT.md.  Inputs: the road pickle of osm_extract.py (as for compile_osm.py) and
the address/place pickle of osm_addr_extract.py.

  D (32x32): a1 street pieces: [0]/[1] name offsets (A record of the 4x4 parent cell),
             [2]/[3] house numbers left/right (from | to << 16, bit 15 = both parities),
             [4]/[5] node ids (per 8x8 cell), [6] flags, [7] class << 27 | length
  A (4x4):   one item (country) with one 0x18 sub element per place group: the '|'
             separated place names of the group, blk5 offset and count of its street names,
             lat/lon; blk5/blk6 = street names grouped by place
  index:     search tree over the place names and postcode districts (tools/ta_index.py),
             stored behind the records of the last tile (header 0x70)

Streets are the named roads of road classes motorway .. pedestrian, cut at junctions (like
compile_osm.py) and at the 32x32 cell borders, so every piece lies in one D cell and the
firmware's interpolation along the piece (FUN_00165a0c) uses the whole range.  House numbers
come from OSM addresses (addr:housenumber + addr:street) matched to the nearest piece with the
same name (< MAX_DIST), the side from the cross product (left = [2], checked against the
Danish original), from/to = numbers at the first/last address along the piece.

Places: every piece gets a settlement (city/town/village/hamlet: smallest distance / radius),
optionally a suburb (suburb/quarter/neighbourhood) and the addr:city values of its street
(up to 3, each >= 25 % of its addresses); pieces of one street (same name, ends < 60 m
apart) share the towns with >= 10 % of the street's length and the main suburb.  The group name is "Town, Suburb|Town|PostTown|...".  The search index has one result
per place name (type 0) with the 4x4 cells it occurs in, the other place nodes as type 2
(coordinates only) and the postcode districts (outward codes, type 1) with their streets.

usage: compile_ta.py [--country=N] [--name=<country>] [--cache=<pkl>] <ways.pkl> <addr.pkl>
                     <area.poly> <out> [YYYYMMDD]

--cache: the street pieces and matched house numbers (the slow part: loading the road
pickle, cutting, matching) are stored there and reused if the file exists.
"""
import collections
import gc
import math
import multiprocessing as mp
import os
import pickle
import re
import struct
import sys
import time

import numpy as np
import shapely
from scipy.spatial import cKDTree

import layers as L
import poly
import ta_index
from compile_osm import LEN_FACTOR, SCALE, road_class, sort_key
from writer import cli, write_chart

CELL = 32768                     # D cell in 360/2^28 units
WORKERS = 8                      # D record workers
MARGIN = 512
BCELL = 4 * CELL                 # node ids are numbered per 8x8 cell
ACELL = 8 * CELL
TA_CLASS = {0: 0, 1: 1, 2: 2, 3: 3, 4: 3, 6: 4, 7: 6, 8: 8}   # osm class -> ta class
FLAGS = 0x280                    # most common value in the original
NONE = 0xFFFF7FFF                # no house numbers
MAX_DIST = 100 / 0.149           # address -> street, units of 360/2^28 deg (~0.15 m N-S)
SETTLEMENT_R = {"city": 12000, "town": 5000, "village": 2000, "hamlet": 800}     # metres
SUBURB_R = {"suburb": 2500, "borough": 4000, "quarter": 1500, "neighbourhood": 800}
JOIN = 60 / 0.149                # pieces of one street: ends closer than 60 m (N-S units)
MAX_ALTS = 12                    # place names per street group
MAX_CITIES = 3                   # addr:city values per street ...
CITY_SHARE = 0.25                # ... with at least this share of its addresses
ALT_SHARE = 0.1                  # places of a street: at least this share of its length
CORR_TOL = 1e-12                 # a correlation this small carries no direction
OTHER_PLACES = {"locality", "isolated_dwelling", "farm", "island", "islet"}
M_PER_UNIT = 2 * math.pi * 6371000 / 2 ** 28          # N-S metres per unit
LANGS = [("DAN", 1), ("GER", 2), ("ENG", 3), ("FIN", 4), ("FRE", 5), ("NOR", 6), ("SPA", 7),
         ("SWE", 8), ("ITA", 9), ("DUT", 10), ("POR", 11)]
POSTCODE = re.compile(r"^([A-Z]{1,2}[0-9][A-Z0-9]?) ?[0-9][A-Z]{2}$")


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, flush=True)


def house_numbers(s):
    """'12' -> [12], '12a' -> [12], '12-16' -> [12, 16], 'Flat 3' -> []"""
    s = (s or "").strip()
    m = re.fullmatch(r"(\d+)\s*[A-Za-z]?\s*[-–/]\s*(\d+)\s*[A-Za-z]?", s)
    if m:
        return [int(m.group(1)), int(m.group(2))]
    m = re.match(r"(\d+)(?:\s*[A-Za-z]{0,2})?$", s)
    return [int(m.group(1))] if m else []


def split_cells(pts):
    """Cut a polyline (list of (x, y)) at the D cell borders -> [(cell, [points])]."""
    out = []
    cur, cell = [pts[0]], None
    for (ax, ay), (bx, by) in zip(pts, pts[1:]):
        # parameters where the segment crosses a vertical/horizontal cell border
        ts = []
        for a, b in ((ax, bx), (ay, by)):
            lo, hi = sorted((a, b))
            k0, k1 = math.floor(lo / CELL) + 1, math.floor(hi / CELL)
            ts += [(k * CELL - a) / (b - a) for k in range(k0, k1 + 1) if lo < k * CELL < hi]
        ts = sorted(set(ts)) + [1.0]
        t0 = 0.0
        for t in ts:
            mx, my = ax + (t0 + t) / 2 * (bx - ax), ay + (t0 + t) / 2 * (by - ay)
            c = (int(mx // CELL), int(my // CELL))
            p = (ax + t * (bx - ax), ay + t * (by - ay))
            if cell is not None and c != cell:
                out.append((cell, cur))
                cur = [cur[-1]]
            cell = c
            cur.append(p)
            t0 = t
    out.append((cell, cur))
    return [(c, p) for c, p in out if len(p) >= 2]


def plen(p):
    return sum(math.hypot(bx - ax, by - ay) for (ax, ay), (bx, by) in zip(p, p[1:]))


# ------------------------------------------------------------------------------------------
# 1. street pieces
# ------------------------------------------------------------------------------------------

def street_pieces(ext, inside):
    """-> list of pieces (name, ta class, cell, points, length units, start node, end node);
    start/end node = OSM node id or None at a cell border."""
    W, NID, X, Y = ext["w"], ext["nid"], ext["X"], ext["Y"]
    nw = len(ext["ways"])
    starts = np.r_[0, np.flatnonzero(W[1:] != W[:-1]) + 1]
    ends = np.r_[starts[1:], len(W)]
    cls = np.full(nw, -1, np.int8)
    names = {}
    for k, (w_id, t) in enumerate(ext["ways"]):
        c = road_class(t)
        if c is not None:
            cls[k] = c
            if c in TA_CLASS and t.get("name"):
                names[k] = t["name"].strip()
    ext["ways"] = None
    gc.collect()
    isroad = cls[W] >= 0
    uniq, cnt = np.unique(NID[isroad], return_counts=True)
    split = np.isin(NID, uniq[cnt > 1])
    del uniq, cnt
    cut = np.zeros(len(W), bool)
    cut[starts], cut[ends - 1] = True, True
    cut |= split
    named = np.zeros(nw, bool)
    named[list(names)] = True
    cut &= named[W]
    cp = np.flatnonzero(cut)
    same = W[cp[1:]] == W[cp[:-1]]
    EA, EB = cp[:-1][same], cp[1:][same]
    log(f"{len(names)} named streets, {len(EA)} edges")
    # metres along the ways (haversine) for the length field, scaled per piece
    lon, lat = np.radians(X / SCALE - 180), np.radians(90 - Y / SCALE)
    h = np.sin(np.diff(lat) / 2) ** 2 + \
        np.cos(lat[:-1]) * np.cos(lat[1:]) * np.sin(np.diff(lon) / 2) ** 2
    del lon, lat
    seg = 2 * 6371000 * np.arcsin(np.sqrt(h))
    del h
    seg[W[1:] != W[:-1]] = 0
    cum = np.r_[0, np.cumsum(seg)]
    del seg
    pieces = []
    for a, b in zip(EA.tolist(), EB.tolist()):
        k = int(W[a])
        pts = list(zip(X[a:b + 1].tolist(), Y[a:b + 1].tolist()))
        metres, planar = cum[b] - cum[a], plen(pts)
        parts = split_cells(pts)
        for j, (cell, p) in enumerate(parts):
            if not inside(cell):
                continue
            ln = metres * plen(p) / planar if planar else 0
            pieces.append((names[k], TA_CLASS[int(cls[k])], cell, p,
                           int(round(ln * LEN_FACTOR)),
                           int(NID[a]) if j == 0 else None,
                           int(NID[b]) if j == len(parts) - 1 else None))
    ext.clear()                                      # free the node arrays
    gc.collect()
    log(f"{len(pieces)} pieces")
    return pieces


# ------------------------------------------------------------------------------------------
# 2. addresses -> pieces
# ------------------------------------------------------------------------------------------

def match_addresses(pieces, addr, interp):
    """-> {piece index: [(t along piece 0..1, side 'L'/'R', number, city, outward code)]}"""
    pts = []                                        # (x, y, number, street, city, outward)
    for a in addr:
        x, y, hn, street, pc, city = a[:6]
        if not street:
            continue
        m = POSTCODE.match((pc or "").upper().strip())
        for n in house_numbers(hn):
            if 0 < n < 0x7FFF:
                pts.append((x, y, n, street.strip(), city, m.group(1) if m else None))
    for kind, nodes, street in interp:               # addr:interpolation ways
        step = {"odd": 2, "even": 2, "all": 1}.get(kind)
        ns = [(x, y, house_numbers(h)) for x, y, h, s in nodes]
        if not step or not street or any(len(n) != 1 for _, _, n in (ns[0], ns[-1])):
            continue
        n0, n1 = ns[0][2][0], ns[-1][2][0]
        if n0 == n1 or abs(n1 - n0) > 400 or (n1 - n0) % step:
            continue
        line = [(x, y) for x, y, _ in ns]
        total = plen(line)
        for i in range(1, abs(n1 - n0) // step):
            n = n0 + i * step * (1 if n1 > n0 else -1)
            d = total * i * step / abs(n1 - n0)
            for (ax, ay), (bx, by) in zip(line, line[1:]):
                s = math.hypot(bx - ax, by - ay)
                if d <= s:
                    f = d / s if s else 0
                    pts.append((ax + f * (bx - ax), ay + f * (by - ay), n, street.strip(), None,
                                None))
                    break
                d -= s
    log(f"{len(pts)} house numbers ({len(addr)} addresses, {len(interp)} interpolations)")

    by_cell = collections.defaultdict(list)          # (name, cell) -> piece indices
    for i, (name, _, cell, p, *_r) in enumerate(pieces):
        for dx in (-1, 0, 1):
            for dy in (-1, 0, 1):
                by_cell[(name.casefold(), (cell[0] + dx, cell[1] + dy))].append(i)
    groups = collections.defaultdict(list)
    for j, (x, y, n, street, city, pc) in enumerate(pts):
        groups[(street.casefold(), (int(x // CELL), int(y // CELL)))].append(j)
    res = collections.defaultdict(list)
    matched = 0
    for key, js in groups.items():
        cand = by_cell.get(key)
        if not cand:
            continue
        seg, owner, before, total = [], [], [], {}
        for i in cand:
            p = pieces[i][3]
            acc = 0.0
            for (ax, ay), (bx, by) in zip(p, p[1:]):
                seg.append((ax, ay, bx, by))
                owner.append(i)
                before.append(acc)
                acc += math.hypot(bx - ax, by - ay)
            total[i] = acc
        S = np.array(seg)
        P = np.array([(pts[j][0], pts[j][1]) for j in js])
        ax, ay, bx, by = S[:, 0], S[:, 1], S[:, 2], S[:, 3]
        dx, dy = bx - ax, by - ay
        l2 = np.maximum(dx * dx + dy * dy, 1e-9)
        t = np.clip(((P[:, :1] - ax) * dx + (P[:, 1:] - ay) * dy) / l2, 0, 1)
        d = np.hypot(P[:, :1] - ax - t * dx, P[:, 1:] - ay - t * dy)
        best = d.argmin(1)
        for r, j in enumerate(js):
            s = best[r]
            if d[r, s] > MAX_DIST:
                continue
            i = owner[s]
            cross = dx[s] * (P[r, 1] - ay[s]) - dy[s] * (P[r, 0] - ax[s])
            tt = (before[s] + t[r, s] * math.sqrt(l2[s])) / total[i] if total[i] else 0.5
            _, _, n, _, city, pc = pts[j]
            res[i].append((tt, "R" if cross > 0 else "L", n, city, pc))
            matched += 1
    log(f"{matched} house numbers matched to {len(res)} pieces")
    return res


def side_range(nums):
    """[(t, n)] of one side -> packed u32 (from | to << 16, bit 15 of from = both parities)."""
    if not nums:
        return NONE
    nums.sort()
    par = collections.Counter(n % 2 for _, n in nums)
    mixed = min(par.values()) > 0.1 * len(nums) if len(par) == 2 else False
    if not mixed and len(par) == 2:                  # drop the few of the other parity
        keep = par.most_common(1)[0][0]
        nums = [(t, n) for t, n in nums if n % 2 == keep]
    ns = [n for _, n in nums]
    ts = [t for t, _ in nums]
    lo, hi = min(ns), max(ns)
    # a correlation of 1e-17 is rounding noise, not a direction: with >= 0 the
    # last bit of numpy's covariance would decide whether the range counts up
    # or down (and no two implementations agree on that bit)
    up = len(ns) < 2 or np.corrcoef(ts, ns)[0, 1] >= -CORR_TOL \
        if len(set(ts)) > 1 and lo != hi else True
    a, b = (lo, hi) if up else (hi, lo)
    return (a | (0x8000 if mixed else 0)) | b << 16


# ------------------------------------------------------------------------------------------
# 3. places
# ------------------------------------------------------------------------------------------

def assign_places(pieces, hn, places):
    """-> per piece a tuple of (name, anchor) alternatives, anchor = (x, y) of the place node
    or None; the first alternative is the full name ("Town, Suburb")."""
    c = math.cos(math.radians(54))                   # x scale for the KD trees
    mid = np.array([p[3][len(p[3]) // 2] for p in pieces])
    lat_scale = np.cos(np.radians(90 - mid[:, 1] / SCALE))
    q = mid * [c, 1]

    def best(radii):
        """Per piece: (x, y, name) of the place with the smallest distance / radius < 1."""
        cands, score = [], np.full(len(pieces), np.inf)
        which = np.full(len(pieces), -1, np.int64)      # index into the concatenated cands
        for typ, rm in radii.items():
            cand = [(x, y, name) for x, y, t, name, _ in places if t == typ]
            if not cand:
                continue
            base = len(cands)
            cands += cand
            arr = np.array([(x, y) for x, y, _ in cand])
            r = rm / M_PER_UNIT
            d, idx = cKDTree(arr * [c, 1]).query(q, k=8, distance_upper_bound=1.2 * r)
            for kk in range(d.shape[1]):
                rows = np.flatnonzero(np.isfinite(d[:, kk]))
                j = idx[rows, kk]
                sc = np.hypot((arr[j, 0] - mid[rows, 0]) * lat_scale[rows],
                              arr[j, 1] - mid[rows, 1]) / r
                better = (sc < 1) & (sc < score[rows])
                score[rows[better]] = sc[better]
                which[rows[better]] = base + j[better]
        return [cands[w] if w >= 0 else None for w in which.tolist()]

    town, sub = best(SETTLEMENT_R), best(SUBURB_R)
    # the addr:city values (postal towns) of each street name in a 4x4 cell, anchored at the
    # nearest place node of that name (< 30 km)
    city = collections.defaultdict(collections.Counter)
    for i, lst in hn.items():
        key = (pieces[i][0], pieces[i][2][0] // 8, pieces[i][2][1] // 8)
        for *_x, cty, _pc in lst:
            if cty:
                city[key][cty.strip()] += 1
    by_name = collections.defaultdict(list)
    for x, y, typ, name, _ in places:
        by_name[name].append((x, y))
    trees = {}

    def anchor_of(name, x, y):
        if name not in by_name:
            return None
        if name not in trees:
            trees[name] = cKDTree(np.array(by_name[name]) * [c, 1])
        d, j = trees[name].query((x * c, y))
        return tuple(by_name[name][j]) if d * M_PER_UNIT < 30000 else None

    out = []
    for i, p in enumerate(pieces):
        t, s = town[i], sub[i]
        alts = []
        if t and s and s[2] != t[2]:
            alts.append((f"{t[2]}, {s[2]}", s[:2]))
        if t:
            alts.append((t[2], t[:2]))
        elif s:
            alts.append((s[2], s[:2]))
        cc = city.get((p[0], p[2][0] // 8, p[2][1] // 8))
        total = sum(cc.values()) if cc else 0
        for pc, n in (cc.most_common(MAX_CITIES) if cc else []):
            if n >= CITY_SHARE * total and pc not in [a for a, _ in alts]:
                alts.append((pc, anchor_of(pc, *mid[i])))
        out.append(tuple(alts))
    return out


def merge_streets(pieces, grp):
    """One group per street: pieces of the same name in a 4x4 cell whose ends are closer
    than JOIN are one street (union-find).  It gets the alternatives of all its pieces
    (ordered by length) except that only the main "Town, Suburb" is kept, so the street
    has one name offset in the cell and every town it passes lists it."""
    by = collections.defaultdict(list)
    for i, p in enumerate(pieces):
        by[(p[0], p[2][0] // 8, p[2][1] // 8)].append(i)
    out = list(grp)
    nstreets = 0
    for idx in by.values():
        parent = list(range(len(idx)))

        def root(k):
            while parent[k] != k:
                parent[k] = parent[parent[k]]
                k = parent[k]
            return k
        if len(idx) > 1:
            ends = np.array([pt for i in idx for pt in (pieces[i][3][0], pieces[i][3][-1])])
            for a, b in cKDTree(ends).query_pairs(JOIN):
                ra, rb = root(a // 2), root(b // 2)
                if ra != rb:
                    parent[ra] = rb
        comp = collections.defaultdict(list)
        for k, i in enumerate(idx):
            comp[root(k)].append(i)
        for members in comp.values():
            nstreets += 1
            w = collections.Counter()
            total = 0
            for i in members:
                total += pieces[i][4] + 1
                for alt in grp[i]:
                    w[alt] += pieces[i][4] + 1
            # the main "Town, Suburb" only (each suburb set would be a group of its own and
            # blow up the A records), the towns and postal towns with >= ALT_SHARE of the
            # street; canonical order, so equal sets give one group
            names, alts, sub = set(), [], None
            for k, (alt, n) in enumerate(w.most_common()):
                if alt[0] in names or len(alts) >= MAX_ALTS or (k and n < ALT_SHARE * total):
                    continue
                if ", " in alt[0]:
                    if sub is None:
                        sub = alt
                        names.add(alt[0])
                    continue
                names.add(alt[0])
                alts.append(alt)
            alts = ([sub] if sub else []) + sorted(alts, key=lambda a: (a[0], repr(a[1])))
            for i in members:
                out[i] = tuple(alts)
    log(f"{nstreets} streets")
    return out


# ------------------------------------------------------------------------------------------
# 4. records
# ------------------------------------------------------------------------------------------

def encode(p, cell):
    ox, oy = cell[0] * CELL, cell[1] * CELL
    pts = []
    for x, y in p:
        v = (min(max(int(round(y - oy)), -MARGIN), CELL + MARGIN) + MARGIN) << 16 | \
            (min(max(int(round(x - ox)), -MARGIN), CELL + MARGIN) + MARGIN)
        if not pts or pts[-1] != v:
            pts.append(v)
    if len(pts) < 2:
        pts.append(pts[0])
    return [len(pts) << 16 | len(pts)] + pts


def build_a(cx, cy, groups, country):
    """groups: [(alternatives string, (lat, lon), [street names])] -> (raw, {(gi, name): off})"""
    words, woff, blk6, blk5, index, s18 = [], {}, bytearray(), [], {}, []
    for gi, (alts, (lat, lon), names) in enumerate(groups):
        start = 4 * len(blk5)
        for name in sorted(set(names), key=sort_key):
            ws = name.split()
            index[(gi, name)] = 4 * len(blk5)
            for j, w in enumerate(ws):
                if w not in woff:
                    woff[w] = len(blk6)
                    enc = w.encode("utf-16le", "surrogatepass")
                    blk6 += struct.pack("<H", len(enc) // 2) + enc
                blk5.append((len(ws) if j == 0 else 0) << 24 | woff[w])
        st = L.u16enc(alts)
        fl = struct.unpack("<2I", struct.pack("<2f", lat, lon)) if alts else (0, 0)
        s18.append({"s": [0, len(st), start, len(set(names)), *fl], "str": st})
    cname = L.u16enc(country)
    rec = {"hdr": [cx, cy, 0, 1, 0, 0, 0],
           "items": [{"s": [0, len(cname), 1, len(s18), 0, 0, 0], "name": cname,
                      "s18": s18, "s10": []}],
           "blk5": struct.pack("<%dI" % len(blk5), *blk5), "blk6": bytes(blk6)}
    head = len(L.build_a(dict(rec, blk5=b"", blk6=b"")))
    rec["hdr"][5], rec["hdr"][6] = head, head + len(rec["blk5"])
    return L.build_a(rec), index


def d_record(job):
    """D record of one cell (worker): job = (cell, [(name offset, left numbers, right
    numbers, node id a, node id b, ta class, length, points)])."""
    cell, pcs = job
    items = []
    for off, left, right, ia, ib, tcls, ln, p in pcs:
        geo = encode(p, cell)
        items.append({"s": [off, off, side_range(left), side_range(right), ia, ib,
                            FLAGS, tcls << 27 | min(ln, 0xFFFFFF), len(geo), 0],
                      "v": [tuple(geo)]})
    items.sort(key=lambda it: (it["s"][7] >> 27, it["s"][0]))
    rec = {"a1": items, "a2": [], "a3": [], "a4": [], "a5": []}
    rec["hdr"] = [len(items) * 0x28, 0, 0, len(items), 0, 0, 0, 0, 0, 0, 0, 0, 0]
    return cell, L.build_d(rec)


def to_latlon(x, y):
    return 90 - y / SCALE, x / SCALE - 180


def sort_extract(ad):
    """Canonical order of the extractor's output.

    osm_addr_extract.py appends in libosmium's order, and that order decides
    `Counter` ties, the order of the index results and the summation order of
    the postcode centres -- it is not reproducible (and the Rust extractor
    reads the blocks in parallel).  Sorting makes the layer reproducible; the
    data is the same, only assembled in a defined order."""
    s = lambda v: v or ""
    ad["addr"].sort(key=lambda a: (a[0], a[1], *[s(v) for v in a[2:8]]))
    ad["places"].sort(key=lambda p: (p[0], p[1], s(p[2]), s(p[3])))
    ad.setdefault("interp", [])
    ad["interp"].sort(key=lambda i: (i[1][0][0], i[1][0][1], i[1][-1][0], i[1][-1][1],
                                     s(i[0]), s(i[2]), len(i[1])))


def sort_kids(nd):
    """Children of a search index node in character order.

    They used to be appended in the iteration order of a `set` of strings,
    which changes with the hash seed from run to run."""
    nd["kids"].sort(key=lambda k: k[0])
    for _, _, kid in nd["kids"]:
        sort_kids(kid)


def build(ext, ad, P, date, country=17, cname="United Kingdom", cache=None):
    """ext: function returning the road pickle (only called without a valid cache)."""
    t0 = time.time()
    sort_extract(ad)
    Pb = P.buffer(CELL)
    shapely.prepare(Pb)
    memo = {}

    def inside(cell):
        if cell not in memo:
            x, y = cell[0] * CELL, cell[1] * CELL
            memo[cell] = Pb.intersects(shapely.box(x, y, x + CELL, y + CELL))
        return memo[cell]

    if cache and os.path.exists(cache):
        pieces, hn = pickle.load(open(cache, "rb"))
        log(f"{len(pieces)} pieces, house numbers of {len(hn)} from {cache}")
    else:
        pieces = street_pieces(ext(), inside)
        hn = dict(match_addresses(pieces, ad["addr"], ad.get("interp", [])))
        if cache:
            pickle.dump((pieces, hn), open(cache, "wb"), protocol=5)
    places = [p for p in ad["places"] if p[2] in SETTLEMENT_R or p[2] in SUBURB_R
              or p[2] in OTHER_PLACES]
    grp = merge_streets(pieces, assign_places(pieces, hn, places))
    log(f"places assigned ({time.time() - t0:.0f} s)")

    # house numbers per piece and side, node ids per 8x8 cell
    ids = {}
    nid_count = collections.Counter()

    def node_id(key, x, y):
        if key not in ids:
            bc = (int(x // BCELL), int(y // BCELL))
            nid_count[bc] += 1
            ids[key] = nid_count[bc]
        return ids[key]

    # A: groups per 4x4 cell
    acells = collections.defaultdict(lambda: collections.defaultdict(list))
    for i, p in enumerate(pieces):
        acells[(p[2][0] // 8, p[2][1] // 8)][grp[i]].append(p[0])
    A, aindex, gidx = {}, {}, {}
    for ac, gs in acells.items():
        order = sorted(gs, key=lambda a: (a != (), [n for n, _ in a], repr(a)))  # place-less first
        groups = []
        for gi, alts in enumerate(order):
            anchor = next((an for _, an in alts if an), None)
            ll = to_latlon(*anchor) if anchor else (0.0, 0.0)
            groups.append(("|".join(n for n, _ in alts), ll, gs[alts]))
            gidx[(ac, alts)] = gi
        A[ac], aindex[ac] = build_a(ac[0], ac[1], groups, cname)
    log(f"A built: {len(A)} records ({time.time() - t0:.0f} s)")

    # D: pieces per 32x32 cell, built in parallel.  The workers get only the data of
    # their cells (forked workers sharing `pieces` would copy most of it: > 20 GB for GB).
    by_cell = collections.defaultdict(list)
    for i, (name, tcls, cell, p, ln, na, nb) in enumerate(pieces):
        ac = (cell[0] // 8, cell[1] // 8)
        sides = {"L": [], "R": []}
        for tt, sd, n, _c, _pc in hn.get(i, []):
            sides[sd].append((tt, n))
        by_cell[cell].append((aindex[ac][(gidx[(ac, grp[i])], name)], sides["L"], sides["R"],
                              node_id(na if na is not None else ("b", i, 0), *p[0]),
                              node_id(nb if nb is not None else ("b", i, 1), *p[-1]),
                              tcls, ln, p))

    def jobs():
        while by_cell:
            yield by_cell.popitem()
    with mp.get_context("forkserver").Pool(WORKERS) as pool:
        Drec = dict(pool.imap_unordered(d_record, jobs(), chunksize=16))
    log(f"D built: {len(Drec)} records ({time.time() - t0:.0f} s)")

    # search index: one result per place name and anchor node
    results = collections.defaultdict(set)           # (name, anchor) -> 4x4 cells
    for (ac, alts), gi in gidx.items():
        for name, anchor in alts:
            results[(name, anchor)].add(ac)
    res = []
    for (name, anchor), cells in results.items():
        if anchor:
            x, y = anchor
        else:
            x = (np.mean([cx for cx, _ in cells]) + 0.5) * ACELL
            y = (np.mean([cy for _, cy in cells]) + 0.5) * ACELL
        lat, lon = to_latlon(x, y)
        res.append({"type": 0, "pool": False, "name": name, "lon": lon, "lat": lat,
                    "cells": [(cx, cy, None) for cx, cy in sorted(cells)]})
    used = {(n, a) for n, a in results if a}
    for x, y, typ, name, tags in places:
        if (name, (x, y)) not in used:
            lat, lon = to_latlon(x, y)
            res.append({"type": 2, "pool": False, "name": name, "lon": lon, "lat": lat,
                        "cells": []})
            used.add((name, (x, y)))
    # postcode districts: streets with addresses of that outward code
    pcs = collections.defaultdict(lambda: {"xy": [], "cells": collections.defaultdict(set)})
    for i, lst in hn.items():
        name, _, cell, p = pieces[i][:4]
        ac = (cell[0] // 8, cell[1] // 8)
        off = aindex[ac][(gidx[(ac, grp[i])], name)]
        for *_x, pc in lst:
            if pc:
                pcs[pc]["cells"][ac].add(off)
                pcs[pc]["xy"].append(p[0])
    for pc, v in pcs.items():
        x, y = np.mean(v["xy"], axis=0)
        lat, lon = to_latlon(x, y)
        res.append({"type": 1, "pool": False, "name": pc, "lon": lon, "lat": lat,
                    "cells": [(cx, cy, sorted(o)) for (cx, cy), o in sorted(v["cells"].items())]})
    log(f"index: {sum(r['type'] == 0 for r in res)} places with streets, "
        f"{sum(r['type'] == 2 for r in res)} without, {len(pcs)} postcode districts")
    root = {"kids": [], "res": []}
    for r in res:
        for k in keys(r["name"]):
            nd = root
            for ch in k:
                nxt = next((c for c in nd["kids"] if c[0] == ch), None)
                if nxt is None:
                    nxt = (ch, ta_index.ALL, {"kids": [], "res": []})
                    nd["kids"].append(nxt)
                nd = nxt[2]
            nd["res"].append(r)
    sort_kids(root)
    index = ta_index.build({"langs": LANGS, "one": 1, "country": cname, "root": root,
                            "pool": {}})
    log(f"index {len(index)} B ({time.time() - t0:.0f} s)")

    del pieces, hn, grp, root
    gc.collect()
    tiles = collections.defaultdict(lambda: {"A": {}, "D": {}})
    for area, recs, g in (("A", A, 4), ("D", Drec, 32)):
        for (cx, cy), raw in recs.items():
            tiles[(cx // g, cy // g)][area][(cx % g) * g + cy % g] = raw
    order = sorted(tiles)
    last = order[-1]
    out = [(x, y, tiles[(x, y)], index if (x, y) == last else b"") for x, y in order]
    return write_chart({"date": date, "type": 2, "layer": 0x12, "country": country,
                        "tail_tile": last}, out)


def keys(name):
    """Search keys of a place name: every word and every multi-word part (TA_FORMAT.md)."""
    out = set()
    for comp in name.split(", "):
        f = ta_index.fold(comp).replace("'", "").replace("’", "")
        f = re.sub(r"[^a-z0-9]+", " ", f)
        ws = f.split()
        out |= set(ws)
        if len(ws) > 1:
            out.add(" ".join(ws))
    return out


if __name__ == "__main__":
    args, opts = cli(sys.argv[1:])
    if len(args) < 4:
        sys.exit(__doc__)
    ad = pickle.load(open(args[1], "rb"))
    P = shapely.union_all([shapely.Polygon(r) for r, hole in poly.load(args[2]) if not hole])
    date = (args[4] if len(args) > 4 else time.strftime("%Y%m%d")).encode()
    d = build(lambda: pickle.load(open(args[0], "rb")), ad, P, date,
              int(opts.get("country", 17)), opts.get("name", "United Kingdom"),
              opts.get("cache"))
    open(args[3], "wb").write(d)
    print(len(d), "B ->", args[3])
