# ta-Format (`Denmark_ta.v20180608`)

Stand: 2026-09-19. **Vollständig verstanden**:
- `parse_a`/`build_a` und `parse_d`/`build_d` in `tools/layers.py` bauen alle 4125 Records
  bytegleich nach (A 101, D 4024).
- Den Suchindex baut `tools/ta_index.py` bytegleich nach.
- Der Suchablauf der Firmware ist entschlüsselt (Abschnitt „Suche“).

**Compiler:** `tools/compile_ta.py` baut den Layer aus OSM, siehe „Aus OSM erzeugen“.

Inhalt: **Adressdaten für die Adresssuche**:
- ein Straßennetz mit Straßennamen je Straßenseite und **Hausnummernbereichen**,
- ein Ortsverzeichnis mit Koordinaten und mehrsprachigen Regionsnamen,
- ein landesweiter **Suchindex** über die Orts- und Postleitzahlnamen.

Ohne ta-Datei findet die Suche nur Straßen in der Nähe (osm-Layer). Mit ihr findet sie Orte im
ganzen Land, dazu deren Straßen und Hausnummern.

Die dänische Datei ist deutlich älter (2018) als die osm-Layer (2021). „ta“ steht vermutlich
für Tele Atlas (TomTom), die Daten stammen also wahrscheinlich nicht aus OSM.

**Koordinaten:** Die Linien haben wie osm D einen Rand von 512 Einheiten (Wertebereich
0 … 33.792), siehe [OSM_FORMAT.md](OSM_FORMAT.md).

Hülle, Verschlüsselung, Slot-Bereiche und Koordinatensystem: siehe
[KARTEN_ENTSCHLUESSELUNG.md](KARTEN_ENTSCHLUESSELUNG.md), Abschnitte 1, 1a und 3. Die
Container sind dieselben wie in osm, siehe [OSM_FORMAT.md](OSM_FORMAT.md) (A-Records,
a1-Kanten, Geometrie) und [OSMPOI_FORMAT.md](OSMPOI_FORMAT.md) (allgemeiner D-Record).

## Einordnung

| | |
|---|---|
| Header `0x4C` (Typ) | `2` (die osm-Layer haben 1) |
| Header `0x50` | `0x12` |
| Header `0x70` | Offset des Suchindex (s. u.) |
| Dateiname | muss `_ta.v` enthalten: Nur dann verknüpft die Firmware den Index mit dieser Karte (`FUN_0016068c`, String bei VA `0x4a751c`) |
| Tiles | 14 |
| Slot-Bereich D | 4024 Records, nur Array a1: 1.163.823 Straßenkanten, 642.517 davon mit Hausnummern |
| Slot-Bereich A | 101 Records: Orts- und Straßennamenverzeichnis |

## Suche (Firmware)

Der Ablauf führt vom Ort über die Straße zur Hausnummer:

1. **Ort:** Die Eingabe wird im Suchindex nachgeschlagen (`FUN_00162788`, `FUN_00163384`,
   `FUN_0016201c`). Ein Treffer liefert Name, Koordinaten und die Liste der 4×4-Zellen, in
   denen der Ort vorkommt.
2. **Straße** (`FUN_00163960`): Für jede Zelle lädt die Firmware den A-Record der ta-Karte.
   Der Eintrag muss denselben Ländernamen tragen wie der Index. Gesucht werden die
   0x18-Unterelemente, deren String den Ortsnamen als eine der durch `|` getrennten
   Alternativen enthält (`FUN_00163844`: Vergleich bis `|`, `#` oder NUL). Deren Straßennamen
   (blk5-Bereich) bilden die Straßenliste. Der Text hinter `#` wird an die Straßennamen
   angehängt.
3. **Hausnummer** (`FUN_00165a0c`):
   - Die Firmware geht die 8×8 D-Zellen der 4×4-Zelle durch, x außen, y innen, und darin die
     a1-Kanten.
   - Sie nimmt die **erste** Kante mit `[0]` oder `[1]` = Namens-Offset, bei der die Nummer n
     im Bereich einer Seite liegt, zuerst links `[2]`, dann rechts `[3]`.
   - Bedingung: `min ≤ n ≤ max` und gleiche Parität wie `min`. Mit Bit 15 im ersten Wert ist
     jede Parität erlaubt.
   - Position: Anteil `t = (n − min + 0,5) / (max − min + 1)` der Länge entlang der Geometrie.
     Fällt der Bereich (from > to), gilt `1 − t`. Ohne Nummer nimmt sie die Mitte (`t = 0,5`).

   Die Seite beeinflusst also nur die Paritätsprüfung. Die Position wird nicht zur Seite hin
   versetzt.

## D-Records: Straßen mit Hausnummern

Gleicher a1-Aufbau wie in osm (10 × u32), aber mit zusätzlicher Belegung:

| Feld | osm | **ta** |
|---|---|---|
| `[0]` | Name | Name (linke Seite) = Byte-Offset in blk5 des A-Records der 4×4-Elternzelle |
| `[1]` | = `[0]` | Name der rechten Seite. Weicht in 50.945 von 1.163.823 Fällen von `[0]` ab (Straßen auf Ortsgrenzen: gleicher Name, aber in der Straßenliste eines anderen Orts); 105.410 Kanten sind namenlos |
| `[2]` | `0xFFFF7FFF` | **Hausnummern links**: `u16 von \| u16 bis << 16`, `0xFFFF7FFF` = keine |
| `[3]` | `0xFFFF7FFF` | **Hausnummern rechts** |
| `[4]`, `[5]` | Knoten-IDs | Knoten-IDs, **je 8×8-Zelle** von 1 an durchnummeriert (geprüft: 57.985 IDs, jede nur an einer Position); keine Verbindung zum osm-Routing-Graphen |
| `[6]` | Flags | Flags (z. B. `0x280`), von der Suche nicht gelesen |
| `[7]` | Klasse · Länge | Klasse (0–9, **eigene Skala**) · Länge (gleiche Einheit wie osm) |
| `[8]` | Anzahl u32 der Geometrie | ebenso |

**Hausnummern-Wort:** Die unteren 16 Bit sind die Nummer am Anfang der Geometrie, die oberen
16 Bit die am Ende. Die Nummern können fallen (`52 → 50`). **Bit 15** der ersten Nummer
bedeutet „beide Paritäten“, etwa bei fortlaufender Nummerierung auf einer Seite (siehe
Suche; 5249 Seiten in DK).

**Links/rechts** gilt in Geometrie-Richtung. Geprüft mit OSM-Adressen (`denmark-220101`,
jede 20. D-Zelle): Adressen mit Kreuzprodukt < 0 (in u, v mit v nach Süden) liegen zu 97 %
in `[2]`, solche mit Kreuzprodukt > 0 zu 97 % in `[3]`.

Geprüft in Aarhus (Namen über die A-Records aufgelöst):

```
Kystvejen          L (57, 63)       Kystvejen        L (65, 65)
Nørreport          R (2, 6)         Nørreport        R (24, 24)
Marselis Boulevard L (52, 50)       Marselis Boulevard R (11, 3)
```

Klassen in ta: 0 (5298), 1 (1339), 2 (20.315), 3 (56.681), 4 (86.140), 5 (22.424),
6 (171.085), 7 (672.615), 8 (127.521), 9 (405). In der Aarhuser Innenstadt haben z. B.
Nørreport, Nørrebrogade und Hallandsgade Klasse 2. Die Skala entspricht also **nicht** der
osm-Straßenklasse. Vermutlich ist es die TomTom-„Functional Road Class“.

## A-Records: Orte und ihre Straßen

Aufbau wie in osm (siehe OSM_FORMAT.md „A-Records“), ein Eintrag mit dem Ländernamen
(`Denmark`, `[2] = 1`). blk5/blk6 enthalten die Straßennamen, auf die `a1[0]`/`a1[1]` zeigen.
Sie sind **nach Orten gruppiert**: Jedes 0x18-Unterelement beschreibt eine Gruppe (10.641 in DK):

| Feld | Inhalt |
|---|---|
| `[0]` | 0 (Zeiger auf den String) |
| `[1]` | Länge des Strings (UTF-16 inkl. NUL) |
| `[2]` | **Byte-Offset in blk5** des ersten Straßennamens der Gruppe |
| `[3]` | **Anzahl Straßennamen** der Gruppe |
| `[4]` | Breite als float32 (z. B. 56,93876) |
| `[5]` | Länge als float32 (z. B. 8,36874) |

Die Gruppen folgen in blk5 lückenlos aufeinander. Innerhalb einer Gruppe sind die Namen sortiert
(wie in osm). Derselbe Straßenname kann in mehreren Gruppen stehen und hat dann mehrere Offsets.
Eine Gruppe ohne String (19 Records, z. B. Autobahnen) steht immer zuerst.

Der String nennt die Orte der Gruppe als Alternativen, durch `|` getrennt, optional mit
`#Zusatz` am Ende:

```
Ejby, Wedellsborg (Fyn)|Ejby (Fyn)
[DANAalborg¦GERAalborg¦…], Svenstrup ([DANNordjylland¦…])|[DANAalborg¦…] ([DANNordjylland¦…])#Svenstrup
```

Jede Alternative ist genau der Name eines Treffers im Suchindex, z. B. „Kommune, Ortsteil
(Region)“. Mehrsprachige Namen haben die Form `[DANKøbenhavn¦GERKopenhagen¦…]`. Die
0x10-Unterelemente sind in DK nicht belegt.

## Suchindex (Zusatztabelle, Header `0x70`)

Die Tabelle liegt unverschlüsselt hinter den Records eines Tiles (DK: Tile (138,24), 2.596.612 B).
Kein Slot zeigt darauf. Die Firmware meldet sie beim Laden an (`FUN_003f282c` →
`FUN_0016068c`, nur bei Layer-Bit `0x10` und Dateiname mit `_ta.v`) und liest sie bei der
Suche direkt aus der Datei. Alle Offsets zählen ab Tabellenanfang.

```
u32   Offset des Wortpools (am Tabellenende)
u8    n Sprachen, n × (3 ASCII-Zeichen, u8 Index): DAN 1, GER 2, ENG 3, … POR 11
u32   1
u16   n + UTF-16 Ländername ("Denmark", muss dem Namen im A-Record entsprechen)
u32   Offset des Wurzelknotens
Knoten (Präfixbaum, pre-order: Knoten, dann die Teilbäume seiner Kinder)
Wortpool: je Wort u16 Bytelänge + UTF-8
```

**Knoten:**

```
u32   Kinderzahl << 24 | Zahl der verschiedenen Treffer im Teilbaum
      (0xFFFFFFFF: danach u32 Treffer, u32 Kinder)
n ×   u16 Zeichen, u64 Sprachmaske, u32 Offset des Kindknotens
u16   Zahl der Treffer an diesem Knoten, dann je Treffer:
      u8   Typ: Bit 7 = Name aus dem Wortpool
           0 = Ort mit Straßen, 1 = Postleitzahl, 2 = Ort ohne Straßen (nur Koordinaten)
      Name: Pool: u32 (Wortzahl << 24 | Offset), dann (Wortzahl − 1) × u32 Offset, mit
            Leerzeichen verbunden; sonst u16 Bytelänge + UTF-8
      f32 Länge, f32 Breite (bei Typ 1 nur, wenn das Datei-Magic ≥ 0x1B5D ist; 0x1B62 ist es)
      Typ 0 und 1: u8 Zellen (0xFF: danach u16), je Zelle u16 x, u16 y der 4×4-Zelle,
                   bei Typ 1 zusätzlich u16 k + k × u32 blk5-Offsets der Straßen
```

Der Pfad zu einem Knoten ist der **Suchschlüssel**. Jeder Treffer steht unter allen seinen
Schlüsseln:
- jedes Wort des Namens,
- jeder mehrteilige Namensteil zwischen den Kommas, ohne den Regionsteil in Klammern,
- je Sprachvariante.

Die Schlüssel sind kleingeschrieben, ohne Akzente (ø → o, æ → a) und mit Bindestrich als
Leerzeichen. Geprüft: Alle 13.121 dänischen Treffer haben genau diese Schlüssel
(`ta_index.fold`). Die Sprachmaske ist fast immer `0xFFFFFFFF` oder `0xFFFFFFFFFFFFFFFF`,
also alle Sprachen. Die Firmware vergleicht sie mit der aktuellen Sprache.

DK: 45.070 Knoten, 13.121 Treffer: 9146 Orte mit Straßen, 2887 ohne, 1088 Postleitzahlen.
Die Postleitzahlen haben Schlüssel wie `9990` und listen je Zelle die Straßen-Offsets.

## Nachbauen

```python
import sys; sys.path.insert(0, "tools")
import layers as L, ta_index
d = open(PFAD, "rb").read()
for tx, ty, cx, cy, g, raw in L.iter_records(d, "D"):
    assert L.build_d(L.parse_d(raw)) == raw
for tx, ty, cx, cy, g, raw in L.iter_records(d, "A"):
    assert L.build_a(L.parse_a(raw)) == raw
t = open("build/ref/ta_dk_extra.bin", "rb").read()   # d[header 0x70 : Tile-Ende]
assert ta_index.build(ta_index.parse(t)) == t
```

`tools/ta_lookup.py <ta> <Ort> <Straße> [<Nummer>]` spielt die Suche der Firmware nach (Ort →
Zellen → Straßenliste → erste passende Kante → Position).

## Aus OSM erzeugen

```bash
.venv/bin/python tools/osm_addr_extract.py osm_ref/great-britain-latest.osm.pbf build/gb/addr.pkl
.venv/bin/python tools/compile_ta.py --cache=build/gb/ta_cache.pkl --country=17 "--name=United Kingdom" \
    build/gb/ways.pkl build/gb/addr.pkl osm_ref/great-britain.poly build/gb/GreatBritain_ta.v20260919 20260919
```

Laufzeiten für Großbritannien:
- Extrakt: ca. 20 min.
- Compiler: 8 min beim ersten Lauf. Dabei entsteht der Cache (840 MB) mit den Straßenstücken und
  den zugeordneten Hausnummern.
- Mit vorhandenem Cache: 4 min, Speicherspitze 16 GB. Der Cache muss gelöscht werden, wenn sich
  `ways.pkl`, `addr.pkl` oder die Stück-/Zuordnungsregeln ändern.

Die D-Records bauen 8 Worker. Sie bekommen nur die Daten ihrer Zellen, denn geforkte Worker, die
die Stückliste teilen, kopieren sie und brauchen zusammen über 28 GB.

- `osm_addr_extract.py` speichert alle Adressen (`addr:housenumber` mit `addr:street`), die
  Interpolationslinien (`addr:interpolation`) und die Orte (`place=*` mit Namen).
- `compile_ta.py` nimmt die Straßen aus dem Pickle von `osm_extract.py` (wie `compile_osm.py`).

1. **Straßen:** benannte Straßen der osm-Klassen 0–8 (Autobahn bis Fußgängerzone), an Kreuzungen
   geteilt wie in osm und **zusätzlich an den D-Zellgrenzen**. So liegt jedes Stück in einer
   Zelle, und die Interpolation der Firmware nutzt den ganzen Bereich. ta-Klasse: motorway 0,
   trunk 1, primary 2, secondary 3, tertiary 4, residential/unclassified 6, pedestrian 8.
   Flags `0x280`.
2. **Hausnummern:**
   - Jede Nummer kommt zum nächsten Straßenstück gleichen Namens (höchstens 100 m entfernt).
   - Die Seite ergibt sich aus dem Kreuzprodukt.
   - Von/bis sind die Nummern am ersten und letzten Punkt entlang des Stücks. Die Richtung
     folgt der Korrelation.
   - Bei mehr als 10 % Nummern der anderen Parität wird Bit 15 gesetzt, sonst fallen diese
     Nummern weg.
   - `12a` zählt als 12, `12-16` als 12 und 16.
   - Interpolationslinien werden zu einzelnen Nummern aufgelöst.
3. **Orte:** Jedes Stück bekommt eine Siedlung (city/town/village/hamlet; kleinstes Verhältnis
   Abstand / Radius mit 12 / 5 / 2 / 0,8 km) und einen Ortsteil (suburb/quarter/neighbourhood,
   2,5 / 1,5 / 0,8 km). Dazu kommen bis zu 3 `addr:city`-Werte der Straße in der 4×4-Zelle
   (Poststädte). Stücke gleichen Namens mit Enden unter 60 m Abstand bilden eine Straße; sie
   bekommt eine gemeinsame Gruppe (Filter s. u.). Gruppenstring: `Stadt, Ortsteil|Stadt|Poststadt|…`.
   Die Firmware verkettet gleichnamige Straßen aus mehreren Zellen und probiert sie der Reihe
   nach (Kette bei `+0x108`/`+0x114` in `FUN_00165a0c`).
4. **Index:**
   - Typ 0: ein Treffer je Name und Ortsknoten, mit den Zellen, in denen er vorkommt.
   - Typ 2: die übrigen Orte (auch locality, isolated_dwelling, farm, island).
   - Typ 1: britische Postleitzahlbezirke (Outward Code, z. B. `SW1A`) mit ihren Straßen.
   - Namen stehen inline (ohne Wortpool), alle Sprachmasken voll, Sprachliste wie DK.

### In Rust

`rust/src/ta.rs` macht dasselbe ohne Pickle, mit `rust/src/grid.rs` statt scipys
`cKDTree`; siehe [../rust/README.md](../rust/README.md).

```bash
./target/release/teasi ta --country=17 "--name=United Kingdom" \
    osm_ref/great-britain-latest.osm.pbf osm_ref/great-britain.poly \
    build/gb/GreatBritain_ta.v20260919 20260919      # 3:19, 13,7 GB
```

Für Dänemark sind **alle 3958 Records und der Suchindex bytegleich** mit der
Python-Version, für Großbritannien 13.679 von 13.680 Records und beide Dateien auf das
Byte gleich groß; die eine Abweichung und 9 von 522.760 Index-Knoten gehen auf die
zwei Adressen zurück, die der Rust-Extraktor zusätzlich findet.

Drei Dinge mussten dafür auch in `compile_ta.py` geändert werden, damit zwei Läufe
überhaupt dasselbe liefern: Adressen, Orte und Interpolationslinien werden kanonisch
sortiert (der Extraktor liefert libosmiums Reihenfolge), die Kinder eines
Suchindex-Knotens nach Zeichen (vorher die Iterationsreihenfolge eines `set` — die
wechselt mit dem Hash-Seed), und eine Korrelation unter 10⁻¹² gilt als null
(`CORR_TOL`), statt mit ihrem letzten Bit über die Richtung eines
Hausnummernbereichs zu entscheiden.

**Abgleich an Dänemark:** Aus `denmark-220101` gebaut: 22,6 MB (Original 22,2 MB). An 3000
zufälligen OSM-Adressen mit der Firmware-Logik (`ta_lookup.py`) nachgeschlagen:

| | gefunden | Median | 75 % | 90 % | < 50 m |
|---|---:|---:|---:|---:|---:|
| Original (2018) | 2727 | 25 m | 47 m | 167 m | 70 % |
| neu aus OSM | 2882 | 24 m | 44 m | 111 m | 76 % |

Der Abstand enthält den Weg vom Hausmittelpunkt zur Straße. Nutzerablauf Ort (`addr:city`) →
Straße → Nummer (1500 Adressen mit Poststadt): 96 % gefunden, 76 % unter 50 m.

**Großbritannien (2026-09-19):** `GreatBritain_ta.v20260919`, 114 MB.

| Kennzahl | Wert |
|---|---|
| Straßen | 5,3 Mio. Stücke (915.000 Straßen) |
| Hausnummern (OSM) | 5,06 Mio., 97 % davon an 825.000 Stücken |
| Index | 48.352 Orte mit Straßen, 82.998 ohne, 2603 Postleitzahlbezirke (21 MB) |
| größter A-Record | 1,18 MB (London; deutsches Original 1,25 MB) |
| größter D-Record | 0,7 MB |

Nutzerablauf an 1500 OSM-Adressen: 94 % gefunden, Median 20 m, 86 % unter 50 m.

**Einschränkung:** OSM enthält für GB nur rund 5 Mio. der etwa 30 Mio. Adressen. Gut abgedeckt
sind u. a. Bristol, Nottingham, Coventry, Edinburgh und Teile Londons. Anderswo findet die Suche
Ort und Straße (fast vollständig), aber oft keine Hausnummer; dann landet man in der Mitte der
Straße.

Damit die A-Records klein bleiben, bekommt jede Straße nur ihren Hauptstadtteil, dazu die Orte
mit mindestens 10 % ihrer Länge und die Poststädte mit mindestens 25 % ihrer Adressen. Ohne
diese Filter hatte London 8558 Gruppen und 2,3 MB.

## Offen

1. Die Klassenskala 0–9 und die Flags `[6]`. Die Suche liest sie nicht; ob sie woanders wirken,
   ist offen.
2. Die Bedeutung von `#Zusatz` (in DK der Postort), das Feld „1“ im Indexkopf und die
   Sprachmasken ungleich „alle“.
3. Die 0x10-Unterelemente der A-Records.
