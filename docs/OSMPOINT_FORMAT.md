# osmpoint format (`Denmark_osmpoint.v20210916`)

As of 2026-09-18. **Fully understood**: the parser and builder in `tools/layers.py` produce
all 110 records **byte-identically** (round-trip test).

Content: **seamarks from OpenSeaMap** (`seamark:*`) — buoys, beacons, lights with sectors,
harbours, wind turbines, rocks, wrecks and so on. The Danish file: 5211 objects in 110
records, the Faroe Islands among them.

Shell, encryption, slot areas and the coordinate system are described in
[CHART_FILES.md](CHART_FILES.md) (sections 1, 1a and 3).

## Where it sits

| | |
|---|---|
| Header `0x50` | `1024` (layer id) |
| Slot area | **C** (tile head `+0x140`, 4×4 sub-cells) |
| Reader in the firmware | `FUN_003e52d0` (internal type 5) |
| 1 record | = every object of **one 4×4 sub-cell** (0.3515625° × 0.3515625°) |

## Layout of the unpacked record

```
0x00  u32  n·0x30     size of the struct array (the firmware overwrites it with cell_x)
0x04  u32  cell_y     y of the sub-cell (overwritten with cell_y)
0x08  u32  0
0x0C  u32  n          number of objects
0x10  u32  0          (firmware: pointer to the array)
0x14  n × 0x30 B      object structs, TRANSPOSED BYTEWISE
      strings         per object: name, attrs, label, then the sector entries (16 B each)
      sector labels   for every object, only after the whole string block
```

There is no padding at the end: the record size (`pltx`) comes out exactly.

**Transposed** means byte 0 of all n structs comes first, then byte 1 of all structs, and
so on. The firmware undoes this after decompressing
(`tools/layers.py: untranspose/transpose`).

### Object struct (0x30 B = 12 × u32, after transposing back)

| Field | Content |
|---|---|
| `[0]` | 0 (firmware: pointer to `name`) |
| `[1]` | length of `name` in UTF-16 characters **including** the NUL, 0 = no string |
| `[2]` | category (symbol), see below |
| `[3]` | **position**: `(v << 16) \| u`, see Coordinates |
| `[4]` | colour pattern of the body (bit field, not fully decoded yet) |
| `[5]` | topmark (shape + colour, not fully decoded yet) |
| `[6]` | 0 (pointer to `attrs`) |
| `[7]` | length of `attrs` including the NUL |
| `[8]` | 0 (pointer to `label`) |
| `[9]` | length of `label` including the NUL |
| `[10]` | number of sectors |
| `[11]` | 0 (pointer to the sector array) |

An empty string has length 0 and takes up no space, so it has no NUL either.

### Sector entry (16 B, **not** transposed)

```
u16 colour    colour code as in [4]: 1 = white, 3 = red, 4 = green, 8 = yellow, 9 = amber
              (12 appears 18 times, origin unclear)
u16 from      start of the sector in 1/4096 of a circle, rotated by 180° (the direction
              away from the light): int((sector_start + 180) mod 360 · 4096/360),
              e.g. 152.5° → 3783
u16 to        end of the sector, likewise
u16 radius    370 for real sectors, 0 for all-round lights
u32 0         (pointer to the label)
u32 len       length of the sector label including the NUL (e.g. "Fl.G.3s")
```

## Coordinates

```
u = pos & 0xFFFF      offset east from the western edge of the sub-cell
v = pos >> 16         offset south from the northern edge of the sub-cell
unit: 360° / 2^25     (32768 units = one sub-cell = 0.3515625°), range 0..32768

lat = 90  − (cell_y · 32768 + v) · 360/2^25
lon = −180 + (cell_x · 32768 + u) · 360/2^25
```

`cell_x = tile_x·4 + slot//4`, `cell_y = tile_y·4 + slot%4`.

Checked against lighthouses with a known position (deviation ≤ 20 m):

| Object | from the file | Expected |
|---|---|---|
| Hanstholm | 57.11271 N, 8.59856 E | 57.1128 N, 8.5985 E |
| Hirtshals | 57.58474 N, 9.94191 E | 57.5847 N, 9.9419 E |
| Stevns | 55.29067 N, 12.45359 E | 55.2907 N, 12.4536 E |
| Leynar (Faroe Islands) | 62.110 N, 7.041 W | 62.11 N, 7.04 W |

## Strings

Every string is UTF-16LE.

**`name`**: `seamark:name` or `name` ("Hanstholm", "No. 14", …).

**`attrs`**: an attribute list `NNvalue|NNvalue|…` with a two-digit code before each value.
The codes recognised so far:

| Code | Meaning | Example |
|---|---|---|
| `01` | website | `01http://www.hanstholmhavn.dk/…` |
| `02` | phone | `02+45 96 550710` |
| `11` | description/note | `11Opført 1884` |
| `17` | email | |
| `18` | operator | `18DS, Lemvig Sejlklub` |
| `19` | **kind of object (plain text)** | `19Port-hand Lateral Buoy`, `19Light minor` |
| `20` | information | `20Lighted by day when visibility is 3M or less.` |
| `21` | radar transponder | `21Racon(T)` |
| `23` | fog signal | `23Horn` |
| `24` | character of the light | `24Fl.W.20s65m26M` |
| `25` | light list number | `25B 2084` |
| `26` | buoyage system | `26iala-a` |
| `36` | height in m | `3665` |
| `45` | water level (S-57 WATLEV) | `45submerged`, `45awash` |
| `35`, `37`, `04`, `06`, `10`, `16`, `27`, `38`, … | still open | |

Harbours have a trailing `\n` on `19` (`19Yacht harbour/marina\n`); that is how the original
has it too.

**`label`**: short text for the map display, also with a prefix: `5` = character of the
light (`5Fl.W.20s65m26M`), `0` = fog or radar signal (`0Horn`, `0Racon(T)`), `7` = signal
station, `4` = clearance height. Several entries are separated by `|`.

## Category `[2]`

`[2] = group << 16 | subkind << 8 | class`, fully resolved by matching against OSM (99.6 %
hit rate, rules in `category()` of `tools/compile_osmpoint.py`):

- **Class** (byte 0) from `seamark:type`: buoy 0, beacon 1, light 3, landmark 4, harbour 6,
  anchorage 7, mooring 8, wreck 9, rock 0xC, bridge 0xD, radio station 0xE, signal station
  0xF, platform 0x10, wind farm 0x11.
- **Group** (byte 2): lateral/cardinal 0x0A, special purpose/isolated danger 0x0C, safe
  water 0x09, lights and harbours 0x09, landmarks 0x0A, anchorage/bridge 0x0E,
  mooring/rock/wreck 0x0F, platform/wind farm 0x0B, radio/signal 0x0C.
- **Subkind** (byte 1): buoy shape (`conical` 0, `can` 1, `spherical` 2, `pillar` 3 and the
  default, `spar` 4, `barrel` 5, `super-buoy` 6); beacon shape (`stake/pole/post` 0, `tower`
  2, `pile/lattice` 3 and the default); light major 0 / minor 1; landmark = S-57 CATLMK − 1
  (`chimney` 2, `mast` 6, `tower` 16, `windmotor` 18 …); harbour (`fishing` 0, `marina` 1,
  `marina_no_facilities` 2, otherwise 3); mooring (`dolphin` 0, `bollard` 2, `wall` 3,
  `post/pile` 4, `buoy` 5); rock by `water_level` (`covers` 0, `awash` 1, otherwise 2);
  wreck (`non-dangerous` 0, `dangerous`/empty 1, `hull_showing` 2).
- A landmark without a category counts as a major light (0x090003).

The most common values in Denmark:

| `[2]` | Objects | Count |
|---|---|---:|
| `0x0A0400` | lateral and cardinal buoys (buoy shape 4, spar presumably) | 977 |
| `0x090103` | lights (minor), leading lights included | 899 |
| `0x0A1204` | wind turbine | 564 |
| `0x0A0300` | lateral and cardinal buoys (buoy shape 3) | 346 |
| `0x0A0100` | port-hand buoy (shape 1, can presumably) | 267 |
| `0x0F0508` | mooring buoy | 263 |
| `0x0F020C` | rock (submerged) | 257 |
| `0x090106` | yacht harbour | 247 |
| `0x0A0000` | starboard-hand buoy (shape 0, conical presumably) | 217 |
| `0x0C0400` | special purpose (buoy) | 165 |
| `0x0C0301` | special purpose (beacon), isolated danger mark | 108 |
| `0x0F0008` | dolphins | 101 |
| `0x0A0301` | lateral beacon | 82 |
| `0x090003` | light (major) | 42 |
| `0x0A1004` | lighthouse | 26 |
| `0x0F0109` / `0x0F0009` / `0x0F0209` | wreck (dangerous / non-dangerous / hull showing) | 15 / 11 / 4 |
| `0x0E0007` | anchorage | 17 |
| `0x0E000D` | fixed bridge | 1 |

`tools/layers.py` together with `collections.Counter` over `o["cat"]` gives the full list
(55 values) with examples.

## Colours `[4]` and topmarks `[5]`

Resolved by matching against OSM (98.4 % / 99.4 % hit rate):

```
colour codes: grey 0, white 1, black 2, red 3, green 4, blue 5, yellow 8

[4] (buoys and beacons only, from seamark:<type>:colour / :colour_pattern):
    bits 0-3  pattern (vertical = 4, otherwise 0)
    bits 4-6  number of colours
    from bit 7  the first two colours, 4 bits each (with 3 bands the third is missing)
[5] (from seamark:topmark:shape / :colour):
    bits 0-6  shape: cone point up 1, cone point down 2, sphere 3, 2 spheres 4,
              cylinder 5, board 6, x-shape 7, upright cross 8, 2 cones point together 10,
              2 cones base together 11, rhombus 12, 2 cones up 13, 2 cones down 14,
              square 17, triangle point up 18, triangle point down 19
    bits 7-8  number of colours
    from bit 9  the first two colours, 4 bits each
```

Examples:

| Object | `[4]` | `[5]` |
|---|---|---|
| port hand (red) | `0x190` | `0x685` (cylinder) |
| starboard hand (green) | `0x210` | `0x881` (cone) |
| special purpose (yellow) | `0x410` | `0x1087` (cross) |
| north cardinal (black over yellow) | `0x4120` | `0x48D` |
| south cardinal (yellow over black) | `0x1420` | `0x48E` |
| east cardinal (black-yellow-black) | `0x4130` | `0x48B` |
| west cardinal (yellow-black-yellow) | `0x1430` | `0x48A` |
| safe water buoy (red-white vertical) | `0x9A4` | `0x683` (sphere) |
| no colour/topmark | `0` | `0` |


## Rebuilding it

```python
import sys; sys.path.insert(0, "tools")
from layers import iter_records, parse_osmpoint, build_osmpoint, to_latlon

for tx, ty, cx, cy, grid, raw in iter_records(open(PATH, "rb").read(), "C"):
    objs = parse_osmpoint(raw)                  # list of dicts
    assert build_osmpoint(objs, cy) == raw      # byte-identical
```

The order of the objects in the original file follows no recognisable sorting (the OSM
order, presumably).

## Building it from OSM

```bash
.venv/bin/python tools/osm_poi_extract.py osm_ref/denmark-latest.osm.pbf build/ref/poi_latest.pkl   # as for osmpoi
.venv/bin/python tools/compile_osmpoint.py build/ref/poi_latest.pkl osm_ref/denmark.poly \
    build/<dir>/Denmark_osmpoint.v20210916 [YYYYMMDD]
```

In Rust, `teasi osmpoint <pbf> <poly> <out> [YYYYMMDD] --country=N` does both in one run
without a pickle; for Denmark all 128 records are byte-identical, for Great Britain 353 of
354 (`rust/README.md`, stage 3).

Every object with a `seamark:type` that has a category is taken (not `small_craft_facility`,
`pile`, `cable_submarine` or `navigation_line`, for instance), inside the Geofabrik
boundary. Position: nodes directly, areas via their centroid, rounded to 360°/2²⁵.

**Calibration** (4691 original objects with an exactly matching OSM node in
`denmark-220101`):

| Field | Hit rate |
|---|---:|
| category `[2]` | 99.6 % |
| colour `[4]` / topmark `[5]` | 98.4 % / 99.4 % |
| name (`seamark:name`, not `name`) | 99.9 % |
| attributes | 94 % |
| label | 97 % |
| sectors | 97 % |

Rules for the texts (`description()`, `light_string()`, `attributes()`):

- **`19`**: an English description from the type and the category, e.g. `Port-hand Lateral
  Buoy`, `North Cardinal Buoy`, `Diving mark Special Purpose Buoy`, `Front light minor`,
  `Lower,leading light minor` (light categories with a comma), `Yacht harbour/marina\n`
  (harbour categories with `\n`, several as a list `- …\n`), `Windmotor`, `Rock`,
  `Mooring buoy`.
- **`24` / label `5`**: `character.colours.period s height m range M(category)`. Colours as
  letters in the order W, R, G, Y; the height only for a single light without a sector; the
  range truncated, and `min-max` for several lights; the category of the last light
  capitalised. Examples: `Fl.G.5s6m4M`, `Oc.WRG.12s9-12M`, `F.R(Air_obstruction)`, `.R`
  (without a character).
- **`36`** = `light:height` (or `light:1:height`), whole numbers without `.0`. **`35`** =
  `landmark:height`, **`37`** = `light:exhibition`, **`45`** = `water_level` (a rock with no
  value: `submerged`), **`20`** = `seamark:information`, **`21`** = `Racon(<group>)`,
  **`23`** = `Horn`, `Siren(2)60s`, **`25`** = `light:reference`, **`26`** =
  `<type>:system`, **`27`** = `port_of_entry`, plus `01`, `02`, `04`, `06`, `11`, `16`,
  `17`, `18` as in osmpoi.
- **Label**: `0` + racon/fog signal, `7SS` for warning signal stations,
  `(Bridge Passage)` for traffic signal stations, `4` + clearance height, `5` + character.
- **Sectors**: one entry per light, sorted by colour code. Without `sector_start`/
  `sector_end`, `from = to = 0`, `radius = 0` and the label is empty; otherwise
  `radius = 370` and the label is `character.colour.period s`.

Current state (2026-09-18): 9921 seamarks (the original has 5211). There are more because
OpenSeaMap has grown (1465 instead of 564 wind turbines, among other things), but also
because the Geofabrik boundary takes in neighbouring waters: the original contains no
seamarks east of 13° E off Rügen/Skåne and hardly any in the Bay of Kiel. The Faroe Islands
are missing.

## Open questions

- Special cases of the light character (alternating lights `Al.M`, the colour order `RWG`
  on some sector lights), sector colour code 12.
- `[4]` variants with bits 0–1 set (`0x191`, `0x9A5`, `0x2` on lights).
- Which boundary the original uses (no neighbouring waters).
