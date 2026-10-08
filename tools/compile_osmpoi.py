"""Compile the Teasi osmpoi layer from OSM (OSMPOI_FORMAT.md).

Input is the pickle of osm_poi_extract.py.  The tag -> POI type rules and the
attribute string were derived by matching the original Denmark_osmpoi.v20210915
against the Geofabrik extract denmark-220101 (see OSMPOI_FORMAT.md "Aus OSM erzeugen").

usage: compile_osmpoi.py [--country=N] <poi.pkl> <area.poly> <out chart> [YYYYMMDD]

--country: header country index (default 4 = Denmark, 17 = United Kingdom)

Only POIs inside <area.poly> (Geofabrik boundary) are kept: the extracts also contain
complete ways and relations that reach far outside the country.
"""
import collections
import pickle
import sys
import time

import poly
from layers import build_osmpoi
from writer import cli, write_chart

G = 32                              # slot area D: 32x32 cells per tile
CELL = 32768                        # units per cell (360/2^28 deg)
DEDUP = 500                         # same type + name closer than this: keep one

RELIGION = {"christian": 0x1D, "muslim": 0x1E, "buddhist": 0x1F, "hindu": 0x20,
            "shinto": 0x21, "taoist": 0x22, "sikh": 0x23, "jewish": 0x24}
SCF = {"visitor_berth": 0x37, "nautical_club": 0x38, "boat_hoist": 0x39, "boatyard": 0x3A,
       "chandler": 0x3B, "water_tap": 0x3C, "fuel_station": 0x3D, "electricity": 0x3E,
       "showers": 0x3F, "laundrette": 0x40, "toilets": 0x41, "pump-out": 0x42,
       "slipway": 0x43, "visitors_mooring": 0x44}
AERIALWAY = {"cable_car": 0x29, "gondola": 0x2A, "chair_lift": 0x2B, "mixed_lift": 0x2C,
             "t-bar": 0x2D, "j-bar": 0x2E, "platter": 0x2F, "rope_tow": 0x30,
             "magic_carpet": 0x31}
HISTORIC_SKIP = {"railway", "hollow_way", "bunker", "wall", "aircraft", "no"}

# (key, value, type); first match wins
RULES = [
    ("amenity", "fuel", 0x0B), ("amenity", "bicycle_rental", 0x26),
    ("shop", "bicycle", 0x01), ("amenity", "bank", 0x00), ("amenity", "atm", 0x03),
    ("amenity", "pharmacy", 0x0C), ("amenity", "doctors", 0x05),
    ("amenity", "hospital", 0x08), ("amenity", "police", 0x0D),
    ("emergency", "defibrillator", 0x06), ("amenity", "marketplace", 0x0A),
    ("amenity", "bus_station", 0x25),
    ("amenity", "cafe", 0x02), ("amenity", "bar", 0x02), ("amenity", "pub", 0x02),
    ("amenity", "restaurant", 0x0E), ("amenity", "fast_food", 0x0E),
    ("amenity", "shelter", 0x36),
    ("tourism", "museum", 0x49), ("tourism", "gallery", 0x49),
    ("tourism", "zoo", 0x4A), ("tourism", "aquarium", 0x4A),
    ("tourism", "attraction", 0x09), ("tourism", "viewpoint", 0x0F),
    ("tourism", "information", 0x13), ("tourism", "camp_site", 0x14),
    ("tourism", "hotel", 0x15), ("tourism", "hostel", 0x15), ("tourism", "motel", 0x15),
    ("tourism", "alpine_hut", 0x33),
    ("shop", "supermarket", 0x11), ("shop", "department_store", 0x04),
    ("shop", "convenience", 0x10), ("shop", "bakery", 0x10), ("shop", "butcher", 0x10),
    ("shop", "kiosk", 0x10),
    ("leisure", "sports_centre", 0x12), ("leisure", "stadium", 0x12),
    ("railway", "station", 0x1C), ("railway", "halt", 0x1C),
    ("railway", "tram_stop", 0x1A), ("railway", "subway_entrance", 0x1B),
    ("highway", "bus_stop", 0x19),
]


def poi_type(t, kind="n"):
    if t.get("amenity") == "place_of_worship":
        return RELIGION.get(t.get("religion"))
    if t.get("seamark:type") == "small_craft_facility":
        return SCF.get(t.get("seamark:small_craft_facility:category"))
    for k, v, typ in RULES:
        if t.get(k) == v:
            return typ
    if t.get("man_made") == "tower" and (t.get("tower:type") == "observation"
                                         or t.get("leisure") == "bird_hide"):
        return 0x0F
    if t.get("amenity") == "charging_station" and t.get("bicycle") == "yes":
        return 0x28
    if t.get("aeroway") == "aerodrome" and kind == "n":     # original: only small
        return 0x27                                            # airfields mapped as nodes
    if t.get("aerialway") in AERIALWAY:
        return AERIALWAY[t["aerialway"]]
    if "historic" in t and t["historic"] not in HISTORIC_SKIP:
        return 0x48
    return None


def first(t, *keys):
    for k in keys:
        if t.get(k):
            return t[k]
    return None


def attributes(t):
    """'NNvalue|NNvalue|...' sorted by code (OSMPOI_FORMAT.md, Attribut-String)."""
    a = []
    street = " ".join(filter(None, (t.get("addr:street"), t.get("addr:housenumber"))))
    city = " ".join(filter(None, (t.get("addr:postcode"), t.get("addr:city"))))
    if street or city:
        a.append(("00", "\n".join(filter(None, (street, city)))))
    for code, keys in (("01", ("website", "url", "contact:website")),
                       ("02", ("phone", "contact:phone")),
                       ("03", ("opening_hours",)), ("04", ("internet_access",)),
                       ("05", ("smoking",)), ("06", ("wheelchair",)), ("07", ("beds",)),
                       ("08", ("rooms",)), ("09", ("cuisine",)), ("10", ("capacity",)),
                       ("11", ("description",)), ("16", ("fax", "contact:fax")),
                       ("17", ("email", "contact:email")), ("18", ("operator",)),
                       ("74", ("access",))):
        v = first(t, *keys)
        if v:
            if code == "03":
                v = v.replace("; ", "\n").replace(";", "\n")
            a.append((code, v))
    return "|".join(c + v for c, v in a)


def position(kind, x, y, t):
    if kind == "w" and t.get("@area"):
        x, y = t["@area"]
    return round(x), round(y)


def collect(objs, area=None):
    """-> [(type, X, Y, name, attrs)] after de-duplication."""
    pois = []
    for kind, oid, x, y, t in objs:
        typ = poi_type(t, kind)
        if typ is None:
            continue
        X, Y = position(kind, x, y, t)
        if area and not poly.contains(area, X, Y):
            continue
        pois.append((kind != "n", oid, typ, X, Y, t.get("name", ""), attributes(t)))
    pois.sort()                        # nodes first, then by OSM id
    near = collections.defaultdict(list)
    out = []
    for _, _, typ, X, Y, name, attrs in pois:
        if name:
            k = (typ, name)
            if any(abs(X - a) < DEDUP and abs(Y - b) < DEDUP for a, b in near[k]):
                continue
            near[k].append((X, Y))
        out.append((typ, X, Y, name, attrs))
    return out


def build(pois, date, country=4):
    cells = collections.defaultdict(list)
    for typ, X, Y, name, attrs in pois:
        cx, cy = X // CELL, Y // CELL
        cells[(cx, cy)].append({"type": typ, "pos": (Y - cy * CELL) << 16 | (X - cx * CELL),
                                "name": name, "attrs": attrs})
    tiles = collections.defaultdict(dict)
    for (cx, cy), lst in cells.items():
        tiles[(cx // G, cy // G)][(cx % G) * G + cy % G] = build_osmpoi(lst)
    content = [(x, y, {"D": tiles[(x, y)]}, b"") for x, y in sorted(tiles)]
    meta = {"date": date, "type": 1, "layer": 8, "country": country}
    return write_chart(meta, content)


if __name__ == "__main__":
    args, opts = cli(sys.argv[1:])
    if len(args) < 3:
        sys.exit(__doc__)
    src, area, dst = args[:3]
    date = (args[3] if len(args) > 3 else time.strftime("%Y%m%d")).encode()
    pois = collect(pickle.load(open(src, "rb")), poly.load(area))
    d = build(pois, date, int(opts.get("country", 4)))
    open(dst, "wb").write(d)
    print(f"{len(pois)} POIs, {len(d)} B -> {dst}")
