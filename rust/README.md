# teasi (Rust)

The whole tool chain in one binary: **the shell** (decryption, compression, every record
container, the writer, the search index), **reading OSM** (PBF reader, node index, address
extraction), **the five OSM layer compilers** (`osmpoi`, `osmpoint`, `osmarea`, `osm` and
`ta`, straight from the PBF into the chart file), **the `terrain` layer** (elevation model
and map images) and **the elevation grid** itself, from the Copernicus DEM.

## Status

| Building block | Module | Notes |
|---|---|---|
| PC1, MD5 checksum, global key | `pc1.rs`, `chart.rs` | |
| Raw LZMA1 as in the originals | `lzma.rs` | `xz2`, see the pitfalls at the end |
| Containers A, B, C, D, osmpoint | `layers.rs` | |
| Writing files, device binding | `writer.rs` | `chart::Signer`, bound or `--generic` |
| Country list and codes | `chart.rs` | `COUNTRIES`, 5 of 17 confirmed on the device |
| Search index of the address search | `ta_index.rs` | |
| Reading OSM PBF, node index | `pbf.rs` | the `osmpbf` crate, instead of pyosmium |
| Addresses, places, interpolation ways | `addr.rs` | |
| Country boundary (`.poly`) | `poly.rs` | |
| Reading POIs | `poi.rs` | |
| Building the `osmpoi` layer | `osmpoi.rs` | |
| Building the `osmpoint` layer | `osmpoint.rs` | |
| Reading areas and the coastline | `area.rs` | |
| Worldwide land polygons (shapefile) | `land.rs` | own shapefile reader |
| Geometry (GEOS) | `geos.rs` | libgeos at run time, as shapely uses it |
| Building the `osmarea` layer | `osmarea.rs` | |
| Reading street and line ways | `way.rs` | |
| The Copernicus DEM, the elevation grid | `dem.rs` | own GeoTIFF reader, scipy's Gaussian |
| Reading and querying heights | `heights.rs` | reads what `dem.rs` writes |
| Building the `osm` layer | `osm.rs` | |
| Nearest-neighbour search | `grid.rs` | instead of scipy's kd-tree |
| Building the `ta` layer (address search) | `ta.rs` | |
| Drawing, scaling, JPEG, JPEG 2000 | `raster.rs` | the pieces of Pillow, OpenJPEG |
| Building the `terrain` layer | `terrain.rs` | |

## Building and checking

```bash
cargo build --release
cargo test                 # frozen reference values, see tests/compat.rs
TEASI_GEOS=… cargo test    # plus the two GEOS tests (otherwise skipped, see below)
```

The build needs a C compiler: `xz2` compiles liblzma, `openjpeg-sys` compiles OpenJPEG (for
the elevation tiles of the terrain layer). libgeos, by contrast, is **not** built in but
loaded at runtime, see below.

The real acceptance test runs against actual maps (which are not in this repository):

```bash
./target/release/teasi check <directory>/Denmark_*.v2*
```

`check` decrypts every record, takes it apart and puts it back together; the result has to
match the original byte for byte. For the Danish map that is all **14,185 records** (osm
5878, ta 4125, osmpoi 3704, osmarea 368, osmpoint 110) in 1.3 s.

A compiled layer cannot be checked against an original — it is built from today's OSM, the
originals are from 2021 — so what the layer documents in [../docs/](../docs/) record instead
is how the rules were calibrated against the original: what fraction of the objects at an
identical position get the same class, name and flags.

| Command | Purpose |
|---|---|
| `teasi all <pbf> <poly> <dir> [date]` | the elevation grid and all six layers of one country (`--country=<name\|code>`, `--only=`, `--land=`, `--tiles=`, `--original=`, `--prefix=`) |
| `teasi info <map>…` | header, tiles, record counts per slot area |
| `teasi check <map>…` | take every record apart and rebuild it byte-identically |
| `teasi roundtrip <map> [out]` | decrypt the whole file, write it again, compare the records |
| `teasi md5s <map>` | `area cx cy md5` per record — to compare two charts |
| `teasi dump <map> <dir>` | the decrypted records as individual files |
| `teasi index <ta map>` | take the search index apart and rebuild it byte-identically |
| `teasi addr <file.osm.pbf> [out]` | addresses, places and interpolation ways from OSM |
| `teasi poi <file.osm.pbf> <out>` | POI candidates as a canonical dump |
| `teasi osmpoi <pbf> <poly> <map> [date]` | build the osmpoi layer (`--country=N`) |
| `teasi osmpoint <pbf> <poly> <map> [date]` | build the osmpoint layer (`--country=N`) |
| `teasi area <file.osm.pbf> <out>` | areas and the coastline as a canonical dump |
| `teasi land <land_polygons.shp> <poly>` | the count and area of the worldwide land polygons |
| `teasi osmarea <pbf> <poly> <original\|-> <map> [date]` | build the osmarea layer (`--country=N`, `--land=…`) |
| `teasi ways <file.osm.pbf> <out>` | street and line ways as a canonical dump |
| `teasi osm <pbf> <poly> <original\|-> <map> [date]` | build the osm layer (`--country=N`, `--name=…`, `--heights=…`) |
| `teasi ta <pbf> <poly> <map> [date]` | build the address search (`--country=N`, `--name=…`) |
| `teasi dem <poly> <tile dir> <out.bin>` | download the Copernicus DEM for that area and write the elevation grid (`--sigma=S`) |
| `teasi terrain <heights> <poly> <map> [date]` | build the elevation model and the map images (`--land=…`, `--area=…`, `--rate=R`, `--only=x,y`) |

Every compiler signs the file for `TEASI_DEVICE`, or for no device in particular with
`--generic` (`chart::Signer`); the firmware then binds it to the first device that opens it.

The serial number comes from `TEASI_DEVICE`, the default being the one in `chart.rs`.

Decrypting is bounded by PC1, a byte-at-a-time stream cipher that runs on one core per
record; writing is bounded by the LZMA compression at around 1.5 MB/s per core, which is why
rewriting the whole Danish osm file takes 61 s and cannot be pushed down much further.

## Reading OSM

`pbf.rs` reads `.osm.pbf` files (the `osmpbf` crate), decodes the blocks in parallel and
folds them into per-thread accumulators. Then the node index: only the ids that an earlier
pass asked for are collected — sorted, 16 bytes per node — instead of an index over every
node in the file.

`addr.rs` extracts the addresses, places and interpolation ways in three passes, because a
relation needs its member ways and those need their nodes:

1. relations: which `multipolygon` or `boundary` relations carry address or place
   information,
2. ways: interpolation ways, closed ways (areas in their own right) and the member ways,
3. nodes: the results that are nodes, the coordinates for the ways, and the house numbers of
   the interpolation nodes.

`teasi addr <pbf> <out>` writes a canonical dump of the result — the entries sorted, with
the coordinates as IEEE bit patterns — so that two runs, or two versions of the extractor,
can be diffed line by line:

```bash
./target/release/teasi addr osm_ref/denmark-latest.osm.pbf addr.txt
```

Denmark yields 2,628,399 entries (2,614,623 addresses and 13,776 places), Great Britain
5,023,358 addresses, 111,340 places and 140,467 interpolation points.

| | |
|---|---:|
| Denmark (494 MB PBF) | 4.7 s, 1.5 GB |
| Great Britain (2.2 GB PBF) | 31 s, 5.3 GB |

The three passes take 6, 8 and 11 s for Great Britain; one pass over the 2.2 GB costs about
6 s on its own, and the rest is the actual work.

### Assembling areas the way libosmium does

Everything that hangs off an area — the centre of an address, the position of a POI, the
rings of the osmarea layer — depends on how a multipolygon's rings are assembled, and the
reference for that is **libosmium**, whose assembler is reproduced in `pbf.rs`
(`assemble_segments`, `split_rings`, `area_loc_rings`), like this:

- **From edges, not from ways.** Every edge (a pair of nodes) is normalised so that the
  smaller location comes first, then all the edges are sorted — and **equal edges cancel out
  in pairs**. That is the heart of it: two ways running alongside each other for a stretch
  merge into *one* ring that way, and a spike doubling back on itself disappears. Afterwards
  the rings are walked out of what is left.
- **Locations, not node ids.** libosmium compares positions. Two different nodes in the same
  place are the same point as far as the assembler is concerned.
- **A ring that touches itself falls apart** at that point into two rings (`split_rings`) —
  usually into an outer ring and a hole.
- **A ring starts at its smallest vertex** and repeats it at the end; the comparison runs on
  the decimicrodegree integers, not on the Teasi units (where y is mirrored). The mean
  therefore counts that point twice.
- **Outer and inner is decided by nesting, not by the member role.** A ring inside an even
  number of other rings is an outer one. The roles in OSM are often wrong — in Great Britain
  there are multipolygons whose outer rings are tagged as `inner` or even as `building`. When
  two rings touch at a vertex, the first point is useless as a test point (it lies on the
  other ring); what gets used is the first point that is not a vertex of the other ring.
- **`area=no` forbids the area**, however the other tags may look.
- The **direction** (outer rings counter-clockwise in lon/lat) is decided on the
  decimicrodegree coordinates, not on the rounded ones: a tiny area collapses when rounded,
  and then the rounding would decide the direction.

What that buys, in numbers: of the 5,023,357 British addresses that sit on an area, the
assembler first placed 712,719 the way libosmium does, then 5,022,875, and finally **all** of
them; of 978,957 Danish OSM areas it builds 978,902 exactly as libosmium does.

**What is left** are areas whose rings touch themselves — 55 of 978,957 in Denmark, 347 of
4,868,591 in Great Britain (0.007 %): at such crossings libosmium does not follow "the first
free edge" but searches, splits and rejoins the rings afterwards. The number of rings or the
starting point of a ring then differs. Reproducing that part of the assembler (about 1000
lines in libosmium) is not worth it for a thousandth of a thousandth.

One spot was subtler still: a centre is the sum of many floats, and a naive loop lands one
bit away from a **compensated** sum (Neumaier). That bit is enough to flip the coordinate
once it is rounded to whole Teasi units, so `osm::fsum` sums with compensation.

## The osmpoi and osmpoint layers

`poi.rs` serves both layers — it keeps whatever `poi_type` accepts plus everything with a
`seamark:type`. On top of it sit `osmpoi.rs` (type rules, the attribute string,
deduplication) and `osmpoint.rs` (categories, colours, topmarks, light strings, sectors).
Nothing in between — one command from the extract to the chart file:

```bash
./target/release/teasi osmpoi osm_ref/great-britain-latest.osm.pbf \
    osm_ref/great-britain.poly build/GreatBritain_osmpoi.v20260918 20260918 --country=17
./target/release/teasi osmpoint …    # the same arguments
```

`teasi poi <pbf> <out>` writes a canonical dump of the candidates, one sorted line each, so
that two runs can be diffed; `teasi md5s` does the same for the finished files, which differ
in their bytes (every tile gets a new random key) but have to agree in their decrypted
records. What the self-touching rings from above cost here: nodes and ways are unaffected,
and only for individual multipolygons is the centre off by metres.

| | PBF → chart file |
|---|---:|
| Denmark (494 MB PBF) | 5.2 s / 3.7 s |
| Great Britain (2.2 GB PBF) | 35 s / 17 s |

The tags the rules look at are written into an array once while reading, and the rules are
then evaluated against that rather than against a map — `poi_type` alone asks up to 43
questions per object, 370 million of them across Great Britain. Only for the seamarks does
`osm::TagMap` keep every tag, because their keys are open-ended (`seamark:light:3:colour`).

Two rounding rules are easy to get wrong: halves go towards the **even** side
(`f64::round_ties_even`), and the capitalisation of the seamark texts lower-cases the rest of
the word — `DGPS` becomes `Dgps`.

## The osmarea layer

The first compiler that needs an outside library: it merges all the areas of one class,
simplifies them, clips them to the country boundary and into blocks, and builds the sea from
the coastline. All of that is geometry, and geometry at this scale means **GEOS** — the same
bytes only ever come out of the same library, so `geos.rs` talks to libgeos directly, with
shapely's semantics on top (see below).

```bash
./target/release/teasi osmarea osm_ref/denmark-latest.osm.pbf osm_ref/denmark.poly \
    <original>/Denmark_osmarea.v20210810 build/Denmark_osmarea.v20210810 20210810
# a country without an original file: the sea from the worldwide land polygons
./target/release/teasi osmarea osm_ref/great-britain-latest.osm.pbf \
    osm_ref/great-britain.poly - build/GreatBritain_osmarea.v20260918 20260918 \
    --country=17 --land=osm_ref/land-polygons-split-4326/land_polygons.shp
```

`area.rs` reads the areas and the coastline, with the coordinates straight in osmarea units
of 360/2^25 degrees; `land.rs` reads the worldwide land polygons with a small shapefile
reader of its own — a polygon shapefile is a flat sequence of records, so it needs no crate.
`osmarea.rs` is the compiler itself.

### libgeos at runtime

`geos.rs` loads `libgeos_c` with `dlopen`, binds only the roughly 40 functions in use and
keeps one GEOS context per thread. The advantage: `cargo build` needs neither GEOS nor its
headers, only `teasi osmarea` and `teasi osm` need the library — and one can point it at any
build. It looks for `$TEASI_GEOS`, then `libgeos_c.so.1`, then `libgeos_c.so`:

```bash
TEASI_GEOS=/path/to/libgeos_c.so.1 ./target/release/teasi osmarea …
```

The reference results were taken with the GEOS that shapely bundles (3.13.1), picked up from
an installed shapely's `shapely.libs/` with `TEASI_GEOS` plus an `LD_LIBRARY_PATH` to the
same directory — its `libgeos_c` looks for `libgeos` next to itself, without an RPATH.
Reproducing a chart byte for byte needs that same version.

The wrappers stick to shapely's semantics: `parts` is `shapely.get_parts` (a polygon is its
own single part), `Geom::rect` builds the ring exactly
like `shapely.box`, open rings are closed as they are there, and the STRtree has shapely's
node capacity of 10 — otherwise a query would come back in a different order, and that order
decides which polygon enters a union first.

### Order decides the output

The obvious thing is to take the areas in the order the extractor produced them — and that is
the order in which libosmium flushes its buffers. That order decides which polygon enters a
union first and hence what ends up in the file, and it is not reproducible, so the areas are
sorted by their osmium id instead.

### What comes out, and where it is not exact

```bash
./target/release/teasi area osm_ref/denmark-latest.osm.pbf area.txt
```

That dump holds one line per area and coastline way, keyed by the osmium id.

| | Denmark | Great Britain |
|---|---|---|
| areas | 978,957 | 4,868,591 |
| coastline ways | 2230 | 28,184 |
| polygons per class | 700,620 in 16 classes | 2,520,840 in 16 classes |
| records | 340 | 1157 |
| runtime (PBF → chart file) | 61 s | 190 s, 7.1 GB |

The areas whose rings touch themselves are the ones that come out differently from libosmium
— 55 in Denmark, 347 in Great Britain (see above) — and that reaches further than it sounds:
one such area changes the union of its class, and with it every block the union lies in,
while one record holds every class of a cell. On the ground it is individual rings with one
point more or less.

The sea is the most involved part of this and matches completely: the coastline is assembled
into chains (land on the left, open chains closed with a straight line), outside the boundary
`#OW` comes from the original file, and its rings are combined pairwise in a tree under the
even-odd rule.

## The osm layer

The largest compiler: it builds four kinds of record out of the street ways — **D** with the
edges and lines, **A** with the name table, **B** with the routing graph and **C** with the
overview lines. It holds the whole country at once, as flat `Vec`s with index columns rather
than per-object structs — the 12.1 M graph nodes and 30 M edges of Great Britain fit in
16 GB that way.

```bash
./target/release/teasi ways osm_ref/denmark-latest.osm.pbf ways.txt   # the extractor alone
./target/release/teasi osm --heights=build/ref/heights.bin \
    osm_ref/denmark-latest.osm.pbf osm_ref/denmark.poly \
    <original>/Denmark_osm.v20210916 build/Denmark_osm.v20210916 20260918
# a country without an original file: c2/c4 stay empty, nothing is copied
./target/release/teasi osm --heights=build/gb/dem.bin --country=17 "--name=United Kingdom" \
    osm_ref/great-britain-latest.osm.pbf osm_ref/great-britain.poly - \
    build/GreatBritain_osm.v20260918 20260918
```

`way.rs` keeps the ways that `road_class` or `line_type` accepts, with their node ids, then
the coordinates of those nodes. Keeping the rest would be wasted: the compiler only sees
streets and lines, and the node rows of the others never enter a calculation.  `osm.rs` is
the compiler.

The order of the ways decides how entries of equal rank are sorted within a record. Reading
the file front to back gives the order of the file, which for a Geofabrik extract is by id;
`pbf.rs` decodes the blocks in parallel instead and therefore sorts explicitly by way id
afterwards.

### Heights

The ascent of an edge (B edge word [2]) comes from node heights, and those come from the
**Copernicus DEM GLO-90** (`dem.rs`): `teasi dem` downloads the 1°×1° tiles covering an area
from the public AWS bucket, decimates them onto one grid of 3″ (1200 points per degree) and
smooths it with a Gaussian of σ = 1 grid point, because the DEM is a *surface* model and
trees and houses would otherwise add up to spurious ascents along the roads.

```bash
./target/release/teasi dem osm_ref/great-britain.poly osm_ref/dem build/gb/dem.bin
```

Great Britain is 234 tiles and a 15,600 × 21,600 grid: 3.6 s and 3.3 GB from cached tiles.
Two pieces of this have to be exact, and both are pinned against the libraries that define
them (`tests/compat.rs`):

- **The GeoTIFF.** The tiles are float32, one DEFLATE-compressed tile per file, with the
  floating-point predictor of TIFF Technical Note 3: per tile row the bytes are accumulated
  with a stride of **one sample** — not one value, which is the easy mistake, and a tempting
  one because flat zero areas still come out right, so three quarters of the grid looks fine
  — and then the byte planes are de-interleaved, most significant first. The images get
  narrower towards the pole (1200 columns below 50° N, 800 up to 60°, 600 above), which is
  what the decimation to 1200 is for.
- **The Gaussian.** `scipy.ndimage.gaussian_filter`, down to the rounding: the kernel is
  normalised with **numpy's pairwise summation**, each pass accumulates in f64 and stores
  f32, and the taps are added **last to first**. Adding them the other way round is in fact
  the more accurate of the two (`math.fsum` agrees with it), but it moves one value in 337
  million by an ULP — with the order right, the whole 1.35 GB grid for Great Britain matches
  scipy's own output byte for byte.

### What comes out

```bash
./target/release/teasi ways osm_ref/denmark-latest.osm.pbf ways.txt
```

The dump holds the class or line type, the flags, the number of rows, the name and an MD5
over the node ids and coordinates (as IEEE bit patterns) per way, so two runs of the
extractor can be diffed line by line — tag tables included.

| | Denmark | Great Britain |
|---|---|---|
| ways from the extractor | 1,538,105 | 8,481,450 |
| records | 5982 | 22,711 |
| file size | 92,253,033 B | 531,333,357 B |
| runtime (PBF → chart file) | 66 s | 4:13, 16.0 GB |

What that is made of: Denmark 1,454,524 streets and 83,581 lines, 1,816,001 shared nodes,
3,004,615 edges, 2,404,747 graph nodes in 354 B cells (336 kept), 134 A, 336 B, 5129 D and
95 C records, 21 tiles including the three copied Faroese tiles — Great Britain
7,861,689/619,761, 15,029,377 edges, 12,119,884 graph nodes, 20,507 D records.

## The ta layer (address search)

The last OSM compiler and the one with the most rules: it cuts the named streets into
pieces, attaches every house number to the nearest piece of the same name, gives every piece
its places, groups pieces into streets and builds the D and A records plus the country-wide
**search index** from them.

```bash
./target/release/teasi ta --country=17 "--name=United Kingdom" \
    osm_ref/great-britain-latest.osm.pbf osm_ref/great-britain.poly \
    build/GreatBritain_ta.v20260919 20260919
```

`ta.rs` needs both the addresses (`addr.rs`) and the streets (`way.rs`), and reads the PBF
file twice for that. `grid.rs` replaces scipy's `cKDTree`: a uniform grid over the points,
searched ring by ring outwards. Three queries are needed — the 8 nearest places within a
radius (`query(k=8, distance_upper_bound=…)`), the nearest place node of a name, and all
pairs under 60 m (`query_pairs`).

### Order is everything here

More results hang off orderings in this layer than in any other compiler: which place wins
a group is a most-common count with ties, the centres of the postcode districts are
floating-point sums, and the order of the index hits is the order in which they arose.
`ta.rs` therefore keeps an insertion-ordered map (`Ordered`) and a `Counter` that breaks ties
by insertion order. Three rules on top of that, all so that the same input yields the same
output twice:

1. Addresses, places and interpolation lines are **sorted canonically**. The extractor hands
   them out in libosmium's order, and `pbf.rs` reads the blocks in parallel anyway.
2. **A search index node's children** are sorted by character. Appending them in the
   iteration order of a hash set makes the file depend on the hash seed, so two runs of the
   same input disagree.
3. **The direction of a house number range** (from/to) comes from the correlation between
   position and number. If the correlation is 10⁻¹⁷ the numbers do not run along the piece at
   all, and the last bit of the covariance would decide whether the range counts up or down.
   Correlations below 10⁻¹² therefore count as zero (`CORR_TOL`).

The third one was the hardest to find: without it, 2 Danish and 37 British records came out
differently from one run to the next.

### What comes out

```bash
./target/release/teasi ta --country=4 --name=Denmark denmark-latest.osm.pbf denmark.poly \
    dk.v2 20260918
./target/release/teasi index dk.v2        # take the search index apart and rebuild it
```

| | Denmark | Great Britain |
|---|---|---|
| records | 3958 | 13,680 |
| search index | 1,943,962 B | 522,760 nodes |
| file size | 24,644,993 B | 113,851,396 B |
| runtime (PBF → chart file) | 40 s | 3:19, 13.7 GB |

What that is made of: Denmark 408,262 named streets, 1,257,070 edges, 1,296,848 pieces,
2,480,980 matched house numbers on 735,263 pieces, 117,162 streets, 99 A and 3859 D records,
12,483 places with streets and 4935 without; Great Britain 1,969,512 / 5,225,753 / 5,334,223
/ 4,909,355 / 915,297, 376 A and 13,304 D records, 48,352 places with streets, 82,998 without
and 2603 postcode districts.

The self-touching rings reach this layer too, through the addresses: a single extra address
can turn a range 111–147 into 111–149, and an extra place shifts the centre of a postcode
district — BN10 by 5 cm.

## The terrain layer (elevation model and map images)

`terrain.rs` builds the type 5 file: per region (1.40625°) 8×8 cells of 256×256 px, the
heights as JPEG 2000 tiles and the map images as JPEG, the latter additionally as a pyramid
of 4×4, 2×2 and 1×1. The heights come from `heights.rs` (what `teasi dem` wrote), the land
cover from `area.rs` and the sea from `land.rs`.

```bash
# the elevation profile only, like region (122,20) of Denmark_terrain
./target/release/teasi terrain build/gb/dem.bin osm_ref/great-britain.poly out 20260919

# with map images (needs libgeos, see above)
./target/release/teasi terrain --country=17 \
    --land=osm_ref/land-polygons-split-4326/land_polygons.shp \
    --area=osm_ref/great-britain-latest.osm.pbf \
    build/gb/dem.bin osm_ref/great-britain.poly out 20260919    # 1:24, 7.9 GB
```

### The pieces of Pillow

Drawing the regions needs five things that live in libraries rather than in any code of our
own. Three of them are reproduced in `raster.rs`, line by line from the C sources, float
widths and rounding macros included; two stay libraries:

| Defined by | Here | identical? |
|---|---|---|
| Pillow's `ImageDraw.polygon` (`Draw.c: polygon_generic`) | `Mask::polygon` | bit-identical |
| Pillow's `Image.resize(…, LANCZOS)` (`Resample.c`) | `raster::resize` | bit-identical |
| numpy's `gradient`, for the hillshade | `terrain::shade` | bit-identical |
| OpenJPEG, as Pillow's `JPEG2000` plugin drives it | `raster::jp2`, `openjpeg-sys` | down to one byte |
| libjpeg-turbo | `raster::jpeg`, `jpeg-encoder` | no |

With the polygon filler every detail counts: the coordinates are **truncated towards zero**
as in C (not rounded), an edge yields its `x` a **second time** when the scanline hits its
`ymax` and it is not the last row (otherwise a pass-through vertex counts twice and the row
flips), horizontal edges are drawn directly instead of being intersected, and the fill runs
from `floor(x+0.5)` to `ceil(x-0.5)`. With "a sensible guess" instead of the original, 80 %
of the polygons deviated; with the original, none out of 5000
(`polygon_fill_matches_pil`).

The resizing works in fixed point like Pillow: the Lanczos coefficients are multiplied by 2²²
and rounded away from zero to `i32`, the accumulator starts at 2²¹ and is shifted right by 22
bits at the end. The horizontal direction runs first, and only over the rows the vertical one
actually reads.

### JPEG 2000 and JPEG

For the elevation tiles the same library Pillow uses is needed: a pure Rust encoder would
deliver different bytes (how the rate is distributed over the code blocks is a matter of
implementation), and what the decoder in the device accepts is not documented.
`openjpeg-sys` compiles OpenJPEG 2.5.3 into the binary; the parameters are those of Pillow's
plugin (`irreversible`, 6 resolutions, code blocks 64×64, LRCP, one layer, ratio 50), and the
version it writes into the codestream's comment marker is its own.

The map images go through `jpeg-encoder` (pure Rust) instead of libjpeg-turbo: baseline,
4:2:0, standard Huffman tables, IJG quantisation for quality 80, JFIF with 96 dpi and the
EXIF APP1 of the originals. The bytes are different — the segments come in a different order,
and the chroma subsampling averages per block while libjpeg filters triangularly. For images
that are lossy anyway and that the file keeps no checksum over, that is accepted.

### What comes out

| | Regions | Elevation tiles | Map images | PBF and shapefile → chart file |
|---|---|---|---|---:|
| Denmark | 28 | 1034 | 2029 | 17 s, 1.7 GB — 7 s of that the regions |
| Great Britain | 85 | 1999 | 5391 | 1:24, 7.9 GB |

Those times include reading the 2.2 GB PBF and the 1.3 GB shapefile; the drawing itself is
spread across every core. `teasi check` cannot look into this layer — it has no slot areas —
so a region has to be unpacked to be inspected.

The self-touching rings cost a little here as well: 525,037 land cover areas go into the
Danish regions, three of which depend on how those 55 of 978,957 areas are split. The number
of images and elevation tiles does not change.

## Two pitfalls

**Raw LZMA1.** liblzma only writes the `.lzma` format, and that is exactly a 13-byte header
in front of the raw stream — so cut the header off when writing and put it back when reading.
But the header declares the length as *unknown*: our own records end with the end marker
liblzma appends, the originals do not, and a stream with a declared length must not have that
marker. With "unknown", liblzma accepts both, and the caller stops after `pltx` bytes —
exactly like `LzmaDecode` in the firmware.

**Discarded fields.** Every container also returns the fields nobody interprets and writes
them back unchanged. That is the only reason the round trip is byte-identical; recomputing
lengths and offsets would be more convenient but would lose exactly the places where the
originals depart from our assumptions.
