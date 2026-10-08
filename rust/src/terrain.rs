//! The terrain layer (type 5, layer 0x20): the elevation model as JPEG 2000
//! tiles and the map images as JPEG tiles; format in docs/TERRAIN_FORMAT.md.
//!
//! One region covers 1.40625 degrees and holds 8x8 cells of 256x256 px.  A cell
//! gets an elevation tile if it contains land, and a map image if it touches the
//! boundary polygon or has land; the images also exist as a pyramid of 4x4, 2x2
//! and 1x1 cells.  The map images are a hillshade (light from the north west)
//! coloured by land cover: sea from the worldwide land polygons (land.rs),
//! everything else from the OSM areas (area.rs).
//!
//! The drawing and the two codecs live in raster.rs, which copies Pillow's
//! algorithms; what differs from the Python output is listed in rust/README.md.

use std::collections::{HashMap, HashSet};

use anyhow::{ensure, Result};
use rayon::prelude::*;

use crate::area::{Extract, Poly};
use crate::chart;
use crate::geos::{Geom, Prepared};
use crate::heights::Heights;
use crate::pc1;
use crate::poly::Ring;
use crate::raster::{self, Mask, Rgb};
use crate::writer::urandom;

/// degrees per region
pub const TILE: f64 = 1.40625;
/// degrees per cell
const CS: f64 = TILE / 8.0;
/// pixels per cell
const N: usize = 256;
/// pixels per region (8 cells)
const R: usize = 2048;
/// compression ratio of the elevation tiles
pub const RATE: f32 = 50.0;
/// values per 1/6 m, at most
const MAX_A0: i64 = 9999;
/// bytes before the first tile of a region
const HEAD: usize = 0x159C;
/// 360/2^25 degree units (the land and area coordinates) per pixel
const UNITS: f64 = 64.0;
/// quantisation tables of the originals = IJG quality 80
const JPEG_Q: u8 = 80;

/// `jP`, `ftyp`, `jp2h` (`ihdr` 256x256x1 16 bit, `colr`), `res ` and the
/// `jp2c` box header (length 0 = up to the end of the file), copied from
/// Denmark_terrain.v20210916.
const JP2_HEAD: &str = "\
    0000000c6a5020200d0a870a00000014667479706a703220000000006a703220000000476a7032\
    680000001669686472000001000000010000010f0700000000000f636f6c720100000000001100\
    00001a7265732000000012726573634890fffc4890fffc0404000000006a703263";
/// the EXIF APP1 of the original JPEGs (GDI+ style: 96 dpi, no tile data)
const EXIF: &str = "\
    45786966000049492a000800000003001a01050001000000320000001b010500010000003a0000\
    002801030001000000020053000000000000703839809698000070383980969800";

/// Hex text (with any spaces) to bytes.
fn unhex(s: &str) -> Vec<u8> {
    let d: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    d.chunks_exact(2)
        .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
        .collect()
}

// land cover classes, painted in this order; colours after the originals
const SEA: u8 = 0;
const LAND: u8 = 1;
const WATER: u8 = 9;
const COLOURS: [(f32, f32, f32); 10] = [
    (150.0, 180.0, 206.0),
    (226.0, 227.0, 203.0),
    (230.0, 226.0, 196.0),
    (232.0, 216.0, 198.0),
    (212.0, 219.0, 172.0),
    (184.0, 205.0, 148.0),
    (204.0, 218.0, 196.0),
    (224.0, 221.0, 214.0),
    (238.0, 231.0, 200.0),
    (150.0, 180.0, 206.0),
];

const FARM: u8 = 2;
const URBAN: u8 = 3;
const HEATH: u8 = 4;
const FOREST: u8 = 5;
const WET: u8 = 6;
const ROCK: u8 = 7;
const SAND: u8 = 8;

/// The land cover class of one tag, or `None`.
/// An area with several of these keys takes the first one in tag order, which is
/// why [`crate::area::ATags`] remembers the result instead of the tags.
pub fn cover_class(k: &str, v: &str) -> Option<u8> {
    Some(match (k, v) {
        ("landuse", "farmland" | "farmyard" | "orchard" | "vineyard" | "allotments") => FARM,
        (
            "landuse",
            "residential" | "commercial" | "industrial" | "retail" | "construction"
            | "railway" | "brownfield",
        ) => URBAN,
        ("landuse", "quarry") => ROCK,
        ("natural", "heath" | "scrub" | "grassland" | "fell" | "moor") => HEATH,
        ("landuse", "forest") | ("natural", "wood") => FOREST,
        ("natural", "wetland") => WET,
        ("natural", "bare_rock" | "scree" | "glacier") => ROCK,
        ("natural", "shingle" | "sand" | "beach") => SAND,
        ("natural", "water") | ("waterway", "riverbank") => WATER,
        ("landuse", "reservoir" | "basin") => WATER,
        _ => return None,
    })
}

/// Bilinear heights of the grid at `lat` (rows) x `lon` (columns), in metres,
/// zeroed outside the grid.
fn sample(g: &Heights, lat: &[f64], lon: &[f64]) -> Vec<f64> {
    let Heights { rows, cols, lon0, lat0, step, z } = g;
    let rs: Vec<(usize, f64)> = lat
        .iter()
        .map(|&la| {
            let r = ((lat0 - la) / step).clamp(0.0, *rows as f64 - 1.001);
            (r as usize, r - (r as usize) as f64)
        })
        .collect();
    let cs: Vec<(usize, f64)> = lon
        .iter()
        .map(|&lo| {
            let c = ((lo - lon0) / step).clamp(0.0, *cols as f64 - 1.001);
            (c as usize, c - (c as usize) as f64)
        })
        .collect();
    let mut out = vec![0.0f64; lat.len() * lon.len()];
    out.par_chunks_mut(lon.len()).enumerate().for_each(|(i, row)| {
        let (r0, fr) = rs[i];
        if lat[i] > *lat0 || lat[i] < lat0 - *rows as f64 * step {
            return; // outside the grid
        }
        for (j, v) in row.iter_mut().enumerate() {
            if lon[j] < *lon0 || lon[j] > lon0 + *cols as f64 * step {
                continue;
            }
            let (c0, fc) = cs[j];
            let a = f64::from(z[r0 * cols + c0]);
            let b = f64::from(z[r0 * cols + c0 + 1]);
            let d = f64::from(z[(r0 + 1) * cols + c0]);
            let e = f64::from(z[(r0 + 1) * cols + c0 + 1]);
            *v = (a * (1.0 - fc) + b * fc) * (1.0 - fr) + (d * (1.0 - fc) + e * fc) * fr;
        }
    });
    out
}

/// Hillshade (flat = 1) of the heights `h` (`side` x `side` with a 1 px margin),
/// light from the north west at 45 degrees; `side - 2` squared values back.
/// Public for the test against numpy's `np.gradient`.
pub fn shade(h: &[f64], side: usize, lat: f64) -> Vec<f64> {
    let dy = CS / N as f64 * 111320.0;
    let dx = dy * lat.to_radians().cos();
    let z = 1.5; // relief exaggeration
    let (az, alt) = (315f64.to_radians(), 45f64.to_radians());
    let lx = alt.cos() * az.sin();
    let ly = alt.cos() * az.cos();
    let lz = alt.sin();
    let w = side - 2;
    let mut out = vec![0.0; w * w];
    out.par_chunks_mut(w).enumerate().for_each(|(i, row)| {
        for (j, s) in row.iter_mut().enumerate() {
            let (i, j) = (i + 1, j + 1);
            let gy = (h[(i + 1) * side + j] * z - h[(i - 1) * side + j] * z) / (2.0 * dy);
            let gx = (h[i * side + j + 1] * z - h[i * side + j - 1] * z) / (2.0 * dx);
            let (nx, ny) = (-gx, gy);
            *s = (nx * lx + ny * ly + lz) / (nx * nx + ny * ny + 1.0).sqrt() / lz;
        }
    });
    out
}

/// One land polygon, flattened out of GEOS so that the regions can be drawn in
/// parallel: `Geom` holds a raw pointer and cannot cross threads.
pub struct Shape {
    outer: Vec<(f64, f64)>,
    holes: Vec<Vec<(f64, f64)>>,
    bounds: (f64, f64, f64, f64),
}

/// What one region needs for its images.
#[derive(Default)]
struct Reg {
    /// cells touching the boundary polygon
    cells: HashSet<u32>,
    /// indices into [`Prep::land`]
    land: Vec<usize>,
    /// cover class, area index, polygon index -- sorted by class before drawing
    areas: Vec<(u8, u32, u32)>,
}

/// The land polygons plus the per-region bins.  Port of `prepare()`.
pub struct Prep {
    land: Vec<Shape>,
    regs: HashMap<(i64, i64), Reg>,
}

fn prepare(
    rings: &[Ring],
    land: &[Geom],
    areas: &Extract,
    xs: &[i64],
    ys: &[i64],
    log: &dyn Fn(&str),
) -> Result<Prep> {
    let area = crate::osmarea::boundary_at(rings, 8.0)?;
    let prep = Prepared::new(&area)?;
    let s = (R as f64) * UNITS;
    let mut regs: HashMap<(i64, i64), Reg> = HashMap::new();
    for &x in xs {
        for &y in ys {
            let mut cells = HashSet::new();
            for k in 0..64u32 {
                let (c, r) = ((k / 8) as f64, (k % 8) as f64);
                let b = Geom::rect(
                    x as f64 * s + c * s / 8.0,
                    y as f64 * s + r * s / 8.0,
                    x as f64 * s + (c + 1.0) * s / 8.0,
                    y as f64 * s + (r + 1.0) * s / 8.0,
                )?;
                if prep.intersects(&b) {
                    cells.insert(k);
                }
            }
            regs.insert((x, y), Reg { cells, ..Reg::default() });
        }
    }

    let mut shapes = Vec::new();
    for g in land {
        for part in g.polygons()? {
            let outer = part.exterior();
            if outer.is_empty() {
                continue;
            }
            let (x0, y0, x1, y1) = part.bounds();
            shapes.push(Shape { outer, holes: part.interiors(), bounds: (x0, y0, x1, y1) });
        }
    }
    for (i, sh) in shapes.iter().enumerate() {
        let (x0, y0, x1, y1) = sh.bounds;
        for x in (x0 / s).floor() as i64..=(x1 / s).floor() as i64 {
            for y in (y0 / s).floor() as i64..=(y1 / s).floor() as i64 {
                if let Some(r) = regs.get_mut(&(x, y)) {
                    r.land.push(i);
                }
            }
        }
    }

    let mut n = 0usize;
    for (ai, a) in areas.areas.iter().enumerate() {
        let Some(c) = a.tags.cover else { continue };
        for (pi, p) in a.polys.iter().enumerate() {
            let (x0, y0, x1, y1) = ring_bounds(p);
            if x1 - x0 < UNITS && y1 - y0 < UNITS {
                continue; // smaller than a pixel
            }
            for x in (x0 / s).floor() as i64..=(x1 / s).floor() as i64 {
                for y in (y0 / s).floor() as i64..=(y1 / s).floor() as i64 {
                    if let Some(r) = regs.get_mut(&(x, y)) {
                        r.areas.push((c, ai as u32, pi as u32));
                        n += 1;
                    }
                }
            }
        }
    }
    for r in regs.values_mut() {
        r.areas.sort_by_key(|a| a.0); // stable, like Python's sorted()
    }
    log(&format!("{} land polygons, {} land-cover areas", shapes.len(), n));
    Ok(Prep { land: shapes, regs })
}

fn ring_bounds(p: &Poly) -> (f64, f64, f64, f64) {
    let mut b = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &(x, y) in &p.outer {
        b.0 = b.0.min(f64::from(x));
        b.1 = b.1.min(f64::from(y));
        b.2 = b.2.max(f64::from(x));
        b.3 = b.3.max(f64::from(y));
    }
    b
}

/// Land cover class per pixel of region `x, y`.  Port of `cover()`.
fn cover(x: i64, y: i64, p: &Prep, reg: &Reg, areas: &Extract) -> Mask {
    let mut im = Mask::new(R, R, SEA);
    let (x0, y0) = (x as f64 * R as f64 * UNITS, y as f64 * R as f64 * UNITS);
    let px = |pts: &[(f64, f64)]| -> Vec<(f64, f64)> {
        pts.iter().map(|&(a, b)| ((a - x0) / UNITS - 0.5, (b - y0) / UNITS - 0.5)).collect()
    };
    let pxi = |pts: &[(i32, i32)]| -> Vec<(f64, f64)> {
        pts.iter()
            .map(|&(a, b)| ((f64::from(a) - x0) / UNITS - 0.5, (f64::from(b) - y0) / UNITS - 0.5))
            .collect()
    };
    for &i in &reg.land {
        let sh = &p.land[i];
        im.polygon(&px(&sh.outer), LAND);
        for h in &sh.holes {
            im.polygon(&px(h), SEA);
        }
    }
    for &(c, ai, pi) in &reg.areas {
        let poly = &areas.areas[ai as usize].polys[pi as usize];
        if poly.outer.len() >= 3 {
            im.polygon(&pxi(&poly.outer), c);
        }
        for h in &poly.inners {
            if h.len() >= 3 {
                im.polygon(&pxi(h), LAND);
            }
        }
    }
    im
}

/// The 85 cells of a region as JPEG bytes.  Port of `images()`.
fn images(
    x: i64,
    y: i64,
    h: &[f64],
    p: &Prep,
    reg: &Reg,
    areas: &Extract,
) -> Result<Vec<Option<Vec<u8>>>> {
    let cls = cover(x, y, p, reg, areas);
    let sh = shade(h, R + 2, 90.0 - (y as f64 + 0.5) * TILE);
    let exif = unhex(EXIF);
    let mut img = Rgb::new(R, R);
    for i in 0..R {
        for j in 0..R {
            let c = cls.px[i * R + j];
            let s = if c == SEA || c == WATER {
                1.0 // water is not shaded
            } else {
                (1.0 + 0.7 * (sh[i * R + j] - 1.0)).clamp(0.4, 1.12)
            };
            let (r, g, b) = COLOURS[c as usize];
            img.set(
                j,
                i,
                (
                    (f64::from(r) * s).clamp(0.0, 255.0) as u8,
                    (f64::from(g) * s).clamp(0.0, 255.0) as u8,
                    (f64::from(b) * s).clamp(0.0, 255.0) as u8,
                ),
            );
        }
    }
    let mut have: Vec<bool> = (0..64)
        .map(|k: usize| {
            reg.cells.contains(&(k as u32))
                || (0..N).any(|r| {
                    (0..N).any(|c| cls.px[((k % 8) * N + r) * R + (k / 8) * N + c] != SEA)
                })
        })
        .collect();
    let mut out: Vec<Option<Vec<u8>>> = (0..85).map(|_| None).collect();
    let mut base = 0usize;
    for g in [8usize, 4, 2, 1] {
        let lvl = raster::resize(&img, g * N, g * N);
        let tiles: Vec<Option<Vec<u8>>> = (0..g * g)
            .into_par_iter()
            .map(|k| {
                if !have[k] {
                    return Ok(None);
                }
                let (c, r) = (k / g, k % g);
                let t = lvl.crop(c * N, r * N, N, N);
                raster::jpeg(&t, JPEG_Q, 96, &exif).map(Some)
            })
            .collect::<Result<Vec<_>>>()?;
        for (k, t) in tiles.into_iter().enumerate() {
            out[base + k] = t;
        }
        if g > 1 {
            let half = g / 2;
            have = (0..half * half)
                .map(|k| {
                    [(0, 0), (0, 1), (1, 0), (1, 1)].iter().any(|&(dc, dr)| {
                        have[(2 * (k / half) + dc) * g + 2 * (k % half) + dr]
                    })
                })
                .collect();
        }
        base += g * g;
    }
    Ok(out)
}

/// The elevation record of one cell: `a0`, `a1` and the JP2 file.
fn encode(h: &[f64], rate: f32) -> Result<(u16, u16, Vec<u8>)> {
    let u: Vec<f64> = h.iter().map(|z| 6.0 * (z + 1000.0)).collect();
    let lo = u.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = u.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let a1 = lo.floor() as i64;
    let span = (hi.ceil() as i64 - a1).max(1);
    let a0 = (65535 / span).clamp(1, MAX_A0);
    ensure!((0..=65535).contains(&a1), "a1 = {} does not fit a u16", a1);
    let v: Vec<u16> = u
        .iter()
        .map(|z| ((z - a1 as f64) * a0 as f64).round_ties_even().clamp(0.0, 65535.0) as u16)
        .collect();
    let mut jp2 = unhex(JP2_HEAD);
    jp2.extend_from_slice(&raster::jp2(&v, N as u32, rate)?);
    Ok((a0 as u16, a1 as u16, jp2))
}

/// One finished region.
struct Built {
    head: Vec<u8>,
    body: Vec<u8>,
    /// length of the longest JP2, for the file header at 0x6C
    maxlen: usize,
}

/// Build one region, or `None` if it has neither images nor heights.
/// Port of `region()`.
fn region(
    x: i64,
    y: i64,
    g: &Heights,
    rate: f32,
    p: Option<&Prep>,
    areas: Option<&Extract>,
) -> Result<Option<Built>> {
    let (lon_w, lat_n) = (x as f64 * TILE - 180.0, 90.0 - y as f64 * TILE);
    // heights at the pixel centres, with a margin of 1 px for the hillshade
    let lat: Vec<f64> =
        (-1..=R as i64).map(|i| lat_n - (i as f64 + 0.5) / R as f64 * TILE).collect();
    let lon: Vec<f64> =
        (-1..=R as i64).map(|i| lon_w + (i as f64 + 0.5) / R as f64 * TILE).collect();
    let hm = sample(g, &lat, &lon);
    let side = R + 2;

    let jpgs = match (p, areas) {
        (Some(p), Some(a)) => match p.regs.get(&(x, y)) {
            Some(reg) => images(x, y, &hm, p, reg, a)?,
            None => (0..85).map(|_| None).collect(),
        },
        _ => (0..85).map(|_| None).collect(),
    };

    let cells: Vec<(usize, Vec<f64>)> = (0..64usize)
        .filter_map(|k| {
            let (col, row) = (k / 8, k % 8);
            let mut c = Vec::with_capacity(N * N);
            for r in 0..N {
                let o = (row * N + r + 1) * side + col * N + 1;
                c.extend_from_slice(&hm[o..o + N]);
            }
            let max = c.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            if max <= 0.5 {
                None
            } else {
                Some((k, c))
            }
        })
        .collect();
    let recs: Vec<(usize, u16, u16, Vec<u8>)> = cells
        .into_par_iter()
        .map(|(k, c)| encode(&c, rate).map(|(a0, a1, jp2)| (k, a0, a1, jp2)))
        .collect::<Result<Vec<_>>>()?;
    if recs.is_empty() && jpgs.iter().all(|j| j.is_none()) {
        return Ok(None);
    }

    let mut body: Vec<u8> = Vec::new();
    let mut t1: Vec<(u32, u32)> = Vec::with_capacity(85);
    for j in &jpgs {
        match j {
            None => t1.push((0xFFFF_FFFD, 0)),
            Some(j) => {
                t1.push(((HEAD + body.len()) as u32, j.len() as u32));
                body.extend_from_slice(j);
            }
        }
    }
    let mut t2 = [0xFFFF_FFFFu32; 85];
    let mut maxlen = 0;
    for (i, (k, a0, a1, jp2)) in recs.iter().enumerate() {
        t2[*k] = (HEAD + body.len()) as u32;
        // the originals end each record with 2 stale bytes, mostly the next record's a0
        let nxt = if i + 1 < recs.len() { recs[i + 1].1 } else { *a0 };
        body.extend_from_slice(&a0.to_le_bytes());
        body.extend_from_slice(&a1.to_le_bytes());
        body.extend_from_slice(&(jp2.len() as u32).to_le_bytes());
        body.extend_from_slice(jp2);
        body.extend_from_slice(&nxt.to_le_bytes());
        maxlen = maxlen.max(jp2.len());
    }
    let mut head = vec![0xFFu8; 0x1180];
    for (s, d) in &t1 {
        head.extend_from_slice(&s.to_le_bytes());
        head.extend_from_slice(&d.to_le_bytes());
    }
    for v in t2 {
        head.extend_from_slice(&v.to_le_bytes());
    }
    Ok(Some(Built { head, body, maxlen }))
}

/// Build the whole layer.  Without `land`/`areas` only the elevation tiles are
/// written, like region (122,20) of Denmark_terrain.
#[allow(clippy::too_many_arguments)]
pub fn build(
    g: &Heights,
    rings: &[Ring],
    date: &[u8],
    country: u32,
    rate: f32,
    device: &[u8],
    land: Option<&[Geom]>,
    areas: Option<&Extract>,
    only: Option<(i64, i64)>,
    log: &dyn Fn(&str),
) -> Result<Vec<u8>> {
    let pts: Vec<(f64, f64)> = rings.iter().flat_map(|r| r.pts.iter().cloned()).collect();
    ensure!(!pts.is_empty(), "the boundary polygon is empty");
    let lon: Vec<f64> = pts.iter().map(|p| p.0 / 2f64.powi(28) * 360.0 - 180.0).collect();
    let lat: Vec<f64> = pts.iter().map(|p| 90.0 - p.1 / 2f64.powi(28) * 360.0).collect();
    let fmin = |v: &[f64]| v.iter().cloned().fold(f64::INFINITY, f64::min);
    let fmax = |v: &[f64]| v.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let (mut xs, mut ys): (Vec<i64>, Vec<i64>) = (
        (((fmin(&lon) + 180.0) / TILE).floor() as i64
            ..=((fmax(&lon) + 180.0) / TILE).floor() as i64)
            .collect(),
        (((90.0 - fmax(&lat)) / TILE).floor() as i64
            ..=((90.0 - fmin(&lat)) / TILE).floor() as i64)
            .collect(),
    );
    if let Some((x, y)) = only {
        xs = vec![x];
        ys = vec![y];
    }

    let none = Extract::default();
    let prep = match land {
        Some(l) => Some(prepare(rings, l, areas.unwrap_or(&none), &xs, &ys, log)?),
        None => None,
    };

    let key = chart::global_key(device);
    let t0 = std::time::Instant::now();
    let mut out: Vec<(i64, i64, Vec<u8>)> = Vec::new();
    let mut maxlen = 0usize;
    for &x in &xs {
        for &y in &ys {
            let Some(r) = region(x, y, g, rate, prep.as_ref(), areas)? else {
                continue;
            };
            let njpg = (0..85)
                .filter(|k| chart::u32_at(&r.head, 0x1180 + 8 * k) != 0xFFFF_FFFD)
                .count();
            let ndem =
                (0..85).filter(|k| chart::u32_at(&r.head, 0x1428 + 4 * k) != 0xFFFF_FFFF).count();
            log(&format!(
                "({},{}) {} images, {} heights, {} KB  [{:.0} s]",
                x,
                y,
                njpg,
                ndem,
                r.body.len() / 1024,
                t0.elapsed().as_secs_f32()
            ));
            let mut blob = r.head;
            blob.extend_from_slice(&pc1::encrypt_blob(&urandom(32), &key));
            ensure!(blob.len() == HEAD, "region head is {} bytes", blob.len());
            blob.extend_from_slice(&r.body);
            out.push((x, y, blob));
            maxlen = maxlen.max(r.maxlen);
        }
    }

    let n = out.len();
    let mut off = 0x78 + 8 * n;
    let mut d: Vec<u8> = Vec::new();
    d.extend_from_slice(&chart::MAGIC.to_le_bytes());
    d.extend_from_slice(&urandom(48));
    d.resize(d.len() + 16, 0); // MAC, filled in below
    d.extend_from_slice(date);
    for v in [5u32, 0x20, country, 0, 0, 0, 0, 0, maxlen as u32, 0xFFFF_FFFF, n as u32] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    for (x, y, blob) in &out {
        d.extend_from_slice(&(*x as u16).to_le_bytes());
        d.extend_from_slice(&(*y as u16).to_le_bytes());
        d.extend_from_slice(&(off as u32).to_le_bytes());
        off += blob.len();
    }
    for (_, _, blob) in &out {
        d.extend_from_slice(blob);
    }
    let mac = chart::header_md5(&d, device);
    d[0x34..0x44].copy_from_slice(&mac);
    Ok(d)
}
