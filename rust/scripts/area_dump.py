"""Dump the area pickle of osm_area_extract.py the way `teasi area` does.

usage: area_dump.py <area.pkl> <out>
"""
import hashlib
import pickle
import sys


def digest(r):
    h = hashlib.md5()
    for x, y in r:
        h.update(int(x).to_bytes(4, "little", signed=True))
        h.update(int(y).to_bytes(4, "little", signed=True))
    return h.hexdigest()[:8]


def main(src, dst):
    d = pickle.load(open(src, "rb"))
    with open(dst, "w") as out:
        for aid, t, polys in sorted(d["areas"], key=lambda a: a[0]):
            row = [f"a\t{aid}\t{len(polys)}"]
            for o, inners in polys:
                f = f"{len(o)},{len(inners)},{o[0][0]},{o[0][1]},{digest(o)}"
                f += "".join(f",{len(i)}:{digest(i)}" for i in inners)
                row.append(f)
            out.write("\t".join(row) + "\n")
        for wid, a, b, r, place in sorted(d["coast"], key=lambda c: c[0]):
            out.write(f"c\t{wid}\t{a}\t{b}\t{int(place == 'islet')}\t{len(r)}\t{digest(r)}\n")
    print(len(d["areas"]), "areas,", len(d["coast"]), "coastline ways ->", dst)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
