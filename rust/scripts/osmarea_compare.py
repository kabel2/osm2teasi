"""Compare two osmarea charts object by object (the Rust port against Python).

usage: osmarea_compare.py <chart A> <chart B> [max cells shown]
"""
import collections
import sys

sys.path.insert(0, __file__.rsplit("/", 3)[0] + "/tools")

import layers as L

ARRS = ("c3", "c5", "c6")


def cells(path):
    out = {}
    d = open(path, "rb").read()
    for tx, ty, cx, cy, g, raw in L.iter_records(d, "C"):
        rec = L.parse_c(raw)
        objs = {}
        for a in ARRS:
            objs[a] = [(it["s"][2] if a == "c5" else None, L.u16str(it["v"][0]),
                        L.geometry_parts(it["v"][-1])) for it in rec[a]]
        out[(cx, cy)] = objs
    return out


def key(o):
    """class, name, and the rings as (hi, points) -- the object's whole content"""
    return (o[0], o[1], tuple((hi, tuple(p)) for hi, p in o[2]))


def main(pa, pb, show=5):
    A, B = cells(pa), cells(pb)
    print(len(A), "cells vs", len(B))
    stat = collections.Counter()
    shown = 0
    for cell in sorted(set(A) | set(B)):
        a, b = A.get(cell), B.get(cell)
        if a is None or b is None:
            stat["cell only in " + ("B" if a is None else "A")] += 1
            continue
        if a == b:
            continue
        stat["cells differing"] += 1
        diff = []
        for arr in ARRS:
            ka = [key(o) for o in a[arr]]
            kb = [key(o) for o in b[arr]]
            if ka == kb:
                continue
            sa, sb = set(ka), set(kb)
            stat[f"{arr}: objects only in A"] += len(sa - sb)
            stat[f"{arr}: objects only in B"] += len(sb - sa)
            if sa == sb:
                stat[f"{arr}: same objects, different order"] += 1
            diff.append(f"{arr}: {len(ka)} vs {len(kb)} objects, "
                        f"{len(sa - sb)} only A, {len(sb - sa)} only B")
            for o in sorted(sa - sb)[:1]:
                other = [x for x in sb - sa if x[0] == o[0] and x[1] == o[1]]
                near = other[0] if other else None
                ra = [(h, len(p)) for h, p in o[2]]
                diff.append(f"    A: cls={o[0]} name={o[1]!r} {len(ra)} rings")
                if near:
                    rb = [(h, len(p)) for h, p in near[2]]
                    at = next((i for i in range(min(len(ra), len(rb))) if ra[i] != rb[i]), None)
                    diff.append(f"    B: cls={near[0]} name={near[1]!r} {len(rb)} rings"
                                + (f", first difference at ring {at}: {ra[at]} vs {rb[at]}"
                                   if at is not None else
                                   ", same rings, different points"))
        if shown < show:
            shown += 1
            print(f"cell {cell}:")
            for line in diff:
                print("  " + line)
    for k, v in sorted(stat.items()):
        print(f"{v:8d}  {k}")


if __name__ == "__main__":
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2], int(sys.argv[3]) if len(sys.argv) > 3 else 5)
