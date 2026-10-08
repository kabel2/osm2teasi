//! Osmosis `.poly` files (a Geofabrik country boundary) and a point-in-polygon
//! test in Teasi units.  Port of tools/poly.py.

use anyhow::{Context, Result};

use crate::osm::xy;

/// One ring of the boundary; holes are cut out of the enclosing rings.
pub struct Ring {
    pub pts: Vec<(f64, f64)>,
    pub hole: bool,
}

/// Read a `.poly` file.  The first line is the name, then blocks of
/// "<ring name>" ... "END", a leading "!" marking a hole, closed by a final "END".
pub fn load(path: &str) -> Result<Vec<Ring>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("open {}", path))?;
    let mut rings: Vec<Ring> = Vec::new();
    let mut cur: Option<Ring> = None;
    for line in text.lines().skip(1) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some(r) = cur.as_mut() else {
            if line == "END" {
                break;
            }
            cur = Some(Ring { pts: Vec::new(), hole: line.starts_with('!') });
            continue;
        };
        if line == "END" {
            rings.push(cur.take().unwrap());
            continue;
        }
        let mut it = line.split_whitespace();
        let lon: f64 = it.next().context("empty coordinate line")?.parse()?;
        let lat: f64 = it.next().context("one coordinate only")?.parse()?;
        r.pts.push(xy(lon, lat));
    }
    Ok(rings)
}

/// Crossing number test; the ring is treated as closed.
pub fn inside(ring: &[(f64, f64)], x: f64, y: f64) -> bool {
    let mut c = false;
    for i in 0..ring.len() {
        let (x1, y1) = ring[i];
        let (x2, y2) = ring[(i + 1) % ring.len()];
        if (y1 > y) != (y2 > y) && x < x1 + (y - y1) * (x2 - x1) / (y2 - y1) {
            c = !c;
        }
    }
    c
}

/// Inside at least one ring and in none of the holes.
pub fn contains(rings: &[Ring], x: f64, y: f64) -> bool {
    rings.iter().any(|r| !r.hole && inside(&r.pts, x, y))
        && !rings.iter().any(|r| r.hole && inside(&r.pts, x, y))
}
