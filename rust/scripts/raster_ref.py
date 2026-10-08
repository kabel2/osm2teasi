"""Reference values for the raster tests of tests/compat.rs: PIL draws and resizes and
numpy shades, we print the md5 of the result.  The shapes come from the same little
generator the test uses, so both sides see the same input.

usage: raster_ref.py
"""
import hashlib
import sys

import numpy as np
from PIL import Image, ImageDraw

sys.path.insert(0, __file__.rsplit("/rust/", 1)[0] + "/tools")
from compile_terrain import shade

MASK = 2 ** 64 - 1


class Lcg:
    def __init__(self, seed):
        self.s = seed

    def next(self):
        self.s = (self.s * 6364136223846793005 + 1442695040888963407) & MASK
        return (self.s >> 11) / 2 ** 53


def polygons():
    r = Lcg(12345)
    h = hashlib.md5()
    for _ in range(200):
        k = 3 + int(r.next() * 7)
        pts = [(-6.0 + r.next() * 44.0, -6.0 + r.next() * 44.0) for _ in range(k)]
        im = Image.new("L", (32, 32), 0)
        ImageDraw.Draw(im).polygon(pts, fill=1)
        h.update(im.tobytes())
    return h.hexdigest()


def resizes():
    im = Image.new("RGB", (64, 64))
    px = im.load()
    for x in range(64):
        for y in range(64):
            px[x, y] = ((x * 37 + y * 17) % 251, (x * 11 + y * 53) % 241,
                        (x * 73 + y * 29) % 233)
    out = []
    for n in (32, 16, 8):
        out.append(hashlib.md5(im.resize((n, n), Image.LANCZOS).tobytes()).hexdigest())
    return out


def shading():
    """shade() on a height field without transcendentals, so that both sides
    start from exactly the same numbers"""
    side = 18
    i, j = np.indices((side, side))
    h = ((i * 7 + j * 13) % 23) * 3.5 - ((i * j) % 5)
    s = shade(h.astype(np.float64), 55.0)
    return hashlib.md5(np.ascontiguousarray(s, "<f8").tobytes()).hexdigest()


if __name__ == "__main__":
    print("polygons:", polygons())
    for n, d in zip((32, 16, 8), resizes()):
        print(f"resize {n}: {d}")
    print("shade:", shading())
