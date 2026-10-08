"""Address lookup in a ta file, emulating the firmware (TA_FORMAT.md "Suche").

  place  -> search index result (type 0) -> its 4x4 cells
  cell   -> A record: 0x18 sub elements whose '|' alternatives contain the place name
            (FUN_00163844) -> street names of that group (blk5 range [2], count [3])
  street -> D records of the 8x8 D cells of the A cell, first a1 piece with that name offset
            whose left or right range holds the number with the right parity
            (FUN_00165a0c) -> position interpolated along the piece

usage: ta_lookup.py <ta file> <place> <street> [<number>]
"""
import struct
import sys

import layers as L
import ta_index
from chart import DEVICE, decode_record, global_key, tiles

NONE = 0xFFFF7FFF


class Ta:
    def __init__(self, path, device=DEVICE):
        self.d = open(path, "rb").read()
        self.key = global_key(device)
        off = struct.unpack_from("<I", self.d, 0x70)[0]
        self.slots = {}                              # (area, cell) -> (tile start, rel)
        for x, y, s, e in tiles(self.d):
            if s == e:
                continue
            for area in "AD":
                o, g = L.AREAS[area]
                for k, rel in enumerate(struct.unpack_from("<%dI" % (g * g), self.d, s + o)):
                    if rel != 0xFFFFFFFF:
                        self.slots[(area, (x * g + k // g, y * g + k % g))] = (s, rel)
        self.index = ta_index.parse(self.d[off:self._tile_end(off)])
        self._a, self._dd = {}, {}

    def raw(self, area, cell):
        """Decrypted record or None (decoded on demand: PC1 in Python is slow)."""
        if (area, cell) not in self.slots:
            return None
        s, rel = self.slots[(area, cell)]
        ln, pltx = struct.unpack_from("<II", self.d, s + rel)
        return decode_record(self.d[s + rel + 8:s + rel + 8 + ln], pltx,
                             self.d[s + 0x157C:s + 0x159C], self.key)

    def _tile_end(self, off):
        n = struct.unpack_from("<I", self.d, 0x74)[0]
        starts = sorted(struct.unpack_from("<HHI", self.d, 0x78 + 8 * i)[2] for i in range(n))
        return next((s for s in starts if s > off), len(self.d))

    def a(self, cell):
        if cell not in self._a:
            raw = self.raw("A", cell)
            self._a[cell] = L.parse_a(raw) if raw else None
        return self._a[cell]

    def d_rec(self, cell):
        if cell not in self._dd:
            raw = self.raw("D", cell)
            self._dd[cell] = L.parse_d(raw) if raw else None
        return self._dd[cell]

    def text(self, r):
        """Result name as a string (pool names are lists of word offsets)."""
        n = r["name"]
        return n if isinstance(n, str) else " ".join(self.index["pool"][o] for o in n)

    def find(self, key):
        """Results reachable under the search key prefix (all of its subtree)."""
        nd = self.index["root"]
        for ch in key:
            nd = next((k for c, _, k in nd["kids"] if c == ch), None)
            if nd is None:
                return []
        out, stack = [], [nd]
        while stack:
            n = stack.pop()
            out += n["res"]
            stack += [k for _, _, k in n["kids"]]
        return out

    def streets(self, cell, place):
        """{street name: [blk5 offsets]} of the groups of A cell whose alternatives hold
        place (all groups if place is None)."""
        r = self.a(cell)
        out = {}
        if r is None:
            return out
        names = names_by_offset(r)
        order = sorted(names)
        for sub in r["items"][0]["s18"]:
            alts = L.u16str(sub["str"]).split("#")[0].split("|")
            if place is not None and place not in alts:
                continue
            start, cnt = sub["s"][2], sub["s"][3]
            k = order.index(start) if cnt else 0
            for o in order[k:k + cnt]:
                out.setdefault(names[o], []).append(o)
        return out

    def address(self, place, street, number=None, cells=None):
        """Firmware chain: every (cell, offset) of the street under the place results named
        `place` (FUN_00165a0c tries them in turn) -> first position or None."""
        if cells is None:
            key = ta_index.fold(place)
            cells = sorted({(cx, cy) for r in self.find(key) if r["type"] == 0 and
                            self.text(r) == place for cx, cy, _ in r["cells"]})
        for cell in cells:
            for off in self.streets(cell, place).get(street, []):
                pos = self.locate(cell, off, number)
                if pos:
                    return pos
        return None

    def locate(self, acell, name_off, number):
        """First piece in the 8x8 D cells of acell -> (lat, lon) or None (FUN_00165a0c)."""
        for dx in range(8):
            for dy in range(8):
                cell = (acell[0] * 8 + dx, acell[1] * 8 + dy)
                rec = self.d_rec(cell)
                if rec is None:
                    continue
                for it in rec["a1"]:
                    s = it["s"]
                    if name_off not in (s[0], s[1]):
                        continue
                    t = fraction(s[2], number) if number is not None else 0.5
                    if t is None:
                        t = fraction(s[3], number)
                    if t is None:
                        continue
                    return along(cell, it["v"][0], t)
        return None


def names_by_offset(r):
    b5 = struct.unpack("<%dI" % (len(r["blk5"]) // 4), r["blk5"])
    b6 = r["blk6"]

    def word(o):
        n = struct.unpack_from("<H", b6, o)[0]
        return b6[o + 2:o + 2 + 2 * n].decode("utf-16le")
    out, i = {}, 0
    while i < len(b5):
        n = b5[i] >> 24
        out[4 * i] = " ".join(word(b5[i + j] & 0xFFFFFF) for j in range(n))
        i += max(n, 1)
    return out


def fraction(w, n):
    """Firmware rule for one side: range, parity (bit 15 = any), position 0..1 or None."""
    lo16, hi16 = w & 0xFFFF, w >> 16
    if lo16 == 0x7FFF or hi16 == 0xFFFF:
        return None
    a = lo16 & 0x7FFF
    lo, hi = min(a, hi16), max(a, hi16)
    if not (lo16 & 0x8000 or (lo ^ n) & 1 == 0) or not lo <= n <= hi:
        return None
    t = (n - lo + 0.5) / (hi - lo + 1)
    return 1 - t if a != lo else t


def along(cell, geo, t):
    pts = [p for _, part in L.geometry_parts(geo) for p in part]
    xy = [((p & 0xFFFF) - 512, (p >> 16) - 512) for p in pts]
    segs = [((ax, ay), (bx, by), ((bx - ax) ** 2 + (by - ay) ** 2) ** 0.5)
            for (ax, ay), (bx, by) in zip(xy, xy[1:])]
    d = t * sum(s for _, _, s in segs)
    for (ax, ay), (bx, by), s in segs:
        if d <= s and s:
            f = d / s
            u, v = ax + f * (bx - ax), ay + f * (by - ay)
            break
        d -= s
    else:
        u, v = xy[-1]
    return L.to_latlon(cell[0], cell[1], 32, int(round(v)) << 16 | int(round(u)))


if __name__ == "__main__":
    if len(sys.argv) < 4:
        sys.exit(__doc__)
    ta = Ta(sys.argv[1])
    place, street = sys.argv[2], sys.argv[3]
    number = int(sys.argv[4]) if len(sys.argv) > 4 else None
    key = ta_index.fold(place)
    res = [r for r in ta.find(key) if r["type"] == 0 and
           ta_index.fold(ta.text(r)).split(",")[0].split(" (")[0].strip() == key]
    print(len(res), "place results:", sorted({ta.text(r) for r in res})[:10])
    for name in sorted({ta.text(r) for r in res}):
        pos = ta.address(name, street, number)
        if pos:
            print(f"{name!r}: {street} {number or ''} -> {pos[0]:.6f}, {pos[1]:.6f}")
