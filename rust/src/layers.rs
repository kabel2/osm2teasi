//! The decrypted record contents of the vector layers.  Port of tools/layers.py;
//! formats in docs/OSM*_FORMAT.md and docs/TA_FORMAT.md.
//!
//! Every container keeps the fields it does not interpret verbatim, so
//! `build(parse(raw)) == raw` for all records of the original files.

use anyhow::{bail, ensure, Result};

/// Slot areas in the tile head: (offset, cells per tile edge).
pub const AREAS: [(char, usize, usize); 4] =
    [('A', 0x000, 4), ('B', 0x040, 8), ('C', 0x140, 4), ('D', 0x180, 32)];

pub fn area_index(area: char) -> usize {
    AREAS.iter().position(|&(a, _, _)| a == area).expect("unknown slot area")
}

pub fn area_grid(area: char) -> usize {
    AREAS[area_index(area)].2
}

pub fn area_offset(area: char) -> usize {
    AREAS[area_index(area)].1
}

// --------------------------------------------------------------------------
// byte helpers
// --------------------------------------------------------------------------

struct Cur<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Cur<'a> {
    fn new(d: &'a [u8]) -> Self {
        Cur { d, p: 0 }
    }

    fn need(&self, n: usize) -> Result<()> {
        ensure!(self.p + n <= self.d.len(), "record truncated at {} (+{})", self.p, n);
        Ok(())
    }

    fn u32s(&mut self, n: usize) -> Result<Vec<u32>> {
        self.need(4 * n)?;
        let v = (0..n)
            .map(|i| u32::from_le_bytes(self.d[self.p + 4 * i..self.p + 4 * i + 4].try_into().unwrap()))
            .collect();
        self.p += 4 * n;
        Ok(v)
    }

    fn u16s(&mut self, n: usize) -> Result<Vec<u16>> {
        self.need(2 * n)?;
        let v = (0..n)
            .map(|i| u16::from_le_bytes(self.d[self.p + 2 * i..self.p + 2 * i + 2].try_into().unwrap()))
            .collect();
        self.p += 2 * n;
        Ok(v)
    }

    /// n structs of `words` u32 each, stored byte-transposed (FUN_002704f0).
    fn transposed(&mut self, n: usize, words: usize) -> Result<Vec<Vec<u32>>> {
        let size = 4 * words;
        self.need(n * size)?;
        let src = &self.d[self.p..self.p + n * size];
        let out = (0..n)
            .map(|i| {
                (0..words)
                    .map(|j| {
                        u32::from_le_bytes([
                            src[(4 * j) * n + i],
                            src[(4 * j + 1) * n + i],
                            src[(4 * j + 2) * n + i],
                            src[(4 * j + 3) * n + i],
                        ])
                    })
                    .collect()
            })
            .collect();
        self.p += n * size;
        Ok(out)
    }
}

fn put_u32s(out: &mut Vec<u8>, v: &[u32]) {
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
}

fn put_u16s(out: &mut Vec<u8>, v: &[u16]) {
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
}

fn put_transposed(out: &mut Vec<u8>, structs: &[Vec<u32>]) {
    if structs.is_empty() {
        return;
    }
    let words = structs[0].len();
    for b in 0..4 * words {
        for s in structs {
            out.push(s[b / 4].to_le_bytes()[b % 4]);
        }
    }
}

// --------------------------------------------------------------------------
// generic "header + transposed arrays + variable parts" container (D and C)
// --------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Var {
    U16(Vec<u16>),
    U32(Vec<u32>),
}

#[derive(Clone, Copy, Debug)]
pub enum VarKind {
    U16,
    U32,
}

/// One array: struct size in u32 words, header word holding the count, and the
/// variable parts per struct as (element type, index of the counting field).
pub struct ArraySpec {
    pub name: &'static str,
    pub words: usize,
    pub count: usize,
    pub vars: &'static [(VarKind, usize)],
}

macro_rules! spec {
    ($name:literal, $words:expr, $count:expr, [$(($k:ident, $f:expr)),*]) => {
        ArraySpec { name: $name, words: $words, count: $count,
                    vars: &[$((VarKind::$k, $f)),*] }
    };
}

/// Slot area D (32x32 cells), FUN_003e56fc: osm, osmpoi and ta.
pub const D_HDR: usize = 13;
pub const D_SPEC: [ArraySpec; 5] = [
    spec!("a1", 10, 3, [(U32, 8)]),
    spec!("a2", 2, 5, [(U32, 0)]),
    spec!("a3", 3, 7, [(U32, 1)]),
    spec!("a4", 5, 9, [(U16, 1), (U32, 3)]),
    spec!("a5", 6, 11, [(U16, 1), (U16, 3)]),
];

/// Slot area C (4x4 cells), FUN_003e651c: osm and osmarea.
pub const C_HDR: usize = 15;
pub const C_SPEC: [ArraySpec; 6] = [
    spec!("c1", 4, 3, [(U32, 2)]),
    spec!("c2", 2, 5, [(U32, 0)]),
    spec!("c3", 6, 7, [(U16, 1), (U32, 4)]),
    spec!("c4", 3, 9, [(U32, 1)]),
    spec!("c5", 7, 11, [(U16, 1), (U32, 5)]),
    spec!("c6", 6, 13, [(U16, 1), (U32, 4)]),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub s: Vec<u32>,
    pub v: Vec<Var>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArrayRec {
    pub hdr: Vec<u32>,
    pub arrays: Vec<Vec<Item>>,
}

impl ArrayRec {
    pub fn get(&self, name: &str, spec: &[ArraySpec]) -> &[Item] {
        let i = spec.iter().position(|s| s.name == name).expect("unknown array");
        &self.arrays[i]
    }
}

pub fn parse_arrays(raw: &[u8], nhdr: usize, spec: &[ArraySpec]) -> Result<ArrayRec> {
    let mut c = Cur::new(raw);
    let hdr = c.u32s(nhdr)?;
    let mut arrays: Vec<Vec<Item>> = Vec::with_capacity(spec.len());
    for a in spec {
        let n = hdr[a.count] as usize;
        arrays.push(
            c.transposed(n, a.words)?
                .into_iter()
                .map(|s| Item { s, v: Vec::new() })
                .collect(),
        );
    }
    for (ai, a) in spec.iter().enumerate() {
        for it in arrays[ai].iter_mut() {
            for &(kind, field) in a.vars {
                let n = it.s[field] as usize;
                it.v.push(match kind {
                    VarKind::U16 => Var::U16(c.u16s(n)?),
                    VarKind::U32 => Var::U32(c.u32s(n)?),
                });
            }
        }
    }
    ensure!(c.p == raw.len(), "{} of {} bytes used", c.p, raw.len());
    Ok(ArrayRec { hdr, arrays })
}

pub fn build_arrays(rec: &ArrayRec, _spec: &[ArraySpec]) -> Vec<u8> {
    let mut out = Vec::new();
    put_u32s(&mut out, &rec.hdr);
    for arr in &rec.arrays {
        let structs: Vec<Vec<u32>> = arr.iter().map(|it| it.s.clone()).collect();
        put_transposed(&mut out, &structs);
    }
    for arr in &rec.arrays {
        for it in arr {
            for v in &it.v {
                match v {
                    Var::U16(x) => put_u16s(&mut out, x),
                    Var::U32(x) => put_u32s(&mut out, x),
                }
            }
        }
    }
    out
}

/// Split a geometry u32 array into parts [(hi, points)].  Each part is a u32
/// (hi << 16 | n) followed by n packed points; for lines hi == n, for osmarea
/// polygons hi is a detail level 9..14.
pub fn geometry_parts(g: &[u32]) -> Vec<(u32, &[u32])> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < g.len() {
        let n = (g[i] & 0xFFFF) as usize;
        let end = (i + 1 + n).min(g.len());
        out.push((g[i] >> 16, &g[i + 1..end]));
        i = end;
    }
    out
}

pub fn parse_d(raw: &[u8]) -> Result<ArrayRec> {
    parse_arrays(raw, D_HDR, &D_SPEC)
}

pub fn parse_c(raw: &[u8]) -> Result<ArrayRec> {
    parse_arrays(raw, C_HDR, &C_SPEC)
}

// --------------------------------------------------------------------------
// osmpoi: only array a5 of a D record is used (OSMPOI_FORMAT.md)
// --------------------------------------------------------------------------

/// a5 struct: [0] pointer to the name, [1] its length in UTF-16 units,
/// [2]/[3] the same for the attribute string, [4] POI type, [5] position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoiItem {
    pub typ: u32,
    pub pos: u32,
    pub name: String,
    pub attrs: String,
}

/// UTF-16 code units of `text` including the terminating NUL; empty for "".
pub fn u16enc(text: &str) -> Vec<u16> {
    if text.is_empty() {
        return Vec::new();
    }
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// UTF-16 code units -> string, without the terminating NUL.
pub fn u16str(v: &[u16]) -> String {
    let s = String::from_utf16_lossy(v);
    s.trim_end_matches('\0').to_string()
}

pub fn parse_osmpoi(raw: &[u8]) -> Result<Vec<PoiItem>> {
    let rec = parse_d(raw)?;
    Ok(rec
        .get("a5", &D_SPEC)
        .iter()
        .map(|it| {
            let text = |i: usize| match &it.v[i] {
                Var::U16(v) => u16str(v),
                Var::U32(_) => String::new(),
            };
            PoiItem { typ: it.s[4], pos: it.s[5], name: text(0), attrs: text(1) }
        })
        .collect())
}

pub fn build_osmpoi(pois: &[PoiItem]) -> Vec<u8> {
    let a5: Vec<Item> = pois
        .iter()
        .map(|p| {
            let (name, attrs) = (u16enc(&p.name), u16enc(&p.attrs));
            Item {
                s: vec![0, name.len() as u32, 0, attrs.len() as u32, p.typ, p.pos],
                v: vec![Var::U16(name), Var::U16(attrs)],
            }
        })
        .collect();
    let mut hdr = vec![0u32; D_HDR];
    hdr[0] = (a5.len() * 0x18) as u32;
    hdr[11] = a5.len() as u32;
    let rec = ArrayRec { hdr, arrays: vec![Vec::new(), Vec::new(), Vec::new(), Vec::new(), a5] };
    build_arrays(&rec, &D_SPEC)
}

// --------------------------------------------------------------------------
// slot area A (4x4 cells), FUN_003e6044: osm and ta.  Not transposed.
// --------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sub18 {
    pub s: Vec<u32>, // 6 words
    pub text: Vec<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sub10 {
    pub s: Vec<u32>, // 4 words
    pub text: Vec<u16>,
    pub nums: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AItem {
    pub s: Vec<u32>, // 7 words
    pub name: Vec<u16>,
    pub s18: Vec<Sub18>,
    pub s10: Vec<Sub10>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ARec {
    pub hdr: Vec<u32>, // 7 words; [5] and [6] are the offsets of blk5 / blk6
    pub items: Vec<AItem>,
    pub blk5: Vec<u8>,
    pub blk6: Vec<u8>,
}

pub fn parse_a(raw: &[u8]) -> Result<ARec> {
    let mut c = Cur::new(raw);
    let hdr = c.u32s(7)?;
    let n = hdr[3] as usize;
    let mut items: Vec<AItem> = Vec::with_capacity(n);
    for _ in 0..n {
        items.push(AItem { s: c.u32s(7)?, name: Vec::new(), s18: Vec::new(), s10: Vec::new() });
    }
    for it in items.iter_mut() {
        let len = it.s[1] as usize;
        it.name = c.u16s(len)?;
    }
    for it in items.iter_mut() {
        for _ in 0..it.s[3] {
            it.s18.push(Sub18 { s: c.u32s(6)?, text: Vec::new() });
        }
    }
    for it in items.iter_mut() {
        for _ in 0..it.s[5] {
            it.s10.push(Sub10 { s: c.u32s(4)?, text: Vec::new(), nums: Vec::new() });
        }
    }
    for it in items.iter_mut() {
        for sub in it.s18.iter_mut() {
            let len = sub.s[1] as usize;
            sub.text = c.u16s(len)?;
        }
    }
    for it in items.iter_mut() {
        for sub in it.s10.iter_mut() {
            let len = sub.s[1] as usize;
            sub.text = c.u16s(len)?;
        }
    }
    c.p += c.p & 2; // pad to 4 bytes
    for it in items.iter_mut() {
        for sub in it.s10.iter_mut() {
            let n = sub.s[2] as usize;
            sub.nums = c.u32s(n)?;
        }
    }
    let (b5, b6) = (hdr[5] as usize, hdr[6] as usize);
    ensure!(
        c.p == b5 && b5 <= b6 && b6 <= raw.len(),
        "A record: {} / {} / {} / {}",
        c.p,
        b5,
        b6,
        raw.len()
    );
    Ok(ARec { hdr, items, blk5: raw[b5..b6].to_vec(), blk6: raw[b6..].to_vec() })
}

pub fn build_a(rec: &ARec) -> Vec<u8> {
    let mut out = Vec::new();
    put_u32s(&mut out, &rec.hdr);
    for it in &rec.items {
        put_u32s(&mut out, &it.s);
    }
    for it in &rec.items {
        put_u16s(&mut out, &it.name);
    }
    for it in &rec.items {
        for sub in &it.s18 {
            put_u32s(&mut out, &sub.s);
        }
    }
    for it in &rec.items {
        for sub in &it.s10 {
            put_u32s(&mut out, &sub.s);
        }
    }
    for it in &rec.items {
        for sub in &it.s18 {
            put_u16s(&mut out, &sub.text);
        }
    }
    for it in &rec.items {
        for sub in &it.s10 {
            put_u16s(&mut out, &sub.text);
        }
    }
    out.resize(out.len() + (out.len() & 2), 0);
    for it in &rec.items {
        for sub in &it.s10 {
            put_u32s(&mut out, &sub.nums);
        }
    }
    out.extend_from_slice(&rec.blk5);
    out.extend_from_slice(&rec.blk6);
    out
}

// --------------------------------------------------------------------------
// slot area B (8x8 cells), FUN_003e5d20: the osm routing graph.
// LZMA only, no PC1, not transposed.
// --------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BRec {
    pub hdr: Vec<u32>, // 11 words
    pub nodes: Vec<Vec<u32>>,
    pub edges: Vec<Vec<u32>>,
    pub b3: Vec<Vec<u32>>,
    pub extra: Vec<Vec<u32>>,
}

const B_ARRAYS: [(usize, usize); 4] = [(3, 3), (5, 4), (7, 4), (9, 3)]; // (count word, words)

pub fn parse_b(raw: &[u8]) -> Result<BRec> {
    let mut c = Cur::new(raw);
    let hdr = c.u32s(11)?;
    let mut arr: Vec<Vec<Vec<u32>>> = Vec::with_capacity(4);
    for &(ci, words) in B_ARRAYS.iter() {
        let n = hdr[ci] as usize;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(c.u32s(words)?);
        }
        arr.push(v);
    }
    ensure!(c.p == raw.len(), "B record: {} of {} bytes", c.p, raw.len());
    let mut it = arr.into_iter();
    Ok(BRec {
        hdr,
        nodes: it.next().unwrap(),
        edges: it.next().unwrap(),
        b3: it.next().unwrap(),
        extra: it.next().unwrap(),
    })
}

pub fn build_b(rec: &BRec) -> Vec<u8> {
    let mut out = Vec::new();
    put_u32s(&mut out, &rec.hdr);
    for arr in [&rec.nodes, &rec.edges, &rec.b3, &rec.extra] {
        for s in arr {
            put_u32s(&mut out, s);
        }
    }
    out
}

// --------------------------------------------------------------------------
// osmpoint: seamarks (FUN_003e52d0), slot area C of the osmpoint layer
// --------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sector {
    pub w: Vec<u16>, // colour, from, to (tenths of a degree), radius
    pub a: u32,      // unused in the originals
    pub len: u32,    // label length in UTF-16 units
    pub label: Vec<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PointObj {
    pub s: Vec<u32>, // 12 words
    pub name: Vec<u16>,
    pub attrs: Vec<u16>,
    pub label: Vec<u16>,
    pub sectors: Vec<Sector>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PointRec {
    pub hdr: Vec<u32>, // 5 words
    pub objs: Vec<PointObj>,
}

pub fn parse_osmpoint(raw: &[u8]) -> Result<PointRec> {
    let mut c = Cur::new(raw);
    let hdr = c.u32s(5)?;
    let n = hdr[3] as usize;
    let sts = c.transposed(n, 12)?;
    let mut objs: Vec<PointObj> = Vec::with_capacity(n);
    for s in sts {
        let (ln, la, ll, ns) = (s[1] as usize, s[7] as usize, s[9] as usize, s[10] as usize);
        let name = c.u16s(ln)?;
        let attrs = c.u16s(la)?;
        let label = c.u16s(ll)?;
        let mut sectors = Vec::with_capacity(ns);
        for _ in 0..ns {
            let w = c.u16s(4)?;
            let nums = c.u32s(2)?;
            sectors.push(Sector { w, a: nums[0], len: nums[1], label: Vec::new() });
        }
        objs.push(PointObj { s, name, attrs, label, sectors });
    }
    for o in objs.iter_mut() {
        for sec in o.sectors.iter_mut() {
            let len = sec.len as usize;
            sec.label = c.u16s(len)?;
        }
    }
    ensure!(c.p == raw.len(), "osmpoint record: {} of {} bytes", c.p, raw.len());
    Ok(PointRec { hdr, objs })
}

pub fn build_osmpoint(rec: &PointRec) -> Vec<u8> {
    let mut out = Vec::new();
    put_u32s(&mut out, &rec.hdr);
    let structs: Vec<Vec<u32>> = rec.objs.iter().map(|o| o.s.clone()).collect();
    put_transposed(&mut out, &structs);
    for o in &rec.objs {
        put_u16s(&mut out, &o.name);
        put_u16s(&mut out, &o.attrs);
        put_u16s(&mut out, &o.label);
        for sec in &o.sectors {
            put_u16s(&mut out, &sec.w);
            put_u32s(&mut out, &[sec.a, sec.len]);
        }
    }
    for o in &rec.objs {
        for sec in &o.sectors {
            put_u16s(&mut out, &sec.label);
        }
    }
    out
}

// --------------------------------------------------------------------------
// dispatch: which container a record uses depends on layer and slot area
// --------------------------------------------------------------------------

pub const LAYER_OSM: u32 = 1;
pub const LAYER_OSMAREA: u32 = 4;
pub const LAYER_OSMPOI: u32 = 8;
pub const LAYER_TA: u32 = 0x12;
pub const LAYER_OSMPOINT: u32 = 1024;

pub fn layer_name(layer: u32) -> &'static str {
    match layer {
        LAYER_OSM => "osm",
        LAYER_OSMAREA => "osmarea",
        LAYER_OSMPOI => "osmpoi",
        LAYER_TA => "ta",
        LAYER_OSMPOINT => "osmpoint",
        32 => "terrain",
        _ => "?",
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    D(ArrayRec),
    C(ArrayRec),
    A(ARec),
    B(BRec),
    Point(PointRec),
}

pub fn parse_record(layer: u32, area: char, raw: &[u8]) -> Result<Record> {
    Ok(match area {
        'A' => Record::A(parse_a(raw)?),
        'B' => Record::B(parse_b(raw)?),
        'C' if layer == LAYER_OSMPOINT => Record::Point(parse_osmpoint(raw)?),
        'C' => Record::C(parse_c(raw)?),
        'D' => Record::D(parse_d(raw)?),
        _ => bail!("unknown slot area {}", area),
    })
}

pub fn build_record(rec: &Record) -> Vec<u8> {
    match rec {
        Record::D(r) => build_arrays(r, &D_SPEC),
        Record::C(r) => build_arrays(r, &C_SPEC),
        Record::A(r) => build_a(r),
        Record::B(r) => build_b(r),
        Record::Point(r) => build_osmpoint(r),
    }
}

// --------------------------------------------------------------------------
// slot tables of a tile head
// --------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct Slot {
    pub cx: usize,
    pub cy: usize,
    pub slot: usize,
    pub rec: crate::chart::Rec,
}

/// The records a tile's slot table of `area` points at, in slot order.
/// Slot k of a g x g area belongs to sub cell (x*g + k/g, y*g + k%g).
pub fn slots(d: &[u8], tile: &crate::chart::Tile, area: char) -> Vec<Slot> {
    let (off, g) = (area_offset(area), area_grid(area));
    let mut out = Vec::new();
    for k in 0..g * g {
        let rel = crate::chart::u32_at(d, tile.start + off + 4 * k) as usize;
        if rel == 0xFFFF_FFFF {
            continue;
        }
        out.push(Slot {
            cx: tile.x as usize * g + k / g,
            cy: tile.y as usize * g + k % g,
            slot: k,
            rec: crate::chart::Rec {
                rel,
                len: crate::chart::u32_at(d, tile.start + rel) as usize,
                pltx: crate::chart::u32_at(d, tile.start + rel + 4) as usize,
            },
        });
    }
    out
}
