# Terrain-Format (`Denmark_terrain.v20210916`)

Die Terrain-Datei ist der einzige Layer, der **nicht** verschlüsselt ist. Sie besteht aus
JPEG-Kartenkacheln und JPEG-2000-Höhenkacheln. Die gemeinsame Hülle aller Chart-Dateien
(Header, Prüfsumme, Tile-Verzeichnis) und die Verschlüsselung der Vektor-Layer stehen in
**[KARTEN_ENTSCHLUESSELUNG.md](KARTEN_ENTSCHLUESSELUNG.md)**.

Stand: 2026-09-18. Kachel-Raster und Zoom-Pyramide sind gelöst, die Georeferenzierung ist offen.

## Hülle (Kurzfassung)

| Offset | Inhalt |
|---|---|
| 0x00 | Magic `0x1B62` |
| 0x04 | 48 B Salt + 16 B MD5-MAC (Integrität/Gerätebindung, **kein** Schlüssel) |
| 0x44 | Datum ASCII `20210916` |
| 0x4C | Typ = **5** (terrain) |
| 0x50 | 32 |
| 0x74 | Anzahl Tiles = **23** (x 122–139, y 19–25) |
| 0x78 | Verzeichnis `[u16 x][u16 y][u32 offset]`, danach direkt das erste Tile (0x130) |

## Aufbau eines Tiles (Region)

Das Layout ist dasselbe wie bei den Vektor-Layern. Der Tile-Loader `FUN_003e4460` liest der Reihe nach:

```
+0x0000  0x40 + 0x100 + 0x40 + 0x1000 B   Kopfbereiche; bei Terrain alle 0xFF (= 4480 B)
+0x1180  85 × 8 B  Zellen-Tabelle (s,d) für 8×8 + 4×4 + 2×2 + 1×1 = 85 Zellen   (JPEG)
+0x1428  85 × 4 B  zweite Tabelle (JP2/DEM-Offsets)
+0x157C  32 B      Blob (bei Vektor-Layern der verschlüsselte Record-Schlüssel)
+0x159C  Daten     erstes Bild beginnt immer hier (s_0 = 0x159C)
```

Die frühere Beschreibung „Slot-Array ab Region-Start, Liste ab Byte 4480" meint genau das:
Die 4480 `0xFF`-Bytes sind die bei Terrain leeren Kopfbereiche.

## Inhalt: 1132 JPEG-Kartenkacheln (256×256, Klartext)

`Denmark_terrain.v20210916` (9.08 MB) besteht fast vollständig aus
**1132 JPEG-Bildern** (SOI `FFD8FF`, EOI `FFD9`), je 1,7–18 KB,
**alle 256×256 px**. Jede Datei enthält die Standard-`Exif\0\0`-APP1
(TIFF-LE `II\2a\0`, IFD-Offset 8; IFD-Werte 96/96/Compression=2 sind
Platzhalter, keine Kachel-Koordinaten).

Inhalt (PIL-Pixelanalyse aller 1132):
- **Karten-Palette**: Wasser `RGB(150,180,206)` (651 Tiles),
  Land `RGB(225,227,203)` (~425), Vegetation `RGB(~163,195,148)`.
- 1037 Tiles mit ≥256 Farben → detaillierte Navigationskarte
  (Küsten, Straßen, Landbedeckung), keine Fotos.
- Beispiel: Tile 366 (0x2b6e3f) = 95 % Wasser, nur 19 Farben (Nord-/
  Ostsee); dunkelste Tiles (111–119) = Vegetationsdominiert.

→ Der terrain-Block je Verzeichnis-Tile = eine Sequenz dieser
256px-JPEGs plus davor liegende `0xFF`-Sparse-Index-Blöcke.
**Nichts verschlüsselt**; die hohe Entropie der „dense“-Blöcke ist
schlicht JPEG-Daten. 10 Beispiel-Tiles liegen in `terrain_tiles/`.

**Struktur je Verzeichnis-Tile** (23 Gruppen, 13–85 Sub-Tiles, Σ=1132):
1. **Slot-Array** (u32, beginnt **am Region-Start**): vorne viele leere
   Slots (`FFFFFFFF`, z. B. 4480 B = reiner `0xFF`-Bereich), dann ggf.
   **führende `(-3,0)`-Marker**, dann die JPEG-Liste — s. u.
2. JPEG-Sub-Tiles (256×256)
3. JP2/DEM-Sub-Tiles (256×256, 16 bit) — s. u.

Konstanten (alle 23 Regionen identisch): das **Slot-Array ist 5532 B groß**,
und die Liste beginnt immer bei **Byte 4480** (= 560 leere 8-Byte-Slots
`FFFFFFFF` davor). Die Maße (Rasterbreite 8, Ebenen 16/4/1) stehen
**nirgends** als Feld — Datei-Header hat nur Zähler (`0x74` = 23), die
JPEGs nur Platzhalter-EXIF (96/96), die JP2 nur Standard-Boxen
(`jP␣␣`/`ftyp`/`jp2h`/`jp2c`, ihdr 256×256). Die Geometrie ergibt sich
ausschließlich aus Liste + `(-3,0)`-Markern.

**Wichtigste Konstante:** die Liste beschreibt bei **jeder** Region genau
**85 Zellen** — 64 (Block 1) + 16 + 4 + 1. Parsen: Paare ab Byte 4480
lesen, `FFFFFFFF` = Ende, `(-3,0)` = leere Zelle, sonst Tile; **Stop,
sobald die Tile-Zahl der JPEG-Anzahl entspricht** (dahinter folgt die
JP2-Liste als einzelne u32-Offsets — als Paare gelesen entstehen sonst
Phantom-Tiles).

> Wichtig: der `0xFF`-Block ist **kein** eigenes Feld, sondern der leere
> Anfang des Slot-Arrays. Die führenden Marker gehören zur Liste und
> bedeuten leere Zellen am Anfang — wer sie ignoriert, verschiebt alle
> Kacheln. Beleg: (138,25) hat 4 führende Marker; mit ihnen ergibt sich die
> Zeilenbelegung `[4,3,4,4,4,4,5,5]`, was die unabhängige
> Naht-Korrelation bestätigt (ohne sie: `[4,3,4,4,4,5,5,4]` → Tile 23 saß
> falsch in Spalte 7).

### Index-Block: kumulatives `(s,d)`-Slot-Array (dekodiert)

Liegt unmittelbar vor dem ersten SOI; besteht aus 8-Byte-Slots
`(s_i, d_i)` (u32 LE), **kumulativ**: `s_{i+1} = s_i + d_i`.
`s` = Offset **relativ zum Region-Start**.

- **Slot-Typen**: `(s,d)` = Datensatz; `0xFFFFFFFD,0x00000000` (−3, 0) =
  Marker/Gruppengrenze; `0xFFFFFFFF,0xFFFFFFFF` = Listen-Ende/Leerslot.
- `s_0` ist **keine** Blockgröße, sondern der Offset des ersten JPEG
  (meist `0x159C` = 5532 → konstanter Header-/Index-Bereich davor).
- **N+1 Offsets** für N Sub-Tiles: der letzte Eintrag ist das End-Offset
  des letzten Sub-Tiles.
- **Zwei Listen** pro Region, getrennt durch den `0xFFFFFFFF`-Terminator,
  als Fortsetzung derselben kumulativen Kette:
  1. JPEG-Tiles (mit `(-3,0)`-Markern)
  2. **JP2/DEM-Tiles** (sparse, `0xFFFFFFFF`-Leerslots dazwischen).

Verifiziert für alle 23 Regionen: jeder dekodierte Offset trifft exakt ein
SOI (`okSOI = jpgs`). Beispiel (122,19), 13 JPEGs:
`0x159C (+0x0C5E) → 0x21FA (+0x0FCA) → 0x31C4 …`, absolut ab 0x130 =
0x16CC, 0x232A, 0x32F4 … = exakt die 13 SOIs. Marker-Gruppen dort:
1, 1, 3, 3, 1, 2, 2, 1.

### Zweite Bildebene: 801 JPEG-2000-Tiles = Höhenmodell (DEM)

Neben den 1132 JPEGs enthält die Datei **801 JP2-Codestreams**
(Signatur-Box `00 00 00 0C 6A 50 20 20 0D 0A 87 0A` = `jP␣␣`), je
**256×256 px, 16 Bit Graustufen (`I;16`)** → **Höhenmodell**, kein Bild.
Feld ist glatt (innerer Gradient ≈ 678/Spalte, Spannweite 0…65000),
bereichsweise 0 (Meer?). Pro Region z. B. (136,22): 83 JPEG + 62 JP2,
(134,24): 85 + 64, Σ JP2 = 801. Die JP2-Liste steht im selben
kumulativen Index (2. Liste); die Kettenwerte zeigen auf den
Record-Start, die JP2-Signatur folgt **+8 B**.

### Das Kachel-Raster einer Region (gelöst, final)

Jede Region besteht aus **genau 85 Index-Zellen** — bei allen 23 Regionen
identisch (verifiziert). Die Zellen sind eine **Mipmap-Pyramide
derselben Fläche**:

| Zellen | Ebene | Raster | Inhalt |
|---|---|---|---|
| 0–63 | **Block 1** (feinste) | 8 × 8 | Karte 1:1, DEM 1:1 |
| 64–79 | Ebene 1 | 4 × 4 | dieselbe Fläche, halbe Auflösung |
| 80–83 | Ebene 2 | 2 × 2 | Viertel-Auflösung |
| 84 | Ebene 3 | 1 × 1 | Overview der ganzen Region |

Füllregeln (alle Ebenen gleich):

- **zeilenweise, Zeile 0 = unten (Süd)**, in der Zeile von West nach Ost;
- `0xFFFFFFFD, 0x00000000` = **leere Zelle** (kein Tile);
- ein Tile steht genau dann in einer Zelle, wenn der **2×2-Block der
  darunterliegenden Ebene mindestens ein Tile enthält** (dynamisch!).
  Verifiziert: Ebene 1 **21/23**, Ebene 2 **22/23**, Ebene 3 **22/23**
  Regionen stimmen exakt mit dem Index überein. Einzige bekannte
  Abweichung: (133,24) — dort sitzt ein einzelnes Tile isoliert unterhalb
  des Blocks, die Ebene-1-Zelle (Zeile 2, Spalte 1) bleibt trotzdem leer;
- die Summe der Tiles ergibt **exakt** die JPEG-Anzahl jeder Region
  (23/23). Bilder: `terrain_tiles/final_{x}_{y}.jpg`
  (Panels: Block 1 | 4×4 | 2×2 | Overview, je auf 2048 px skaliert).

> **Wichtig (früherer Fehler):** Block 1 endet **nicht** beim letzten Tile,
> sondern nach **64 Zellen** — die Löcher der letzten Zeile gehören zu
> Block 1. Rechnet man nur bis zum letzten Tile, verschiebt sich der
> gesamte Tail. `n_jp2` (DEM-Anzahl) ist nur *meist* gleich der
> Block-1-Tilezahl ((135,22): 18 vs. 16, (137,24): 44 vs. 43).

Die Pyramide ist **kein** separates Patch: Template-Match gegen Block 1
ergibt Tail-Fine vs. Block 1/2 = **+0.808**, Overview vs. Block 1/8 =
**+0.792** (NCC). Das früher angenommene „Block 2 = eigene Fläche / 5×5-
Patch" war ein Artefakt vermischter Zoom-Stufen.

### Rotation und Kachelreihenfolge im Raster (gelöst)

- **Beide Ebenen sind 90° CW gespeichert** → Restore mit `np.rot90(a,1)`
  (CCW) / PIL `rotate(90)`. Zwei unabhängige Belege:
  - Wasser-/Land-Nahtmetrik (JPEG): 621 vs. Random-Baseline 7238 (~10×)
  - DEM-Kantenkorrelation: 0.811 (43/61 Paare > 0.9) vs. Random 0.02
- **Reihenfolge** (verifiziert): zeilenweise **von unten nach oben (S→N)**,
  in der Zeile **links→rechts (W→E)**, **8 Spalten**.
  Nachweis je Region über Naht-Scan (Horizontalnaht innerhalb der Zeile +
  Vertikalnaht zur Nachbarzeile), Score `H + max(V)`:
  | Region | bester W | Score | Zeile 0 |
  |---|---|---|---|
  | (136,22) Block 1 | 8 | 0.786 | unten (S) |
  | (134,25) Block 1 | 8 | 0.234 (V=0.61) | unten (S) |
  Zeilengrenzen von (136,22) Block 1 liegen bei **6, 14, 22, 30, 38, 46,
  54, 62** (erste Zeile nur 6 Tiles, letzte nur 5) — nicht bei Vielfachen
  von 8! Ein Mosaic mit `i//8` ist deshalb falsch (Zeilen ab Tile 6
  verschoben).
- **Verifikation durch Bildanalyse** (qwen27b):
  - `block1_136_22.jpg` (8×8, Zeile 0 unten): **COHERENT** — Küsten,
    Seen und Relief laufen über die Kachelgrenzen; nur die 2 fehlenden
    Kacheln (unten rechts) sind grau.
  - `reg_134_25.jpg` (8×8): **COHERENT**.
  - Bilder in `terrain_tiles/`.
- Die JP2-Tiles sind **pro Tile affin normiert** (eigener Scale/Offset) →
  Nahtvergleich nur über **Korrelation** (nicht absolute Differenz).

Warum die ersten Mosaics „zerhackt“ wirkten: falsche Rotation *und*
Zeilen von oben statt unten *und* 8×12 statt 8 Spalten *und* Ebenen
vermischt. Die von qwen27b erkannten Stilbrüche (Hillshade vs. Straighten/
Wegenetz, Flächen-Fills) sind Layer-/LoD-Wechsel bzw. No-Data-Zellen.

### Rezept: eine Region ohne Kantenvergleich zusammenbauen

Das Layout steht **vollständig im Index** — Kantenvergleich dient nur noch
der Verifikation (Rotation, Plausibilität). Schritte:

1. **Verzeichnis** ab `0x78`: Records `(x:u16, y:u16, off:u32)`;
   Region-Ende = `off` des nächsten Records (letzte Region = Dateiende).
2. **Slot-Array ab dem Region-Start** als 8-Byte-Slots `(s,d)` lesen,
   kumulativ (`s_{i+1} = s_i + d_i`), `s` relativ zum Region-Start.
   Führende `FFFFFFFF`-Slots überspringen (leerer Array-Anfang); die
   Liste beginnt beim ersten Slot ≠ `FFFFFFFF` — **führende Marker
   mitzählen** (s. o.):
   - `(s,d)` → **Tile** bei `region_off + s`
   - `(-3, 0)` = `0xFFFFFFFD, 0x00000000` → **übersprungene Zelle**
     (Lücke im Raster, kein Tile)
   - `0xFFFFFFFF, 0xFFFFFFFF` → Listenende
   - der **letzte** Tile-Slot ist nur das End-Offset (N+1) → verwerfen.
3. **Zeilenbreite** ergibt sich aus den Markern: die Tiles vor dem ersten
   Marker plus die Marker-Anzahl = eine volle Zeile.
   (136,22): 6 Tiles + 2 Marker = **8**; Regionen mit vollem Raster
   ((134,24), (134,25): 64 Tiles) haben **gar keine** Marker.
4. **Rotation:** jedes Tile ist 90° CW gespeichert → `rotate(90)`
   (PIL, = CCW) bzw. `np.rot90(a, 1)`.
5. **Block 1** = die Tiles in den **ersten 64 Zellen** (8 × 8 Raster).
   Platzierung: **8 Spalten**, zeilenweise (in der Zeile W→E),
   **Zeile 0 = unten (Süd)**; an jedem `(-3,0)`-Marker eine Zelle leer
   lassen. Fehlende Zellen bleiben grau. (`n_jp2` = DEM-Anzahl ist nur
   *meist* identisch mit der Block-1-Tilezahl — nicht als Grenze benutzen!)
6. **Tail** = Zellen 64–84 → Pyramide:
   64–79 → 4×4, 80–83 → 2×2, 84 → Overview; je zeilenweise, Zeile 0 unten.
   Die Tiles werden **fortlaufend** weitergezählt (Block 1 belegt
   `jpeg[0..k-1]`, der Tail `jpeg[k..]`).
7. **Verifikation** (optional): normierte Kreuzkorrelation der
   **gemeinsamen** Kanten — horizontal `rechts(A) ↔ links(B)`,
   vertikal `oben(unten) ↔ unten(oben)`; als affininvariantes Merkmal die
   Wasser-Maske `(B − R) > 20`. DEM-Kanten sind wesentlich deutlicher
   (mean 0.811) als JPEG-Kanten (mean 0.023).

**Stolperfallen** (waren die Ursache für „zerhackte" Mosaics):

- `i // 8` ist falsch, sobald eine Zeile Lücken hat — die Zeilengrenzen von
  (136,22) liegen bei 6, 14, 22, …, weil die **erste** Zeile nur 6 Tiles hat.
- Zoom-Stufen bzw. Blöcke nicht vermischen (Block 1 ist eine andere Fläche
  als die Block-2-Pyramide).
- Flache Tiles (offenes Wasser, Binnenland) liefern keine Korrelation
  (Std ≈ 0 → Sentinel) — Naht-Tests dort nicht interpretieren.
- DEM-Tiles sind pro Tile affin normiert → nur Korrelation, nie absolute
  Differenz.

**Georeferenzierung — Update 2026-09-18:** Die Vektor-Layer bestätigen das equirektanguläre
1,40625°-Raster (`lon = x·1,40625 − 180`, `lat = 90 − y·1,40625`): Leuchttürme in osmpoint
liegen damit auf ~10 m genau, Tile (122,19) enthält die Färöer. Siehe
[KARTEN_ENTSCHLUESSELUNG.md](KARTEN_ENTSCHLUESSELUNG.md), Abschnitt 1a. Anker 2 (Oslo) war
damit falsch, und die „verworfene Hypothese“ unten ist richtig. Wie die 8×8-Terrainkacheln
genau im Tile liegen (Rotation, Zeile 0 = Süd), muss dagegen noch geprüft werden.

**Georeferenzierung (historisch, Stand vor dem Update):**

- **Anker 1 (Nutzer):** Tile 684 (global #684, 1-basiert = 69. Tile der
  Region (136,22) = **Index 68**, 0-basiert) ≈ **58.1904365 N,
  11.6903675 E**. Index 68 liegt in **Block 2**, nicht im 8×8-Raster.
- **Anker 2 (Nutzer, visuell):** `reg_134_25_preview.jpg` (Block 1 von
  Region (134,25), korrekt zusammengesetzt) zeigt **Norwegen und einen Teil
  Schwedens, Oslo (59.91 N, 10.75 E) in der Mitte**.
- **Verworfene Hypothese:** Verzeichnis-Koordinaten als equirechteckiges
  Raster mit 1.40625°-Zellen (`lon = x·1.40625 − 180`,
  `lat = 90 − y·1.40625`). Sie erklärt Anker 1 ((136,22) = 11.25–12.66 E,
  57.66–59.06 N = „Schweden über Göteborg", vom Nutzer bestätigt), aber
  **nicht** Anker 2: dort läge (134,25) bei 8.44–9.84 E / 53.44–54.84 N
  (Deutsche Bucht) statt bei Oslo. Ein gemeinsamer linearer Fit
  (Zellgröße, Ursprung) aus beiden Ankern liefert keine plausiblen Werte
  (Δx=2 → Δlon≈1.25°, Δy=−3 → Δlat≈−1.4° ⇒ ~0.63°/Schritt in Länge,
  ~0.47°/Schritt in Breite).
- **Fester Anker aus der Datei (neu!):** jedes JP2/DEM-Tile trägt eine
  `res `/`resc`-Box (Capture-Resolution). Wert bei **allen 801** Tiles
  identisch: `18576 / 65532 × 10^4` = **2834.646 px pro Einheit**
  (raw `48 90 ff fc 48 90 ff fc 04 04`). Liest man die Einheit als Grad,
  ergibt sich **1 Kachel = 256 / 2834.646 = 0.090311° = 5.4187′ ≈ 10 km**
  und damit 8×8 = **0.7225° pro Region**. (Als px/m gelesen ergäbe sich
  0.35 mm/Pixel — unplausibel, daher Grad-Annahme; die Einheit ist im
  JP2-Standard „pixels per meter", der Wert hier also mit Vorsicht.)
- **Nächster Ansatz:** Übersichtsbild aller 23 Regionen an ihren
  Verzeichnis-Positionen (je Block-1-Mosaic verkleinert) rendern und
  geografisch identifizieren lassen → Koordinaten aus Landmarken fitten.

## Höhenmodell: Lage und Werte (gelöst 2026-09-19)

Geprüft wurde Region (135,24) (Fünen) gegen das Copernicus-DEM GLO-90, Skripte liegen im Scratchpad
der Sitzung.

- **Nur Ebene 0:** Alle 801 JP2 stehen in den Zellen 0–63 der zweiten Tabelle (`+0x1428`).
  Die Pyramide gibt es nur für die JPEGs.
- **Lage (anders als bei den JPEGs!):** Zelle k liegt in Spalte `k // 8` von West und Zeile
  `k % 8` von **Nord**, also spaltenweise von NW nach Süden. Die Kachel ist **nicht gedreht**
  (Pixelzeile 0 = Norden). Die mittlere Korrelation mit Copernicus ist 0,975; alle anderen
  Varianten liegen unter 0,14.
- **Record:** `[u16 a0][u16 a1][u32 Länge]`, danach die JP2-Datei (`jP  `, `ftyp`,
  `jp2h` mit `ihdr` 256×256×1 16 Bit, `colr`, `res `, dann `jp2c`). Der Codestream wurde mit
  JasPer 1.701 erzeugt: eine Kachel, 5 Zerlegungsstufen, 9/7 irreversibel, Codeblöcke 64×64,
  verlustbehaftet. Ein Record ist 1113–4201 B lang, und das Maximum steht im Header bei 0x6C
  (4201).
- **Höhe in m:** `h = (v / a0 + a1) / 6 − 1000`. Dabei ist `a1` der Sockel in 1/6 m mit
  1000 m Versatz und `a0` die Werte pro 1/6 m. Die gepoolte freie Anpassung ergibt
  `(v/a0 + a1 − 5990) / 5,94`. Mit den runden Konstanten weicht die Formel im Median um 2,1 m ab,
  mit Copernicus um +1,7 m höher, was als Oberflächenmodell (Bäume, Häuser) zu erwarten ist.
- Die Firmware kennt den Speicher `HeightMapCacheMemory` und einen eigenen JP2-Decoder
  („Failed to decode jp2 structure“).
- Header 0x54 = 4 (Landescode DK), wie bei den Vektor-Layern. 0x6C ist die größte JP2-Länge `n`
  (4201).
- **Record-Ende:** Auf die JP2 (endet mit `FFD9`) folgen noch 2 Bytes. Meist ist das `a0` des
  nächsten Records, in 117 von 779 Fällen ein anderer Wert. Das sind offenbar Pufferreste; der
  nächste Record beginnt dahinter.
- Eine Region ganz ohne Kacheln ((122,20)) hat nur den Kopf (5532 B). Die JPEG-Tabelle darf
  also komplett leer sein (`(-3,0)` in allen 85 Zellen).

**Kartenbilder, Lage (2026-09-19, gegen das Höhenmodell geprüft):** Die JPEGs liegen genau wie
die Höhenkacheln: Zelle k in Spalte `k // 8` von West, Zeile `k % 8` von Nord, **nicht gedreht**.
Das gilt auch für die Pyramide: 4×4 bei 64 + `c*4 + r`, 2×2 bei 80 + `c*2 + r`, dann 84. Geprüft
über die Korrelation mit dem verkleinerten 8×8-Mosaik (spaltenweise 0,85–0,91, zeilenweise etwa 0).
Die ältere Deutung oben („90° CW gedreht, Zeile 0 = Süd“) beschreibt dasselbe Mosaik, nur als Ganzes
gedreht; sie ist ohne Georeferenz entstanden. Eine gröbere Zelle hat ein Bild, wenn eines ihrer vier
Kinder eins hat. In Dänemark stimmen 482 von 483 Zellen, in Norwegen 12736 von 12789.
Die JPEGs sind Baseline mit JFIF und Exif (96 dpi), 4:2:0 und IJG-Qualität 80. Die Records der JPEGs
stehen in Zellenreihenfolge, danach folgen die Höhen-Records.

**Compiler** `tools/compile_terrain.py` (2026-09-19):
- Ohne `--land`/`--area` bleiben alle JPEG-Zellen leer; das Höhenprofil funktioniert damit schon.
- Mit `--land`/`--area` entsteht pro Region ein 2048×2048-Bild:
  - Das Meer kommt aus den Landpolygonen.
  - Wasser, Wald, Heide, Fels, Sand, Feuchtgebiet, Siedlung und Acker kommen aus den OSM-Flächen.
  - Darüber liegt eine Reliefschattierung aus dem DEM: Licht aus NW, 45°, Überhöhung 1,5,
    Faktor 0,4–1,12.
  - Das Bild wird in 256er-Kacheln zerlegt, dazu kommen die Pyramidenstufen (Lanczos).
- Bilder bekommen alle Zellen, die das Polygon berühren oder Land enthalten.
- Höhenkacheln gibt es nur für Zellen mit Land (Höhe > 0).
- Die Höhen werden bilinear aus dem `dem_heights.py`-Raster gelesen. `a1` ist das Minimum und
  `a0 = 65535 // Spanne` in 1/6 m.
- Kodiert wird mit OpenJPEG (Pillow): `irreversible`, 6 Auflösungen, 64×64, LRCP, Verhältnis 50.
  COD und SIZ sind identisch mit dem Original, der Fehler liegt bei etwa 0,5 m RMS. Die
  JP2-Boxen davor werden aus dem Original kopiert.
- Der Blob ist wie bei den Vektor-Layern `encrypt_blob` eines Zufallsschlüssels.

### In Rust

`rust/src/terrain.rs` macht dasselbe ohne Pickle; `rust/src/raster.rs` baut die drei
Pillow-Teile nach (Polygonfüller, Lanczos-Skalierung, Schattierung — alle drei bitgleich)
und ruft OpenJPEG für die Höhenkacheln auf. Siehe [../rust/README.md](../rust/README.md).

```bash
./target/release/teasi terrain --country=17 \
    --land=osm_ref/land-polygons-split-4326/land_polygons.shp \
    --area=osm_ref/great-britain-latest.osm.pbf \
    build/gb/dem.bin osm_ref/great-britain.poly \
    build/gb/GreatBritain_terrain.v20260919 20260919      # 1:24, 7,9 GB
```

Verglichen wird mit `rust/scripts/terrain_compare.py`, nicht mit `teasi check` — der
Layer hat keine Slot-Bereiche. Für Dänemark wie für Großbritannien sind **alle
Höhenkacheln** (1034 bzw. 1999) byteidentisch mit der Python-Version, bis auf ein Byte:
OpenJPEG schreibt seine eigene Version in den COM-Marker des Codestreams. Die
Kartenbilder gehen durch einen anderen JPEG-Encoder (`jpeg-encoder` statt
libjpeg-turbo); etwa die Hälfte ist trotzdem pixelgleich, die mittlere Abweichung
beträgt 0,025 von 255, und die Dateien sind drei Hunderttausendstel kleiner.

## Umfeld-Dateien (Kontext)

- `acsldata.dat` (67 B): `LBnXSdsP6rB8D5CiDqd9w9qv0xs5CH2o` \n
  `marius1692@web.de` \n\n `193088` \n — Lizenz-/Zugangsdaten
  (Key, E-Mail, ID).
- `lastdevice.dat`: `2013021200000368` (Device-ID = Name des Datenordners).
- `settings/customroutecache*.rti` (30 Dateien, 28–52 KB): von der gleichen
  Engine geschrieben, **im Klartext**:
  - Magic `B18EDA7A` (LE `7ada8eb1`), dann u32 ≈ 6907 (0x1B5B —
    gleicher „Header-Größe“-Gedanke wie die Chart-Header!),
  - UTF-16-Strings („Koldinghus 0 ist ein anderes Königsschloss in der
    Stadt K…“), u32-Paare die wie Offsets in eine größere DB aussehen
    (0x1E7B1F44, 0x0860xxxx).
  → Der Engine-Zwischenspeicher ist nicht verschlüsselt (anders als die Chart-Records: PC1 + LZMA).
- `Tahuna.rar` (112 MB, RAR5): nur `TrackImages/Tourbook_*.tourbook.png`.
- `monitor.sqlite`: Tabellen (Maps, StorageDevices, …) **alle leer**.

## Offene Punkte

1. **Georeferenzierung**: Das Tile-Raster ist geklärt (siehe Update oben). Offen ist, wie die
   Kacheln innerhalb eines Tiles liegen.
2. Ebene-1-Regel bei (133,24) (eine Zelle weicht ab).
3. ~~DEM: Wertebereich/Einheit~~ gelöst, siehe „Höhenmodell: Lage und Werte“.
4. Header-Felder 0x50–0x70 (bei Terrain u. a. `0x6C = 4201`).
5. `settings/customroutecache*.rti` (Magic `B18EDA7A`) dokumentieren.

Skripte zum Rendern: `attic/terrain_render/` (`render_final.py`, `region_full.py`,
`tail_render.py`, `overview_fix.py`; Ausgabe in `terrain_tiles/`).
