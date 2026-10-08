# Teasi-Karten aus OpenStreetMap

Werkzeuge, um die Kartendateien eines **Teasi PRO** (Tahuna/Falk, `bikenav.exe` 4.4.1.0,
WinCE/ARM) zu lesen, zu schreiben und aus aktuellen OSM-Daten neu zu erzeugen.

Der Hersteller liefert für das Gerät keine neuen Karten mehr. Die Dateien sind aber kein
eigenes Format mit eigenem Codec, sondern eine MD5-Prüfsumme, **PC1**-Verschlüsselung und
**LZMA** um gut dokumentierbare Datenstrukturen. Alle sechs Layer sind entschlüsselt, ihre
Container werden bytegleich nachgebaut, und für jeden gibt es einen Compiler, der ihn aus
einem Geofabrik-Extrakt baut.

**Stand:** Großbritannien ist komplett gebaut und läuft auf dem Gerät (Karte, POIs,
Höhenprofil, Adresssuche). Offen sind der Routing-Test in dichten Städten und ein paar
Einzelfelder, siehe [docs/KARTEN_ENTSCHLUESSELUNG.md](docs/KARTEN_ENTSCHLUESSELUNG.md),
Abschnitt 8.

## Die Layer

Eine Karte ist ein Satz Dateien `<Land>_<Layer>.vJJJJMMTT` (Magic `0x1B62`), auf dem Gerät
unter `<Seriennummer>/7/943/20317/`.

| Layer | Inhalt | Compiler | GB |
|---|---|---|---:|
| `osm` | Straßen- und Wegenetz, Namen, Routing-Graph | `compile_osm.py` | 531 MB |
| `osmarea` | Flächen: Landnutzung, Wald, Wasser, Siedlungen | `compile_osmarea.py` | 83 MB |
| `osmpoi` | Points of Interest | `compile_osmpoi.py` | 17 MB |
| `osmpoint` | Seezeichen aus OpenSeaMap | `compile_osmpoint.py` | 0,5 MB |
| `ta` | Adresssuche: Orte, Straßen, Hausnummern, Suchindex | `compile_ta.py` | 114 MB |
| `terrain` | Höhenmodell und Kartenbilder (unverschlüsselt) | `compile_terrain.py` | 37 MB |

Jedes Format hat ein eigenes Dokument in [docs/](docs/), jeweils mit einem Abschnitt
„Aus OSM erzeugen“ (Befehle, Laufzeit, RAM) und einem Abschnitt „Offen“.

## Aufbau des Repos

```
tools/            Die Werkzeugkette: Hülle lesen/schreiben, Extraktoren, Compiler
docs/             Formatdokumentation (deutsch), Einstieg: KARTEN_ENTSCHLUESSELUNG.md
rust/             Rust-Portierung, Stufe 1: Hülle, Container, Schreiber, Suchindex
ghidra_scripts/   Headless-Skripte für die Firmware-Analyse in Ghidra
attic/            Einmal-Skripte aus der Analysephase, nicht gepflegt
```

In `tools/` liegen die Module flach, sie importieren sich gegenseitig ohne Paket. Deshalb
Skripte entweder aus `tools/` heraus starten oder vom Repo-Wurzelverzeichnis mit
`python tools/<script>.py` (beides funktioniert).

| Werkzeug | Zweck |
|---|---|
| `chart.py` | Hülle und Verschlüsselung: Header prüfen, Records entschlüsseln |
| `pc1.py` | PC1 (Pukall Cipher 1, 256 Bit) |
| `layers.py` | Parser und Builder aller Record-Arten |
| `writer.py` | gültige Chart-Dateien schreiben (inkl. Gerätebindung) |
| `roundtrip.py` | Prüfung: Datei zerlegen, neu bauen, Records vergleichen |
| `packages.py` | Größe und MD5 in `packages.xml` nachziehen |
| `poly.py` | Geofabrik-Landesgrenze (`*.poly`) |
| `osm_extract.py`, `osm_poi_extract.py`, `osm_area_extract.py`, `osm_addr_extract.py` | OSM-PBF in kompakte Pickles |
| `land_extract.py`, `dem_heights.py`, `osm_heights.py` | Landpolygone, Copernicus-DEM, Höhen |
| `compile_*.py` | die sechs Layer-Compiler |
| `ta_lookup.py` | Adresssuche der Firmware offline nachspielen (Prüfung) |

## Loslegen

```bash
python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
```

Eine vorhandene Karte entschlüsseln und die Records einzeln ablegen:

```bash
.venv/bin/python tools/chart.py charts/Denmark_osm.v20210916 out/osm
```

Prüfen, dass die Werkzeuge eine Datei verlustfrei nachbauen:

```bash
.venv/bin/python tools/roundtrip.py charts/Denmark_osmpoi.v20210915
```

Einen Layer aus OSM bauen (hier POIs; die anderen Layer stehen in den Layer-Docs):

```bash
.venv/bin/python tools/osm_poi_extract.py --filter osm_ref/great-britain-latest.osm.pbf build/poi.pkl
.venv/bin/python tools/compile_osmpoi.py --country=17 build/poi.pkl osm_ref/great-britain.poly \
    build/GreatBritain_osmpoi.v20260919 20260919
```

### Gerätebindung

Die Header-Prüfsumme bindet eine Karte an die Seriennummer des Geräts, und daraus leitet sich
auch der PC1-Schlüssel ab. Alle Werkzeuge nehmen dafür die Nummer aus `chart.py` (`DEVICE`),
überschreibbar per Umgebungsvariable:

```bash
export TEASI_DEVICE=2013021200000368   # eigene 16-stellige Seriennummer
```

Damit das Gerät eine neu gebaute Datei ohne Warnung annimmt, müssen Größe und MD5 in
`BikeNav/packages.xml` angepasst werden (`packages.py`, Doku Abschnitt 5.4).

## Rust-Portierung

In [rust/](rust/) wird die Werkzeugkette nach Rust portiert. Fertig sind die Hülle
(PC1, Prüfsumme, rohes LZMA1, alle Record-Container, Schreiber, Suchindex) und das
Lesen von OSM (PBF-Leser, Knoten-Index, Adressextraktion). Die Python-Werkzeuge
bleiben die Referenz; geprüft wird gegen sie:

```bash
cd rust && cargo build --release
./target/release/teasi check <karten>/Denmark_*.v2*      # Records bytegleich
./target/release/teasi addr osm_ref/denmark-latest.osm.pbf
```

Alle 14.185 Records der dänischen Karte in 1,3 s zerlegt und bytegleich neu gebaut;
Entschlüsseln ist 40- bis 70-mal schneller als in Python, Schreiben etwa 8-mal, die
Adressextraktion 56-mal (Dänemark 270 s → 4,8 s). Die Layer-Compiler sind noch nicht
portiert, Details in [rust/README.md](rust/README.md).

## Was hier nicht drin ist

Kein Gerätedump, keine Firmware, keine OSM-Extrakte, keine gebauten Karten — das sind
mehrere Gigabyte, und die Originalkarten sind fremde, lizenzierte Daten. Gebraucht werden:

- ein Geofabrik-Extrakt und die passende `.poly`-Datei (`download.geofabrik.de`),
- für `terrain` das Copernicus-DEM (lädt `dem_heights.py` selbst) und die Landpolygone
  von `osmdata.openstreetmap.de`,
- zum Eichen eine Originalkarte des Geräts.

## Rechtliches

Reverse Engineering zur Interoperabilität mit dem eigenen, gekauften Gerät. Die neu gebauten
Karten enthalten ausschließlich OpenStreetMap-Daten (ODbL) sowie Höhen aus dem Copernicus-DEM;
Originalkarten oder Firmware-Teile gehören nicht ins Repo und werden hier nicht verteilt.

Die Werkzeuge stehen unter der [MIT-Lizenz](LICENSE).
