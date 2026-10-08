//! The elevation grid behind the ascents of the routing graph (B edge word [2])
//! and the shaded relief of the terrain layer.
//!
//! `dem.rs` (`teasi dem`) builds it from the Copernicus DEM; it travels between
//! the two as this flat little-endian file:
//!
//! ```text
//! "TEASIHT1" kind:u32 = 1
//!   rows:u64 cols:u64 lon0:f64 lat0:f64 step:f64, then rows*cols f32 (m)
//! ```

use anyhow::{ensure, Context, Result};
use rayon::prelude::*;

use crate::pbf::SCALE;

/// A regular grid of heights in m, rows running north to south.
pub struct Heights {
    pub rows: usize,
    pub cols: usize,
    /// Western edge in degrees.
    pub lon0: f64,
    /// **Northern** edge in degrees (the first row).
    pub lat0: f64,
    /// Degrees per grid point.
    pub step: f64,
    pub z: Vec<f32>,
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
}

impl Heights {
    pub fn load(path: &str) -> Result<Heights> {
        let d = std::fs::read(path)?;
        ensure!(d.len() > 12 && &d[..8] == b"TEASIHT1", "{}: not a heights file", path);
        let kind = u32::from_le_bytes(d[8..12].try_into().unwrap());
        ensure!(kind == 1, "{}: unknown heights kind {}", path, kind);
        let mut c = Cur(&d, 12);
        let (rows, cols) = (c.u64()? as usize, c.u64()? as usize);
        let (lon0, lat0, step) = (c.f64()?, c.f64()?, c.f64()?);
        ensure!(c.1 + 4 * rows * cols <= d.len(), "heights file truncated");
        let z = d[c.1..c.1 + 4 * rows * cols]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        Ok(Heights { rows, cols, lon0, lat0, step, z })
    }

    /// Write the file `teasi osm --heights=` and `teasi terrain` read back.
    pub fn write(&self, path: &str) -> Result<()> {
        let mut out = Vec::with_capacity(44 + 4 * self.z.len());
        out.extend_from_slice(b"TEASIHT1");
        out.extend_from_slice(&1u32.to_le_bytes()); // kind 1 = grid
        out.extend_from_slice(&(self.rows as u64).to_le_bytes());
        out.extend_from_slice(&(self.cols as u64).to_le_bytes());
        out.extend_from_slice(&self.lon0.to_le_bytes());
        out.extend_from_slice(&self.lat0.to_le_bytes());
        out.extend_from_slice(&self.step.to_le_bytes());
        for v in &self.z {
            out.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(path, &out).with_context(|| format!("write {}", path))?;
        Ok(())
    }

    pub fn describe(&self) -> String {
        format!("{}x{} grid, {:.5} deg", self.rows, self.cols, self.step)
    }

    /// Heights in cm at the given points (360/2^28 units), bilinear and clipped
    /// to the grid.
    pub fn at(&self, px: &[f64], py: &[f64]) -> Vec<f64> {
        px.par_iter().zip(py).map(|(&x, &y)| self.at1(x, y)).collect()
    }

    /// One point of [`Heights::at`].
    pub fn at1(&self, x: f64, y: f64) -> f64 {
        let Heights { rows, cols, lon0, lat0, step, z } = self;
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
    }
}
