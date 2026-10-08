# teasi (Rust)

Portierung der Werkzeugkette nach Rust. Fertig sind **die Hülle** (Stufe 1:
Entschlüsselung, Kompression, alle Record-Container, der Schreiber, der Suchindex)
und **das Lesen von OSM** (Stufe 2: PBF-Leser, Knoten-Index, Adressextraktion).
Die Python-Werkzeuge in [../tools/](../tools/) bleiben die Referenz; was hier steht,
muss dasselbe liefern.

Noch nicht portiert: die sechs Layer-Compiler (`compile_*.py`). Die Semantik der
Records — Straßenklassen, Flags, POI-Typen, Hausnummern — steckt dort und nicht in
diesen Stufen; hier werden alle Felder, auch die unverstandenen, unverändert
durchgereicht.

## Stand

| Baustein | Python | Rust |
|---|---|---|
| PC1, MD5-Prüfsumme, globaler Schlüssel | `pc1.py`, `chart.py` | `pc1.rs`, `chart.rs` |
| Rohes LZMA1 wie die Originale | `chart.py` | `lzma.rs` |
| Container A, B, C, D, osmpoint | `layers.py` | `layers.rs` |
| Dateien schreiben, Gerätebindung | `writer.py` | `writer.rs` |
| Suchindex der Adresssuche | `ta_index.py` | `ta_index.rs` |
| OSM-PBF lesen, Knoten-Index | pyosmium | `osm.rs` |
| Adressen, Orte, Interpolationswege | `osm_addr_extract.py` | `addr.rs` |
| Straßen, POIs, Flächen, Höhen lesen | `osm_extract.py` u. a. | – |
| Layer bauen | `compile_*.py` | – |

## Bauen und prüfen

```bash
cargo build --release
cargo test                 # Vergleichswerte aus den Python-Werkzeugen
```

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

`osm.rs` liest `.osm.pbf`-Dateien (Crate `osmpbf`), dekodiert die Blöcke parallel und
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

**Dänemark** (`denmark-latest`): dieselben 2.628.399 Einträge, keiner fehlt, keiner
kommt dazu; die 2.614.459 Adressen und 12.511 Orte an Knoten sind **bitgleich**.

**Großbritannien**: 5.023.358 Adressen und 29.868 Interpolationswege wie in Python,
deren 140.467 Punkte bitgleich. Bei den Orten fehlen 3 von 111.340, bei den Adressen
fehlen 2 und 2 kommen dazu — Flächen mit kaputten Ringen, bei denen libosmiums
Zusammenbau und der einfache Test hier (jedes Wegende muss an einer geraden Zahl von
Wegen hängen) unterschiedlich entscheiden.

| | Python | Rust |
|---|---:|---:|
| Dänemark (494 MB PBF) | 270 s, 2,2 GB | 4,7 s, 1,5 GB |
| Großbritannien (2,2 GB PBF) | ~20 min | 30 s, 4,7 GB |

Die drei Durchgänge brauchen für Großbritannien 6, 8 und 11 s; ein Durchgang über die
2,2 GB kostet allein rund 6 s, der Rest ist die eigentliche Arbeit.

### Wo Rust abweicht: Mittelpunkte von Flächen

Eine Adresse an einem Gebäudepolygon bekommt als Position den Mittelwert der
Eckpunkte. libosmium dreht einen zusammengesetzten Ring aber so, dass er am
geometrisch kleinsten Eckpunkt beginnt, und der Ring wiederholt seinen ersten Punkt
am Ende — der Mittelwert zählt also einen Punkt doppelt, und zwar einen anderen als
in der Reihenfolge des Wegs. `ring_centre` lässt den wiederholten Punkt deshalb weg;
das Ergebnis hängt dann nicht mehr davon ab, wo der Ring anfängt.

Betroffen sind nur Einträge aus Flächen — in Dänemark 164 von 2,6 Mio. Adressen, in
Großbritannien dagegen 4,3 von 5,0 Mio., weil dort fast jede Adresse an einem
Gebäudepolygon hängt. Die Abweichung ist klein: Median 1,6 m, 90 % unter 2,2 m, bei
Orten Median 5,1 m. Im Extremfall — eine riesige Fläche mit wenigen Eckpunkten — sind
es einige hundert Meter. Für den ta-Layer ist das ohne Belang: Adressen werden mit
100 m Radius auf Straßenstücke gezogen, Orte dienen als Anker mit Radien von 800 bis
12.000 m.

Die andere Stelle war feiner: CPythons `sum()` summiert Gleitkommazahlen seit 3.12
**kompensiert** (Neumaier). Ein naives `for`-Summieren in Rust lag deshalb ein Bit
daneben — `osm::fsum` macht es jetzt genauso.

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
