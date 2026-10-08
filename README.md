# Teasi maps from OpenStreetMap

Tools to read and write the chart files of a **Teasi PRO** (Tahuna/Falk, `bikenav.exe`
4.4.1.0, WinCE/ARM) and to rebuild them from current OSM data.

The manufacturer no longer ships maps for this device. The files are not a proprietary
format with a proprietary codec, though: they are an MD5 checksum, **PC1** encryption and
**LZMA** wrapped around data structures that document well. All six layers are decrypted,
their containers are rebuilt byte for byte, and each one has a compiler that builds it
from a Geofabrik extract.

**Status:** Great Britain is built completely and runs on the device — map, POIs, elevation
profile, address search and routing. What is left is a handful of individual fields, see
[docs/CHART_FILES.md](docs/CHART_FILES.md), section 8.

## The layers

A map is a set of files `<Country>_<layer>.vYYYYMMDD` (magic `0x1B62`), on the device
under `<serial>/7/943/20317/`.

| Layer | Content | Compiler | GB |
|---|---|---|---:|
| `osm` | street and path network, names, routing graph | `compile_osm.py` | 531 MB |
| `osmarea` | areas: land use, forest, water, built-up land | `compile_osmarea.py` | 83 MB |
| `osmpoi` | points of interest | `compile_osmpoi.py` | 17 MB |
| `osmpoint` | seamarks from OpenSeaMap | `compile_osmpoint.py` | 0.5 MB |
| `ta` | address search: places, streets, house numbers, search index | `compile_ta.py` | 114 MB |
| `terrain` | elevation model and map images (unencrypted) | `compile_terrain.py` | 37 MB |

Every format has its own document in [docs/](docs/), each with a section "Building it from
OSM" (commands, runtime, RAM) and a section "Open questions".

## Layout of this repository

```
tools/            the tool chain: read/write the shell, extractors, compilers
docs/             format documentation, start with CHART_FILES.md
rust/             Rust port: the shell, reading OSM, all six layer compilers
ghidra_scripts/   headless scripts for analysing the firmware in Ghidra
attic/            one-off scripts from the analysis phase, not maintained
```

The modules in `tools/` sit flat and import each other without a package, so run the
scripts either from inside `tools/` or from the repository root as
`python tools/<script>.py` (both work).

| Tool | Purpose |
|---|---|
| `chart.py` | shell and encryption: check the header, decrypt records |
| `pc1.py` | PC1 (Pukall Cipher 1, 256 bit) |
| `layers.py` | parser and builder for every kind of record |
| `writer.py` | write valid chart files (including the device binding) |
| `roundtrip.py` | check: take a file apart, rebuild it, compare the records |
| `packages.py` | update size and MD5 in `packages.xml` |
| `poly.py` | Geofabrik boundary polygon (`*.poly`) |
| `osm_extract.py`, `osm_poi_extract.py`, `osm_area_extract.py`, `osm_addr_extract.py` | OSM PBF into compact pickles |
| `land_extract.py`, `dem_heights.py`, `osm_heights.py` | land polygons, Copernicus DEM, heights |
| `compile_*.py` | the six layer compilers |
| `ta_lookup.py` | replay the firmware's address search offline (check) |

## Getting started

```bash
python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
```

Decrypt an existing map and write out its records one by one:

```bash
.venv/bin/python tools/chart.py charts/Denmark_osm.v20210916 out/osm
```

Check that the tools rebuild a file without loss:

```bash
.venv/bin/python tools/roundtrip.py charts/Denmark_osmpoi.v20210915
```

Build a layer from OSM (POIs here; the other layers are in the layer documents):

```bash
.venv/bin/python tools/osm_poi_extract.py --filter osm_ref/great-britain-latest.osm.pbf build/poi.pkl
.venv/bin/python tools/compile_osmpoi.py --country=17 build/poi.pkl osm_ref/great-britain.poly \
    build/GreatBritain_osmpoi.v20260919 20260919
```

### Device binding

The header checksum binds a map to the serial number of the device, and the PC1 key is
derived from that number as well. Every tool takes it from `chart.py` (`DEVICE`), which an
environment variable overrides:

```bash
export TEASI_DEVICE=2013021200000368   # your own 16-digit serial
```

For the device to accept a freshly built file without a warning, size and MD5 in
`BikeNav/packages.xml` have to be updated (`packages.py`, documentation section 5.4).

## Rust port

[rust/](rust/) holds the port of the tool chain to Rust. Finished are the shell (PC1,
checksum, raw LZMA1, every record container, the writer, the search index), reading OSM
(PBF reader, node index, address extraction) and all six layer compilers (`osmpoi`,
`osmpoint`, `osmarea`, `osm`, `ta` and `terrain`, straight from the PBF into the chart
file). Only the height sources (`osm_heights.py`, `dem_heights.py`) stay in Python. The
Python tools remain the reference, and the port is checked against them:

```bash
cd rust && cargo build --release
./target/release/teasi check <maps>/Denmark_*.v2*      # records byte-identical
./target/release/teasi osmpoi osm_ref/great-britain-latest.osm.pbf \
    osm_ref/great-britain.poly build/GreatBritain_osmpoi.v20260918 20260918 --country=17
```

All 14,185 records of the Danish map are taken apart and rebuilt byte-identically in 1.3 s.
Decrypting is 40 to 70 times faster than in Python, writing about 8 times, address
extraction 56 times (Denmark 270 s to 4.7 s, all 2,628,399 entries bit-identical). The
osmpoi layer for Great Britain takes 35 s instead of 24 min for the extraction alone. For
Denmark every osmpoi and osmpoint record is byte-identical with the Python version, and
308 of 340 for the area layer — the rest are multipolygons that libosmium's assembler
splits differently. For the street layer **all 22,711 records** of the British map are
byte-identical (4:13 instead of 18:23), and 5981 of 5982 for Denmark. For the address
search all 3958 Danish records **and the search index** are byte-identical, and 13,679 of
13,680 for Great Britain. For the terrain layer every elevation tile is byte-identical in
both countries, down to one byte per tile: OpenJPEG writes its own version into the
codestream's comment marker. `osmarea`, `osm`, `ta` and `terrain` need libgeos (loaded at
runtime, see `rust/src/geos.rs`). Details in [rust/README.md](rust/README.md).

## What is not in here

No device dump, no firmware, no OSM extracts, no built maps — those are several gigabytes,
and the original maps are someone else's licensed data. What you need:

- a Geofabrik extract and the matching `.poly` file (`download.geofabrik.de`),
- for `terrain` the Copernicus DEM (`dem_heights.py` downloads it itself) and the land
  polygons from `osmdata.openstreetmap.de`,
- an original map from the device, for calibration.

## Legal

Reverse engineering for interoperability with one's own, purchased device. The maps built
here contain nothing but OpenStreetMap data (ODbL) plus elevations from the Copernicus DEM;
original maps and parts of the firmware do not belong in this repository and are not
distributed here.

The tools are under the [MIT licence](LICENSE).
