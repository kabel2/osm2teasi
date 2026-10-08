"""Search index of the ta layer: the extra table behind header 0x70 (TA_FORMAT.md).

The firmware registers it per country (FUN_0016068c) and walks it for the place search
(FUN_00162788, FUN_00163384, FUN_0016201c).  Offsets are relative to the table start.

  u32   pool offset (word pool for names, at the end of the table)
  u8    n languages, n x (3 ASCII chars, u8 index)   e.g. DAN 1, GER 2, ... POR 11
  u32   1
  u16   n + UTF-16 country name ("Denmark")
  u32   offset of the root node
  nodes (pre-order: a node, then its children's subtrees in child order)
  pool: u16 byte length + UTF-8 per word

Node:
  u32   (n children << 24) | distinct results in the subtree; 0xFFFFFFFF: u32 results,
        u32 children
  n x   u16 character, u64 language mask, u32 child offset
  u16   n results, then per result:
        u8 type: bit 7 = name from the pool; 0 = place with streets, 1 = postal code,
                 2 = place without streets (coordinates only)
        name: pool: u32 (n words << 24 | offset), (n - 1) x u32 offset (joined with " ")
              inline: u16 byte length + UTF-8
        f32 lon, f32 lat
        type 0, 1: u8 n cells (0xFF: u16 n), n x (u16 cell_x, u16 cell_y of the 4x4 A cell,
                   type 1 only: u16 k, k x u32 byte offsets of street names in blk5)

Keys (the path of characters to a node) are the words of the name and each multi-word
part, lower case without accents (fold()).
"""
import struct
import unicodedata

ALL = (1 << 64) - 1


def fold(s):
    """Search key normalisation (checked against all 13,121 Danish results)."""
    s = s.lower().replace("ø", "o").replace("æ", "a").replace("ß", "ss")
    return "".join(c for c in unicodedata.normalize("NFKD", s) if not unicodedata.combining(c))


class _R:
    def __init__(self, t, p):
        self.t, self.p = t, p

    def u(self, f):
        v = struct.unpack_from("<" + f, self.t, self.p)
        self.p += struct.calcsize("<" + f)
        return v if len(v) > 1 else v[0]


def parse(t):
    """-> {"langs": [(code, idx)], "one": u32, "country": str, "root": node, "pool": [words]}
    node = {"kids": [(char, mask, node)], "res": [result]},
    result = {"type", "pool": bool, "name": str or [word offsets], "lon", "lat", "cells"}"""
    r = _R(t, 0)
    pool_off = r.u("I")
    langs = [(bytes(r.u("3B")).decode(), r.u("B")) for _ in range(r.u("B"))]
    one = r.u("I")
    country = t[r.p + 2:r.p + 2 + 2 * struct.unpack_from("<H", t, r.p)[0]].decode("utf-16le")
    r.p += 2 + 2 * len(country)
    root_off = r.u("I")
    end = [0]

    def node(off):
        r = _R(t, off)
        h = r.u("I")
        n = r.u("II")[1] if h == 0xFFFFFFFF else h >> 24
        kids = [r.u("HQI") for _ in range(n)]
        res = []
        for _ in range(r.u("H")):
            ty = r.u("B")
            x = {"type": ty & 0x7F, "pool": bool(ty & 0x80)}
            if ty & 0x80:
                w = r.u("I")
                x["name"] = [w & 0xFFFFFF] + [r.u("I") for _ in range((w >> 24) - 1)]
            else:
                n = r.u("H")
                x["name"] = t[r.p:r.p + n].decode("utf-8")
                r.p += n
            x["lon"], x["lat"] = r.u("ff")
            x["cells"] = []
            if x["type"] != 2:
                n = r.u("B")
                if n == 0xFF:
                    n = r.u("H")
                for _ in range(n):
                    cx, cy = r.u("HH")
                    offs = [r.u("I") for _ in range(r.u("H"))] if x["type"] == 1 else None
                    x["cells"].append((cx, cy, offs))
            res.append(x)
        end[0] = max(end[0], r.p)
        return {"kids": [(chr(c), m, node(o)) for c, m, o in kids], "res": res}

    root = node(root_off)
    assert end[0] == pool_off, (end[0], pool_off)
    pool, p = {}, pool_off
    while p < len(t):
        n = struct.unpack_from("<H", t, p)[0]
        pool[p - pool_off] = t[p + 2:p + 2 + n].decode("utf-8")
        p += 2 + n
    return {"langs": langs, "one": one, "country": country, "root": root, "pool": pool}


def _ident(x):
    return x["type"], tuple(x["name"]) if x["pool"] else x["name"], x["lon"], x["lat"]


def results(nd, acc=None):
    """Distinct results in the subtree (a result is stored under each of its keys)."""
    acc = set() if acc is None else acc
    acc.update(_ident(x) for x in nd["res"])
    for _, _, k in nd["kids"]:
        results(k, acc)
    return acc


def count(nd):
    return len(results(nd))


def build(idx):
    """Inverse of parse.  idx["pool"]: {offset: word} (words referenced by pool names)."""
    head = bytearray(struct.pack("<I", 0))
    head += struct.pack("<B", len(idx["langs"]))
    for code, i in idx["langs"]:
        head += code.encode() + struct.pack("<B", i)
    head += struct.pack("<IH", idx["one"], len(idx["country"])) + idx["country"].encode("utf-16le")
    root_off = len(head) + 4
    head += struct.pack("<I", root_off)
    out = bytearray(head)

    def res_bytes(x):
        b = bytearray(struct.pack("<B", x["type"] | 0x80 * x["pool"]))
        if x["pool"]:
            w = x["name"]
            b += struct.pack("<I", len(w) << 24 | w[0]) + struct.pack("<%dI" % (len(w) - 1), *w[1:])
        else:
            s = x["name"].encode("utf-8")
            b += struct.pack("<H", len(s)) + s
        b += struct.pack("<ff", x["lon"], x["lat"])
        if x["type"] != 2:
            n = len(x["cells"])
            b += struct.pack("<B", n) if n < 0xFF else struct.pack("<BH", 0xFF, n)
            for cx, cy, offs in x["cells"]:
                b += struct.pack("<HH", cx, cy)
                if x["type"] == 1:
                    b += struct.pack("<H%dI" % len(offs), len(offs), *offs)
        return b

    def write(nd):
        """Append nd and its subtree at the end of out; -> (offset, distinct results)."""
        off = len(out)
        n = len(nd["kids"])
        big = n >= 0x100
        kid_at = off + (12 if big else 4)
        out.extend(bytes(kid_at - off + 14 * n))
        out.extend(struct.pack("<H", len(nd["res"])))
        for x in nd["res"]:
            out.extend(res_bytes(x))
        acc = {_ident(x) for x in nd["res"]}
        for i, (ch, mask, kid) in enumerate(nd["kids"]):
            ko, sub = write(kid)
            acc |= sub
            struct.pack_into("<HQI", out, kid_at + 14 * i, ord(ch), mask, ko)
        tot = len(acc)
        if big or tot >= 1 << 24:
            assert big, "more than 2^24 results needs the long header"
            struct.pack_into("<III", out, off, 0xFFFFFFFF, tot, n)
        else:
            struct.pack_into("<I", out, off, n << 24 | tot)
        return off, acc

    write(idx["root"])
    struct.pack_into("<I", out, 0, len(out))
    for o in sorted(idx["pool"]):
        s = idx["pool"][o].encode("utf-8")
        assert len(out) - struct.unpack_from("<I", out, 0)[0] == o
        out += struct.pack("<H", len(s)) + s
    return bytes(out)


if __name__ == "__main__":
    import sys
    t = open(sys.argv[1], "rb").read()
    idx = parse(t)
    print(idx["langs"], idx["one"], idx["country"], count(idx["root"]), "results,",
          len(idx["pool"]), "pool words")
    b = build(idx)
    print("rebuild identical:", b == t, len(b), len(t))
