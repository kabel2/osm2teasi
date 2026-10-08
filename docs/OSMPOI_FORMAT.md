# osmpoi format (`Denmark_osmpoi.v20210915`)

As of 2026-09-18. **Fully understood**: `layers.rs` rebuilds all 3704 records
**byte-identically** (`build_osmpoi(parse_osmpoi(raw)) == raw`, checked by `teasi check`).

Content: **points of interest from OSM** (restaurants, cafés, cash dispensers, stops,
churches, hotels, camp sites, monuments …). The Danish file: 79,862 POIs in 3704 records
(the Faroe Islands included), 32,597 of them without a name.

Shell, encryption, slot areas and the coordinate system: see
[CHART_FILES.md](CHART_FILES.md), sections 1, 1a and 3.

## Where it sits

| | |
|---|---|
| Header `0x50` | `8` (layer id) |
| Slot area | **D** (tile head `+0x180`, 32×32 sub-cells) |
| Reader in the firmware | `FUN_003e56fc` (internal type 1) |
| 1 record | = every POI of **one 32×32 sub-cell** (0.0439° × 0.0439°, about 4.9 × 2.8 km in DK) |

## D record (general layout)

osmpoi, osm and ta all use the same container in slot area D. The generic parser/builder
`parse_d` / `build_arrays` in `layers.rs` reads and writes all 12,934 D records of the three
files byte-identically.

```
0x00  13 × u32  head
        [0]   size (see below; the firmware overwrites the field with cell_x)
        [1]   0  (overwritten with cell_y)
        [2]   0
        [3]   n1  number of array a1 (0x28 B)   [4]  0 (pointer)
        [5]   n2  number of array a2 (0x08 B)   [6]  0
        [7]   n3  number of array a3 (0x0C B)   [8]  0
        [9]   n4  number of array a4 (0x14 B)   [10] 0
        [11]  n5  number of array a5 (0x18 B)   [12] 0
0x34  a1, a2, a3, a4, a5 one after another, every array TRANSPOSED BYTEWISE
      then the variable parts, array by array and element by element within each:
        a1: field [8] = number of u32 that follow      (pointer in [9])
        a2: field [0] = number of u32                  (pointer in [1])
        a3: field [1] = number of u32                  (pointer in [2])
        a4: field [1] = number of u16 (string),        (pointer in [0])
            then field [3] = number of u32             (pointer in [4])
        a5: field [1] = number of u16 (string),        (pointer in [0])
            then field [3] = number of u16 (string)    (pointer in [2])
```

There is no alignment and no padding; the record comes out exactly. Head `[0]`: in osmpoi
it is the sum of the array sizes (`n5 · 0x18`), in osm and ta mostly `n1 · 0x28`. The
firmware does not evaluate the field.

## osmpoi: array a5 only

In osmpoi `n1..n4 = 0`; there is only a5, with one entry per POI.

| a5 field | Content |
|---|---|
| `[0]` | 0 (pointer to the name) |
| `[1]` | length of the name in UTF-16 characters including the NUL, 0 = no name |
| `[2]` | 0 (pointer to the attributes) |
| `[3]` | length of the attribute string including the NUL, 0 = no attributes |
| `[4]` | **POI type** (index into the table below) |
| `[5]` | **position** `(v << 16) \| u` |

The name and the attribute string follow per POI, both UTF-16LE.

### Coordinates

As in every record, relative to the north-west corner of the sub-cell, **32768 units per
sub-cell**. At 32×32 one unit is therefore 360°/2²⁸ ≈ 1.34·10⁻⁶° (about 15 cm):

```
cell_x = tile_x·32 + slot // 32,   cell_y = tile_y·32 + slot % 32
lat = 90  − (cell_y · 32768 + v) · 360/2^28
lon = −180 + (cell_x · 32768 + u) · 360/2^28
```

Checked: "Den Gamle By" (museum, Aarhus) → 56.1589 N, 10.1916 E; "Tivoli" → 55.6733 N,
12.5690 E; "Gulahusid" (Mykines, Faroe Islands) → 62.1037 N, 7.6456 W.

### POI types `[4]`

The names come from a pointer table in `bikenav.exe` (`.data`, VA `0x4fbb50`, 75 entries;
the string `poitype_<name>` is the translation key). The "DK" column gives the count in the
Danish file.

| ID | poitype | DK | | ID | poitype | DK |
|---:|---|---:|---|---:|---|---:|
| 0x00 | bank | 705 | | 0x1c | train_station | 573 |
| 0x01 | bikestore | 689 | | 0x1d | christian_church | 2687 |
| 0x02 | cafe_pub | 5776 | | 0x1e | muslim_mosque | 23 |
| 0x03 | cashdispenser | 543 | | 0x1f | buddhist_temple | 8 |
| 0x04 | departmentstore | 10 | | 0x20–0x23 | hindu/shinto/taoist/sikh_temple | 0 |
| 0x05 | doctor | 210 | | 0x24 | jewish_synagog | 1 |
| 0x06 | emergencymedicalservice (defibrillator) | 586 | | 0x25 | bus_station (ferry berths too) | 211 |
| 0x07 | healthcareservice | 0 | | 0x26 | bicycle_rental | 182 |
| 0x08 | hospital_polyclinic | 188 | | 0x27 | areodrome | 7 |
| 0x09 | importanttouristattraction | 745 | | 0x28 | charging_station | 1 |
| 0x0a | market | 57 | | 0x29–0x35 | cable cars, lifts, skiing (cable_car … ski_school) | 8 |
| 0x0b | petrolstation | 1958 | | 0x36 | shelter | 2583 |
| 0x0c | pharmacy | 382 | | 0x37–0x44 | smallcraftfacility_* (marina facilities) | 321 |
| 0x0d | policestation | 96 | | 0x45 | harbour | 0 |
| 0x0e | restaurant | 10186 | | 0x46 | landmark | 0 |
| 0x0f | scenic_panoramicview | 1015 | | 0x47 | conti_services | 0 |
| 0x10 | shop | 2724 | | 0x48 | landmark (monuments, historic sites) | 27881 |
| 0x11 | shoppingcenter (supermarkets) | 2869 | | 0x49 | museum | 917 |
| 0x12 | sportscenter | 1869 | | 0x4a | zoo | 54 |
| 0x13 | touristinformationoffice | 2067 | | | | |
| 0x14 | camping_ground | 1234 | | | | |
| 0x15 | hotel_motel | 1021 | | | | |
| 0x16 | sla (here: bike shops) | 33 | | | | |
| 0x17 | sla_extern | 0 | | | | |
| 0x18 | bike_tyres | 0 | | | | |
| 0x19 | bus_stop | 9376 | | | | |
| 0x1a | tram_stop | 35 | | | | |
| 0x1b | subway_entrance (metro) | 31 | | | | |

How OSM tags map onto the types is not in the file; it was worked out by matching against
OSM, see "Building it from OSM".

### Attribute string

Same format as in osmpoint: `NNvalue|NNvalue|…` with a two-digit code, UTF-16LE, with a
trailing NUL.

| Code | OSM tag (derived) | Example |
|---|---|---|
| `00` | address `addr:street addr:housenumber` `\n` `addr:postcode addr:city` | `00Langgade 35\n7550 Sørvad` |
| `01` | `website`, else `url`, else `contact:website` | `01https://www.bat.dk/` |
| `02` | `phone`, else `contact:phone` | `02+45 72 54 30 00` |
| `03` | `opening_hours`, `;` replaced by a line break | `03Mo-Su 08:00-21:00`, `0324/7` |
| `04` | `internet_access` | `04wlan` |
| `05` | `smoking` | `05outside` |
| `06` | `wheelchair` | `06limited` |
| `07` | `beds` | |
| `08` | `rooms` | `0835` |
| `10` | `capacity` | `1070` |
| `09` | `cuisine` | `09pizza` |
| `11` | `description` | `11Gravhøj` |
| `16` | `fax` | |
| `17` | `email`, else `contact:email` | `17info@dragsholm-slot.dk` |
| `18` | `operator` | `18Sydbus`, `18Danske Bank` |
| `74` | `access` | `74private` |

## Rebuilding it

```python
import sys; sys.path.insert(0, "tools")
from layers import iter_records, parse_osmpoi, build_osmpoi, to_latlon, POI_TYPES

for tx, ty, cx, cy, grid, raw in iter_records(open(PATH, "rb").read(), "D"):
    for p in parse_osmpoi(raw):          # {"type", "pos", "name", "attrs"}
        lat, lon = to_latlon(cx, cy, grid, p["pos"])
```

Within a record the original is not sorted by type (only 806 of 3704 records happen to be
sorted).

## Building it from OSM

One command from the extract to the chart file; `--country=N` defaults to 4 = Denmark:

```bash
teasi osmpoi osm_ref/denmark-latest.osm.pbf osm_ref/denmark.poly \
    build/<dir>/Denmark_osmpoi.v20210915 [YYYYMMDD]                 # 5.2 s
teasi osmpoint …                                                    # the same arguments
```

- `poi.rs` keeps every tagged node, way and multipolygon that `poi_type` accepts or that
  carries `seamark:type` (osmpoint), each with a centre (the node, the mean of the corners,
  the centre of the bounding box, the centroid).
- `osmpoi.rs` assigns the types, builds the attributes, removes duplicates, groups by 32×32
  sub-cell and writes the file with `writer.rs`. The date (header `0x44`) defaults to today.
- Both layers read the same candidates, so one pass over the PBF serves either; for Great
  Britain that is 35 and 17 s (`rust/README.md`, stage 3).
- `denmark.poly` is the Geofabrik boundary
  (`download.geofabrik.de/europe/denmark.poly`). Only POIs inside it are taken, because the
  extract also contains ways and relations reaching far abroad (points at 51° N, 1° E, for
  instance). **The Faroe Islands are missing** from the Geofabrik extract for Denmark (they
  have their own extract, `faroe-islands`); the original includes them (735 POIs in the
  tiles (122,19), (123,19), (123,20)).

**Calibration against the original.** The rules come from matching the 79,127 Danish
original POIs against `denmark-220101` (3.5 months younger than the original): 84 % sit
exactly on an OSM node or on the mean of a way's corners, and 93 % in total could be
assigned to an object (by name as well). Built with the rules below and compared with the
original: of 65,205 POIs at an identical position, 99.8 % have the same type and 98 % the
same attribute string. Nearly all the differences are OSM changes after September 2021
(among them an import by the Fødevarestyrelsen with `fvst:*` tags that added ~800 bakeries,
butchers and snack bars).

**Type rules** (first match wins, `RULES` in `osmpoi.rs`):

| OSM | Type |
|---|---|
| `amenity=place_of_worship` by `religion` (christian, muslim, buddhist, hindu, shinto, taoist, sikh, jewish) | 0x1d–0x24 |
| `seamark:type=small_craft_facility` by `seamark:small_craft_facility:category`, a **single** value only (`toilets;showers` is dropped) | 0x37–0x44 |
| `amenity=fuel` · `bicycle_rental` · `shop=bicycle` | 0x0b · 0x26 · 0x01 |
| `amenity=bank/atm/pharmacy/doctors/hospital/police/marketplace/bus_station` | 0x00/03/0c/05/08/0d/0a/25 |
| `emergency=defibrillator` | 0x06 |
| `amenity=cafe/bar/pub` → 0x02, `restaurant/fast_food` → 0x0e, `shelter` → 0x36 | |
| `tourism=museum/gallery` → 0x49, `zoo/aquarium` → 0x4a, `attraction` → 0x09, `viewpoint` → 0x0f, `information` → 0x13, `camp_site` → 0x14, `hotel/hostel/motel` → 0x15, `alpine_hut` → 0x33 | |
| `shop=supermarket` → 0x11, `department_store` → 0x04, `convenience/bakery/butcher/kiosk` → 0x10 | |
| `leisure=sports_centre/stadium` → 0x12 | |
| `railway=station/halt` → 0x1c, `tram_stop` → 0x1a, `subway_entrance` → 0x1b, `highway=bus_stop` → 0x19 | |
| `man_made=tower` with `tower:type=observation`, or `leisure=bird_hide` | 0x0f |
| `amenity=charging_station` with `bicycle=yes` | 0x28 |
| `aeroway=aerodrome`, as a node only (the original has just 7 small airfields) | 0x27 |
| `aerialway=*` (lifts) | 0x29–0x31 |
| `historic=*` except railway, hollow_way, bunker, wall, aircraft, no | 0x48 |

Not taken are `tourism=guest_house/chalet/caravan_site`, `amenity=toilets`,
`charging_station` for cars and other `shop=*` values, among others. A POI does not need a
name. Type 0x16 (`sla`, 33 bike shops in the original, apparently a partner list) is not
produced; those shops become 0x01.

**Position:** nodes directly. For areas the compiler takes the centroid. Which method the
original uses is open: for small areas the centroid, the centre of the bounding box and the
mean of the corners all agree exactly, while for large ones (churches, say) the original is
2 m off in the median, with no preferred direction.

**Duplicates:** POIs of the same type and name less than 500 units apart (~75 m N-S,
~40 m E-W) are merged into one (nodes before areas, then by OSM id). This mostly affects
bus stops, which OSM records per side of the road. In the original, excluded stops are
almost all < 500 units from their nearest namesake and the ones that were kept almost all
> 500.

## Open questions

- The position rule for large areas (see above) and the selection rule for tram stops
  (original 35, compiler 80).
- Whether the firmware treats empty sub-cells (slot `0xFFFFFFFF`) differently from records
  without POIs. The original has no empty records.
