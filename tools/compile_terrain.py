"""Build a terrain chart (type 5): elevation model (height profile) and map images.

The terrain layer holds JPEG map tiles and JPEG 2000 elevation tiles (TERRAIN_FORMAT.md).
Heights come from dem_heights.py.  Without --land/--area only the elevation tiles are
written (all JPEG cells empty, like region (122,20) of Denmark_terrain).

Per region (1.40625 deg tile x, y): 8x8 cells, cell k at column k // 8 from west and row
k % 8 from north, 256x256 px each, row 0 = north.  A cell gets an elevation tile if it
contains land (height > 0).

Map images: shaded relief (light from NW) coloured by land cover -- sea from the land
polygons (land_extract.py), water/forest/heath/rock/urban/farmland from the OSM areas
(osm_area_extract.py).  JPEG cells 0-63 = 8x8 (level 0), 64-79 = 4x4, 80-83 = 2x2, 84 = 1x1,
all column by column from NW; level-0 cells touching <area.poly> or with land get an
image, a coarser cell if one of its 4 children has one.  Record = [u16 a0][u16 a1][u32 n][JP2, n bytes][u16], height in m =
(v / a0 + a1) / 6 - 1000.  The JP2 imitates the originals (JasPer 1.701): one tile,
5 decomposition levels, 9/7 irreversible, 64x64 code blocks, LRCP, one layer.

usage: compile_terrain.py [--country=N] [--rate=R] [--land=<land.pkl> --area=<area.pkl>]
                          [--only=x,y] <dem.pkl> <area.poly> <out> [date]
(--only: just one region, for trying out the rendering)
"""
import io
import multiprocessing
import math
import os
import pickle
import struct
import sys
import time
from concurrent.futures import ProcessPoolExecutor

import numpy as np
import shapely
from PIL import Image, ImageDraw

import poly
from chart import DEVICE, header_md5, global_key
from pc1 import encrypt_blob
from writer import cli

TILE = 1.40625
CS = TILE / 8
N = 256
RATE = 50                       # compression ratio (originals ~100, ours ~2.5 KB/tile)
MAX_A0 = 9999
HEAD = 0x159C
# jP, ftyp, jp2h (ihdr 256x256x1 16 bit, colr), res  and the jp2c box header (length 0 =
# up to the end), copied from Denmark_terrain.v20210916
JP2_HEAD = bytes.fromhex(
    "0000000c6a5020200d0a870a00000014667479706a703220000000006a703220000000476a703268"
    "0000001669686472000001000000010000010f0700000000000f636f6c7201000000000011000000"
    "1a7265732000000012726573634890fffc4890fffc0404000000006a703263")
# the EXIF APP1 of the original JPEGs (GDI+ style: 96 dpi, no tile data)
EXIF = bytes.fromhex(
    "45786966000049492a000800000003001a01050001000000320000001b010500010000003a000000"
    "2801030001000000020053000000000000703839809698000070383980969800")
JPEG_Q = 80                     # quantisation tables of the originals = IJG quality 80
R = 2048                        # region image size (8 cells)
UNITS = 64                      # 360/2^25 deg units (land/area pickles) per pixel

# land cover classes, painted in this order; colours after the originals
SEA, LAND, FARM, URBAN, HEATH, FOREST, WET, ROCK, SAND, WATER = range(10)
COLOURS = np.array([(150, 180, 206), (226, 227, 203), (230, 226, 196), (232, 216, 198),
                    (212, 219, 172), (184, 205, 148), (204, 218, 196), (224, 221, 214),
                    (238, 231, 200), (150, 180, 206)], np.float32)
COVER = {
    ("landuse", "farmland"): FARM, ("landuse", "farmyard"): FARM, ("landuse", "orchard"): FARM,
    ("landuse", "vineyard"): FARM, ("landuse", "allotments"): FARM,
    ("landuse", "residential"): URBAN, ("landuse", "commercial"): URBAN,
    ("landuse", "industrial"): URBAN, ("landuse", "retail"): URBAN,
    ("landuse", "construction"): URBAN, ("landuse", "railway"): URBAN,
    ("landuse", "brownfield"): URBAN, ("landuse", "quarry"): ROCK,
    ("natural", "heath"): HEATH, ("natural", "scrub"): HEATH, ("natural", "grassland"): HEATH,
    ("natural", "fell"): HEATH, ("natural", "moor"): HEATH,
    ("landuse", "forest"): FOREST, ("natural", "wood"): FOREST,
    ("natural", "wetland"): WET, ("natural", "bare_rock"): ROCK, ("natural", "scree"): ROCK,
    ("natural", "glacier"): ROCK, ("natural", "shingle"): SAND, ("natural", "sand"): SAND,
    ("natural", "beach"): SAND, ("natural", "water"): WATER, ("waterway", "riverbank"): WATER,
    ("landuse", "reservoir"): WATER, ("landuse", "basin"): WATER,
}


def sample(g, lat, lon):
    """bilinear heights of the grid at lat (rows) x lon (cols)"""
    G = g["grid"]
    r = (g["lat0"] - lat) / g["step"]
    c = (lon - g["lon0"]) / g["step"]
    r = np.clip(r, 0, G.shape[0] - 1.001)
    c = np.clip(c, 0, G.shape[1] - 1.001)
    r0, c0 = r.astype(int), c.astype(int)
    fr, fc = (r - r0)[:, None], (c - c0)[None, :]
    a = G[np.ix_(r0, c0)]
    b = G[np.ix_(r0, c0 + 1)]
    d = G[np.ix_(r0 + 1, c0)]
    e = G[np.ix_(r0 + 1, c0 + 1)]
    h = (a * (1 - fc) + b * fc) * (1 - fr) + (d * (1 - fc) + e * fc) * fr
    rows, cols = G.shape
    h[(lat > g["lat0"]) | (lat < g["lat0"] - rows * g["step"]), :] = 0      # outside the grid
    h[:, (lon < g["lon0"]) | (lon > g["lon0"] + cols * g["step"])] = 0
    return h


def encode(h, rate):
    """heights (256x256, m) -> record bytes (without the trailing u16)"""
    u = 6 * (h.astype(np.float64) + 1000)
    a1 = int(math.floor(u.min()))
    span = math.ceil(u.max()) - a1
    a0 = max(1, min(MAX_A0, 65535 // max(span, 1)))
    v = np.clip(np.round((u - a1) * a0), 0, 65535).astype(np.uint16)
    b = io.BytesIO()
    Image.fromarray(v).save(b, "JPEG2000", no_jp2=True, irreversible=True, num_resolutions=6,
                            codeblock_size=(64, 64), progression="LRCP",
                            quality_mode="rates", quality_layers=[rate])
    jp2 = JP2_HEAD + b.getvalue()
    return a0, a1, jp2


G = None                        # elevation grid, shared with the forked workers
REG = {}                        # (x, y) -> {"cells", "land", "areas"} for the images


def cover(x, y, reg):
    """land cover class per pixel (R x R, uint8) of region x, y"""
    im = Image.new("L", (R, R), SEA)
    dr = ImageDraw.Draw(im)
    X0, Y0 = x * R * UNITS, y * R * UNITS

    def px(a):
        a = (np.asarray(a, np.float64) - (X0, Y0)) / UNITS - 0.5
        return [tuple(p) for p in a]

    for p in reg["land"]:
        for q in getattr(p, "geoms", [p]):
            dr.polygon(px(q.exterior.coords), fill=LAND)
            for h in q.interiors:
                dr.polygon(px(h.coords), fill=SEA)
    for c, outer, inners in sorted(reg["areas"], key=lambda a: a[0]):
        if len(outer) >= 3:
            dr.polygon(px(outer), fill=c)
        for h in inners:
            if len(h) >= 3:
                dr.polygon(px(h), fill=LAND)
    return np.array(im)


def shade(h, lat):
    """hillshade (flat = 1) of heights h (R+2 x R+2, 1 px margin), light from NW at 45 deg"""
    dy = CS / N * 111320.0
    dx = dy * math.cos(math.radians(lat))
    z = 1.5                                          # relief exaggeration
    gy, gx = np.gradient(h * z, dy, dx)              # gy: towards south
    nx, ny, nz = -gx, gy, np.ones_like(h)            # normal, y = north
    az, alt = math.radians(315), math.radians(45)
    lx, ly, lz = math.cos(alt) * math.sin(az), math.cos(alt) * math.cos(az), math.sin(alt)
    s = (nx * lx + ny * ly + nz * lz) / np.sqrt(nx * nx + ny * ny + 1) / lz
    return s[1:-1, 1:-1]


def jpeg(img):
    b = io.BytesIO()
    img.save(b, "JPEG", quality=JPEG_Q, subsampling=2, dpi=(96, 96), exif=EXIF)
    return b.getvalue()


def images(x, y, h, reg):
    """-> [JPEG bytes or None] for the 85 cells"""
    cls = cover(x, y, reg)
    sh = np.clip(1 + 0.7 * (shade(h, 90 - (y + 0.5) * TILE) - 1), 0.4, 1.12)
    water = (cls == SEA) | (cls == WATER)
    sh[water] = 1.0
    rgb = COLOURS[cls] * sh[..., None]
    img = Image.fromarray(np.clip(rgb, 0, 255).astype(np.uint8))
    have = [k in reg["cells"] or (cls[(k % 8) * N:(k % 8 + 1) * N, (k // 8) * N:(k // 8 + 1) * N]
                                   != SEA).any() for k in range(64)]
    out = [None] * 85
    base = 0
    for g in (8, 4, 2, 1):
        lvl = img if g == 8 else img.resize((g * N, g * N), Image.LANCZOS)
        for k in range(g * g):
            c, r = k // g, k % g
            if have[k]:
                out[base + k] = jpeg(lvl.crop((c * N, r * N, (c + 1) * N, (r + 1) * N)))
        if g > 1:
            have = [any(have[(2 * (k // (g // 2)) + dc) * g + 2 * (k % (g // 2)) + dr]
                        for dc in (0, 1) for dr in (0, 1)) for k in range(g * g // 4)]
        base += g * g
    return out


def region(args):
    x, y, rate = args
    g = G
    lonW, latN = x * TILE - 180, 90 - y * TILE
    # heights at the pixel centres, with a margin of 1 px for the hillshade
    lat = latN - (np.arange(-1, R + 1) + 0.5) / R * TILE
    lon = lonW + (np.arange(-1, R + 1) + 0.5) / R * TILE
    hm = sample(g, lat, lon)
    h = hm[1:-1, 1:-1]
    jpgs = images(x, y, hm, REG[(x, y)]) if (x, y) in REG else [None] * 85
    t2 = [0xFFFFFFFF] * 85
    recs = []
    for k in range(64):
        col, row = k // 8, k % 8
        c = h[row * N:(row + 1) * N, col * N:(col + 1) * N]
        if c.max() <= 0.5:
            continue
        a0, a1, jp2 = encode(c, rate)
        recs.append([k, a0, a1, jp2])
    if not recs and not any(jpgs):
        return x, y, None, 0
    body = bytearray()
    t1 = []
    for j in jpgs:
        if j is None:
            t1.append((0xFFFFFFFD, 0))
        else:
            t1.append((HEAD + len(body), len(j)))
            body += j
    for i, (k, a0, a1, jp2) in enumerate(recs):
        t2[k] = HEAD + len(body)
        # the originals end each record with 2 stale bytes, mostly the next record's a0
        nxt = recs[i + 1][1] if i + 1 < len(recs) else a0
        body += struct.pack("<HHI", a0, a1, len(jp2)) + jp2 + struct.pack("<H", nxt)
    head = b"\xff" * 0x1180 + b"".join(struct.pack("<II", *t) for t in t1) \
        + struct.pack("<85I", *t2)
    return x, y, (head, bytes(body)), max([len(r[3]) for r in recs] or [0])


def prepare(rings, land, areas, xs, ys, log):
    """bin land polygons and land-cover areas into the regions -> REG"""
    area = shapely.Polygon([(p[0] / 8, p[1] / 8) for p in rings[0][0]])
    for r, hole in rings[1:]:
        g = shapely.Polygon([(p[0] / 8, p[1] / 8) for p in r])
        area = area.difference(g) if hole else area.union(g)
    shapely.prepare(area)
    S = R * UNITS
    for x in xs:
        for y in ys:
            cells = set()
            for k in range(64):
                c, r = k // 8, k % 8
                if area.intersects(shapely.box(x * S + c * S / 8, y * S + r * S / 8,
                                               x * S + (c + 1) * S / 8, y * S + (r + 1) * S / 8)):
                    cells.add(k)
            REG[(x, y)] = {"cells": cells, "land": [], "areas": []}
    for p in land:
        x0, y0, x1, y1 = p.bounds
        for x in range(int(x0 // S), int(x1 // S) + 1):
            for y in range(int(y0 // S), int(y1 // S) + 1):
                if (x, y) in REG:
                    REG[(x, y)]["land"].append(p)
    n = 0
    for aid, tags, polys in areas:
        c = next((COVER[kv] for kv in tags.items() if kv in COVER), None)
        if c is None:
            continue
        for outer, inners in polys:
            x0, y0 = outer.min(0)
            x1, y1 = outer.max(0)
            if x1 - x0 < UNITS and y1 - y0 < UNITS:      # smaller than a pixel
                continue
            for x in range(int(x0 // S), int(x1 // S) + 1):
                for y in range(int(y0 // S), int(y1 // S) + 1):
                    if (x, y) in REG:
                        REG[(x, y)]["areas"].append((c, outer, inners))
                        n += 1
    log(f"  {len(land)} land polygons, {n} land-cover areas")


def build(g, rings, date, country=17, rate=RATE, device=DEVICE, log=print,
          land=None, areas=None, only=None):
    global G
    G = g
    lon = [p[0] / 2 ** 28 * 360 - 180 for r, _ in rings for p in r]
    lat = [90 - p[1] / 2 ** 28 * 360 for r, _ in rings for p in r]
    xs = range(int((min(lon) + 180) // TILE), int((max(lon) + 180) // TILE) + 1)
    ys = range(int((90 - max(lat)) // TILE), int((90 - min(lat)) // TILE) + 1)
    if only:
        xs, ys = [only[0]], [only[1]]
    if land is not None:
        prepare(rings, land, areas or [], xs, ys, log)
    jobs = [(x, y, rate) for x in xs for y in ys]
    key = global_key(device)
    out, maxlen = [], 0
    t = time.time()
    with ProcessPoolExecutor(mp_context=multiprocessing.get_context("fork")) as ex:
        for x, y, parts, ml in ex.map(region, jobs):
            if parts is None:
                continue
            head, body = parts
            ndem = sum(1 for k in range(85) if struct.unpack_from("<I", head, 0x1428 + 4 * k)[0] != 0xFFFFFFFF)
            njpg = sum(1 for k in range(85) if struct.unpack_from("<I", head, 0x1180 + 8 * k)[0] != 0xFFFFFFFD)
            log(f"  ({x},{y}) {njpg} images, {ndem} heights, {len(body) // 1024} KB  [{time.time() - t:.0f} s]")
            out.append((x, y, head + encrypt_blob(os.urandom(32), key) + body))
            maxlen = max(maxlen, ml)
    n = len(out)
    off = 0x78 + 8 * n
    dirs = []
    for x, y, blob in out:
        dirs.append(struct.pack("<HHI", x, y, off))
        off += len(blob)
    hdr = bytearray(struct.pack("<I", 0x1B62) + os.urandom(48) + bytes(16))
    hdr += date + struct.pack("<3I", 5, 0x20, country)
    hdr += struct.pack("<5I", 0, 0, 0, 0, 0) + struct.pack("<III", maxlen, 0xFFFFFFFF, n)
    d = bytearray(hdr + b"".join(dirs) + b"".join(b for _, _, b in out))
    d[0x34:0x44] = header_md5(bytes(d), device)
    return bytes(d)


if __name__ == "__main__":
    args, opts = cli(sys.argv[1:])
    if len(args) < 3:
        sys.exit(__doc__)
    g = pickle.load(open(args[0], "rb"))
    land = pickle.load(open(opts["land"], "rb")) if "land" in opts else None
    areas = pickle.load(open(opts["area"], "rb"))["areas"] if "area" in opts else None
    only = tuple(map(int, opts["only"].split(","))) if "only" in opts else None
    date = (args[3] if len(args) > 3 else time.strftime("%Y%m%d")).encode()
    d = build(g, poly.load(args[1]), date, int(opts.get("country", 17)),
              float(opts.get("rate", RATE)), log=lambda *a: print(*a, flush=True),
              land=land, areas=areas, only=only)
    open(args[2], "wb").write(d)
    print(len(d), "B ->", args[2])
