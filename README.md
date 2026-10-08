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

## Building a map

Everything in one command, for the country of your choice:

```bash
cd rust && cargo build --release

export TEASI_DEVICE=2013021200000368        # the serial of your own device

./target/release/teasi all --country=Denmark \
    --land=land-polygons-split-4326/land_polygons.shp \
    denmark-latest.osm.pbf denmark.poly out/
```

That builds the elevation grid and all six layer files, named the way the device expects
them, and prints their size and MD5 at the end. Denmark takes about three and a half
minutes, Great Britain about thirteen with a peak of 16 GB of memory — the street layer is
the expensive part.

What you need:

- the **extract and the boundary** of your country from
  [download.geofabrik.de](https://download.geofabrik.de): `<country>-latest.osm.pbf` and
  the matching `<country>.poly`.
- the **land polygons** from
  [osmdata.openstreetmap.de](https://osmdata.openstreetmap.de/data/land-polygons.html)
  (split, WGS84) for `--land=`. Without them the water areas have no coastline to end at
  and the relief images get no land cover; everything else works.
- **libgeos**, which is opened at run time, so `cargo build` does not need it — see
  `rust/src/geos.rs`. Only `osmarea`, `osm`, `ta` and `terrain` use it.
- the **elevation tiles**, which `teasi all` downloads itself from the public Copernicus
  bucket (234 of them for Great Britain) and caches in `out/dem_tiles`.

`--country=` takes a name from the firmware's list or a bare code, `--only=osm,ta` builds a
subset, and `--original=<chart>` lets a map of the same country that is already on the
device fill in the sea outside the boundary. `teasi all` without arguments lists the rest.

### Onto the device

Confirm the USB connection **on the display** first, otherwise the volume stays unreadable.
Then copy the files in — nothing else to do:

```bash
cp out/Denmark_*.v* /run/media/$USER/TFAT/BikeNav/Map/Countries/
```

The device only loads countries that are unlocked for it, and only one file per layer and
country: a file is skipped if one with the same or a newer **date** is already loaded. So
either give your map a date newer than the one it replaces, or delete that file — an older
date is silently ignored. Back up what is there before overwriting it.

## Looking at an existing map

```bash
./target/release/teasi info <maps>/Denmark_osm.v20210916
./target/release/teasi dump <maps>/Denmark_osm.v20210916 out/osm
```

`check` takes every record apart and writes it again; the result has to be byte-identical,
which is the test that the format is understood rather than guessed:

```bash
./target/release/teasi check <maps>/Denmark_*.v2*
```

Single layers have their own subcommands, one per compiler, and each layer document has a
section "Building it from OSM" with its commands, runtime and memory. `teasi dem` builds
only the elevation grid:

```bash
./target/release/teasi dem great-britain.poly osm_ref/dem build/gb/dem.bin
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
such a file to the first device that opens it, rewriting 64 bytes and nothing else
(`FUN_001061bc`, confirmed on a device 2026-10-08). A map built that way runs on any device
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
