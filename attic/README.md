# Attic

One-off scripts from the analysis phase. They served their purpose and are only kept here
for the record, not maintained: hard-coded paths, and in part intermediate files that no
longer exist. The normal route (read a map, build a layer) does not need them — that one
is in [../README.md](../README.md).

| File | What it was |
|---|---|
| `chart_audit.py` | Checks the outer shell of a chart file (header, tile directory, record chain), still without interpreting the encrypted payload. This was the way in. |
| `ks_attack.py` | A known-plaintext attack on the payloads, under the mistaken assumption of a proprietary XOR keystream. Disproved — it is PC1 plus LZMA. |
| `seed_hunt.py` | Hunt for the global key by actually emulating the firmware routines `FUN_00289660`/`FUN_002896c8` on ARM (Unicorn). Later served as a cross-check for `tools/pc1.py`. |
| `terrain_render/` | Reconstruction of the tile arrangement in the terrain layer (block 1 plus the mipmap pyramid) by rendering and comparing. The result is in `docs/TERRAIN_FORMAT.md`. |
| `ghidra/` | 23 one-off searches in Ghidra (`*Hunt.java`, `Decomp*.java`, …), out of which the four maintained scripts in `../ghidra_scripts/` grew. |
