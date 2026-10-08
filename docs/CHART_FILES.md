# Teasi chart files: encryption and tools

As of 2026-09-18. Applies to the chart files `<Country>_<layer>.vYYYYMMDD` (magic `0x1B62`)
of the Teasi PRO (`bikenav.exe` 4.4.1.0, WinCE/ARM), for example
`2013021200000368/7/943/20317/Denmark_osm.v20210916`.

> In brief: the maps are **not** protected by a proprietary codec. They are three standard
> building blocks: an **MD5 checksum** in the header, **PC1 encryption** (Pukall Cipher 1,
> 256 bit) with partial encryption of the payload, and **LZMA** compression. All 14,185
> records of the vector layers (osm, osmarea, osmpoi, osmpoint, ta) can be decrypted and
> decompressed with them.
>
> The other direction works too: `writer.rs` builds valid files, and a freshly built
> test package runs on the device (2026-09-18). To avoid a warning, size and MD5 in
> `BikeNav/packages.xml` have to be updated (section 5.4).

An overview of the repository and of the route from OSM to a map: [../README.md](../README.md).

---

## 1. File layout (the outer shell)

```
0x00  u32     magic 0x00001B62
0x04  48 B    salt (random)                   ┐ header checksum,
0x34  16 B    MAC = MD5(...)                  ┘ see section 2
0x44  8 B     date in ASCII, e.g. "20210916"
0x4C  u32     type (1 = osm/osmarea/osmpoi/osmpoint, 2 = ta, 5 = terrain)
0x50  u32     layer bit mask (osm 1, osmarea 4, osmpoi 8, ta 0x12, terrain 0x20, osmpoint 0x400)
0x54  u32     country (DK 4, DE 7, NO 12, UK 17), checked against an unlock table (index < 0x153)
0x58  u32     buffer size: the largest compressed record length (len)
0x5C  u32     largest uncompressed size (pltx) in slot area D
0x60  u32     likewise slot area C
0x64  u32     likewise slot area B
0x68  u32     likewise slot area A
0x6C  u32     0 (a buffer size, unused in the vector layers)
0x70  u32     offset of an extra table, only with layer bit 0x10 (ta), otherwise 0xFFFFFFFF
0x74  u32     number of tiles N
0x78  N × 8 B tile directory: [u16 x][u16 y][u32 file offset], ascending
```

That is how the chart loader `FUN_003f282c` reads the header. The **country code** is the
index into the firmware's country list (UTF-16 strings in `bikenav.exe`: Andorra 1, Austria
2, Belgium 3, Denmark 4, Finland 5, France 6, Germany 7, Ireland 8, Italy 9, Luxembourg 10,
Netherlands 11, Norway 12, Portugal 13, San Marino 14, Spain 15, Sweden 16, United Kingdom
17, …); DE 7 and NO 12 confirm it. Only countries with their byte set in the unlock table
(`*DAT_003f3f64 + 0x324 + code`) are loaded; UK (17) runs on this device (2026-09-18). The
fields `0x58`–`0x6C` are buffer sizes: per layer the firmware takes the maximum over every
loaded file. Larger values than needed are harmless (osmpoi and osmpoint have more at `0x58`
than their largest record), smaller ones must not occur. The **date** decides which file
counts: if a file with the same or a newer date is already loaded for that layer and country,
the new one is skipped.

A tile ends where the next one begins (the last one at the end of the file).

### Inside a tile (offsets relative to the start of the tile)

```
+0x0000  16 × u32   slot area A (4×4 sub-cells)      ┐ record offsets relative to the
+0x0040  64 × u32   slot area B (8×8)                │ tile start, 0xFFFFFFFF = empty,
+0x0140  16 × u32   slot area C (4×4)                │ see section 1a
+0x0180  1024 × u32 slot area D (32×32)              ┘
+0x1180  85 × 8 B   cell table of the zoom pyramid 8×8 + 4×4 + 2×2 + 1×1
+0x1428  85 × 4 B   second cell table (u32); both are only filled in terrain and empty in
                    the vector layers: (0xFFFFFFFF, 0) and 0xFFFFFFFF respectively
+0x157C  32 B   blob = the tile's encrypted record key
+0x159C  records as an unbroken chain up to the end of the tile
```

### Record

```
[u32 len][u32 pltx][payload: len bytes]
```

- `len`: the length of the encrypted, compressed payload
- `pltx`: the size **after** decompressing (always even)
- The next record: `offset + 8 + len`. `chart.rs` follows this chain, whereas the
  firmware jumps straight to a record through the slots (section 1a). Both give the same set:
  every record sits in exactly one slot.

### The Danish files at a glance

| File | Size | Type (0x4C) | 0x50 | Tiles | Records |
|---|---:|---:|---:|---:|---:|
| `Denmark_osm.v20210916` | 62,976,522 | 1 | 1 | 22 | 5878 (406 of them unencrypted) |
| `Denmark_osmarea.v20210810` | 19,916,110 | 1 | 4 | 24 | 368 |
| `Denmark_osmpoi.v20210915` | 1,621,671 | 1 | 8 | 17 | 3704 |
| `Denmark_osmpoint.v20210916` | 206,699 | 1 | 1024 | 17 | 110 |
| `Denmark_ta.v20180608` | 22,242,944 | 2 | 18 | 14 | 4125 |
| `Denmark_terrain.v20210916` | 9,081,123 | 5 | 32 | 23 | – (images) |

Details that were checked:

- `len` does **not** count the 8 header bytes. For all consecutive records
  `start[i+1] = start[i] + 8 + len[i]` holds, checked across every layer.
- The first directory entry of `osmarea` is a **placeholder** `(0,0)` of size 0; the 23 real
  tiles follow it.
- Tiles with the same blob form spatially contiguous groups. Because PC1 restarts for every
  record, all the records of a group begin with the same ciphertext byte. That had earlier
  been read as a "type byte".
- Header `0x58`–`0x70`: see above. In osm, osmarea and ta the pltx fields match the largest
  records exactly, and in osm and osmarea `0x58` is precisely the largest record length.
- In `ta` the record chain ends in tile (138,24) at `+0x361FC`. That is where the **extra
  table** begins (header `0x70`), reaching to the end of the tile (2,596,612 B). It is the
  country-wide **search index** of the address search (a prefix tree over the place and
  postcode names, see [TA_FORMAT.md](TA_FORMAT.md)). It is not a record, but its first u32
  looks like a record length. `teasi dump` therefore runs into it and reports a "failed" entry;
  the slots are what counts.
- The DE and NO folders (`20322`, `20339`) contain only 0-byte files.

### 1a. Slot areas, kinds of record and the coordinate system

**Grid.** The world is divided equirectangularly into 256 × 128 tiles of **1.40625°** each
(`lon = x·1.40625 − 180`, `lat = 90 − y·1.40625`, with y counting from north to south). Every
tile is divided into sub-cells again, and which division applies depends on the slot area. A
record always holds the data of **exactly one sub-cell**. The slot of a sub-cell is found
like this:

```
slot = (cell_x mod g) · g + (cell_y mod g)       g = 4 (A, C), 8 (B), 32 (D)
cell_x = tile_x · g + slot // g,  cell_y = tile_y · g + slot % g
```

| Area | Tile head | Grid | Reader (firmware) | Used by |
|---|---|---|---|---|
| A | `+0x000` | 4×4 | `FUN_003e6044` | osm (161), ta (101) |
| B | `+0x040` | 8×8 | `FUN_003e5d20` | osm (406, **unencrypted**) |
| C | `+0x140` | 4×4 | `FUN_003e52d0` (osmpoint) / `FUN_003e651c` (osm, osmarea) | osm (105), osmarea (368), osmpoint (110) |
| D | `+0x180` | 32×32 | `FUN_003e56fc` | osm (5206), osmpoi (3704), ta (4024) |

The readers check `cell_x < 0x400, cell_y < 0x200` (4×4) or `< 0x2000, < 0x1000` (32×32).

**Coordinates in the records** are relative to the north-west corner of the sub-cell:
`u32 = (v << 16) | u`, with u going east and v going south. **A sub-cell is always 32768
units** wide, so the unit is 360°/2²⁵ (≈ 1.2 m) for 4×4 cells and 360°/2²⁸ (≈ 15 cm) for
32×32 cells. Rounding goes to the nearest unit, and the points are then **exactly OSM nodes**
(matched against the Geofabrik extract `denmark-220101`, `teasi ways`).

| Layer / area | Margin | Checked |
|---|---|---|
| osmpoi (D), osmpoint (C), osmarea (C) | none | exactly on OSM nodes |
| osm D, osm C, ta D (lines) | **512 units**: stored as `u = x + 512`, range 0 … 33,792 | osm exactly on OSM nodes, ta only through the range of values |
| osm B (routing graph, 8×8) | none, but **65,536** units per cell (360°/2²⁷) | ±1–2 units |

`layers.to_latlon(cx, cy, g, p, margin=layers.MARGIN)` and `layers.b_node_latlon`. The grid
also fits the terrain hypothesis with 1.40625° (tile (122,19) contains the Faroe Islands).

**Features shared by the contents of every record:**

- A head of u32 fields comes first. The firmware overwrites `[0]` and `[1]` with
  `cell_x`/`cell_y` while loading, and `[3]` is always the count of the first array. Pointer
  fields are 0 in the file.
- The struct arrays are usually **transposed bytewise**, and strings are UTF-16LE with their
  length given in characters including the NUL.
- The formats of the individual layers:
  [OSMPOINT_FORMAT.md](OSMPOINT_FORMAT.md) · [OSMPOI_FORMAT.md](OSMPOI_FORMAT.md) (with the
  general D record) · [OSM_FORMAT.md](OSM_FORMAT.md) (with the geometry and the A and B
  records) · [OSMAREA_FORMAT.md](OSMAREA_FORMAT.md) (with the general C record) ·
  [TA_FORMAT.md](TA_FORMAT.md) · [TERRAIN_FORMAT.md](TERRAIN_FORMAT.md).
- **All 14,185 records** of every vector layer can be parsed with `layers.rs` and built
  again **byte-identically** (`teasi check`).

The terrain file (type 5) is laid out differently and is **unencrypted** (JPEG tiles and a
JPEG 2000 elevation model), see [TERRAIN_FORMAT.md](TERRAIN_FORMAT.md).

---

## 2. Header checksum (integrity and device binding)

Function `FUN_00105d04`, called from the chart loader `FUN_003f282c`.

```
SECRET = "d8ethebrestezexaqathaTrepedEkubafr5vuhaprupe3ucUphedeyuhaGespenU"   (64 B, VA 0x4b7768)

generic:       MAC == MD5(salt ‖ SECRET ‖ file[0x44:0x444] ‖ file[-0x400:])
device-bound:  MAC == MD5(salt ‖ SECRET ‖ file[0x44:0x444] ‖ file[-0x400:] ‖ device_id)
```

- `device_id` is the serial number as ASCII, here `2013021200000368` (see
  `firmware/deviceid.dat`, `firmware/license.dat`).
- Every file present (DK and DE) is signed **device-bound**.
- If a file is signed **generically**, `FUN_001061bc` binds it to the device itself on first
  opening: it generates a new random salt and rewrites the MAC with the serial number. Files
  built here can therefore be signed either generically (for any device) or device-bound
  straight away.
- Because the MAC covers the first and the last kilobyte of the file, it has to be
  recomputed **after** every change.

---

## 3. Decrypting a record

```
K     = the global key (32 ASCII bytes, section 4)
blob  = tile[+0x157C : +0x159C]
rk    = PC1_decrypt(blob, K)                  all 32 bytes             (FUN_002897b0)
pt    = PC1_decrypt_partial(payload, rk)      bytes 0..99, then        (FUN_002898ec)
                                              every 10th byte (100, 110, 120, …)
raw   = LZMA1 raw decode(pt)                  exactly pltx bytes       (FUN_00374958 = LzmaDecode)
        props byte 0x5D (lc=3, lp=0, pb=2), dictionary 16 MiB (0x01000000)
```

Notes:

- **Not every record is encrypted:** the 406 osm records from slot area **B** are only packed
  with LZMA (they begin with `00 00 6C …`). Their reader does not call `FUN_002898ec`
  (`FUN_003e5d20`). `decode_record()` tries PC1 + LZMA first, then LZMA alone.
- PC1 restarts with `rk` for every record, so the key stream resets per record. That is why
  the first ciphertext byte within a tile is always the same: the first LZMA byte is always
  `0x00`.
- Every byte that is not at a position 0..99 or 100 + 10·k is pure LZMA stream.
- Section 1a describes the layout of the decompressed content, with the details in the layer
  documents.

### PC1 in detail

This is standard PC1 with a 256-bit key (the constants `0x4E35`, `0x015A`), implemented in
`FUN_00289660` (assemble) and `FUN_002896c8` (code). Per byte:

```
inter = assemble()                       16 rounds over the 16 key words
c     = c XOR (inter >> 8) XOR (inter & 0xFF)
key[i] ^= c   for i = 0..31              when decrypting, with the PLAINTEXT
```

When encrypting, the key is modified with the plaintext byte first and the byte is emitted
afterwards. `si`, `x1a2` and `i` all start at 0.

---

## 4. The global key K

Function `FUN_002050a0`. The result ends up in a 0xA8-byte PC1 context at `ctx+0x88`, with
`ctx = *(*DAT_0055d504 + 0x1d8)`. Ghidra has not assigned the code for it (`0x1fd148`) to any
function.

```
if device_id[:8] in PREFIXES:
    K = "E89ACE5CE51E0669B4BA068CE8F63990"            (static, VA 0x484f08)
else:
    K = MD5(device_id[:8]) as upper-case hex          ("%02X" × 16 = 32 characters)
```

`PREFIXES` (19 entries, the table at VA `0x484e24`):
`20130125, 20130212, 20130213, 20130807, 20131010, 20131020, 20131026, 20150215,
20160505, 20160509, 20161014, 20161028, 20170606, 20170707, 20180914, 20181225,
20190320, 20190415, 20190618`

The device `2013021200000368` has the prefix `20130212` and therefore uses the static key.

---

## 5. Tools

Everything is one binary, `teasi`, built from `rust/`:

```bash
cd rust && cargo build --release        # ./target/release/teasi
```

| Command | Purpose |
|---|---|
| `info <chart>…` | header, tiles, record counts, and the `packages.xml` values (5.4) |
| `dump <chart> <out-dir>` | write every decrypted record to a file |
| `md5s <chart>` | `area cx cy md5` per record, to compare two charts |
| `check <chart>…` | parse and rebuild every record; must be byte-identical |
| `roundtrip <chart> [out]` | decode the whole file, write it again, compare (5.3) |
| `index <ta chart>` | the search index: parse, rebuild, compare |
| `ways`, `addr`, `poi`, `area`, `land` | the OSM extractors, as canonical dumps (5.5) |
| `dem <area.poly> <tile dir> <out.bin>` | the Copernicus elevation grid (5.5) |
| `osmpoi`, `osmpoint`, `osmarea`, `osm`, `ta`, `terrain` | the six layer compilers (5.6) |

`osmarea`, `osm`, `ta` and `terrain` need libgeos, opened at run time (`src/geos.rs`), so
building the binary itself needs nothing but a Rust toolchain, and running it needs nothing
else at all.

The device's serial number sits in one place: `DEVICE` in `rust/src/chart.rs`, overridable
with the environment variable `TEASI_DEVICE`. Every function with a `device` parameter takes
it as the default.

### 5.1 Reading and decrypting maps

`teasi info` prints the header, `teasi dump` decrypts every record of a file and writes them
out as individual files:

```bash
teasi dump 2013021200000368/7/943/20317/Denmark_osm.v20210916 out/osm
```

File names: `out/osm/<x>_<y>_<offset-within-the-tile-in-hex>.bin`. That is the decompressed
plaintext (`pltx` bytes). The serial number comes from `DEVICE` (`TEASI_DEVICE`, see
section 5).

**As a library** (`chart.rs`):

```rust
use teasi::chart::{self, Chart};

let c = Chart::open("2013021200000368/7/943/20317/Denmark_osmpoi.v20210915")?;
assert!(c.bound(&chart::device()));            // check the header checksum

for t in c.tiles() {                            // the tile directory
    for r in chart::records(&c.data, t.start, t.end) {
        let raw = c.record(&t, &r)?;            // plaintext of one record
    }
}
```

| Item | Purpose |
|---|---|
| `Chart::open(path)` | read a file, check the magic, keep the bytes in `.data` |
| `c.date()`, `c.typ()`, `c.layer()`, `c.country()`, `c.extra()` | the header fields |
| `c.bound(device)`, `c.generic()` | is the MAC right, device-bound or generic (section 2) |
| `c.tiles()` / `tiles(d)` | the tile directory as `Tile { x, y, start, end }` |
| `records(d, start, end)` | the record chain from `+0x159C` as `Rec { rel, len, pltx, .. }` |
| `c.record(&tile, &rec)` | the plaintext: PC1 → partial PC1 → LZMA, otherwise LZMA alone |
| `decode_record(payload, pltx, rk)` | the same step with the record key passed in |
| `record_key(d, tile_start, K)` | the tile's 32-byte key, decrypted with K |
| `global_key(device_id)` | the key K per section 4 |
| `header_md5(d, device)` | the MAC per section 2 (empty `device` = generic) |
| `package_md5(d)` | the checksum `packages.xml` carries (5.4) |
| `SECRET`, `STATIC_KEY`, `KNOWN_PREFIXES` | constants from the firmware |

### 5.2 `pc1.rs`: PC1 encryption

| Function | Purpose |
|---|---|
| `decrypt_blob(blob, key)` / `encrypt_blob(data, key)` | every byte (for the 32 B tile blob) |
| `decrypt_payload(data, key)` / `encrypt_payload(data, key)` | partial: 0..99, then every 10th byte |
| `Pc1::new(key)` | the low-level state with `dec_byte()` / `enc_byte()` |

The implementation was checked against the emulated ARM code (Unicorn): the results are
byte-identical. `rust/tests/compat.rs` keeps the fixtures.

### 5.3 `writer.rs`: writing maps

The inverse of `chart.rs`: out of the plaintext records per tile and slot, `write_chart`
builds a complete file. That includes tile heads with their slots, empty cell tables and a
new record key per tile. The records are packed with LZMA and encrypted with PC1, except in
slot area B. `write_chart` fills the header with the buffer sizes, a new salt and the
device-bound MAC.

```rust
use teasi::writer::{write_chart, Meta, TileContent};

let meta = Meta { date: b"20260918".to_vec(), typ: 1, layer: 8, country: 4,
                  tail_tile: None };                                   // osmpoi
let d = write_chart(&meta, &tiles, b"2013021200000368", true, None)?;  // false: generic
```

- Within a tile the records come in the order A, B, C, D, each sorted by slot, as in the
  original.
- A `TileContent` with `areas: None` = an empty placeholder tile (like `(0,0)` in osmarea).
  The ta extra table is passed as `tail`, together with `meta.tail_tile`.
- LZMA: the encoder appends an end marker, which the originals do not have. That does not
  matter: the firmware uses an unmodified `LzmaDecode` from the LZMA SDK 9.x
  (`FUN_0037be30`, props `5D 00 00 00 01`), which stops after `pltx` bytes and then returns 0
  (OK); 0 and 6 are accepted.
- The tiles are built in parallel.

**Round trip** (`teasi roundtrip <file> [<out>]`): the file is decrypted completely, rebuilt
and read again. What gets checked is that every record has the same plaintext, that the MAC
is right, and that the directory, the extra data and the header fields `0x44`–`0x57` agree.
The result for all five vector layers: **14,185 records identical**, with the pltx buffer
sizes equal to the original's. `teasi check` does the same for several files at once.

A test package for the device is in `build/test1/`: every layer rebuilt, with the POI
"Tivoli" (Copenhagen) renamed to "Tivoli TEASI-TEST" in osmpoi. **Tested on the device
(2026-09-18):** the map is displayed normally and the renamed POI shows up. That confirms the
whole chain (PC1, LZMA with the end marker, new record keys, the device-bound MAC).
`packages.xml` has to be updated as well, see 5.4.

### 5.4 Getting it onto the device: `packages.xml`

The Teasi presents itself over USB as mass storage ("SiRF GPS HH", Windows CE). The storage
only becomes readable after **confirming the connection on the display**; before that it
reports an invalid size and Linux gets nothing but I/O errors. Afterwards a FAT volume
`TFAT` (3.8 GB) appears:

```
BikeNav/Map/Countries/<Country>_<layer>.vYYYYMMDD   chart files (DE, NO and SE are complete too)
BikeNav/packages.xml                                the Tahuna software's installation list
BikeNav/Program/bikenav.exe, gpstuner.dat …         firmware and resources (texts, fonts)
```

**The check at startup.** `bikenav.exe` walks every map package in `packages.xml`
(`FUN_0028937c` → `FUN_002891d8` → `FUN_00288b04`) and checks each file:

| Check | Function | Condition |
|---|---|---|
| size | `FUN_002887c0` | `<size>` = the file size |
| checksum | `FUN_00288858` | `<md5>` = `MD5(file[0x44:0x444] + file[-0x400:])` |

So the MD5 only runs over 1 KB from `0x44` (behind the salt and the MAC) plus the last
kilobyte. That is why it stays valid when the firmware binds a file to the device and
rewrites the salt and the MAC. Checked on all six original Danish files. If a check fails,
"Karten nicht korrekt installiert. Teasi mit dem Computer verbinden und Karten erneut laden!"
appears ("maps not installed correctly, connect the Teasi to the computer and load the maps
again"; the text key `error_message_mappackage_tahuna` in `gpstuner.dat`, called in
`FUN_001faa98`). The map is loaded and displayed anyway.

**The procedure** after replacing chart files (back up the originals and `packages.xml`
first, into `build/device_backup/` for instance):

```bash
C=/run/media/$USER/TFAT/BikeNav
cp build/test1/Denmark_osm.v20210916 $C/Map/Countries/
teasi info $C/Map/Countries/Denmark_osm.v20210916     # prints size and md5
```

Put those two numbers into the `<size>` and `<md5>` of the entry whose `<url>` ends in that
file name. **Files without an entry** are not checked but are loaded anyway: the firmware
reads every `*.v*` in `Countries` (which is how the Great Britain files run, 5.7). With the
entries updated the message disappears (checked on the device, 2026-09-18).

### 5.5 The extractors and the elevation model

Each compiler reads its own objects out of a Geofabrik extract: `way.rs` the roads, paths,
watercourses, railways and land use with their tags, node ids and coordinates in Teasi units
(360°/2²⁸) plus the cycle and walking route relations per way, `addr.rs` the addresses and
places, `poi.rs` the POI and seamark candidates, `area.rs` the areas and the coastline, and
`land.rs` the worldwide land polygons. The compilers call them directly; `teasi ways`,
`addr`, `poi`, `area` and `land` run one on its own and write a canonical dump, which is
what the output of two runs can be compared with. For Denmark the way extractor takes 5.5 s
for 1,114,606 ways.

```bash
teasi ways osm_ref/denmark-220101.osm.pbf out/dk_ways.txt
```

The extract `osm_ref/denmark-220101.osm.pbf` (as of 2022-01-01, the closest one to the maps
of 2021) comes from `download.geofabrik.de/europe/denmark-220101.osm.pbf`. A Teasi D edge is
assigned to a way by converting its points (`X = cell_x·32768 + u − 512`) and comparing
against `round(X_osm)`.

**Elevations** do not come from OSM but from the **Copernicus DEM GLO-90**. `teasi dem`
downloads the 1°×1° tiles covering a boundary polygon from the public AWS bucket
`copernicus-dem-90m` (a tile that does not exist is open sea and is remembered as an empty
`.none` file), decimates them onto one grid of 3″ — 1200 points per degree, which is what
the tiles hold below 50° N; further north they are narrower — and smooths it with a Gaussian
of σ = 1 grid point, because the DEM is a *surface* model and trees and houses would
otherwise add up to spurious ascents along the roads. `teasi osm --heights=` and
`teasi terrain` read the result.

```bash
teasi dem osm_ref/great-britain.poly osm_ref/dem build/gb/dem.bin   # 234 tiles, 3.6 s cached
```

### 5.6 Compilers: layers from current OSM data

| Layer | Command | Status |
|---|---|---|
| osmpoi | `teasi osmpoi` | finished, calibrated against the original, see [OSMPOI_FORMAT.md](OSMPOI_FORMAT.md) "Building it from OSM" |
| osmpoint | `teasi osmpoint` | finished, calibrated against the original, see [OSMPOINT_FORMAT.md](OSMPOINT_FORMAT.md) "Building it from OSM" |
| osmarea | `teasi osmarea` | finished, calibrated against the original, see [OSMAREA_FORMAT.md](OSMAREA_FORMAT.md) "Building it from OSM"; the sea outside the boundary and the Faroe Islands come from the original file |
| osm | `teasi osm` | finished, calibrated against the original, see [OSM_FORMAT.md](OSM_FORMAT.md) "Building it from OSM"; with left turns and ascents in the routing graph (`--heights`); the Faroe Islands from the original file; the map is OK on the device |
| ta | `teasi ta` | address search: places, streets, house numbers, postcode districts and the search index, calibrated against the original, see [TA_FORMAT.md](TA_FORMAT.md) "Building it from OSM"; the search itself was replayed offline against the firmware's semantics (TA_FORMAT.md, "Search index") and then tested on the device |
| terrain | `teasi terrain` | elevation model (the elevation profile) and map images (a hillshade coloured by land cover), see [TERRAIN_FORMAT.md](TERRAIN_FORMAT.md); the heights come from `teasi dem` |

Every compiler takes `--country=N`; osm and osmarea also run without an original file (`-`),
see 5.7.

The procedure per layer: run the compiler on `denmark-220101` and compare object by object
with the original file (calibrating the rules), then switch to `denmark-latest`. The file
names stay as they were, so that `packages.xml` only needs new sizes and MD5s (5.4); the date
in the header is the date of creation. `poly.rs` reads the Geofabrik boundary
(`osm_ref/denmark.poly`), and the compilers only take objects inside it. The Faroe Islands
are not part of the Geofabrik extract and are therefore missing from the newly built layers.

### 5.7 Great Britain (England, Scotland, Wales)

As of 2026-09-18, country code **17** (United Kingdom), file names
`GreatBritain_<layer>.v20260918`, with no entry in `packages.xml`. Source: the Geofabrik
`great-britain-latest.osm.pbf` (2.2 GB) and `great-britain.poly`, plus the land polygons from
osmdata.openstreetmap.de and the Copernicus DEM. terrain
(`GreatBritain_terrain.v20260919`) and ta (`GreatBritain_ta.v20260919`, the address search)
are produced as well, see below.

Every layer is one command from the 2.2 GB extract straight into the chart file; the
details are in the layer documents.

| Step | Command | Time | RAM |
|---|---|---:|---:|
| DEM grid | `teasi dem osm_ref/great-britain.poly osm_ref/dem build/gb/dem.bin` | 3.6 s | 3.3 GB |
| POIs | `teasi osmpoi … --country=17` / `teasi osmpoint …` | 35 + 17 s | 0.9 GB |
| Areas | `teasi osmarea … - --country=17 --land=…` | 3:10 | 7.1 GB |
| Streets | `teasi osm --heights=… --country=17 "--name=United Kingdom" … -` | 4:13 | 16.0 GB |
| Addresses | `teasi ta --country=17 "--name=United Kingdom" …` | 3:19 | 13.7 GB |
| Terrain | `teasi terrain --country=17 --land=… --area=… …` | 1:24 | 7.9 GB |

The result: osm 531 MB (12.1 M graph nodes, 30 M edges, 57 % with an ascent, 33 % of the
nodes with left turns), osmarea 83 MB, osmpoi 17 MB (750,000 POIs), osmpoint 0.5 MB (12,561
seamarks). On the device: POIs and streets checked in London.

**Careful with the record sizes:** 3 B records (north London, Manchester, up to 10.8 MB
decompressed) and 12 D records (up to 3.2 MB) are larger than anything in the original German
map (6.5 / 1.3 MB). Should the device hang on them: make the dense cells smaller (by not
routing footpaths, for instance).

**terrain (2026-09-19):** without a terrain file the elevation profile of a tour shows
nothing. `GreatBritain_terrain.v20260919` (37 MB, 85 regions) holds the elevation tiles from
`build/gb/dem.bin` (what `teasi dem` wrote) plus the map images:
```bash
teasi terrain --country=17 --land=osm_ref/land-polygons-split-4326/land_polygons.shp \
    --area=osm_ref/great-britain-latest.osm.pbf \
    build/gb/dem.bin osm_ref/great-britain.poly build/gb 20260919     # 1:24, 7.9 GB
```
The map images are a hillshade, coloured by water and land cover (the OSM areas); Ireland and
France at the edge get the relief only. Reading the heights back: Ben Nevis 1292 m (really
1345), Snowdon 1025 m (1085), Trafalgar Square 19 m. The summits come out slightly too low
because of the smoothing. On the device: the elevation profile was OK with the first version
(heights only), and the map images and the elevation profile of the version with images are
OK too (2026-09-19).

**ta (2026-09-19):** the address search with a country-wide place index. Without this file
the search only finds places and streets near the current position.
```bash
teasi ta --country=17 "--name=United Kingdom" osm_ref/great-britain-latest.osm.pbf \
    osm_ref/great-britain.poly build/gb/GreatBritain_ta.v20260919 20260919    # 3:19, 13.7 GB
```
114 MB. House numbers only exist where OSM has addresses (5 of about 30 M). The details and
the checks are in [TA_FORMAT.md](TA_FORMAT.md). **Tested on the device (2026-10-07):**
searching a place, a street and a house number works, and so does routing across north London
(Wembley → Walthamstow, over the two heaviest B records).

### 5.8 Ghidra helper scripts

The headless scripts are in `ghidra_scripts/`, and how to call them with which arguments is
in [../ghidra_scripts/README.md](../ghidra_scripts/README.md). `bikenav.exe` was analysed in
a Ghidra project of its own (Ghidra 12); the one-off searches from the analysis phase are in
`ghidra_scripts/hunts/`.

Code that Ghidra has not assigned to a function is only found by `InsnGrep` (`in ?` in its
output). That is where the key initialisation sits (`0x1fd148`), for instance. Such places
can be disassembled with Capstone.

### 5.9 How it was verified

Everything described above was first written in Python and then ported to Rust module by
module, with the Python output as the reference at every step: the shell (PC1, the header MAC,
raw LZMA1, the containers A/B/C/D and osmpoint, the writer, the search index), reading OSM,
and all six compilers, and finally the elevation grid. Once every layer agreed the reference
was removed; it is in the history up to commit `fc17505`. The record of what agreed with what
is this section, and the checks that do not need the reference are still here:

```bash
cd rust && cargo build --release
./target/release/teasi check <maps>/Denmark_*.v2*   # 14,185 records byte-identical, 1.3 s
./target/release/teasi roundtrip <maps>/Denmark_ta.v20180608
./target/release/teasi index <maps>/Denmark_ta.v20180608
```

Decrypting is 40 to 70 times faster than in Python (where PC1 manages only 0.13 MB/s),
writing about 8 times; there the LZMA compression at around 1.5 MB/s per core sets the lower
bound. Two pitfalls are described in `rust/README.md`: liblzma writes raw LZMA1 only as
`.lzma` (cut off the 13-byte header, and the length in the header has to stay "unknown", or
the end marker gets in the way), and the containers have to pass the fields nobody
understands straight through.

Reading OSM is ported too (`pbf.rs`: the PBF reader and the node index, `addr.rs`:
the address extractor). For Denmark it delivers all 2,628,399 entries **bit-identically**, in
4.7 instead of 270 s; for Great Britain 5,023,341 of 5,023,358 addresses and 111,338 of
111,340 places bit-identically. Both sides wrote the same canonical dump, which was compared
line by line; `teasi addr <pbf> <out>` still writes it.

That the areas come out right at all is down to libosmium's area assembler, reproduced in
`pbf.rs`: out of **edges**, which are normalised and sorted and cancel each other out in
pairs when they appear twice (so two ways running alongside each other merge into one ring);
locations are compared, not node ids; a ring that touches itself falls apart into two there;
every ring starts at its smallest vertex and repeats it at the end; outer/inner is decided by
nesting, not by the member role; and `area=no` forbids the area. Without all that, 4.3 of the
5.0 M British addresses were off by a median of 1.6 m.

All six layer compilers are ported. `poi.rs` reads the candidates for `osmpoi.rs` and
`osmpoint.rs`, all in one pass over the PBF (Great Britain 35 and 17 s instead of 24 min for
the extraction alone); for Denmark all 3699 osmpoi and all 128 osmpoint records are
byte-identical with the Python output, for Great Britain 15,603 of 15,619 osmpoi records and
all 354 osmpoint records. `osmarea.rs` (with `area.rs`, `land.rs` and `geos.rs`) builds the
area layer: 308 of 340 Danish records byte-identical, 1029 of 1157 British ones. What
deviates are multipolygons with self-touching rings, which libosmium splits differently.

`osm.rs` (with `way.rs` for the ways and `heights.rs` for the ascents) builds the street
layer: for Great Britain **all 22,711 records** are byte-identical and the file is the same
size down to the byte as Python's (531,333,357 B), in 4:13 instead of 18:23 and with 16.0
instead of 18.0 GB. For Denmark 5981 of 5982 records; the one difference is 5 of 3,004,615
edges with an ascent that differs by 1 cm, because 16 pairs of known node heights sit at the
same position and carry different heights — which of them enters the mean of the 4 neighbours
is arbitrary. All 1,538,105 Danish ways come out of the extractor bit-identically, tag tables
and flags included.

`ta.rs` (with `grid.rs` instead of scipy's kd-tree) builds the address search together with
the search index: for Denmark **all 3958 records and the search index are byte-identical**,
for Great Britain 13,679 of 13,680 records, and both files are the same size down to the byte
(113,851,396 B) — in 3:19 instead of 4:12 and with 13.7 instead of 16 GB. The one deviation
is a house number range ending at 149 instead of 147, because the Rust address extractor
finds two addresses more (the same residue as above). Three things had to change before two
runs over the same input produced the same output at all: addresses, places and interpolation
lines are sorted canonically, a search index node's children are sorted by character
(previously the iteration order of a Python `set`, so dependent on the hash seed), and a
correlation below 10⁻¹² counts as zero instead of its last bit deciding the direction of a
house number range.

`terrain.rs` builds the elevation model and the map images. For Denmark as for Great Britain
**every elevation tile** (1034 and 1999 respectively) is byte-identical with the Python
output — down to one byte per tile, because OpenJPEG writes its own version into the comment
marker and Pillow ships a different one. Great Britain takes 1:24 and 7.9 GB and reads the
PBF and the shapefile itself along the way; Python needs 2 min and 10 GB for the compiler
alone, plus the three extractions. Three pieces of Pillow sit in `raster.rs`, reproduced line
by line from the C sources and all three bit-identical: the polygon filler
(`ImageDraw.polygon`), the Lanczos resize (`Image.resize`) and the hillshade (`np.gradient`).
The elevation tiles go through OpenJPEG itself (`openjpeg-sys`), because how the rate is
distributed over the code blocks is a matter of implementation and the device's JP2 decoder is
undocumented; the map images go through `jpeg-encoder` instead of libjpeg-turbo, which yields
different bytes and about half the images pixel-identical (mean deviation 0.025 out of 255).
That comparison ran region by region outside `teasi check`, which does not know this layer —
it has no slot areas.

```bash
./target/release/teasi osmpoi  <pbf> <poly> <out> [YYYYMMDD] --country=17
./target/release/teasi osmpoint <pbf> <poly> <out> [YYYYMMDD] --country=17
./target/release/teasi osmarea <pbf> <poly> <original|-> <out> [YYYYMMDD] --country=17
./target/release/teasi osm <pbf> <poly> <original|-> <out> [YYYYMMDD] --country=17 \
    "--name=United Kingdom" --heights=<file.bin>
./target/release/teasi ta <pbf> <poly> <out> [YYYYMMDD] --country=17 "--name=United Kingdom"
./target/release/teasi terrain <heights.bin> <poly> <out> [YYYYMMDD] --country=17 \
    --land=<land_polygons.shp> --area=<pbf>
./target/release/teasi md5s <map>   # on both files, then diff
```

`osmarea`, `osm`, `ta` and `terrain` (with images) need **libgeos** (shapely uses it too, and
the result is only bit-identical with the same version): `geos.rs` loads the library at
runtime with `dlopen`, found through `TEASI_GEOS`. The build itself does not need it. One
thing the port forced: the areas are sorted by their osmium id, because the extractor's order
(libosmium's buffer order) helps decide which polygon comes first in a union and cannot be
reproduced.

**Nothing stays in Python.** The elevation data was the last piece (`dem.rs`): its own
GeoTIFF reader for the Copernicus tiles — float32, DEFLATE, the floating-point predictor of
TIFF Technical Note 3 — and `scipy.ndimage.gaussian_filter` reproduced down to the rounding,
numpy's pairwise summation for the kernel included. The 1.35 GB grid for Great Britain comes
out **byte-identical** with the one Python built, and a Danish test box does too; the TIFF
reader and the Gaussian are pinned in `rust/tests/compat.rs` against fixtures from
`tifffile` and scipy. What remains unported is only the reconstruction of node heights from
an original chart's ascents (scipy's `lsqr` over 2.1 M equations), which needs an original
file to begin with.

---

## 6. Important addresses in `bikenav.exe`

| Address | Meaning |
|---|---|
| `FUN_003f282c` | chart loader: magic, header, tile directory |
| `FUN_00105d04` | check the header MAC (generic / device-bound) |
| `FUN_001061bc` | bind a generic file to the device (re-sign it) |
| `FUN_002050a0` | derive the global key K |
| `0x1fd148` | `new(0xA8)` PC1 context, K to `ctx+0x88` (not assigned to a function) |
| `FUN_003e4460` | load a tile, read the blob at `+0x157C` and decrypt it |
| `FUN_003e6044` / `003e5d20` / `003e52d0` / `003e651c` / `003e56fc` | record readers for slot area A / B (routing graph) / C (osmpoint) / C / D (section 1a) |
| `FUN_002704f0` | transpose an array back |
| `FUN_003cf4d8` | routing cost function: flags, the cost factor (bits 20–23, the table at VA `0x4fc040`), the ascent (edge `[2]`), left turns (node `[2]`) |
| VA `0x4fbb50` | table POI type → `poitype_*` name (75 entries) |
| `FUN_003a25c8`, `FUN_003a20c0` | image decoding (RGB565), also calls `FUN_002898ec` |
| `FUN_00289660` / `FUN_002896c8` | PC1 assemble / code |
| `FUN_002897b0` | PC1 complete (32 B) |
| `FUN_002898ec` | PC1 partial (0..99, then every 10th byte) |
| `FUN_0028937c` / `FUN_00288b04` | check the map packages from `packages.xml` (size `FUN_002887c0`, MD5 `FUN_00288858`), see 5.4 |
| `FUN_00374958` / `FUN_0037be30` | `LzmaDecode` (LZMA SDK 9.x, wrapper / implementation) |
| `FUN_001b2f9c` / `FUN_001b3070` / `FUN_001b2564` | MD5 update / final / transform |
| VA `0x4b7768` | SECRET (64 characters) |
| VA `0x484e24` | the serial prefix table (19 × 12 B); `+0xe4` the static key, `+0x108` `"%02X"` |

---

## 7. Dead ends (do not pick these up again)

- The codec family `FUN_00354610` / `FUN_00354c34` / `FUN_00354f8c` belongs to the **RFP0**
  format, not to the charts.
- XOR hunts, the hypothesis of "entropy coding only", zlib/Deflate: all disproved. The even
  byte distribution comes from LZMA plus PC1.
- `0x1B62` and `0x159C` do not appear as constants in the code. That is because they are
  read, not because nothing is checked.

---

## 8. Next steps

1. **Plaintext structures per layer**: the containers are solved everywhere (the round trip
   is byte-identical). Assigned by matching against OSM: street classes, almost every flag
   bit, line types, area classes, the routing graph. The remainders are under "Open
   questions" in the layer documents.
2. ~~Georeferencing~~: solved (section 1a, including the margin of 512 on lines).
3. ~~Find the routing graph~~: slot area B of osm, see [OSM_FORMAT.md](OSM_FORMAT.md).
4. **OSM → Teasi compilers** (5.6): the writer, the round trip and the device test all
   succeeded (5.3, 5.4), and the osmpoi, osmpoint, osmarea and osm compilers are finished,
   osm with left turns and ascents. Routing is tested on the device (north London,
   2026-10-07).
5. **Great Britain** (5.7): every OSM layer, terrain (heights and map images) and ta (the
   address search) are built and on the device, and the elevation profile, the address search
   and routing all work. What is left are the individual fields listed under "Open questions"
   in the layer documents.
