# Teasi-Kartendateien: Verschlüsselung und Werkzeuge

Stand: 2026-09-18. Gilt für die Chart-Dateien `<Land>_<Layer>.vJJJJMMTT` (Magic `0x1B62`)
des Teasi PRO (`bikenav.exe` 4.4.1.0, WinCE/ARM), z. B.
`2013021200000368/7/943/20317/Denmark_osm.v20210916`.

> Kurzfassung: Die Karten sind **nicht** mit einem eigenen Codec geschützt. Es sind drei
> Standardbausteine: eine **MD5-Prüfsumme** im Header, **PC1-Verschlüsselung**
> (Pukall Cipher 1, 256 Bit) mit teilweiser Verschlüsselung der Nutzdaten und
> **LZMA**-Kompression. Alle 14.185 Records der Vektor-Layer (osm, osmarea, osmpoi, osmpoint,
> ta) lassen sich damit entschlüsseln und entpacken.
>
> Der umgekehrte Weg funktioniert ebenfalls: `tools/writer.py` baut gültige Dateien, und ein
> neu gebautes Testpaket läuft auf dem Gerät (2026-09-18). Damit keine Warnung erscheint,
> müssen Größe und MD5 in `BikeNav/packages.xml` angepasst werden (Abschnitt 5.4).

Überblick über das Repo und den Weg von OSM zur Karte: [../README.md](../README.md).

---

## 1. Dateiaufbau (äußere Hülle)

```
0x00  u32     Magic 0x00001B62
0x04  48 B    Salt (zufällig)                 ┐ Header-Prüfsumme,
0x34  16 B    MAC = MD5(...)                  ┘ siehe Abschnitt 2
0x44  8 B     Datum ASCII, z. B. "20210916"
0x4C  u32     Typ (1 = osm/osmarea/osmpoi/osmpoint, 2 = ta, 5 = terrain)
0x50  u32     Layer-Bitmaske (osm 1, osmarea 4, osmpoi 8, ta 0x12, terrain 0x20, osmpoint 0x400)
0x54  u32     Land (DK 4, DE 7, NO 12, UK 17), wird gegen eine Freischalttabelle geprüft (Index < 0x153)
0x58  u32     Puffergröße: größte gepackte Record-Länge (len)
0x5C  u32     größte entpackte Größe (pltx) in Slot-Bereich D
0x60  u32     ebenso Slot-Bereich C
0x64  u32     ebenso Slot-Bereich B
0x68  u32     ebenso Slot-Bereich A
0x6C  u32     0 (Puffergröße, in den Vektor-Layern ungenutzt)
0x70  u32     Offset einer Zusatztabelle, nur bei Layer-Bit 0x10 (ta), sonst 0xFFFFFFFF
0x74  u32     Anzahl Tiles N
0x78  N × 8 B Tile-Verzeichnis: [u16 x][u16 y][u32 Datei-Offset], aufsteigend
```

So liest der Chart-Loader `FUN_003f282c` den Header. Der **Ländercode** ist der Index in
der Länderliste der Firmware (UTF-16-Strings in `bikenav.exe`: Andorra 1, Austria 2,
Belgium 3, Denmark 4, Finland 5, France 6, Germany 7, Ireland 8, Italy 9, Luxembourg 10,
Netherlands 11, Norway 12, Portugal 13, San Marino 14, Spain 15, Sweden 16,
United Kingdom 17, …); DE 7 und NO 12 bestätigen das. Nur Länder mit gesetztem Byte in der
Freischalttabelle (`*DAT_003f3f64 + 0x324 + code`) werden geladen; UK (17) läuft auf diesem
Gerät (2026-09-18). Die Felder `0x58`–`0x6C` sind
Puffergrößen: Die Firmware nimmt je Layer das Maximum über alle geladenen Dateien. Größere Werte
als nötig sind harmlos (osmpoi und osmpoint haben bei `0x58` mehr als ihr größter Record),
kleinere dürfen nicht vorkommen. Das **Datum** entscheidet, welche Datei gilt: Ist für
Layer und Land schon eine Datei mit gleichem oder neuerem Datum geladen, wird die neue
übersprungen.

Ein Tile endet dort, wo das nächste beginnt (das letzte am Dateiende).

### Innenaufbau eines Tiles (Offsets relativ zum Tile-Start)

```
+0x0000  16 × u32   Slot-Bereich A (4×4 Unterzellen)    ┐ Record-Offsets relativ zum
+0x0040  64 × u32   Slot-Bereich B (8×8)                │ Tile-Start, 0xFFFFFFFF = leer,
+0x0140  16 × u32   Slot-Bereich C (4×4)                │ siehe Abschnitt 1a
+0x0180  1024 × u32 Slot-Bereich D (32×32)              ┘
+0x1180  85 × 8 B   Zellen-Tabelle der Zoom-Pyramide 8×8 + 4×4 + 2×2 + 1×1
+0x1428  85 × 4 B   zweite Zellen-Tabelle (u32); beide nur bei Terrain belegt, in den
                    Vektor-Layern leer: (0xFFFFFFFF, 0) bzw. 0xFFFFFFFF
+0x157C  32 B   Blob = verschlüsselter Record-Schlüssel des Tiles
+0x159C  Records als lückenlose Kette bis zum Tile-Ende
```

### Record

```
[u32 len][u32 pltx][payload: len Bytes]
```

- `len`: Länge der verschlüsselten, komprimierten Nutzdaten
- `pltx`: Größe **nach** dem Entpacken (immer gerade)
- Nächster Record: `offset + 8 + len`. `tools/chart.py` folgt dieser Kette, die Firmware
  springt dagegen über die Slots (Abschnitt 1a) direkt zum Record. Beides ergibt dieselbe Menge:
  Jeder Record steht in genau einem Slot.

### Die Dänemark-Dateien im Überblick

| Datei | Größe | Typ (0x4C) | 0x50 | Tiles | Records |
|---|---:|---:|---:|---:|---:|
| `Denmark_osm.v20210916` | 62.976.522 | 1 | 1 | 22 | 5878 (davon 406 unverschlüsselt) |
| `Denmark_osmarea.v20210810` | 19.916.110 | 1 | 4 | 24 | 368 |
| `Denmark_osmpoi.v20210915` | 1.621.671 | 1 | 8 | 17 | 3704 |
| `Denmark_osmpoint.v20210916` | 206.699 | 1 | 1024 | 17 | 110 |
| `Denmark_ta.v20180608` | 22.242.944 | 2 | 18 | 14 | 4125 |
| `Denmark_terrain.v20210916` | 9.081.123 | 5 | 32 | 23 | – (Bilder) |

Geprüfte Details:

- `len` zählt die 8 Header-Bytes **nicht** mit. Für alle aufeinanderfolgenden Records gilt
  `start[i+1] = start[i] + 8 + len[i]`, geprüft über alle Layer.
- Der erste Verzeichnis-Eintrag von `osmarea` ist ein **Platzhalter** `(0,0)` mit Größe 0,
  die 23 echten Tiles folgen danach.
- Tiles mit demselben Blob bilden räumlich zusammenhängende Gruppen. Weil PC1 pro Record neu
  startet, beginnen alle Records einer Gruppe mit demselben Chiffrat-Byte. Das war früher als
  „Typ-Byte" gedeutet worden.
- Header `0x58`–`0x70`: siehe oben. Bei osm, osmarea und ta passen die pltx-Felder exakt
  zu den größten Records, `0x58` ist bei osm und osmarea genau die größte Record-Länge.
- In `ta` endet die Record-Kette in Tile (138,24) bei `+0x361FC`. Dort beginnt die
  **Zusatztabelle** (Header `0x70`), die bis zum Tile-Ende reicht (2.596.612 B). Sie ist der
  landesweite **Suchindex** der Adresssuche (Präfixbaum über die Orts- und
  Postleitzahlnamen, siehe [TA_FORMAT.md](TA_FORMAT.md)). Sie ist kein Record, aber ihr
  erstes u32 sieht wie eine Record-Länge aus. `chart.py` läuft deshalb hinein und meldet
  einen „failed“-Eintrag; maßgeblich sind die Slots.
- Die DE- und NO-Ordner (`20322`, `20339`) enthalten nur Dateien mit 0 Byte.

### 1a. Slot-Bereiche, Record-Arten und Koordinatensystem

**Raster.** Die Welt ist equirektangulär in 256 × 128 Tiles zu je **1,40625°** geteilt
(`lon = x·1,40625 − 180`, `lat = 90 − y·1,40625`, y zählt von Nord nach Süd). Jedes Tile ist
noch einmal in Unterzellen geteilt. Welche Teilung gilt, hängt vom Slot-Bereich ab. Ein Record
enthält immer die Daten **genau einer Unterzelle**. Den Slot einer Unterzelle findet man so:

```
slot = (cell_x mod g) · g + (cell_y mod g)       g = 4 (A, C), 8 (B), 32 (D)
cell_x = tile_x · g + slot // g,  cell_y = tile_y · g + slot % g
```

| Bereich | Tile-Kopf | Raster | Leser (Firmware) | genutzt von |
|---|---|---|---|---|
| A | `+0x000` | 4×4 | `FUN_003e6044` | osm (161), ta (101) |
| B | `+0x040` | 8×8 | `FUN_003e5d20` | osm (406, **unverschlüsselt**) |
| C | `+0x140` | 4×4 | `FUN_003e52d0` (osmpoint) / `FUN_003e651c` (osm, osmarea) | osm (105), osmarea (368), osmpoint (110) |
| D | `+0x180` | 32×32 | `FUN_003e56fc` | osm (5206), osmpoi (3704), ta (4024) |

Die Leser prüfen `cell_x < 0x400, cell_y < 0x200` (4×4) bzw. `< 0x2000, < 0x1000` (32×32).

**Koordinaten in den Records** sind relativ zur Nordwest-Ecke der Unterzelle:
`u32 = (v << 16) | u`, u nach Osten, v nach Süden. **Eine Unterzelle ist immer 32768 Einheiten**
breit, die Einheit ist also 360°/2²⁵ (≈ 1,2 m) bei 4×4-Zellen und 360°/2²⁸ (≈ 15 cm) bei
32×32-Zellen. Gerundet wird auf die nächste Einheit; die Punkte sind dann **exakt OSM-Knoten**
(Abgleich mit dem Geofabrik-Extrakt `denmark-220101`, `tools/osm_extract.py`).

| Layer / Bereich | Rand | geprüft |
|---|---|---|
| osmpoi (D), osmpoint (C), osmarea (C) | keiner | exakt auf OSM-Knoten |
| osm D, osm C, ta D (Linien) | **512 Einheiten**: gespeichert `u = x + 512`, Bereich 0 … 33.792 | osm exakt auf OSM-Knoten, ta nur über den Wertebereich |
| osm B (Routing-Graph, 8×8) | keiner, aber **65.536** Einheiten pro Zelle (360°/2²⁷) | ±1–2 Einheiten |

`layers.to_latlon(cx, cy, g, p, margin=layers.MARGIN)` bzw. `layers.b_node_latlon`. Das Raster
passt auch zur Terrain-Hypothese mit 1,40625° (Tile (122,19) enthält die Färöer).

**Gemeinsame Merkmale aller Record-Inhalte:**

- Vorne steht ein Kopf aus u32-Feldern. `[0]` und `[1]` überschreibt die Firmware beim Laden
  mit `cell_x`/`cell_y`, `[3]` ist immer die Anzahl des ersten Arrays. Zeigerfelder sind in
  der Datei 0.
- Die Struktur-Arrays sind meist **byteweise transponiert**, Strings sind UTF-16LE mit
  Längenangabe in Zeichen inklusive NUL.
- Formate der einzelnen Layer:
  [OSMPOINT_FORMAT.md](OSMPOINT_FORMAT.md) · [OSMPOI_FORMAT.md](OSMPOI_FORMAT.md) (mit dem
  allgemeinen D-Record) · [OSM_FORMAT.md](OSM_FORMAT.md) (mit Geometrie, A- und B-Records) ·
  [OSMAREA_FORMAT.md](OSMAREA_FORMAT.md) (mit dem allgemeinen C-Record) ·
  [TA_FORMAT.md](TA_FORMAT.md) · [TERRAIN_FORMAT.md](TERRAIN_FORMAT.md).
- **Alle 14.185 Records** aller Vektor-Layer lassen sich mit `tools/layers.py` parsen und
  **bytegleich** wieder bauen (Round-Trip-Test).

Die Terrain-Datei (Typ 5) ist anders aufgebaut und **unverschlüsselt** (JPEG-Kacheln und
JPEG-2000-Höhenmodell), siehe [TERRAIN_FORMAT.md](TERRAIN_FORMAT.md).

---

## 2. Header-Prüfsumme (Integrität und Gerätebindung)

Funktion `FUN_00105d04`, aufgerufen aus dem Chart-Loader `FUN_003f282c`.

```
SECRET = "d8ethebrestezexaqathaTrepedEkubafr5vuhaprupe3ucUphedeyuhaGespenU"   (64 B, VA 0x4b7768)

generisch:       MAC == MD5(salt ‖ SECRET ‖ file[0x44:0x444] ‖ file[-0x400:])
gerätegebunden:  MAC == MD5(salt ‖ SECRET ‖ file[0x44:0x444] ‖ file[-0x400:] ‖ device_id)
```

- `device_id` ist die Seriennummer als ASCII, hier `2013021200000368`
  (siehe `firmware/deviceid.dat`, `firmware/license.dat`).
- Alle vorhandenen Dateien (DK und DE) sind **gerätegebunden** signiert.
- Ist eine Datei **generisch** signiert, bindet `FUN_001061bc` sie beim ersten Öffnen selbst
  ans Gerät: Sie erzeugt ein neues Zufalls-Salt und schreibt den MAC mit der Seriennummer neu.
  Selbst gebaute Dateien können also generisch (für jedes Gerät) oder direkt
  gerätegebunden signiert werden.
- Weil der MAC das erste und letzte KB der Datei abdeckt, muss er **nach** jeder Änderung
  neu berechnet werden.

---

## 3. Entschlüsselung eines Records

```
K     = globaler Schlüssel (32 ASCII-Bytes, Abschnitt 4)
blob  = tile[+0x157C : +0x159C]
rk    = PC1_decrypt(blob, K)                  alle 32 Bytes          (FUN_002897b0)
pt    = PC1_decrypt_partial(payload, rk)      Bytes 0..99, danach    (FUN_002898ec)
                                              jedes 10. Byte (100, 110, 120, …)
raw   = LZMA1 raw decode(pt)                  exakt pltx Bytes       (FUN_00374958 = LzmaDecode)
        Props-Byte 0x5D (lc=3, lp=0, pb=2), Dictionary 16 MiB (0x01000000)
```

Hinweise:

- **Nicht alle Records sind verschlüsselt:** Die 406 osm-Records aus Slot-Bereich **B** sind
  nur mit LZMA gepackt (sie beginnen mit `00 00 6C …`). Ihr Leser ruft `FUN_002898ec`
  (`FUN_003e5d20`) nicht auf. `decode_record()` probiert erst PC1 + LZMA, dann nur LZMA.
- Für jeden Record startet PC1 neu mit `rk`, der Schlüsselstrom setzt also pro Record zurück.
  Deshalb ist das erste Chiffrat-Byte innerhalb eines Tiles immer gleich: Das erste
  LZMA-Byte ist immer `0x00`.
- Alle Bytes, die nicht an den Positionen 0..99 oder 100 + 10·k liegen, sind reiner
  LZMA-Strom.
- Den Aufbau des entpackten Inhalts beschreibt Abschnitt 1a, Details stehen in den
  Layer-Dokumenten.

### PC1 im Detail

Das ist Standard-PC1 mit 256-Bit-Schlüssel (Konstanten `0x4E35`, `0x015A`), implementiert
in `FUN_00289660` (assemble) und `FUN_002896c8` (code). Pro Byte gilt:

```
inter = assemble()                       16 Runden über die 16 Schlüsselwörter
c     = c XOR (inter >> 8) XOR (inter & 0xFF)
key[i] ^= c   für i = 0..31              beim Entschlüsseln mit dem KLARTEXT
```

Beim Verschlüsseln wird der Schlüssel zuerst mit dem Klartext-Byte verändert, danach wird
das Byte ausgegeben. `si`, `x1a2` und `i` starten mit 0.

---

## 4. Globaler Schlüssel K

Funktion `FUN_002050a0`. Das Ergebnis landet in einem 0xA8-Byte-PC1-Kontext bei
`ctx+0x88`, mit `ctx = *(*DAT_0055d504 + 0x1d8)`. Den Code dazu (`0x1fd148`) hat Ghidra
keiner Funktion zugeordnet.

```
if device_id[:8] in PRAEFIXE:
    K = "E89ACE5CE51E0669B4BA068CE8F63990"            (statisch, VA 0x484f08)
else:
    K = MD5(device_id[:8]) als Großbuchstaben-Hex    ("%02X" × 16 = 32 Zeichen)
```

`PRAEFIXE` (19 Einträge, Tabelle bei VA `0x484e24`):
`20130125, 20130212, 20130213, 20130807, 20131010, 20131020, 20131026, 20150215,
20160505, 20160509, 20161014, 20161028, 20170606, 20170707, 20180914, 20181225,
20190320, 20190415, 20190618`

Dein Gerät `2013021200000368` hat das Präfix `20130212` und nutzt deshalb den statischen
Schlüssel.

---

## 5. Werkzeuge

Alle Python-Tools liegen in `tools/`. `chart.py`, `pc1.py`, `layers.py`, `writer.py`,
`roundtrip.py`, `packages.py`, `poly.py` und die Compiler `compile_*.py` brauchen nur die
Standardbibliothek (`hashlib`, `lzma`), `osm_extract.py`, `osm_poi_extract.py` und `osm_area_extract.py` zusätzlich
`osmium` und `numpy`, `compile_osmarea.py` und `compile_osm.py` `shapely`, `osm_heights.py`,
`compile_osm.py` und `compile_osmarea.py` `scipy`, `land_extract.py` `pyshp`,
`dem_heights.py` `tifffile` + `imagecodecs` (`requirements.txt`). Zum Ausführen das Projekt-venv
verwenden (`.venv/bin/python`).

Die Seriennummer des Geräts steht an einer Stelle: `DEVICE` in `tools/chart.py`, überschreibbar
mit der Umgebungsvariablen `TEASI_DEVICE`. Alle Funktionen mit einem `device`-Parameter nehmen
sie als Standardwert.

### 5.1 `tools/chart.py`: Karten lesen und entschlüsseln

**Als Kommandozeilen-Tool:** Es entschlüsselt alle Records einer Datei und schreibt sie als
einzelne Dateien.

```bash
.venv/bin/python tools/chart.py 2013021200000368/7/943/20317/Denmark_osm.v20210916 out/osm
```

Ausgabe:

```
header MD5 (device-bound): True | generic: False
NNN records written, 0 failed      (bei ta: 1 failed = Zusatztabelle, s. o.)
```

Dateinamen: `out/osm/<x>_<y>_<offset-im-tile-hex>.bin`. Das ist der entpackte Klartext
(`pltx` Bytes). Die Seriennummer kommt aus `DEVICE` (`TEASI_DEVICE`, siehe Abschnitt 5).

**Als Modul:**

```python
import sys; sys.path.insert(0, "tools")
from chart import tiles, records, decode_record, global_key, header_md5

d   = open("2013021200000368/7/943/20317/Denmark_osmpoi.v20210915", "rb").read()
dev = b"2013021200000368"
K   = global_key(dev)

assert header_md5(d, dev) == d[0x34:0x44]         # Header-Prüfsumme prüfen

for x, y, start, end in tiles(d):                  # Tile-Verzeichnis
    blob = d[start + 0x157C : start + 0x159C]
    for rel, ln, pltx, payload in records(d, start, end):
        raw = decode_record(payload, pltx, blob, K)   # bytes oder None
```

| Funktion | Zweck |
|---|---|
| `tiles(d)` | liefert `(x, y, start, end)` für jedes Tile |
| `records(d, start, end)` | liefert `(rel_offset, len, pltx, payload)` entlang der Record-Kette ab `+0x159C` |
| `decode_record(payload, pltx, blob, K)` | PC1 → PC1-partiell → LZMA, sonst nur LZMA; `None` bei Fehler |
| `lzma_unpack(buf, size)` | nur LZMA-Schritt |
| `global_key(device_id)` | Schlüssel K nach Abschnitt 4 |
| `header_md5(d, device=b"")` | MAC nach Abschnitt 2 (ohne `device` = generisch) |
| `SECRET`, `STATIC_KEY`, `KNOWN_PREFIXES` | Konstanten aus der Firmware |

### 5.2 `tools/pc1.py`: PC1-Verschlüsselung

```python
from pc1 import decrypt_blob, encrypt_blob, decrypt_payload, encrypt_payload
```

| Funktion | Zweck |
|---|---|
| `decrypt_blob(blob, key)` / `encrypt_blob(data, key)` | alle Bytes (für den 32-B-Tile-Blob) |
| `decrypt_payload(data, key)` / `encrypt_payload(data, key)` | teilweise: 0..99, dann jedes 10. Byte |
| `PC1(key)` | Low-Level-Klasse mit `dec_byte()` / `enc_byte()` |

Die Implementierung ist gegen den emulierten ARM-Code (`attic/seed_hunt.py`, Unicorn) geprüft:
Die Ergebnisse sind bytegleich.

### 5.3 `tools/writer.py`: Karten schreiben

Die Umkehrung von `chart.py`: Aus den Klartext-Records pro Tile und Slot baut `write_chart`
eine komplette Datei. Dazu gehören Tile-Köpfe mit Slots, leere Zellen-Tabellen und ein neuer
Record-Schlüssel pro Tile. Die Records werden per LZMA gepackt und mit PC1 verschlüsselt,
außer in Slot-Bereich B. Den Header füllt `write_chart` mit den Puffergrößen, einem neuen Salt
und der gerätegebundenen MAC.

```python
import sys; sys.path.insert(0, "tools")
from writer import write_chart

meta  = {"date": b"20260918", "type": 1, "layer": 8, "country": 4}      # osmpoi
tiles = [(tx, ty, {"D": {slot: klartext, ...}}, b""), ...]            # Verzeichnis-Reihenfolge
d = write_chart(meta, tiles, device=b"2013021200000368")               # bind=False: generisch
```

- Records stehen pro Tile in der Reihenfolge A, B, C, D, jeweils nach Slot sortiert, wie im
  Original.
- `tiles`-Eintrag `(x, y, None, b"")` = leeres Platzhalter-Tile (wie `(0,0)` in osmarea). Die
  ta-Zusatztabelle wird als `tail` übergeben, zusammen mit `meta["tail_tile"] = (x, y)`.
- LZMA: `lzma.compress` (liblzma) hängt einen End-Marker an, die Originale haben keinen. Das
  ist unkritisch: Die Firmware nutzt unverändertes `LzmaDecode` aus dem LZMA-SDK 9.x
  (`FUN_0037be30`, Props `5D 00 00 00 01`), das nach `pltx` Bytes aufhört und dann 0 (OK)
  liefert; akzeptiert werden 0 und 6.
- Die Tiles werden parallel gebaut. osm braucht rund 70 s, weil PC1 in reinem Python läuft.

**Round-Trip** (`tools/roundtrip.py <datei> [<ausgabe>]`): Die Datei wird komplett
entschlüsselt, neu gebaut und wieder gelesen. Geprüft wird, dass jeder Record denselben
Klartext hat, die MAC stimmt, Verzeichnis, Zusatzdaten und Header-Felder `0x44`–`0x57`
übereinstimmen. Ergebnis (2026-09-18) für alle fünf Vektor-Layer:
**14.185 Records identisch**, die pltx-Puffergrößen gleich dem Original.

Ein Testpaket fürs Gerät liegt in `build/test1/`: alle Layer neu gebaut, in osmpoi heißt der
POI „Tivoli“ (Kopenhagen) „Tivoli TEASI-TEST“. **Auf dem Gerät getestet (2026-09-18):** Die
Karte wird normal angezeigt, der umbenannte POI erscheint. Damit ist die ganze Kette bestätigt
(PC1, LZMA mit End-Marker, neue Record-Schlüssel, gerätegebundene MAC). Zusätzlich muss
`packages.xml` angepasst werden, siehe 5.4.

### 5.4 Auf das Gerät bringen: `packages.xml` und `tools/packages.py`

Das Teasi meldet sich per USB als Massenspeicher („SiRF GPS HH“, Windows CE). Der Speicher ist
erst lesbar, nachdem man die Verbindung **auf dem Display bestätigt** hat; vorher meldet er
eine ungültige Größe und Linux bekommt nur I/O-Fehler. Danach erscheint ein FAT-Laufwerk
`TFAT` (3,8 GB):

```
BikeNav/Map/Countries/<Land>_<Layer>.vJJJJMMTT    Kartendateien (auch DE, NO, SE vollständig)
BikeNav/packages.xml                             Installationsliste der Tahuna-Software
BikeNav/Program/bikenav.exe, gpstuner.dat …      Firmware und Ressourcen (Texte, Schriften)
```

**Prüfung beim Start.** `bikenav.exe` geht alle Kartenpakete in `packages.xml` durch
(`FUN_0028937c` → `FUN_002891d8` → `FUN_00288b04`) und prüft jede Datei:

| Prüfung | Funktion | Bedingung |
|---|---|---|
| Größe | `FUN_002887c0` | `<size>` = Dateigröße |
| Prüfsumme | `FUN_00288858` | `<md5>` = `MD5(datei[0x44:0x444] + datei[-0x400:])` |

Die MD5 läuft also nur über 1 KB ab `0x44` (hinter Salt und MAC) und das letzte KB. Deshalb
bleibt sie gültig, wenn die Firmware eine Datei ans Gerät bindet und Salt und MAC neu schreibt.
Geprüft an allen sechs Original-Dänemark-Dateien. Schlägt eine Prüfung fehl, erscheint
„Karten nicht korrekt installiert. Teasi mit dem Computer verbinden und Karten erneut laden!“
(Textschlüssel `error_message_mappackage_tahuna` in `gpstuner.dat`, Aufruf in
`FUN_001faa98`). Die Karte wird trotzdem geladen und angezeigt.

**Vorgehen** nach dem Ersetzen von Kartendateien (Originale und `packages.xml` vorher sichern,
z. B. nach `build/device_backup/`):

```bash
C=/run/media/$USER/TFAT/BikeNav
cp build/test1/Denmark_osm.v20210916 $C/Map/Countries/
.venv/bin/python tools/packages.py $C/packages.xml $C/Map/Countries/Denmark_osm.v20210916
```

`packages.py` ändert nur `<md5>` und `<size>` der passenden `<file>`-Einträge (Zuordnung über
das Ende von `<url>`). **Dateien ohne Eintrag** werden nicht geprüft, aber trotzdem geladen:
Die Firmware liest alle `*.v*` in `Countries` (so laufen die Großbritannien-Dateien, 5.7). Mit geänderten Einträgen verschwindet die Meldung (auf dem Gerät
geprüft, 2026-09-18).

### 5.5 `tools/osm_extract.py`: OSM-Vergleichsdaten

Liest einen Geofabrik-Extrakt und speichert alle relevanten Ways (Straßen, Wege, Gewässer,
Bahn, Landnutzung …) mit Tags, Knoten-IDs und Koordinaten in Teasi-Einheiten (360°/2²⁸) sowie
die Rad- und Wanderrouten-Relationen je Way. Für Dänemark dauert das rund 1 Minute.

```bash
.venv/bin/python tools/osm_extract.py osm_ref/denmark-220101.osm.pbf osm_ref/dk_ways.pkl
```

Der Extrakt `osm_ref/denmark-220101.osm.pbf` (Stand 1.1.2022, der nächstgelegene zu den
Karten von 2021) stammt von `download.geofabrik.de/europe/denmark-220101.osm.pbf`. Eine
Teasi-D-Kante ordnet man einem Way zu, indem man ihre Punkte umrechnet
(`X = cell_x·32768 + u − 512`) und mit `round(X_osm)` vergleicht.

### 5.6 Compiler: Layer aus aktuellen OSM-Daten

| Layer | Werkzeuge | Stand |
|---|---|---|
| osmpoi | `osm_poi_extract.py` → `compile_osmpoi.py` | fertig, am Original geeicht, siehe [OSMPOI_FORMAT.md](OSMPOI_FORMAT.md) „Aus OSM erzeugen“ |
| osmpoint | `osm_poi_extract.py` → `compile_osmpoint.py` | fertig, am Original geeicht, siehe [OSMPOINT_FORMAT.md](OSMPOINT_FORMAT.md) „Aus OSM erzeugen“ |
| osmarea | `osm_area_extract.py` → `compile_osmarea.py` | fertig, am Original geeicht, siehe [OSMAREA_FORMAT.md](OSMAREA_FORMAT.md) „Aus OSM erzeugen“ (braucht `shapely`); Meer außerhalb der Grenze und die Färöer kommen aus der Originaldatei |
| osm | `osm_extract.py` (+ `osm_heights.py`) → `compile_osm.py` | fertig, am Original geeicht, siehe [OSM_FORMAT.md](OSM_FORMAT.md) „Aus OSM erzeugen“; mit Linksabbiegen und Anstiegen im Routing-Graphen; Färöer aus der Originaldatei; Karte auf dem Gerät OK |
| ta | `osm_addr_extract.py` (+ Straßen aus `osm_extract.py`) → `compile_ta.py` | Adresssuche: Orte, Straßen, Hausnummern, Postleitzahlbezirke und Suchindex, am Original geeicht, siehe [TA_FORMAT.md](TA_FORMAT.md) „Aus OSM erzeugen“; Prüfung offline mit `ta_lookup.py` |
| terrain | `dem_heights.py` (+ `land_extract.py`, `osm_area_extract.py`) → `compile_terrain.py` | Höhenmodell (Höhenprofil) und Kartenbilder (Reliefschattierung mit Landbedeckung), siehe [TERRAIN_FORMAT.md](TERRAIN_FORMAT.md) |

Alle Compiler nehmen `--country=N`; osm und osmarea laufen auch ohne Originaldatei (`-`),
siehe 5.7.

Vorgehen je Layer: Compiler auf `denmark-220101` laufen lassen und Objekt für Objekt mit der
Originaldatei vergleichen (Regeln eichen), dann auf `denmark-latest` umstellen. Die Dateinamen
bleiben die alten, damit `packages.xml` nur neue Größen/MD5 braucht (5.4); das Datum im Header
ist das Erzeugungsdatum. `tools/poly.py` liest die Geofabrik-Grenze (`osm_ref/denmark.poly`), die
Compiler übernehmen nur Objekte darin. Die Färöer sind im Geofabrik-Extrakt nicht enthalten und
fehlen deshalb in den neu gebauten Layern.

### 5.7 Großbritannien (England, Schottland, Wales)

Stand 2026-09-18, Ländercode **17** (United Kingdom), Dateinamen `GreatBritain_<layer>.v20260918`,
ohne Eintrag in `packages.xml`. Quelle: Geofabrik `great-britain-latest.osm.pbf` (2,2 GB) und
`great-britain.poly`, dazu die Landpolygone von osmdata.openstreetmap.de und das Copernicus-DEM.
terrain (`GreatBritain_terrain.v20260919`) und ta (`GreatBritain_ta.v20260919`, Adresssuche)
werden ebenfalls erzeugt, siehe unten.

| Schritt | Befehl (Details in den Layer-Docs) | Zeit | RAM |
|---|---|---:|---:|
| POIs | `osm_poi_extract.py --filter` → `compile_osmpoi.py --country=17` / `compile_osmpoint.py --country=17` | 24 + 1 min | 6 GB |
| Straßen | `osm_extract.py --filter`, `dem_heights.py`, `compile_osm.py --country=17 "--name=United Kingdom" … -` | 12 + 1 + 18 min | 18 GB |
| Flächen | `osm_area_extract.py`, `land_extract.py`, `compile_osmarea.py --country=17 --land=… -` | 10 + 12 min | 15 GB |

Ergebnis: osm 531 MB (12,1 Mio. Graph-Knoten, 30 Mio. Kanten, 57 % mit Anstieg, 33 % der
Knoten mit Linksabbiegen), osmarea 83 MB, osmpoi 17 MB (750.000 POIs), osmpoint 0,5 MB
(12.561 Seezeichen). Auf dem Gerät: POIs und Straßen in London geprüft.

**Achtung, Record-Größen:** 3 B-Records (Nord-London, Manchester, bis 10,8 MB entpackt) und
12 D-Records (bis 3,2 MB) sind größer als alles in der deutschen Originalkarte (6,5 / 1,3 MB).
Falls das Gerät dort hängt: dichte Zellen verkleinern (z. B. Fußwege nicht routen).

**terrain (2026-09-19):** Ohne terrain-Datei zeigt das Höhenprofil einer Tour nichts an.
`GreatBritain_terrain.v20260919` (37 MB, 85 Regionen) enthält die Höhenkacheln aus `build/gb/dem.pkl`
und die Kartenbilder:
```bash
.venv/bin/python tools/compile_terrain.py --land=build/gb/land.pkl --area=build/gb/area.pkl \
    build/gb/dem.pkl osm_ref/great-britain.poly build/gb/GreatBritain_terrain.v20260919 20260919   # ~2 min, 10 GB
```
Die Kartenbilder sind eine Reliefschattierung, eingefärbt nach Wasser und Landbedeckung (OSM-Flächen);
Irland und Frankreich am Rand bekommen nur Relief.
Rücklese-Prüfung der Höhen: Ben Nevis 1292 m (echt 1345), Snowdon 1025 m (1085), Trafalgar Square 19 m.
Die Gipfel sind durch die Glättung etwas zu niedrig. Auf dem Gerät: Höhenprofil OK mit der ersten
Version (nur Höhen), Kartenbilder und Höhenprofil der Version mit Bildern OK (2026-09-19).

**ta (2026-09-19):** Adresssuche mit landesweitem Ortsindex. Ohne diese Datei findet die Suche
nur Orte und Straßen in der Nähe der aktuellen Position.
```bash
.venv/bin/python tools/osm_addr_extract.py osm_ref/great-britain-latest.osm.pbf build/gb/addr.pkl   # ~20 min
.venv/bin/python tools/compile_ta.py --cache=build/gb/ta_cache.pkl --country=17 "--name=United Kingdom" \
    build/gb/ways.pkl build/gb/addr.pkl osm_ref/great-britain.poly build/gb/GreatBritain_ta.v20260919 20260919  # 4 min mit Cache
```
114 MB. Hausnummern gibt es nur, wo OSM Adressen hat (5 von ca. 30 Mio.). Details und Prüfung
stehen in [TA_FORMAT.md](TA_FORMAT.md). Auf dem Gerät kopiert, Test steht aus.

### 5.8 Ghidra-Hilfsskripte

Die Headless-Skripte liegen in `ghidra_scripts/`, Aufruf und Argumente stehen in
[../ghidra_scripts/README.md](../ghidra_scripts/README.md). Analysiert wurde `bikenav.exe`
in einem eigenen Ghidra-Projekt (Ghidra 12); die Einmal-Suchen aus der Analysephase liegen
in `attic/ghidra/`.

Code, den Ghidra keiner Funktion zugeordnet hat, findet nur `InsnGrep` (im Output `in ?`).
Dort sitzt z. B. die Schlüssel-Initialisierung (`0x1fd148`). Solche Stellen lassen sich mit
Capstone disassemblieren.

---

## 6. Wichtige Adressen in `bikenav.exe`

| Adresse | Bedeutung |
|---|---|
| `FUN_003f282c` | Chart-Loader: Magic, Header, Tile-Verzeichnis |
| `FUN_00105d04` | Header-MAC prüfen (generisch / gerätegebunden) |
| `FUN_001061bc` | generische Datei ans Gerät binden (neu signieren) |
| `FUN_002050a0` | globalen Schlüssel K ableiten |
| `0x1fd148` | `new(0xA8)` PC1-Kontext, K nach `ctx+0x88` (keiner Funktion zugeordnet) |
| `FUN_003e4460` | Tile laden, Blob bei `+0x157C` lesen und entschlüsseln |
| `FUN_003e6044` / `003e5d20` / `003e52d0` / `003e651c` / `003e56fc` | Record-Leser für Slot-Bereich A / B (Routing-Graph) / C (osmpoint) / C / D (Abschnitt 1a) |
| `FUN_002704f0` | Array zurücktransponieren |
| `FUN_003cf4d8` | Routing-Kostenfunktion: Flags, Kostenfaktor (Bits 20–23, Tabelle VA `0x4fc040`), Anstieg (Kante `[2]`), Linksabbiegen (Knoten `[2]`) |
| VA `0x4fbb50` | Tabelle POI-Typ → `poitype_*`-Name (75 Einträge) |
| `FUN_003a25c8`, `FUN_003a20c0` | Bild-Dekodierung (RGB565), ruft ebenfalls `FUN_002898ec` auf |
| `FUN_00289660` / `FUN_002896c8` | PC1 assemble / code |
| `FUN_002897b0` | PC1 vollständig (32 B) |
| `FUN_002898ec` | PC1 teilweise (0..99, dann jedes 10. Byte) |
| `FUN_0028937c` / `FUN_00288b04` | Kartenpakete aus `packages.xml` prüfen (Größe `FUN_002887c0`, MD5 `FUN_00288858`), siehe 5.4 |
| `FUN_00374958` / `FUN_0037be30` | `LzmaDecode` (LZMA-SDK 9.x, Wrapper / Implementierung) |
| `FUN_001b2f9c` / `FUN_001b3070` / `FUN_001b2564` | MD5 update / final / transform |
| VA `0x4b7768` | SECRET (64 Zeichen) |
| VA `0x484e24` | Seriennummer-Präfixtabelle (19 × 12 B); `+0xe4` statischer Schlüssel, `+0x108` `"%02X"` |

---

## 7. Sackgassen (nicht wieder aufgreifen)

- Die Codec-Familie `FUN_00354610` / `FUN_00354c34` / `FUN_00354f8c` gehört zum **RFP0**-Format,
  nicht zu den Charts.
- XOR-Suchen, die Hypothese „nur Entropiecodierung", zlib/Deflate: alles widerlegt. Die
  gleichmäßige Byteverteilung kommt von LZMA plus PC1.
- `0x1B62` und `0x159C` tauchen nicht als Konstanten im Code auf. Das liegt daran, dass
  gelesen wird, nicht daran, dass nichts geprüft würde.

---

## 8. Nächste Schritte

1. **Klartext-Strukturen je Layer**: Container überall gelöst (Round-Trip bytegleich). Per
   OSM-Abgleich zugeordnet: Straßenklassen, fast alle Flag-Bits, Linientypen, Flächenklassen,
   Routing-Graph. Reste stehen unter „Offen“ in den Layer-Docs.
2. ~~Georeferenzierung~~: gelöst (Abschnitt 1a, inkl. Rand von 512 bei Linien).
3. ~~Routing-Graph finden~~: Slot-Bereich B von osm, siehe [OSM_FORMAT.md](OSM_FORMAT.md).
4. **OSM → Teasi-Compiler** (5.6): Schreiber, Round-Trip und Gerätetest erfolgreich (5.3, 5.4),
   osmpoi-, osmpoint-, osmarea- und osm-Compiler fertig, osm mit Linksabbiegen und Anstiegen;
   Gerätetest des Routings steht aus.
5. **Großbritannien** (5.7): alle vier OSM-Layer, terrain (Höhen und Kartenbilder) und ta
   (Adresssuche) gebaut und auf dem Gerät; das Höhenprofil funktioniert. Offen sind der
   Routing-Test in London/Manchester (große Records) und der Test der Adresssuche.
