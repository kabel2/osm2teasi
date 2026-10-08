"""Reconstruct node heights from the routing graph of an original osm chart.

B edge word [2] is the ascent (cm) in the edge direction.  For every edge pair
u->v / v->u, h(v) - h(u) = ascent(u->v) - ascent(v->u); solving these 2.1 million
equations in the least-squares sense gives a height per node (97.5 % of the edges within
50 cm).  The absolute offset is arbitrary (only differences are used).  compile_osm.py
interpolates the new edge ascents from these heights (OSM_FORMAT.md "Aus OSM erzeugen").

Output (pickle): {"X", "Y": node positions in 360/2^28 deg units, "h": height in cm}

usage: osm_heights.py <original osm> <out.pkl>
"""
import pickle
import sys

import numpy as np
from scipy.sparse import coo_matrix
from scipy.sparse.linalg import lsqr

import layers as L


def heights(orig):
    B = {(cx, cy): L.parse_b(raw) for _, _, cx, cy, _, raw in L.iter_records(orig, "B")}
    base, X, Y = {}, [], []
    for bc in sorted(B):
        base[bc] = len(X)
        for nd in B[bc]["nodes"]:          # cell mapped onto 0..65535 (65536 units)
            X.append(2 * (bc[0] * 65536 + (nd[1] & 0xFFFF) * 65536 / 65535))
            Y.append(2 * (bc[1] * 65536 + (nd[1] >> 16) * 65536 / 65535))
    asc = {}
    for bc, b in B.items():
        N, E = b["nodes"], b["edges"]
        for i in range(1, len(N)):
            f = N[i][0]
            for e in E[f & 0x7FFFF:(f & 0x7FFFF) + (f >> 26)]:
                t = e[3]
                tc = (bc[0] + (t >> 25) - 64, bc[1] + ((t >> 18) & 127) - 64)
                if tc in base:
                    asc[(base[bc] + i, base[tc] + (t & 0x3FFFF), e[0] & 0xFFFFF)] = \
                        e[2] if e[2] < 1 << 31 else 0
    rows, cols, vals, rhs = [], [], [], []
    for (u, v, ln), a in asc.items():
        if u < v and (v, u, ln) in asc:
            k = len(rhs)
            rows += [k, k]
            cols += [v, u]
            vals += [1.0, -1.0]
            rhs.append(a - asc[(v, u, ln)])
    A = coo_matrix((vals, (rows, cols)), shape=(len(rhs), len(X))).tocsr()
    h = lsqr(A, np.array(rhs, float), atol=1e-8, btol=1e-8, iter_lim=4000)[0]
    used = np.bincount(np.array(cols), minlength=len(X)) > 0
    r = A @ h - np.array(rhs)
    print(f"{len(rhs)} edge pairs, {used.sum()} nodes, residual < 50 cm: "
          f"{np.mean(np.abs(r) < 50):.3f}")
    return {"X": np.array(X)[used], "Y": np.array(Y)[used], "h": h[used]}


if __name__ == "__main__":
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    out = heights(open(sys.argv[1], "rb").read())
    pickle.dump(out, open(sys.argv[2], "wb"), protocol=5)
