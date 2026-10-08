#!/usr/bin/env python3
"""Regenerate regions.tsv: Geofabrik's regions mapped to Teasi country codes.

A one-off data generator, not part of building anything -- run it when Geofabrik
adds or renames regions.  It needs the country table out of the firmware, which
is `COUNTRIES` in ../rust/src/chart.rs, and the alpha-2 to alpha-3 mapping from
the iso-codes package (Debian/Ubuntu: iso-codes).

    python3 tools/regions.py > tools/regions.tsv
"""
import collections
import json
import re
import sys
import unicodedata
import urllib.request

INDEX = "https://download.geofabrik.de/index-v1.json"
ISO = "/usr/share/iso-codes/json/iso_3166-1.json"
CHART = "rust/src/chart.rs"

CONTINENTS = ["africa", "antarctica", "asia", "australia-oceania", "central-america",
              "europe", "north-america", "russia", "south-america"]

# Regions that cover ground other regions already cover, or that the firmware
# has no single code for.
SKIP = {
    "alps": "overlaps eight countries",
    "dach": "overlaps Germany, Austria, Switzerland",
    "britain-and-ireland": "overlaps united-kingdom and ireland",
    "great-britain": "part of united-kingdom",
    "sea": "overlaps eleven countries",
    "south-africa-and-lesotho": "overlaps south-africa and lesotho",
    "us-midwest": "part of us", "us-northeast": "part of us", "us-pacific": "part of us",
    "us-south": "part of us", "us-west": "part of us",
    "ile-de-clipperton": "not in the firmware's list",
    "american-oceania": "several US territories at once, no single code",
}

# Where Geofabrik's name does not meet the firmware's, and the parts of a
# country that have no code of their own.
ALIAS = {
    "azores": "Portugal", "canary-islands": "Spain", "kaliningrad": "Russia",
    "comores": "Comoros", "guernsey-jersey": "Channel Islands",
    "polynesie-francaise": "French Polynesia", "wallis-et-futuna": "Wallis and Futuna",
    "pitcairn-islands": "Pitcairn",
    **{f"{d}-fed-district": "Russia" for d in
       ["central", "crimean", "far-eastern", "north-caucasus", "northwestern",
        "siberian", "south", "ural", "volga"]},
}


def norm(t):
    t = unicodedata.normalize("NFKD", t).encode("ascii", "ignore").decode().lower()
    return re.sub(r"[^a-z0-9]", "", t)


def countries():
    """`(code, name, file prefix)` per entry of chart.rs's COUNTRIES."""
    src = open(CHART, encoding="utf-8").read()
    rows = re.findall(r'^    \("([^"]+)", (\d+), "([^"]+)", "([^"]+)"\),$', src, re.M)
    if len(rows) != 338:
        sys.exit(f"{CHART}: found {len(rows)} countries, expected 338")
    return [(int(c), n, p) for n, c, _, p in rows]


def main():
    geo = json.load(urllib.request.urlopen(INDEX))["features"]
    tab = countries()
    iso3 = {c["alpha_2"]: c["alpha_3"] for c in json.load(open(ISO))["3166-1"]}
    prefix = {c: p for c, _, p in tab}
    by_name = {norm(n): c for c, n, _ in tab}
    by_head = {norm(n.split("(")[0]): c for c, n, _ in tab}
    by_iso = {}
    for c, _, p in tab:                      # the prefix carries no ISO code, so use
        by_iso.setdefault(p, c)              # chart.rs's own third column instead
    src = open(CHART, encoding="utf-8").read()
    for n, c, i, p in re.findall(r'^    \("([^"]+)", (\d+), "([^"]+)", "([^"]+)"\),$', src, re.M):
        by_iso[i] = int(c)
    name = {c: n for c, n, _ in tab}

    rows, open_ones = [], []
    for f in geo:
        p = f["properties"]
        fid, parent = p["id"], p.get("parent")
        if parent not in CONTINENTS or fid in SKIP or fid.split("/")[-1] in SKIP:
            continue
        short = fid.split("/")[-1]
        code = None
        if fid.startswith("us/"):            # a state, not the country of the same name
            code = by_name.get(norm(short.replace("-", " ") + " (United States)"))
        if code is None and short in ALIAS:
            code = by_name.get(norm(ALIAS[short])) or by_head.get(norm(ALIAS[short]))
        if code is None:
            for cand in (p["name"], short.replace("-", " ")):
                code = code or by_name.get(norm(cand)) or by_head.get(norm(cand))
        if code is None:                     # Geofabrik's ISO tag last: it has Pitcairn
            for a2 in p.get("iso3166-1:alpha2") or []:   # as MH and six Pacific regions
                code = code or by_iso.get(iso3.get(a2, ""))   # as VU
        if code is None:
            open_ones.append((fid, p["name"]))
        else:
            rows.append((parent, f"{parent}/{fid}", code))
    if open_ones:
        sys.exit("no country code for: " + ", ".join(f"{i} ({n})" for i, n in open_ones))
    rows.sort()
    shared = {c for c, k in collections.Counter(r[2] for r in rows).items() if k > 1}

    print(__doc__.splitlines()[0].replace("Regenerate regions.tsv: ", "# "), end="")
    print("""
# Columns: continent, Geofabrik path (without -latest.osm.pbf or .poly), the
# country code of the chart header, the country as the firmware names it, and
# the prefix of the chart file names.
#
# A '*' before the code marks one that several regions share.  The device keeps
# only one file per layer and country, so those cannot be installed next to each
# other: the ten Russian federal districts, the Azores (Portugal) and the Canary
# Islands (Spain).
#
# Generated by tools/regions.py -- see there for what is left out and why.
#continent\tregion\tcode\tcountry\tprefix""")
    for parent, path, code in rows:
        star = "*" if code in shared else ""
        print(f"{parent}\t{path}\t{star}{code}\t{name[code]}\t{prefix[code]}")


if __name__ == "__main__":
    main()
