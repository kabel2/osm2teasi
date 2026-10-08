//! The elevation grid behind the ascents of the routing graph (B edge word [2])
//! and the shaded relief of the terrain layer.
//!
//! `dem.rs` (`teasi dem`) builds it from the Copernicus DEM; it travels between
//! the two as this flat little-endian file:
//!
//! ```text
//! "TEASIHT1" kind:u32 = 2
//!   rows:u64 cols:u64 lon0:f64 lat0:f64 step:f64 tile:u64
//!   one u32 per tile (row by row): its number in what follows, or 0xFFFFFFFF
//!   tile*tile f32 (m) per stored tile, row by row
//! ```
//!
//! The grid is cut into square tiles and only the tiles near land are stored;
//! every other point is 0.  A country's bounding box is mostly sea, and around
//! the antimeridian it is the whole width of the earth.

use std::io::{BufReader, BufWriter, Read, Write};

use anyhow::{ensure, Context, Result};
use rayon::prelude::*;

use crate::pbf::SCALE;

/// Marks a tile that is not stored.
pub const NONE: u32 = u32::MAX;

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
    /// Grid points per tile side; `rows` and `cols` are multiples of it.
    pub tile: usize,
    /// Per tile, row by row: where it is in `data`, or [`NONE`].
    pub slots: Vec<u32>,
    /// The stored tiles, `tile * tile` points each.
    pub data: Vec<f32>,
}

fn u64_of(r: &mut impl Read) -> Result<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b).context("heights file truncated")?;
    Ok(u64::from_le_bytes(b))
}

impl Heights {
    /// Tile columns.
    pub fn tcols(&self) -> usize {
        self.cols / self.tile
    }

    /// The height at grid point `r, c`.
    #[inline]
    pub fn get(&self, r: usize, c: usize) -> f32 {
        let t = self.tile;
        match self.slots[(r / t) * self.tcols() + c / t] {
            NONE => 0.0,
            s => self.data[s as usize * t * t + (r % t) * t + c % t],
        }
    }

    pub fn load(path: &str) -> Result<Heights> {
        let f = std::fs::File::open(path).with_context(|| format!("open {}", path))?;
        let mut f = BufReader::with_capacity(1 << 20, f);
        let mut head = [0u8; 12];
        f.read_exact(&mut head).with_context(|| format!("{}: not a heights file", path))?;
        ensure!(&head[..8] == b"TEASIHT1", "{}: not a heights file", path);
        let kind = u32::from_le_bytes(head[8..12].try_into().unwrap());
        ensure!(kind == 2, "{}: heights kind {}, rebuild it with this version", path, kind);
        let (rows, cols) = (u64_of(&mut f)? as usize, u64_of(&mut f)? as usize);
        let lon0 = f64::from_bits(u64_of(&mut f)?);
        let lat0 = f64::from_bits(u64_of(&mut f)?);
        let step = f64::from_bits(u64_of(&mut f)?);
        let tile = u64_of(&mut f)? as usize;
        ensure!(tile > 0 && rows % tile == 0 && cols % tile == 0, "{}: bad tiling", path);
        let mut b = vec![0u8; 4 * (rows / tile) * (cols / tile)];
        f.read_exact(&mut b).context("heights file truncated")?;
        let slots: Vec<u32> =
            b.chunks_exact(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
        let n = slots.iter().filter(|&&s| s != NONE).count();
        ensure!(slots.iter().all(|&s| s == NONE || (s as usize) < n), "{}: bad tile index", path);
        let mut data = vec![0f32; n * tile * tile];
        let mut b = vec![0u8; 4 << 20];
        for chunk in data.chunks_mut(1 << 20) {
            let b = &mut b[..4 * chunk.len()];
            f.read_exact(b).context("heights file truncated")?;
            for (v, b) in chunk.iter_mut().zip(b.chunks_exact(4)) {
                *v = f32::from_le_bytes(b.try_into().unwrap());
            }
        }
        Ok(Heights { rows, cols, lon0, lat0, step, tile, slots, data })
    }

    /// Write the file `teasi osm --heights=` and `teasi terrain` read back.
    pub fn write(&self, path: &str) -> Result<()> {
        let f = std::fs::File::create(path).with_context(|| format!("write {}", path))?;
        let mut out = BufWriter::with_capacity(1 << 20, f);
        out.write_all(b"TEASIHT1")?;
        out.write_all(&2u32.to_le_bytes())?; // kind 2 = tiled grid
        for v in [self.rows as u64, self.cols as u64] {
            out.write_all(&v.to_le_bytes())?;
        }
        for v in [self.lon0, self.lat0, self.step] {
            out.write_all(&v.to_le_bytes())?;
        }
        out.write_all(&(self.tile as u64).to_le_bytes())?;
        for s in &self.slots {
            out.write_all(&s.to_le_bytes())?;
        }
        for v in &self.data {
            out.write_all(&v.to_le_bytes())?;
        }
        out.flush().with_context(|| format!("write {}", path))?;
        Ok(())
    }

    pub fn describe(&self) -> String {
        let n = self.slots.iter().filter(|&&s| s != NONE).count();
        format!(
            "{}x{} grid, {:.5} deg, {} of {} tiles",
            self.rows,
            self.cols,
            self.step,
            n,
            self.slots.len()
        )
    }

    /// Whether any stored tile overlaps the box (degrees); outside it every
    /// height is 0.
    pub fn covers(&self, west: f64, south: f64, east: f64, north: f64) -> bool {
        let span = self.step * self.tile as f64;
        let tc = |lon: f64| ((lon - self.lon0) / span).floor();
        let tr = |lat: f64| ((self.lat0 - lat) / span).floor();
        let (c0, c1) = (tc(west).max(0.0), tc(east).min(self.tcols() as f64 - 1.0));
        let (r0, r1) = (tr(north).max(0.0), tr(south).min((self.rows / self.tile) as f64 - 1.0));
        if c0 > c1 || r0 > r1 {
            return false;
        }
        (r0 as usize..=r1 as usize)
            .any(|r| (c0 as usize..=c1 as usize).any(|c| self.slots[r * self.tcols() + c] != NONE))
    }

    /// Heights in cm at the given points (360/2^28 units), bilinear and clipped
    /// to the grid.
    pub fn at(&self, px: &[f64], py: &[f64]) -> Vec<f64> {
        px.par_iter().zip(py).map(|(&x, &y)| self.at1(x, y)).collect()
    }

    /// One point of [`Heights::at`].
    pub fn at1(&self, x: f64, y: f64) -> f64 {
        let Heights { rows, cols, lon0, lat0, step, .. } = self;
        let c = ((x / SCALE - 180.0) - lon0) / step;
        let r = (lat0 - (90.0 - y / SCALE)) / step;
        let c = c.clamp(0.0, *cols as f64 - 1.001);
        let r = r.clamp(0.0, *rows as f64 - 1.001);
        let (c0, r0) = (c as usize, r as usize);
        let (fc, fr) = (c - c0 as f64, r - r0 as f64);
        let at = |r: usize, c: usize| f64::from(self.get(r, c));
        let v = (at(r0, c0) * (1.0 - fc) + at(r0, c0 + 1) * fc) * (1.0 - fr)
            + (at(r0 + 1, c0) * (1.0 - fc) + at(r0 + 1, c0 + 1) * fc) * fr;
        100.0 * v
    }
}
