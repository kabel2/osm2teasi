# Terrain format (`Denmark_terrain.v20210916`)

The terrain file is the only layer that is **not** encrypted. It consists of JPEG map tiles
and JPEG 2000 elevation tiles. The shell shared by all chart files (header, checksum, tile
directory) and the encryption of the vector layers are described in
**[CHART_FILES.md](CHART_FILES.md)**.

As of 2026-09-18. The tile grid and the zoom pyramid are solved, the georeferencing is open.

## Shell (in brief)

| Offset | Content |
|---|---|
| 0x00 | magic `0x1B62` |
| 0x04 | 48 B salt + 16 B MD5 MAC (integrity/device binding, **not** a key) |
| 0x44 | date, ASCII `20210916` |
| 0x4C | type = **5** (terrain) |
| 0x50 | 32 |
| 0x74 | number of tiles = **23** (x 122–139, y 19–25) |
| 0x78 | directory `[u16 x][u16 y][u32 offset]`, with the first tile right behind it (0x130) |

## Layout of one tile (region)

The layout is the same as in the vector layers. The tile loader `FUN_003e4460` reads in
order:

```
+0x0000  0x40 + 0x100 + 0x40 + 0x1000 B   head areas; all 0xFF in terrain (= 4480 B)
+0x1180  85 × 8 B  cell table (s,d) for 8×8 + 4×4 + 2×2 + 1×1 = 85 cells   (JPEG)
+0x1428  85 × 4 B  second table (JP2/DEM offsets)
+0x157C  32 B      blob (the encrypted record key in the vector layers)
+0x159C  data      the first image always starts here (s_0 = 0x159C)
```

The earlier description "slot array from the start of the region, list from byte 4480" means
exactly this: the 4480 `0xFF` bytes are the head areas, which are empty in terrain.

## Content: 1132 JPEG map tiles (256×256, plain)

`Denmark_terrain.v20210916` (9.08 MB) consists almost entirely of **1132 JPEG images** (SOI
`FFD8FF`, EOI `FFD9`), 1.7–18 KB each, **all 256×256 px**. Every one of them carries the
standard `Exif\0\0` APP1 (TIFF-LE `II\2a\0`, IFD offset 8; the IFD values
96/96/Compression=2 are placeholders, not tile coordinates).

Content (PIL pixel analysis of all 1132):
- **Map palette**: water `RGB(150,180,206)` (651 tiles), land `RGB(225,227,203)` (~425),
  vegetation `RGB(~163,195,148)`.
- 1037 tiles with ≥256 colours → a detailed navigation map (coasts, roads, land cover), not
  photographs.
- Example: tile 366 (0x2b6e3f) = 95 % water, only 19 colours (North Sea/Baltic); the darkest
  tiles (111–119) are dominated by vegetation.

→ The terrain block per directory tile = a sequence of these 256px JPEGs plus the `0xFF`
sparse index blocks in front of them. **Nothing is encrypted**; the high entropy of the
"dense" blocks is simply JPEG data. Ten sample tiles are in `terrain_tiles/`.

**Structure per directory tile** (23 groups, 13–85 sub-tiles, Σ = 1132):
1. **Slot array** (u32, starting **at the region start**): many empty slots at the front
   (`FFFFFFFF`, e.g. 4480 B of pure `0xFF`), then possibly **leading `(-3,0)` markers**,
   then the JPEG list — see below.
2. JPEG sub-tiles (256×256)
3. JP2/DEM sub-tiles (256×256, 16 bit) — see below

Constants (identical in all 23 regions): the **slot array is 5532 B** and the list always
begins at **byte 4480** (= 560 empty 8-byte slots `FFFFFFFF` in front). The dimensions (grid
width 8, levels 16/4/1) appear **nowhere** as a field — the file header only has counters
(`0x74` = 23), the JPEGs only placeholder EXIF (96/96), the JP2 only standard boxes
(`jP␣␣`/`ftyp`/`jp2h`/`jp2c`, ihdr 256×256). The geometry follows from the list and the
`(-3,0)` markers alone.

**The most important constant:** in **every** region the list describes exactly **85 cells**
— 64 (block 1) + 16 + 4 + 1. Parsing: read pairs from byte 4480, `FFFFFFFF` = end, `(-3,0)`
= empty cell, otherwise a tile; **stop as soon as the tile count matches the number of
JPEGs** (the JP2 list follows behind it as single u32 offsets — read as pairs it would
produce phantom tiles).

> Important: the `0xFF` block is **not** a field of its own but the empty start of the slot
> array. The leading markers belong to the list and mean empty cells at the beginning —
> ignoring them shifts every tile. Evidence: (138,25) has 4 leading markers; with them the
> row occupancy comes out as `[4,3,4,4,4,4,5,5]`, which the independent seam correlation
> confirms (without them: `[4,3,4,4,4,5,5,4]` → tile 23 sat in the wrong column 7).

### Index block: cumulative `(s,d)` slot array (decoded)

It sits directly in front of the first SOI and consists of 8-byte slots `(s_i, d_i)` (u32
LE), **cumulative**: `s_{i+1} = s_i + d_i`. `s` = offset **relative to the region start**.

- **Slot types**: `(s,d)` = a record; `0xFFFFFFFD,0x00000000` (−3, 0) = marker/group
  boundary; `0xFFFFFFFF,0xFFFFFFFF` = end of list/empty slot.
- `s_0` is **not** a block size but the offset of the first JPEG (usually `0x159C` = 5532 →
  the constant header/index area in front of it).
- **N+1 offsets** for N sub-tiles: the last entry is the end offset of the last sub-tile.
- **Two lists** per region, separated by the `0xFFFFFFFF` terminator, as a continuation of
  the same cumulative chain:
  1. JPEG tiles (with the `(-3,0)` markers)
  2. **JP2/DEM tiles** (sparse, with `0xFFFFFFFF` empty slots in between).

Verified for all 23 regions: every decoded offset hits an SOI exactly (`okSOI = jpgs`).
Example (122,19), 13 JPEGs: `0x159C (+0x0C5E) → 0x21FA (+0x0FCA) → 0x31C4 …`, in absolute
terms from 0x130 = 0x16CC, 0x232A, 0x32F4 … = exactly the 13 SOIs. The marker groups there:
1, 1, 3, 3, 1, 2, 2, 1.

### Second image layer: 801 JPEG 2000 tiles = the elevation model (DEM)

Besides the 1132 JPEGs, the file contains **801 JP2 codestreams** (signature box
`00 00 00 0C 6A 50 20 20 0D 0A 87 0A` = `jP␣␣`), each **256×256 px, 16 bit greyscale
(`I;16`)** → an **elevation model**, not an image. The field is smooth (inner gradient ≈
678 per column, range 0…65000) and 0 in places (sea?). Per region, for instance (136,22):
83 JPEG + 62 JP2, (134,24): 85 + 64, Σ JP2 = 801. The JP2 list sits in the same cumulative
index (the 2nd list); the chain values point at the start of the record, with the JP2
signature **+8 B** behind it.

### The tile grid of a region (solved, final)

Every region consists of **exactly 85 index cells** — identical in all 23 regions
(verified). The cells are a **mipmap pyramid of the same area**:

| Cells | Level | Grid | Content |
|---|---|---|---|
| 0–63 | **block 1** (finest) | 8 × 8 | map 1:1, DEM 1:1 |
| 64–79 | level 1 | 4 × 4 | the same area at half the resolution |
| 80–83 | level 2 | 2 × 2 | quarter resolution |
| 84 | level 3 | 1 × 1 | overview of the whole region |

Fill rules (the same on every level):

- **row by row, row 0 = at the bottom (south)**, west to east within the row;
- `0xFFFFFFFD, 0x00000000` = an **empty cell** (no tile);
- a tile sits in a cell exactly when the **2×2 block of the level below contains at least one
  tile** (dynamic!). Verified: level 1 **21/23**, level 2 **22/23**, level 3 **22/23**
  regions agree exactly with the index. The only known deviation: (133,24) — a single tile
  there sits isolated below the block, and the level-1 cell (row 2, column 1) stays empty
  nonetheless;
- the sum of the tiles gives **exactly** the number of JPEGs in each region (23/23). Images:
  `terrain_tiles/final_{x}_{y}.jpg` (panels: block 1 | 4×4 | 2×2 | overview, each scaled to
  2048 px).

> **Important (an earlier mistake):** block 1 does **not** end at the last tile but after
> **64 cells** — the holes in the last row belong to block 1. Counting only up to the last
> tile shifts the whole tail. `n_jp2` (the DEM count) is only *mostly* equal to the number
> of tiles in block 1 ((135,22): 18 vs. 16, (137,24): 44 vs. 43).

The pyramid is **not** a separate patch: a template match against block 1 gives tail-fine
vs. block 1/2 = **+0.808** and overview vs. block 1/8 = **+0.792** (NCC). The earlier
assumption of "block 2 = its own area / a 5×5 patch" was an artefact of mixing zoom levels.

### Rotation and tile order within the grid (solved)

- **Both levels are stored rotated 90° CW** → restore with `np.rot90(a,1)` (CCW) or PIL
  `rotate(90)`. Two independent pieces of evidence:
  - water/land seam metric (JPEG): 621 vs. a random baseline of 7238 (~10×)
  - DEM edge correlation: 0.811 (43/61 pairs > 0.9) vs. random 0.02
- **Order** (verified): row by row **from bottom to top (S→N)**, **left to right (W→E)**
  within the row, **8 columns**. Shown per region by a seam scan (the horizontal seam within
  the row plus the vertical seam to the neighbouring row), score `H + max(V)`:
  | Region | best W | Score | Row 0 |
  |---|---|---|---|
  | (136,22) block 1 | 8 | 0.786 | bottom (S) |
  | (134,25) block 1 | 8 | 0.234 (V=0.61) | bottom (S) |
  The row boundaries of (136,22) block 1 lie at **6, 14, 22, 30, 38, 46, 54, 62** (the first
  row has only 6 tiles, the last only 5) — not at multiples of 8! A mosaic using `i//8` is
  therefore wrong (rows from tile 6 on are shifted).
- **Verification by image analysis** (qwen27b):
  - `block1_136_22.jpg` (8×8, row 0 at the bottom): **COHERENT** — coasts, lakes and relief
    run across the tile borders; only the 2 missing tiles (bottom right) are grey.
  - `reg_134_25.jpg` (8×8): **COHERENT**.
  - Images in `terrain_tiles/`.
- The JP2 tiles are **affinely normalised per tile** (their own scale/offset) → compare
  seams only by **correlation**, never by absolute difference.

Why the first mosaics looked "shredded": the wrong rotation *and* rows from the top instead
of the bottom *and* 8×12 instead of 8 columns *and* mixed levels. The stylistic breaks
qwen27b noticed (hillshade vs. straightened road network, area fills) are changes of
layer/LoD or no-data cells.

### Recipe: assembling a region without comparing edges

The layout is **fully described by the index** — comparing edges now only serves as
verification (rotation, plausibility). The steps:

1. **Directory** from `0x78`: records `(x:u16, y:u16, off:u32)`; the end of a region = the
   `off` of the next record (the last region = the end of the file).
2. **Read the slot array from the region start** as 8-byte slots `(s,d)`, cumulatively
   (`s_{i+1} = s_i + d_i`), with `s` relative to the region start. Skip the leading
   `FFFFFFFF` slots (the empty beginning of the array); the list begins at the first slot ≠
   `FFFFFFFF` — **count the leading markers** (see above):
   - `(s,d)` → a **tile** at `region_off + s`
   - `(-3, 0)` = `0xFFFFFFFD, 0x00000000` → a **skipped cell** (a gap in the grid, no tile)
   - `0xFFFFFFFF, 0xFFFFFFFF` → the end of the list
   - the **last** tile slot is only the end offset (N+1) → discard it.
3. **The row width** follows from the markers: the tiles before the first marker plus the
   number of markers = one full row. (136,22): 6 tiles + 2 markers = **8**; regions with a
   full grid ((134,24), (134,25): 64 tiles) have **no** markers at all.
4. **Rotation:** every tile is stored rotated 90° CW → `rotate(90)` (PIL, = CCW) or
   `np.rot90(a, 1)`.
5. **Block 1** = the tiles in the **first 64 cells** (an 8 × 8 grid). Placement: **8
   columns**, row by row (W→E within the row), **row 0 = at the bottom (south)**; leave one
   cell empty at every `(-3,0)` marker. Missing cells stay grey. (`n_jp2`, the DEM count, is
   only *mostly* identical with the number of tiles in block 1 — do not use it as the
   boundary!)
6. **Tail** = cells 64–84 → the pyramid: 64–79 → 4×4, 80–83 → 2×2, 84 → overview; each row
   by row, row 0 at the bottom. The tiles keep being counted **consecutively** (block 1
   occupies `jpeg[0..k-1]`, the tail `jpeg[k..]`).
7. **Verification** (optional): the normalised cross-correlation of the **shared** edges —
   horizontally `right(A) ↔ left(B)`, vertically `top(bottom) ↔ bottom(top)`; as an
   affine-invariant feature the water mask `(B − R) > 20`. DEM edges are considerably clearer
   (mean 0.811) than JPEG edges (mean 0.023).

**Pitfalls** (these were the cause of the "shredded" mosaics):

- `i // 8` is wrong as soon as a row has gaps — the row boundaries of (136,22) lie at 6, 14,
  22, … because the **first** row has only 6 tiles.
- Do not mix zoom levels or blocks (block 1 is a different area from the block-2 pyramid).
- Flat tiles (open water, inland) yield no correlation (std ≈ 0 → sentinel) — do not read
  anything into seam tests there.
- DEM tiles are affinely normalised per tile → correlation only, never absolute difference.

**Georeferencing — update 2026-09-18:** the vector layers confirm the equirectangular
1.40625° grid (`lon = x·1.40625 − 180`, `lat = 90 − y·1.40625`): lighthouses in osmpoint come
out accurate to ~10 m with it, and tile (122,19) contains the Faroe Islands. See
[CHART_FILES.md](CHART_FILES.md), section 1a. Anchor 2 (Oslo) was therefore wrong, and the
"discarded hypothesis" below is the right one. How exactly the 8×8 terrain tiles sit within
the tile still has to be checked, though.

**Georeferencing (historical, as it stood before the update):**

- **Anchor 1 (user):** tile 684 (global #684, 1-based = the 69th tile of region (136,22) =
  **index 68**, 0-based) ≈ **58.1904365 N, 11.6903675 E**. Index 68 lies in **block 2**, not
  in the 8×8 grid.
- **Anchor 2 (user, visual):** `reg_134_25_preview.jpg` (block 1 of region (134,25),
  correctly assembled) shows **Norway and part of Sweden, with Oslo (59.91 N, 10.75 E) in the
  middle**.
- **Discarded hypothesis:** the directory coordinates as an equirectangular grid with
  1.40625° cells (`lon = x·1.40625 − 180`, `lat = 90 − y·1.40625`). It explains anchor 1
  ((136,22) = 11.25–12.66 E, 57.66–59.06 N = "Sweden above Gothenburg", confirmed by the
  user) but **not** anchor 2: (134,25) would lie at 8.44–9.84 E / 53.44–54.84 N (the German
  Bight) instead of at Oslo. A joint linear fit (cell size, origin) from both anchors gives
  no plausible values (Δx=2 → Δlon≈1.25°, Δy=−3 → Δlat≈−1.4° ⇒ ~0.63° per step in longitude,
  ~0.47° per step in latitude).
- **A fixed anchor from the file (new!):** every JP2/DEM tile carries a `res `/`resc` box
  (capture resolution). The value is identical in **all 801** tiles:
  `18576 / 65532 × 10^4` = **2834.646 px per unit** (raw `48 90 ff fc 48 90 ff fc 04 04`).
  Reading the unit as degrees gives **1 tile = 256 / 2834.646 = 0.090311° = 5.4187′ ≈ 10 km**
  and hence 8×8 = **0.7225° per region**. (Read as px/m it would give 0.35 mm per pixel,
  which is implausible, hence the degree assumption; in the JP2 standard the unit is "pixels
  per meter", so treat the value with care.)
- **Next approach:** render an overview of all 23 regions at their directory positions (each
  as a scaled-down block-1 mosaic) and have it identified geographically → fit the
  coordinates from landmarks.

## Elevation model: placement and values (solved 2026-09-19)

Region (135,24) (Funen) was checked against the Copernicus DEM GLO-90; the scripts are in
the session's scratchpad.

- **Level 0 only:** all 801 JP2 sit in cells 0–63 of the second table (`+0x1428`). The
  pyramid exists only for the JPEGs.
- **Placement (different from the JPEGs!):** cell k lies in column `k // 8` from the west and
  row `k % 8` from the **north**, so column by column from the north-west southwards. The
  tile is **not rotated** (pixel row 0 = north). The mean correlation with Copernicus is
  0.975; every other variant is below 0.14.
- **Record:** `[u16 a0][u16 a1][u32 length]`, then the JP2 file (`jP  `, `ftyp`, `jp2h` with
  `ihdr` 256×256×1 16 bit, `colr`, `res `, then `jp2c`). The codestream was produced with
  JasPer 1.701: one tile, 5 decomposition levels, 9/7 irreversible, code blocks 64×64,
  lossy. A record is 1113–4201 B long, and the maximum sits in the header at 0x6C (4201).
- **Height in m:** `h = (v / a0 + a1) / 6 − 1000`, where `a1` is the base in 1/6 m with a
  1000 m offset and `a0` the number of values per 1/6 m. A pooled free fit gives
  `(v/a0 + a1 − 5990) / 5.94`. With the round constants the formula is off by 2.1 m in the
  median, and 1.7 m higher than Copernicus, which is what a surface model (trees, houses)
  should do.
- The firmware knows a `HeightMapCacheMemory` and has a JP2 decoder of its own ("Failed to
  decode jp2 structure").
- Header 0x54 = 4 (country code DK), as in the vector layers. 0x6C is the largest JP2 length
  `n` (4201).
- **End of a record:** 2 more bytes follow the JP2 (which ends with `FFD9`). Usually they are
  the `a0` of the next record, in 117 of 779 cases a different value. These are apparently
  buffer leftovers; the next record begins behind them.
- A region with no tiles at all ((122,20)) has nothing but the head (5532 B). The JPEG table
  may therefore be completely empty (`(-3,0)` in all 85 cells).

**Map images, placement (2026-09-19, checked against the elevation model):** the JPEGs sit
exactly like the elevation tiles: cell k in column `k // 8` from the west, row `k % 8` from
the north, **not rotated**. That holds for the pyramid too: 4×4 at 64 + `c*4 + r`, 2×2 at
80 + `c*2 + r`, then 84. Checked by correlation against the scaled-down 8×8 mosaic (0.85–0.91
column-wise, about 0 row-wise). The older reading above ("rotated 90° CW, row 0 = south")
describes the same mosaic, only rotated as a whole; it came about without a georeference. A
coarser cell has an image when one of its four children has one. In Denmark 482 of 483 cells
agree, in Norway 12,736 of 12,789. The JPEGs are baseline with JFIF and Exif (96 dpi), 4:2:0
and IJG quality 80. The records of the JPEGs come in cell order, followed by the elevation
records.

**Compiler** `tools/compile_terrain.py` (2026-09-19):
- Without `--land`/`--area` every JPEG cell stays empty; the elevation profile already works
  like that.
- With `--land`/`--area` a 2048×2048 image is produced per region:
  - The sea comes from the land polygons.
  - Water, forest, heath, rock, sand, wetland, built-up land and farmland come from the OSM
    areas.
  - On top of that comes a hillshade from the DEM: light from the north west, 45°, relief
    exaggeration 1.5, factor 0.4–1.12.
  - The image is cut into 256-pixel tiles, and the pyramid levels are added (Lanczos).
- Images go to every cell that touches the polygon or contains land.
- Elevation tiles exist only for cells with land (height > 0).
- The heights are read bilinearly from the `dem_heights.py` grid. `a1` is the minimum and
  `a0 = 65535 // span` in 1/6 m.
- Encoding is done with OpenJPEG (Pillow): `irreversible`, 6 resolutions, 64×64, LRCP,
  ratio 50. COD and SIZ are identical with the original, and the error is around 0.5 m RMS.
  The JP2 boxes in front are copied from the original.
- The blob is `encrypt_blob` of a random key, as in the vector layers.

### In Rust

`rust/src/terrain.rs` does the same without a pickle; `rust/src/raster.rs` reproduces the
three Pillow pieces (the polygon filler, the Lanczos resize and the hillshade — all three
bit-identical) and calls OpenJPEG for the elevation tiles. See
[../rust/README.md](../rust/README.md).

```bash
./target/release/teasi terrain --country=17 \
    --land=osm_ref/land-polygons-split-4326/land_polygons.shp \
    --area=osm_ref/great-britain-latest.osm.pbf \
    build/gb/dem.bin osm_ref/great-britain.poly \
    build/gb/GreatBritain_terrain.v20260919 20260919      # 1:24, 7.9 GB
```

The comparison is done with `rust/scripts/terrain_compare.py`, not with `teasi check` — the
layer has no slot areas. For Denmark as for Great Britain **every elevation tile** (1034 and
1999 respectively) is byte-identical with the Python version, down to one byte: OpenJPEG
writes its own version into the codestream's comment marker. The map images go through a
different JPEG encoder (`jpeg-encoder` instead of libjpeg-turbo); about half of them are
pixel-identical anyway, the mean deviation is 0.025 out of 255, and the files are three
hundred-thousandths smaller.

## Surrounding files (context)

- `acsldata.dat` (67 B): `<32-character key>` \n `<email address>` \n\n `193088` \n —
  licence/access data (key, email, id). The real values are redacted here; they are on the
  device.
- `lastdevice.dat`: `2013021200000368` (the device id = the name of the data folder).
- `settings/customroutecache*.rti` (30 files, 28–52 KB): written by the same engine, **in
  plain text**:
  - magic `B18EDA7A` (LE `7ada8eb1`), then a u32 ≈ 6907 (0x1B5B — the same "header size"
    idea as the chart headers!),
  - UTF-16 strings ("Koldinghus 0 ist ein anderes Königsschloss in der Stadt K…"), and u32
    pairs that look like offsets into a larger database (0x1E7B1F44, 0x0860xxxx).
  → The engine's cache is not encrypted (unlike the chart records: PC1 + LZMA).
- `Tahuna.rar` (112 MB, RAR5): nothing but `TrackImages/Tourbook_*.tourbook.png`.
- `monitor.sqlite`: the tables (Maps, StorageDevices, …) are **all empty**.

## Open questions

1. **Georeferencing**: the tile grid is settled (see the update above). What is open is how
   the tiles sit within a tile.
2. The level-1 rule at (133,24) (one cell deviates).
3. ~~DEM: range of values/unit~~ solved, see "Elevation model: placement and values".
4. Header fields 0x50–0x70 (in terrain `0x6C = 4201`, among others).
5. Document `settings/customroutecache*.rti` (magic `B18EDA7A`).

Scripts for rendering: `attic/terrain_render/` (`render_final.py`, `region_full.py`,
`tail_render.py`, `overview_fix.py`; output in `terrain_tiles/`).
