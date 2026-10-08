"""Compare two `teasi ways` dumps (Python vs Rust), paired by way id.

usage: ways_compare.py <python.txt> <rust.txt>
"""
import collections
import sys


def load(p):
    out = {}
    for line in open(p):
        f = line.rstrip("\n").split("\t")
        out[int(f[0])] = tuple(f[1:])
    return out


def main(pa, pb):
    a, b = load(pa), load(pb)
    only_a = sorted(set(a) - set(b))
    only_b = sorted(set(b) - set(a))
    same, diff = 0, collections.Counter()
    examples = collections.defaultdict(list)
    for k in sorted(set(a) & set(b)):
        if a[k] == b[k]:
            same += 1
            continue
        for i, name in enumerate(("kind", "type", "flags", "rows", "geometry", "name")):
            if a[k][i] != b[k][i]:
                diff[name] += 1
                if len(examples[name]) < 5:
                    examples[name].append(f"way {k}: {a[k][i]!r} / {b[k][i]!r}")
    print(f"{same} of {len(set(a) & set(b))} ways identical, "
          f"{len(only_a)} only in A, {len(only_b)} only in B")
    for name, n in diff.most_common():
        print(f"  {name}: {n}")
        for e in examples[name]:
            print(f"    {e}")
    for k in only_a[:5]:
        print(f"  only in A: way {k} {a[k]}")
    for k in only_b[:5]:
        print(f"  only in B: way {k} {b[k]}")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
