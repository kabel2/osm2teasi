//! The pieces of Pillow and its codecs that the terrain layer needs: polygon
//! filling, Lanczos resizing, JPEG and JPEG 2000.
//!
//! The first two are ported from Pillow's C sources literally, down to the
//! float widths and the rounding macros -- `Draw.c: polygon_generic` and
//! `Resample.c: ImagingResampleInner` -- so that the land cover and the image
//! pyramid come out exactly as Pillow draws them.  JPEG 2000 goes
//! through the same library Pillow uses (OpenJPEG, here compiled into the
//! binary), with the parameters its `JPEG2000` plugin sets.  Only the JPEG
//! encoder is a different one (`jpeg-encoder` instead of libjpeg-turbo): the map
//! images are lossy anyway, and the layer carries no checksum over them.

// The two ports below follow Pillow's C loops index by index, so that the
// arithmetic stays comparable with the original; iterators would hide it.
#![allow(clippy::needless_range_loop)]

use std::ffi::{c_void, CStr};
use std::os::raw::{c_char, c_int};

use anyhow::{bail, Result};
use jpeg_encoder::{ChromaSubsamplingMethod, ColorType, Encoder, PixelDensity, SamplingFactor};
use openjpeg_sys as opj;

/// An image with four bytes per pixel, the way Pillow keeps "RGB" -- the
/// resampler reads a stride of 4 whatever the band count is.
pub struct Rgb {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Rgb {
    pub fn new(w: usize, h: usize) -> Rgb {
        Rgb { w, h, px: vec![0; 4 * w * h] }
    }

    pub fn set(&mut self, x: usize, y: usize, c: (u8, u8, u8)) {
        let o = 4 * (y * self.w + x);
        self.px[o] = c.0;
        self.px[o + 1] = c.1;
        self.px[o + 2] = c.2;
    }

    /// `Image.crop((x0, y0, x0 + w, y0 + h))`.
    pub fn crop(&self, x0: usize, y0: usize, w: usize, h: usize) -> Rgb {
        let mut out = Rgb::new(w, h);
        for y in 0..h {
            let s = 4 * ((y0 + y) * self.w + x0);
            let d = 4 * y * w;
            out.px[d..d + 4 * w].copy_from_slice(&self.px[s..s + 4 * w]);
        }
        out
    }

    /// The three bands without the padding byte, as Pillow's `tobytes()`.
    pub fn rgb(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(3 * self.w * self.h);
        for p in self.px.chunks_exact(4) {
            v.extend_from_slice(&p[..3]);
        }
        v
    }
}

// ---------------------------------------------------------------- Draw.c

/// `ROUND_UP` of Draw.c: half away from zero.
fn round_up(f: f32) -> i64 {
    if f >= 0.0 {
        (f as f64 + 0.5).floor() as i64
    } else {
        -((f as f64).abs() + 0.5).floor() as i64
    }
}

/// `ROUND_DOWN` of Draw.c: half towards zero.
fn round_down(f: f32) -> i64 {
    if f >= 0.0 {
        (f as f64 - 0.5).ceil() as i64
    } else {
        -((f as f64).abs() - 0.5).ceil() as i64
    }
}

/// C's `roundf`: half away from zero, in single precision.
fn roundf(f: f32) -> f32 {
    if f >= 0.0 {
        (f + 0.5).floor()
    } else {
        -((-f) + 0.5).floor()
    }
}

/// One polygon edge, as `add_edge` builds it.
struct Edge {
    x0: i64,
    y0: i64,
    xmin: i64,
    xmax: i64,
    ymin: i64,
    ymax: i64,
    dx: f32,
}

impl Edge {
    fn new(x0: i64, y0: i64, x1: i64, y1: i64) -> Edge {
        Edge {
            x0,
            y0,
            xmin: x0.min(x1),
            xmax: x0.max(x1),
            ymin: y0.min(y1),
            ymax: y0.max(y1),
            dx: if y0 == y1 { 0.0 } else { (x1 - x0) as f32 / (y1 - y0) as f32 },
        }
    }

    fn x(&self, y: i64) -> f32 {
        (y - self.y0) as f32 * self.dx + self.x0 as f32
    }
}

/// A single-band canvas, Pillow's mode "L".
pub struct Mask {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Mask {
    pub fn new(w: usize, h: usize, fill: u8) -> Mask {
        Mask { w, h, px: vec![fill; w * h] }
    }

    /// `hline8`: clip to the canvas, then paint `x0..=x1`.
    fn hline(&mut self, x0: i64, y: i64, x1: i64, ink: u8) {
        if y < 0 || y >= self.h as i64 {
            return;
        }
        let mut x0 = x0;
        let mut x1 = x1;
        if x0 < 0 {
            x0 = 0;
        } else if x0 >= self.w as i64 {
            return;
        }
        if x1 < 0 {
            return;
        } else if x1 >= self.w as i64 {
            x1 = self.w as i64 - 1;
        }
        if x0 <= x1 {
            let row = y as usize * self.w;
            self.px[row + x0 as usize..=row + x1 as usize].fill(ink);
        }
    }

    /// `ImageDraw.polygon(pts, fill=ink)`: `polygon_generic` without alpha.
    /// The coordinates are cast to int the way C does it, towards zero.
    pub fn polygon(&mut self, pts: &[(f64, f64)], ink: u8) {
        if pts.is_empty() {
            return;
        }
        let p: Vec<(i64, i64)> = pts.iter().map(|&(x, y)| (x as i64, y as i64)).collect();
        let n = p.len();
        let e: Vec<Edge> = (0..n)
            .map(|i| {
                let (x0, y0) = p[i];
                let (x1, y1) = p[(i + 1) % n];
                Edge::new(x0, y0, x1, y1)
            })
            .collect();

        let mut ymin = self.h as i64 - 1;
        let mut ymax = 0i64;
        let mut table: Vec<&Edge> = Vec::with_capacity(n);
        for ed in &e {
            ymin = ymin.min(ed.ymin);
            ymax = ymax.max(ed.ymax);
            if ed.ymin == ed.ymax {
                self.hline(ed.xmin, ed.ymin, ed.xmax, ink);
                continue;
            }
            table.push(ed);
        }
        ymin = ymin.max(0);
        ymax = ymax.min(self.h as i64);

        let mut xx: Vec<f32> = Vec::with_capacity(2 * table.len());
        for y in ymin..=ymax {
            xx.clear();
            for (i, cur) in table.iter().enumerate() {
                if y < cur.ymin || y > cur.ymax {
                    continue;
                }
                xx.push(cur.x(y));
                if y == cur.ymax && y < ymax {
                    // needed to draw consistent polygons
                    let v = xx[xx.len() - 1];
                    xx.push(v);
                } else if (y == cur.ymin || y == cur.ymax) && cur.dx != 0.0 {
                    // connect discontiguous corners
                    for other in table[..i].iter() {
                        if (y != other.ymin && y != other.ymax) || other.dx == 0.0 {
                            continue;
                        }
                        let last = xx.len() - 1;
                        if roundf(xx[last]) != roundf(other.x(y)) {
                            continue;
                        }
                        let off = if y == cur.ymax { -1 } else { 1 };
                        let adj = cur.x(y + off);
                        if y + off >= other.ymin && y + off <= other.ymax {
                            let adj_o = other.x(y + off);
                            if xx[last] > adj + 1.0 && xx[last] > adj_o + 1.0 {
                                xx[last] = roundf(adj.max(adj_o)) + 1.0;
                            } else if xx[last] < adj - 1.0 && xx[last] < adj_o - 1.0 {
                                xx[last] = roundf(adj.min(adj_o)) - 1.0;
                            }
                        }
                        break;
                    }
                }
            }
            xx.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let mut i = 1;
            while i < xx.len() {
                self.hline(round_up(xx[i - 1]), y, round_down(xx[i]), ink);
                i += 2;
            }
        }
    }
}

// ------------------------------------------------------------ Resample.c

const PRECISION_BITS: i32 = 32 - 8 - 2;
const SUPPORT: f64 = 3.0;

fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else {
        let x = x * std::f64::consts::PI;
        x.sin() / x
    }
}

fn lanczos(x: f64) -> f64 {
    if (-3.0..3.0).contains(&x) {
        sinc(x) * sinc(x / 3.0)
    } else {
        0.0
    }
}

/// `precompute_coeffs` -> (ksize, bounds as (min, count), coefficients).
fn coeffs(in_size: usize, out_size: usize) -> (usize, Vec<(usize, usize)>, Vec<i32>) {
    let scale = in_size as f32 as f64 / out_size as f64;
    let filterscale = scale.max(1.0);
    let support = SUPPORT * filterscale;
    let ksize = support.ceil() as usize * 2 + 1;
    let inv = 1.0 / filterscale;
    let mut bounds = Vec::with_capacity(out_size);
    let mut kk = vec![0i32; out_size * ksize];
    for xx in 0..out_size {
        let center = (xx as f64 + 0.5) * scale;
        let xmin = ((center - support + 0.5) as i64).max(0) as usize;
        let xmax = (((center + support + 0.5) as i64).max(0) as usize).min(in_size);
        let xmax = xmax.saturating_sub(xmin);
        let mut k = vec![0.0f64; ksize];
        let mut ww = 0.0;
        for x in 0..xmax {
            let w = lanczos(((x + xmin) as f64 - center + 0.5) * inv);
            k[x] = w;
            ww += w;
        }
        if ww != 0.0 {
            for x in 0..xmax {
                k[x] /= ww;
            }
        }
        // normalize_coeffs_8bpc: fixed point, half away from zero
        for x in 0..ksize {
            let v = k[x] * f64::from(1 << PRECISION_BITS);
            kk[xx * ksize + x] = if v < 0.0 { (v - 0.5) as i32 } else { (v + 0.5) as i32 };
        }
        bounds.push((xmin, xmax));
    }
    (ksize, bounds, kk)
}

/// `clip8`: shift back out of the fixed point and clamp.
fn clip8(v: i32) -> u8 {
    (v >> PRECISION_BITS).clamp(0, 255) as u8
}

/// `Image.resize((w, h), Image.LANCZOS)` for a square image, both passes in
/// Pillow's order (horizontal first, which is what it picks for these sizes).
pub fn resize(src: &Rgb, w: usize, h: usize) -> Rgb {
    if w == src.w && h == src.h {
        return Rgb { w, h, px: src.px.clone() };
    }
    let (kv, bv, cv) = coeffs(src.h, h);
    let (kh, bh, ch) = coeffs(src.w, w);
    // horizontal first: only the rows the vertical pass will read
    let y0 = bv[0].0;
    let y1 = bv[h - 1].0 + bv[h - 1].1;
    let mut tmp = Rgb::new(w, y1 - y0);
    for yy in 0..tmp.h {
        let line = 4 * (yy + y0) * src.w;
        for xx in 0..w {
            let (xmin, xmax) = bh[xx];
            let k = &ch[xx * kh..];
            let mut ss = [1 << (PRECISION_BITS - 1); 3];
            for x in 0..xmax {
                let o = line + 4 * (x + xmin);
                for (c, s) in ss.iter_mut().enumerate() {
                    *s += i32::from(src.px[o + c]) * k[x];
                }
            }
            let o = 4 * (yy * w + xx);
            for c in 0..3 {
                tmp.px[o + c] = clip8(ss[c]);
            }
        }
    }
    let mut out = Rgb::new(w, h);
    for yy in 0..h {
        let (ymin, ymax) = bv[yy];
        let ymin = ymin - y0;
        let k = &cv[yy * kv..];
        for xx in 0..w {
            let mut ss = [1 << (PRECISION_BITS - 1); 3];
            for y in 0..ymax {
                let o = 4 * ((y + ymin) * w + xx);
                for (c, s) in ss.iter_mut().enumerate() {
                    *s += i32::from(tmp.px[o + c]) * k[y];
                }
            }
            let o = 4 * (yy * w + xx);
            for c in 0..3 {
                out.px[o + c] = clip8(ss[c]);
            }
        }
    }
    out
}

// ----------------------------------------------------------------- JPEG

/// `img.save(b, "JPEG", quality=q, subsampling=2, dpi=(dpi, dpi), exif=EXIF)`.
/// Baseline, standard Huffman tables and the IJG quantisation tables of that
/// quality, like Pillow; only the chroma downsampling differs (box average
/// here, libjpeg's triangular `h2v2_fancy_downsample` there).
pub fn jpeg(img: &Rgb, quality: u8, dpi: u16, exif: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut enc = Encoder::new(&mut out, quality);
    enc.set_density(PixelDensity::dpi(dpi));
    enc.set_sampling_factor(SamplingFactor::F_2_2); // 4:2:0
    enc.set_chroma_subsampling_method(ChromaSubsamplingMethod::Average);
    enc.add_app_segment(1, exif.to_vec())?;
    enc.encode(&img.rgb(), img.w as u16, img.h as u16, ColorType::Rgb)?;
    Ok(out)
}

// ------------------------------------------------------------ JPEG 2000

struct Sink(Vec<u8>);

unsafe extern "C" fn write_cb(p: *mut c_void, n: usize, user: *mut c_void) -> usize {
    let s = &mut *(user as *mut Sink);
    s.0.extend_from_slice(std::slice::from_raw_parts(p as *const u8, n));
    n
}

unsafe extern "C" fn skip_cb(n: i64, _user: *mut c_void) -> i64 {
    n
}

unsafe extern "C" fn seek_cb(_n: i64, _user: *mut c_void) -> c_int {
    1
}

unsafe extern "C" fn err_cb(msg: *const c_char, _user: *mut c_void) {
    eprintln!("openjpeg: {}", CStr::from_ptr(msg).to_string_lossy().trim_end());
}

/// The OpenJPEG version the codestreams carry in their comment marker.
pub fn openjpeg_version() -> String {
    unsafe { CStr::from_ptr(opj::opj_version()).to_string_lossy().into_owned() }
}

/// `Image.fromarray(v).save(b, "JPEG2000", no_jp2=True, irreversible=True,
/// num_resolutions=6, codeblock_size=(64, 64), progression="LRCP",
/// quality_mode="rates", quality_layers=[rate])` for a 16 bit grey image.
pub fn jp2(v: &[u16], side: u32, rate: f32) -> Result<Vec<u8>> {
    let n = side as usize;
    if v.len() != n * n {
        bail!("{} values for a {}x{} tile", v.len(), side, side);
    }
    unsafe {
        let mut cp: opj::opj_cparameters_t = std::mem::zeroed();
        opj::opj_set_default_encoder_parameters(&mut cp);
        cp.irreversible = 1;
        cp.numresolution = 6;
        cp.cblockw_init = 64;
        cp.cblockh_init = 64;
        cp.prog_order = opj::OPJ_PROG_ORDER::OPJ_LRCP;
        cp.tcp_numlayers = 1;
        cp.tcp_rates[0] = rate;
        cp.cp_disto_alloc = 1;

        let mut cmpt: opj::opj_image_cmptparm_t = std::mem::zeroed();
        cmpt.dx = 1;
        cmpt.dy = 1;
        cmpt.w = side;
        cmpt.h = side;
        cmpt.prec = 16;
        cmpt.bpp = 16;
        cmpt.sgnd = 0;
        let img = opj::opj_image_create(1, &mut cmpt, opj::OPJ_COLOR_SPACE::OPJ_CLRSPC_GRAY);
        if img.is_null() {
            bail!("opj_image_create failed");
        }
        (*img).x1 = side;
        (*img).y1 = side;
        let data = (*(*img).comps).data;
        for (i, &p) in v.iter().enumerate() {
            *data.add(i) = i32::from(p);
        }

        let mut sink = Sink(Vec::new());
        let codec = opj::opj_create_compress(opj::OPJ_CODEC_FORMAT::OPJ_CODEC_J2K);
        opj::opj_set_error_handler(codec, Some(err_cb), std::ptr::null_mut());
        let stream = opj::opj_stream_default_create(0);
        opj::opj_stream_set_write_function(stream, Some(write_cb));
        opj::opj_stream_set_skip_function(stream, Some(skip_cb));
        opj::opj_stream_set_seek_function(stream, Some(seek_cb));
        opj::opj_stream_set_user_data(stream, &mut sink as *mut Sink as *mut c_void, None);

        let ok = opj::opj_setup_encoder(codec, &mut cp, img) == 1
            && opj::opj_start_compress(codec, img, stream) == 1
            && opj::opj_encode(codec, stream) == 1
            && opj::opj_end_compress(codec, stream) == 1;
        opj::opj_stream_destroy(stream);
        opj::opj_destroy_codec(codec);
        opj::opj_image_destroy(img);
        if !ok {
            bail!("OpenJPEG could not encode the tile");
        }
        Ok(sink.0)
    }
}
