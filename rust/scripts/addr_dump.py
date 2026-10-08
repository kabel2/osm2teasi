"""Canonical dump of an osm_addr_extract.py pickle, to compare with `teasi addr`.

Floats are printed as their IEEE bit pattern, so the comparison is exact.

usage: addr_dump.py <addr.pkl> <out.txt>
"""
import pickle
import struct
import sys


def bits(v):
    return "%016x" % struct.unpack("<Q", struct.pack("<d", v))[0]


def flat(s):
    return "" if s is None else str(s).replace("\t", " ").replace("\n", " ").replace("\r", " ")


def main(src, dst):
    d = pickle.load(open(src, "rb"))
    lines = []
    for rec in d["addr"]:
        x, y = rec[0], rec[1]
        f = list(rec[2:]) + [None] * (6 - len(rec[2:]))      # older pickles lack addr:suburb
        lines.append("A\t%s\t%s\t%s" % (bits(x), bits(y), "\t".join(flat(v) for v in f)))
    for x, y, place, name, tags in d["places"]:
        lines.append("P\t%s\t%s\t%s" % (bits(x), bits(y), "\t".join(
            [flat(tags.get("place")), flat(tags.get("name")), flat(tags.get("name:en")),
             flat(tags.get("population")), flat(tags.get("is_in")),
             "area" if tags.get("area") else ""])))
    for kind, pts, street in d["interp"]:
        for n, (x, y, hn, st) in enumerate(pts):
            lines.append("I\t%s\t%s\t%s\t%s\t%d\t%d\t%s\t%s" % (
                bits(x), bits(y), flat(kind), flat(street), len(pts), n, flat(hn), flat(st)))
    lines.sort()
    open(dst, "w").write("\n".join(lines) + "\n")
    print(len(lines), "lines ->", dst)


if __name__ == "__main__":
    main(*sys.argv[1:3])
