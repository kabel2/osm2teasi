"""Reader for Teasi/Tahuna chart files (*.vYYYYMMDD, magic 0x1B62)."""
import hashlib
import lzma
import os
import struct

from pc1 import decrypt_blob, decrypt_payload

# Device ID the charts are bound to (serial of the Teasi, 16 digits).  All tools
# use it as the default; override it with TEASI_DEVICE=<serial> in the environment.
DEVICE = os.environ.get("TEASI_DEVICE", "2013021200000368").encode()

SECRET = b"d8ethebrestezexaqathaTrepedEkubafr5vuhaprupe3ucUphedeyuhaGespenU"
LZMA_FILTER = [{"id": lzma.FILTER_LZMA1, "dict_size": 1 << 24, "lc": 3, "lp": 0, "pb": 2}]


def header_md5(d: bytes, device: bytes = b"") -> bytes:
    return hashlib.md5(d[4:0x34] + SECRET + d[0x44:0x444] + d[-0x400:] + device).digest()


def tiles(d: bytes):
    n = struct.unpack_from("<I", d, 0x74)[0]
    dirs = [struct.unpack_from("<HHI", d, 0x78 + 8 * i) for i in range(n)]
    for i, (x, y, off) in enumerate(dirs):
        end = dirs[i + 1][2] if i + 1 < n else len(d)
        yield x, y, off, end


def records(d: bytes, start: int, end: int):
    """Yield (rel_offset, len, pltx, payload) of the records of one tile.

    Records form a gapless chain from +0x159C to the end of the tile
    ([u32 len][u32 pltx][payload], next = cur + 8 + len).  The firmware jumps
    to them via the four slot areas in the tile head (layers.AREAS); every
    record is in exactly one slot, so walking the chain yields the same set.
    """
    rel = 0x159C
    while start + rel + 8 <= end:
        ln, pltx = struct.unpack_from("<II", d, start + rel)
        if ln == 0 or start + rel + 8 + ln > end:
            break
        yield rel, ln, pltx, d[start + rel + 8 : start + rel + 8 + ln]
        rel += 8 + ln


def lzma_unpack(buf: bytes, size: int):
    try:
        dec = lzma.LZMADecompressor(format=lzma.FORMAT_RAW, filters=LZMA_FILTER)
        out = dec.decompress(buf, max_length=size)
        return out if len(out) == size else None
    except lzma.LZMAError:
        return None


def decode_record(payload: bytes, pltx: int, blob: bytes, key: bytes):
    """PC1 -> LZMA.  The osm slot-B records (routing graph) are stored
    LZMA-only, without PC1; those are recognised by the fallback."""
    rk = decrypt_blob(blob, key)
    out = lzma_unpack(decrypt_payload(payload, rk), pltx)
    if out is None:
        out = lzma_unpack(payload, pltx)
    return out


# Global PC1 key.  bikenav.exe (FUN_002050a0): if the first 8 chars of the
# device ID are in a built-in list (e.g. "20130212"), this static key is used;
# otherwise key = "%02X" hex string of MD5(device_id[:8]).
STATIC_KEY = b"E89ACE5CE51E0669B4BA068CE8F63990"
KNOWN_PREFIXES = {
    b"20130125", b"20130212", b"20130213", b"20130807", b"20131010", b"20131020",
    b"20131026", b"20150215", b"20160505", b"20160509", b"20161014", b"20161028",
    b"20170606", b"20170707", b"20180914", b"20181225", b"20190320", b"20190415",
    b"20190618",
}


def global_key(device_id: bytes) -> bytes:
    if device_id[:8] in KNOWN_PREFIXES:
        return STATIC_KEY
    return hashlib.md5(device_id[:8]).hexdigest().upper().encode()


if __name__ == "__main__":
    import sys

    if len(sys.argv) != 3:
        sys.exit("usage: chart.py <chart-file> <out-dir>   (dumps decrypted records)")
    path, outdir = sys.argv[1:]
    d = open(path, "rb").read()
    dev = DEVICE
    print("header MD5 (device-bound):", header_md5(d, dev) == d[0x34:0x44],
          "| generic:", header_md5(d) == d[0x34:0x44])
    key = global_key(dev)
    os.makedirs(outdir, exist_ok=True)
    ok = bad = 0
    for x, y, s, e in tiles(d):
        blob = d[s + 0x157C : s + 0x159C]
        for rel, ln, pltx, pl in records(d, s, e):
            out = decode_record(pl, pltx, blob, key)
            if out is None:
                bad += 1
                continue
            ok += 1
            open(os.path.join(outdir, f"{x}_{y}_{rel:08x}.bin"), "wb").write(out)
    print(f"{ok} records written, {bad} failed")
