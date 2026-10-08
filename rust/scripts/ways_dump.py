"""Dump the way pickle of osm_extract.py the way `teasi ways` does.

usage: ways_dump.py <ways.pkl> <out>
"""
import hashlib
import os
import pickle
import struct
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "tools"))
from compile_osm import flags, line_type, road_class       # noqa: E402


def main(src, dst):
    d = pickle.load(open(src, "rb"))
    W, NID, X, Y = d["w"], d["nid"], d["X"], d["Y"]
    rels = d["rels"]
    starts, ends = {}, {}
    for i, k in enumerate(W.tolist()):
        if k not in starts:
            starts[k] = i
        ends[k] = i + 1
    rows = []
    for k, (wid, t) in enumerate(d["ways"]):
        c = road_class(t)
        if c is not None:
            kind, ty, name = "r", c, t.get("name", "")
        else:
            lt = line_type(t)
            if lt is None:
                continue                      # these the Rust extractor drops
            kind, ty = ("3", lt[1]) if lt[0] == "a3" else ("4", lt[1])
            name = lt[2] if lt[0] == "a4" else ""
        a, b = starts.get(k, 0), ends.get(k, 0)
        h = hashlib.md5()
        for i in range(a, b):
            h.update(int(NID[i]).to_bytes(8, "little", signed=True))
            h.update(struct.pack("<dd", X[i], Y[i]))
        rows.append((wid, kind, ty, flags(t, rels.get(wid, [])), b - a,
                     h.hexdigest()[:8], name))
    rows.sort()
    with open(dst, "w") as out:
        for r in rows:
            out.write("\t".join(str(v) for v in r) + "\n")
    print(len(rows), "ways ->", dst)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
