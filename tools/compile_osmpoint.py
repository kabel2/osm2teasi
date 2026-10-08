"""Compile the Teasi osmpoint layer (seamarks) from OSM / OpenSeaMap (OSMPOINT_FORMAT.md).

Input is the pickle of osm_poi_extract.py (it keeps every object with seamark:type).  The
rules were derived by matching Denmark_osmpoint.v20210916 against denmark-220101
(OSMPOINT_FORMAT.md "Aus OSM erzeugen").

usage: compile_osmpoint.py [--country=N] <poi.pkl> <area.poly> <out chart> [YYYYMMDD]

--country: header country index (default 4 = Denmark, 17 = United Kingdom)
"""
import collections
import pickle
import re
import sys
import time

import poly
from layers import build_osmpoint
from writer import cli, write_chart

G = 4                               # slot area C: 4x4 cells per tile
CELL = 32768                        # units per cell (360/2^25 deg)

# ---- category [2] = group << 16 | sub << 8 | class ------------------------------
# class: buoy 0, beacon 1, light 3, landmark 4, harbour 6, anchorage 7, mooring 8,
# wreck 9, rock 0xC, bridge 0xD, radio station 0xE, signal station 0xF, platform 0x10,
# production area 0x11.  sub: shape or category, see the tables.
BUOY_SHAPE = {"conical": 0, "can": 1, "spherical": 2, "pillar": 3, "spar": 4, "barrel": 5,
              "super-buoy": 6}
BEACON_SHAPE = {"stake": 0, "pole": 0, "post": 0, "tower": 2, "pile": 3, "lattice": 3}
LANDMARK = {"cairn": 0, "cemetery": 1, "chimney": 2, "dish_aerial": 3, "flagstaff": 4,
            "flare_stack": 5, "mast": 6, "windsock": 7, "monument": 8, "column": 9,
            "memorial": 10, "obelisk": 11, "statue": 12, "cross": 13, "dome": 14,
            "radar_scanner": 15, "tower": 16, "windmill": 17, "windmotor": 18, "spire": 19}
HARBOUR = {"fishing": 0, "marina": 1, "marina_no_facilities": 2}
MOORING = {"dolphin": 0, "deviation_dolphin": 1, "bollard": 2, "wall": 3, "post": 4,
           "pile": 4, "buoy": 5}
ROCK = {"covers": 0, "awash": 1}
WRECK = {"non-dangerous": 0, "dangerous": 1, "hull_showing": 2}


def category(ty, t):
    g = lambda k: t.get(f"seamark:{ty}:{k}")
    kind = ty.split("_")[0]
    if ty.startswith(("buoy_", "beacon_")):
        group = {"lateral": 0xA, "cardinal": 0xA, "safe": 0x9}.get(ty.split("_")[1], 0xC)
        if kind == "buoy":
            return group << 16 | BUOY_SHAPE.get(g("shape"), 3) << 8
        return group << 16 | BEACON_SHAPE.get(g("shape"), 3) << 8 | 1
    if ty in ("light_major", "light_minor"):
        return 0x090003 | (ty == "light_minor") << 8
    if ty == "landmark":
        c = (g("category") or "").split(";")[0]
        return 0x0A0004 | LANDMARK[c] << 8 if c in LANDMARK else 0x090003
    if ty == "harbour":
        return 0x090006 | HARBOUR.get(g("category"), 3) << 8
    if ty == "anchorage":
        return 0x0E0007
    if ty == "bridge":
        return 0x0E000D
    if ty == "mooring":
        return 0x0F0008 | MOORING.get(g("category"), 0) << 8
    if ty == "rock":
        return 0x0F000C | ROCK.get(g("water_level"), 2) << 8
    if ty == "wreck":
        return 0x0F0009 | WRECK.get(g("category"), 1) << 8
    if ty == "radio_station":
        return 0x0C000E
    if ty.startswith("signal_station"):
        return 0x0C000F
    if ty == "platform":
        return 0x0B0010
    if ty == "production_area":
        return 0x0B0011
    if ty == "light_vessel":
        return 0x0C0202
    return None


# ---- colours [4] and topmark [5] ---------------------------------------------------
COLOUR = {"grey": 0, "white": 1, "black": 2, "red": 3, "green": 4, "blue": 5, "yellow": 8}
PATTERN = {"vertical": 4}
TOPSHAPE = {"cone, point up": 1, "cone, point down": 2, "sphere": 3, "2 spheres": 4,
            "cylinder": 5, "board": 6, "x-shape": 7, "upright cross": 8,
            "2 cones point together": 10, "2 cones base together": 11, "rhombus": 12,
            "2 cones up": 13, "2 cones down": 14, "square": 17, "triangle, point up": 18,
            "triangle, point down": 19}


def colours(v):
    cs = [COLOUR[c] for c in (v or "").split(";") if c in COLOUR]
    return cs, cs[:2]                 # (all, the first two are stored)


def body_colour(ty, t):
    """[4]: bits 0-3 pattern, 4-6 number of colours, then 4-bit colours from bit 7."""
    if not ty.startswith(("buoy_", "beacon_")):
        return 0
    allc, cs = colours(t.get(f"seamark:{ty}:colour"))
    if not allc:
        return 0
    v = PATTERN.get(t.get(f"seamark:{ty}:colour_pattern"), 0) | len(allc) << 4
    for i, c in enumerate(cs):
        v |= c << (7 + 4 * i)
    return v


def topmark(t):
    """[5]: bits 0-6 shape, 7-8 number of colours, then 4-bit colours from bit 9."""
    shape = TOPSHAPE.get(t.get("seamark:topmark:shape"), 0)
    allc, cs = colours(t.get("seamark:topmark:colour"))
    v = shape | min(len(allc), 3) << 7
    for i, c in enumerate(cs):
        v |= c << (9 + 4 * i)
    return v


# ---- texts ---------------------------------------------------------------------------
BUOY_KIND = {"lateral": "Lateral", "cardinal": "Cardinal", "safe_water": "Safe Water",
             "special_purpose": "Special Purpose", "isolated_danger": "Isolated Danger"}
LATERAL = {"port": "Port-hand ", "starboard": "Starboard-hand ",
           "preferred_channel_port": "Preferred Channel to Port ",
           "preferred_channel_starboard": "Preferred Channel to Starboard "}
SPECIAL = {"anchorage": "Anchorage mark ", "cable": "Cable mark ", "diving": "Diving mark ",
           "leading": "Leading mark ", "mooring": "Mooring mark ",
           "no_entry": "Entry prohibited mark ", "notice": "Notice mark ",
           "outfall": "Outfall mark ", "pipeline": "Pipeline mark ",
           "unknown_purpose": "Mark with unknown purpose ", "warning": "General warning mark ",
           "yachting": "Yachting mark "}
HARBOUR_TXT = {"bulk": "Bulk terminal", "cargo": "General cargo terminal",
               "container": "Container terminal", "ferry": "Ferry terminal",
               "fishing": "Fishing harbour", "lay_up": "Lay up vessels berth",
               "marina": "Yacht harbour/marina",
               "marina_no_facilities": "Yacht berths without facilities",
               "naval": "Naval base", "passenger": "Passenger terminal",
               "roro": "RoRo-terminal", "service_repair": "Service and repair",
               "shipyard": "Shipyard"}
MOORING_TXT = {"dolphin": "Dolphin", "bollard": "Bollard", "wall": "Tie-up wall",
               "post": "Post or pile", "pile": "Post or pile", "buoy": "Mooring buoy"}
WRECK_TXT = {"dangerous": "Dangerous wreck", "non-dangerous": "Non-dangerous wreck",
             "hull_showing": "Wreck showing any portion of hull or superstructure"}
LIGHT_CAT = {"floodlight": "flood"}


def light_category(t):
    return t.get("seamark:light:category") or t.get("seamark:light:1:category")


def description(ty, t):
    """Attribute 19."""
    c = t.get(f"seamark:{ty}:category")
    if ty.startswith(("buoy_", "beacon_")):
        kind, sub = ty.split("_", 1)
        base = f"{BUOY_KIND.get(sub, sub)} {kind.capitalize()}"
        if sub == "lateral":
            return LATERAL.get(c, "") + base
        if sub == "cardinal":
            return f"{c.capitalize()} {base}" if c else base
        if sub == "special_purpose":
            return SPECIAL.get(c, "") + base
        return base
    if ty in ("light_major", "light_minor"):
        base = ty.replace("_", " ")
        lc = light_category(t)
        if lc:
            lc = ",".join(LIGHT_CAT.get(x, x.replace("_", " ")) for x in lc.split(";"))
            return f"{lc.capitalize()} {base}"
        return base.capitalize()
    if ty == "landmark":
        return c.split(";")[0].capitalize() if c else None
    if ty == "harbour":
        cs = [HARBOUR_TXT[x] for x in (c or "").split(";") if x in HARBOUR_TXT]
        if not cs:
            return "Harbour"
        return cs[0] + "\n" if len(cs) == 1 else "".join(f"- {x}\n" for x in cs)
    if ty == "mooring":
        return MOORING_TXT.get(c, "Dolphin")
    if ty == "rock":
        return "Rock"
    if ty == "wreck":
        return WRECK_TXT.get(c)
    if ty == "anchorage":
        return {"unrestricted": "Unrestricted anchorage",
                "24_hour": "Anchorage for periods up to 24 hours"}.get(c)
    if ty == "bridge":
        return f"{c.capitalize()} bridge" if c else "Bridge"
    if ty == "platform":
        return f"Platform({c})" if c else "Platform"
    if ty == "radio_station":
        return {"dgps": "Differential GPS"}.get(c, "Radio station")
    if ty == "signal_station_traffic":
        return "Traffic Signal Station" + (f": {c.replace('_', ' ').title()}" if c else "")
    if ty == "signal_station_warning":
        return "Warning Signal Stations"
    if ty == "light_vessel":
        return "Light vessel"
    return None


LETTER = {"white": "W", "red": "R", "green": "G", "yellow": "Y", "blue": "Bu",
          "orange": "Or", "violet": "Vi"}
SECTOR_COLOUR = dict(COLOUR, amber=9)


def num(v):
    try:
        f = float(v)
    except (TypeError, ValueError):
        return v
    return str(int(f)) if f == int(f) else str(f)


def lights(t):
    """-> list of light dicts (seamark:light:* or seamark:light:N:*)."""
    idx = sorted({int(m.group(1)) for k in t for m in [re.match(r"seamark:light:(\d+):", k)] if m})
    if idx:
        return [{k.split(":", 3)[3]: v for k, v in t.items()
                 if k.startswith(f"seamark:light:{i}:")} for i in idx]
    one = {k[14:]: v for k, v in t.items() if k.startswith("seamark:light:")
           and not k.startswith("seamark:light:reference")}
    return [one] if one else []


def light_string(ls):
    """Attribute 24 / label 5, e.g. 'Fl.G.5s6m4M', 'Oc.WRG.12s9-12M', 'F.R(Air_obstruction)'."""
    l0 = ls[0]
    cols = []
    for l in ls:
        for c in (l.get("colour") or "").split(";"):
            if c in LETTER and LETTER[c] not in cols:
                cols.append(LETTER[c])
    order = ["W", "R", "G", "Y", "Bu", "Or", "Vi"]
    s = (l0.get("character") or "") + "." + "".join(sorted(cols, key=order.index))
    rest = ""
    if l0.get("period"):
        rest += num(l0["period"]) + "s"
    if len(ls) == 1 and l0.get("height") and "sector_start" not in l0:
        rest += num(l0["height"]) + "m"
    rs = sorted({float(l["range"]) for l in ls if l.get("range")
                 and re.fullmatch(r"[\d.]+", l["range"])})
    if rs:
        rs = [int(r) for r in rs]                  # nautical miles, truncated
        rest += (str(rs[0]) if rs[0] == rs[-1] else f"{rs[0]}-{rs[-1]}") + "M"
    if rest:
        s += "." + rest
    cat = ls[-1].get("category")
    if cat:
        s += f"({cat[0].upper() + cat[1:]})"
    return s


def sectors(ls):
    out = []
    for i, l in enumerate(ls):
        col = SECTOR_COLOUR.get((l.get("colour") or "").split(";")[0], 0)
        try:
            a, b = float(l["sector_start"]), float(l["sector_end"])
        except (KeyError, ValueError):
            out.append((col, i, {"colour": col, "from": 0.0, "to": 0.0, "radius": 0, "label": ""}))
            continue
        conv = lambda d: int((d + 180) % 360 * 4096 / 360) / 10
        lab = (l.get("character") or "") + "." + LETTER.get(l.get("colour"), "")
        if l.get("period"):
            lab += "." + num(l["period"]) + "s"
        out.append((col, i, {"colour": col, "from": conv(a), "to": conv(b), "radius": 370,
                             "label": lab}))
    return [s for _, _, s in sorted(out, key=lambda x: x[:2])]


def attributes(ty, t, ls):
    a = []
    for code, keys in (("01", ("website", "contact:website")), ("02", ("phone", "contact:phone")),
                       ("04", ("internet_access",)), ("06", ("wheelchair",)),
                       ("11", ("description",)), ("16", ("fax",)),
                       ("17", ("email", "contact:email")), ("18", ("operator",))):
        v = next((t[k] for k in keys if t.get(k)), None)
        if v:
            a.append((code, v))
    d = description(ty, t)
    if d:
        a.append(("19", d))
    if t.get("seamark:information"):
        a.append(("20", t["seamark:information"]))
    if t.get("seamark:radar_transponder:category") == "racon":
        a.append(("21", f"Racon({t.get('seamark:radar_transponder:group', '')})"))
    fog = t.get("seamark:fog_signal:category")
    if fog:
        f = fog.capitalize()
        if t.get("seamark:fog_signal:group"):
            f += f"({t['seamark:fog_signal:group']})"
        if t.get("seamark:fog_signal:period"):
            f += num(t["seamark:fog_signal:period"]) + "s"
        a.append(("23", f))
    if ls:
        a.append(("24", light_string(ls)))
    if t.get("seamark:light:reference"):
        a.append(("25", t["seamark:light:reference"]))
    if t.get(f"seamark:{ty}:system"):
        a.append(("26", t[f"seamark:{ty}:system"]))
    if t.get("port_of_entry"):
        a.append(("27", t["port_of_entry"]))
    if t.get("seamark:landmark:height"):
        a.append(("35", num(t["seamark:landmark:height"])))
    h = ls[0].get("height") if ls else None
    if h:
        a.append(("36", num(h)))
    ex = ls[0].get("exhibition") if ls else None
    if ex:
        a.append(("37", {"24h": "24h", "day": "day", "night": "night"}.get(ex, ex)))
    if ty in ("rock", "wreck"):
        wl = t.get(f"seamark:{ty}:water_level")
        if wl or ty == "rock":
            a.append(("45", wl or "submerged"))
    return "|".join(c + v for c, v in a)


def label(ty, t, ls):
    parts = []
    fog = [p for p in attributes(ty, t, []).split("|") if p[:2] in ("21", "23")]
    parts += ["0" + p[2:] for p in sorted(fog, reverse=True)]
    if ty.startswith("signal_station_warning"):
        parts.append("7SS")
    if ty == "signal_station_traffic" and t.get(f"seamark:{ty}:category"):
        parts.append(f"({t[f'seamark:{ty}:category'].replace('_', ' ').title()})")
    if ty == "bridge" and t.get("seamark:bridge:clearance_height"):
        parts.append("4" + num(t["seamark:bridge:clearance_height"]))
    if ls:
        parts.append("5" + light_string(ls))
    return "|".join(parts)


def position(kind, x, y, t):
    if kind == "w" and t.get("@area"):
        x, y = t["@area"]
    return round(x / 8), round(y / 8)          # 360/2^28 -> 360/2^25 units


def collect(objs, area=None):
    out = []
    for kind, oid, x, y, t in sorted(objs, key=lambda o: (o[0] != "n", o[1])):
        ty = t.get("seamark:type")
        if not ty:
            continue
        cat = category(ty, t)
        if cat is None:
            continue
        X, Y = position(kind, x, y, t)
        if area and not poly.contains(area, X * 8, Y * 8):
            continue
        ls = lights(t)
        out.append((X, Y, {"cat": cat, "col": body_colour(ty, t), "top": topmark(t),
                           "name": t.get("seamark:name", ""),
                           "attrs": attributes(ty, t, ls), "label": label(ty, t, ls),
                           "sectors": sectors(ls)}))
    return out


def build(objs, date, country=4):
    cells = collections.defaultdict(list)
    for X, Y, o in objs:
        cx, cy = X // CELL, Y // CELL
        cells[(cx, cy)].append(dict(o, pos=(Y - cy * CELL) << 16 | (X - cx * CELL)))
    tiles = collections.defaultdict(dict)
    for (cx, cy), lst in cells.items():
        tiles[(cx // G, cy // G)][(cx % G) * G + cy % G] = build_osmpoint(lst, cy)
    content = [(x, y, {"C": tiles[(x, y)]}, b"") for x, y in sorted(tiles)]
    return write_chart({"date": date, "type": 1, "layer": 0x400, "country": country}, content)


if __name__ == "__main__":
    args, opts = cli(sys.argv[1:])
    if len(args) < 3:
        sys.exit(__doc__)
    src, area, dst = args[:3]
    date = (args[3] if len(args) > 3 else time.strftime("%Y%m%d")).encode()
    objs = collect(pickle.load(open(src, "rb")), poly.load(area))
    d = build(objs, date, int(opts.get("country", 4)))
    open(dst, "wb").write(d)
    print(f"{len(objs)} seamarks, {len(d)} B -> {dst}")
