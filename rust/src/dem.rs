//! The elevation grid: Copernicus DEM GLO-90 tiles into one smoothed grid.
//!
//! The ascents of the routing graph (osm, B edge word [2]) and the elevation
//! tiles of the terrain layer both read heights from here.  The 1x1 degree
//! tiles covering a boundary polygon are downloaded from the public AWS bucket
//! `copernicus-dem-90m`, decimated onto one grid of 3" (1200 points per degree
//! in both directions) and smoothed with a Gaussian filter of `SIGMA` grid
//! points: the DEM is a *surface* model, so trees and houses would otherwise
//! add up to spurious ascents along the roads.
//!
//! The tiles are GeoTIFFs: float32, one DEFLATE-compressed tile per file,
//! floating-point predictor (TIFF Technical Note 3).  Their width shrinks with
//! latitude (1200 columns below 50 N, 800 up to 60 N, 600 above), which is what
//! the decimation to 1200 columns is for.
//!
//! Output is the grid of `heights.rs`, written out as the flat file both
//! compilers read back.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use rayon::prelude::*;

use crate::heights::Heights;
use crate::poly::Ring;

/// Grid points per degree (3").
pub const N: usize = 1200;
/// Default width of the Gaussian, in grid points.
pub const SIGMA: f64 = 1.0;
/// Where scipy's `gaussian_filter` cuts the kernel off.
const TRUNCATE: f64 = 4.0;

const BUCKET: &str = "https://copernicus-dem-90m.s3.amazonaws.com";

// --------------------------------------------------------------------------
// downloading
// --------------------------------------------------------------------------

/// `N49`, `E000` -- the two halves of a tile's name.
fn tile_name(lat: i64, lon: i64) -> (String, String) {
    let a = if lat >= 0 { format!("N{:02}", lat) } else { format!("S{:02}", -lat) };
    let b = if lon >= 0 { format!("E{:03}", lon) } else { format!("W{:03}", -lon) };
    (a, b)
}

/// The tile with that south-west corner, `None` if the bucket has none (open
/// sea).  Downloads it into `tdir` once; a missing tile is remembered as an
/// empty `.none` file so that a second run does not ask again.
fn fetch(lat: i64, lon: i64, tdir: &Path) -> Result<Option<PathBuf>> {
    let (a, b) = tile_name(lat, lon);
    let path = tdir.join(format!("{}_{}.tif", a, b));
    if path.exists() {
        return Ok(Some(path));
    }
    let none = tdir.join(format!("{}_{}.tif.none", a, b));
    if none.exists() {
        return Ok(None);
    }
    let name = format!("Copernicus_DSM_COG_30_{}_00_{}_00_DEM", a, b);
    let url = format!("{}/{}/{}.tif", BUCKET, name, name);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .build()
        .into();
    let mut last = String::new();
    for attempt in 0..5 {
        match agent.get(&url).call() {
            Ok(mut r) => {
                let body = r.body_mut().with_config().limit(64 << 20).read_to_vec()?;
                std::fs::write(&path, &body).with_context(|| format!("write {:?}", path))?;
                return Ok(Some(path));
            }
            // the bucket answers 403 for keys that do not exist
            Err(ureq::Error::StatusCode(403 | 404)) => {
                std::fs::write(&none, [])?;
                return Ok(None);
            }
            Err(e) => {
                last = e.to_string();
                if attempt < 4 {
                    std::thread::sleep(Duration::from_secs(2 + 3 * attempt));
                }
            }
        }
    }
    bail!("{}: {}", url, last)
}

// --------------------------------------------------------------------------
// the GeoTIFF the bucket serves
// --------------------------------------------------------------------------

fn u16_at(d: &[u8], o: usize) -> Result<u16> {
    ensure!(o + 2 <= d.len(), "TIFF truncated");
    Ok(u16::from_le_bytes(d[o..o + 2].try_into().unwrap()))
}

fn u32_at(d: &[u8], o: usize) -> Result<u32> {
    ensure!(o + 4 <= d.len(), "TIFF truncated");
    Ok(u32::from_le_bytes(d[o..o + 4].try_into().unwrap()))
}

/// One IFD entry's values, as far as this reader needs them (SHORT and LONG).
fn entry(d: &[u8], pos: usize) -> Result<(u16, Vec<u32>)> {
    let tag = u16_at(d, pos)?;
    let typ = u16_at(d, pos + 2)?;
    let count = u32_at(d, pos + 4)? as usize;
    let width = match typ {
        3 => 2, // SHORT
        4 => 4, // LONG
        _ => 0,
    };
    if width == 0 {
        return Ok((tag, Vec::new())); // a type this reader does not need
    }
    let mut v = Vec::with_capacity(count);
    // up to 4 bytes live in the entry itself, more sit at the offset
    let base = if count * width <= 4 { pos + 8 } else { u32_at(d, pos + 8)? as usize };
    for i in 0..count {
        v.push(match width {
            2 => u16_at(d, base + 2 * i)? as u32,
            _ => u32_at(d, base + 4 * i)?,
        });
    }
    Ok((tag, v))
}

/// Undo the floating-point predictor of one tile row (TIFF Technical Note 3,
/// `fpAcc` in libtiff): accumulate the differences byte by byte -- with a
/// stride of one sample, not one value -- then de-interleave the byte planes.
/// On a little-endian file the most significant plane comes first.
fn unpredict(row: &mut [u8], tmp: &mut Vec<u8>, bytes: usize, stride: usize) {
    for i in stride..row.len() {
        row[i] = row[i].wrapping_add(row[i - stride]);
    }
    tmp.clear();
    tmp.extend_from_slice(row);
    let n = row.len() / bytes;
    for i in 0..n {
        for b in 0..bytes {
            row[bytes * i + b] = tmp[(bytes - b - 1) * n + i];
        }
    }
}

/// Read one Copernicus tile: `(rows, cols, heights in m)`, rows north to south.
pub fn read_tiff(path: &Path) -> Result<(usize, usize, Vec<f32>)> {
    let d = std::fs::read(path).with_context(|| format!("open {:?}", path))?;
    decode_tiff(&d, &format!("{:?}", path))
}

/// The same from memory; `what` only names the source in error messages.
pub fn decode_tiff(d: &[u8], what: &str) -> Result<(usize, usize, Vec<f32>)> {
    let path = what;
    ensure!(d.len() > 8 && &d[..2] == b"II" && u16_at(d, 2)? == 42, "{}: not a TIFF", path);
    let ifd = u32_at(d, 4)? as usize;
    let n = u16_at(d, ifd)? as usize;
    let (mut w, mut h, mut tw, mut tl) = (0usize, 0usize, 0usize, 0usize);
    let (mut bits, mut comp, mut pred, mut spp, mut fmt, mut planar) = (0, 0, 1, 1, 1, 1);
    let (mut offs, mut counts) = (Vec::new(), Vec::new());
    for i in 0..n {
        let (tag, v) = entry(d, ifd + 2 + 12 * i)?;
        let first = v.first().copied().unwrap_or(0) as usize;
        match tag {
            256 => w = first,
            257 => h = first,
            258 => bits = first,
            259 => comp = first,
            277 => spp = first,
            284 => planar = first,
            317 => pred = first,
            322 => tw = first,
            323 => tl = first,
            324 => offs = v,
            325 => counts = v,
            339 => fmt = first,
            _ => {}
        }
    }
    ensure!(bits == 32 && fmt == 3, "{}: not float32 ({} bit, format {})", path, bits, fmt);
    ensure!(comp == 8 || comp == 32946, "{}: compression {} is not DEFLATE", path, comp);
    ensure!(pred == 3, "{}: predictor {} is not the floating point one", path, pred);
    ensure!(spp == 1 && planar == 1, "{}: {} samples per pixel", path, spp);
    ensure!(w > 0 && h > 0 && tw > 0 && tl > 0, "{}: not tiled", path);
    ensure!(offs.len() == counts.len() && !offs.is_empty(), "{}: no tiles", path);

    let across = w.div_ceil(tw);
    ensure!(offs.len() == across * h.div_ceil(tl), "{}: {} tiles", path, offs.len());
    let mut z = vec![0f32; w * h];
    let mut tmp = Vec::new();
    for (k, (&off, &cnt)) in offs.iter().zip(counts.iter()).enumerate() {
        let (off, cnt) = (off as usize, cnt as usize);
        ensure!(off + cnt <= d.len(), "{}: tile {} outside the file", path, k);
        let mut raw = Vec::with_capacity(tw * tl * 4);
        flate2::read::ZlibDecoder::new(&d[off..off + cnt])
            .read_to_end(&mut raw)
            .with_context(|| format!("{}: tile {}", path, k))?;
        ensure!(raw.len() == tw * tl * 4, "{}: tile {} is {} B", path, k, raw.len());
        let (x0, y0) = ((k % across) * tw, (k / across) * tl);
        for r in 0..tl.min(h - y0) {
            let row = &mut raw[r * tw * 4..(r + 1) * tw * 4];
            unpredict(row, &mut tmp, 4, spp);
            for c in 0..tw.min(w - x0) {
                let b = &row[4 * c..4 * c + 4];
                z[(y0 + r) * w + x0 + c] = f32::from_le_bytes(b.try_into().unwrap());
            }
        }
    }
    Ok((h, w, z))
}

// --------------------------------------------------------------------------
// the Gaussian filter, as scipy.ndimage does it
// --------------------------------------------------------------------------

/// numpy's pairwise summation, so that the kernel is normalised by the same
/// value `numpy.sum` produces (`PW_BLOCKSIZE` is 128).
fn npsum(a: &[f64]) -> f64 {
    let n = a.len();
    if n < 8 {
        return a.iter().fold(0.0, |s, &v| s + v);
    }
    if n <= 128 {
        let mut r = [a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]];
        let mut i = 8;
        while i < n - n % 8 {
            for (j, acc) in r.iter_mut().enumerate() {
                *acc += a[i + j];
            }
            i += 8;
        }
        let mut res = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
        while i < n {
            res += a[i];
            i += 1;
        }
        return res;
    }
    let mut half = n / 2;
    half -= half % 8;
    npsum(&a[..half]) + npsum(&a[half..])
}

/// `scipy.ndimage._gaussian_kernel1d(sigma, 0, radius)` -- the half width and
/// the normalised weights.
pub fn kernel(sigma: f64) -> (usize, Vec<f64>) {
    let lw = (TRUNCATE * sigma + 0.5) as usize;
    let c = -0.5 / (sigma * sigma);
    let mut w: Vec<f64> = (0..2 * lw + 1)
        .map(|i| {
            let x = i as i64 - lw as i64;
            (c * (x * x) as f64).exp()
        })
        .collect();
    let s = npsum(&w);
    for v in &mut w {
        *v /= s;
    }
    (lw, w)
}

/// scipy's `reflect` boundary: `(d c b a | a b c d | d c b a)`.
fn reflect(mut i: i64, n: i64) -> usize {
    loop {
        if i < 0 {
            i = -i - 1;
        } else if i >= n {
            i = 2 * n - i - 1;
        } else {
            return i as usize;
        }
    }
}

/// `scipy.ndimage.gaussian_filter(z, sigma)` over a float32 grid: one
/// `correlate1d` per axis, axis 0 first, each accumulating in f64 and storing
/// f32.  The taps are added from the last to the first, which is the order
/// `NI_Correlate1D` ends up with -- it reverses the weights and then walks the
/// window.  With a symmetric kernel that changes nothing but the rounding, and
/// the rounding is what has to match: summing the other way round moves one
/// value in 337 million by an ULP (and is in fact the more accurate of the
/// two, `math.fsum` agrees with it).
pub fn gaussian(z: &[f32], rows: usize, cols: usize, sigma: f64) -> Vec<f32> {
    let (lw, w) = kernel(sigma);
    // axis 0: down the columns
    let mut a = vec![0f32; rows * cols];
    a.par_chunks_mut(cols).enumerate().for_each(|(i, out)| {
        let src: Vec<&[f32]> = (0..w.len())
            .map(|k| {
                let r = reflect(i as i64 + k as i64 - lw as i64, rows as i64);
                &z[r * cols..(r + 1) * cols]
            })
            .collect();
        for (j, o) in out.iter_mut().enumerate() {
            let mut s = 0f64;
            for (k, &wk) in w.iter().enumerate().rev() {
                s += wk * src[k][j] as f64;
            }
            *o = s as f32;
        }
    });
    // axis 1: along the rows
    let mut b = vec![0f32; rows * cols];
    b.par_chunks_mut(cols)
        .zip(a.par_chunks(cols))
        .for_each(|(out, src)| {
            for (j, o) in out.iter_mut().enumerate() {
                let mut s = 0f64;
                for (k, &wk) in w.iter().enumerate().rev() {
                    s += wk * src[reflect(j as i64 + k as i64 - lw as i64, cols as i64)] as f64;
                }
                *o = s as f32;
            }
        });
    b
}

// --------------------------------------------------------------------------
// building the grid
// --------------------------------------------------------------------------

/// Download the tiles covering `rings` and build the smoothed grid.  `sigma`
/// of 0 leaves the DEM unsmoothed.
pub fn build(rings: &[Ring], tdir: &Path, sigma: f64, log: &dyn Fn(&str)) -> Result<Heights> {
    ensure!(!rings.is_empty(), "the boundary polygon is empty");
    let (mut west, mut east) = (f64::MAX, f64::MIN);
    let (mut south, mut north) = (f64::MAX, f64::MIN);
    for r in rings {
        for &(x, y) in &r.pts {
            let (lon, lat) = (x / 2f64.powi(28) * 360.0 - 180.0, 90.0 - y / 2f64.powi(28) * 360.0);
            west = west.min(lon);
            east = east.max(lon);
            south = south.min(lat);
            north = north.max(lat);
        }
    }
    let (lon0, lon1) = (west.floor() as i64, east.ceil() as i64);
    let (lat0, lat1) = (south.floor() as i64, north.ceil() as i64);
    let (rows, cols) = (((lat1 - lat0) as usize) * N, ((lon1 - lon0) as usize) * N);
    log(&format!(
        "{} to {} N, {} to {} E: {} tiles, grid {}x{}",
        lat0,
        lat1,
        lon0,
        lon1,
        (lat1 - lat0) * (lon1 - lon0),
        rows,
        cols
    ));
    std::fs::create_dir_all(tdir).with_context(|| format!("create {:?}", tdir))?;

    let mut z = vec![0f32; rows * cols];
    let mut n = 0;
    for la in lat0..lat1 {
        for lo in lon0..lon1 {
            let Some(path) = fetch(la, lo, tdir)? else { continue };
            let (th, tw, a) = read_tiff(&path)?;
            let r = ((lat1 - la - 1) as usize) * N;
            let c = ((lo - lon0) as usize) * N;
            for i in 0..N {
                let si = (i * th) / N;
                for j in 0..N {
                    let v = a[si * tw + (j * tw) / N];
                    // numpy.maximum(a, 0), which keeps a NaN
                    z[(r + i) * cols + c + j] = if v.is_nan() || v > 0.0 { v } else { 0.0 };
                }
            }
            n += 1;
        }
        log(&format!("  {} N: {} tiles", la, n));
    }
    ensure!(n > 0, "not one tile of the area exists in the bucket");
    if sigma > 0.0 {
        z = gaussian(&z, rows, cols, sigma);
    }
    Ok(Heights { rows, cols, lon0: lon0 as f64, lat0: lat1 as f64, step: 1.0 / N as f64, z })
}
