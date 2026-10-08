# ta format (`Denmark_ta.v20180608`)

As of 2026-09-19. **Fully understood**:
- `parse_a`/`build_a` and `parse_d`/`build_arrays` in `layers.rs` rebuild all 4125 records
  byte-identically (A 101, D 4024).
- `ta_index.rs` rebuilds the search index byte-identically.
- The firmware's search flow is decoded (section "Search").

**Compiler:** `teasi ta` builds the layer from OSM, see "Building it from OSM".

Content: **address data for the address search**:
- a street network with a street name per side of the road and **house number ranges**,
- a directory of places with coordinates and multilingual region names,
- a country-wide **search index** over the place and postcode names.

Without a ta file the search only finds streets nearby (the osm layer). With it, it finds
places across the whole country, plus their streets and house numbers.

The Danish file is considerably older (2018) than the osm layers (2021). "ta" presumably
stands for Tele Atlas (TomTom), so the data probably does not come from OSM.

**Coordinates:** as in osm's D records, the lines have a margin of 512 units (range
0 … 33,792), see [OSM_FORMAT.md](OSM_FORMAT.md).

Shell, encryption, slot areas and the coordinate system: see
[CHART_FILES.md](CHART_FILES.md), sections 1, 1a and 3. The containers are the same as in
osm, see [OSM_FORMAT.md](OSM_FORMAT.md) (A records, a1 edges, geometry) and
[OSMPOI_FORMAT.md](OSMPOI_FORMAT.md) (the general D record).

## Where it sits

| | |
|---|---|
| Header `0x4C` (type) | `2` (the osm layers have 1) |
| Header `0x50` | `0x12` |
| Header `0x70` | offset of the search index (see below) |
| File name | has to contain `_ta.v`: only then does the firmware tie the index to this map (`FUN_0016068c`, string at VA `0x4a751c`) |
| Tiles | 14 |
| Slot area D | 4024 records, array a1 only: 1,163,823 street edges, 642,517 of them with house numbers |
| Slot area A | 101 records: the directory of places and street names |

## Search (firmware)

The flow goes from the place via the street to the house number:

1. **Place:** the input is looked up in the search index (`FUN_00162788`, `FUN_00163384`,
   `FUN_0016201c`). A hit gives the name, the coordinates and the list of 4×4 cells the
   place appears in.
2. **Street** (`FUN_00163960`): for every cell the firmware loads the A record of the ta
   map. The entry has to carry the same country name as the index. It looks for the 0x18
   sub-elements whose string contains the place name as one of the alternatives separated by
   `|` (`FUN_00163844`: compare up to `|`, `#` or NUL). Their street names (the blk5 range)
   make up the street list. The text behind `#` is appended to the street names.
3. **House number** (`FUN_00165a0c`):
   - The firmware walks the 8×8 D cells of the 4×4 cell, x outer and y inner, and the a1
     edges within them.
   - It takes the **first** edge with `[0]` or `[1]` = the name offset where the number n
     falls into the range of one side, the left `[2]` first, then the right `[3]`.
   - Condition: `min ≤ n ≤ max` and the same parity as `min`. With bit 15 in the first word,
     any parity is allowed.
   - Position: the fraction `t = (n − min + 0.5) / (max − min + 1)` of the length along the
     geometry. If the range descends (from > to), `1 − t` applies. Without a number it takes
     the middle (`t = 0.5`).

   The side therefore only affects the parity check. The position is not offset towards
   that side.

## D records: streets with house numbers

Same a1 layout as in osm (10 × u32), but with additional fields in use:

| Field | osm | **ta** |
|---|---|---|
| `[0]` | name | name (left side) = byte offset into blk5 of the A record of the parent 4×4 cell |
| `[1]` | = `[0]` | name of the right side. Differs from `[0]` in 50,945 of 1,163,823 cases (streets on a place boundary: same name, but in the street list of a different place); 105,410 edges have no name |
| `[2]` | `0xFFFF7FFF` | **house numbers on the left**: `u16 from \| u16 to << 16`, `0xFFFF7FFF` = none |
| `[3]` | `0xFFFF7FFF` | **house numbers on the right** |
| `[4]`, `[5]` | node ids | node ids, numbered from 1 **per 8×8 cell** (checked: 57,985 ids, each at only one position); no connection to the osm routing graph |
| `[6]` | flags | flags (`0x280`, say), not read by the search |
| `[7]` | class · length | class (0–9, **its own scale**) · length (same unit as osm) |
| `[8]` | number of u32 of geometry | likewise |

**House number word:** the low 16 bits are the number at the start of the geometry, the high
16 bits the one at the end. The numbers may descend (`52 → 50`). **Bit 15** of the first
number means "both parities", as with consecutive numbering on one side (see Search; 5249
sides in DK).

**Left/right** is in the direction of the geometry. Checked against OSM addresses
(`denmark-220101`, every 20th D cell): addresses with a cross product < 0 (in u, v with v
pointing south) are in `[2]` 97 % of the time, those with a cross product > 0 in `[3]` 97 %
of the time.

Checked in Aarhus (names resolved through the A records):

```
Kystvejen          L (57, 63)       Kystvejen        L (65, 65)
Nørreport          R (2, 6)         Nørreport        R (24, 24)
Marselis Boulevard L (52, 50)       Marselis Boulevard R (11, 3)
```

Classes in ta: 0 (5298), 1 (1339), 2 (20,315), 3 (56,681), 4 (86,140), 5 (22,424),
6 (171,085), 7 (672,615), 8 (127,521), 9 (405). In the centre of Aarhus, Nørreport,
Nørrebrogade and Hallandsgade have class 2, for example. So the scale is **not** the osm
street class. Presumably it is TomTom's "functional road class".

## A records: places and their streets

Same layout as in osm (see OSM_FORMAT.md "A records"), with one entry carrying the country
name (`Denmark`, `[2] = 1`). blk5/blk6 hold the street names that `a1[0]`/`a1[1]` point at.
They are **grouped by place**: every 0x18 sub-element describes one group (10,641 in DK):

| Field | Content |
|---|---|
| `[0]` | 0 (pointer to the string) |
| `[1]` | length of the string (UTF-16 including the NUL) |
| `[2]` | **byte offset into blk5** of the group's first street name |
| `[3]` | **number of street names** in the group |
| `[4]` | latitude as float32 (56.93876, say) |
| `[5]` | longitude as float32 (8.36874, say) |

The groups follow one another in blk5 without gaps. Within a group the names are sorted (as
in osm). The same street name can appear in several groups and then has several offsets. A
group without a string (19 records, motorways for instance) always comes first.

The string names the group's places as alternatives separated by `|`, optionally with
`#suffix` at the end:

```
Ejby, Wedellsborg (Fyn)|Ejby (Fyn)
[DANAalborg¦GERAalborg¦…], Svenstrup ([DANNordjylland¦…])|[DANAalborg¦…] ([DANNordjylland¦…])#Svenstrup
```

Every alternative is exactly the name of one hit in the search index, e.g. "municipality,
district (region)". Multilingual names have the form `[DANKøbenhavn¦GERKopenhagen¦…]`. The
0x10 sub-elements are not used in DK.

## Search index (extra table, header `0x70`)

The table sits unencrypted behind the records of one tile (DK: tile (138,24), 2,596,612 B).
No slot points at it. The firmware registers it while loading (`FUN_003f282c` →
`FUN_0016068c`, only for layer bit `0x10` and a file name containing `_ta.v`) and reads it
straight from the file during a search. All offsets count from the start of the table.

```
u32   offset of the word pool (at the end of the table)
u8    n languages, n × (3 ASCII characters, u8 index): DAN 1, GER 2, ENG 3, … POR 11
u32   1
u16   n + UTF-16 country name ("Denmark", must match the name in the A record)
u32   offset of the root node
nodes (prefix tree, pre-order: the node, then the subtrees of its children)
word pool: per word a u16 byte length + UTF-8
```

**Node:**

```
u32   number of children << 24 | number of distinct hits in the subtree
      (0xFFFFFFFF: then u32 hits, u32 children)
n ×   u16 character, u64 language mask, u32 offset of the child node
u16   number of hits at this node, then per hit:
      u8   type: bit 7 = name from the word pool
           0 = place with streets, 1 = postcode, 2 = place without streets (coordinates only)
      name: pool: u32 (word count << 24 | offset), then (word count − 1) × u32 offset,
            joined with spaces; otherwise u16 byte length + UTF-8
      f32 longitude, f32 latitude (for type 1 only if the file magic is ≥ 0x1B5D; 0x1B62 is)
      types 0 and 1: u8 cells (0xFF: then u16), per cell u16 x, u16 y of the 4×4 cell,
                     for type 1 additionally u16 k + k × u32 blk5 offsets of the streets
```

The path to a node is the **search key**. Every hit sits under all of its keys:
- every word of the name,
- every multi-part part of the name between the commas, without the region part in
  parentheses,
- per language variant.

The keys are lower case, without accents (ø → o, æ → a) and with hyphens as spaces.
Checked: all 13,121 Danish hits have exactly these keys (`ta_index.fold`). The language mask
is almost always `0xFFFFFFFF` or `0xFFFFFFFFFFFFFFFF`, i.e. every language. The firmware
compares it against the current language.

DK: 45,070 nodes, 13,121 hits: 9146 places with streets, 2887 without, 1088 postcodes. The
postcodes have keys like `9990` and list the street offsets per cell.

## Rebuilding it

```bash
teasi check <maps>/Denmark_ta.v20180608    # every A and D record, taken apart and rebuilt
teasi index <maps>/Denmark_ta.v20180608    # the search index behind header 0x70
```

The search itself — place → cells → street list → first matching edge → position — can be
replayed offline against the semantics above; "Match against Denmark" below does that at 3000
random addresses.

## Building it from OSM

```bash
teasi ta --country=17 "--name=United Kingdom" osm_ref/great-britain-latest.osm.pbf \
    osm_ref/great-britain.poly build/gb/GreatBritain_ta.v20260919 20260919
```

One command from the extract to the chart file: 2:24 and 4.7 GB for Great Britain, 10:45 and
12.0 GB for Germany, 38 s for Denmark. It needs libgeos and reads the PBF twice, because it
needs both the addresses and the streets.

- `addr.rs` reads every address (`addr:housenumber` with `addr:street`), the interpolation
  lines (`addr:interpolation`) and the places (`place=*` with a name).
- `ta.rs` takes the streets from `way.rs`, the same extractor the osm layer uses, and
  searches for the nearest points with the uniform grid of `grid.rs` instead of scipy's
  `cKDTree`; see [../rust/README.md](../rust/README.md).

1. **Streets:** named streets of the osm classes 0–8 (motorway down to pedestrian zone),
   split at junctions as in osm and **additionally at the D cell borders**. That way every
   piece lies within one cell and the firmware's interpolation uses the whole range. ta
   class: motorway 0, trunk 1, primary 2, secondary 3, tertiary 4,
   residential/unclassified 6, pedestrian 8. Flags `0x280`.
2. **House numbers:**
   - Every number goes to the nearest street piece of the same name (at most 100 m away).
   - The side follows from the cross product.
   - From/to are the numbers at the first and last point along the piece. The direction
     follows the correlation.
   - With more than 10 % of the numbers of the other parity, bit 15 is set; otherwise those
     numbers are dropped.
   - `12a` counts as 12, `12-16` as 12 and 16.
   - Interpolation lines are expanded into individual numbers.
3. **Places:** every piece gets a settlement (city/town/village/hamlet; the smallest ratio
   of distance to radius, with 12 / 5 / 2 / 0.8 km) and a district
   (suburb/quarter/neighbourhood, 2.5 / 1.5 / 0.8 km). On top come up to 3 `addr:city`
   values of the street within the 4×4 cell (post towns). Pieces of the same name whose ends
   are less than 60 m apart form one street, and that street gets a shared group (filters
   below). The group string: `town, district|town|post town|…`. Every alternative gets the
   **region** in parentheses, as in the originals: the `admin_level=4` boundary its place
   node lies in (`Borken (Nordrhein-Westfalen)`; a post town without a place node takes the
   region of its 4×4 cell). Without it, the five German places called Borken were five
   identical hits, and the one in Westphalia could not be picked. Places of the same name
   and region more than 5 km apart also get the levels below the region that tell them
   apart (admin levels 5–8; per name the first of 6, 7, 8, 5, 6+8, 7+8, 5+8, 6+7 that
   separates them, or the one that comes closest): `Berg (Bayern, Landkreis Bad
   Tölz-Wolfratshausen, Eurasburg)`, `Ejby (Region Sjælland, Køge Kommune)`. In Germany
   that takes the names occurring more than once in a state from 7273 to 108 (2026-10-09;
   the rest are names that already end in parentheses, such as `Frankfurt (Oder)`, which
   stay as they are). A boundary cut by the edge of the extract is left out. Germany: 12,517
   boundaries, 645 s and 12.0 GB instead of 510 s and 11.5 GB. The firmware chains streets
   of the same name across several cells and tries them in turn (the chain at
   `+0x108`/`+0x114` in `FUN_00165a0c`).
4. **Index:**
   - Type 0: one hit per name and place node, with the cells it appears in.
   - Type 2: the remaining places (locality, isolated_dwelling, farm and island too).
   - Type 1: postcodes with their streets, from `addr:postcode`: the outward code of a
     British postcode (`SW1A`), otherwise the code up to the first space (`46325`, `1234`
     of the Dutch `1234 AB`). Up to 2026-10-09 only the British ones were built, so a
     German postcode found nothing (the original has `46325` with 983 streets).
   - Keys leave out the region in parentheses, as in the originals.
   - Names are stored inline (no word pool), every language mask full, the language list as
     in DK.

### Reproducibility

Three rules decide whether two runs over the same input produce the same file at all:
addresses, places and interpolation lines are sorted canonically (the extractor hands out
libosmium's order), a search index node's children are sorted by character (appending them in
the iteration order of a hash set makes the file depend on the hash seed), and a correlation
below 10⁻¹² counts as zero (`CORR_TOL`) instead of letting its last bit decide the direction
of a house number range.

**Match against Denmark:** built from `denmark-220101`: 22.6 MB (the original is 22.2 MB).
Looked up at 3000 random OSM addresses with the firmware's logic:

| | found | median | 75 % | 90 % | < 50 m |
|---|---:|---:|---:|---:|---:|
| original (2018) | 2727 | 25 m | 47 m | 167 m | 70 % |
| rebuilt from OSM | 2882 | 24 m | 44 m | 111 m | 76 % |

The distance includes the way from the centre of the house to the street. The user's flow
place (`addr:city`) → street → number (1500 addresses with a post town): 96 % found, 76 %
under 50 m.

**Great Britain (2026-09-19):** `GreatBritain_ta.v20260919`, 114 MB.

| Figure | Value |
|---|---|
| streets | 5.3 M pieces (915,000 streets) |
| house numbers (OSM) | 5.06 M, 97 % of them on 825,000 pieces |
| index | 48,352 places with streets, 82,998 without, 2603 postcode districts (21 MB) |
| largest A record | 1.18 MB (London; the German original has 1.25 MB) |
| largest D record | 0.7 MB |

The user's flow at 1500 OSM addresses: 94 % found, median 20 m, 86 % under 50 m.

**Limitation:** for GB, OSM holds only about 5 M of the roughly 30 M addresses. Bristol,
Nottingham, Coventry, Edinburgh and parts of London are well covered, among others.
Elsewhere the search finds the place and the street (almost completely) but often no house
number; you then end up in the middle of the street.

To keep the A records small, every street gets only its main district, plus the places that
cover at least 10 % of its length and the post towns holding at least 25 % of its
addresses. Without these filters London had 8558 groups and 2.3 MB.

## Open questions

1. The class scale 0–9 and the flags `[6]`. The search does not read them; whether they
   matter elsewhere is open.
2. The meaning of `#suffix` (the postal town in DK), the field "1" in the index header, and
   the language masks that are not "all".
3. The 0x10 sub-elements of the A records.
