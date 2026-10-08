//! Node heights for the ascents of the routing graph (B edge word [2]).
//!
//! Two sources: heights per node, reconstructed from the routing graph of an
//! original chart, or an elevation grid from the Copernicus DEM
//! (`tools/dem_heights.py`).  Both arrive here as the flat binary that
//! `tools/heights_export.py` writes from their pickle:
//!
//! ```text
//! "TEASIHT1" kind:u32  kind 0 = nodes, 1 = grid
//!   nodes: n:u64, then n f64 each of X, Y (360/2^28 units) and h (cm)
//!   grid:  rows:u64 cols:u64 lon0:f64 lat0:f64 step:f64, then rows*cols f32 (m)
//! ```

use anyhow::{bail, ensure, Result};
use rayon::prelude::*;

use crate::grid::Grid;
use crate::pbf::SCALE;

/// Number of known nodes the height of a point is interpolated from.
const K: usize = 4;

pub enum Heights {
    /// Known heights at scattered points; inverse-distance interpolation.
    Nodes { h: Vec<f64>, grid: Grid },
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
                let pts: Vec<(f64, f64)> = x.into_iter().zip(y).collect();
                Ok(Heights::Nodes { h, grid: Grid::new(&pts) })
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
            Heights::Nodes { h, .. } => format!("{} known node heights", h.len()),
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
            Heights::Nodes { h, grid } => px
                .par_iter()
                .zip(py)
                .map(|(&qx, &qy)| {
                    // 1 / max(dist, 1)**2, then a sequential sum of four terms
                    // -- numpy sums the last axis of an (n, 4) array that way
                    let mut num = 0.0;
                    let mut den = 0.0;
                    for (j, (d, i)) in grid.knn((qx, qy), K, f64::INFINITY).into_iter().enumerate() {
                        let m = d.max(1.0);
                        let w = 1.0 / (m * m);
                        if j == 0 {
                            num = w * h[i as usize];
                            den = w;
                        } else {
                            num += w * h[i as usize];
                            den += w;
                        }
                    }
                    num / den
                })
                .collect(),
        }
    }
}
