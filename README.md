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
| `osm` | street and path network, names, routing graph | `teasi osm` | 531 MB |
| `osmarea` | areas: land use, forest, water, built-up land | `teasi osmarea` | 83 MB |
| `osmpoi` | points of interest | `teasi osmpoi` | 17 MB |
| `osmpoint` | seamarks from OpenSeaMap | `teasi osmpoint` | 0.5 MB |
| `ta` | address search: places, streets, house numbers, search index | `teasi ta` | 114 MB |
| `terrain` | elevation model and map images (unencrypted) | `teasi terrain` | 37 MB |

Every format has its own document in [docs/](docs/), each with a section "Building it from
OSM" (commands, runtime, RAM) and a section "Open questions".

## Layout of this repository

```
rust/             everything: the shell, reading OSM, all six compilers, the DEM
docs/             format documentation, start with CHART_FILES.md
ghidra_scripts/   headless scripts for analysing the firmware in Ghidra
```

One binary, `teasi`, with a subcommand per job — [rust/README.md](rust/README.md) describes
them all. No other runtime and no build step beyond `cargo build`.

## Getting started

```bash
cd rust && cargo build --release
```

Look at an existing map and write out its records one by one:

```bash
./target/release/teasi info <maps>/Denmark_osm.v20210916
./target/release/teasi dump <maps>/Denmark_osm.v20210916 out/osm
```

Check that a file is rebuilt without loss — every record is taken apart and written again,
and the result has to be byte-identical:

```bash
./target/release/teasi check <maps>/Denmark_*.v2*
```

Build a layer from OSM (POIs here; the other layers are in the layer documents):

```bash
./target/release/teasi osmpoi osm_ref/great-britain-latest.osm.pbf \
    osm_ref/great-britain.poly build/GreatBritain_osmpoi.v20260919 20260919 --country=17
```

`osmarea`, `osm`, `ta` and `terrain` need libgeos, which is opened at run time, so building
`teasi` does not need it — see `rust/src/geos.rs`.

The elevation data for `terrain` and for the ascents of the routing graph comes from the
Copernicus DEM GLO-90; `teasi dem` downloads the 1°×1° tiles covering an area, decimates
them onto one 3″ grid and smooths it:

```bash
./target/release/teasi dem osm_ref/great-britain.poly osm_ref/dem build/gb/dem.bin
```

### Device binding

The header checksum binds a map to the serial number of the device, and the PC1 key is
derived from that number as well. `teasi` takes it from `rust/src/chart.rs` (`DEVICE`),
which an environment variable overrides:

```bash
export TEASI_DEVICE=2013021200000368   # your own 16-digit serial
```

Only the **first eight digits** pick the key — they are the production date of a batch, and
the 19 batches listed in `chart.rs` all share one built-in key. Every compiler also takes
`--generic`, which leaves the serial out of the header checksum; the firmware then binds
such a file to the first device that opens it (`FUN_001061bc`, read from the firmware and
not yet confirmed on a device). A map built that way should therefore run on any device
whose serial starts with one of those prefixes, which is what makes it worth passing on.

`BikeNav/packages.xml` lists size and MD5 for every map file it knows. If they do not
match, the device shows a warning at startup and loads the map anyway; a file with no entry
is not checked at all. Giving a new map its own file name is therefore enough — nothing in
`packages.xml` has to be touched. `teasi info` prints both values for a file, ready to paste
into its `<size>` and `<md5>` if you do want to replace a map that is listed there
(documentation section 5.4).

## What is not in here

No device dump, no firmware, no OSM extracts, no built maps — those are several gigabytes,
and the original maps are someone else's licensed data. What you need:

- a Geofabrik extract and the matching `.poly` file (`download.geofabrik.de`),
- for `terrain` the Copernicus DEM (`teasi dem` downloads it itself) and the land polygons
  from `osmdata.openstreetmap.de`,
- an original map from the device, for calibration.

## Legal

Reverse engineering for interoperability with one's own, purchased device. The maps built
here contain nothing but OpenStreetMap data (ODbL) plus elevations from the Copernicus DEM;
original maps and parts of the firmware do not belong in this repository and are not
distributed here.

The tools are under the [MIT licence](LICENSE).
