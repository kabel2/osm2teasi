# teasi (Rust)

The tool chain ported to Rust. Finished are **the shell** (stage 1: decryption,
compression, every record container, the writer, the search index), **reading OSM**
(stage 2: PBF reader, node index, address extraction), **all five OSM layer compilers**
(stage 3: `osmpoi`, `osmpoint`, `osmarea`, `osm` and `ta`, straight from the PBF into the
chart file) and **the `terrain` layer** (stage 4: elevation model and map images). The
Python tools in [../tools/](../tools/) remain the reference; what is here has to deliver the
same.

Only the height sources themselves stay in Python (`osm_heights.py` with scipy's `lsqr`,
`dem_heights.py` with the Copernicus model); Rust reads their results, see below.

## Status

| Building block | Python | Rust |
|---|---|---|
| PC1, MD5 checksum, global key | `pc1.py`, `chart.py` | `pc1.rs`, `chart.rs` |
| Raw LZMA1 as in the originals | `chart.py` | `lzma.rs` |
| Containers A, B, C, D, osmpoint | `layers.py` | `layers.rs` |
| Writing files, device binding | `writer.py` | `writer.rs` |
| Search index of the address search | `ta_index.py` | `ta_index.rs` |
| Reading OSM PBF, node index | pyosmium | `pbf.rs` |
| Addresses, places, interpolation ways | `osm_addr_extract.py` | `addr.rs` |
| Country boundary (`.poly`) | `poly.py` | `poly.rs` |
| Reading POIs | `osm_poi_extract.py` | `poi.rs` |
| Building the `osmpoi` layer | `compile_osmpoi.py` | `osmpoi.rs` |
| Building the `osmpoint` layer | `compile_osmpoint.py` | `osmpoint.rs` |
| Reading areas and the coastline | `osm_area_extract.py` | `area.rs` |
| Worldwide land polygons (shapefile) | `land_extract.py` | `land.rs` |
| Geometry (GEOS) | shapely | `geos.rs` |
| Building the `osmarea` layer | `compile_osmarea.py` | `osmarea.rs` |
| Reading street and line ways | `osm_extract.py` | `way.rs` |
| Reading and querying heights | `osm_heights.py`, `dem_heights.py` | `heights.rs` |
| Building the `osm` layer | `compile_osm.py` | `osm.rs` |
| Nearest-neighbour search (instead of scipy's kd-tree) | `scipy.spatial` | `grid.rs` |
| Building the `ta` layer (address search) | `compile_ta.py` | `ta.rs` |
| Drawing, scaling, JPEG, JPEG 2000 | Pillow, OpenJPEG | `raster.rs` |
| Building the `terrain` layer | `compile_terrain.py` | `terrain.rs` |

## Building and checking

```bash
cargo build --release
cargo test                 # reference values from the Python tools
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

| Command | Purpose |
|---|---|
| `teasi info <map>…` | header, tiles, record counts per slot area |
| `teasi check <map>…` | take every record apart and rebuild it byte-identically |
| `teasi roundtrip <map> [out]` | decrypt the whole file, write it again, compare the records |
| `teasi md5s <map>` | `area cx cy md5` per record — to compare with Python |
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
| `teasi terrain <heights> <poly> <map> [date]` | build the elevation model and the map images (`--land=…`, `--area=…`, `--rate=R`, `--only=x,y`) |

As with the Python tools, the serial number comes from `TEASI_DEVICE` (the default being the
one in `chart.rs`).

## Measured

Against the original Danish map, 16 cores. "Python" is the same thing through
`tools/layers.py` or `tools/ta_index.py`.

| | Python | Rust |
|---|---:|---:|
| decrypting osmpoi (3704 records) | 4.5 s | 0.06 s |
| decrypting osmarea (368) | 16.2 s | 0.33 s |
| decrypting ta (4125) | 20.1 s | 0.42 s |
| decrypting osm (5878) | 56.9 s | 1.35 s |
| taking the search index apart and rebuilding it | 0.80 s | 0.12 s |
| rewriting osmpoi (`roundtrip`) | 24 s | 3.1 s |

So reading is 40 to 70 times faster, because PC1 in Python manages only 0.13 MB/s. For
**writing** the gap is smaller (8×), since what counts there is the LZMA compression, and
both languages hand that to the same C library at around 1.5 MB/s per core. Rewriting the
whole Danish osm file takes 61 s in Rust; that is almost pure compression time and cannot be
pushed down much further.

## Stage 2: reading OSM

`pbf.rs` reads `.osm.pbf` files (the `osmpbf` crate), decodes the blocks in parallel and
folds them into per-thread accumulators. Then the node index: pyosmium keeps an index of
**every** node in the file in RAM, whereas here only the ids that an earlier pass asked for
are collected — sorted, 16 bytes per node.

`addr.rs` is `osm_addr_extract.py`, in three passes (a relation needs its ways, which need
their nodes):

1. relations: which `multipolygon` or `boundary` relations carry address or place
   information,
2. ways: interpolation ways, closed ways (areas in their own right) and the member ways,
3. nodes: the results that are nodes, the coordinates for the ways, and the house numbers of
   the interpolation nodes.

### Checked against Python

```bash
./target/release/teasi addr osm_ref/denmark-latest.osm.pbf rs.txt
python scripts/addr_dump.py build/ref/addr_dk.pkl py.txt     # the same from the pickle
python scripts/addr_compare.py py.txt rs.txt
```

`addr_dump.py` writes the entries canonically, with coordinates as IEEE bit patterns;
`addr_compare.py` pairs them by their tag fields and measures the deviation in metres.

**Denmark** (`denmark-latest`): all 2,628,399 entries **bit-identical** — 2,614,623
addresses and 13,776 places.

**Great Britain**: 5,023,341 of 5,023,358 addresses bit-identical, 111,338 of 111,340
places, all 140,467 interpolation points. 17 addresses are off by a median of 0.93 m (at
most 5.6 m), 2 places by 27 m, and 2 addresses are added — all of them areas whose rings
touch themselves (see below).

| | Python | Rust |
|---|---:|---:|
| Denmark (494 MB PBF) | 270 s, 2.2 GB | 4.7 s, 1.5 GB |
| Great Britain (2.2 GB PBF) | ~20 min | 31 s, 5.3 GB |

The three passes take 6, 8 and 11 s for Great Britain; one pass over the 2.2 GB costs about
6 s on its own, and the rest is the actual work.

### Assembling areas the way libosmium does

Everything that hangs off an area — the centre of an address, the position of a POI, the
rings of the osmarea layer — comes from libosmium in Python. Its area assembler is
reproduced in `pbf.rs` (`assemble_segments`, `split_rings`, `area_loc_rings`), like this:

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

What that buys, in numbers: the British addresses went from 712,719 via 5,022,875 to **all
5,023,357** bit-identical, and the Danish osmpoi records from 3693 to **all 3699**. Of
978,957 Danish OSM areas the extractor builds 978,902 bit-identically.

**What is left** are areas whose rings touch themselves — 55 of 978,957 in Denmark, 347 of
4,868,591 in Great Britain (0.007 %): at such crossings libosmium does not follow "the first
free edge" but searches, splits and rejoins the rings afterwards. The number of rings or the
starting point of a ring then differs. Reproducing that part of the assembler (about 1000
lines in libosmium) is not worth it for a thousandth of a thousandth.

A second spot was subtler: since 3.12, CPython's `sum()` adds floats with **compensation**
(Neumaier). A naive `for` loop in Rust was therefore one bit off — `osm::fsum` now does the
same thing.

## Stage 3: the osmpoi and osmpoint layers

`poi.rs` is `osm_poi_extract.py --filter` and serves both layers — it keeps whatever
`poi_type` accepts plus everything with a `seamark:type`. On top of it sit `osmpoi.rs`
(`compile_osmpoi.py`: type rules, the attribute string, deduplication) and `osmpoint.rs`
(`compile_osmpoint.py`: categories, colours, topmarks, light strings, sectors). No pickle in
between — one command from the extract to the chart file:

```bash
./target/release/teasi osmpoi osm_ref/great-britain-latest.osm.pbf \
    osm_ref/great-britain.poly build/GreatBritain_osmpoi.v20260918 20260918 --country=17
./target/release/teasi osmpoint …    # the same arguments
```

Checking happens in two stages. The candidates against the Python pickle first, as with the
addresses:

```bash
./target/release/teasi poi osm_ref/denmark-latest.osm.pbf rs.txt
python scripts/poi_dump.py build/ref/poi_dk.pkl py.txt
python scripts/poi_compare.py py.txt rs.txt      # pairs by kind and OSM id
```

Then the finished file record by record against the map built with Python (`teasi md5s` on
both, then `diff`). The files themselves always differ, because every tile gets a new random
key; the decrypted records have to be equal.

| | Candidates bit-identical | Records bit-identical |
|---|---|---|
| osmpoi Denmark | 88,978 nodes, 18,596 ways, **330 of 330** relations | **3699 of 3699** |
| osmpoi Great Britain | 601,625 nodes, 217,134 ways, 2125 of 2139 relations | 15,603 of 15,619 |
| osmpoint Denmark | | **128 of 128** |
| osmpoint Great Britain | | **354 of 354** |

What deviates are the area rings from above: nodes and ways agree 100 %, and only for
individual multipolygons is the centre off by metres.

| | Python (the extraction alone) | Rust (PBF → chart file) |
|---|---:|---:|
| Denmark (494 MB PBF) | 3 min | 5.2 s / 3.7 s |
| Great Britain (2.2 GB PBF) | 24 min | 35 s / 17 s |

The gap is larger than for the addresses because `poi_type` in Python makes up to 43
`dict.get` calls per object — 370 million across Great Britain. Here the interesting tags are
written into an array once while reading, and the rules are evaluated against that. Only for
the seamarks does `osm::TagMap` keep every tag, because their keys are open-ended
(`seamark:light:3:colour`).

Two small things that came up while reproducing this: Python's `round()` rounds halves
towards the even side (`f64::round_ties_even`), and `str.capitalize()` lower-cases the rest
of the word — `DGPS` becomes `Dgps`.

## Stage 3: the osmarea layer

The first compiler that needs an outside library: `compile_osmarea.py` merges all the areas
of one class, simplifies them, clips them to the country boundary and into blocks, and builds
the sea from the coastline — all of it with shapely, and therefore with **GEOS**. The same
bytes only come out of the same library, so `geos.rs` talks to libgeos directly.

```bash
./target/release/teasi osmarea osm_ref/denmark-latest.osm.pbf osm_ref/denmark.poly \
    <original>/Denmark_osmarea.v20210810 build/Denmark_osmarea.v20210810 20210810
# a country without an original file: the sea from the worldwide land polygons
./target/release/teasi osmarea osm_ref/great-britain-latest.osm.pbf \
    osm_ref/great-britain.poly - build/GreatBritain_osmarea.v20260918 20260918 \
    --country=17 --land=osm_ref/land-polygons-split-4326/land_polygons.shp
```

`area.rs` is `osm_area_extract.py` (areas and the coastline, coordinates straight in osmarea
units of 360/2^25 degrees), `land.rs` is `land_extract.py` together with a small shapefile
reader — a polygon shapefile is a flat sequence of records, so it needs no crate.
`osmarea.rs` is the compiler itself.

### libgeos at runtime

`geos.rs` loads `libgeos_c` with `dlopen`, binds only the roughly 40 functions in use and
keeps one GEOS context per thread. The advantage: `cargo build` needs neither GEOS nor its
headers, only `teasi osmarea` and `teasi osm` need the library — and one can use exactly the
one shapely uses. It looks for `$TEASI_GEOS`, then `libgeos_c.so.1`, then `libgeos_c.so`:

```bash
G=.venv/lib/python3*/site-packages/shapely.libs
TEASI_GEOS=$PWD/$G/libgeos_c-*.so.* LD_LIBRARY_PATH=$PWD/$G ./target/release/teasi osmarea …
```

(`LD_LIBRARY_PATH` is needed because shapely's `libgeos_c` looks for its `libgeos` next to
itself, without an RPATH.) For **bit-identical** results it has to be the same GEOS version
shapely has — 3.13.1 here. The wrappers stick to shapely's semantics: `parts` is
`shapely.get_parts` (a polygon is its own single part), `Geom::rect` builds the ring exactly
like `shapely.box`, open rings are closed as they are there, and the STRtree has shapely's
node capacity of 10 — otherwise a query would come back in a different order, and that order
decides which polygon enters a union first.

### One change in Python

`compile_osmarea.py` took the areas in the order the extractor wrote them — and that is the
order in which libosmium flushes its buffers. That order decides which polygon enters a union
first and hence what ends up in the file. It cannot be reproduced, so **Python** now sorts by
the osmium id too (`load()` in `compile_osmarea.py`). That makes the output reproducible; the
map is the same as before, only assembled in a defined order.

### Checked against Python

In two stages as with the POIs — the extractor against the pickle first, then the records:

```bash
./target/release/teasi area osm_ref/denmark-latest.osm.pbf rs.txt
python scripts/area_dump.py build/ref/area_latest.pkl py.txt
python scripts/area_compare.py py.txt rs.txt          # pairs by the osmium id
python scripts/osmarea_compare.py py.v20210810 rs.v20210810   # object by object
```

| | Denmark | Great Britain |
|---|---|---|
| areas bit-identical | 978,902 of 978,957 | 4,868,244 of 4,868,591 |
| coastline ways bit-identical | 2230 of 2230 | 28,184 of 28,184 |
| polygons per class | 700,620 in 16 classes, 6 of them deviating | 2,520,840 in 16 classes, 52 of them deviating |
| records bit-identical | 308 of 340 | 1029 of 1157 |
| runtime (PBF → chart file) | 61 s | 190 s, 7.1 GB |
| Python (from the pickle on) | 150 s + 1 min extraction | 456 s + 10 min extraction |

The deviating records hang off the deviating areas from above (55 in Denmark, 347 in Great
Britain): a single deviating area changes the union of its class and with it every block it
lies in — and one record holds every class of a cell. Compared object by object
(`osmarea_compare.py`) it comes down to individual rings with one point more or less.

The sea is the most involved part of this and matches completely: the coastline is assembled
into chains (land on the left, open chains closed with a straight line), outside the boundary
`#OW` comes from the original file, and its rings are combined pairwise in a tree under the
even-odd rule.

## Stage 3: the osm layer

The largest compiler: `compile_osm.py` builds four kinds of record out of the street ways —
**D** with the edges and lines, **A** with the name table, **B** with the routing graph and
**C** with the overview lines. Python computes that with numpy across the whole country and
needs 18 GB for it; here they are flat `Vec`s with the same index columns.

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

`way.rs` is `osm_extract.py`: the ways that `road_class` or `line_type` accepts, with their
node ids, then the coordinates of those nodes. That corresponds to `--filter` on the Python
side; the ways Python additionally keeps without the switch cannot reach the output (the
compiler only sees streets and lines, and the node rows of the rest do not enter any
calculation). `osm.rs` is the compiler.

The order of the ways decides how entries of equal rank are sorted within a record. Python
takes it as libosmium hands it out — the order of the file, and the Geofabrik extracts are
sorted by id. Rust decodes the blocks in parallel and therefore sorts explicitly by way id
afterwards.

### Heights

The ascent of an edge (B edge word [2]) comes from node heights, and those come either from
the routing graph of an original file (`osm_heights.py`, scipy's `lsqr` over 2.1 M equations)
or from the Copernicus elevation model (`dem_heights.py`, tiles from the AWS bucket, Gaussian
smoothing). Both are one-off computations and stay in Python;
`scripts/heights_export.py` writes their pickle into a flat binary file that `heights.rs`
reads:

```bash
python scripts/heights_export.py build/ref/heights.pkl build/ref/heights.bin
```

For the node heights the 4 nearest neighbours are needed (inverse distance²). Instead of a
kd-tree as in scipy, a uniform grid lies over the known points and is searched ring by ring
outwards until the next ring cannot be any closer — the same result, only without a tree.

### Checked against Python

In two stages as with the other layers:

```bash
./target/release/teasi ways osm_ref/denmark-latest.osm.pbf rs.txt
python scripts/ways_dump.py build/ref/ways_latest.pkl py.txt
python scripts/ways_compare.py py.txt rs.txt
./target/release/teasi md5s <python.v20210916> ; ./target/release/teasi md5s <rust.v20210916>
```

`ways_dump.py` writes the class or line type, the flags, the number of rows, the name and an
MD5 over the node ids and coordinates (as IEEE bit patterns) per way — which puts the tag
tables on the test bench as well.

| | Denmark | Great Britain |
|---|---|---|
| ways bit-identical (extractor) | **all 1,538,105** | – |
| records bit-identical | 5981 of 5982 | **all 22,711** |
| file size | 92,253,033 B (Python 92,253,016) | **531,333,357 B, identical** |
| runtime (PBF → chart file) | 66 s | 4:13, 16.0 GB |
| Python (from the pickle on) | 282 s + 2 min extraction | 18:23, 18.0 GB + 12 min extraction |

Every intermediate figure agrees down to the entry: 1,454,524 streets and 83,581 lines,
1,816,001 shared nodes, 3,004,615 edges, 2,404,747 graph nodes in 354 B cells (336 kept),
134 A, 336 B, 5129 D and 95 C records, 21 tiles including the three copied Faroese tiles —
for Great Britain correspondingly 7,861,689/619,761, 15,029,377 edges, 12,119,884 graph
nodes, 20,507 D records.

Denmark's one deviating record is a B cell in which **5 of 3,004,615 edges** have an ascent
that differs by 1 cm. The cause is 16 pairs of known node heights that sit at the **same
position** and carry different heights (up to 6 cm apart, reconstruction noise from `lsqr`);
which of the two enters the mean of the 4 neighbours is arbitrary in both implementations.
Great Britain takes the grid route and is therefore completely identical.

## Stage 3: the ta layer (address search)

The last OSM compiler and the one with the most rules: `compile_ta.py` cuts the named streets
into pieces, attaches every house number to the nearest piece of the same name, gives every
piece its places, groups pieces into streets and builds the D and A records plus the
country-wide **search index** from them.

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

More results hang off orderings in `compile_ta.py` than in any other compiler:
`Counter.most_common` breaks ties by insertion order, the centres of the postcode districts
are floating-point sums, and the order of the index hits is the order in which they arose.
`ta.rs` therefore has an `Ordered` map and a `Counter` that behave like Python's `dict` and
`collections.Counter`. On top of that, **three changes in Python**, all of them so that the
same run yields the same output twice:

1. Addresses, places and interpolation lines are **sorted canonically**. The extractor hands
   them out in libosmium's order, which cannot be reproduced (and Rust reads the blocks in
   parallel).
2. **A search index node's children** are sorted by character. Previously they were appended
   in the iteration order of a `set` of strings — which changes with the hash seed from run
   to run, so Python was not even reproducible with itself.
3. **The direction of a house number range** (from/to) comes from the correlation between
   position and number. If the correlation is 10⁻¹⁷ the numbers do not run along the piece at
   all, and the last bit of numpy's covariance decided whether the range counts up or down.
   Both sides now take correlations below 10⁻¹² as zero (`CORR_TOL`).

The third change was the last remaining deviation: without it there were 2 Danish and 37
British records, with it none.

### Checked against Python

```bash
python tools/compile_ta.py --country=4 --name=Denmark ways.pkl addr.pkl denmark.poly py.v2 20260918
./target/release/teasi ta --country=4 --name=Denmark denmark-latest.osm.pbf denmark.poly rs.v2 20260918
./target/release/teasi md5s py.v2 ; ./target/release/teasi md5s rs.v2     # then diff
./target/release/teasi index rs.v2                                        # the search index
```

| | Denmark | Great Britain |
|---|---|---|
| records bit-identical | **all 3958** | 13,679 of 13,680 |
| search index | **byte-identical** (1,943,962 B) | 9 of 522,760 nodes differ |
| file size | **24,644,993 B, identical** | **113,851,396 B, identical** |
| runtime (PBF → chart file) | 40 s | 3:19, 13.7 GB |
| Python (from the pickle on) | 85 s + 2 min extraction | 4:12, ~16 GB + 12 + 20 min extraction |

Here too every intermediate figure agrees: Denmark 408,262 named streets, 1,257,070 edges,
1,296,848 pieces, 2,480,980 matched house numbers on 735,263 pieces, 117,162 streets, 99 A
and 3859 D records, 12,483 places with streets and 4935 without; Great Britain
1,969,512 / 5,225,753 / 5,334,223 / 4,909,355 / 915,297, 376 A and 13,304 D records, 48,352
places with streets, 82,998 without and 2603 postcode districts.

The one deviating British record and the 9 index nodes do **not** go back to the ta compiler
but to the known residue in the address extractor (areas whose rings touch themselves): Rust
finds 2 addresses more, one of which turns the range 111–147 into 111–149 and one of which
shifts the centre of the district BN10 by 5 cm; the two other affected index hits are the two
place areas that already differ by 27 m there.

## Stage 4: the terrain layer (elevation model and map images)

`terrain.rs` builds the type 5 file: per region (1.40625°) 8×8 cells of 256×256 px, the
heights as JPEG 2000 tiles and the map images as JPEG, the latter additionally as a pyramid
of 4×4, 2×2 and 1×1. The heights come from `heights.rs` (the export of `dem_heights.py`), the
land cover from `area.rs` and the sea from `land.rs`.

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

Five places in the Python compiler sit in libraries rather than in its own code. Three are
reproduced in `raster.rs`, line by line from the C sources, float widths and rounding macros
included; two stay libraries:

| Python | Rust | identical? |
|---|---|---|
| `ImageDraw.polygon` (`Draw.c: polygon_generic`) | `Mask::polygon` | bit-identical |
| `Image.resize(…, LANCZOS)` (`Resample.c`) | `raster::resize` | bit-identical |
| `np.gradient` for the hillshade | `terrain::shade` | bit-identical |
| `save("JPEG2000", …)`, OpenJPEG | `raster::jp2`, `openjpeg-sys` | down to one byte |
| `save("JPEG", …)`, libjpeg-turbo | `raster::jpeg`, `jpeg-encoder` | no |

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
plugin (`irreversible`, 6 resolutions, code blocks 64×64, LRCP, one layer, ratio 50). The
result is **byte-identical** down to one byte: OpenJPEG writes its own version into the
comment marker, and Pillow ships 2.5.4. That stays as it is — writing a false version into
the file would be worse than one byte of difference, and `scripts/terrain_compare.py` masks
it out.

The map images go through `jpeg-encoder` (pure Rust) instead of libjpeg-turbo: baseline,
4:2:0, standard Huffman tables, IJG quantisation for quality 80, JFIF with 96 dpi and the
EXIF APP1 of the originals. The bytes are different — the segments come in a different order,
and the chroma subsampling averages per block while libjpeg filters triangularly. For images
that are lossy anyway and that the file keeps no checksum over, that is accepted.

### Checked against Python

`scripts/terrain_compare.py` compares two chart files region by region — the tables, the
elevation tiles byte for byte, the map images as pixels. `teasi check` cannot do this; the
layer has no slot areas.

| | Regions | Elevation tiles | Map images |
|---|---|---|---|
| Denmark | 28 of 28 | **1034 of 1034** | 516 of 2029 pixel-identical |
| Great Britain | 85 of 85 | **1999 of 1999** | 2680 of 5391 pixel-identical |

The elevation tiles are identical down to the version byte, `a0` and `a1` included; not a
single one deviates in its codestream. For the map images the mean deviation is 0.025 out of
255 and the median of the largest deviation per image is **1**; in the worst image 95, as
ringing around individual pixels in densely drawn areas. That half of the images come through
pixel-identically — and just as detailed ones as the deviating ones — shows that the source
images agree and only the encoder differs. The files are 36,886,754 instead of 36,887,738 B,
three hundred-thousandths smaller.

| | Python (the compiler alone) | Rust (PBF and shapefile → chart file) |
|---|---:|---:|
| Denmark (28 regions) | 10 s | 17 s, 1.7 GB — 7 s of that the regions |
| Great Britain (85 regions) | ~2 min, 10 GB | 1:24, 7.9 GB |

The two columns do not measure the same thing: Python is handed three finished pickles
(`dem_heights.py`, `land_extract.py`, `osm_area_extract.py` — over 20 min for Great Britain),
while Rust reads the 2.2 GB PBF and the 1.3 GB shapefile within its own times. The regions
alone are about as fast in Rust as in Python, because the work is already in C there and
spread across every core.

One small thing deviates before the drawing starts: Rust binds 525,037 instead of 525,034
land cover areas into the Danish regions. These are the same 55 of 978,957 areas that the
extractor already splits differently for the osmarea layer (rings that touch themselves); the
number of images and elevation tiles is the same in every region.

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
