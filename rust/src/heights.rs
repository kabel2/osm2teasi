//! Node heights for the ascents of the routing graph (B edge word [2]).
//!
//! Two sources, as on the Python side: heights per node, reconstructed from the
//! routing graph of an original chart (`tools/osm_heights.py`), or an elevation
//! grid from the Copernicus DEM (`tools/dem_heights.py`).  Both arrive here as
//! the flat binary that `scripts/heights_export.py` writes from their pickles:
//!
//! ```text
//! "TEASIHT1" kind:u32  kind 0 = nodes, 1 = grid
//!   nodes: n:u64, then n f64 each of X, Y (360/2^28 units) and h (cm)
//!   grid:  rows:u64 cols:u64 lon0:f64 lat0:f64 step:f64, then rows*cols f32 (m)
//! ```

use anyhow::{bail, ensure, Result};
use rayon::prelude::*;

use crate::pbf::SCALE;

/// Number of known nodes the height of a point is interpolated from.
const K: usize = 4;

pub enum Heights {
    /// Known heights at scattered points; inverse-distance interpolation.
    Nodes { x: Vec<f64>, y: Vec<f64>, h: Vec<f64>, grid: Buckets },
    /// Regular grid, bilinear interpolation.
    Grid { rows: usize, cols: usize, lon0: f64, lat0: f64, step: f64, z: Vec<f32> },
}

struct Cur<'a>(&'a [u8], usize);

impl Cur<'_> {
    fn u64(&mut self) -> Result<u64> {
        ensure!(self.1 + 8 <= self.0.len(), "heights file truncated");
        let v = u64::from_le_bytes(self.0[self.1..self.1 + 8].try_into().unwrap());
        self.1 += 8;
        Ok(v)
    }
    fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.u64()?))
    }
    fn f64s(&mut self, n: usize) -> Result<Vec<f64>> {
        ensure!(self.1 + 8 * n <= self.0.len(), "heights file truncated");
        let v = self.0[self.1..self.1 + 8 * n]
            .chunks_exact(8)
            .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
            .collect();
        self.1 += 8 * n;
        Ok(v)
    }
}

impl Heights {
    pub fn load(path: &str) -> Result<Heights> {
        let d = std::fs::read(path)?;
        ensure!(d.len() > 12 && &d[..8] == b"TEASIHT1", "{}: not a heights file", path);
        let kind = u32::from_le_bytes(d[8..12].try_into().unwrap());
        let mut c = Cur(&d, 12);
        match kind {
            0 => {
                let n = c.u64()? as usize;
                let (x, y, h) = (c.f64s(n)?, c.f64s(n)?, c.f64s(n)?);
                ensure!(n >= K, "only {} known heights", n);
                let grid = Buckets::new(&x, &y);
                Ok(Heights::Nodes { x, y, h, grid })
            }
            1 => {
                let (rows, cols) = (c.u64()? as usize, c.u64()? as usize);
                let (lon0, lat0, step) = (c.f64()?, c.f64()?, c.f64()?);
                ensure!(c.1 + 4 * rows * cols <= d.len(), "heights file truncated");
                let z = d[c.1..c.1 + 4 * rows * cols]
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                    .collect();
                Ok(Heights::Grid { rows, cols, lon0, lat0, step, z })
            }
            _ => bail!("unknown heights kind {}", kind),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Heights::Nodes { x, .. } => format!("{} known node heights", x.len()),
            Heights::Grid { rows, cols, step, .. } => {
                format!("{}x{} grid, {:.5} deg", rows, cols, step)
            }
        }
    }

    /// Heights in cm at the given points (360/2^28 units), as the Python
    /// compiler computes them into its `H` array.
    pub fn at(&self, px: &[f64], py: &[f64]) -> Vec<f64> {
        match self {
            Heights::Grid { rows, cols, lon0, lat0, step, z } => px
                .par_iter()
                .zip(py)
                .map(|(&x, &y)| {
                    // bilinear, clipped to the grid like grid_sample()
                    let c = ((x / SCALE - 180.0) - lon0) / step;
                    let r = (lat0 - (90.0 - y / SCALE)) / step;
                    let c = c.clamp(0.0, *cols as f64 - 1.001);
                    let r = r.clamp(0.0, *rows as f64 - 1.001);
                    let (c0, r0) = (c as usize, r as usize);
                    let (fc, fr) = (c - c0 as f64, r - r0 as f64);
                    let at = |r: usize, c: usize| f64::from(z[r * cols + c]);
                    let v = (at(r0, c0) * (1.0 - fc) + at(r0, c0 + 1) * fc) * (1.0 - fr)
                        + (at(r0 + 1, c0) * (1.0 - fc) + at(r0 + 1, c0 + 1) * fc) * fr;
                    100.0 * v
                })
                .collect(),
            Heights::Nodes { x, y, h, grid } => px
                .par_iter()
                .zip(py)
                .map(|(&qx, &qy)| {
                    let near = grid.nearest(x, y, qx, qy);
                    // 1 / max(dist, 1)**2, then a sequential sum of four terms
                    // -- numpy sums the last axis of a (n, 4) array that way
                    let mut num = 0.0;
                    let mut den = 0.0;
                    for (j, &(d2, i)) in near.iter().enumerate() {
                        let m = d2.sqrt().max(1.0);
                        let w = 1.0 / (m * m);
                        if j == 0 {
                            num = w * h[i];
                            den = w;
                        } else {
                            num += w * h[i];
                            den += w;
                        }
                    }
                    num / den
                })
                .collect(),
        }
    }
}

/// A uniform grid over the known points, for the k nearest neighbours.
/// scipy uses a kd-tree; with four neighbours out of a dense cloud the result
/// is the same set, and ties are broken by index here.
pub struct Buckets {
    x0: f64,
    y0: f64,
    size: f64,
    nx: usize,
    ny: usize,
    start: Vec<u32>,
    items: Vec<u32>,
}

impl Buckets {
    fn new(x: &[f64], y: &[f64]) -> Buckets {
        let n = x.len();
        let (mut x0, mut x1) = (f64::MAX, f64::MIN);
        let (mut y0, mut y1) = (f64::MAX, f64::MIN);
        for i in 0..n {
            x0 = x0.min(x[i]);
            x1 = x1.max(x[i]);
            y0 = y0.min(y[i]);
            y1 = y1.max(y[i]);
        }
        // about two points per bucket
        let size = (((x1 - x0) * (y1 - y0) / n as f64) * 2.0).sqrt().max(1.0);
        let nx = (((x1 - x0) / size) as usize + 1).max(1);
        let ny = (((y1 - y0) / size) as usize + 1).max(1);
        let cell = |i: usize| {
            let cx = (((x[i] - x0) / size) as usize).min(nx - 1);
            let cy = (((y[i] - y0) / size) as usize).min(ny - 1);
            cy * nx + cx
        };
        let mut count = vec![0u32; nx * ny + 1];
        for i in 0..n {
            count[cell(i) + 1] += 1;
        }
        for i in 0..nx * ny {
            count[i + 1] += count[i];
        }
        let start = count.clone();
        let mut items = vec![0u32; n];
        let mut at = count;
        for i in 0..n {
            let c = cell(i);
            items[at[c] as usize] = i as u32;
            at[c] += 1;
        }
        Buckets { x0, y0, size, nx, ny, start, items }
    }

    /// The K nearest points as (squared distance, index), closest first.
    fn nearest(&self, x: &[f64], y: &[f64], qx: f64, qy: f64) -> [(f64, usize); K] {
        let mut best = [(f64::MAX, usize::MAX); K];
        let cx = (((qx - self.x0) / self.size).floor() as i64).clamp(0, self.nx as i64 - 1);
        let cy = (((qy - self.y0) / self.size).floor() as i64).clamp(0, self.ny as i64 - 1);
        for r in 0i64.. {
            if best[K - 1].0 < f64::MAX {
                // everything in this ring and beyond is at least this far away
                let d = (r - 1).max(0) as f64 * self.size;
                if d * d > best[K - 1].0 {
                    break;
                }
            }
            let mut any = false;
            for by in cy - r..=cy + r {
                if by < 0 || by >= self.ny as i64 {
                    continue;
                }
                for bx in cx - r..=cx + r {
                    if bx < 0 || bx >= self.nx as i64 {
                        continue;
                    }
                    if r > 0 && (bx - cx).abs() != r && (by - cy).abs() != r {
                        continue; // inside the ring, already seen
                    }
                    any = true;
                    let c = by as usize * self.nx + bx as usize;
                    for &i in &self.items[self.start[c] as usize..self.start[c + 1] as usize] {
                        let i = i as usize;
                        let (dx, dy) = (x[i] - qx, y[i] - qy);
                        let d2 = dx * dx + dy * dy;
                        if d2 < best[K - 1].0 || (d2 == best[K - 1].0 && i < best[K - 1].1) {
                            best[K - 1] = (d2, i);
                            let mut j = K - 1;
                            while j > 0 && best[j] < best[j - 1] {
                                best.swap(j, j - 1);
                                j -= 1;
                            }
                        }
                    }
                }
            }
            if !any && r > self.nx as i64 + self.ny as i64 {
                break;
            }
        }
        best
    }
}
