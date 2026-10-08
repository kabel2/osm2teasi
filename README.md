# Teasi maps from OpenStreetMap

> **Note:** This project was created with AI. The reverse engineering, the code and the
> documentation were written by Claude (Anthropic), guided and tested on the device by a human.

Tools to read and write the chart files of a **Teasi PRO** (Tahuna/Falk, `bikenav.exe`
4.4.1.0, WinCE/ARM) and to rebuild them from current OSM data.

The manufacturer no longer ships maps for this device. The files are not a proprietary
format with a proprietary codec, though: they are an MD5 checksum, **PC1** encryption and
**LZMA** wrapped around data structures that document well. All six layers are decrypted,
their containers are rebuilt byte for byte, and each one has a compiler that builds it
from a Geofabrik extract.

**Status:** maps for every Geofabrik region of the world (264 of them) are built, and
Great Britain, Germany and Denmark run on the device — map, POIs, elevation profile,
address search and routing. What is left is a handful of individual fields, see
[docs/CHART_FILES.md](docs/CHART_FILES.md), section 8.

**Tested on:**

| | |
|---|---|
| Device | TEASI PRO (`BikeNav/deviceid.dat` says `Teasi PRO`), serial prefix `20130212` |
| Software | BikeNav 4.4.1.0, SVN 26268, built 2020-06-05 (update package 4410, the last one) |
| Original maps | map update 5130 (2021-09-29), basemap 5010 (2020-03-09) |
| Platform | Windows CE on ARM, 4 GB internal storage (FAT, label `TFAT`) |

Other TEASI models (ONE, Pro Pulse, Volt) are untested. The software version of your own
device is in `BikeNav/settings.xml`, the first line.

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
tools/            build_world.sh and its region list: every Geofabrik region as a zip
web/              download page and browser installer for those zips
```

One binary, `teasi`, with a subcommand per job — [rust/README.md](rust/README.md) describes
them all. No other runtime and no build step beyond `cargo build`.

## Building a map

Everything in one command, for the country of your choice. Either take the Linux binary
from the [releases](https://github.com/kabel2/osm2teasi/releases) or build it with
`cd rust && cargo build --release`; in both cases libgeos has to be installed
(`apt install libgeos-c1t64`, on older Debian/Ubuntu `libgeos-c1v5`).

```bash
cd rust && cargo build --release

export TEASI_DEVICE=2013021200000368        # the serial of your own device

./target/release/teasi all --country=Denmark \
    --land=land-polygons-split-4326/land_polygons.shp \
    denmark-latest.osm.pbf denmark.poly out/
```

That builds the elevation grid and all six layer files, named the way the device expects
them, and prints their size and MD5 at the end. Denmark takes about three minutes with a
peak of 2.5 GB of memory, Great Britain (2.2 GB extract) about ten minutes with 6.4 GB,
Germany (4.9 GB extract) about 35 minutes with 13.3 GB. The layers are built one after the
other, so the peak is that of the most expensive one: the areas, then the streets and the
address search.

What you need:

- the **extract and the boundary** of your country from
  [download.geofabrik.de](https://download.geofabrik.de): `<country>-latest.osm.pbf` and
  the matching `<country>.poly`.
- the **land polygons** from
  [osmdata.openstreetmap.de](https://osmdata.openstreetmap.de/data/land-polygons.html)
  (split, WGS84) for `--land=`. Without them the water areas have no coastline to end at
  and the relief images get no land cover; everything else works.
- **libgeos** (see above), which is opened at run time, so `cargo build` does not need it —
  see `rust/src/geos.rs`. Only `osmarea`, `osm`, `ta` and `terrain` use it.
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

### Every region at once

`tools/build_world.sh <out dir>` works through `tools/regions.tsv` — every region below a
continent on Geofabrik, 264 of them, each with the country code the firmware has for it —
and leaves one zip per region, with a README inside, in a folder per continent. It
downloads one extract at a time, deletes it once the maps are built, signs with
`--generic` and skips regions whose zip is already there, so it can be interrupted and
started again. Extracts bigger than a third of the machine's memory are left out
(`MAX_PBF_MB`); with 29 GB that is none of them. `ONLY=ta` rebuilds just that layer in
the zips that are already there. [web/](web/) serves the result: a download page and an
installer that copies a map onto the device straight from Chrome or Edge.

The list is generated: `teasi regions > tools/regions.tsv` reads Geofabrik's index and
matches it against the firmware's country list. A country whose parts all have a code of
their own is listed by part — the US states and the Canadian provinces — and a region of
several countries gets the code of the biggest (`gcc-states` is Saudi Arabia).

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
is not checked at all, but an entry whose file is gone fails too. A new country therefore
needs nothing in `packages.xml`; replacing one of the original maps (Denmark, Germany,
Norway, Sweden) does: remove the old files and point their six entries at the new ones —
`teasi info` prints the `<size>` and `<md5>` to paste in (documentation section 5.4).

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
