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
//! Output is the tiled grid of `heights.rs`, written out as the file both
//! compilers read back.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use rayon::prelude::*;

use crate::heights::{Heights, NONE};
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

/// `gaussian` over a tiled grid, in place, one row of tiles at a time: the
/// same sums in the same order, so the same floats, without a second copy of
/// the grid.  Tiles that are not stored count as 0, as they would in the full
/// grid, and stay 0 -- whoever wants the filter to spill into a tile has to
/// store it.  The grid's own edges reflect like scipy's.
pub fn smooth(h: &mut Heights, sigma: f64) {
    let (lw, w) = kernel(sigma);
    let t = h.tile;
    assert!(lw <= t, "kernel wider than a tile");
    let (trows, tcols) = (h.rows / t, h.cols / t);
    let (rows, cols) = (h.rows as i64, h.cols as i64);
    let p = t + 2 * lw;
    // the bottom lw rows of each tile of the row above, as they were before
    let mut above: Vec<Option<Vec<f32>>> = vec![None; tcols];
    for tr in 0..trows {
        let row: Vec<(usize, usize)> = (0..tcols)
            .filter_map(|tc| match h.slots[tr * tcols + tc] {
                NONE => None,
                s => Some((tc, s as usize)),
            })
            .collect();
        let out: Vec<(usize, Vec<f32>)> = {
            let g = &*h;
            let above = &above;
            let orig = |r: usize, c: usize| -> f32 {
                if tr > 0 && r / t == tr - 1 {
                    match &above[c / t] {
                        Some(a) => a[(r % t + lw - t) * t + c % t],
                        None => 0.0,
                    }
                } else {
                    g.get(r, c)
                }
            };
            row.par_iter()
                .map(|&(tc, s)| {
                    let (r0, c0) = ((tr * t) as i64, (tc * t) as i64);
                    let gc: Vec<usize> =
                        (0..p).map(|j| reflect(c0 + j as i64 - lw as i64, cols)).collect();
                    let mut src = vec![0f32; p * p];
                    for (i, line) in src.chunks_mut(p).enumerate() {
                        let r = reflect(r0 + i as i64 - lw as i64, rows);
                        for (v, &c) in line.iter_mut().zip(&gc) {
                            *v = orig(r, c);
                        }
                    }
                    // axis 0 over the padded columns, then axis 1
                    let mut a = vec![0f32; t * p];
                    for (i, out) in a.chunks_mut(p).enumerate() {
                        for (j, o) in out.iter_mut().enumerate() {
                            let mut s = 0f64;
                            for (k, &wk) in w.iter().enumerate().rev() {
                                s += wk * src[(i + k) * p + j] as f64;
                            }
                            *o = s as f32;
                        }
                    }
                    let mut b = vec![0f32; t * t];
                    for (out, a) in b.chunks_mut(t).zip(a.chunks(p)) {
                        for (j, o) in out.iter_mut().enumerate() {
                            let mut s = 0f64;
                            for (k, &wk) in w.iter().enumerate().rev() {
                                s += wk * a[j + k] as f64;
                            }
                            *o = s as f32;
                        }
                    }
                    (s, b)
                })
                .collect()
        };
        for (tc, a) in above.iter_mut().enumerate() {
            *a = match h.slots[tr * tcols + tc] {
                NONE => None,
                s => {
                    let o = s as usize * t * t;
                    Some(h.data[o + (t - lw) * t..o + t * t].to_vec())
                }
            };
        }
        for (s, b) in out {
            h.data[s * t * t..(s + 1) * t * t].copy_from_slice(&b);
        }
    }
}

// --------------------------------------------------------------------------
// building the grid
// --------------------------------------------------------------------------

/// The 1x1 degree tiles of the box `lat0..lat1`, `lon0..lon0 + tcols` (row 0
/// the northernmost) that the rings touch, plus two tiles all around -- more
/// than the 1.4 degrees of a terrain region, so every region that touches the
/// area has its heights.  An edge
/// marks the tiles it runs through, a scanline through the middle of each row
/// of tiles the ones inside.
fn touched(rings: &[Ring], lat0: i64, lat1: i64, lon0: i64, tcols: usize) -> Vec<bool> {
    let trows = (lat1 - lat0) as usize;
    let deg = |&(x, y): &(f64, f64)| (x / 2f64.powi(28) * 360.0 - 180.0, 90.0 - y / 2f64.powi(28) * 360.0);
    let mut hit = vec![false; trows * tcols];
    let mut mark = |lon: f64, lat: f64| {
        let r = (lat1 as f64 - lat).floor().clamp(0.0, trows as f64 - 1.0) as usize;
        let c = (lon - lon0 as f64).floor().clamp(0.0, tcols as f64 - 1.0) as usize;
        hit[r * tcols + c] = true;
    };
    let mut edges: Vec<((f64, f64), (f64, f64))> = Vec::new();
    for ring in rings {
        let pts: Vec<(f64, f64)> = ring.pts.iter().map(deg).collect();
        for (i, &a) in pts.iter().enumerate() {
            let b = pts[(i + 1) % pts.len()];
            let n = ((a.0 - b.0).abs().max((a.1 - b.1).abs()) / 0.1).ceil() as usize + 1;
            for k in 0..=n {
                let f = k as f64 / n as f64;
                mark(a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f);
            }
            edges.push((a, b));
        }
    }
    for r in 0..trows {
        let lat = lat1 as f64 - r as f64 - 0.5;
        let mut xs: Vec<f64> = edges
            .iter()
            .filter(|(a, b)| (a.1 > lat) != (b.1 > lat))
            .map(|(a, b)| a.0 + (lat - a.1) / (b.1 - a.1) * (b.0 - a.0))
            .collect();
        xs.sort_by(f64::total_cmp);
        for pair in xs.chunks_exact(2) {
            for c in 0..tcols {
                let lon = lon0 as f64 + c as f64 + 0.5;
                if lon >= pair[0] && lon <= pair[1] {
                    mark(lon, lat);
                }
            }
        }
    }
    dilate(&dilate(&hit, trows, tcols), trows, tcols)
}

/// Each tile and its eight neighbours.
fn dilate(m: &[bool], trows: usize, tcols: usize) -> Vec<bool> {
    (0..trows * tcols)
        .map(|i| {
            let (r, c) = ((i / tcols) as i64, (i % tcols) as i64);
            (r - 1..=r + 1).any(|r| {
                (c - 1..=c + 1).any(|c| {
                    r >= 0
                        && c >= 0
                        && (r as usize) < trows
                        && (c as usize) < tcols
                        && m[r as usize * tcols + c as usize]
                })
            })
        })
        .collect()
}

/// Download the tiles covering `rings` and build the smoothed grid.  `sigma`
/// of 0 leaves the DEM unsmoothed.
///
/// The grid spans the rings' bounding box in whole degrees, but only the
/// tiles within two degrees of the rings are fetched, and of those only the ones
/// the bucket has (land) are stored -- with smoothing also their neighbours,
/// which the filter spills a few points into.
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
    let (trows, tcols) = ((lat1 - lat0) as usize, (lon1 - lon0) as usize);
    let want = touched(rings, lat0, lat1, lon0, tcols);
    log(&format!(
        "{} to {} N, {} to {} E: {} tiles near the area of {} in the box, grid {}x{}",
        lat0,
        lat1,
        lon0,
        lon1,
        want.iter().filter(|&&w| w).count(),
        trows * tcols,
        trows * N,
        tcols * N
    ));
    std::fs::create_dir_all(tdir).with_context(|| format!("create {:?}", tdir))?;

    let mut found: Vec<Option<PathBuf>> = vec![None; trows * tcols];
    let mut n = 0;
    for la in lat0..lat1 {
        let r = (lat1 - la - 1) as usize;
        if !(0..tcols).any(|c| want[r * tcols + c]) {
            continue;
        }
        for lo in lon0..lon1 {
            let i = r * tcols + (lo - lon0) as usize;
            if want[i] {
                found[i] = fetch(la, lo, tdir)?;
                n += usize::from(found[i].is_some());
            }
        }
        log(&format!("  {} N: {} tiles", la, n));
    }
    ensure!(n > 0, "not one tile of the area exists in the bucket");

    let land: Vec<bool> = found.iter().map(Option::is_some).collect();
    let keep = if sigma > 0.0 { dilate(&land, trows, tcols) } else { land };
    let mut slots = vec![NONE; trows * tcols];
    let mut paths: Vec<Option<&PathBuf>> = Vec::new();
    for i in 0..trows * tcols {
        if keep[i] {
            slots[i] = paths.len() as u32;
            paths.push(found[i].as_ref());
        }
    }
    let mut data = vec![0f32; paths.len() * N * N];
    data.par_chunks_mut(N * N).zip(&paths).try_for_each(|(out, p)| -> Result<()> {
        let Some(path) = p else { return Ok(()) };
        let (th, tw, a) = read_tiff(path)?;
        for (i, line) in out.chunks_mut(N).enumerate() {
            let si = (i * th) / N;
            for (j, o) in line.iter_mut().enumerate() {
                let v = a[si * tw + (j * tw) / N];
                // numpy.maximum(a, 0), which keeps a NaN
                *o = if v.is_nan() || v > 0.0 { v } else { 0.0 };
            }
        }
        Ok(())
    })?;
    let mut h = Heights {
        rows: trows * N,
        cols: tcols * N,
        lon0: lon0 as f64,
        lat0: lat1 as f64,
        step: 1.0 / N as f64,
        tile: N,
        slots,
        data,
    };
    if sigma > 0.0 {
        smooth(&mut h, sigma);
    }
    Ok(h)
}
