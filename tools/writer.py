"""Write Teasi chart files (*.vYYYYMMDD) -- the inverse of chart.py.

Header fields as read by the chart loader FUN_003f282c (CHART_FILES.md 1):
  0x44 date (8 ASCII; a file is skipped if one with the same layer/country and an
  equal or newer date is already loaded), 0x4C type, 0x50 layer bit mask, 0x54 country,
  0x58..0x6C buffer sizes (maxed over all loaded files): 0x58 largest compressed
  record, 0x5C/0x60/0x64/0x68 largest plaintext of slot area D/C/B/A, 0x6C unused (0);
  0x70 offset of an extra table (only used when layer & 0x10, i.e. ta), else -1.
"""
import lzma
import os
import struct
from concurrent.futures import ProcessPoolExecutor

from chart import DEVICE, LZMA_FILTER, header_md5, global_key
from layers import AREAS
from pc1 import encrypt_blob, encrypt_payload

HEAD = 0x159C                      # tile head size, records start here
EMPTY_CELLS = struct.pack("<II", 0xFFFFFFFF, 0) * 85 + b"\xff" * (85 * 4)
PLAIN_AREAS = {"B"}                # osm routing graph: LZMA only, no PC1


def compress(raw):
    """Raw LZMA1 (lc=3 lp=0 pb=2, dict 16 MiB) like the originals.  liblzma appends
    an end marker; the firmware's LzmaDecode stops after pltx bytes anyway."""
    return lzma.compress(raw, format=lzma.FORMAT_RAW, filters=LZMA_FILTER)


def pack_record(raw, rk, plain=False):
    comp = compress(raw)
    payload = comp if plain else encrypt_payload(comp, rk)
    return struct.pack("<II", len(payload), len(raw)) + payload


def build_tile(areas, rk, key, tail=b""):
    """areas = {"A".."D": {slot: plaintext}} -> (tile bytes, max len, {area: max pltx}).

    Records are written area by area (A, B, C, D) in slot order, as in the originals."""
    heads = {a: [0xFFFFFFFF] * (g * g) for a, (_, g) in AREAS.items()}
    body = bytearray()
    maxlen, maxraw = 0, {}
    for a in "ABCD":
        for slot, raw in sorted(areas.get(a, {}).items()):
            rec = pack_record(raw, rk, a in PLAIN_AREAS)
            heads[a][slot] = HEAD + len(body)
            body += rec
            maxlen = max(maxlen, len(rec) - 8)
            maxraw[a] = max(maxraw.get(a, 0), len(raw))
    head = b"".join(struct.pack("<%dI" % len(heads[a]), *heads[a]) for a in "ABCD")
    head += EMPTY_CELLS + encrypt_blob(rk, key)
    assert len(head) == HEAD
    return head + bytes(body) + tail, maxlen, maxraw


def write_chart(meta, tiles, device=DEVICE, bind=True, rk=None):
    """meta: {"date": b"20260918", "type": 1, "layer": 1, "country": 4}
    tiles: [(x, y, areas or None, tail)] in directory order; None = empty placeholder
    tile (size 0, like osmarea's (0,0)).  tail = extra bytes after the records; if
    meta["tail_tile"] == (x, y), header 0x70 points to that tile's tail.

    bind=True signs the file for `device`, otherwise generically (the firmware then
    binds it on first open by rewriting salt and MAC)."""
    key = global_key(device)
    n = len(tiles)
    off = 0x78 + 8 * n
    blobs, dirs, maxlen, maxraw, extra = [], [], 0, {}, 0xFFFFFFFF
    jobs = [(areas, rk or os.urandom(32), key, tail) for x, y, areas, tail in tiles if areas is not None]
    with ProcessPoolExecutor() as ex:          # LZMA + pure-Python PC1 are slow
        built = iter(list(ex.map(build_tile, *zip(*jobs))))
    for x, y, areas, tail in tiles:
        if areas is None:
            dirs.append((x, y, off))
            continue
        t, ml, mr = next(built)
        if meta.get("tail_tile") == (x, y):
            extra = off + len(t) - len(tail)
        dirs.append((x, y, off))
        blobs.append(t)
        off += len(t)
        maxlen = max(maxlen, ml)
        for a, v in mr.items():
            maxraw[a] = max(maxraw.get(a, 0), v)
    hdr = bytearray(struct.pack("<I", 0x1B62) + os.urandom(48) + bytes(16))
    hdr += meta["date"]
    hdr += struct.pack("<3I", meta["type"], meta["layer"], meta["country"])
    hdr += struct.pack("<6I", maxlen, *(maxraw.get(a, 0) for a in "DCBA"), 0)
    hdr += struct.pack("<II", extra, n)
    hdr += b"".join(struct.pack("<HHI", *d) for d in dirs)
    d = bytearray(hdr + b"".join(blobs))
    d[0x34:0x44] = header_md5(bytes(d), device if bind else b"")
    return bytes(d)


def cli(argv):
    """argv -> (positional args, {option: value}) for --key=value / --flag options"""
    opts = dict((a[2:].split("=", 1) + [""])[:2] for a in argv if a.startswith("--"))
    return [a for a in argv if not a.startswith("--")], opts
