"""Canonical dump of an osm_poi_extract.py pickle, to compare with `teasi poi`.

Floats are printed as their IEEE bit pattern, so the comparison is exact.
Only the objects compile_osmpoi.poi_type accepts are dumped -- `teasi poi`
keeps exactly those.

usage: poi_dump.py <poi.pkl> <out.txt>
"""
import pickle
import struct
import sys

sys.path.insert(0, __file__.rsplit("/rust/", 1)[0] + "/tools")
from compile_osmpoi import attributes, poi_type      # noqa: E402


def bits(v):
    return "%016x" % struct.unpack("<Q", struct.pack("<d", v))[0]


def flat(s):
    return "" if s is None else str(s).replace("\t", " ").replace("\n", " ").replace("\r", " ")


def main(src, dst):
    lines = []
    for kind, oid, x, y, t in pickle.load(open(src, "rb")):
        typ = poi_type(t, kind)
        if typ is None:
            continue
        c = t.get("@area") if kind == "w" else None
        lines.append("%s\t%d\t%d\t%s\t%s\t%s\t%s\t%s" % (
            kind, oid, typ, bits(x), bits(y),
            "%s:%s" % (bits(c[0]), bits(c[1])) if c else "-",
            flat(t.get("name", "")), flat(attributes(t))))
    lines.sort()
    open(dst, "w").write("\n".join(lines) + "\n")
    print(len(lines), "lines ->", dst)


if __name__ == "__main__":
    main(*sys.argv[1:3])
