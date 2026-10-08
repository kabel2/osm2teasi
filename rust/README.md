# teasi (Rust)

Portierung der Werkzeugkette nach Rust. Fertig sind **die Hülle** (Stufe 1:
Entschlüsselung, Kompression, alle Record-Container, der Schreiber, der Suchindex),
**das Lesen von OSM** (Stufe 2: PBF-Leser, Knoten-Index, Adressextraktion) und
**der erste Layer-Compiler** (Stufe 3: `osmpoi`, aus dem PBF direkt in die
Kartendatei). Die Python-Werkzeuge in [../tools/](../tools/) bleiben die Referenz;
was hier steht, muss dasselbe liefern.

Noch nicht portiert: die fünf übrigen Layer-Compiler (`compile_osm.py`,
`compile_osmarea.py`, `compile_osmpoint.py`, `compile_ta.py`,
`compile_terrain.py`).

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
| Landesgrenze (`.poly`) | `poly.py` | `poly.rs` |
| POIs lesen | `osm_poi_extract.py` | `poi.rs` |
| Layer `osmpoi` bauen | `compile_osmpoi.py` | `osmpoi.rs` |
| Straßen, Flächen, Höhen lesen | `osm_extract.py` u. a. | – |
| Die fünf übrigen Layer bauen | `compile_*.py` | – |

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
| `teasi poi <datei.osm.pbf> <aus>` | POI-Kandidaten als kanonischer Dump |
| `teasi osmpoi <pbf> <poly> <karte> [datum]` | den osmpoi-Layer bauen (`--country=N`) |

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

**Großbritannien**: 5.022.875 der 5.023.357 Adressen bitgleich, 111.333 der 111.338
Orte, alle 140.467 Interpolationspunkte. Von den Adressen weichen 480 um Median 0,78 m
ab (höchstens 40 m), von den Orten 5; 3 Adressen und 2 Orte fehlen, 2 Adressen kommen
dazu. Alles davon sind Flächen mit kaputten Ringen (siehe unten).

| | Python | Rust |
|---|---:|---:|
| Dänemark (494 MB PBF) | 270 s, 2,2 GB | 4,7 s, 1,5 GB |
| Großbritannien (2,2 GB PBF) | ~20 min | 30 s, 4,7 GB |

Die drei Durchgänge brauchen für Großbritannien 6, 8 und 11 s; ein Durchgang über die
2,2 GB kostet allein rund 6 s, der Rest ist die eigentliche Arbeit.

### Flächen so zusammenbauen wie libosmium

Eine Adresse oder ein POI an einem Polygon bekommt als Position den Mittelwert der
Eckpunkte. Die Python-Werkzeuge holen diese Punkte von libosmium, und das hat zwei
Konventionen, die man beide nachbauen muss, sonst liegt der Mittelpunkt Meter daneben
(`osm.rs`, Abschnitt „areas the way libosmium assembles them"):

- **Ein Ring beginnt an seinem geometrisch kleinsten Eckpunkt** und wiederholt ihn am
  Ende. libosmium normiert jede Kante so, dass der kleinere Endpunkt vorne steht,
  sortiert die Kanten und fängt den Ring bei der ersten an. Der Mittelwert zählt
  dadurch einen Punkt doppelt — und bei der Reihenfolge des Wegs wäre es ein anderer.
- **Außen und innen entscheidet die Verschachtelung, nicht die Member-Rolle.** Ein Ring
  in einer geraden Zahl anderer Ringe ist außen. Die Rollen in OSM sind oft falsch —
  in Großbritannien gibt es Multipolygone, deren Außenringe als `inner` oder gar als
  `building` eingetragen sind.

Damit sind die Flächenadressen bitgleich: vor dieser Nachbildung wichen 4,3 von
5,0 Mio. britischen Adressen um Median 1,6 m ab, jetzt sind es 480.

**Was übrig bleibt**, sind Flächen, deren Ringe nicht sauber schließen. Der
Zusammenbau hier ist einfach — Wege an gemeinsamen Enden aneinanderhängen —, während
libosmium eine Kantenmenge sortiert, doppelte Kanten entfernt und an Berührpunkten
aufteilt. Zwei geschlossene Wege, die sich in zwei Knoten berühren, werden dort zu
*einem* Ring verschmolzen, hier bleiben es zwei; in Großbritannien betrifft das 156
von 2142 POI-Relationen (Median 1,3 m, im Extremfall 242 m). In vier Fällen baut
libosmium eine Fläche ganz ohne Außenring und liefert deshalb keinen POI, hier
entsteht einer; in einem Fall ist es umgekehrt.

Eine zweite Stelle war feiner: CPythons `sum()` summiert Gleitkommazahlen seit 3.12
**kompensiert** (Neumaier). Ein naives `for`-Summieren in Rust lag deshalb ein Bit
daneben — `osm::fsum` macht es jetzt genauso.

## Stufe 3: der osmpoi-Layer

`poi.rs` ist `osm_poi_extract.py --filter`, `osmpoi.rs` ist `compile_osmpoi.py`:
Typ-Regeln, Attribut-String, Entdoppeln, Aufteilen in Zellen und Kacheln. Dazwischen
kein Pickle — ein Befehl vom Extrakt bis zur Kartendatei:

```bash
./target/release/teasi osmpoi osm_ref/great-britain-latest.osm.pbf \
    osm_ref/great-britain.poly build/GreatBritain_osmpoi.v20260918 20260918 --country=17
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
| Dänemark | 88.978 Knoten, 18.596 Wege, 323 von 330 Relationen | 3692 von 3699 |
| Großbritannien | 601.625 Knoten, 217.134 Wege, 1982 von 2142 Relationen | 15.498 von 15.619 |

Alle Abweichungen sind die Flächen-Ringe von oben: Knoten und Wege stimmen zu 100 %,
nur bei Multipolygonen liegt der Mittelpunkt um Meter daneben.

| | Python (nur die Extraktion) | Rust (PBF → Kartendatei) |
|---|---:|---:|
| Dänemark (494 MB PBF) | 3 min | 5,0 s |
| Großbritannien (2,2 GB PBF) | 24 min | 29 s, 0,7 GB |

Der Abstand ist größer als bei den Adressen, weil `poi_type` in Python für jedes
Objekt bis zu 43 `dict.get`-Aufrufe macht — über Großbritannien 370 Millionen. Hier
werden die interessanten Tags beim Lesen einmal in ein Array geschrieben und die
Regeln darauf geprüft.

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
