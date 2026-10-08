"""Parser for the decrypted record contents of the Teasi vector layers.

Formats: see OSM*_FORMAT.md / TA_FORMAT.md.  The outer container (header, tile
directory, PC1, LZMA) is handled by chart.py.
"""
import struct

from chart import DEVICE, tiles, decode_record, global_key

# Index slot areas in the tile head (offsets relative to tile start).
# Slot k of a 4x4 area belongs to sub cell (x*4 + k//4, y*4 + k%4); analogous
# for 8x8 and 32x32.  0xFFFFFFFF = empty, otherwise record offset in the tile.
AREAS = {"A": (0x000, 4), "B": (0x040, 8), "C": (0x140, 4), "D": (0x180, 32)}



def cell_origin(cx, cy, grid):
    """North-west corner (lat, lon) of sub cell (cx, cy); grid = cells per tile."""
    size = 1.40625 / grid
    return 90 - cy * size, cx * size - 180


# Line layers (osm D/C, ta D) store coordinates with a margin of 512 units
# around the cell (value range 0..33792 = -512..33280); point and area layers
# (osmpoi, osmpoint, osmarea) have no margin.  Verified against OSM nodes.
MARGIN = 512


def to_latlon(cx, cy, grid, packed, margin=0):
    """packed = (v << 16) | u: u east / v south of the cell's NW corner.

    One cell is always 32768 units, so a unit is 360/2^25 deg for 4x4 cells and
    360/2^28 deg for 32x32 cells.  Pass margin=MARGIN for osm and ta lines."""
    lat0, lon0 = cell_origin(cx, cy, grid)
    unit = 1.40625 / grid / 32768
    u, v = (packed & 0xFFFF) - margin, (packed >> 16) - margin
    return lat0 - v * unit, lon0 + u * unit


def b_node_latlon(cx, cy, pos):
    """Position of a B node (routing graph, 8x8 cells): 65536 units per cell,
    no margin (unit 360/2^27 deg; matches the OSM node within 1-2 units)."""
    lat0, lon0 = cell_origin(cx, cy, 8)
    unit = 1.40625 / 8 / 65536
    return lat0 - (pos >> 16) * unit, lon0 + (pos & 0xFFFF) * unit


def untranspose(buf, off, n, size):
    """Arrays are stored byte-transposed: byte 0 of all n structs, then byte 1 ..."""
    src = buf[off : off + n * size]
    return [bytes(src[j * n + i] for j in range(size)) for i in range(n)]


def transpose(structs):
    size = len(structs[0])
    return bytes(s[j] for j in range(size) for s in structs)


def utf16(buf, off, nchars):
    """String of nchars UTF-16LE units (incl. terminating NUL) -> (str, new_off)."""
    return buf[off : off + 2 * nchars].decode("utf-16le").rstrip("\0"), off + 2 * nchars


def iter_records(d, area, device=DEVICE):
    """Yield (tile_x, tile_y, cell_x, cell_y, grid, plaintext) for one slot area."""
    key = global_key(device)
    off, g = AREAS[area]
    for x, y, s, e in tiles(d):
        if s == e:
            continue
        blob = d[s + 0x157C : s + 0x159C]
        for k, rel in enumerate(struct.unpack_from("<%dI" % (g * g), d, s + off)):
            if rel == 0xFFFFFFFF:
                continue
            ln, pltx = struct.unpack_from("<II", d, s + rel)
            raw = decode_record(d[s + rel + 8 : s + rel + 8 + ln], pltx, blob, key)
            yield x, y, x * g + k // g, y * g + k % g, g, raw


# --------------------------------------------------------------------------
# osmpoint: seamarks (FUN_003e52d0), slot area C
# --------------------------------------------------------------------------

def parse_osmpoint(raw):
    """-> list of dicts.  Layout: OSMPOINT_FORMAT.md"""
    _, _, _, n, _ = struct.unpack_from("<5I", raw)
    sts = [struct.unpack("<12I", s) for s in untranspose(raw, 0x14, n, 0x30)]
    p = 0x14 + n * 0x30
    objs = []
    for st in sts:
        o = {"cat": st[2], "pos": st[3], "col": st[4], "top": st[5]}
        o["name"], p = utf16(raw, p, st[1])
        o["attrs"], p = utf16(raw, p, st[7])
        o["label"], p = utf16(raw, p, st[9])
        o["sectors"] = []
        for k in range(st[10]):
            col, a0, a1, rad, _, ln = struct.unpack_from("<4HII", raw, p)
            o["sectors"].append({"colour": col, "from": a0 / 10, "to": a1 / 10, "radius": rad, "_len": ln})
            p += 16
        objs.append(o)
    for o in objs:
        for sec in o["sectors"]:
            sec["label"], p = utf16(raw, p, sec.pop("_len"))
    assert p == len(raw)
    return objs


def build_osmpoint(objs, cell_y):
    """Inverse of parse_osmpoint (byte-identical for all Denmark records)."""
    sts, strs, secstrs = [], b"", b""
    for o in objs:
        name, attrs, label = (o[k] + "\0" if o[k] else "" for k in ("name", "attrs", "label"))
        sts.append(struct.pack("<12I", 0, len(name), o["cat"], o["pos"], o["col"], o["top"],
                               0, len(attrs), 0, len(label), len(o["sectors"]), 0))
        strs += (name + attrs + label).encode("utf-16le")
        for sec in o["sectors"]:
            lab = sec["label"] + "\0" if sec["label"] else ""
            strs += struct.pack("<4HII", sec["colour"], round(sec["from"] * 10),
                                round(sec["to"] * 10), sec["radius"], 0, len(lab))
            secstrs += lab.encode("utf-16le")
    n = len(objs)
    return struct.pack("<5I", n * 0x30, cell_y, 0, n, 0) + transpose(sts) + strs + secstrs


# --------------------------------------------------------------------------
# Slot area D (32x32 cells): FUN_003e56fc.  Used by osm, osmpoi, ta.
# --------------------------------------------------------------------------
# Header: 13 x u32.  [3],[5],[7],[9],[11] = counts of the five arrays, the
# following word is a pointer slot (0 in the file).  All five arrays are
# byte-transposed and follow each other from 0x34 on; after them come the
# variable parts, array by array.
D_ARRAYS = [  # (name, struct size in u32 words, header count index)
    ("a1", 10, 3), ("a2", 2, 5), ("a3", 3, 7), ("a4", 5, 9), ("a5", 6, 11)]


def _var_d(name, st):
    """Variable parts of one struct: list of (fmt, count-field, ptr-field)."""
    return {"a1": [("I", 8)], "a2": [("I", 0)], "a3": [("I", 1)],
            "a4": [("H", 1), ("I", 3)], "a5": [("H", 1), ("H", 3)]}[name]


def parse_d(raw):
    """-> {"hdr": 13 words, "a1".."a5": [ {"s": words, "v": [tuple, ...]} ]}"""
    hdr = list(struct.unpack_from("<13I", raw))
    rec = {"hdr": hdr}
    p = 0x34
    for name, words, ci in D_ARRAYS:
        n = hdr[ci]
        rec[name] = [{"s": list(struct.unpack("<%dI" % words, b))}
                     for b in untranspose(raw, p, n, 4 * words)]
        p += n * 4 * words
    for name, _, _ in D_ARRAYS:
        for it in rec[name]:
            it["v"] = []
            for fmt, cf in _var_d(name, it["s"]):
                n = it["s"][cf]
                it["v"].append(struct.unpack_from("<%d%s" % (n, fmt), raw, p))
                p += n * struct.calcsize(fmt)
    assert p == len(raw), (p, len(raw))
    return rec


def build_d(rec):
    out = struct.pack("<13I", *rec["hdr"])
    for name, words, _ in D_ARRAYS:
        if rec[name]:
            out += transpose([struct.pack("<%dI" % words, *it["s"]) for it in rec[name]])
    for name, _, _ in D_ARRAYS:
        for it in rec[name]:
            for (fmt, _), vals in zip(_var_d(name, it["s"]), it["v"]):
                out += struct.pack("<%d%s" % (len(vals), fmt), *vals)
    return out


def u16str(vals):
    return struct.pack("<%dH" % len(vals), *vals).decode("utf-16le", "surrogatepass").rstrip("\0")


def u16enc(text):
    """str -> UTF-16 code units incl. NUL (surrogate pairs for non-BMP), () if empty."""
    if not text:
        return ()
    b = (text + "\0").encode("utf-16le", "surrogatepass")
    return struct.unpack("<%dH" % (len(b) // 2), b)


# osmpoi: only array a5 is used.  a5 struct: [0] ptr name, [1] len name,
# [2] ptr attrs, [3] len attrs, [4] poi type (POI_TYPES), [5] position.
POI_TYPES = [
    "bank", "bikestore", "cafe_pub", "cashdispenser", "departmentstore", "doctor",
    "emergencymedicalservice", "healthcareservice", "hospital_polyclinic",
    "importanttouristattraction", "market", "petrolstation", "pharmacy", "policestation",
    "restaurant", "scenic_panoramicview", "shop", "shoppingcenter", "sportscenter",
    "touristinformationoffice", "camping_ground", "hotel_motel", "sla", "sla_extern",
    "bike_tyres", "bus_stop", "tram_stop", "subway_entrance", "train_station",
    "christian_church", "muslim_mosque", "buddhist_temple", "hindu_temple", "shinto_temple",
    "taoist_temple", "sikh_temple", "jewish_synagog", "bus_station", "bicycle_rental",
    "areodrome", "charging_station", "cable_car", "gondola", "chair_lift", "mixed_lift",
    "t_bar", "j_bar", "platter", "rope_tow", "magic_carpet", "ski_rental", "alpine_hut",
    "montain_rescue", "ski_school", "shelter", "scf_visitor_berth", "scf_nautical_club",
    "scf_boat_hoist", "scf_boatyard", "scf_chandler", "scf_water_tap", "scf_fuel_station",
    "scf_electricity", "scf_showers", "scf_launderette", "scf_toilets", "scf_pump_out",
    "scf_slipway", "scf_visitors_mooring", "harbour", "landmark", "conti_services",
    "landmark", "museum", "zoo"]  # bikenav.exe pointer table at VA 0x4fbb50


def parse_osmpoi(raw):
    rec = parse_d(raw)
    return [{"type": it["s"][4], "pos": it["s"][5], "name": u16str(it["v"][0]),
             "attrs": u16str(it["v"][1])} for it in rec["a5"]]


def build_osmpoi(pois):
    a5 = [{"s": [0, len(u16enc(p["name"])), 0, len(u16enc(p["attrs"])), p["type"], p["pos"]],
           "v": [u16enc(p["name"]), u16enc(p["attrs"])]} for p in pois]
    hdr = [len(a5) * 0x18] + [0] * 10 + [len(a5), 0]
    return build_d({"hdr": hdr, "a1": [], "a2": [], "a3": [], "a4": [], "a5": a5})


# --------------------------------------------------------------------------
# Generic "header + transposed arrays + variable parts" container.
# spec: list of (name, words per struct, header index of the count,
#                [(fmt, count field), ...] variable parts per struct)
# --------------------------------------------------------------------------

def parse_arrays(raw, nhdr, spec):
    hdr = list(struct.unpack_from("<%dI" % nhdr, raw))
    rec = {"hdr": hdr}
    p = 4 * nhdr
    for name, words, ci, _ in spec:
        n = hdr[ci]
        rec[name] = [{"s": list(struct.unpack("<%dI" % words, b))}
                     for b in untranspose(raw, p, n, 4 * words)]
        p += n * 4 * words
    for name, _, _, var in spec:
        for it in rec[name]:
            it["v"] = []
            for fmt, cf in var:
                n = it["s"][cf]
                it["v"].append(struct.unpack_from("<%d%s" % (n, fmt), raw, p))
                p += n * struct.calcsize(fmt)
    assert p == len(raw), (p, len(raw))
    return rec


def build_arrays(rec, spec):
    out = struct.pack("<%dI" % len(rec["hdr"]), *rec["hdr"])
    for name, words, _, _ in spec:
        if rec[name]:
            out += transpose([struct.pack("<%dI" % words, *it["s"]) for it in rec[name]])
    for name, _, _, var in spec:
        for it in rec[name]:
            for (fmt, _), vals in zip(var, it["v"]):
                out += struct.pack("<%d%s" % (len(vals), fmt), *vals)
    return out


# Slot area C, FUN_003e651c (osm, osmarea): 15 header words, six arrays.
C_SPEC = [
    ("c1", 4, 3, [("I", 2)]),              # 0x10
    ("c2", 2, 5, [("I", 0)]),              # 0x08
    ("c3", 6, 7, [("H", 1), ("I", 4)]),    # 0x18
    ("c4", 3, 9, [("I", 1)]),              # 0x0C
    ("c5", 7, 11, [("H", 1), ("I", 5)]),   # 0x1C
    ("c6", 6, 13, [("H", 1), ("I", 4)]),   # 0x18
]


def parse_c(raw):
    return parse_arrays(raw, 15, C_SPEC)


def build_c(rec):
    return build_arrays(rec, C_SPEC)


# Slot area A, FUN_003e6044 (osm, ta): 7 header words, NOT transposed.
#   hdr[3] = n, hdr[5]/hdr[6] = offsets relative to the record start
#   n x 0x1C items; item[1] = len name (u16), item[3] = count of 0x18 subs,
#   item[5] = count of 0x10 subs.  Variable part order: all names, all 0x18
#   sub arrays, all 0x10 sub arrays, strings of the 0x18 subs (sub[1]),
#   strings of the 0x10 subs (sub[1]), pad to 4, u32 arrays of the 0x10 subs
#   (sub[2] = count).

def parse_a(raw):
    hdr = list(struct.unpack_from("<7I", raw))
    n = hdr[3]
    items = [list(struct.unpack_from("<7I", raw, 0x1C + 0x1C * i)) for i in range(n)]
    p = 0x1C + 0x1C * n
    rec = {"hdr": hdr, "items": []}
    for it in items:
        rec["items"].append({"s": it})
        rec["items"][-1]["name"] = struct.unpack_from("<%dH" % it[1], raw, p); p += 2 * it[1]
    for r in rec["items"]:
        k = r["s"][3]
        r["s18"] = [{"s": list(struct.unpack_from("<6I", raw, p + 24 * j))} for j in range(k)]
        p += 24 * k
    for r in rec["items"]:
        k = r["s"][5]
        r["s10"] = [{"s": list(struct.unpack_from("<4I", raw, p + 16 * j))} for j in range(k)]
        p += 16 * k
    for r in rec["items"]:
        for sub in r["s18"]:
            sub["str"] = struct.unpack_from("<%dH" % sub["s"][1], raw, p); p += 2 * sub["s"][1]
    for r in rec["items"]:
        for sub in r["s10"]:
            sub["str"] = struct.unpack_from("<%dH" % sub["s"][1], raw, p); p += 2 * sub["s"][1]
    rec["pad"] = p & 2
    p += p & 2
    for r in rec["items"]:
        for sub in r["s10"]:
            sub["u32"] = struct.unpack_from("<%dI" % sub["s"][2], raw, p); p += 4 * sub["s"][2]
    # hdr[5] / hdr[6]: two further blocks, not interpreted by FUN_003e6044
    assert p == hdr[5] <= hdr[6] <= len(raw), (p, hdr[5], hdr[6], len(raw))
    rec["blk5"], rec["blk6"] = raw[hdr[5]:hdr[6]], raw[hdr[6]:]
    return rec


def build_a(rec):
    out = struct.pack("<7I", *rec["hdr"])
    out += b"".join(struct.pack("<7I", *r["s"]) for r in rec["items"])
    out += b"".join(struct.pack("<%dH" % len(r["name"]), *r["name"]) for r in rec["items"])
    out += b"".join(struct.pack("<6I", *x["s"]) for r in rec["items"] for x in r["s18"])
    out += b"".join(struct.pack("<4I", *x["s"]) for r in rec["items"] for x in r["s10"])
    for key in ("s18", "s10"):
        out += b"".join(struct.pack("<%dH" % len(x["str"]), *x["str"])
                        for r in rec["items"] for x in r[key])
    out += b"\0" * (len(out) & 2)
    out += b"".join(struct.pack("<%dI" % len(x["u32"]), *x["u32"])
                    for r in rec["items"] for x in r["s10"])
    return out + rec["blk5"] + rec["blk6"]


# Slot area B (8x8 cells), FUN_003e5d20, osm only.  NOT encrypted (LZMA only),
# not transposed, no strings.  Routing graph, see OSM_FORMAT.md.
# 11 header words: [3] n nodes (12 B), [5] n edges (16 B), [7] n (16 B,
# always 0 in DK), [9] n extra (12 B).  Node index = node id in D a1[4]/[5].

def parse_b(raw):
    hdr = list(struct.unpack_from("<11I", raw))
    p, rec = 0x2C, {"hdr": hdr}
    for name, ci, words in (("nodes", 3, 3), ("edges", 5, 4), ("b3", 7, 4), ("extra", 9, 3)):
        rec[name] = [list(struct.unpack_from("<%dI" % words, raw, p + 4 * words * i))
                     for i in range(hdr[ci])]
        p += 4 * words * hdr[ci]
    assert p == len(raw), (p, len(raw))
    return rec


def build_b(rec):
    out = struct.pack("<11I", *rec["hdr"])
    for name in ("nodes", "edges", "b3", "extra"):
        out += b"".join(struct.pack("<%dI" % len(x), *x) for x in rec[name])
    return out


def geometry_parts(g):
    """Split a geometry u32 array into parts [(hi, [packed points])].

    Each part: u32 (hi << 16 | n), then n packed points.  For lines hi == n,
    for osmarea polygons hi is a detail level 9..14."""
    i, out = 0, []
    while i < len(g):
        n = g[i] & 0xFFFF
        out.append((g[i] >> 16, list(g[i + 1 : i + 1 + n])))
        i += 1 + n
    assert i == len(g)
    return out


if __name__ == "__main__":
    import glob
    import sys

    path = sys.argv[1] if len(sys.argv) > 1 else glob.glob(
        f"{DEVICE.decode()}/7/943/20317/Denmark_osmpoint.*")[0]
    d = open(path, "rb").read()
    for tx, ty, cx, cy, g, raw in iter_records(d, "C"):
        for o in parse_osmpoint(raw):
            lat, lon = to_latlon(cx, cy, g, o["pos"])
            print(f"{lat:.6f} {lon:.6f} cat={o['cat']:06x} {o['name']!r} {o['attrs']!r}")
