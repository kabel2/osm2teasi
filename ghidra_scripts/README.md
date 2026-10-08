# Ghidra helper scripts

Four headless scripts for analysing `bikenav.exe` (ARM, WinCE). They were the main tool
for finding the encryption, the record reader and the search flow; all the results are in
[../docs/CHART_FILES.md](../docs/CHART_FILES.md), sections 5.8 and 6.

Link them into Ghidra's script directory once, for example:

```bash
ln -s "$PWD"/*.java ~/ghidra_scripts/
```

Then run them against an already analysed project (`-readOnly`, so that the project stays
untouched):

```bash
ghidra_12.1.3_PUBLIC/support/analyzeHeadless <project folder> <project> \
  -process bikenav.exe -noanalysis -readOnly \
  -postScript <Script>.java <output.txt> <arguments…>
```

| Script | Arguments | Purpose | Runtime |
|---|---|---|---|
| `Dx.java` | `<out> ADDR …` | decompile the function containing `ADDR` | ~1 min |
| | `xref:ADDR` | list every reference to `ADDR` (with its function) | |
| | `callers:ADDR` | list the references **and** decompile every referencing function | |
| `Grep.java` | `<out> REGEX [REGEX …]` | decompile all ~9500 functions and print those whose C code contains **all** the regexes | ~30–45 min |
| `InsnGrep.java` | `<out> REGEX` | regex over every assembler instruction, e.g. `'^str .*#0x1d8\]'` | ~1 min |
| `Range.java` | `<out> LO HI` | decompile every function whose entry point lies in `[LO, HI)` | ~1 min per 300 functions |

Addresses in hex without `0x`, e.g. `Dx.java out.txt 002050a0 callers:002897b0`.

Written for **Ghidra 12**; the API changed from 11 (`decompileFunction(Function, …)`
instead of `decompile(Address, …)`, `getErrorMessage()` instead of `getMsgLog()`,
`dispose()` instead of `close()`, and `getReferencesTo()` returns a `ReferenceIterator`).
Older scripts already fail to compile against it.

`InsnGrep` also finds code that Ghidra has not assigned to any function (`in ?` in its
output). That is where the key initialisation at `0x1fd148` sits, for instance.

The unmaintained one-off searches from the analysis phase are in
[../attic/ghidra/](../attic/ghidra/).
