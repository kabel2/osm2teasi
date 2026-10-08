"""Elevation grid from the Copernicus DEM GLO-90, for `teasi terrain` and the B edge
ascents of `teasi osm --heights`.

Downloads the 1x1 deg tiles covering <area.poly> from the public AWS bucket
copernicus-dem-90m into <tile dir> (tiles that do not exist are open sea), resamples
them to one grid of 3" (1200 values per degree in both directions) and smooths it with
a Gaussian filter of SIGMA grid points: the DEM is a surface model (trees, buildings)
and its noise would add up to spurious ascents along the roads.

Pass the result through heights_export.py to get the flat file the Rust side reads.

Output (pickle): {"grid": float32 array (rows, cols), heights in m, "lon0", "lat0":
north-west corner in deg, "step": deg per grid point}

usage: dem_heights.py [--sigma=S] <area.poly> <tile dir> <out.pkl>
"""
import math
import os
import pickle
import sys
import time
import urllib.error
import urllib.request

import numpy as np
import tifffile
from scipy.ndimage import gaussian_filter

import poly


def cli(argv):
    """argv -> (positional args, {option: value}) for --key=value / --flag options"""
    opts = dict((a[2:].split("=", 1) + [""])[:2] for a in argv if a.startswith("--"))
    return [a for a in argv if not a.startswith("--")], opts


N = 1200                             # grid points per degree (3")
SIGMA = 1.0
URL = ("https://copernicus-dem-90m.s3.amazonaws.com/Copernicus_DSM_COG_30_{0}_00_{1}_00_DEM/"
       "Copernicus_DSM_COG_30_{0}_00_{1}_00_DEM.tif")


def tile_name(lat, lon):
    return (f"N{lat:02d}" if lat >= 0 else f"S{-lat:02d}",
            f"E{lon:03d}" if lon >= 0 else f"W{-lon:03d}")


def fetch(lat, lon, tdir):
    """-> path of the tile with south-west corner lat/lon, None if it does not exist"""
    a, b = tile_name(lat, lon)
    path = os.path.join(tdir, f"{a}_{b}.tif")
    if os.path.exists(path):
        return path
    if os.path.exists(path + ".none"):
        return None
    for attempt in range(5):
        try:
            data = urllib.request.urlopen(URL.format(a, b), timeout=60).read()
            break
        except urllib.error.HTTPError as e:
            if e.code not in (403, 404):
                raise
            open(path + ".none", "w").close()
            return None
        except (urllib.error.URLError, ConnectionError, TimeoutError):
            if attempt == 4:
                raise
            time.sleep(2 + 3 * attempt)
    open(path, "wb").write(data)
    return path


def build(rings, tdir, sigma=SIGMA, log=print):
    lon = [x / 2 ** 28 * 360 - 180 for r, h in rings for x, y in r]
    lat = [90 - y / 2 ** 28 * 360 for r, h in rings for x, y in r]
    lon0, lon1 = math.floor(min(lon)), math.ceil(max(lon))
    lat0, lat1 = math.floor(min(lat)), math.ceil(max(lat))
    grid = np.zeros(((lat1 - lat0) * N, (lon1 - lon0) * N), np.float32)
    n = 0
    for la in range(lat0, lat1):
        for lo in range(lon0, lon1):
            path = fetch(la, lo, tdir)
            if path is None:
                continue
            a = tifffile.imread(path)            # rows north -> south, width by latitude
            rows = (np.arange(N) * a.shape[0]) // N
            cols = (np.arange(N) * a.shape[1]) // N
            r = (lat1 - la - 1) * N
            c = (lo - lon0) * N
            grid[r:r + N, c:c + N] = np.maximum(a[np.ix_(rows, cols)], 0)
            n += 1
        log(f"  {la} N: {n} tiles")
    if sigma:
        grid = gaussian_filter(grid, sigma)
    return {"grid": grid, "lon0": float(lon0), "lat0": float(lat1), "step": 1 / N}


if __name__ == "__main__":
    args, opts = cli(sys.argv[1:])
    if len(args) < 3:
        sys.exit(__doc__)
    os.makedirs(args[1], exist_ok=True)
    g = build(poly.load(args[0]), args[1], float(opts.get("sigma", SIGMA)),
              log=lambda *a: print(*a, flush=True))
    pickle.dump(g, open(args[2], "wb"), protocol=5)
    print(g["grid"].shape, "->", args[2])
