//! Write chart files: the inverse of chart.rs.
//!
//! Header fields as read by the chart loader FUN_003f282c (see
//! docs/CHART_FILES.md 1): 0x44 date, 0x4C type, 0x50 layer,
//! 0x54 country, 0x58..0x6C buffer sizes, 0x70 offset of the extra table
//! (only used when layer & 0x10, i.e. ta), else -1.

use std::collections::{BTreeMap, HashSet};
use std::io::Read;

use anyhow::{ensure, Result};
use rayon::prelude::*;

use crate::chart::{self, HEAD};
use crate::layers::{area_grid, AREAS};
use crate::lzma;
use crate::pc1;

/// Slot area B (the osm routing graph) is stored LZMA-only, without PC1.
fn is_plain(area: char) -> bool {
    area == 'B'
}

pub fn urandom(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .expect("/dev/urandom");
    buf
}

/// Plaintext records of one tile: area -> slot -> bytes.
pub type Areas = BTreeMap<char, BTreeMap<usize, Vec<u8>>>;

pub struct TileContent {
    pub x: u16,
    pub y: u16,
    /// None = empty placeholder tile of size 0 (osmarea's first tile).
    pub areas: Option<Areas>,
    /// Extra bytes after the last record (ta's search index).
    pub tail: Vec<u8>,
}

pub struct Meta {
    pub date: Vec<u8>,
    pub typ: u32,
    pub layer: u32,
    pub country: u32,
    /// Tile whose tail the header's 0x70 offset points at.
    pub tail_tile: Option<(u16, u16)>,
}

fn pack_record(raw: &[u8], rk: &[u8], plain: bool) -> Result<Vec<u8>> {
    let comp = lzma::compress(raw)?;
    let payload = if plain { comp } else { pc1::encrypt_payload(&comp, rk) };
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

/// A tile whose records are compressed and encrypted already, so that a
/// compiler can hand them over tile by tile instead of holding the plaintext
/// of the whole country until the end.
pub struct Packed {
    pub x: u16,
    pub y: u16,
    rk: Vec<u8>,
    /// area -> slot -> record as it goes into the file (8-byte header first)
    recs: BTreeMap<char, BTreeMap<usize, Vec<u8>>>,
    pub tail: Vec<u8>,
}

impl Packed {
    /// An empty tile with a fresh record key.
    pub fn new(x: u16, y: u16) -> Packed {
        Packed {
            x,
            y,
            rk: urandom(32),
            recs: BTreeMap::new(),
            tail: Vec::new(),
        }
    }

    /// Compress and encrypt plaintext records into the tile, in parallel.
    pub fn add(&mut self, area: char, recs: Vec<(usize, Vec<u8>)>) -> Result<()> {
        let rk = &self.rk;
        let packed: Vec<(usize, Vec<u8>)> = recs
            .into_par_iter()
            .map(|(slot, raw)| Ok((slot, pack_record(&raw, rk, is_plain(area))?)))
            .collect::<Result<_>>()?;
        self.recs.entry(area).or_default().extend(packed);
        Ok(())
    }

    fn of(areas: &Areas, rk: Vec<u8>, x: u16, y: u16) -> Result<Packed> {
        let mut recs: BTreeMap<char, BTreeMap<usize, Vec<u8>>> = BTreeMap::new();
        for (&area, slots) in areas {
            for (&slot, raw) in slots {
                recs.entry(area)
                    .or_default()
                    .insert(slot, pack_record(raw, &rk, is_plain(area))?);
            }
        }
        Ok(Packed {
            x,
            y,
            rk,
            recs,
            tail: Vec::new(),
        })
    }
}

struct Built {
    bytes: Vec<u8>,
    maxlen: usize,
    maxraw: BTreeMap<char, usize>,
}

/// Records are written area by area (A, B, C, D) in slot order, as in the originals.
fn build_tile(t: &Packed, key: &[u8], tail: &[u8]) -> Result<Built> {
    let rk = &t.rk;
    let mut heads: BTreeMap<char, Vec<u32>> = AREAS
        .iter()
        .map(|&(a, _, g)| (a, vec![0xFFFF_FFFFu32; g * g]))
        .collect();
    let mut body: Vec<u8> = Vec::new();
    let mut maxlen = 0usize;
    let mut maxraw: BTreeMap<char, usize> = BTreeMap::new();
    for &(area, _, _) in AREAS.iter() {
        let Some(slots) = t.recs.get(&area) else {
            continue;
        };
        for (&slot, rec) in slots {
            heads.get_mut(&area).unwrap()[slot] = (HEAD + body.len()) as u32;
            maxlen = maxlen.max(rec.len() - 8);
            let raw = u32::from_le_bytes(rec[4..8].try_into().unwrap()) as usize;
            let e = maxraw.entry(area).or_default();
            *e = (*e).max(raw);
            body.extend_from_slice(rec);
        }
    }
    let mut head: Vec<u8> = Vec::with_capacity(HEAD);
    for &(area, _, _) in AREAS.iter() {
        for v in &heads[&area] {
            head.extend_from_slice(&v.to_le_bytes());
        }
    }
    for _ in 0..85 {
        head.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        head.extend_from_slice(&0u32.to_le_bytes());
    }
    head.resize(head.len() + 85 * 4, 0xFF);
    head.extend_from_slice(&pc1::encrypt_blob(rk, key));
    ensure!(head.len() == HEAD, "tile head is {} bytes", head.len());
    head.extend_from_slice(&body);
    head.extend_from_slice(tail);
    Ok(Built { bytes: head, maxlen, maxraw })
}

pub fn write_chart(
    meta: &Meta,
    tiles: &[TileContent],
    sign: &chart::Signer,
    rk: Option<&[u8]>,
) -> Result<Vec<u8>> {
    let keys: Vec<Vec<u8>> = tiles
        .iter()
        .map(|_| rk.map(|r| r.to_vec()).unwrap_or_else(|| urandom(32)))
        .collect();
    let packed: Vec<Option<Packed>> = tiles
        .par_iter()
        .zip(keys.into_par_iter())
        .map(|(t, k)| {
            t.areas
                .as_ref()
                .map(|a| Packed::of(a, k, t.x, t.y))
                .transpose()
        })
        .collect::<Result<_>>()?;
    let list: Vec<(u16, u16, Option<&Packed>, &[u8])> = tiles
        .iter()
        .zip(&packed)
        .map(|(t, p)| (t.x, t.y, p.as_ref(), &t.tail[..]))
        .collect();
    assemble(meta, &list, sign)
}

/// [`write_chart`] for tiles packed by the compiler, in directory order.
pub fn write_packed(meta: &Meta, tiles: &[Packed], sign: &chart::Signer) -> Result<Vec<u8>> {
    let list: Vec<(u16, u16, Option<&Packed>, &[u8])> = tiles
        .iter()
        .map(|t| (t.x, t.y, Some(t), &t.tail[..]))
        .collect();
    assemble(meta, &list, sign)
}

/// The file from its tiles: (x, y, records or an empty placeholder, tail).
fn assemble(
    meta: &Meta,
    tiles: &[(u16, u16, Option<&Packed>, &[u8])],
    sign: &chart::Signer,
) -> Result<Vec<u8>> {
    let key = sign.key();
    let n = tiles.len();
    let built: Vec<Option<Built>> = tiles
        .par_iter()
        .map(|(_, _, p, tail)| p.map(|p| build_tile(p, &key, tail)).transpose())
        .collect::<Result<_>>()?;

    let mut off = 0x78 + 8 * n;
    let mut dir: Vec<(u16, u16, usize)> = Vec::with_capacity(n);
    let mut blobs: Vec<&[u8]> = Vec::new();
    let mut maxlen = 0usize;
    let mut maxraw: BTreeMap<char, usize> = BTreeMap::new();
    let mut extra = 0xFFFF_FFFFu32;
    for (&(x, y, _, tail), b) in tiles.iter().zip(&built) {
        let Some(b) = b else {
            dir.push((x, y, off));
            continue;
        };
        if meta.tail_tile == Some((x, y)) {
            extra = (off + b.bytes.len() - tail.len()) as u32;
        }
        dir.push((x, y, off));
        blobs.push(&b.bytes);
        off += b.bytes.len();
        maxlen = maxlen.max(b.maxlen);
        for (a, v) in &b.maxraw {
            let e = maxraw.entry(*a).or_default();
            *e = (*e).max(*v);
        }
    }

    let mut d: Vec<u8> = Vec::with_capacity(off);
    d.extend_from_slice(&chart::MAGIC.to_le_bytes());
    d.extend_from_slice(&urandom(48));
    d.resize(d.len() + 16, 0); // MAC, filled in below
    d.extend_from_slice(&meta.date);
    for v in [meta.typ, meta.layer, meta.country, maxlen as u32] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    for a in ['D', 'C', 'B', 'A'] {
        d.extend_from_slice(&(*maxraw.get(&a).unwrap_or(&0) as u32).to_le_bytes());
    }
    d.extend_from_slice(&0u32.to_le_bytes()); // 0x6C unused
    d.extend_from_slice(&extra.to_le_bytes());
    d.extend_from_slice(&(n as u32).to_le_bytes());
    for (x, y, o) in &dir {
        d.extend_from_slice(&x.to_le_bytes());
        d.extend_from_slice(&y.to_le_bytes());
        d.extend_from_slice(&(*o as u32).to_le_bytes());
    }
    for b in blobs {
        d.extend_from_slice(b);
    }
    let mac = sign.mac(&d)?;
    d[0x34..0x44].copy_from_slice(&mac);
    Ok(d)
}

/// Everything needed to rebuild a file: the plaintext of all records per tile
/// plus each tile's tail.
pub fn read_all(c: &chart::Chart) -> Result<(Meta, Vec<TileContent>)> {
    let d = &c.data;
    let tiles = c.tiles();
    let mut extra_tile = None;
    let extra = chart::u32_at(d, 0x70) as usize;
    if extra != 0xFFFF_FFFF {
        for t in &tiles {
            if t.start <= extra && extra < t.end {
                extra_tile = Some((t.x, t.y));
            }
        }
    }
    let meta = Meta {
        date: d[0x44..0x4C].to_vec(),
        typ: chart::u32_at(d, 0x4C),
        layer: chart::u32_at(d, 0x50),
        country: chart::u32_at(d, 0x54),
        tail_tile: extra_tile,
    };

    let mut out = Vec::with_capacity(tiles.len());
    for t in &tiles {
        if t.start == t.end {
            out.push(TileContent { x: t.x, y: t.y, areas: None, tail: Vec::new() });
            continue;
        }
        let rk = chart::record_key(d, t.start, &c.key);
        let mut areas: Areas = BTreeMap::new();
        let mut used: HashSet<usize> = HashSet::new();
        for &(area, _, _) in AREAS.iter() {
            let g = area_grid(area);
            for s in crate::layers::slots(d, t, area) {
                let from = t.start + s.rec.rel + 8;
                let raw = chart::decode_record(&d[from..from + s.rec.len], s.rec.pltx, &rk)?;
                areas
                    .entry(area)
                    .or_default()
                    .insert((s.cx % g) * g + s.cy % g, raw);
                used.insert(s.rec.rel);
            }
        }
        // records() would also walk into ta's extra table (its first u32 looks
        // like a length), so the chain ends after the last record a slot uses.
        let mut end = t.start + HEAD;
        for r in chart::records(d, t.start, t.end) {
            if !used.contains(&r.rel) {
                break;
            }
            end = t.start + r.rel + 8 + r.len;
        }
        out.push(TileContent {
            x: t.x,
            y: t.y,
            areas: Some(areas),
            tail: d[end..t.end].to_vec(),
        });
    }
    Ok((meta, out))
}
