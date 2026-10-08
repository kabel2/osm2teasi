# osmpoint-Format (`Denmark_osmpoint.v20210916`)

Stand: 2026-09-18. **Vollständig verstanden**: Parser und Builder in `tools/layers.py`
erzeugen alle 110 Records **bytegleich** zurück (Round-Trip-Test).

Inhalt: **Seezeichen aus OpenSeaMap** (`seamark:*`), also Tonnen, Baken, Leuchtfeuer mit
Sektoren, Häfen, Windräder, Felsen, Wracks usw. Dänemark-Datei: 5211 Objekte in 110 Records,
darunter auch die Färöer.

Hülle, Verschlüsselung, Slot-Bereiche und Koordinatensystem sind in
[KARTEN_ENTSCHLUESSELUNG.md](KARTEN_ENTSCHLUESSELUNG.md) beschrieben (Abschnitte 1, 1a und 3).

## Einordnung

| | |
|---|---|
| Header `0x50` | `1024` (Layer-Kennung) |
| Slot-Bereich | **C** (Tile-Kopf `+0x140`, 4×4 Unterzellen) |
| Leser in der Firmware | `FUN_003e52d0` (interner Typ 5) |
| 1 Record | = alle Objekte **einer 4×4-Unterzelle** (0,3515625° × 0,3515625°) |

## Aufbau des entpackten Records

```
0x00  u32  n·0x30     Größe des Struktur-Arrays (wird von der Firmware mit cell_x überschrieben)
0x04  u32  cell_y     y der Unterzelle (wird mit cell_y überschrieben)
0x08  u32  0
0x0C  u32  n          Anzahl Objekte
0x10  u32  0          (Firmware: Zeiger auf das Array)
0x14  n × 0x30 B      Objekt-Strukturen, BYTEWEISE TRANSPONIERT
      Strings         pro Objekt: name, attrs, label, danach die Sektor-Einträge (je 16 B)
      Sektor-Labels   für alle Objekte, erst nach dem gesamten String-Block
```

Es gibt kein Padding am Ende: Die Record-Größe (`pltx`) geht exakt auf.

**Transponiert** heißt: Zuerst steht Byte 0 aller n Strukturen, dann Byte 1 aller Strukturen usw.
Die Firmware dreht das nach dem Entpacken zurück (`tools/layers.py: untranspose/transpose`).

### Objekt-Struktur (0x30 B = 12 × u32, nach dem Zurücktransponieren)

| Feld | Inhalt |
|---|---|
| `[0]` | 0 (Firmware: Zeiger auf `name`) |
| `[1]` | Länge `name` in UTF-16-Zeichen **inkl.** NUL, 0 = kein String |
| `[2]` | Kategorie (Symbol), siehe unten |
| `[3]` | **Position**: `(v << 16) \| u`, siehe Koordinaten |
| `[4]` | Farbmuster des Körpers (Bitfeld, noch nicht vollständig entschlüsselt) |
| `[5]` | Toppzeichen (Form + Farbe, noch nicht vollständig entschlüsselt) |
| `[6]` | 0 (Zeiger auf `attrs`) |
| `[7]` | Länge `attrs` inkl. NUL |
| `[8]` | 0 (Zeiger auf `label`) |
| `[9]` | Länge `label` inkl. NUL |
| `[10]` | Anzahl Sektoren |
| `[11]` | 0 (Zeiger auf das Sektor-Array) |

Ein leerer String hat die Länge 0 und belegt keinen Platz, er hat also auch kein NUL.

### Sektor-Eintrag (16 B, **nicht** transponiert)

```
u16 colour    Farbcode wie bei [4]: 1 = weiß, 3 = rot, 4 = grün, 8 = gelb, 9 = bernstein
              (12 kommt 18× vor, Herkunft offen)
u16 from      Sektorbeginn in 1/4096 Kreis, um 180° gedreht (Richtung vom Feuer weg):
              int((sector_start + 180) mod 360 · 4096/360), z. B. 152,5° → 3783
u16 to        Sektorende ebenso
u16 radius    370 bei echten Sektoren, 0 bei Rundumfeuern
u32 0         (Zeiger auf das Label)
u32 len       Länge des Sektor-Labels inkl. NUL (z. B. "Fl.G.3s")
```

## Koordinaten

```
u = pos & 0xFFFF      Ost-Offset ab der West-Kante der Unterzelle
v = pos >> 16         Süd-Offset ab der Nord-Kante der Unterzelle
Einheit: 360° / 2^25  (32768 Einheiten = 1 Unterzelle = 0,3515625°), Bereich 0..32768

lat = 90  − (cell_y · 32768 + v) · 360/2^25
lon = −180 + (cell_x · 32768 + u) · 360/2^25
```

`cell_x = tile_x·4 + slot//4`, `cell_y = tile_y·4 + slot%4`.

Geprüft an Leuchttürmen mit bekannter Position (Abweichung ≤ 20 m):

| Objekt | aus der Datei | Soll |
|---|---|---|
| Hanstholm | 57.11271 N, 8.59856 E | 57.1128 N, 8.5985 E |
| Hirtshals | 57.58474 N, 9.94191 E | 57.5847 N, 9.9419 E |
| Stevns | 55.29067 N, 12.45359 E | 55.2907 N, 12.4536 E |
| Leynar (Färöer) | 62.110 N, 7.041 W | 62.11 N, 7.04 W |

## Strings

Alle Strings sind UTF-16LE.

**`name`**: `seamark:name` bzw. `name` (z. B. „Hanstholm“, „No. 14“).

**`attrs`**: Attributliste `NNwert|NNwert|…` mit einem zweistelligen Code vor jedem Wert.
Bisher erkannte Codes:

| Code | Bedeutung | Beispiel |
|---|---|---|
| `01` | Website | `01http://www.hanstholmhavn.dk/…` |
| `02` | Telefon | `02+45 96 550710` |
| `11` | Beschreibung/Notiz | `11Opført 1884` |
| `17` | E-Mail | |
| `18` | Betreiber | `18DS, Lemvig Sejlklub` |
| `19` | **Objektart (Klartext)** | `19Port-hand Lateral Buoy`, `19Light minor` |
| `20` | Information | `20Lighted by day when visibility is 3M or less.` |
| `21` | Radartransponder | `21Racon(T)` |
| `23` | Nebelsignal | `23Horn` |
| `24` | Kennung des Feuers | `24Fl.W.20s65m26M` |
| `25` | Leuchtfeuer-Verzeichnisnummer | `25B 2084` |
| `26` | Betonnungssystem | `26iala-a` |
| `36` | Höhe in m | `3665` |
| `45` | Wasserstand (S-57 WATLEV) | `45submerged`, `45awash` |
| `35`, `37`, `04`, `06`, `10`, `16`, `27`, `38`, … | noch offen | |

Häfen haben bei `19` ein abschließendes `\n` (`19Yacht harbour/marina\n`), so steht es auch im
Original.

**`label`**: Kurztext für die Kartenanzeige, ebenfalls mit Präfix: `5` = Kennung des Feuers
(`5Fl.W.20s65m26M`), `0` = Nebel- oder Radarsignal (`0Horn`, `0Racon(T)`), `7` = Signalstelle,
`4` = Durchfahrtshöhe. Mehrere Einträge werden mit `|` getrennt.

## Kategorie `[2]`

`[2] = Gruppe << 16 | Unterart << 8 | Klasse`, per OSM-Abgleich vollständig aufgelöst
(99,6 % Treffer, Regeln in `category()` von `tools/compile_osmpoint.py`):

- **Klasse** (Byte 0) nach `seamark:type`: Tonne 0, Bake 1, Leuchtfeuer 3, Landmarke 4,
  Hafen 6, Ankerplatz 7, Festmacher 8, Wrack 9, Fels 0xC, Brücke 0xD, Funkstation 0xE,
  Signalstelle 0xF, Plattform 0x10, Windpark 0x11.
- **Gruppe** (Byte 2): Lateral/Kardinal 0x0A, Sonderzeichen/Einzelgefahr 0x0C, Ansteuerung
  0x09, Leuchtfeuer und Häfen 0x09, Landmarken 0x0A, Ankerplatz/Brücke 0x0E,
  Festmacher/Fels/Wrack 0x0F, Plattform/Windpark 0x0B, Funk/Signal 0x0C.
- **Unterart** (Byte 1): Tonnenform (`conical` 0, `can` 1, `spherical` 2, `pillar` 3 und
  Standard, `spar` 4, `barrel` 5, `super-buoy` 6); Bakenform (`stake/pole/post` 0, `tower` 2,
  `pile/lattice` 3 und Standard); Leuchtfeuer groß 0 / klein 1; Landmarke = S-57-CATLMK − 1
  (`chimney` 2, `mast` 6, `tower` 16, `windmotor` 18 …); Hafen (`fishing` 0, `marina` 1,
  `marina_no_facilities` 2, sonst 3); Festmacher (`dolphin` 0, `bollard` 2, `wall` 3,
  `post/pile` 4, `buoy` 5); Fels nach `water_level` (`covers` 0, `awash` 1, sonst 2); Wrack
  (`non-dangerous` 0, `dangerous`/leer 1, `hull_showing` 2).
- Eine Landmarke ohne Kategorie gilt als großes Leuchtfeuer (0x090003).

Häufigste Werte in Dänemark:

| `[2]` | Objekte | Anzahl |
|---|---|---:|
| `0x0A0400` | Lateral- und Kardinaltonnen (Tonnenform 4, vermutlich Spiere) | 977 |
| `0x090103` | Leuchtfeuer (klein), inkl. Richtfeuer | 899 |
| `0x0A1204` | Windrad | 564 |
| `0x0A0300` | Lateral- und Kardinaltonnen (Tonnenform 3) | 346 |
| `0x0A0100` | Backbordtonne (Form 1, vermutlich Stumpftonne) | 267 |
| `0x0F0508` | Festmachertonne | 263 |
| `0x0F020C` | Fels (unter Wasser) | 257 |
| `0x090106` | Yachthafen | 247 |
| `0x0A0000` | Steuerbordtonne (Form 0, vermutlich Spitztonne) | 217 |
| `0x0C0400` | Sonderzeichen (Tonne) | 165 |
| `0x0C0301` | Sonderzeichen (Bake), Einzelgefahrenzeichen | 108 |
| `0x0F0008` | Dalben | 101 |
| `0x0A0301` | Lateralbake | 82 |
| `0x090003` | Leuchtfeuer (groß) | 42 |
| `0x0A1004` | Leuchtturm | 26 |
| `0x0F0109` / `0x0F0009` / `0x0F0209` | Wrack (gefährlich / ungefährlich / sichtbar) | 15 / 11 / 4 |
| `0x0E0007` | Ankerplatz | 17 |
| `0x0E000D` | feste Brücke | 1 |

Eine vollständige Liste (55 Werte) mit Beispielen liefert `tools/layers.py` zusammen mit
`collections.Counter` über `o["cat"]`.

## Farben `[4]` und Toppzeichen `[5]`

Per OSM-Abgleich aufgelöst (98,4 % / 99,4 % Treffer):

```
Farbcodes: grau 0, weiß 1, schwarz 2, rot 3, grün 4, blau 5, gelb 8

[4] (nur Tonnen und Baken, aus seamark:<type>:colour / :colour_pattern):
    Bits 0-3  Muster (vertical = 4, sonst 0)
    Bits 4-6  Anzahl Farben
    ab Bit 7  die ersten zwei Farben, je 4 Bit (bei 3 Bändern fehlt das dritte)
[5] (aus seamark:topmark:shape / :colour):
    Bits 0-6  Form: cone point up 1, cone point down 2, sphere 3, 2 spheres 4,
              cylinder 5, board 6, x-shape 7, upright cross 8, 2 cones point together 10,
              2 cones base together 11, rhombus 12, 2 cones up 13, 2 cones down 14,
              square 17, triangle point up 18, triangle point down 19
    Bits 7-8  Anzahl Farben
    ab Bit 9  die ersten zwei Farben, je 4 Bit
```

Beispiele:

| Objekt | `[4]` | `[5]` |
|---|---|---|
| Backbord (rot) | `0x190` | `0x685` (Zylinder) |
| Steuerbord (grün) | `0x210` | `0x881` (Kegel) |
| Sonderzeichen (gelb) | `0x410` | `0x1087` (Kreuz) |
| Nord-Kardinal (schwarz über gelb) | `0x4120` | `0x48D` |
| Süd-Kardinal (gelb über schwarz) | `0x1420` | `0x48E` |
| Ost-Kardinal (schwarz-gelb-schwarz) | `0x4130` | `0x48B` |
| West-Kardinal (gelb-schwarz-gelb) | `0x1430` | `0x48A` |
| Ansteuerungstonne (rot-weiß senkrecht) | `0x9A4` | `0x683` (Kugel) |
| ohne Farbe/Toppzeichen | `0` | `0` |


## Nachbauen

```python
import sys; sys.path.insert(0, "tools")
from layers import iter_records, parse_osmpoint, build_osmpoint, to_latlon

for tx, ty, cx, cy, grid, raw in iter_records(open(PFAD, "rb").read(), "C"):
    objs = parse_osmpoint(raw)                  # Liste von dicts
    assert build_osmpoint(objs, cy) == raw      # bytegleich
```

Die Reihenfolge der Objekte in der Originaldatei folgt keiner erkennbaren Sortierung
(vermutlich der OSM-Reihenfolge).

## Aus OSM erzeugen

```bash
.venv/bin/python tools/osm_poi_extract.py osm_ref/denmark-latest.osm.pbf build/ref/poi_latest.pkl   # wie osmpoi
.venv/bin/python tools/compile_osmpoint.py build/ref/poi_latest.pkl osm_ref/denmark.poly \
    build/<ordner>/Denmark_osmpoint.v20210916 [JJJJMMTT]
```

Übernommen wird jedes Objekt mit `seamark:type`, für das es eine Kategorie gibt (nicht z. B.
`small_craft_facility`, `pile`, `cable_submarine`, `navigation_line`), innerhalb der
Geofabrik-Grenze. Position: Knoten direkt, Flächen über den Schwerpunkt, gerundet auf 360°/2²⁵.

**Eichung** (4691 Original-Objekte mit exakt passendem OSM-Knoten in `denmark-220101`):

| Feld | Treffer |
|---|---:|
| Kategorie `[2]` | 99,6 % |
| Farbe `[4]` / Toppzeichen `[5]` | 98,4 % / 99,4 % |
| Name (`seamark:name`, nicht `name`) | 99,9 % |
| Attribute | 94 % |
| Label | 97 % |
| Sektoren | 97 % |

Regeln für die Texte (`description()`, `light_string()`, `attributes()`):

- **`19`**: englische Beschreibung aus Typ und Kategorie, z. B. `Port-hand Lateral Buoy`,
  `North Cardinal Buoy`, `Diving mark Special Purpose Buoy`, `Front light minor`,
  `Lower,leading light minor` (Lichtkategorien mit Komma), `Yacht harbour/marina\n`
  (Hafenkategorien mit `\n`, mehrere als Liste `- …\n`), `Windmotor`, `Rock`, `Mooring buoy`.
- **`24` / Label `5`**: `Kennung.Farben.Periode s Höhe m Reichweite M(Kategorie)`. Farben als
  Buchstaben in der Reihenfolge W, R, G, Y; die Höhe nur bei einem einzelnen Feuer ohne
  Sektor; Reichweite abgeschnitten, bei mehreren Feuern `min-max`; die Kategorie des letzten
  Feuers mit großem Anfangsbuchstaben. Beispiele: `Fl.G.5s6m4M`, `Oc.WRG.12s9-12M`,
  `F.R(Air_obstruction)`, `.R` (ohne Kennung).
- **`36`** = `light:height` (bzw. `light:1:height`), ganze Zahlen ohne `.0`. **`35`** =
  `landmark:height`, **`37`** = `light:exhibition`, **`45`** = `water_level` (Fels ohne Angabe:
  `submerged`), **`20`** = `seamark:information`, **`21`** = `Racon(<group>)`, **`23`** =
  `Horn`, `Siren(2)60s`, **`25`** = `light:reference`, **`26`** = `<type>:system`, **`27`** =
  `port_of_entry`, dazu `01`, `02`, `04`, `06`, `11`, `16`, `17`, `18` wie bei osmpoi.
- **Label**: `0` + Racon/Nebelsignal, `7SS` bei Warnsignalstellen,
  `(Bridge Passage)` bei Verkehrssignalstellen, `4` + Durchfahrtshöhe, `5` + Kennung.
- **Sektoren**: je Feuer ein Eintrag, sortiert nach Farbcode. Ohne `sector_start`/`sector_end`
  ist `from = to = 0`, `radius = 0` und das Label leer, sonst `radius = 370` und das Label
  `Kennung.Farbe.Periode s`.

Aktueller Stand (2026-09-18): 9921 Seezeichen (Original 5211). Mehr sind es, weil OpenSeaMap
gewachsen ist (u. a. 1465 statt 564 Windräder), aber auch weil die Geofabrik-Grenze
Nachbargewässer einschließt: Das Original enthält keine Seezeichen östlich von 13° O bei
Rügen/Schonen und kaum welche in der Kieler Bucht. Die Färöer fehlen.

## Offen

- Sonderfälle bei der Kennung (Wechselfeuer `Al.M`, Farbreihenfolge `RWG` bei manchen
  Sektorfeuern), Sektor-Farbcode 12.
- `[4]`-Varianten mit gesetzten Bits 0–1 (`0x191`, `0x9A5`, `0x2` bei Leuchtfeuern).
- Welche Grenze das Original nutzt (keine Nachbargewässer).
