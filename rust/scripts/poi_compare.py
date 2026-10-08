"""Compare two canonical POI dumps (Python vs Rust).

Objects are matched by kind and OSM id, so every difference is localised:
position, area centroid, name or attribute string.  Deviations are reported in
metres (1 Teasi unit = 0.149 m).

usage: poi_compare.py <a.txt> <b.txt>
"""
import collections
import struct
import sys

M_PER_UNIT = 0.149
KIND = {"n": "nodes", "w": "ways", "r": "relations"}


def num(v):
    return struct.unpack("<d", struct.pack("<Q", int(v, 16)))[0]


def load(path):
    out = {}
    for line in open(path):
        f = line.rstrip("\n").split("\t")
        pos = (num(f[3]), num(f[4]))
        c = tuple(num(v) for v in f[5].split(":")) if f[5] != "-" else None
        out[(f[0], int(f[1]))] = (int(f[2]), pos, c, f[6], f[7])
    return out


def dist(p, q):
    return ((p[0] - q[0]) ** 2 + (p[1] - q[1]) ** 2) ** 0.5 * M_PER_UNIT


def main(pa, pb):
    a, b = load(pa), load(pb)
    print(f"{len(a)} / {len(b)} candidates")
    for kind in "nwr":
        ka = {k for k in a if k[0] == kind}
        kb = {k for k in b if k[0] == kind}
        same = 0
        diff = collections.Counter()
        devs = []
        for k in ka & kb:
            x, y = a[k], b[k]
            if x == y:
                same += 1
                continue
            if x[0] != y[0]:
                diff["type"] += 1
            if x[3] != y[3] or x[4] != y[4]:
                diff["name/attrs"] += 1
            if x[1] != y[1]:
                diff["position"] += 1
                devs.append(dist(x[1], y[1]))
            if x[2] != y[2]:
                diff["centroid"] += 1
                if x[2] and y[2]:
                    devs.append(dist(x[2], y[2]))
        print(f"  {KIND[kind]}: {same} identical, {len(ka & kb) - same} differ"
              f"{' (' + ', '.join(f'{k} {v}' for k, v in sorted(diff.items())) + ')' if diff else ''}"
              f", {len(ka - kb)} only in A, {len(kb - ka)} only in B")
        if devs:
            devs.sort()
            print(f"    deviation: median {devs[len(devs) // 2]:.2f} m, "
                  f"90% < {devs[int(len(devs) * 0.9)]:.2f} m, max {devs[-1]:.2f} m")
        for k in sorted(ka - kb)[:3]:
            print("    only in A:", k, a[k][0], a[k][3])
        for k in sorted(kb - ka)[:3]:
            print("    only in B:", k, b[k][0], b[k][3])


if __name__ == "__main__":
    main(*sys.argv[1:3])
