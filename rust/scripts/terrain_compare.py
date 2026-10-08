"""Compare two terrain charts region by region: tables, elevation tiles, map images.

The elevation tiles have to be identical byte for byte -- apart from the OpenJPEG
version in their comment marker, which names the library that wrote them.  The map
images are compared as pixels, because the Rust port uses a different JPEG encoder.

usage: terrain_compare.py <a> <b> [--images]
"""
import io
import struct
import sys

import numpy as np
from PIL import Image

HEAD = 0x159C
NONE_JPG = 0xFFFFFFFD
NONE_DEM = 0xFFFFFFFF


def regions(d):
    n = struct.unpack_from("<I", d, 0x74)[0]
    out = {}
    for i in range(n):
        x, y, off = struct.unpack_from("<HHI", d, 0x78 + 8 * i)
        jpg, dem = [], []
        for k in range(85):
            s, ln = struct.unpack_from("<II", d, off + 0x1180 + 8 * k)
            jpg.append(None if s == NONE_JPG else d[off + s:off + s + ln])
            t = struct.unpack_from("<I", d, off + 0x1428 + 4 * k)[0]
            if t == NONE_DEM:
                dem.append(None)
            else:
                a0, a1, ln2 = struct.unpack_from("<HHI", d, off + t)
                dem.append((a0, a1, d[off + t + 8:off + t + 8 + ln2]))
        out[(x, y)] = (jpg, dem)
    return out


def norm(jp2):
    """the codestream with the OpenJPEG version in its comment blanked"""
    i = jp2.find(b"Created by OpenJPEG version ")
    return jp2 if i < 0 else jp2[:i] + b"Created by OpenJPEG version x.x.x" + jp2[i + 33:]


def pixels(jpg):
    return np.asarray(Image.open(io.BytesIO(jpg)).convert("RGB"), np.int16)


def main(pa, pb, images):
    a, b = regions(open(pa, "rb").read()), regions(open(pb, "rb").read())
    if a.keys() != b.keys():
        print("regions differ:", sorted(set(a) ^ set(b)))
        return 1
    bad = dem_eq = dem_ver = dem_dif = jpg_eq = jpg_dif = 0
    worst = (0, 0.0, None)
    sizes = [0, 0]
    for key in sorted(a):
        ja, da = a[key]
        jb, db = b[key]
        for k in range(85):
            if (da[k] is None) != (db[k] is None) or (ja[k] is None) != (jb[k] is None):
                print(f"{key} cell {k}: height present {da[k] is not None}/{db[k] is not None},"
                      f" image {ja[k] is not None}/{jb[k] is not None}")
                bad += 1
                continue
            if da[k] is not None:
                if da[k][:2] != db[k][:2]:
                    print(f"{key} cell {k}: a0/a1 {da[k][:2]} vs {db[k][:2]}")
                    bad += 1
                elif da[k][2] == db[k][2]:
                    dem_eq += 1
                elif norm(da[k][2]) == norm(db[k][2]):
                    dem_ver += 1
                else:
                    dem_dif += 1
                    if dem_dif <= 5:
                        print(f"{key} cell {k}: the JP2 differs"
                              f" ({len(da[k][2])} vs {len(db[k][2])} B)")
            if ja[k] is not None:
                sizes[0] += len(ja[k])
                sizes[1] += len(jb[k])
                if ja[k] == jb[k]:
                    jpg_eq += 1
                else:
                    jpg_dif += 1
                    if images:
                        pa_, pb_ = pixels(ja[k]), pixels(jb[k])
                        d = np.abs(pa_ - pb_)
                        rms = float(np.sqrt((d.astype(np.float64) ** 2).mean()))
                        if (int(d.max()), rms) > worst[:2]:
                            worst = (int(d.max()), rms, (key, k))
    print(f"elevation tiles: {dem_eq} byte-identical, {dem_ver} differing only in the"
          f" OpenJPEG version, {dem_dif} differing")
    print(f"map images: {jpg_eq} byte-identical, {jpg_dif} differing,"
          f" {sizes[0]} vs {sizes[1]} B in total")
    if images and worst[2]:
        print(f"  worst image {worst[2]}: max {worst[0]}, RMS {worst[1]:.2f} out of 255")
    return 1 if bad or dem_dif else 0


if __name__ == "__main__":
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if len(args) != 2:
        sys.exit(__doc__)
    sys.exit(main(args[0], args[1], "--images" in sys.argv))
