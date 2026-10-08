"""Compare two area dumps (area_dump.py vs `teasi area`), paired by id.

usage: area_compare.py <python dump> <rust dump>
"""
import collections
import sys


def load(path):
    areas, coast = {}, {}
    for line in open(path):
        f = line.rstrip("\n").split("\t")
        if f[0] == "a":
            areas[int(f[1])] = f[2:]
        else:
            coast[int(f[1])] = f[2:]
    return areas, coast


def main(pa, pb):
    A, Ac = load(pa)
    B, Bc = load(pb)
    stat = collections.Counter()
    examples = collections.defaultdict(list)
    for kind, a, b in (("area", A, B), ("coast", Ac, Bc)):
        for k in a.keys() | b.keys():
            x, y = a.get(k), b.get(k)
            if x is None or y is None:
                stat[f"{kind} only in {'rust' if x is None else 'python'}"] += 1
                examples[f"{kind} only in {'rust' if x is None else 'python'}"].append(k)
            elif x == y:
                stat[f"{kind} identical"] += 1
            elif kind == "area" and x[0] != y[0]:
                stat["area: different ring count"] += 1
                examples["area: different ring count"].append(k)
            else:
                stat[f"{kind}: same ids, different geometry"] += 1
                examples[f"{kind}: same ids, different geometry"].append(k)
    for k, v in sorted(stat.items()):
        ex = examples.get(k, [])[:6]
        print(f"{v:9d}  {k}" + (f"   e.g. {ex}" if ex else ""))


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
