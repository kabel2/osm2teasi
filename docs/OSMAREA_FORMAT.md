# osmarea format (`Denmark_osmarea.v20210810`)

As of 2026-09-18. **Container and semantics understood, compiler finished** (section
"Building it from OSM", runtime ~4 min). **Container fully understood**: `parse_c`/`build_c`
in `tools/layers.py` rebuild all 368 records byte-identically. The area classes were
assigned by matching against OSM (`denmark-220101.osm.pbf`).

Content: **areas** (land use, forest, water, built-up land …) as polygons, plus area
outlines and the fill for open water (`#OW`). Denmark: 21,588 + 6524 + 3339 objects in
368 records.

Shell, encryption, slot areas and the coordinate system: see
[CHART_FILES.md](CHART_FILES.md), sections 1, 1a and 3. The geometry format is in
[OSM_FORMAT.md](OSM_FORMAT.md) ("Geometry").

## Where it sits

| | |
|---|---|
| Header `0x50` | `4` (layer id) |
| Slot area | **C** (tile head `+0x140`, 4×4 sub-cells) |
| Reader in the firmware | `FUN_003e651c` (internal type 2) |
| 1 record | = every area of **one 4×4 sub-cell** (0.3515625°), coordinate unit 360°/2²⁵ |
| Coordinates | **without** a margin (unlike osm/ta); the points are exactly OSM nodes, rounded to 360°/2²⁵ |
| Oddity | The first directory tile is a placeholder `(0,0)` of size 0 |

## C record (general layout)

The C records of osm use the same container (see [OSM_FORMAT.md](OSM_FORMAT.md)).

```
0x00  15 × u32  head
        [0]  size of an array (overwritten with cell_x, see below)
        [1]  cell_y (overwritten)            [2]  0
        [3]  n c1 (0x10 B)   [4]  0 (pointer)
        [5]  n c2 (0x08 B)   [6]  0
        [7]  n c3 (0x18 B)   [8]  0
        [9]  n c4 (0x0C B)   [10] 0
        [11] n c5 (0x1C B)   [12] 0
        [13] n c6 (0x18 B)   [14] 0
0x3C  c1 … c6 one after another, every array TRANSPOSED BYTEWISE
      then the variable parts, array by array, element by element within each:
        c1: [2] = number of u32
        c2: [0] = number of u32
        c3: [1] = number of u16 (string), then [4] = number of u32
        c4: [1] = number of u32
        c5: [1] = number of u16 (string), then [5] = number of u32
        c6: [1] = number of u16 (string), then [4] = number of u32
```

The pointer fields (each the field behind the count, or `[0]` before the string length) are
0 in the file. There is no padding.

## osmarea: arrays c3, c5, c6

In osmarea c1, c2 and c4 are empty. Occupancy: c6 only (249 records), c3 + c5 + c6 (106),
c5 + c6 (13).

### c5: area with a class (0x1C B = 7 × u32), 21,588 of them

| Field | Content |
|---|---|
| `[0]`/`[1]` | name (pointer/length), always empty in DK |
| `[2]` | **area class** 0–17 |
| `[3]` | bounding box minimum, packed as `(v << 16) \| u` |
| `[4]` | bounding box maximum |
| `[5]` | number of u32 of geometry |
| `[6]` | 0 (pointer) |

Assigned by matching (the OSM way with the most points in common; multipolygons made of
untagged ways end up under "unassigned"):

| Class | Count | OSM (share) |
|---:|---:|---|
| 0 | 362 | miscellaneous: `landuse=plant_nursery` 30 %, `brownfield` 27 %, `animal_keeping` … |
| 1 | 412 | `landuse=commercial` 60 %, `retail` 32 % |
| 2 | 411 | `landuse=military` (39 %, the rest unassigned) |
| 3 | 1289 | `landuse=cemetery` 93 % |
| 4 | 1751 | `landuse=industrial` 65 %, `quarry` 13 %, `construction` 8 % |
| 5 | 2724 | leisure: `leisure=park/pitch/playground/golf_course`, `landuse=recreation_ground` |
| 6 | 3836 | `landuse=forest` 73 % (also `natural=wood`, `heath`) |
| 7 | 3646 | `landuse=meadow` 61 %, `grass` 19 % |
| 8 | 3689 | `landuse=farmland` 51 %, `farmyard` 30 % |
| 9 | 726 | `natural=beach` 63 % |
| 10 | 1953 | `natural=wetland` 78 % |
| 11 | 520 | `man_made=pier` as an area, and small islands (`place=islet` on a closed coastline), drawn over the sea |
| 16 | 35 | `man_made=groyne` 80 % |
| 17 | 234 | `man_made=breakwater` 67 % |

### c6: area without a class (0x18 B = 6 × u32), 6524 of them

| Field | Content |
|---|---|
| `[0]`/`[1]` | name, mostly empty. `#OW` (319 times, one per cell) = **sea**: the cell minus the land, islands as holes; for pure sea cells a square over the whole cell (bounding box 0 … `0x80008000`) |
| `[2]`, `[3]` | bounding box min/max |
| `[4]` | number of u32 of geometry |
| `[5]` | 0 (pointer) |

By the match these are **water areas** (`natural=water` 54 %, the rest mostly multipolygons
with no tags of their own on the way), plus `#OW` for the open sea.

### c3: outline (0x18 B = 6 × u32), 3339 of them

Same field layout as c6, name always empty. By the match 92 % **`landuse=residential`**, so
built-up areas (not outlines, as assumed earlier).

### Objects, blocks and geometry

**One object = every area of one class within one 4096-unit block.** Each cell is divided
into 8×8 blocks of 4096 units. The areas are clipped at the block borders, and all the
rings of one class within a block sit as parts in one object (c5: per class, c3/c6:
built-up/water). Exception: `#OW` is one object per cell and is not divided into blocks.

Geometry: a sequence of rings `[u32 (hi << 16) | n][n points]` (see OSM_FORMAT.md), each
ring closed (first = last point). Outer rings have a positive area, holes a negative one
(shoelace in (u, v)); the firmware fills by the even-odd rule per object (`FUN_003df11c`, a
scanline with an edge list).

**`hi` = level of detail:** the drawing routine skips a ring when `hi` is larger than the
current level of detail (`*(param_1 + 0x34) < hi`). Small areas therefore have 14, large
ones 9. The rule (by matching, 96.5 % hit rate): take the shorter side `m` of the bounding
box of the **whole** (merged) area before clipping, then `hi = 14 − k` for the largest
`k ≤ 5` with `m ≥ 33 · 2^k` units. Clipped pieces inherit the `hi` of the whole area, holes
get their `hi` from their own bounding box. `#OW` rings are always 9.

For ~92 % of the objects the bounding box in the struct matches the min/max of the points;
for the rest it is slightly larger (harmless).

## Rebuilding it

```python
import sys; sys.path.insert(0, "tools")
import layers as L
for tx, ty, cx, cy, g, raw in L.iter_records(open(PATH, "rb").read(), "C"):
    rec = L.parse_c(raw)
    for f in rec["c5"]:
        cls = f["s"][2]
        for level, pts in L.geometry_parts(f["v"][1]):
            ring = [L.to_latlon(cx, cy, g, p) for p in pts]
    assert L.build_c(rec) == raw
```

Head `[0]` in the original: for records with a single array its size (249 times), otherwise
mostly the size of c5 (117 times), rarely of c6 (2 times). The firmware overwrites the
field, so the value does not matter.

## Building it from OSM

```bash
.venv/bin/python tools/osm_area_extract.py osm_ref/denmark-latest.osm.pbf build/ref/area_latest.pkl   # ~1 min
.venv/bin/python tools/compile_osmarea.py build/ref/area_latest.pkl osm_ref/denmark.poly \
    2013021200000368/7/943/20317/Denmark_osmarea.v20210810 build/<dir>/Denmark_osmarea.v20210810 [YYYYMMDD]  # ~4 min
```

**Countries without an original file** (Great Britain, say): pass `-` instead of the
original, and the sea then comes entirely from the worldwide land polygons of
osmdata.openstreetmap.de (`land-polygons-split-4326`, built from the whole world's
`natural=coastline`), clipped with `tools/land_extract.py` (needs `pyshp`). Sea = the cell
minus the land in every tile that touches the boundary, so the coasts of the neighbouring
countries are included too (France at Dover, for instance). Details and runtimes:
[CHART_FILES.md](CHART_FILES.md) 5.7.

```bash
.venv/bin/python tools/land_extract.py osm_ref/land-polygons-split-4326/land_polygons.shp \
    osm_ref/great-britain.poly build/gb/land.pkl                                 # ~5 s
.venv/bin/python tools/compile_osmarea.py --country=17 --land=build/gb/land.pkl \
    build/gb/area.pkl osm_ref/great-britain.poly - build/gb/GreatBritain_osmarea.v20260918 20260918
```

The same in Rust, with no pickle in between (needs libgeos, see
[../rust/README.md](../rust/README.md)):

```bash
cd rust && ./target/release/teasi osmarea osm_ref/denmark-latest.osm.pbf osm_ref/denmark.poly \
    <original>/Denmark_osmarea.v20210810 build/Denmark_osmarea.v20210810 20210810   # 61 s
./target/release/teasi osmarea osm_ref/great-britain-latest.osm.pbf osm_ref/great-britain.poly \
    - build/GreatBritain_osmarea.v20260918 20260918 --country=17 \
    --land=osm_ref/land-polygons-split-4326/land_polygons.shp
```

`osm_area_extract.py` stores every area (closed ways and multipolygons, assembled by
osmium) carrying land-use tags, plus the coastline ways. `compile_osmarea.py` builds the
layer from them (the original was matched against `denmark-220101`, rules below):

1. **Class** from the tags, `landuse` before `natural`, `leisure`, `man_made`
   (`area_class`):

   | Target | OSM |
   |---|---|
   | c3 | `landuse=residential` |
   | c5 0 | `landuse=plant_nursery, brownfield, animal_keeping, fishfarm, garages, harbour, religious, scout_camp` |
   | c5 1 / 2 / 3 | `landuse=commercial, retail` / `military` / `cemetery` |
   | c5 4 | `landuse=industrial, quarry, construction, railway, landfill` |
   | c5 5 | `leisure=park, pitch, playground, garden, sports_centre, golf_course, track, marina …`, `landuse=recreation_ground` (**without** `nature_reserve`: large protected areas are missing from the original) |
   | c5 6 | `landuse=forest, scrub`, `natural=heath` (**not** `natural=wood/scrub/grassland`) |
   | c5 7 | `landuse=meadow, grass, greenfield, village_green, greenhouse_horticulture` |
   | c5 8 | `landuse=farmland, farmyard, allotments, orchard, vineyard` |
   | c5 9 / 10 | `natural=beach` / `natural=wetland` |
   | c5 11 / 16 / 17 | `man_made=pier` and islands (`place=islet`) / `groyne` / `breakwater` |
   | c6 | `natural=water` (except `water=river`), `landuse=basin, reservoir, aquaculture`, `waterway=dock` |

2. OSM areas below **150 units²** (~120 m²) are dropped (the adoption rate jumps there).
3. The areas are processed sorted by their osmium id. The order decides which polygon enters
   a union first, and the extractor's write order (libosmium's buffer order) cannot be
   reproduced — sorted, the result is reproducible.
4. All areas of one class are **merged**: in the original, neighbouring parcels sharing an
   edge are one ring (which is why only ~70 % of the fields share their nodes with the
   original).
5. `hi` per merged area (see above), then **Douglas-Peucker with 4 units** (dropped OSM
   nodes lie at most 4 units off the ring in the original), clipping to the Geofabrik
   boundary and to the 4096-unit blocks.
6. **Sea:** the original uses a worldwide coastline (Sweden, Norway and Germany included).
   Inside the boundary the compiler rebuilds the sea from `natural=coastline` (joining
   chains, land on the left; open chains end far outside and are closed with a straight
   line), outside it takes `#OW` from the original file.
7. Tiles with no cell inside the boundary (the **Faroe Islands**) are copied unchanged from
   the original. As in the original, all 16 cells of every tile are in the file (empty ones
   are dropped).

The result against the original (both built from `denmark-220101`, total area per class):
built-up land, industry, farmland, meadow, cemetery, beach and commercial ±8 %, sea 100 %,
forest +13 %, wetland +32 % (OSM changes, presumably), inland water −12 % (the original also
contains water beyond the boundary). From `denmark-latest` (2026-09-18): 700,620 areas,
26.4 MB, installed on the device.

## Open questions

1. Classes 12–15 (not used in DK).
2. The exact drawing order of c3/c5/c6 and the order of the c5 objects (the compiler is
   roughly like the original, `CLASS_ORDER`).
3. Why ~8 % of the bounding boxes are larger than their points.
4. Runtime: for Denmark Python needs 1 min for the extraction and 2.5 min for the compiler
   (merging the large classes and reading the original's sea), the Rust port 1 min from the
   PBF to the file.
