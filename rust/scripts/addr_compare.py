"""Compare two canonical address dumps (Python vs Rust).

Lines are matched by their tag fields, so entries whose coordinates differ
slightly still pair up; the deviation is reported in metres (1 Teasi unit =
0.149 m).  Exact equality is expected for everything except the centre of
areas, see rust/README.md.

usage: addr_compare.py <a.txt> <b.txt>
"""
import collections
import struct
import sys

M_PER_UNIT = 0.149


def load(path):
    """-> {(type, tag fields): [(x, y), ...]}; every line is type, x, y, fields."""
    out = collections.defaultdict(list)
    for line in open(path):
        f = line.rstrip("\n").split("\t")
        x, y = (struct.unpack("<d", struct.pack("<Q", int(v, 16)))[0] for v in f[1:3])
        out[(f[0], *f[3:])].append((x, y))
    return out


def main(pa, pb):
    a, b = load(pa), load(pb)
    print(f"{sum(map(len, a.values()))} / {sum(map(len, b.values()))} entries, "
          f"{len(a)} / {len(b)} keys")
    for t in "API":
        ka = {k for k in a if k[0] == t}
        kb = {k for k in b if k[0] == t}
        only_a, only_b = ka - kb, kb - ka
        same, moved, devs = 0, 0, []
        for k in ka & kb:
            for p, q in zip(sorted(a[k]), sorted(b[k])):
                if p == q:
                    same += 1
                else:
                    moved += 1
                    devs.append(((p[0] - q[0]) ** 2 + (p[1] - q[1]) ** 2) ** 0.5 * M_PER_UNIT)
        name = {"A": "addresses", "P": "places", "I": "interpolation points"}[t]
        print(f"  {name}: {same} identical, {moved} moved, "
              f"{len(only_a)} only in A, {len(only_b)} only in B")
        if devs:
            devs.sort()
            print(f"    deviation: median {devs[len(devs) // 2]:.2f} m, "
                  f"90% < {devs[int(len(devs) * 0.9)]:.2f} m, max {devs[-1]:.2f} m")
        for k in list(only_a)[:3]:
            print("    only in A:", k)
        for k in list(only_b)[:3]:
            print("    only in B:", k)


if __name__ == "__main__":
    main(*sys.argv[1:3])
