# teasi (Rust)

Portierung der Werkzeugkette nach Rust. Fertig sind **die Hülle** (Stufe 1:
Entschlüsselung, Kompression, alle Record-Container, der Schreiber, der Suchindex),
**das Lesen von OSM** (Stufe 2: PBF-Leser, Knoten-Index, Adressextraktion), **alle fünf
OSM-Layer-Compiler** (Stufe 3: `osmpoi`, `osmpoint`, `osmarea`, `osm` und `ta`, aus dem
PBF direkt in die Kartendatei) und **der Layer `terrain`** (Stufe 4: Höhenmodell und
Kartenbilder). Die Python-Werkzeuge in [../tools/](../tools/) bleiben die Referenz; was
hier steht, muss dasselbe liefern.

Nur die Höhenquellen selbst bleiben in Python (`osm_heights.py` mit scipys `lsqr`,
`dem_heights.py` mit dem Copernicus-Modell); ihre Ergebnisse liest Rust, s. u.

## Stand

| Baustein | Python | Rust |
|---|---|---|
| PC1, MD5-Prüfsumme, globaler Schlüssel | `pc1.py`, `chart.py` | `pc1.rs`, `chart.rs` |
| Rohes LZMA1 wie die Originale | `chart.py` | `lzma.rs` |
| Container A, B, C, D, osmpoint | `layers.py` | `layers.rs` |
| Dateien schreiben, Gerätebindung | `writer.py` | `writer.rs` |
| Suchindex der Adresssuche | `ta_index.py` | `ta_index.rs` |
| OSM-PBF lesen, Knoten-Index | pyosmium | `pbf.rs` |
| Adressen, Orte, Interpolationswege | `osm_addr_extract.py` | `addr.rs` |
| Landesgrenze (`.poly`) | `poly.py` | `poly.rs` |
| POIs lesen | `osm_poi_extract.py` | `poi.rs` |
| Layer `osmpoi` bauen | `compile_osmpoi.py` | `osmpoi.rs` |
| Layer `osmpoint` bauen | `compile_osmpoint.py` | `osmpoint.rs` |
| Flächen und Küstenlinie lesen | `osm_area_extract.py` | `area.rs` |
| Weltweite Landpolygone (Shapefile) | `land_extract.py` | `land.rs` |
| Geometrie (GEOS) | shapely | `geos.rs` |
| Layer `osmarea` bauen | `compile_osmarea.py` | `osmarea.rs` |
| Straßen- und Linien-Ways lesen | `osm_extract.py` | `way.rs` |
| Höhen einlesen und abfragen | `osm_heights.py`, `dem_heights.py` | `heights.rs` |
| Layer `osm` bauen | `compile_osm.py` | `osm.rs` |
| Nachbarsuche (statt scipys kd-Baum) | `scipy.spatial` | `grid.rs` |
| Layer `ta` bauen (Adresssuche) | `compile_ta.py` | `ta.rs` |
| Zeichnen, Skalieren, JPEG, JPEG 2000 | Pillow, OpenJPEG | `raster.rs` |
| Layer `terrain` bauen | `compile_terrain.py` | `terrain.rs` |

## Bauen und prüfen

```bash
cargo build --release
cargo test                 # Vergleichswerte aus den Python-Werkzeugen
TEASI_GEOS=… cargo test    # dazu die zwei GEOS-Tests (sonst übersprungen, s. u.)
```

Der Build braucht einen C-Compiler: `xz2` übersetzt liblzma mit, `openjpeg-sys`
OpenJPEG (für die Höhenkacheln des terrain-Layers). libgeos dagegen wird **nicht**
mitgebaut, sondern zur Laufzeit geladen, s. u.

Der eigentliche Abnahmetest läuft gegen echte Karten (die nicht im Repo liegen):

```bash
./target/release/teasi check <verzeichnis>/Denmark_*.v2*
```

`check` entschlüsselt jeden Record, zerlegt ihn und baut ihn wieder zusammen; das
Ergebnis muss Byte für Byte dem Original entsprechen. Für die dänische Karte sind
das alle **14.185 Records** (osm 5878, ta 4125, osmpoi 3704, osmarea 368,
osmpoint 110) in 1,3 s.

| Befehl | Zweck |
|---|---|
| `teasi info <karte>…` | Header, Kacheln, Record-Zahlen je Slot-Bereich |
| `teasi check <karte>…` | jeden Record zerlegen und bytegleich neu bauen |
| `teasi roundtrip <karte> [aus]` | ganze Datei entschlüsseln, neu schreiben, Records vergleichen |
| `teasi md5s <karte>` | `Bereich cx cy md5` je Record — zum Abgleich mit Python |
| `teasi dump <karte> <ordner>` | entschlüsselte Records als Einzeldateien |
| `teasi index <ta-karte>` | Suchindex zerlegen und bytegleich neu bauen |
| `teasi addr <datei.osm.pbf> [aus]` | Adressen, Orte und Interpolationswege aus OSM |
| `teasi poi <datei.osm.pbf> <aus>` | POI-Kandidaten als kanonischer Dump |
| `teasi osmpoi <pbf> <poly> <karte> [datum]` | den osmpoi-Layer bauen (`--country=N`) |
| `teasi osmpoint <pbf> <poly> <karte> [datum]` | den osmpoint-Layer bauen (`--country=N`) |
| `teasi area <datei.osm.pbf> <aus>` | Flächen und Küstenlinie als kanonischer Dump |
| `teasi land <land_polygons.shp> <poly>` | Zahl und Fläche der weltweiten Landpolygone |
| `teasi osmarea <pbf> <poly> <original\|-> <karte> [datum]` | den osmarea-Layer bauen (`--country=N`, `--land=…`) |
| `teasi ways <datei.osm.pbf> <aus>` | Straßen- und Linien-Ways als kanonischer Dump |
| `teasi osm <pbf> <poly> <original\|-> <karte> [datum]` | den osm-Layer bauen (`--country=N`, `--name=…`, `--heights=…`) |
| `teasi ta <pbf> <poly> <karte> [datum]` | die Adresssuche bauen (`--country=N`, `--name=…`) |
| `teasi terrain <höhen> <poly> <karte> [datum]` | Höhenmodell und Kartenbilder bauen (`--land=…`, `--area=…`, `--rate=R`, `--only=x,y`) |

Die Seriennummer kommt wie bei den Python-Werkzeugen aus `TEASI_DEVICE`
(Standard: die in `chart.rs`).

## Gemessen

Gegen die dänische Originalkarte, 16 Kerne. „Python" ist dasselbe über
`tools/layers.py` bzw. `tools/ta_index.py`.

| | Python | Rust |
|---|---:|---:|
| osmpoi entschlüsseln (3704 Records) | 4,5 s | 0,06 s |
| osmarea entschlüsseln (368) | 16,2 s | 0,33 s |
| ta entschlüsseln (4125) | 20,1 s | 0,42 s |
| osm entschlüsseln (5878) | 56,9 s | 1,35 s |
| Suchindex zerlegen und neu bauen | 0,80 s | 0,12 s |
| osmpoi neu schreiben (`roundtrip`) | 24 s | 3,1 s |

Lesen ist also 40- bis 70-mal schneller, weil PC1 in Python nur 0,13 MB/s
schafft. Beim **Schreiben** ist der Abstand kleiner (8-fach), denn dort zählt
vor allem die LZMA-Kompression, und die macht in beiden Sprachen dieselbe
C-Bibliothek mit rund 1,5 MB/s pro Kern. Die ganze dänische osm-Datei neu zu
schreiben dauert in Rust 61 s; das ist fast reine Kompressionszeit und lässt
sich nicht mehr wesentlich drücken.

## Stufe 2: OSM lesen

`pbf.rs` liest `.osm.pbf`-Dateien (Crate `osmpbf`), dekodiert die Blöcke parallel und
faltet sie in Akkumulatoren pro Thread. Dazu der Knoten-Index: pyosmium hält einen
Index **aller** Knoten der Datei im RAM, hier werden nur die Ids gesammelt, die ein
früherer Durchgang angefordert hat — sortiert, 16 Byte pro Knoten.

`addr.rs` ist `osm_addr_extract.py`, in drei Durchgängen (eine Relation braucht ihre
Wege, die ihre Knoten):

1. Relationen: welche `multipolygon`- oder `boundary`-Relationen tragen Adress- oder
   Ortsangaben,
2. Wege: Interpolationswege, geschlossene Wege (eigene Flächen) und die Mitgliedswege,
3. Knoten: die Ergebnisse, die Knoten sind, die Koordinaten für die Wege und die
   Hausnummern der Interpolationsknoten.

### Geprüft gegen Python

```bash
./target/release/teasi addr osm_ref/denmark-latest.osm.pbf rs.txt
python scripts/addr_dump.py build/ref/addr_dk.pkl py.txt     # dasselbe aus dem Pickle
python scripts/addr_compare.py py.txt rs.txt
```

`addr_dump.py` schreibt die Einträge kanonisch, Koordinaten als IEEE-Bitmuster;
`addr_compare.py` paart sie über die Tag-Felder und misst die Abweichung in Metern.

**Dänemark** (`denmark-latest`): alle 2.628.399 Einträge **bitgleich** — 2.614.623
Adressen und 13.776 Orte.

**Großbritannien**: 5.023.341 der 5.023.358 Adressen bitgleich, 111.338 der
111.340 Orte, alle 140.467 Interpolationspunkte. 17 Adressen weichen um Median 0,93 m
ab (höchstens 5,6 m), 2 Orte um 27 m, 2 Adressen kommen dazu — alles Flächen, deren
Ringe sich selbst berühren (siehe unten).

| | Python | Rust |
|---|---:|---:|
| Dänemark (494 MB PBF) | 270 s, 2,2 GB | 4,7 s, 1,5 GB |
| Großbritannien (2,2 GB PBF) | ~20 min | 31 s, 5,3 GB |

Die drei Durchgänge brauchen für Großbritannien 6, 8 und 11 s; ein Durchgang über die
2,2 GB kostet allein rund 6 s, der Rest ist die eigentliche Arbeit.

### Flächen so zusammenbauen wie libosmium

Alles, was an einer Fläche hängt — der Mittelpunkt einer Adresse, die Position eines
POIs, die Ringe des osmarea-Layers — kommt in Python von libosmium. Dessen
Flächenbau ist in `pbf.rs` nachgebaut (`assemble_segments`, `split_rings`,
`area_loc_rings`), und zwar so:

- **Aus Kanten, nicht aus Wegen.** Jede Kante (Knotenpaar) wird so normiert, dass der
  kleinere Ort vorne steht, dann werden alle Kanten sortiert — und **gleiche Kanten
  löschen sich paarweise aus**. Das ist der Kern: zwei Wege, die ein Stück
  aneinander entlanglaufen, verschmelzen dadurch zu *einem* Ring, und ein Zacken, der
  in sich zurückläuft, verschwindet. Danach werden die Ringe aus dem Rest gelaufen.
- **Orte, nicht Knoten-Ids.** libosmium vergleicht Positionen. Zwei verschiedene
  Knoten an derselben Stelle sind für den Zusammenbau derselbe Punkt.
- **Ein Ring, der sich selbst berührt, zerfällt** an dieser Stelle in zwei Ringe
  (`split_rings`) — in der Regel in einen Außenring und ein Loch.
- **Ein Ring beginnt an seinem kleinsten Eckpunkt** und wiederholt ihn am Ende; der
  Vergleich läuft über die Dezimikrograd-Ganzzahlen, nicht über die Teasi-Einheiten
  (dort ist y gespiegelt). Der Mittelwert zählt diesen Punkt dadurch doppelt.
- **Außen und innen entscheidet die Verschachtelung, nicht die Member-Rolle.** Ein
  Ring in einer geraden Zahl anderer Ringe ist außen. Die Rollen in OSM sind oft
  falsch — in Großbritannien gibt es Multipolygone, deren Außenringe als `inner` oder
  gar als `building` eingetragen sind. Berühren sich zwei Ringe in einem Eckpunkt, ist
  der erste Punkt als Testpunkt untauglich (er liegt auf dem anderen Ring); genommen
  wird der erste Punkt, der keine Ecke des anderen Ringes ist.
- **`area=no` verbietet die Fläche**, wie die Tags sonst auch aussehen.
- Die **Richtung** (Außenringe gegen den Uhrzeigersinn in lon/lat) wird an den
  Dezimikrograd-Koordinaten entschieden, nicht an den gerundeten: eine winzige Fläche
  fällt beim Runden zusammen, und dann entscheidet das Runden die Richtung.

Was das bringt, in Zahlen: die britischen Adressen gingen von 712.719 über 5.022.875
auf **alle 5.023.357** bitgleich, die dänischen osmpoi-Records von 3693 auf **alle
3699**. Von 978.957 dänischen OSM-Flächen baut der Extraktor 978.902 bitgleich.

**Was übrig bleibt**, sind Flächen, deren Ringe sich selbst berühren — 55 von 978.957
in Dänemark, 347 von 4.868.591 in Großbritannien (0,007 %): libosmium läuft an solchen Kreuzungen nicht nach „erste freie Kante", sondern
sucht, teilt und fügt die Ringe hinterher wieder zusammen. Dann unterscheiden sich die
Zahl der Ringe oder der Startpunkt eines Rings. Diesen Teil des Assemblers (in
libosmium rund 1000 Zeilen) nachzubauen lohnt für ein Promille eines Promilles nicht.

Eine zweite Stelle war feiner: CPythons `sum()` summiert Gleitkommazahlen seit 3.12
**kompensiert** (Neumaier). Ein naives `for`-Summieren in Rust lag deshalb ein Bit
daneben — `osm::fsum` macht es jetzt genauso.

## Stufe 3: die Layer osmpoi und osmpoint

`poi.rs` ist `osm_poi_extract.py --filter` und bedient beide Layer — es behält, was
`poi_type` annimmt, und alles mit `seamark:type`. Darauf sitzen `osmpoi.rs`
(`compile_osmpoi.py`: Typ-Regeln, Attribut-String, Entdoppeln) und `osmpoint.rs`
(`compile_osmpoint.py`: Kategorien, Farben, Topzeichen, Feuer-Strings, Sektoren).
Dazwischen kein Pickle — ein Befehl vom Extrakt bis zur Kartendatei:

```bash
./target/release/teasi osmpoi osm_ref/great-britain-latest.osm.pbf \
    osm_ref/great-britain.poly build/GreatBritain_osmpoi.v20260918 20260918 --country=17
./target/release/teasi osmpoint …    # dieselben Argumente
```

Geprüft wird zweistufig. Erst die Kandidaten gegen das Python-Pickle, wie bei den
Adressen:

```bash
./target/release/teasi poi osm_ref/denmark-latest.osm.pbf rs.txt
python scripts/poi_dump.py build/ref/poi_dk.pkl py.txt
python scripts/poi_compare.py py.txt rs.txt      # paart über Art und OSM-Id
```

Dann die fertige Datei Record für Record gegen die mit Python gebaute Karte
(`teasi md5s` auf beiden, `diff`). Die Dateien selbst unterscheiden sich immer, denn
jede Kachel bekommt einen neuen Zufallsschlüssel; die entschlüsselten Records müssen
gleich sein.

| | Kandidaten bitgleich | Records bitgleich |
|---|---|---|
| osmpoi Dänemark | 88.978 Knoten, 18.596 Wege, **330 von 330** Relationen | **3699 von 3699** |
| osmpoi Großbritannien | 601.625 Knoten, 217.134 Wege, 2125 von 2139 Relationen | 15.603 von 15.619 |
| osmpoint Dänemark | | **128 von 128** |
| osmpoint Großbritannien | | **354 von 354** |

Was abweicht, sind die Flächen-Ringe von oben: Knoten und Wege stimmen zu 100 %, nur
bei einzelnen Multipolygonen liegt der Mittelpunkt um Meter daneben.

| | Python (nur die Extraktion) | Rust (PBF → Kartendatei) |
|---|---:|---:|
| Dänemark (494 MB PBF) | 3 min | 5,2 s / 3,7 s |
| Großbritannien (2,2 GB PBF) | 24 min | 35 s / 17 s |

Der Abstand ist größer als bei den Adressen, weil `poi_type` in Python für jedes
Objekt bis zu 43 `dict.get`-Aufrufe macht — über Großbritannien 370 Millionen. Hier
werden die interessanten Tags beim Lesen einmal in ein Array geschrieben und die
Regeln darauf geprüft. Nur für die Seezeichen hält `osm::TagMap` alle Tags, denn deren
Schlüssel sind offen (`seamark:light:3:colour`).

Zwei Kleinigkeiten, die beim Nachbauen auffielen: Pythons `round()` rundet die Hälfte
zur geraden Seite (`f64::round_ties_even`), und `str.capitalize()` macht den Rest des
Worts klein — aus `DGPS` wird `Dgps`.

## Stufe 3: der Layer osmarea

Der erste Compiler, der eine fremde Bibliothek braucht: `compile_osmarea.py`
verschmilzt alle Flächen einer Klasse, vereinfacht sie, schneidet sie auf die
Landesgrenze und in Blöcke und baut das Meer aus der Küstenlinie — alles mit shapely,
also mit **GEOS**. Dieselben Bytes gibt es nur mit derselben Bibliothek, darum spricht
`geos.rs` libgeos direkt an.

```bash
./target/release/teasi osmarea osm_ref/denmark-latest.osm.pbf osm_ref/denmark.poly \
    <original>/Denmark_osmarea.v20210810 build/Denmark_osmarea.v20210810 20210810
# Land ohne Originaldatei: Meer aus den weltweiten Landpolygonen
./target/release/teasi osmarea osm_ref/great-britain-latest.osm.pbf \
    osm_ref/great-britain.poly - build/GreatBritain_osmarea.v20260918 20260918 \
    --country=17 --land=osm_ref/land-polygons-split-4326/land_polygons.shp
```

`area.rs` ist `osm_area_extract.py` (Flächen und Küstenlinie, Koordinaten direkt in
osmarea-Einheiten, 360/2^25 Grad), `land.rs` ist `land_extract.py` samt einem kleinen
Shapefile-Leser — eine Polygon-Shapefile ist eine flache Folge von Records, dafür
braucht es keine Crate. `osmarea.rs` ist der Compiler selbst.

### libgeos zur Laufzeit

`geos.rs` lädt `libgeos_c` per `dlopen`, bindet nur die rund 40 benutzten Funktionen
und hält pro Thread einen GEOS-Kontext. Der Vorteil: `cargo build` braucht weder GEOS
noch dessen Header, nur `teasi osmarea` und `teasi osm` brauchen die Bibliothek — und
man kann genau die nehmen, die shapely benutzt. Gesucht wird `$TEASI_GEOS`, dann `libgeos_c.so.1`, dann
`libgeos_c.so`:

```bash
G=.venv/lib/python3*/site-packages/shapely.libs
TEASI_GEOS=$PWD/$G/libgeos_c-*.so.* LD_LIBRARY_PATH=$PWD/$G ./target/release/teasi osmarea …
```

(`LD_LIBRARY_PATH` muss dazu, weil shapelys `libgeos_c` seine `libgeos` daneben
liegend sucht, ohne RPATH.) Für **bitgleiche** Ergebnisse muss es dieselbe
GEOS-Version sein wie die von shapely — hier 3.13.1. Die Wrapper halten sich an
shapelys Semantik: `parts` ist `shapely.get_parts` (ein Polygon ist sein eigener
einziger Teil), `Geom::rect` baut den Ring genau wie `shapely.box`, offene Ringe werden
wie dort geschlossen, und der STRtree hat shapelys Knotenkapazität 10 — sonst käme eine
Abfrage in anderer Reihenfolge zurück, und die entscheidet, welches Polygon als erstes
in eine Vereinigung geht.

### Eine Änderung in Python

`compile_osmarea.py` nahm die Flächen in der Reihenfolge, in der der Extraktor sie
geschrieben hat — und das ist die Reihenfolge, in der libosmium seine Puffer leert.
Diese Reihenfolge entscheidet, welches Polygon als erstes in eine Vereinigung geht und
damit, was in der Datei landet. Sie ist nicht nachbaubar, also sortiert jetzt **auch
Python** nach der osmium-Id (`load()` in `compile_osmarea.py`). Die Ausgabe wird dadurch
reproduzierbar; die Karte ist dieselbe wie vorher, nur in definierter Reihenfolge
zusammengesetzt.

### Geprüft gegen Python

Zweistufig wie bei den POIs — erst der Extraktor gegen das Pickle, dann die Records:

```bash
./target/release/teasi area osm_ref/denmark-latest.osm.pbf rs.txt
python scripts/area_dump.py build/ref/area_latest.pkl py.txt
python scripts/area_compare.py py.txt rs.txt          # paart über die osmium-Id
python scripts/osmarea_compare.py py.v20210810 rs.v20210810   # Objekt für Objekt
```

| | Dänemark | Großbritannien |
|---|---|---|
| Flächen bitgleich | 978.902 von 978.957 | 4.868.244 von 4.868.591 |
| Küstenlinien-Wege bitgleich | 2230 von 2230 | 28.184 von 28.184 |
| Polygone je Klasse | 700.620 in 16 Klassen, 6 davon abweichend | 2.520.840 in 16 Klassen, 52 davon abweichend |
| Records bitgleich | 308 von 340 | 1029 von 1157 |
| Laufzeit (PBF → Kartendatei) | 61 s | 190 s, 7,1 GB |
| Python (ab Pickle) | 150 s + 1 min Extraktion | 456 s + 10 min Extraktion |

Die abweichenden Records hängen an den abweichenden Flächen von oben (55 in Dänemark,
347 in Großbritannien): eine einzige
abweichende Fläche verändert die Vereinigung ihrer Klasse und damit jeden Block, in dem
sie liegt — und ein Record enthält alle Klassen einer Zelle. Objektweise verglichen
(`osmarea_compare.py`) sind es einzelne Ringe mit einem Punkt mehr oder weniger.

Das Meer ist dabei der aufwendigste Teil und stimmt vollständig: die Küstenlinie wird
zu Ketten zusammengesetzt (Land links, offene Ketten gerade geschlossen), außerhalb der
Grenze kommt `#OW` aus der Originaldatei, und deren Ringe werden mit der
Even-Odd-Regel paarweise in einem Baum verrechnet.

## Stufe 3: der Layer osm

Der größte Compiler: `compile_osm.py` baut aus den Straßen-Ways vier Record-Arten —
**D** mit den Kanten und Linien, **A** mit der Namenstabelle, **B** mit dem
Routing-Graphen und **C** mit den Übersichtslinien. Python rechnet das mit numpy über
das ganze Land und braucht dafür 18 GB; hier sind es flache `Vec`s mit denselben
Indexspalten.

```bash
./target/release/teasi ways osm_ref/denmark-latest.osm.pbf ways.txt   # nur der Extraktor
./target/release/teasi osm --heights=build/ref/heights.bin \
    osm_ref/denmark-latest.osm.pbf osm_ref/denmark.poly \
    <original>/Denmark_osm.v20210916 build/Denmark_osm.v20210916 20260918
# Land ohne Originaldatei: c2/c4 bleiben leer, nichts wird kopiert
./target/release/teasi osm --heights=build/gb/dem.bin --country=17 "--name=United Kingdom" \
    osm_ref/great-britain-latest.osm.pbf osm_ref/great-britain.poly - \
    build/GreatBritain_osm.v20260918 20260918
```

`way.rs` ist `osm_extract.py`: die Ways, die `road_class` oder `line_type` annimmt, mit
ihren Knoten-Ids, dann die Koordinaten dieser Knoten. Das entspricht `--filter` auf der
Python-Seite; die Ways, die Python ohne den Schalter zusätzlich behält, können die
Ausgabe nicht erreichen (der Compiler sieht nur Straßen und Linien, und die Knotenzeilen
der übrigen gehen in keine Rechnung ein). `osm.rs` ist der Compiler.

Die Reihenfolge der Ways entscheidet, wie gleichrangige Einträge in einem Record
sortiert werden. Python nimmt sie, wie libosmium sie liefert — die Reihenfolge der
Datei, und die Geofabrik-Auszüge sind nach Id sortiert. Rust dekodiert die Blöcke
parallel und sortiert deshalb danach ausdrücklich nach der Way-Id.

### Höhen

Der Anstieg einer Kante (B-Kantenwort [2]) kommt aus Knotenhöhen, und die stammen
entweder aus dem Routing-Graphen einer Originaldatei (`osm_heights.py`, scipys `lsqr`
über 2,1 Mio. Gleichungen) oder aus dem Copernicus-Höhenmodell (`dem_heights.py`,
Kacheln vom AWS-Bucket, Gauß-Glättung). Beides sind Einmal-Rechnungen und bleiben in
Python; `scripts/heights_export.py` schreibt ihr Pickle in eine flache Binärdatei, die
`heights.rs` liest:

```bash
python scripts/heights_export.py build/ref/heights.pkl build/ref/heights.bin
```

Für die Knotenhöhen braucht es die 4 nächsten Nachbarn (inverse Distanz²). Statt eines
kd-Baums wie scipy liegt hier ein gleichmäßiges Raster über den bekannten Punkten, das
ringweise nach außen durchsucht wird, bis der nächste Ring nicht mehr näher sein kann —
dasselbe Ergebnis, nur ohne Baum.

### Geprüft gegen Python

Zweistufig wie bei den anderen Layern:

```bash
./target/release/teasi ways osm_ref/denmark-latest.osm.pbf rs.txt
python scripts/ways_dump.py build/ref/ways_latest.pkl py.txt
python scripts/ways_compare.py py.txt rs.txt
./target/release/teasi md5s <python.v20210916> ; ./target/release/teasi md5s <rust.v20210916>
```

`ways_dump.py` schreibt je Way Klasse bzw. Linientyp, Flags, Zeilenzahl, Name und eine
MD5 über Knoten-Ids und Koordinaten (als IEEE-Bitmuster) — damit stehen auch die
Tag-Tabellen auf dem Prüfstand.

| | Dänemark | Großbritannien |
|---|---|---|
| Ways bitgleich (Extraktor) | **alle 1.538.105** | – |
| Records bitgleich | 5981 von 5982 | **alle 22.711** |
| Dateigröße | 92.253.033 B (Python 92.253.016) | **531.333.357 B, identisch** |
| Laufzeit (PBF → Kartendatei) | 66 s | 4:13, 16,0 GB |
| Python (ab Pickle) | 282 s + 2 min Extraktion | 18:23, 18,0 GB + 12 min Extraktion |

Alle Zwischenzahlen stimmen auf den Eintrag: 1.454.524 Straßen und 83.581 Linien,
1.816.001 geteilte Knoten, 3.004.615 Kanten, 2.404.747 Graph-Knoten in 354 B-Zellen
(336 behalten), 134 A-, 336 B-, 5129 D- und 95 C-Records, 21 Tiles samt den drei
kopierten Färöer-Kacheln — für Großbritannien entsprechend 7.861.689/619.761,
15.029.377 Kanten, 12.119.884 Graph-Knoten, 20.507 D-Records.

Der einzige abweichende Record Dänemarks ist eine B-Zelle, in der **5 von 3.004.615
Kanten** einen um 1 cm anderen Anstieg haben. Ursache sind 16 Paare bekannter
Knotenhöhen, die auf **derselben Position** liegen und verschiedene Höhen tragen (bis
zu 6 cm auseinander, Rekonstruktionsrauschen von `lsqr`); welcher der beiden in den
Mittelwert der 4 Nachbarn eingeht, ist in beiden Implementierungen Zufall. Großbritannien
nimmt den Rasterweg und ist deshalb vollständig identisch.

## Stufe 3: der Layer ta (Adresssuche)

Der letzte OSM-Compiler und der mit den meisten Regeln: `compile_ta.py` schneidet die
benannten Straßen in Stücke, hängt jede Hausnummer an das nächste Stück gleichen
Namens, gibt jedem Stück seine Orte, fasst Stücke zu Straßen zusammen und baut daraus
die D- und A-Records plus den landesweiten **Suchindex**.

```bash
./target/release/teasi ta --country=17 "--name=United Kingdom" \
    osm_ref/great-britain-latest.osm.pbf osm_ref/great-britain.poly \
    build/GreatBritain_ta.v20260919 20260919
```

`ta.rs` braucht beides, die Adressen (`addr.rs`) und die Straßen (`way.rs`), und liest
die PBF-Datei dafür zweimal. `grid.rs` ersetzt scipys `cKDTree`: ein gleichmäßiges
Raster über die Punkte, ringweise nach außen durchsucht. Gebraucht werden drei
Abfragen — die 8 nächsten Orte innerhalb eines Radius (`query(k=8,
distance_upper_bound=…)`), der nächste Ortsknoten eines Namens und alle Paare unter
60 m (`query_pairs`).

### Reihenfolge ist hier alles

In `compile_ta.py` hängen mehr Ergebnisse an Reihenfolgen als in jedem anderen
Compiler: `Counter.most_common` bricht Gleichstände nach Einfügereihenfolge, die
Mittelpunkte der Postleitzahlbezirke sind Gleitkommasummen, und die Reihenfolge der
Index-Treffer ist die, in der sie entstanden sind. `ta.rs` hat deshalb eine
`Ordered`-Map und einen `Counter`, die sich wie Pythons `dict` und
`collections.Counter` verhalten. Dazu **drei Änderungen in Python**, alle damit
derselbe Lauf zweimal dasselbe liefert:

1. Adressen, Orte und Interpolationslinien werden **kanonisch sortiert**. Der Extraktor
   liefert sie in libosmiums Reihenfolge; die ist nicht nachbaubar (und Rust liest die
   Blöcke parallel).
2. Die **Kinder eines Suchindex-Knotens** werden nach Zeichen sortiert. Vorher wurden
   sie in der Iterationsreihenfolge eines `set` von Strings angehängt — die wechselt
   mit dem Hash-Seed von Lauf zu Lauf, Python war also nicht einmal mit sich selbst
   reproduzierbar.
3. Die **Richtung eines Hausnummernbereichs** (von/bis) kommt aus der Korrelation
   zwischen Position und Nummer. Ist die Korrelation 10⁻¹⁷, laufen die Nummern gar
   nicht entlang des Stücks, und das letzte Bit von numpys Kovarianz entschied, ob der
   Bereich auf- oder abwärts zählt. Beide Seiten nehmen jetzt Korrelationen unter
   10⁻¹² als null (`CORR_TOL`).

Die dritte Änderung war die letzte Abweichung: ohne sie waren es 2 dänische und 37
britische Records, mit ihr keine mehr.

### Geprüft gegen Python

```bash
python tools/compile_ta.py --country=4 --name=Denmark ways.pkl addr.pkl denmark.poly py.v2 20260918
./target/release/teasi ta --country=4 --name=Denmark denmark-latest.osm.pbf denmark.poly rs.v2 20260918
./target/release/teasi md5s py.v2 ; ./target/release/teasi md5s rs.v2     # dann diff
./target/release/teasi index rs.v2                                        # Suchindex
```

| | Dänemark | Großbritannien |
|---|---|---|
| Records bitgleich | **alle 3958** | 13.679 von 13.680 |
| Suchindex | **bytegleich** (1.943.962 B) | 9 von 522.760 Knoten anders |
| Dateigröße | **24.644.993 B, identisch** | **113.851.396 B, identisch** |
| Laufzeit (PBF → Kartendatei) | 40 s | 3:19, 13,7 GB |
| Python (ab Pickle) | 85 s + 2 min Extraktion | 4:12, ~16 GB + 12 + 20 min Extraktion |

Auch hier stimmen alle Zwischenzahlen: Dänemark 408.262 benannte Straßen, 1.257.070
Kanten, 1.296.848 Stücke, 2.480.980 zugeordnete Hausnummern auf 735.263 Stücken,
117.162 Straßen, 99 A- und 3859 D-Records, 12.483 Orte mit Straßen und 4935 ohne;
Großbritannien 1.969.512 / 5.225.753 / 5.334.223 / 4.909.355 / 915.297, 376 A- und
13.304 D-Records, 48.352 Orte mit Straßen, 82.998 ohne und 2603
Postleitzahlbezirke.

Der eine abweichende britische Record und die 9 Index-Knoten gehen **nicht** auf den
ta-Compiler zurück, sondern auf den bekannten Rest im Adressextraktor (Flächen, deren
Ringe sich selbst berühren): Rust findet 2 Adressen mehr, davon macht eine aus dem
Bereich 111–147 den Bereich 111–149, und eine verschiebt den Mittelpunkt des Bezirks
BN10 um 5 cm; die zwei anderen betroffenen Index-Treffer sind die beiden Ortsflächen,
die schon dort um 27 m abweichen.

## Stufe 4: der Layer terrain (Höhenmodell und Kartenbilder)

`terrain.rs` baut die Typ-5-Datei: pro Region (1,40625°) 8×8 Zellen à 256×256 px, die
Höhen als JPEG-2000-Kacheln und die Kartenbilder als JPEG, letztere zusätzlich als
Pyramide 4×4, 2×2, 1×1. Die Höhen kommen aus `heights.rs` (dem Export von
`dem_heights.py`), die Landbedeckung aus `area.rs`, das Meer aus `land.rs`.

```bash
# nur das Höhenprofil, wie Region (122,20) von Denmark_terrain
./target/release/teasi terrain build/gb/dem.bin osm_ref/great-britain.poly out 20260919

# mit Kartenbildern (braucht libgeos, s. o.)
./target/release/teasi terrain --country=17 \
    --land=land-polygons-split-4326/land_polygons.shp \
    --area=osm_ref/great-britain-latest.osm.pbf \
    build/gb/dem.bin osm_ref/great-britain.poly out 20260919    # 1:24, 7,9 GB
```

### Die Teile von Pillow

Fünf Stellen des Python-Compilers stecken in Bibliotheken, nicht in seinem Code. Drei
sind in `raster.rs` nachgebaut, Zeile für Zeile aus den C-Quellen, samt Float-Breiten
und Rundungsmakros; zwei bleiben Bibliotheken:

| Python | Rust | gleich? |
|---|---|---|
| `ImageDraw.polygon` (`Draw.c: polygon_generic`) | `Mask::polygon` | bitgleich |
| `Image.resize(…, LANCZOS)` (`Resample.c`) | `raster::resize` | bitgleich |
| `np.gradient` für die Schattierung | `terrain::shade` | bitgleich |
| `save("JPEG2000", …)`, OpenJPEG | `raster::jp2`, `openjpeg-sys` | bis auf ein Byte |
| `save("JPEG", …)`, libjpeg-turbo | `raster::jpeg`, `jpeg-encoder` | nein |

Beim Polygonfüller zählt jede Kleinigkeit: die Koordinaten werden wie in C **nach null
abgeschnitten** (nicht gerundet), eine Kante liefert ihr `x` ein **zweites Mal**, wenn
die Scanlinie ihr `ymax` trifft und nicht die letzte Zeile ist (sonst zählt ein
durchlaufender Eckpunkt doppelt und die Zeile kippt), waagerechte Kanten werden direkt
gemalt statt geschnitten, und gefüllt wird von `floor(x+0.5)` bis `ceil(x-0.5)`. Mit
„sinnvoll geraten" statt dem Original wichen 80 % der Polygone ab; mit dem Original
keines von 5000 (`polygon_fill_matches_pil`).

Das Skalieren rechnet wie Pillow in Festkomma: die Lanczos-Koeffizienten werden mit 2²²
multipliziert und von null weg zu `i32` gerundet, der Akkumulator startet bei 2²¹ und
wird am Ende um 22 Bit geschoben. Die waagerechte Richtung läuft zuerst, und nur über
die Zeilen, die die senkrechte überhaupt liest.

### JPEG 2000 und JPEG

Für die Höhenkacheln ist dieselbe Bibliothek nötig, die Pillow benutzt: ein reiner
Rust-Encoder würde andere Bytes liefern (wie die Rate über die Codeblöcke verteilt wird,
ist Implementierungssache), und was der Decoder im Gerät annimmt, ist nicht dokumentiert.
`openjpeg-sys` kompiliert OpenJPEG 2.5.3 ins Programm; die Parameter sind die von Pillows
Plugin (`irreversible`, 6 Auflösungen, Codeblöcke 64×64, LRCP, eine Schicht,
Verhältnis 50). Das Ergebnis ist **byteidentisch** bis auf ein Byte: OpenJPEG schreibt
seine eigene Version in den COM-Marker, und Pillow bringt 2.5.4 mit. Das bleibt so —
eine falsche Versionsangabe in die Datei zu schreiben wäre schlechter als ein Byte
Unterschied, und `scripts/terrain_compare.py` blendet es aus.

Die Kartenbilder gehen durch `jpeg-encoder` (reines Rust) statt libjpeg-turbo: Baseline,
4:2:0, Standard-Huffman-Tabellen, IJG-Quantisierung zu Qualität 80, JFIF mit 96 dpi und
das EXIF-APP1 der Originale. Die Bytes sind andere — die Segmente stehen in anderer
Reihenfolge, und die Chroma-Unterabtastung mittelt blockweise, während libjpeg
dreieckig filtert. Für Bilder, die ohnehin verlustbehaftet sind und über die die Datei
keine Prüfsumme führt, ist das in Kauf genommen.

### Geprüft gegen Python

`scripts/terrain_compare.py` vergleicht zwei Kartendateien Region für Region — Tabellen,
Höhenkacheln byteweise, Kartenbilder als Pixel. `teasi check` kann das nicht, der Layer
hat keine Slot-Bereiche.

| | Regionen | Höhenkacheln | Kartenbilder |
|---|---|---|---|
| Dänemark | 28 von 28 | **1034 von 1034** | 516 von 2029 pixelgleich |
| Großbritannien | 85 von 85 | **1999 von 1999** | 2680 von 5391 pixelgleich |

Die Höhenkacheln sind bis auf das Versionsbyte identisch, `a0` und `a1` eingeschlossen;
keine einzige weicht im Codestream ab. Bei den Kartenbildern ist die mittlere Abweichung
0,025 von 255 und der Median der größten Abweichung je Bild **1**; im schlimmsten Bild
95, als Ringen um einzelne Pixel in dicht gezeichneten Gegenden. Dass die Hälfte der
Bilder pixelgleich durchläuft — und zwar genauso detaillierte wie die abweichenden —
zeigt, dass die Quellbilder übereinstimmen und nur der Encoder anders ist. Die Dateien
sind 36.886.754 statt 36.887.738 B groß, drei Hunderttausendstel kleiner.

| | Python (nur der Compiler) | Rust (PBF und Shapefile → Kartendatei) |
|---|---:|---:|
| Dänemark (28 Regionen) | 10 s | 17 s, 1,7 GB — davon 7 s Regionen |
| Großbritannien (85 Regionen) | ~2 min, 10 GB | 1:24, 7,9 GB |

Die beiden Spalten messen nicht dasselbe: Python bekommt drei fertige Pickles
vorgesetzt (`dem_heights.py`, `land_extract.py`, `osm_area_extract.py` — für
Großbritannien über 20 min), Rust liest das 2,2-GB-PBF und das 1,3-GB-Shapefile in
seinen Zeiten selbst. Die Regionen allein sind in Rust etwa so schnell wie in Python,
weil dort die Arbeit schon in C steckt und auf alle Kerne verteilt ist.

Eine Kleinigkeit weicht vor dem Zeichnen ab: Rust bindet 525.037 statt 525.034
Landbedeckungs-Flächen in die dänischen Regionen ein. Das sind dieselben 55 von 978.957
Flächen, die der Extraktor schon beim osmarea-Layer anders zerlegt (Ringe, die sich
selbst berühren); die Zahl der Bilder und Höhenkacheln ist in jeder Region dieselbe.

## Zwei Fallen

**Rohes LZMA1.** liblzma schreibt nur das `.lzma`-Format, und das ist genau ein
13-Byte-Kopf vor dem rohen Strom — also Kopf abschneiden beim Schreiben, Kopf
davorsetzen beim Lesen. Im Kopf steht die Länge aber als *unbekannt*: unsere
eigenen Records enden mit dem End-Marker, den liblzma anhängt, die Originale
nicht, und ein Strom mit angekündigter Länge darf diesen Marker nicht haben.
Mit „unbekannt" akzeptiert liblzma beides, und der Aufrufer hört nach `pltx`
Bytes auf — genau wie `LzmaDecode` in der Firmware.

**Verworfene Felder.** Jeder Container gibt auch die Felder zurück, die niemand
interpretiert, und schreibt sie unverändert zurück. Nur deshalb ist der
Round-Trip bytegleich; Längen und Offsets neu zu berechnen wäre bequemer, würde
aber genau die Stellen verlieren, an denen die Originale von unseren Annahmen
abweichen.
