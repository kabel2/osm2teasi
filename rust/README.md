# teasi (Rust)

Portierung der Werkzeugkette nach Rust, Stufe 1: **die Hülle**. Entschlüsselung,
Kompression, alle Record-Container, der Schreiber und der Suchindex der ta-Schicht.
Die Python-Werkzeuge in [../tools/](../tools/) bleiben die Referenz; was hier steht,
muss bytegleich dasselbe liefern.

Noch nicht portiert: das Lesen von OSM-PBF-Dateien und die sechs Layer-Compiler
(`compile_*.py`). Die Semantik der Records — Straßenklassen, Flags, POI-Typen,
Hausnummern — steckt dort und nicht in dieser Stufe; hier werden alle Felder,
auch die unverstandenen, unverändert durchgereicht.

## Stand

| Baustein | Python | Rust |
|---|---|---|
| PC1, MD5-Prüfsumme, globaler Schlüssel | `pc1.py`, `chart.py` | `pc1.rs`, `chart.rs` |
| Rohes LZMA1 wie die Originale | `chart.py` | `lzma.rs` |
| Container A, B, C, D, osmpoint | `layers.py` | `layers.rs` |
| Dateien schreiben, Gerätebindung | `writer.py` | `writer.rs` |
| Suchindex der Adresssuche | `ta_index.py` | `ta_index.rs` |
| OSM lesen, Layer bauen | `*_extract.py`, `compile_*.py` | – |

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
