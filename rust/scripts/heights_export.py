"""Convert the height pickles of osm_heights.py / dem_heights.py for `teasi osm`.

The Rust compiler reads the same heights as a flat little-endian file:

    "TEASIHT1" kind:u32   kind 0 = nodes, 1 = grid
      nodes: n:u64, then n float64 each of X, Y (360/2^28 units) and h (cm)
      grid:  rows:u64 cols:u64 lon0:f64 lat0:f64 step:f64, then rows*cols float32 (m)

usage: heights_export.py <heights.pkl> <out.bin>
"""
import pickle
import struct
import sys

import numpy as np


def main(src, dst):
    d = pickle.load(open(src, "rb"))
    with open(dst, "wb") as out:
        out.write(b"TEASIHT1")
        if "grid" in d:
            g = np.ascontiguousarray(d["grid"], dtype=np.float32)
            out.write(struct.pack("<IQQddd", 1, g.shape[0], g.shape[1],
                                  d["lon0"], d["lat0"], d["step"]))
            out.write(g.tobytes())
            print(f"grid {g.shape[0]}x{g.shape[1]}, {d['step']:.6f} deg -> {dst}")
        else:
            n = len(d["X"])
            out.write(struct.pack("<IQ", 0, n))
            for k in ("X", "Y", "h"):
                out.write(np.ascontiguousarray(d[k], dtype=np.float64).tobytes())
            print(f"{n} node heights -> {dst}")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
