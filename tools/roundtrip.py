"""Container round trip: decode a chart file completely, rebuild it with writer.py and
check that every record decodes to the same plaintext and the MAC verifies.

usage: roundtrip.py <chart-file> [<out-file>]
"""
import struct
import sys
import time

from chart import DEVICE, tiles, records, header_md5
from layers import AREAS, iter_records
from writer import write_chart

DEV = DEVICE


def read_all(d):
    """-> [(x, y, {area: {slot: plaintext}} or None, tail)] for write_chart."""
    out = {}
    order = []
    for x, y, s, e in tiles(d):
        order.append((x, y, s, e))
    per = {(x, y): {} for x, y, s, e in order}
    for a, (_, g) in AREAS.items():
        for tx, ty, cx, cy, gg, raw in iter_records(d, a, DEV):
            assert raw is not None, (a, tx, ty, cx, cy)
            per[(tx, ty)].setdefault(a, {})[(cx % g) * g + cy % g] = raw
    res = []
    for x, y, s, e in order:
        if s == e:
            res.append((x, y, None, b""))
            continue
        # records() would also walk into ta's extra table (its first u32 looks like a
        # length), so the chain ends after the last record referenced by a slot
        slots = {v for a, (off, g) in AREAS.items()
                 for v in struct.unpack_from("<%dI" % (g * g), d, s + off)}
        end = s + 0x159C
        for rel, ln, pltx, pl in records(d, s, e):
            if rel not in slots:
                break
            end = s + rel + 8 + ln
        res.append((x, y, per[(x, y)], d[end:e]))
    return res


def meta_of(d):
    date = d[0x44:0x4C]
    typ, layer, country = struct.unpack_from("<3I", d, 0x4C)
    m = {"date": date, "type": typ, "layer": layer, "country": country}
    extra = struct.unpack_from("<I", d, 0x70)[0]
    if extra != 0xFFFFFFFF:
        for x, y, s, e in tiles(d):
            if s <= extra < e:
                m["tail_tile"] = (x, y)
    return m


if __name__ == "__main__":
    src = sys.argv[1]
    d = open(src, "rb").read()
    t0 = time.time()
    content = read_all(d)
    new = write_chart(meta_of(d), content, DEV)
    t1 = time.time()
    if len(sys.argv) > 2:
        open(sys.argv[2], "wb").write(new)
    assert new[:4] == d[:4] and new[0x44:0x58] == d[0x44:0x58]
    assert header_md5(new, DEV) == new[0x34:0x44], "MAC"
    old_h = struct.unpack_from("<8I", d, 0x58)
    new_h = struct.unpack_from("<8I", new, 0x58)
    print("header 0x58..0x74 old", [hex(v) for v in old_h])
    print("                  new", [hex(v) for v in new_h])
    assert [t[:2] for t in tiles(d)] == [t[:2] for t in tiles(new)], "directory"
    n = 0
    for a in AREAS:
        old = list(iter_records(d, a, DEV))
        rebuilt = list(iter_records(new, a, DEV))
        assert old == rebuilt, a
        n += len(old)
    for (x, y, s, e), (x2, y2, s2, e2) in zip(tiles(d), tiles(new)):
        assert d[s + 0x1180:s + 0x157C] == new[s2 + 0x1180:s2 + 0x157C]
    tails = [c[3] for c in content]
    assert [c[3] for c in read_all(new)] == tails, "tail"
    print(f"{src.split('/')[-1]}: {n} records identical, size {len(d)} -> {len(new)}, "
          f"tails {sum(map(len, tails))} B, build {t1 - t0:.0f} s")
