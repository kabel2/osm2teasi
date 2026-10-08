# osm-Format (`Denmark_osm.v20210916`)

Stand: 2026-09-18. **Container vollständig verstanden**: `tools/layers.py` baut alle 5878
Records bytegleich nach (A 161, B 406, C 105, D 5206). Klassen, Flags und Linientypen sind per
Abgleich mit OSM zugeordnet (siehe unten). Offen sind nur noch einzelne Felder, siehe „Offen“.
**Compiler:** `tools/compile_osm.py` baut den Layer aus OSM, siehe „Aus OSM erzeugen“.

Inhalt: das **Straßen- und Wegenetz aus OSM** (Geometrie, Klassen, Namen, Längen, Knoten fürs
Routing), dazu Flüsse und weitere Linien, ein Straßennamen- und Ortsverzeichnis für die
Adresssuche und der unverschlüsselte Routing-Graph.

Hülle, Verschlüsselung, Slot-Bereiche und Koordinatensystem: siehe
[KARTEN_ENTSCHLUESSELUNG.md](KARTEN_ENTSCHLUESSELUNG.md), Abschnitte 1, 1a und 3.
Den allgemeinen D-Container beschreibt [OSMPOI_FORMAT.md](OSMPOI_FORMAT.md).

## Einordnung

| Slot-Bereich | Raster | Records | Leser | Inhalt |
|---|---|---:|---|---|
| **D** | 32×32 | 5206 | `FUN_003e56fc` | Straßen/Wege (a1), Polygone (a2), Linien (a3, a4) |
| **A** | 4×4 | 161 | `FUN_003e6044` | Orts- und Straßennamenverzeichnis |
| **B** | 8×8 | 406 | `FUN_003e5d20` | Routing-Graph (1,69 Mio. Knoten), **nur LZMA, nicht verschlüsselt** |
| **C** | 4×4 | 105 | `FUN_003e651c` | Linien und Polygone für kleine Zoomstufen |

### Koordinaten in D- und C-Records: Rand von 512

Anders als bei osmpoi, osmpoint und osmarea haben die Linien in osm (D und C) und ta einen
**Rand von 512 Einheiten** um die Unterzelle: Gespeichert wird `u = x − x_Zelle + 512`, der
Wertebereich 0 … 33.792 entspricht also −512 … 33.280. Gerundet wird auf die nächste ganze
Einheit. Nach Abzug des Rands liegen die Stützpunkte **exakt** auf OSM-Knoten (Restfehler
±0,5 Einheiten), geprüft an Tausenden Punkten. Ohne den Abzug liegen alle Punkte rund 70 m
(D) bzw. 600 m (C) nordwestlich daneben. `to_latlon(cx, cy, g, p, margin=L.MARGIN)`.

Header `0x50` = `1` (Layer-Kennung).

## Geometrie (gilt für alle Layer)

Eine Geometrie ist ein u32-Array aus einem oder mehreren **Teilen**:

```
u32 (hi << 16) | n     n = Anzahl Punkte des Teils
n × u32                Punkte, gepackt (v << 16) | u, relativ zur NW-Ecke der Unterzelle
... nächster Teil bis zum Ende des Arrays
```

Bei Linien (osm, ta) ist `hi == n`. Bei Flächen (osmarea) ist `hi` eine Detailstufe (9–14),
siehe [OSMAREA_FORMAT.md](OSMAREA_FORMAT.md). Die Aufteilung in Teile geht bei allen geprüften
~520.000 Geometrien exakt auf (osmarea vollständig, osm-D jeder 5. Record;
`geometry_parts()` in `tools/layers.py`). In dieser Stichprobe bestehen nur 483 von 474.575
Straßen aus mehr als einem Teil.

Straßen werden am **Rand** der Unterzelle (Zelle ± 512) geschnitten: Eine Kante nahe der
Zellgrenze steht also in beiden Zellen, jeweils bis 512 Einheiten über die Grenze hinaus. Der
Schnittpunkt wird interpoliert und gerundet, Knoten-IDs und Länge bleiben die der ganzen Kante.
Verlässt eine Linie den Bereich und kommt wieder hinein, entstehen mehrere Teile.

## D-Records: Straßennetz

Genutzte Arrays (Dänemark, alle 5206 Records): a1 = 2.339.000, a3 = 31.734, a4 = 37.978,
a2 = 24, a5 = 0.

### a1: Straßenkante (0x28 B = 10 × u32)

| Feld | Inhalt |
|---|---|
| `[0]` | **Name** = Byte-Offset in die Namens-Indextabelle des A-Records der 4×4-Elternzelle (`[0] // 4` = Index), `0xFFFFFFFF` = namenlos |
| `[1]` | in osm immer = `[0]` (in ta: Name der anderen Straßenseite) |
| `[2]`, `[3]` | in osm immer `0xFFFF7FFF` (in ta: Hausnummernbereiche) |
| `[4]` | Knoten-ID am Anfang |
| `[5]` | Knoten-ID am Ende |
| `[6]` | Flags (Bitfeld, siehe unten) |
| `[7]` | Bits 27–31: **Straßenklasse** 0–15 · Bits 24–26: immer 0 · Bits 0–23: **Länge** |
| `[8]` | Anzahl u32 der Geometrie |
| `[9]` | 0 (Zeiger) |

**Länge:** `[7] & 0xFFFFFF` ≈ Meter × 4,1906, also eine Einheit ≈ 0,2386 m (entspricht
Erdumfang / (5·2²⁵)). Geprüft an ~140.000 Kanten > 30 m (Median je Klasse 4,186–4,193) und
~15.600 geraden Kanten > 200 m (Median 4,1906 bei 55°–58° N, in Nord-Süd- wie Ost-West-Richtung). Ausnahme sind einzelne Ausreißer (vermutlich Fähren, siehe Offen).

**Knoten-IDs:** `[4]`/`[5]` sind **Indizes in die Knotenliste des B-Records** (Routing-Graph)
der 8×8-Zelle, **in der der Knoten liegt** (meist `(cx // 4, cy // 4)`; bei Kanten über eine
8×8-Grenze die Nachbarzelle, das waren die scheinbar „ungültigen“ 0,4 %). Knoten sind die
OSM-Knoten an Kreuzungen (von mehreren Straßen-Ways genutzt) und an Way-Enden; dazwischen liegt
genau eine Kante. **Autobahnen** (Klasse 0) und einzelne andere Kanten haben die IDs 0/0 und
sind nicht im Graphen.

**Reihenfolge:** Die a1-Einträge eines Records sind **aufsteigend nach Klasse** sortiert (in
allen Records), innerhalb der Klasse meist nach Name.

**Abgleich mit OSM** (Geofabrik-Extrakt `denmark-220101.osm.pbf`, `tools/osm_extract.py`):
Die Stützpunkte der Kanten sind **exakt OSM-Knoten** (nach Abzug des Rands von 512, siehe
„Koordinaten in D-Records“). 2.298.873 von 2.339.000 Kanten (98,3 %) lassen sich so einem
OSM-Way zuordnen, 2.092.000 davon mit allen Punkten. Die Kante läuft fast immer in
Way-Richtung (1.954.721 vorwärts, 5.122 rückwärts). Alle folgenden Tabellen beruhen auf diesem
Abgleich.

**Straßenklassen** `[7] >> 27`:

| Klasse | OSM | Anzahl (zugeordnet) |
|---:|---|---:|
| 0 | `highway=motorway`, `motorway_link` | 4.652 |
| 1 | `trunk`, `trunk_link` | 1.150 |
| 2 | `primary` | 20.157 |
| 3 | `secondary`, `primary_link` | 50.762 |
| 4 | `secondary_link` | 1.662 |
| 5 | in DK nicht belegt | 0 |
| 6 | `tertiary` (+ `tertiary_link`) | 211.332 |
| 7 | `service` 47 %, `residential` 38 %, `unclassified` 13 %, `living_street` | 1.196.120 |
| 8 | `pedestrian` | 3.086 |
| 9 | `route=ferry` (Fähre) | 1.749 |
| 10 | `cycleway` | 118.109 |
| 11 | `footway` | 212.693 |
| 12 | `track` | 107.152 |
| 13 | `path` (+ `bridleway`) | 153.113 |
| 14 | `steps` | 10.263 |
| 15 | 10 Kanten, keinem OSM-Way zugeordnet | – |

**Flags `[6]`** (Anteil gesetzt über alle zugeordneten Kanten; „P“ = Anteil in der OSM-Gruppe):

| Bit | Bedeutung | Beleg |
|---:|---|---|
| 0 | Kreisverkehr (`junction=roundabout`) | P = 99,9 % |
| 1 | Teil einer **lokalen Radroute** (`route=bicycle`, `network=lcn`) | 96 % |
| 2 | **regionale Radroute** (`rcn`) | 100 % |
| 3 | **nationale Radroute** (`ncn`) | 99 % |
| 4 | Zufahrt beschränkt: `access=private/no/destination/customers/forestry` | 76–100 % |
| 5 | `mtb:scale` vorhanden | 98–100 % |
| 6 | `highway=bridleway` (Reitweg) | 98,7 % |
| 7 | **befestigt**: `surface` ∈ asphalt, paved, paving_stones, concrete…, oder kein `surface` | 100 % bzw. < 5 % bei gravel, dirt, ground, sand, grass, unpaved … |
| 8 | **kein `surface`-Tag** | 99,8 % ohne, 6,6 % mit |
| 9–10 | **Fahrradzugang** (2 Bit): `00` = `bicycle=no`, `01` = nicht getaggt (Standard), `10` = `bicycle=yes`, `11` = `bicycle=designated` oder Radweg/Radspur an der Straße (`cycleway=*`) | je > 93 % |
| 11 | **Einbahn** in Geometrierichtung (`oneway=yes`, Kreisverkehr) | 97–100 % |
| 13–15 | **Fahrspuren** (`lanes`), 3-Bit-Zahl 1–7 | `lanes=2` → 2 (97 %), `lanes=4` → 4 … |
| 18 | Teil einer **Wanderroute** (`route=hiking/foot`) | 95–100 % |
| 20–23 | **Kostenfaktor-Index** 0–15: Der Router (`FUN_003cf4d8`) multipliziert die Kantenkosten mit `Tabelle[i]/128`, Tabelle bei VA `0x4fc040`: 128, 123, 118 … 68, 67, 66, 64 (Index 0 = neutral, höhere = günstiger). Nur 2,3 % der Kanten ≠ 0, vor allem kurze Stücke an größeren Straßen, unabhängig von Radrouten und Wegtyp; aus OSM nicht ableitbar (vermutlich Tahuna-Daten) | Firmware |
| 27 | Teil einer **EuroVelo-Route** (`icn`) | 99 % (EV12, EV3, EV10); EV7 fehlt, gab es 2021 wohl noch nicht |
| 28 | nur zusammen mit Bit 27 (30–57 % der icn-Kanten); von der Router-Kostenfunktion nicht gelesen | |
| 29 | Radweg/Radspur **an der Straße** (`cycleway=track/lane/shared_lane/separate`) | 85–100 % |
| 30 | `bicycle=dismount` (Schieben) | 96,5 % |
| 31 | Fußgänger ausdrücklich erlaubt (`foot=yes/designated`) | 98–99 % |

Bits 12, 16, 17, 19 und 24–26 sind (fast) nie gesetzt. Einzelne Bits wirken auch über andere
Tags (z. B. setzt `oneway=-1` Bit 11 nicht). Für einen Compiler reichen die Regeln der
Tabelle.

### a2: Polygon (8 B = 2 × u32)

`[0]` = Anzahl u32, `[1]` = Zeiger. Variabler Teil: `u32 n` (Punktzahl), dann `n` Punkte.
Sehr selten (24 in DK). In der Stichprobe gehören 6 von 9 zu Verwaltungsgrenzen (`boundary=administrative`).
**Abweichend** von allen anderen Geometrien gibt es hier keinen `hi`-Anteil und keine Teile.

### a3: unbenannte Linie (0x0C B = 3 × u32)

`[0]` = Typ, `[1]` = Anzahl u32 der Geometrie, `[2]` = Zeiger.

| Typ | Anzahl DK | OSM (Abgleich über jeden 3. Record) |
|---:|---:|---|
| 0 | 10.630 | `railway=rail`, `light_rail`, `subway`, `narrow_gauge`, `disused`, `tram`, `preserved`, `miniature` |
| 1, 2, 3 | 42, 15, 75 | nur Ways ohne die ausgewerteten Tags zugeordnet (Bedeutung offen, evtl. Grenzen) |
| 4 | 12.170 | `man_made=pier` (Steg) |
| 5 | 1.465 | `man_made=groyne` (Buhne) |
| 6 | 643 | `man_made=breakwater` (Wellenbrecher) |
| 10 | 6.694 | `power=line` (Hochspannungsleitung) |

### a4: benannte Linie (0x14 B = 5 × u32)

`[0]` = Zeiger Name, `[1]` = Länge Name (UTF-16 inkl. NUL), `[2]` = Typ, `[3]` = Anzahl u32
der Geometrie, `[4]` = Zeiger. Variabel: erst der Name, dann die Geometrie.

| Typ | Anzahl | Inhalt |
|---:|---:|---|
| 7 | 37.601 | Gewässer: `waterway=stream/river/canal` immer, `ditch/drain` nur mit Namen; Name leer, einfach oder mehrsprachig: `[DANVidå¦GERWiedau]` |
| 8 | 351 | `seamark:type=navigation_line`, Name `<orientation>(leading)` |
| 9 | 26 | `seamark:type=recommended_track`, Name `<orientation>(fixed_marks)` |

Mehrsprachige Namen: `[` + mehrere `<3-Buchstaben-Sprachcode><Name>` getrennt durch `¦`
(U+00A6) + `]`.

## A-Records: Namens- und Ortsverzeichnis

Leser `FUN_003e6044`. **Nicht transponiert.**

```
0x00  7 × u32   Kopf: [0],[1] = Zelle (überschrieben), [3] = n Einträge,
                [5] = Offset blk5, [6] = Offset blk6 (relativ zum Record-Anfang)
0x1C  n × 0x1C  Einträge (7 × u32): [1] Länge Name, [3] Anzahl 0x18-Unterelemente,
                [5] Anzahl 0x10-Unterelemente, [0]/[4]/[6] Zeiger
      danach:   alle Namen, alle 0x18-Unterelemente, alle 0x10-Unterelemente,
                Strings der 0x18-Unterelemente ([1] = Länge), Strings der 0x10-Unterelemente,
                Ausrichtung auf 4 Byte, u32-Arrays der 0x10-Unterelemente ([2] = Anzahl)
blk5  u32-Liste: Bits 0–23 = Offset in blk6, Bits 24–31 = Wortzahl des Namens
blk6  Namen: u16 Länge + UTF-16-Zeichen (ohne NUL), weitgehend alphabetisch sortiert
```

In Dänemark hat jeder A-Record genau einen Eintrag („Denmark“). In osm hat er ein einziges
0x18-Unterelement ohne String (Felder `[0, 0, 0, n, 0, 0]`) und keine 0x10-Unterelemente. Die
eigentlichen Daten stehen in blk5/blk6. Im ta-Layer enthalten die 0x18-Unterelemente dagegen
Orte mit Koordinaten, siehe [TA_FORMAT.md](TA_FORMAT.md).

**Straßennamen:** `blk5`/`blk6` bilden die Namenstabelle, auf die `a1[0]` der D-Records zeigt.
`blk6` enthält jedes **Wort** einmal (in der Reihenfolge des ersten Auftretens), `blk5` die
**Namen** (= OSM-Tag `name`, kein `ref`-Ersatz), sortiert ohne Groß-/Kleinschreibung und
Akzente (Å wie A, Æ wie AE). Namen aus mehreren Wörtern (an Leerzeichen getrennt) stehen als
aufeinanderfolgende Einträge: Der erste trägt die Wortzahl im oberen Byte, die folgenden haben 0.
Das 0x18-Unterelement enthält in `[3]` die Anzahl der Namen, der Eintrag selbst `[2] = 2`.
Beispiele:

```
(3, 'Aarhus') (0, 'Syd') (0, 'Motorvejen')      -> "Aarhus Syd Motorvejen"
(2, 'Søndre') (0, 'Ringgade')                   -> "Søndre Ringgade"
(1, 'Skanderborgvej')
```

Geprüft in Aarhus: Skanderborgvej, Silkeborgvej, Viborgvej, Vesterbrogade und Marselis
Boulevard liegen an den richtigen Stellen. Die D-Zelle `(cx, cy)` im 32er-Raster nutzt den
A-Record der Zelle `(cx // 8, cy // 8)` im 4er-Raster.

## B-Records: Routing-Graph (unverschlüsselt)

Leser `FUN_003e5d20`, 8×8-Raster. Er wird aus demselben Codebereich (`0x3ce…–0x3d6…`) aufgerufen
wie der D-Leser, vermutlich also von der Routenberechnung. **Kein PC1**, keine Transposition,
keine Strings. Es ist **kein grobes Netz**, sondern der vollständige Graph aller D-Kanten:
1.692.053 Knoten in 406 Zellen, jede D-Kante erscheint als Kante (in beiden Richtungen).

```
0x00  11 × u32  Kopf: [3] = n Knoten, [5] = n Kanten, [7] = n3 (in DK immer 0), [9] = n Zusatz
0x2C  n Knoten  × 12 B
      n Kanten  × 16 B
      n3        × 16 B
      n Zusatz  × 12 B
```

**Knoten** (12 B). Knoten 0 ist ein leerer Platzhalter, der Index ist die Knoten-ID aus
`a1[4]`/`a1[5]`.

| Feld | Inhalt |
|---|---|
| `[0]` | Bits 0–18 = Index der ersten Kante, Bit 19 = hat Zusatzeintrag, Bits 26–31 = Anzahl der Kanten (Adjazenzliste) |
| `[1]` | Position `(v << 16) \| u` mit `u = floor(x · 65535/65536)`, x = Position in der 8×8-Zelle in 360°/2²⁷ (**die Zelle wird auf 0…65535 abgebildet**, 98–99 % exakt); kein Rand (`layers.b_node_latlon`) |
| `[2]` | **Linksabbiegen** (Kreuzen des Gegenverkehrs): Liste von 6-Bit-Einträgen `von \| nach << 3` (Indizes in die Kantenliste des Knotens: ankommend über Kante `von`, weiter über Kante `nach`), 0 beendet die Liste. Der Router schlägt dafür 90.000 auf (`FUN_003cf4d8`, Schleife über `node[2] >> 6k & 0x3f`). Belegt nur an Knoten mit **mindestens drei Kanten der Klassen ≤ 7** und nur zwischen diesen; Linksabbiegen = Richtungsänderung > 35° nach links (erstes Segment, x mit cos(Breite)). Höchstens 5 Einträge, sonst 0 |

**Kanten** (16 B), je ausgehender Richtung eine:

| Feld | Inhalt |
|---|---|
| `[0]` | Bits 0–19 = **Länge** (wie `a1[7]`), Bit 20 = 0, Bits 21–24 = **Straßenklasse** (wie `a1[7] >> 27`), Bits 25–29 = **Wegkategorie** (s. u.), Bit 30 = **in dieser Richtung befahrbar** (0 bei Einbahn gegen die Richtung), Bit 31 = 1 |
| `[1]` | **Flags**, identisch mit `a1[6]` (99,9 %) |
| `[2]` | **Anstieg in cm** in Fahrtrichtung (Summe der Höhengewinne); Router-Kosten `Gewicht₁·Länge + Gewicht₂·[2]`. Aus `[2](u→v) − [2](v→u) = h(v) − h(u)` lassen sich Knotenhöhen zurückrechnen, 97,5 % der Kanten passen auf 50 cm (`tools/osm_heights.py`) |
| `[3]` | **Zielknoten**: Bits 25–31 = dx + 64, Bits 18–24 = dy + 64 (Versatz der 8×8-Zelle des Ziels), Bits 0–17 = Knotenindex dort. `0x81…` = gleiche Zelle |

Wegkategorie (Bits 25–29, abgeglichen mit OSM): 18 = trunk/primary/secondary (+ Auffahrten),
10 = tertiary/residential/unclassified/living_street, 6 = `service` und Fähre,
16 = pedestrian/cycleway/footway/steps, 5 = track/path. Damit trennt B `service` von
`residential`, die in D beide Klasse 7 haben.

Die Zusatzeinträge (12 B, 104 in DK) gehören zu Knoten mit Bit 19: `[0]` = Knotenindex, `[1]`/`[2]`
große Werte (Bedeutung offen). Die Kanten beider Richtungen tragen dieselben Flags (`a1[6]`).
Knoten 0 ist `[0, 0, 0]`, Kopf `[3]` zählt ihn mit.

## C-Records

Leser `FUN_003e651c`, gleicher Container wie osmarea (siehe
[OSMAREA_FORMAT.md](OSMAREA_FORMAT.md)), aber **mit dem Rand von 512** wie die D-Records
(bei c1 geprüft). In osm sind nur c1 (265 Einträge, 0x10 B), c2 (5) und c4 (42) belegt:

- **c1**: `[0]` = `0xFFFFFFFF`, `[1]` = **Straßenklasse 0–3** (Autobahn, trunk, primary,
  secondary), `[2]` = Anzahl u32 der Geometrie. Übersichtsnetz für kleine Zoomstufen: **ein
  Eintrag je Zelle und Klasse** mit vielen Teilen, stark vereinfacht (≈ 43.000 Punkte für DK).
- **c2**: `[0]` = Anzahl u32, Geometrie wie a2 (`n`, dann Punkte).
- **c4**: `[0]` = Typ, `[1]` = Anzahl u32 der Geometrie.

## Nachbauen

```python
import sys; sys.path.insert(0, "tools")
import layers as L
d = open(PFAD, "rb").read()
for tx, ty, cx, cy, g, raw in L.iter_records(d, "D"):
    rec = L.parse_d(raw)                     # rec["a1"][i] = {"s": [10 Felder], "v": [geometrie]}
    for e in rec["a1"]:
        klasse, laenge_m = e["s"][7] >> 27, (e["s"][7] & 0xFFFFFF) / 4.1906
        for hi, pts in L.geometry_parts(e["v"][0]):
            coords = [L.to_latlon(cx, cy, g, p, margin=L.MARGIN) for p in pts]  # Rand 512!
    assert L.build_d(rec) == raw
```

Entsprechend `parse_a`/`build_a`, `parse_b`/`build_b`, `parse_c`/`build_c`.

## Aus OSM erzeugen

```bash
.venv/bin/python tools/osm_extract.py osm_ref/denmark-latest.osm.pbf build/ref/ways_latest.pkl   # ~2 min
.venv/bin/python tools/osm_heights.py 2013021200000368/7/943/20317/Denmark_osm.v20210916 \
    build/ref/heights.pkl                                   # einmalig, ~5 min (scipy)
.venv/bin/python tools/compile_osm.py --heights=build/ref/heights.pkl build/ref/ways_latest.pkl \
    osm_ref/denmark.poly 2013021200000368/7/943/20317/Denmark_osm.v20210916 \
    build/latest/Denmark_osm.v20210916 20260918            # ~5 min
```

Schritte in `compile_osm.py`:

1. **Straßen**: Ways mit `highway` aus der Klassentabelle (ohne `area=yes`) und `route=ferry`.
   Geteilt wird an jedem OSM-Knoten, den mehrere Straßen-Ways nutzen, und an Way-Enden.
2. **Länge**: Haversine (R = 6.371 km) × 5·2²⁵ / (2π·R) (≈ 4,19 Einheiten/m), Median-Verhältnis zum
   Original 1,000.
3. **Flags** nach der Tabelle oben, mit den Korrekturen aus dem Abgleich: Bits 7–8 sind eine
   **Belagsklasse** (3 = kein `surface`, 1 = asphalt/paved/paving_stones/concrete…,
   2 = sett/cobblestone, 0 = sonst); Fahrradbits 9–10 = 3 bei `bicycle=designated` **oder**
   `cycleway=track/lane/shared_lane/separate` (nur der Schlüssel `cycleway`, nicht
   `cycleway:right` usw., die setzen auch Bit 29 nicht); Bit 31 auch bei `foot=permissive`.
   Bits 20–23 und 28 bleiben 0.
4. **Graph (B)**: Knoten je 8×8-Zelle nach Position sortiert, Index ab 1; Kanten in beide
   Richtungen, Bit 30 aus `oneway` (`-1` rückwärts; `oneway:bicycle=no` beidseitig). Knoten
   `[2]` = Linksabbiegen nach der Regel oben. Kante `[2]` = Anstieg: Höhe an jedem Stützpunkt
   aus den 4 nächsten Knoten des Originals (inverse Distanz², `osm_heights.py`) oder, ohne
   Original, bilinear aus dem Copernicus-Höhenmodell (`dem_heights.py`, s. u.); Summe der
   Höhengewinne. Kanten, deren Endknoten mehr als 63 Zellen auseinanderliegen (lange Fähren),
   kommen nicht in den Graphen. Mit `B_ONLY=<pkl>` endet der Compiler nach dem Graphen
   (zum Eichen).
5. **D**: Kanten und Linien (a3/a4) am Zellrand ± 512 geschnitten, a1 nach Klasse und Name
   sortiert. **A**: Namenstabelle je 4×4-Zelle aus den Namen ihrer 64 D-Zellen.
   **C**: Klassen 0–3 per `shapely.line_merge` verbunden, Douglas-Peucker mit 32 Einheiten
   (360°/2²⁵), ein c1-Eintrag je Zelle und Klasse; c2/c4 (Grenzen) aus dem Original.
6. Nur Zellen, die das Gebiet (`denmark.poly`) plus 32.768 Einheiten (eine D-Zelle) berühren;
   Kanten zu Knoten in weggelassenen B-Zellen fallen aus dem Graphen, ihre IDs in D werden 0.
   Die Färöer-Tiles (westlich 0°) kommen unverändert aus dem Original (nur mit Original).
7. **Speicher:** Kanten, Graph-Knoten und deren Nummerierung je B-Zelle werden für das ganze
   Land mit numpy berechnet, die B-, D- und A-Records danach **Tile für Tile**; die Tag-Dicts
   werden nach dem Auswerten freigegeben. Für Dänemark bytegleiche Records wie vorher bei
   4,9 statt 10,9 GB; Großbritannien 18 min, 18 GB.

**Länder ohne Originaldatei** (Großbritannien): Original `-`, dann bleiben c2/c4 leer und
nichts wird kopiert; `--country=N` (Header-Ländercode, KARTEN_ENTSCHLUESSELUNG.md 1),
`--name=<Land>` (Landesname im A-Record). Die Anstiege kommen aus dem **Copernicus DEM
GLO-90** (`tools/dem_heights.py`: Kacheln aus dem öffentlichen AWS-Bucket
`copernicus-dem-90m`, auf ein 3″-Raster gebracht, Gauß-Glättung σ = 1 Rasterpunkt, da das
Modell ein Oberflächenmodell mit Bäumen und Häusern ist). Geeicht an Dänemark gegen die
Original-Anstiege: σ = 0 Korrelation 0,76 (Summe 1,77×), **σ = 1 Korrelation 0,84, Median-
Verhältnis 0,97**, 61 % auf 10 cm, 88 % auf 50 cm; σ = 2 0,80, σ = 4 0,69. Die Rekonstruktion
aus dem Original (`osm_heights.py`) ist mit 0,88 etwas besser und bleibt für Dänemark.

```bash
.venv/bin/python tools/osm_extract.py --filter osm_ref/great-britain-latest.osm.pbf build/gb/ways.pkl  # 12 min, 14 GB
.venv/bin/python tools/dem_heights.py osm_ref/great-britain.poly osm_ref/dem build/gb/dem.pkl         # ~1 min (Download)
.venv/bin/python tools/compile_osm.py --heights=build/gb/dem.pkl --country=17 "--name=United Kingdom" \
    build/gb/ways.pkl osm_ref/great-britain.poly - build/gb/GreatBritain_osm.v20260918 20260918    # 18 min, 18 GB
```

`--filter` behält nur Ways, die `road_class` oder `line_type` annimmt, und speichert die
Knotenspalten als typisierte Arrays (sonst passt Großbritannien nicht in den Speicher).

**Abgleich** (Build aus `denmark-220101` gegen das Original, jede 7. D-Zelle): 95 % der
a1-Kanten mit identischer Geometrie (Anfangs- und Endpunkt), davon Klasse 99,6 %, Name 99,6 %,
Knoten-ID 0 ja/nein 99,8 %, Flags 92 %, Länge Median 1,000 (p10/p90 0,997/1,001). Graph:
96 % der Knotenpositionen identisch, 91 % der Kanten direkt zugeordnet, Klasse/Kategorie/
Befahrbarkeit bis auf wenige Hundert gleich. Linksabbiegen: ob ein Knoten Einträge hat,
stimmt zu 99,9 %; die Paare (über die Zielpositionen verglichen) zu 85 %, der Rest sind
Abweichungen der Nachbarpositionen. Anstieg: Korrelation 0,88, 86 % auf 10 cm, 96 % auf 50 cm,
Median-Verhältnis 1,00. Der Build aus `denmark-latest` (2026) hat 38 % mehr Kanten (3,0 statt
2,2 Mio.) und ist 92 MB groß (Original 63 MB); die größten Records (B 4,2 MB, D 1,0 MB)
bleiben unter denen der deutschen Karte (6,5 / 1,3 MB). Kostenfaktor (Bits 20–23) und Bit 28
bleiben 0.

## Offen

1. Herkunft des Kostenfaktors (Bits 20–23); Bit 28; Klasse 15.
2. Kanten mit IDs 0/0 außerhalb der Autobahnen (einzelne Fuß-/Wirtschaftswege).
3. B-Zusatzeinträge; zweite Tabelle bei `0x4fc080` (Klasse → Gruppe 0/1/10/11/5/10/6/6/7/8,
   Aufschlag 90.000 beim Wechsel der Gruppe).
4. a3-Typen 1–3, c1/c4-Typen.
5. A-Records: Bedeutung der 0x10-Unterelemente und der Felder `[4]`/`[5]` der Orte.
6. Ausreißer beim Längenfaktor (einzelne Kanten, z. B. um 54° und 59° N sowie auf den Färöern),
   vermutlich Fähren oder Kanten mit Sonderlänge.
