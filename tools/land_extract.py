"""Cut the land polygons of osmdata.openstreetmap.de (land-polygons-split-4326, built from
the worldwide natural=coastline) to the tiles around a country.

compile_osmarea.py --land=<pkl> builds the sea (#OW) as cell minus these polygons.  A
country extract alone is not enough: its coastline stops at the boundary and the
neighbouring coasts are missing (Denmark used the sea of the original file instead).

Output (pickle): list of shapely polygons in 360/2^25 deg units (X east, Y south from 90N),
covering every tile (1.40625 deg) that touches <area.poly>.

usage: land_extract.py <land_polygons.shp> <area.poly> <out.pkl>
"""
import pickle
import sys

import shapely
import shapefile
from shapely.geometry import shape

import poly

SCALE = 2 ** 25 / 360.0
TILE = 1.40625


def extract(shp, rings):
    lon = [x / 2 ** 28 * 360 - 180 for r, h in rings for x, y in r]
    lat = [90 - y / 2 ** 28 * 360 for r, h in rings for x, y in r]
    # whole tiles: the osmarea tiles are written completely
    x0 = (min(lon) + 180) // TILE * TILE - 180
    x1 = ((max(lon) + 180) // TILE + 1) * TILE - 180
    y1 = 90 - (90 - max(lat)) // TILE * TILE
    y0 = 90 - ((90 - min(lat)) // TILE + 1) * TILE
    out = []
    r = shapefile.Reader(shp)
    for s in r.iterShapes(bbox=[x0, y0, x1, y1]):
        g = shapely.transform(shape(s.__geo_interface__),
                              lambda c: ((c + [180, -90]) * [SCALE, -SCALE]))
        g = shapely.clip_by_rect(g, (x0 + 180) * SCALE, (90 - y1) * SCALE,
                                 (x1 + 180) * SCALE, (90 - y0) * SCALE)
        out += [p for p in getattr(g, "geoms", [g]) if p.geom_type == "Polygon" and not p.is_empty]
    return out


if __name__ == "__main__":
    if len(sys.argv) < 4:
        sys.exit(__doc__)
    land = extract(sys.argv[1], poly.load(sys.argv[2]))
    pickle.dump(land, open(sys.argv[3], "wb"), protocol=5)
    print(len(land), "land polygons ->", sys.argv[3])
