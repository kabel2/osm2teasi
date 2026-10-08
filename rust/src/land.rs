//! The worldwide land polygons of osmdata.openstreetmap.de
//! (`land-polygons-split-4326`, built from the whole world's
//! `natural=coastline`), cut to the tiles around a country.  Includes its
//! own little shapefile reader -- a polygon shapefile is a flat sequence of
//! records, so no crate is needed.
//!
//! `osmarea.rs` uses these for countries without an original chart file: the
//! sea of a cell is the cell minus the land here.  A country extract alone is
//! not enough, its coastline stops at the boundary.

use anyhow::{bail, Context, Result};

use crate::area::SCALE;
use crate::geos::Geom;
use crate::poly::Ring;

/// degrees per tile
const TILE: f64 = 1.40625;

fn i32_le(d: &[u8], o: usize) -> i32 {
    i32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}

fn u32_be(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes(d[o..o + 4].try_into().unwrap())
}

fn f64_le(d: &[u8], o: usize) -> f64 {
    f64::from_le_bytes(d[o..o + 8].try_into().unwrap())
}

/// Signed area of a ring, pyshp's convention: positive is counter-clockwise.
/// A shapefile's outer rings run clockwise, its holes counter-clockwise.
fn ccw(r: &[(f64, f64)]) -> bool {
    let mut s = 0.0;
    for i in 0..r.len() {
        let (x1, y1) = r[i];
        let (x2, y2) = r[(i + 1) % r.len()];
        s += x1 * y2 - x2 * y1;
    }
    s > 0.0
}

/// Crossing number test, for attaching a hole to its outer ring.
fn inside(r: &[(f64, f64)], p: (f64, f64)) -> bool {
    let mut c = false;
    for i in 0..r.len() {
        let (x1, y1) = r[i];
        let (x2, y2) = r[(i + 1) % r.len()];
        if (y1 > p.1) != (y2 > p.1) && p.0 < x1 + (p.1 - y1) * (x2 - x1) / (y2 - y1) {
            c = !c;
        }
    }
    c
}

/// The polygons of `shp` inside the whole tiles around `rings`, in file order.
pub fn extract(shp: &str, rings: &[Ring]) -> Result<Vec<Geom>> {
    let s = (1u64 << 28) as f64 / 360.0;
    let lon: Vec<f64> = rings.iter().flat_map(|r| r.pts.iter().map(|p| p.0 / s - 180.0)).collect();
    let lat: Vec<f64> = rings.iter().flat_map(|r| r.pts.iter().map(|p| 90.0 - p.1 / s)).collect();
    if lon.is_empty() {
        bail!("empty boundary");
    }
    let (lo0, lo1) = (lon.iter().cloned().fold(f64::MAX, f64::min), lon.iter().cloned().fold(f64::MIN, f64::max));
    let (la0, la1) = (lat.iter().cloned().fold(f64::MAX, f64::min), lat.iter().cloned().fold(f64::MIN, f64::max));
    // whole tiles: the osmarea tiles are written completely
    let x0 = ((lo0 + 180.0) / TILE).floor() * TILE - 180.0;
    let x1 = (((lo1 + 180.0) / TILE).floor() + 1.0) * TILE - 180.0;
    let y1 = 90.0 - ((90.0 - la1) / TILE).floor() * TILE;
    let y0 = 90.0 - (((90.0 - la0) / TILE).floor() + 1.0) * TILE;

    let d = std::fs::read(shp).with_context(|| format!("open {}", shp))?;
    if d.len() < 100 || u32_be(&d, 0) != 9994 {
        bail!("{}: not a shapefile", shp);
    }
    let mut out: Vec<Geom> = Vec::new();
    let mut p = 100;
    while p + 8 <= d.len() {
        let len = 2 * u32_be(&d, p + 4) as usize;
        let (c, next) = (p + 8, p + 8 + len);
        p = next;
        if next > d.len() || len < 4 || i32_le(&d, c) != 5 {
            continue; // null shape or something that is not a polygon
        }
        let (bx0, by0) = (f64_le(&d, c + 4), f64_le(&d, c + 12));
        let (bx1, by1) = (f64_le(&d, c + 20), f64_le(&d, c + 28));
        if !(x0 <= bx1 && x1 >= bx0 && y0 <= by1 && y1 >= by0) {
            continue;
        }
        let nparts = i32_le(&d, c + 36) as usize;
        let npoints = i32_le(&d, c + 40) as usize;
        let pts = c + 44 + 4 * nparts;
        let at = |i: usize| (f64_le(&d, pts + 16 * i), f64_le(&d, pts + 16 * i + 8));
        let mut parts: Vec<Vec<(f64, f64)>> = Vec::with_capacity(nparts);
        for k in 0..nparts {
            let from = i32_le(&d, c + 44 + 4 * k) as usize;
            let to = if k + 1 < nparts { i32_le(&d, c + 48 + 4 * k) as usize } else { npoints };
            parts.push(
                (from..to).map(|i| at(i)).map(|(x, y)| ((x + 180.0) * SCALE, (90.0 - y) * SCALE)).collect(),
            );
        }
        // clockwise rings are outer, counter-clockwise ones holes (the split
        // dataset has one ring per shape, so this is for completeness)
        let mut polys: Vec<(Vec<(f64, f64)>, Vec<Vec<(f64, f64)>>)> = Vec::new();
        for r in &parts {
            if r.len() < 4 {
                continue;
            }
            if !ccw(r) || polys.is_empty() {
                polys.push((r.clone(), Vec::new()));
            } else {
                let at = polys.iter().position(|(o, _)| inside(o, r[0])).unwrap_or(0);
                polys[at].1.push(r.clone());
            }
        }
        for (o, h) in &polys {
            let g = Geom::polygon(o, h)?.clip_by_rect(
                (x0 + 180.0) * SCALE,
                (90.0 - y1) * SCALE,
                (x1 + 180.0) * SCALE,
                (90.0 - y0) * SCALE,
            )?;
            out.extend(g.polygons()?);
        }
    }
    Ok(out)
}
