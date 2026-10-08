//! The search index of the ta layer: the extra table behind header 0x70.
//! Port of tools/ta_index.py; format in docs/TA_FORMAT.md ("Suchindex").
//!
//! The firmware registers it per country (FUN_0016068c) and walks it for the
//! place search (FUN_00162788, FUN_00163384, FUN_0016201c).  All offsets are
//! relative to the start of the table.

use std::collections::HashSet;

use anyhow::{ensure, Result};

/// Language mask "all languages".
pub const ALL: u64 = u64::MAX;

/// Search key normalisation, `fold()` in tools/ta_index.py: lower case,
/// Danish letters spelled out, accents dropped.
pub fn fold(s: &str) -> String {
    use unicode_normalization::char::canonical_combining_class;
    use unicode_normalization::UnicodeNormalization;
    let folded: String = s
        .to_lowercase()
        .replace('\u{f8}', "o")
        .replace('\u{e6}', "a")
        .replace('\u{df}', "ss");
    folded.nfkd().filter(|&c| canonical_combining_class(c) == 0).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Name {
    /// Word offsets into the pool, joined with a space.
    Pool(Vec<u32>),
    /// UTF-8, stored in the node itself.
    Inline(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub x: u16,
    pub y: u16,
    /// Byte offsets of street names in blk5; only type 1 (postcodes) has them.
    pub offs: Vec<u32>,
}

/// type 0 = place with streets, 1 = postcode, 2 = place without streets.
#[derive(Clone, Debug, PartialEq)]
pub struct Res {
    pub typ: u8,
    pub name: Name,
    pub lon: f32,
    pub lat: f32,
    pub cells: Vec<Cell>,
}

impl Res {
    /// What the node headers count: one result may sit under many keys.
    fn ident(&self) -> Vec<u8> {
        let mut v = vec![self.typ];
        match &self.name {
            Name::Inline(s) => {
                v.push(0);
                v.extend_from_slice(s);
            }
            Name::Pool(w) => {
                v.push(1);
                for x in w {
                    v.extend_from_slice(&x.to_le_bytes());
                }
            }
        }
        v.extend_from_slice(&self.lon.to_bits().to_le_bytes());
        v.extend_from_slice(&self.lat.to_bits().to_le_bytes());
        v
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Kid {
    pub ch: u16,
    pub mask: u64,
    pub node: Node,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Node {
    pub kids: Vec<Kid>,
    pub res: Vec<Res>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Index {
    pub langs: Vec<([u8; 3], u8)>,
    pub one: u32,
    pub country: Vec<u16>,
    pub root: Node,
    /// (offset relative to the pool start, word); must stay in offset order.
    pub pool: Vec<(u32, Vec<u8>)>,
}

impl Index {
    pub fn country_name(&self) -> String {
        String::from_utf16_lossy(&self.country)
    }

    /// Distinct results in the whole index.
    pub fn count(&self) -> usize {
        let mut acc = HashSet::new();
        collect_idents(&self.root, &mut acc);
        acc.len()
    }

    pub fn word(&self, off: u32) -> Option<&[u8]> {
        self.pool
            .binary_search_by_key(&off, |(o, _)| *o)
            .ok()
            .map(|i| self.pool[i].1.as_slice())
    }

    /// Name of a result as a string.
    pub fn text(&self, r: &Res) -> String {
        match &r.name {
            Name::Inline(s) => String::from_utf8_lossy(s).into_owned(),
            Name::Pool(w) => w
                .iter()
                .map(|o| self.word(*o).map(|b| String::from_utf8_lossy(b).into_owned())
                    .unwrap_or_else(|| format!("<{}>", o)))
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
}

fn collect_idents(nd: &Node, acc: &mut HashSet<Vec<u8>>) {
    for r in &nd.res {
        acc.insert(r.ident());
    }
    for k in &nd.kids {
        collect_idents(&k.node, acc);
    }
}

struct Cur<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Cur<'a> {
    fn u8(&mut self) -> u8 {
        self.p += 1;
        self.d[self.p - 1]
    }
    fn u16(&mut self) -> u16 {
        self.p += 2;
        u16::from_le_bytes(self.d[self.p - 2..self.p].try_into().unwrap())
    }
    fn u32(&mut self) -> u32 {
        self.p += 4;
        u32::from_le_bytes(self.d[self.p - 4..self.p].try_into().unwrap())
    }
    fn u64(&mut self) -> u64 {
        self.p += 8;
        u64::from_le_bytes(self.d[self.p - 8..self.p].try_into().unwrap())
    }
    fn f32(&mut self) -> f32 {
        f32::from_bits(self.u32())
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        self.p += n;
        self.d[self.p - n..self.p].to_vec()
    }
}

pub fn parse(t: &[u8]) -> Result<Index> {
    let mut c = Cur { d: t, p: 0 };
    let pool_off = c.u32() as usize;
    let nlang = c.u8() as usize;
    let langs: Vec<([u8; 3], u8)> = (0..nlang)
        .map(|_| {
            let code: [u8; 3] = c.bytes(3).try_into().unwrap();
            (code, c.u8())
        })
        .collect();
    let one = c.u32();
    let clen = c.u16() as usize;
    let country: Vec<u16> = (0..clen).map(|_| c.u16()).collect();
    let root_off = c.u32() as usize;

    let mut end = 0usize;
    let root = parse_node(t, root_off, &mut end)?;
    ensure!(end == pool_off, "nodes end at {}, pool at {}", end, pool_off);

    let mut pool = Vec::new();
    let mut p = pool_off;
    while p < t.len() {
        let n = u16::from_le_bytes(t[p..p + 2].try_into().unwrap()) as usize;
        pool.push(((p - pool_off) as u32, t[p + 2..p + 2 + n].to_vec()));
        p += 2 + n;
    }
    Ok(Index { langs, one, country, root, pool })
}

fn parse_node(t: &[u8], off: usize, end: &mut usize) -> Result<Node> {
    let mut c = Cur { d: t, p: off };
    let h = c.u32();
    let nkids = if h == 0xFFFF_FFFF {
        c.u32();
        c.u32() as usize
    } else {
        (h >> 24) as usize
    };
    let slots: Vec<(u16, u64, usize)> = (0..nkids)
        .map(|_| (c.u16(), c.u64(), c.u32() as usize))
        .collect();
    let nres = c.u16() as usize;
    let mut res = Vec::with_capacity(nres);
    for _ in 0..nres {
        let ty = c.u8();
        let typ = ty & 0x7F;
        let name = if ty & 0x80 != 0 {
            let w = c.u32();
            let mut words = vec![w & 0xFF_FFFF];
            for _ in 1..(w >> 24) {
                words.push(c.u32());
            }
            Name::Pool(words)
        } else {
            let n = c.u16() as usize;
            Name::Inline(c.bytes(n))
        };
        let lon = c.f32();
        let lat = c.f32();
        let mut cells = Vec::new();
        if typ != 2 {
            let mut n = c.u8() as usize;
            if n == 0xFF {
                n = c.u16() as usize;
            }
            for _ in 0..n {
                let (x, y) = (c.u16(), c.u16());
                let offs = if typ == 1 {
                    let k = c.u16() as usize;
                    (0..k).map(|_| c.u32()).collect()
                } else {
                    Vec::new()
                };
                cells.push(Cell { x, y, offs });
            }
        }
        res.push(Res { typ, name, lon, lat, cells });
    }
    *end = (*end).max(c.p);
    let kids = slots
        .into_iter()
        .map(|(ch, mask, o)| Ok(Kid { ch, mask, node: parse_node(t, o, end)? }))
        .collect::<Result<Vec<_>>>()?;
    Ok(Node { kids, res })
}

fn res_bytes(r: &Res) -> Vec<u8> {
    let mut b = Vec::new();
    let pool = matches!(r.name, Name::Pool(_));
    b.push(r.typ | if pool { 0x80 } else { 0 });
    match &r.name {
        Name::Pool(w) => {
            b.extend_from_slice(&(((w.len() as u32) << 24) | w[0]).to_le_bytes());
            for x in &w[1..] {
                b.extend_from_slice(&x.to_le_bytes());
            }
        }
        Name::Inline(s) => {
            b.extend_from_slice(&(s.len() as u16).to_le_bytes());
            b.extend_from_slice(s);
        }
    }
    b.extend_from_slice(&r.lon.to_le_bytes());
    b.extend_from_slice(&r.lat.to_le_bytes());
    if r.typ != 2 {
        let n = r.cells.len();
        if n < 0xFF {
            b.push(n as u8);
        } else {
            b.push(0xFF);
            b.extend_from_slice(&(n as u16).to_le_bytes());
        }
        for cell in &r.cells {
            b.extend_from_slice(&cell.x.to_le_bytes());
            b.extend_from_slice(&cell.y.to_le_bytes());
            if r.typ == 1 {
                b.extend_from_slice(&(cell.offs.len() as u16).to_le_bytes());
                for o in &cell.offs {
                    b.extend_from_slice(&o.to_le_bytes());
                }
            }
        }
    }
    b
}

/// Inverse of `parse`.
pub fn build(idx: &Index) -> Result<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(&0u32.to_le_bytes()); // pool offset, patched below
    out.push(idx.langs.len() as u8);
    for (code, i) in &idx.langs {
        out.extend_from_slice(code);
        out.push(*i);
    }
    out.extend_from_slice(&idx.one.to_le_bytes());
    out.extend_from_slice(&(idx.country.len() as u16).to_le_bytes());
    for ch in &idx.country {
        out.extend_from_slice(&ch.to_le_bytes());
    }
    let root_off = out.len() + 4;
    out.extend_from_slice(&(root_off as u32).to_le_bytes());
    write_node(&idx.root, &mut out)?;

    let pool_off = out.len() as u32;
    out[..4].copy_from_slice(&pool_off.to_le_bytes());
    for (o, w) in &idx.pool {
        ensure!(out.len() as u32 - pool_off == *o, "pool word at {} not in order", o);
        out.extend_from_slice(&(w.len() as u16).to_le_bytes());
        out.extend_from_slice(w);
    }
    Ok(out)
}

/// Append a node and its subtree; -> (offset, distinct results below it).
fn write_node(nd: &Node, out: &mut Vec<u8>) -> Result<(usize, HashSet<Vec<u8>>)> {
    let off = out.len();
    let n = nd.kids.len();
    let big = n >= 0x100;
    let kid_at = off + if big { 12 } else { 4 };
    out.resize(kid_at + 14 * n, 0);
    out.extend_from_slice(&(nd.res.len() as u16).to_le_bytes());
    for r in &nd.res {
        out.extend_from_slice(&res_bytes(r));
    }
    let mut acc: HashSet<Vec<u8>> = nd.res.iter().map(|r| r.ident()).collect();
    for (i, k) in nd.kids.iter().enumerate() {
        let (ko, sub) = write_node(&k.node, out)?;
        acc.extend(sub);
        let at = kid_at + 14 * i;
        out[at..at + 2].copy_from_slice(&k.ch.to_le_bytes());
        out[at + 2..at + 10].copy_from_slice(&k.mask.to_le_bytes());
        out[at + 10..at + 14].copy_from_slice(&(ko as u32).to_le_bytes());
    }
    let tot = acc.len();
    if big || tot >= 1 << 24 {
        ensure!(big, "more than 2^24 results needs the long node header");
        out[off..off + 4].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        out[off + 4..off + 8].copy_from_slice(&(tot as u32).to_le_bytes());
        out[off + 8..off + 12].copy_from_slice(&(n as u32).to_le_bytes());
    } else {
        let h = ((n as u32) << 24) | tot as u32;
        out[off..off + 4].copy_from_slice(&h.to_le_bytes());
    }
    Ok((off, acc))
}

/// The extra table of a ta file: from header 0x70 to the start of the next tile.
pub fn extract(c: &crate::chart::Chart) -> Result<Vec<u8>> {
    let off = c.extra() as usize;
    ensure!(off != 0xFFFF_FFFF, "no extra table (header 0x70 = -1)");
    let end = c
        .tiles()
        .iter()
        .map(|t| t.start)
        .filter(|&s| s > off)
        .min()
        .unwrap_or(c.data.len());
    Ok(c.data[off..end].to_vec())
}
