# Dachboden

Einmal-Skripte aus der Analysephase. Sie haben ihren Zweck erfüllt und sind hier nur
dokumentiert, nicht gepflegt: hartkodierte Pfade, teils Zwischendateien, die es nicht mehr
gibt. Für den normalen Weg (Karte lesen, Layer bauen) braucht man sie nicht — der steht
in [../README.md](../README.md).

| Datei | Was es war |
|---|---|
| `chart_audit.py` | Prüft die äußere Hülle einer Chart-Datei (Header, Tile-Verzeichnis, Record-Kette), noch ohne Interpretation der verschlüsselten Nutzdaten. Das war der Einstieg. |
| `ks_attack.py` | Known-Plaintext-Angriff auf die Payloads unter der falschen Annahme eines eigenen XOR-Keystreams. Widerlegt — es ist PC1 plus LZMA. |
| `seed_hunt.py` | Suche nach dem globalen Schlüssel durch echte ARM-Emulation (Unicorn) der Firmware-Routinen `FUN_00289660`/`FUN_002896c8`. Diente später als Gegenprobe für `tools/pc1.py`. |
| `terrain_render/` | Rekonstruktion der Kachelanordnung im terrain-Layer (Block 1 plus Mipmap-Pyramide) durch Rendern und Vergleichen. Ergebnis steht in `docs/TERRAIN_FORMAT.md`. |
| `ghidra/` | 23 Einmal-Suchen in Ghidra (`*Hunt.java`, `Decomp*.java`, …), aus denen die vier gepflegten Skripte in `../ghidra_scripts/` entstanden sind. |
