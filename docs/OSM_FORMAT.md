# osm format (`Denmark_osm.v20210916`)

As of 2026-09-18. **Container fully understood**: `layers.rs` rebuilds all 5878 records
byte-identically (A 161, B 406, C 105, D 5206). Classes, flags and line types were assigned
by matching against OSM (see below). Only individual fields are open, see "Open questions".
**Compiler:** `teasi osm` builds the layer from OSM, see "Building it from OSM".

Content: the **street and path network from OSM** (geometry, classes, names, lengths, nodes
for routing), plus rivers and other lines, a directory of street and place names for the
address search, and the unencrypted routing graph.

Shell, encryption, slot areas and the coordinate system: see
[CHART_FILES.md](CHART_FILES.md), sections 1, 1a and 3. The general D container is described
in [OSMPOI_FORMAT.md](OSMPOI_FORMAT.md).

## Where it sits

| Slot area | Grid | Records | Reader | Content |
|---|---|---:|---|---|
| **D** | 32×32 | 5206 | `FUN_003e56fc` | streets/paths (a1), polygons (a2), lines (a3, a4) |
| **A** | 4×4 | 161 | `FUN_003e6044` | directory of place and street names |
| **B** | 8×8 | 406 | `FUN_003e5d20` | routing graph (1.69 M nodes), **LZMA only, not encrypted** |
| **C** | 4×4 | 105 | `FUN_003e651c` | lines and polygons for small zoom levels |

### Coordinates in D and C records: a margin of 512

Unlike osmpoi, osmpoint and osmarea, the lines in osm (D and C) and in ta have a **margin of
512 units** around the sub-cell: what is stored is `u = x − x_cell + 512`, so the range
0 … 33,792 corresponds to −512 … 33,280. Rounding goes to the nearest whole unit. After
subtracting the margin the vertices lie **exactly** on OSM nodes (residual error ±0.5
units), checked at thousands of points. Without subtracting it, every point ends up about
70 m (D) or 600 m (C) to the north west. `to_latlon(cx, cy, g, p, margin=L.MARGIN)`.

Header `0x50` = `1` (layer id).

## Geometry (the same in every layer)

A geometry is a u32 array made of one or more **parts**:

```
u32 (hi << 16) | n     n = number of points in the part
n × u32                points, packed as (v << 16) | u, relative to the NW corner of the sub-cell
... the next part, up to the end of the array
```

For lines (osm, ta) `hi == n`. For areas (osmarea) `hi` is a level of detail (9–14), see
[OSMAREA_FORMAT.md](OSMAREA_FORMAT.md). The division into parts comes out exactly in all
~520,000 geometries checked (osmarea completely, osm-D every 5th record;
`geometry_parts()` in `layers.rs`). In that sample only 483 of 474,575 streets consist
of more than one part.

Streets are cut at the **margin** of the sub-cell (the cell ± 512): an edge near a cell
border therefore appears in both cells, in each up to 512 units beyond the border. The
intersection point is interpolated and rounded; the node ids and the length stay those of
the whole edge. If a line leaves the area and comes back in, several parts arise.

## D records: the street network

Arrays in use (Denmark, all 5206 records): a1 = 2,339,000, a3 = 31,734, a4 = 37,978,
a2 = 24, a5 = 0.

### a1: street edge (0x28 B = 10 × u32)

| Field | Content |
|---|---|
| `[0]` | **name** = byte offset into the name index table of the A record of the parent 4×4 cell (`[0] // 4` = the index), `0xFFFFFFFF` = no name |
| `[1]` | in osm always = `[0]` (in ta: the name of the other side of the street) |
| `[2]`, `[3]` | in osm always `0xFFFF7FFF` (in ta: house number ranges) |
| `[4]` | node id at the start |
| `[5]` | node id at the end |
| `[6]` | flags (bit field, see below) |
| `[7]` | bits 27–31: **street class** 0–15 · bits 24–26: always 0 · bits 0–23: **length** |
| `[8]` | number of u32 of geometry |
| `[9]` | 0 (pointer) |

**Length:** `[7] & 0xFFFFFF` ≈ metres × 4.1906, so one unit ≈ 0.2386 m (which is the
circumference of the Earth / (5·2²⁵)). Checked on ~140,000 edges > 30 m (median per class
4.186–4.193) and ~15,600 straight edges > 200 m (median 4.1906 at 55°–58° N, north-south as
well as east-west). The exceptions are isolated outliers (ferries presumably, see "Open
questions").

**Node ids:** `[4]`/`[5]` are **indices into the node list of the B record** (the routing
graph) of the 8×8 cell **the node lies in** (usually `(cx // 4, cy // 4)`; for edges across
an 8×8 border the neighbouring cell, which was the apparently "invalid" 0.4 %). The nodes
are the OSM nodes at junctions (used by several street ways) and at the ends of ways; between
two of them lies exactly one edge. **Motorways** (class 0) and a few other edges have the ids
0/0 and are not in the graph.

**Order:** the a1 entries of a record are sorted **ascending by class** (in every record),
and within a class mostly by name.

**Match against OSM** (Geofabrik extract `denmark-220101.osm.pbf`, `teasi ways`):
the vertices of the edges are **exactly OSM nodes** (after subtracting the margin of 512, see
"Coordinates in D records"). 2,298,873 of 2,339,000 edges (98.3 %) can be assigned to an OSM
way that way, 2,092,000 of them with every point. The edge almost always runs in the
direction of the way (1,954,721 forwards, 5122 backwards). Every table below rests on this
match.

**Street classes** `[7] >> 27`:

| Class | OSM | Count (assigned) |
|---:|---|---:|
| 0 | `highway=motorway`, `motorway_link` | 4652 |
| 1 | `trunk`, `trunk_link` | 1150 |
| 2 | `primary` | 20,157 |
| 3 | `secondary`, `primary_link` | 50,762 |
| 4 | `secondary_link` | 1662 |
| 5 | not used in DK | 0 |
| 6 | `tertiary` (+ `tertiary_link`) | 211,332 |
| 7 | `service` 47 %, `residential` 38 %, `unclassified` 13 %, `living_street` | 1,196,120 |
| 8 | `pedestrian` | 3086 |
| 9 | `route=ferry` | 1749 |
| 10 | `cycleway` | 118,109 |
| 11 | `footway` | 212,693 |
| 12 | `track` | 107,152 |
| 13 | `path` (+ `bridleway`) | 153,113 |
| 14 | `steps` | 10,263 |
| 15 | 10 edges, not assigned to any OSM way | – |

**Flags `[6]`** (the share set across all assigned edges; "P" = the share within the OSM
group):

| Bit | Meaning | Evidence |
|---:|---|---|
| 0 | roundabout (`junction=roundabout`) | P = 99.9 % |
| 1 | part of a **local cycle route** (`route=bicycle`, `network=lcn`) | 96 % |
| 2 | **regional cycle route** (`rcn`) | 100 % |
| 3 | **national cycle route** (`ncn`) | 99 % |
| 4 | restricted access: `access=private/no/destination/customers/forestry` | 76–100 % |
| 5 | `mtb:scale` present | 98–100 % |
| 6 | `highway=bridleway` | 98.7 % |
| 7 | **paved**: `surface` ∈ asphalt, paved, paving_stones, concrete…, or no `surface` | 100 %, vs. < 5 % for gravel, dirt, ground, sand, grass, unpaved … |
| 8 | **no `surface` tag** | 99.8 % without, 6.6 % with |
| 9–10 | **bicycle access** (2 bits): `00` = `bicycle=no`, `01` = untagged (the default), `10` = `bicycle=yes`, `11` = `bicycle=designated` or a cycle track/lane along the road (`cycleway=*`) | > 93 % each |
| 11 | **one-way** in the direction of the geometry (`oneway=yes`, roundabout) | 97–100 % |
| 13–15 | **lanes** (`lanes`), a 3-bit number 1–7 | `lanes=2` → 2 (97 %), `lanes=4` → 4 … |
| 18 | part of a **walking route** (`route=hiking/foot`) | 95–100 % |
| 20–23 | **cost factor index** 0–15: the router (`FUN_003cf4d8`) multiplies the edge cost by `table[i]/128`, with the table at VA `0x4fc040`: 128, 123, 118 … 68, 67, 66, 64 (index 0 = neutral, higher = cheaper). Only 2.3 % of the edges are ≠ 0, mostly short stretches along larger roads, independent of cycle routes and of the kind of way; not derivable from OSM (Tahuna data presumably) | firmware |
| 27 | part of a **EuroVelo route** (`icn`) | 99 % (EV12, EV3, EV10); EV7 is missing, it probably did not exist in 2021 |
| 28 | only together with bit 27 (30–57 % of the icn edges); not read by the router's cost function | |
| 29 | cycle track/lane **along the road** (`cycleway=track/lane/shared_lane/separate`) | 85–100 % |
| 30 | `bicycle=dismount` | 96.5 % |
| 31 | pedestrians explicitly allowed (`foot=yes/designated`) | 98–99 % |

Bits 12, 16, 17, 19 and 24–26 are (almost) never set. Individual bits also react to other
tags (`oneway=-1` does not set bit 11, for instance). For a compiler the rules in the table
are enough.

### a2: polygon (8 B = 2 × u32)

`[0]` = number of u32, `[1]` = pointer. The variable part: `u32 n` (the number of points),
then `n` points. Very rare (24 in DK). In the sample, 6 of 9 belong to administrative
boundaries (`boundary=administrative`). **Unlike** every other geometry, there is no `hi`
component and no parts here.

### a3: unnamed line (0x0C B = 3 × u32)

`[0]` = type, `[1]` = number of u32 of geometry, `[2]` = pointer.

| Type | Count DK | OSM (matched over every 3rd record) |
|---:|---:|---|
| 0 | 10,630 | `railway=rail`, `light_rail`, `subway`, `narrow_gauge`, `disused`, `tram`, `preserved`, `miniature` |
| 1, 2, 3 | 42, 15, 75 | only ways without any of the evaluated tags were assigned (meaning open, boundaries perhaps) |
| 4 | 12,170 | `man_made=pier` |
| 5 | 1465 | `man_made=groyne` |
| 6 | 643 | `man_made=breakwater` |
| 10 | 6694 | `power=line` |

### a4: named line (0x14 B = 5 × u32)

`[0]` = pointer to the name, `[1]` = length of the name (UTF-16 including the NUL), `[2]` =
type, `[3]` = number of u32 of geometry, `[4]` = pointer. Variable part: the name first, then
the geometry.

| Type | Count | Content |
|---:|---:|---|
| 7 | 37,601 | watercourses: `waterway=stream/river/canal` always, `ditch/drain` only with a name; the name empty, simple or multilingual: `[DANVidå¦GERWiedau]` |
| 8 | 351 | `seamark:type=navigation_line`, name `<orientation>(leading)` |
| 9 | 26 | `seamark:type=recommended_track`, name `<orientation>(fixed_marks)` |

Multilingual names: `[` + several `<3-letter language code><name>` separated by `¦`
(U+00A6) + `]`.

## A records: the directory of names and places

Reader `FUN_003e6044`. **Not transposed.**

```
0x00  7 × u32   head: [0],[1] = cell (overwritten), [3] = n entries,
                [5] = offset of blk5, [6] = offset of blk6 (relative to the record start)
0x1C  n × 0x1C  entries (7 × u32): [1] length of the name, [3] number of 0x18 sub-elements,
                [5] number of 0x10 sub-elements, [0]/[4]/[6] pointers
      then:     every name, every 0x18 sub-element, every 0x10 sub-element,
                the strings of the 0x18 sub-elements ([1] = length), the strings of the
                0x10 sub-elements, alignment to 4 bytes, the u32 arrays of the 0x10
                sub-elements ([2] = count)
blk5  u32 list: bits 0–23 = offset into blk6, bits 24–31 = number of words in the name
blk6  names: u16 length + UTF-16 characters (no NUL), largely sorted alphabetically
```

In Denmark every A record has exactly one entry ("Denmark"). In osm it has a single 0x18
sub-element without a string (fields `[0, 0, 0, n, 0, 0]`) and no 0x10 sub-elements. The
actual data sits in blk5/blk6. In the ta layer, by contrast, the 0x18 sub-elements hold
places with coordinates, see [TA_FORMAT.md](TA_FORMAT.md).

**Street names:** `blk5`/`blk6` make up the name table that `a1[0]` of the D records points
at. `blk6` contains every **word** once (in the order of its first appearance), `blk5` the
**names** (= the OSM tag `name`, with no fallback to `ref`), sorted without case and without
accents (Å like A, Æ like AE). Names made of several words (split at spaces) sit as
consecutive entries: the first carries the word count in its top byte, the following ones
have 0. The 0x18 sub-element holds the number of names in `[3]`, and the entry itself
`[2] = 2`. Examples:

```
(3, 'Aarhus') (0, 'Syd') (0, 'Motorvejen')      -> "Aarhus Syd Motorvejen"
(2, 'Søndre') (0, 'Ringgade')                   -> "Søndre Ringgade"
(1, 'Skanderborgvej')
```

Checked in Aarhus: Skanderborgvej, Silkeborgvej, Viborgvej, Vesterbrogade and Marselis
Boulevard are all in the right places. The D cell `(cx, cy)` in the 32-grid uses the A record
of cell `(cx // 8, cy // 8)` in the 4-grid.

## B records: the routing graph (unencrypted)

Reader `FUN_003e5d20`, an 8×8 grid. It is called from the same code area
(`0x3ce…–0x3d6…`) as the D reader, so presumably by the route calculation. **No PC1**, no
transposition, no strings. It is **not** a coarse network but the complete graph of every D
edge: 1,692,053 nodes in 406 cells, with each D edge appearing as an edge (in both
directions).

```
0x00  11 × u32  head: [3] = n nodes, [5] = n edges, [7] = n3 (always 0 in DK), [9] = n extra
0x2C  n nodes   × 12 B
      n edges   × 16 B
      n3        × 16 B
      n extra   × 12 B
```

**Nodes** (12 B). Node 0 is an empty placeholder, and the index is the node id from
`a1[4]`/`a1[5]`.

| Field | Content |
|---|---|
| `[0]` | bits 0–18 = index of the first edge, bit 19 = has an extra entry, bits 26–31 = number of edges (the adjacency list) |
| `[1]` | position `(v << 16) \| u` with `u = floor(x · 65535/65536)`, x = the position within the 8×8 cell in 360°/2²⁷ (**the cell is mapped onto 0…65535**, exact in 98–99 % of cases); no margin (`layers.b_node_latlon`) |
| `[2]` | **left turns** (crossing oncoming traffic): a list of 6-bit entries `from \| to << 3` (indices into the node's edge list: arriving over edge `from`, continuing over edge `to`), terminated by 0. The router adds 90,000 for one (`FUN_003cf4d8`, a loop over `node[2] >> 6k & 0x3f`). Only filled at nodes with **at least three edges of class ≤ 7** and only between those; a left turn = a change of direction of more than 35° to the left (the first segment, x scaled by cos(latitude)). At most 5 entries, otherwise 0 |

**Edges** (16 B), one per outgoing direction:

| Field | Content |
|---|---|
| `[0]` | bits 0–19 = **length** (as in `a1[7]`), bit 20 = 0, bits 21–24 = **street class** (as in `a1[7] >> 27`), bits 25–29 = **way category** (see below), bit 30 = **passable in this direction** (0 for a one-way against the direction), bit 31 = 1 |
| `[1]` | **flags**, identical with `a1[6]` (99.9 %) |
| `[2]` | **ascent in cm** in the direction of travel (the sum of the height gains); the router's cost is `weight₁·length + weight₂·[2]`. From `[2]`(u→v) − `[2]`(v→u) = h(v) − h(u) the node heights of the original can be reconstructed |
| `[3]` | **target node**: bits 25–31 = dx + 64, bits 18–24 = dy + 64 (the offset of the target's 8×8 cell), bits 0–17 = the node index there. `0x81…` = the same cell |

Way category (bits 25–29, matched against OSM): 18 = trunk/primary/secondary (plus the
links), 10 = tertiary/residential/unclassified/living_street, 6 = `service` and ferries,
16 = pedestrian/cycleway/footway/steps, 5 = track/path. B therefore separates `service` from
`residential`, which both have class 7 in D.

The extra entries (12 B, 104 in DK) belong to the nodes with bit 19: `[0]` = the node index,
`[1]`/`[2]` large values (meaning open). The edges of both directions carry the same flags
(`a1[6]`). Node 0 is `[0, 0, 0]`, and head `[3]` counts it.

## C records

Reader `FUN_003e651c`, the same container as osmarea (see
[OSMAREA_FORMAT.md](OSMAREA_FORMAT.md)), but **with the margin of 512** like the D records
(checked for c1). In osm only c1 (265 entries, 0x10 B), c2 (5) and c4 (42) are used:

- **c1**: `[0]` = `0xFFFFFFFF`, `[1]` = **street class 0–3** (motorway, trunk, primary,
  secondary), `[2]` = number of u32 of geometry. An overview network for small zoom levels:
  **one entry per cell and class** with many parts, heavily simplified (≈ 43,000 points for
  DK).
- **c2**: `[0]` = number of u32, geometry as in a2 (`n`, then the points).
- **c4**: `[0]` = type, `[1]` = number of u32 of geometry.

## Rebuilding it

```python
import sys; sys.path.insert(0, "tools")
import layers as L
d = open(PATH, "rb").read()
for tx, ty, cx, cy, g, raw in L.iter_records(d, "D"):
    rec = L.parse_d(raw)                     # rec["a1"][i] = {"s": [10 fields], "v": [geometry]}
    for e in rec["a1"]:
        cls, length_m = e["s"][7] >> 27, (e["s"][7] & 0xFFFFFF) / 4.1906
        for hi, pts in L.geometry_parts(e["v"][0]):
            coords = [L.to_latlon(cx, cy, g, p, margin=L.MARGIN) for p in pts]  # margin 512!
    assert L.build_d(rec) == raw
```

Likewise `parse_a`/`build_a`, `parse_b`/`build_b`, `parse_c`/`build_c`.

## Building it from OSM

```bash
teasi dem osm_ref/denmark.poly osm_ref/dem_dk build/latest/dem.bin           # 43 tiles, 1.2 s cached
teasi osm --heights=build/latest/dem.bin osm_ref/denmark-latest.osm.pbf osm_ref/denmark.poly \
    2013021200000368/7/943/20317/Denmark_osm.v20210916 \
    build/latest/Denmark_osm.v20210916 20260918            # 66 s
```

Two commands from the extract to the chart file; `--heights` is the elevation data, see below.
It needs libgeos, see [../rust/README.md](../rust/README.md). The steps in `osm.rs`:

1. **Streets**: ways with a `highway` from the class table (without `area=yes`) and
   `route=ferry`. They are split at every OSM node used by several street ways, and at the
   ends of ways.
2. **Length**: haversine (R = 6371 km) × 5·2²⁵ / (2π·R) (≈ 4.19 units per metre), with a
   median ratio to the original of 1.000.
3. **Flags** per the table above, with the corrections from the match: bits 7–8 are a
   **surface class** (3 = no `surface`, 1 = asphalt/paved/paving_stones/concrete…,
   2 = sett/cobblestone, 0 = otherwise); the bicycle bits 9–10 = 3 for `bicycle=designated`
   **or** `cycleway=track/lane/shared_lane/separate` (the key `cycleway` only, not
   `cycleway:right` and so on, which do not set bit 29 either); bit 31 also for
   `foot=permissive`. Bits 20–23 and 28 stay 0.
4. **Graph (B)**: the nodes per 8×8 cell sorted by position, indexed from 1; edges in both
   directions, bit 30 from `oneway` (`-1` backwards; `oneway:bicycle=no` both ways). Node
   `[2]` = the left turns per the rule above. Edge `[2]` = the ascent: the height at every
   vertex from the 4 nearest nodes of the original (inverse distance²) or, without an
   original, bilinearly from the Copernicus elevation model (`teasi dem`, see below); then
   the sum of the height gains. Edges whose end nodes are more than 63 cells
   apart (long ferries) do not enter the graph.
5. **D**: edges and lines (a3/a4) cut at the cell margin ± 512, a1 sorted by class and name.
   **A**: the name table per 4×4 cell, from the names of its 64 D cells.
   **C**: classes 0–3 joined with `shapely.line_merge`, Douglas-Peucker with 32 units
   (360°/2²⁵), one c1 entry per cell and class; c2/c4 (boundaries) from the original.
6. Only cells touching the area (`denmark.poly`) plus 32,768 units (one D cell); edges to
   nodes in omitted B cells drop out of the graph and their ids in D become 0. The Faroese
   tiles (west of 0°) are taken unchanged from the original (only with an original).
7. **Memory:** the edges, the graph nodes and their numbering per B cell are computed for
   the whole country at once, the B, D and A records afterwards **tile by tile**, and each
   tile is compressed as soon as it is done. Length, ascent and descent are kept per edge
   (differences of running sums over the node rows, so the values are those of the
   per-node sums). Great Britain needs 6.4 GB and 2:45 from the PBF to the file, Germany
   12.2 GB and 8:20.
8. **Full B cells:** a B cell takes fewer than 2¹⁹ edges (a node's first edge has 19 bits)
   and 2¹⁸ nodes (an edge's target has 18). Central Berlin has 745,713 edges and 275,297
   nodes in one cell. In such a cell the least needed ways leave the routing graph, a level
   at a time until it fits: steps and footways (sidewalks are mapped as footways beside the
   road), then `highway=service`, then paths and bridleways, then tracks. They stay in D
   with node ids 0, like the motorways. For Germany the first level is enough, in two
   cells; Denmark and Great Britain have no full cell.

**The ascents** come from the **Copernicus DEM GLO-90** (`teasi dem`: tiles from the public
AWS bucket `copernicus-dem-90m`, resampled onto a 3″ grid, Gaussian smoothing σ = 1 grid
point, because the model is a surface model with trees and houses). Calibrated on Denmark
against the original ascents: σ = 0 gives a correlation of 0.76 (sum 1.77×), **σ = 1 a
correlation of 0.84 and a median ratio of 0.97**, 61 % within 10 cm, 88 % within 50 cm;
σ = 2 gives 0.80, σ = 4 gives 0.69. Without `--heights` every ascent stays 0.

**Countries without an original file** (Great Britain): pass `-` as the original, and then
c2/c4 stay empty and nothing is copied; `--country=N` (the header's country code,
CHART_FILES.md 1), `--name=<country>` (the country name in the A record).

```bash
teasi dem osm_ref/great-britain.poly osm_ref/dem build/gb/dem.bin    # 234 tiles, 3.6 s cached
teasi osm --heights=build/gb/dem.bin --country=17 "--name=United Kingdom" \
    osm_ref/great-britain-latest.osm.pbf osm_ref/great-britain.poly - \
    build/gb/GreatBritain_osm.v20260918 20260918       # 2:45, 6.4 GB
```

`way.rs` keeps only the ways that `road_class` or `line_type` accepts and stores the node
columns as flat arrays (otherwise Great Britain does not fit in memory).

**Match** (a build from `denmark-220101` against the original, every 7th D cell): 95 % of the
a1 edges have identical geometry (the first and last point), and of those the class agrees
99.6 %, the name 99.6 %, whether the node id is 0 99.8 %, the flags 92 %, and the length has
a median of 1.000 (p10/p90 0.997/1.001). The graph: 96 % of the node positions are identical,
91 % of the edges are assigned directly, and class/category/passability agree down to a few
hundred cases. Left turns: whether a node has entries agrees 99.9 %; the pairs (compared via
the target positions) 85 %, with the rest being deviations in the neighbouring positions.
Ascent (from the DEM grid, σ = 1): see the calibration above. The build
from `denmark-latest` (2026) has 38 % more edges (3.0 instead of 2.2 M) and is 92 MB
(the original 63 MB); the largest records (B 4.2 MB, D 1.0 MB) stay below those of the German
map (6.5 / 1.3 MB). The cost factor (bits 20–23) and bit 28 stay 0.

## Open questions

1. Where the cost factor (bits 20–23) comes from; bit 28; class 15.
2. Edges with the ids 0/0 outside the motorways (isolated footpaths and farm tracks).
3. The B extra entries; the second table at `0x4fc080` (class → group 0/1/10/11/5/10/6/6/7/8,
   with 90,000 added when the group changes).
4. a3 types 1–3, c1/c4 types.
5. A records: the meaning of the 0x10 sub-elements and of the fields `[4]`/`[5]` of the
   places.
6. The outliers in the length factor (individual edges, around 54° and 59° N and on the
   Faroe Islands), ferries presumably, or edges with a special length.
