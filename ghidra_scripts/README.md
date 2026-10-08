# Ghidra-Hilfsskripte

Vier Headless-Skripte für die Analyse von `bikenav.exe` (ARM, WinCE). Sie waren das
Hauptwerkzeug, um Verschlüsselung, Record-Leser und Suchablauf zu finden; alle Ergebnisse
stehen in [../docs/KARTEN_ENTSCHLUESSELUNG.md](../docs/KARTEN_ENTSCHLUESSELUNG.md),
Abschnitte 5.8 und 6.

Einmalig nach Ghidras Skriptverzeichnis verlinken, z. B.:

```bash
ln -s "$PWD"/*.java ~/ghidra_scripts/
```

Dann gegen ein bereits analysiertes Projekt laufen lassen (`-readOnly`, damit das Projekt
unverändert bleibt):

```bash
ghidra_12.1.3_PUBLIC/support/analyzeHeadless <projektordner> <projekt> \
  -process bikenav.exe -noanalysis -readOnly \
  -postScript <Script>.java <ausgabe.txt> <argumente…>
```

| Script | Argumente | Zweck | Dauer |
|---|---|---|---|
| `Dx.java` | `<out> ADDR …` | Funktion dekompilieren, die `ADDR` enthält | ~1 min |
| | `xref:ADDR` | alle Referenzen auf `ADDR` auflisten (mit Funktion) | |
| | `callers:ADDR` | Referenzen auflisten **und** alle referenzierenden Funktionen dekompilieren | |
| `Grep.java` | `<out> REGEX [REGEX …]` | alle ~9500 Funktionen dekompilieren und die ausgeben, deren C-Code **alle** Regexe enthält | ~30–45 min |
| `InsnGrep.java` | `<out> REGEX` | Regex über alle Assembler-Instruktionen, z. B. `'^str .*#0x1d8\]'` | ~1 min |
| `Range.java` | `<out> LO HI` | alle Funktionen mit Einsprungpunkt in `[LO, HI)` dekompilieren | ~1 min / 300 Funktionen |

Adressen hexadezimal ohne `0x`, z. B. `Dx.java out.txt 002050a0 callers:002897b0`.

Geschrieben für **Ghidra 12**; die API hat sich gegenüber 11 geändert
(`decompileFunction(Function, …)` statt `decompile(Address, …)`, `getErrorMessage()` statt
`getMsgLog()`, `dispose()` statt `close()`, `getReferencesTo()` liefert einen
`ReferenceIterator`). Ältere Skripte scheitern daran schon beim Kompilieren.

`InsnGrep` findet auch Code, den Ghidra keiner Funktion zugeordnet hat (im Output `in ?`).
Dort sitzt z. B. die Schlüssel-Initialisierung bei `0x1fd148`.

Die ungepflegten Einmal-Suchen aus der Analysephase liegen in [../attic/ghidra/](../attic/ghidra/).
