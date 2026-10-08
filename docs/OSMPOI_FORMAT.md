# osmpoi-Format (`Denmark_osmpoi.v20210915`)

Stand: 2026-09-18. **Vollständig verstanden**: `tools/layers.py` baut alle 3704 Records
**bytegleich** nach (`build_osmpoi(parse_osmpoi(raw)) == raw`).

Inhalt: **Points of Interest aus OSM** (Restaurants, Cafés, Geldautomaten, Haltestellen,
Kirchen, Hotels, Campingplätze, Denkmäler …). Dänemark-Datei: 79.862 POIs in 3704 Records
(Färöer eingeschlossen), davon 32.597 ohne Namen.

Hülle, Verschlüsselung, Slot-Bereiche und Koordinatensystem: siehe
[KARTEN_ENTSCHLUESSELUNG.md](KARTEN_ENTSCHLUESSELUNG.md), Abschnitte 1, 1a und 3.

## Einordnung

| | |
|---|---|
| Header `0x50` | `8` (Layer-Kennung) |
| Slot-Bereich | **D** (Tile-Kopf `+0x180`, 32×32 Unterzellen) |
| Leser in der Firmware | `FUN_003e56fc` (interner Typ 1) |
| 1 Record | = alle POIs **einer 32×32-Unterzelle** (0,0439° × 0,0439°, ca. 4,9 × 2,8 km in DK) |

## D-Record (allgemeiner Aufbau)

osmpoi, osm und ta verwenden in Slot-Bereich D denselben Container. Der generische
Parser/Builder `parse_d` / `build_d` in `tools/layers.py` liest und schreibt alle 12.934
D-Records der drei Dateien bytegleich.

```
0x00  13 × u32  Kopf
        [0]   Größe (s. u.; die Firmware überschreibt das Feld mit cell_x)
        [1]   0  (wird mit cell_y überschrieben)
        [2]   0
        [3]   n1  Anzahl Array a1 (0x28 B)     [4]  0 (Zeiger)
        [5]   n2  Anzahl Array a2 (0x08 B)     [6]  0
        [7]   n3  Anzahl Array a3 (0x0C B)     [8]  0
        [9]   n4  Anzahl Array a4 (0x14 B)     [10] 0
        [11]  n5  Anzahl Array a5 (0x18 B)     [12] 0
0x34  a1, a2, a3, a4, a5 hintereinander, jedes Array BYTEWEISE TRANSPONIERT
      danach die variablen Teile, Array für Array und darin Element für Element:
        a1: Feld [8] = Anzahl u32, die folgen          (Zeiger in [9])
        a2: Feld [0] = Anzahl u32                      (Zeiger in [1])
        a3: Feld [1] = Anzahl u32                      (Zeiger in [2])
        a4: Feld [1] = Anzahl u16 (String),            (Zeiger in [0])
            dann Feld [3] = Anzahl u32                 (Zeiger in [4])
        a5: Feld [1] = Anzahl u16 (String),            (Zeiger in [0])
            dann Feld [3] = Anzahl u16 (String)        (Zeiger in [2])
```

Es gibt kein Alignment und kein Padding, der Record geht exakt auf.
Kopf `[0]`: Bei osmpoi ist es die Summe der Array-Größen (`n5 · 0x18`), bei osm und ta meist
`n1 · 0x28`. Die Firmware wertet das Feld nicht aus.

## osmpoi: nur Array a5

Bei osmpoi sind `n1..n4 = 0`, es gibt nur a5 mit einem Eintrag pro POI.

| a5-Feld | Inhalt |
|---|---|
| `[0]` | 0 (Zeiger auf den Namen) |
| `[1]` | Länge des Namens in UTF-16-Zeichen inkl. NUL, 0 = kein Name |
| `[2]` | 0 (Zeiger auf die Attribute) |
| `[3]` | Länge des Attribut-Strings inkl. NUL, 0 = keine Attribute |
| `[4]` | **POI-Typ** (Index in die Tabelle unten) |
| `[5]` | **Position** `(v << 16) \| u` |

Danach folgen pro POI der Name und der Attribut-String, beide UTF-16LE.

### Koordinaten

Wie bei allen Records relativ zur Nordwest-Ecke der Unterzelle, **32768 Einheiten pro
Unterzelle**. Bei 32×32 ist eine Einheit also 360°/2²⁸ ≈ 1,34·10⁻⁶° (ca. 15 cm):

```
cell_x = tile_x·32 + slot // 32,   cell_y = tile_y·32 + slot % 32
lat = 90  − (cell_y · 32768 + v) · 360/2^28
lon = −180 + (cell_x · 32768 + u) · 360/2^28
```

Geprüft: „Den Gamle By“ (Museum, Aarhus) → 56.1589 N, 10.1916 E; „Tivoli“ → 55.6733 N,
12.5690 E; „Gulahusid“ (Mykines, Färöer) → 62.1037 N, 7.6456 W.

### POI-Typen `[4]`

Die Namen stammen aus einer Zeigertabelle in `bikenav.exe` (`.data`, VA `0x4fbb50`, 75
Einträge; der String `poitype_<name>` ist der Übersetzungsschlüssel). In der Spalte „DK“ steht
die Anzahl in der Dänemark-Datei.

| ID | poitype | DK | | ID | poitype | DK |
|---:|---|---:|---|---:|---|---:|
| 0x00 | bank | 705 | | 0x1c | train_station | 573 |
| 0x01 | bikestore | 689 | | 0x1d | christian_church | 2687 |
| 0x02 | cafe_pub | 5776 | | 0x1e | muslim_mosque | 23 |
| 0x03 | cashdispenser | 543 | | 0x1f | buddhist_temple | 8 |
| 0x04 | departmentstore | 10 | | 0x20–0x23 | hindu/shinto/taoist/sikh_temple | 0 |
| 0x05 | doctor | 210 | | 0x24 | jewish_synagog | 1 |
| 0x06 | emergencymedicalservice (Defibrillator) | 586 | | 0x25 | bus_station (auch Fähranleger) | 211 |
| 0x07 | healthcareservice | 0 | | 0x26 | bicycle_rental | 182 |
| 0x08 | hospital_polyclinic | 188 | | 0x27 | areodrome | 7 |
| 0x09 | importanttouristattraction | 745 | | 0x28 | charging_station | 1 |
| 0x0a | market | 57 | | 0x29–0x35 | Seilbahnen, Lifte, Ski (cable_car … ski_school) | 8 |
| 0x0b | petrolstation | 1958 | | 0x36 | shelter | 2583 |
| 0x0c | pharmacy | 382 | | 0x37–0x44 | smallcraftfacility_* (Sportboothafen-Einrichtungen) | 321 |
| 0x0d | policestation | 96 | | 0x45 | harbour | 0 |
| 0x0e | restaurant | 10186 | | 0x46 | landmark | 0 |
| 0x0f | scenic_panoramicview | 1015 | | 0x47 | conti_services | 0 |
| 0x10 | shop | 2724 | | 0x48 | landmark (Denkmäler, historische Orte) | 27881 |
| 0x11 | shoppingcenter (Supermärkte) | 2869 | | 0x49 | museum | 917 |
| 0x12 | sportscenter | 1869 | | 0x4a | zoo | 54 |
| 0x13 | touristinformationoffice | 2067 | | | | |
| 0x14 | camping_ground | 1234 | | | | |
| 0x15 | hotel_motel | 1021 | | | | |
| 0x16 | sla (hier: Fahrradläden) | 33 | | | | |
| 0x17 | sla_extern | 0 | | | | |
| 0x18 | bike_tyres | 0 | | | | |
| 0x19 | bus_stop | 9376 | | | | |
| 0x1a | tram_stop | 35 | | | | |
| 0x1b | subway_entrance (Metro) | 31 | | | | |

Die Liste steht auch als `POI_TYPES` in `tools/layers.py`. Wie OSM-Tags auf die Typen
abgebildet werden, steht nicht in der Datei, sondern wurde per Abgleich mit OSM ermittelt,
siehe „Aus OSM erzeugen“.

### Attribut-String

Format wie bei osmpoint: `NNwert|NNwert|…` mit zweistelligem Code, UTF-16LE, mit NUL am Ende.

| Code | OSM-Tag (abgeleitet) | Beispiel |
|---|---|---|
| `00` | Adresse `addr:street addr:housenumber` `\n` `addr:postcode addr:city` | `00Langgade 35\n7550 Sørvad` |
| `01` | `website`, sonst `url`, sonst `contact:website` | `01https://www.bat.dk/` |
| `02` | `phone`, sonst `contact:phone` | `02+45 72 54 30 00` |
| `03` | `opening_hours`, `;` durch Zeilenumbruch ersetzt | `03Mo-Su 08:00-21:00`, `0324/7` |
| `04` | `internet_access` | `04wlan` |
| `05` | `smoking` | `05outside` |
| `06` | `wheelchair` | `06limited` |
| `07` | `beds` | |
| `08` | `rooms` | `0835` |
| `10` | `capacity` | `1070` |
| `09` | `cuisine` | `09pizza` |
| `11` | `description` | `11Gravhøj` |
| `16` | `fax` | |
| `17` | `email`, sonst `contact:email` | `17info@dragsholm-slot.dk` |
| `18` | `operator` | `18Sydbus`, `18Danske Bank` |
| `74` | `access` | `74private` |

## Nachbauen

```python
import sys; sys.path.insert(0, "tools")
from layers import iter_records, parse_osmpoi, build_osmpoi, to_latlon, POI_TYPES

for tx, ty, cx, cy, grid, raw in iter_records(open(PFAD, "rb").read(), "D"):
    for p in parse_osmpoi(raw):          # {"type", "pos", "name", "attrs"}
        lat, lon = to_latlon(cx, cy, grid, p["pos"])
```

Die Reihenfolge innerhalb eines Records ist im Original nicht nach Typ sortiert (nur 806 von
3704 Records sind zufällig sortiert).

## Aus OSM erzeugen

Für große Länder `osm_poi_extract.py --filter`: behält nur Objekte, die `poi_type` annimmt
oder die `seamark:type` haben (osmpoint); für Dänemark ergibt das dieselben Dateien.
`compile_osmpoi.py` und `compile_osmpoint.py` nehmen `--country=N` (Standard 4 = Dänemark).

```bash
.venv/bin/python tools/osm_poi_extract.py osm_ref/denmark-latest.osm.pbf build/ref/poi_latest.pkl   # ~4 min
.venv/bin/python tools/compile_osmpoi.py build/ref/poi_latest.pkl osm_ref/denmark.poly \
    build/<ordner>/Denmark_osmpoi.v20210915 [JJJJMMTT]                                         # ~30 s
```

- `osm_poi_extract.py` speichert alle getaggten Knoten, Ways und Multipolygone mit Mittelpunkt
  (Knoten, Eckpunkt-Mittel, Bounding-Box-Mitte, Flächenschwerpunkt) als Pickle.
- `compile_osmpoi.py` ordnet die Typen zu, baut Attribute, entfernt Doppelte, gruppiert nach
  32×32-Unterzelle und schreibt die Datei mit `writer.py`. Das Datum (Header `0x44`) ist
  standardmäßig heute.
- `denmark.poly` ist die Geofabrik-Grenze (`download.geofabrik.de/europe/denmark.poly`). Nur POIs
  darin werden übernommen, denn der Extrakt enthält auch Ways und Relationen, die weit ins
  Ausland reichen (z. B. Punkte bei 51° N, 1° E). **Die Färöer fehlen** im Geofabrik-Extrakt
  Dänemark (eigener Extrakt `faroe-islands`), im Original sind sie enthalten (735 POIs in den
  Tiles (122,19), (123,19), (123,20)).

**Eichung am Original.** Die Regeln stammen aus einem Abgleich der 79.127 dänischen Original-POIs
mit `denmark-220101` (3,5 Monate jünger als das Original): 84 % liegen exakt auf einem OSM-Knoten
oder dem Eckpunkt-Mittel eines Ways, insgesamt 93 % ließen sich (auch über den Namen) einem
Objekt zuordnen. Mit den Regeln unten gebaut und mit dem Original verglichen: Von 65.205 POIs an
identischer Position haben 99,8 % denselben Typ und 98 % denselben Attribut-String. Die
Unterschiede sind fast alle Änderungen in OSM nach September 2021 (u. a. ein Import der
Fødevarestyrelsen mit `fvst:*`-Tags, der ~800 Bäckereien, Metzger und Imbisse neu brachte).

**Typ-Regeln** (erste passende gewinnt, `RULES` in `tools/compile_osmpoi.py`):

| OSM | Typ |
|---|---|
| `amenity=place_of_worship` nach `religion` (christian, muslim, buddhist, hindu, shinto, taoist, sikh, jewish) | 0x1d–0x24 |
| `seamark:type=small_craft_facility` nach `seamark:small_craft_facility:category`, nur **ein** Wert (`toilets;showers` fällt weg) | 0x37–0x44 |
| `amenity=fuel` · `bicycle_rental` · `shop=bicycle` | 0x0b · 0x26 · 0x01 |
| `amenity=bank/atm/pharmacy/doctors/hospital/police/marketplace/bus_station` | 0x00/03/0c/05/08/0d/0a/25 |
| `emergency=defibrillator` | 0x06 |
| `amenity=cafe/bar/pub` → 0x02, `restaurant/fast_food` → 0x0e, `shelter` → 0x36 | |
| `tourism=museum/gallery` → 0x49, `zoo/aquarium` → 0x4a, `attraction` → 0x09, `viewpoint` → 0x0f, `information` → 0x13, `camp_site` → 0x14, `hotel/hostel/motel` → 0x15, `alpine_hut` → 0x33 | |
| `shop=supermarket` → 0x11, `department_store` → 0x04, `convenience/bakery/butcher/kiosk` → 0x10 | |
| `leisure=sports_centre/stadium` → 0x12 | |
| `railway=station/halt` → 0x1c, `tram_stop` → 0x1a, `subway_entrance` → 0x1b, `highway=bus_stop` → 0x19 | |
| `man_made=tower` mit `tower:type=observation` oder `leisure=bird_hide` | 0x0f |
| `amenity=charging_station` mit `bicycle=yes` | 0x28 |
| `aeroway=aerodrome`, nur als Knoten (im Original nur 7 kleine Flugplätze) | 0x27 |
| `aerialway=*` (Lifte) | 0x29–0x31 |
| `historic=*` außer railway, hollow_way, bunker, wall, aircraft, no | 0x48 |

Nicht übernommen werden u. a. `tourism=guest_house/chalet/caravan_site`, `amenity=toilets`,
`charging_station` für Autos, andere `shop=*`-Werte. Einen Namen braucht ein POI nicht.
Typ 0x16 (`sla`, 33 Fahrradläden im Original, offenbar eine Partnerliste) wird nicht
erzeugt, diese Läden werden zu 0x01.

**Position:** Knoten direkt. Bei Flächen nimmt der Compiler den Flächenschwerpunkt. Welche
Methode das Original nutzt, ist offen: Bei kleinen Flächen passen Schwerpunkt, Bounding-Box-Mitte
und Eckpunkt-Mittel exakt, bei großen (z. B. Kirchen) liegt das Original im Median 2 m daneben,
ohne Richtung.

**Doppelte:** POIs mit gleichem Typ und Namen, die weniger als 500 Einheiten (~75 m N-S,
~40 m O-W) auseinanderliegen, werden zu einem zusammengefasst (Knoten vor Flächen, dann nach
OSM-ID). Das betrifft vor allem Bushaltestellen, die in OSM je Straßenseite eingetragen sind.
Im Original liegen ausgeschlossene Haltestellen fast alle < 500, übernommene gleichnamige fast
alle > 500 Einheiten vom nächsten Namensvetter.

## Offen

- Positionsregel für große Flächen (s. o.) und Auswahlregel bei Straßenbahnhaltestellen
  (Original 35, Compiler 80).
- Ob die Firmware leere Unterzellen (Slot `0xFFFFFFFF`) und Records ohne POIs unterschiedlich
  behandelt. Im Original gibt es keine leeren Records.
