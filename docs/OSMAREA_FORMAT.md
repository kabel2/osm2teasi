# osmarea-Format (`Denmark_osmarea.v20210810`)

Stand: 2026-09-18. **Container und Semantik verstanden, Compiler fertig** (Abschnitt „Aus OSM
erzeugen“, Laufzeit ~4 min). **Container vollständig verstanden**: `parse_c`/`build_c` in
`tools/layers.py` bauen alle 368 Records bytegleich nach. Die Flächenklassen sind per Abgleich
mit OSM zugeordnet (`denmark-220101.osm.pbf`).

Inhalt: **Flächen** (Landnutzung, Wald, Wasser, Siedlungen …) als Polygone, dazu Flächen-
umrisse und die Füllung für offenes Wasser (`#OW`). Dänemark: 21.588 + 6524 + 3339 Objekte in
368 Records.

Hülle, Verschlüsselung, Slot-Bereiche und Koordinatensystem: siehe
[KARTEN_ENTSCHLUESSELUNG.md](KARTEN_ENTSCHLUESSELUNG.md), Abschnitte 1, 1a und 3.
Das Geometrieformat steht in [OSM_FORMAT.md](OSM_FORMAT.md) („Geometrie“).

## Einordnung

| | |
|---|---|
| Header `0x50` | `4` (Layer-Kennung) |
| Slot-Bereich | **C** (Tile-Kopf `+0x140`, 4×4 Unterzellen) |
| Leser in der Firmware | `FUN_003e651c` (interner Typ 2) |
| 1 Record | = alle Flächen **einer 4×4-Unterzelle** (0,3515625°), Koordinaten-Einheit 360°/2²⁵ |
| Koordinaten | **ohne** Rand (anders als osm/ta), die Punkte sind exakt OSM-Knoten (gerundet auf 360°/2²⁵) |
| Besonderheit | Das erste Verzeichnis-Tile ist ein Platzhalter `(0,0)` mit Größe 0 |

## C-Record (allgemeiner Aufbau)

Denselben Container nutzen auch die C-Records von osm (siehe [OSM_FORMAT.md](OSM_FORMAT.md)).

```
0x00  15 × u32  Kopf
        [0]  Größe eines Arrays (überschrieben mit cell_x, s. u.)
        [1]  cell_y (überschrieben)          [2]  0
        [3]  n c1 (0x10 B)   [4]  0 (Zeiger)
        [5]  n c2 (0x08 B)   [6]  0
        [7]  n c3 (0x18 B)   [8]  0
        [9]  n c4 (0x0C B)   [10] 0
        [11] n c5 (0x1C B)   [12] 0
        [13] n c6 (0x18 B)   [14] 0
0x3C  c1 … c6 hintereinander, jedes Array BYTEWEISE TRANSPONIERT
      danach die variablen Teile, Array für Array, darin Element für Element:
        c1: [2] = Anzahl u32
        c2: [0] = Anzahl u32
        c3: [1] = Anzahl u16 (String), dann [4] = Anzahl u32
        c4: [1] = Anzahl u32
        c5: [1] = Anzahl u16 (String), dann [5] = Anzahl u32
        c6: [1] = Anzahl u16 (String), dann [4] = Anzahl u32
```

Die Zeigerfelder (je das Feld hinter der Anzahl bzw. `[0]` vor der String-Länge) sind in der
Datei 0. Es gibt kein Padding.

## osmarea: Arrays c3, c5, c6

In osmarea sind c1, c2 und c4 leer. Belegungsmuster: nur c6 (249 Records), c3 + c5 + c6 (106),
c5 + c6 (13).

### c5: Fläche mit Klasse (0x1C B = 7 × u32), 21.588 Stück

| Feld | Inhalt |
|---|---|
| `[0]`/`[1]` | Name (Zeiger/Länge), in DK immer leer |
| `[2]` | **Flächenklasse** 0–17 |
| `[3]` | Bounding-Box Minimum, gepackt `(v << 16) \| u` |
| `[4]` | Bounding-Box Maximum |
| `[5]` | Anzahl u32 der Geometrie |
| `[6]` | 0 (Zeiger) |

Zuordnung per Abgleich (OSM-Way mit den meisten gemeinsamen Punkten; Multipolygone aus
ungetaggten Ways fallen unter „nicht zugeordnet“):

| Klasse | Anzahl | OSM (Anteil) |
|---:|---:|---|
| 0 | 362 | Sonstiges: `landuse=plant_nursery` 30 %, `brownfield` 27 %, `animal_keeping` … |
| 1 | 412 | `landuse=commercial` 60 %, `retail` 32 % |
| 2 | 411 | `landuse=military` (39 %, Rest nicht zugeordnet) |
| 3 | 1289 | `landuse=cemetery` 93 % |
| 4 | 1751 | `landuse=industrial` 65 %, `quarry` 13 %, `construction` 8 % |
| 5 | 2724 | Freizeit: `leisure=park/pitch/playground/golf_course`, `landuse=recreation_ground` |
| 6 | 3836 | `landuse=forest` 73 % (auch `natural=wood`, `heath`) |
| 7 | 3646 | `landuse=meadow` 61 %, `grass` 19 % |
| 8 | 3689 | `landuse=farmland` 51 %, `farmyard` 30 % |
| 9 | 726 | `natural=beach` 63 % |
| 10 | 1953 | `natural=wetland` 78 % |
| 11 | 520 | `man_made=pier` als Fläche und kleine Inseln (`place=islet` an geschlossener Küstenlinie), über das Meer gezeichnet |
| 16 | 35 | `man_made=groyne` (Buhne) 80 % |
| 17 | 234 | `man_made=breakwater` (Wellenbrecher) 67 % |

### c6: Fläche ohne Klasse (0x18 B = 6 × u32), 6524 Stück

| Feld | Inhalt |
|---|---|
| `[0]`/`[1]` | Name, meist leer. `#OW` (319×, eines pro Zelle) = **Meer**: Zelle minus Land, Inseln als Löcher; bei reinen Meereszellen ein Quadrat über die ganze Zelle (Bounding-Box 0 … `0x80008000`) |
| `[2]`, `[3]` | Bounding-Box Min/Max |
| `[4]` | Anzahl u32 der Geometrie |
| `[5]` | 0 (Zeiger) |

Laut Abgleich **Wasserflächen** (`natural=water` 54 %, der Rest sind überwiegend
Multipolygone ohne eigene Tags am Way), dazu `#OW` für offenes Meer.

### c3: Umriss (0x18 B = 6 × u32), 3339 Stück

Gleiches Feldlayout wie c6, Name immer leer. Laut Abgleich zu 92 % **`landuse=residential`**,
also Siedlungsflächen (nicht Umrisse, wie früher vermutet).

### Objekte, Blöcke und Geometrie

**Ein Objekt = alle Flächen einer Klasse in einem 4096er-Block.** Jede Zelle ist in 8×8 Blöcke
zu 4096 Einheiten geteilt. Die Flächen werden an den Blockgrenzen zugeschnitten, und alle
Ringe einer Klasse in einem Block stehen als Teile in einem Objekt (c5: pro Klasse, c3/c6:
Siedlung/Wasser). Ausnahme: `#OW` ist ein Objekt pro Zelle und nicht in Blöcke geteilt.

Geometrie: Folge von Ringen `[u32 (hi << 16) | n][n Punkte]` (siehe OSM_FORMAT.md), jeder Ring
geschlossen (erster = letzter Punkt). Außenringe haben positive, Löcher negative Fläche
(Shoelace in (u, v)); die Firmware füllt nach der Gerade-Ungerade-Regel pro Objekt
(`FUN_003df11c`, Scanline mit Kantenliste).

**`hi` = Detailstufe:** Die Zeichenroutine überspringt einen Ring, wenn `hi` größer als die
aktuelle Detailstufe ist (`*(param_1 + 0x34) < hi`). Kleine Flächen haben also 14, große 9.
Die Regel (per Abgleich, 96,5 % Treffer): kürzere Seite `m` der Bounding-Box der **ganzen**
(verschmolzenen) Fläche vor dem Zuschnitt, `hi = 14 − k` für das größte `k ≤ 5` mit
`m ≥ 33 · 2^k` Einheiten. Zugeschnittene Stücke erben `hi` der ganzen Fläche, Löcher bekommen
`hi` nach ihrer eigenen Bounding-Box. `#OW`-Ringe haben immer 9.

Die Bounding-Box im Struct entspricht bei ~92 % der Objekte den Min/Max-Werten der Punkte, bei
den übrigen ist sie etwas größer (unkritisch).

## Nachbauen

```python
import sys; sys.path.insert(0, "tools")
import layers as L
for tx, ty, cx, cy, g, raw in L.iter_records(open(PFAD, "rb").read(), "C"):
    rec = L.parse_c(raw)
    for f in rec["c5"]:
        klasse = f["s"][2]
        for level, pts in L.geometry_parts(f["v"][1]):
            ring = [L.to_latlon(cx, cy, g, p) for p in pts]
    assert L.build_c(rec) == raw
```

Kopf `[0]` im Original: bei Records mit nur einem Array dessen Größe (249×), sonst meist die
Größe von c5 (117×), selten von c6 (2×). Die Firmware überschreibt das Feld, der Wert ist also
unkritisch.

## Aus OSM erzeugen

```bash
.venv/bin/python tools/osm_area_extract.py osm_ref/denmark-latest.osm.pbf build/ref/area_latest.pkl   # ~1 min
.venv/bin/python tools/compile_osmarea.py build/ref/area_latest.pkl osm_ref/denmark.poly \
    2013021200000368/7/943/20317/Denmark_osmarea.v20210810 build/<ordner>/Denmark_osmarea.v20210810 [JJJJMMTT]  # ~4 min
```

**Länder ohne Originaldatei** (z. B. Großbritannien): statt des Originals `-`, das Meer kommt
dann vollständig aus den weltweiten Landpolygonen von osmdata.openstreetmap.de
(`land-polygons-split-4326`, aus `natural=coastline` der ganzen Welt), zugeschnitten mit
`tools/land_extract.py` (braucht `pyshp`). Meer = Zelle minus Land in jedem Tile, das die
Grenze berührt, also auch mit den Küsten der Nachbarländer (z. B. Frankreich bei Dover).
Details und Laufzeiten: [KARTEN_ENTSCHLUESSELUNG.md](KARTEN_ENTSCHLUESSELUNG.md) 5.7.

```bash
.venv/bin/python tools/land_extract.py osm_ref/land-polygons-split-4326/land_polygons.shp \
    osm_ref/great-britain.poly build/gb/land.pkl                                 # ~5 s
.venv/bin/python tools/compile_osmarea.py --country=17 --land=build/gb/land.pkl \
    build/gb/area.pkl osm_ref/great-britain.poly - build/gb/GreatBritain_osmarea.v20260918 20260918
```

`osm_area_extract.py` speichert alle Flächen (geschlossene Ways und Multipolygone, per osmium
zusammengesetzt) mit Landnutzungs-Tags sowie die Küstenlinien-Ways. `compile_osmarea.py` baut
daraus den Layer (Abgleich des Originals mit `denmark-220101`, Regeln unten):

1. **Klasse** nach Tags, `landuse` vor `natural`, `leisure`, `man_made` (`area_class`):

   | Ziel | OSM |
   |---|---|
   | c3 | `landuse=residential` |
   | c5 0 | `landuse=plant_nursery, brownfield, animal_keeping, fishfarm, garages, harbour, religious, scout_camp` |
   | c5 1 / 2 / 3 | `landuse=commercial, retail` / `military` / `cemetery` |
   | c5 4 | `landuse=industrial, quarry, construction, railway, landfill` |
   | c5 5 | `leisure=park, pitch, playground, garden, sports_centre, golf_course, track, marina …`, `landuse=recreation_ground` (**ohne** `nature_reserve`: große Schutzgebiete fehlen im Original) |
   | c5 6 | `landuse=forest, scrub`, `natural=heath` (**nicht** `natural=wood/scrub/grassland`) |
   | c5 7 | `landuse=meadow, grass, greenfield, village_green, greenhouse_horticulture` |
   | c5 8 | `landuse=farmland, farmyard, allotments, orchard, vineyard` |
   | c5 9 / 10 | `natural=beach` / `natural=wetland` |
   | c5 11 / 16 / 17 | `man_made=pier` und Inseln (`place=islet`) / `groyne` / `breakwater` |
   | c6 | `natural=water` (außer `water=river`), `landuse=basin, reservoir, aquaculture`, `waterway=dock` |

2. OSM-Flächen unter **150 Einheiten²** (~120 m²) fallen weg (Übernahmequote springt dort).
3. Alle Flächen einer Klasse werden **verschmolzen**: Nachbarparzellen mit gemeinsamer Kante
   sind im Original ein Ring (daher teilen nur ~70 % der Äcker ihre Knoten mit dem Original).
4. `hi` je verschmolzener Fläche (s. o.), dann **Douglas-Peucker mit 4 Einheiten** (weggelassene
   OSM-Knoten liegen im Original höchstens 4 Einheiten neben dem Ring), Zuschnitt auf die
   Geofabrik-Grenze und die 4096er-Blöcke.
5. **Meer:** Das Original nutzt eine weltweite Küstenlinie (Schweden, Norwegen, Deutschland
   inklusive). Innerhalb der Grenze baut der Compiler das Meer neu aus `natural=coastline`
   (Ketten zusammensetzen, Land links; offene Ketten enden weit außerhalb und werden gerade
   geschlossen), außerhalb übernimmt er `#OW` aus der Originaldatei.
6. Tiles ohne Zelle innerhalb der Grenze (die **Färöer**) werden unverändert aus dem Original
   kopiert. Wie im Original stehen alle 16 Zellen jedes Tiles in der Datei (leere entfallen).

Ergebnis gegen das Original (beide aus `denmark-220101` gebaut, Flächensumme je Klasse):
Siedlung, Industrie, Acker, Wiese, Friedhof, Strand, Handel ±8 %, Meer 100 %, Wald +13 %,
Feuchtgebiet +32 % (vermutlich OSM-Änderungen), Binnengewässer −12 % (im Original sind
außerdem Gewässer jenseits der Grenze enthalten). Aus `denmark-latest` (2026-09-18):
700.620 Flächen, 26,4 MB, auf dem Gerät installiert.

## Offen

1. Klassen 12–15 (in DK nicht belegt).
2. Genaue Zeichenreihenfolge c3/c5/c6 und die Reihenfolge der c5-Objekte (Compiler: grob wie
   im Original, `CLASS_ORDER`).
3. Warum ~8 % der Bounding-Boxen größer als die Punkte sind.
4. Der Compiler braucht ~20 min (Verschmelzen der großen Klassen, Einlesen des Original-Meers).
