//! The ta layer: address search -- places, streets and house numbers
//! (TA_FORMAT.md).
//!
//! Records: D (32x32) with the street pieces, A (4x4) with one sub element per
//! place group and the street names grouped by place, and the country-wide
//! search index behind the records of the last tile (`ta_index.rs`).
//!
//! Two deliberate differences to the first implementation, both for
//! reproducibility:
//!   * the addresses, places and interpolation ways are sorted canonically --
//!     the extractor hands them over in libosmium's order, which decides
//!     `Counter` ties and float summation order and is not reproducible;
//!   * the children of a search-index node are sorted by character -- Python
//!     inserted them in the iteration order of a `set` of strings, which
//!     changes with the hash seed from run to run.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::f64::consts::PI;

use anyhow::Result;
use rayon::prelude::*;

use crate::addr::{Addr, Extract, Interp, Place, Region};
use crate::chart;
use crate::geos::{Geom, Prepared, Tree};
use crate::grid::Grid;
use crate::layers::{self, u16enc, AItem, ARec, ArrayRec, Item, Sub18, Var, D_SPEC, LAYER_TA};
use crate::osm::{sort_key, LEN_FACTOR};
use crate::pbf::SCALE;
use crate::ta_index::{self, Cell, Index, Kid, Name, Node, Res, ALL};
use crate::way::Ways;
use crate::writer::{self, Areas, Meta, TileContent};

/// D cell in 360/2^28 units
pub const CELL: i64 = 32768;
const MARGIN: i64 = 512;
/// node ids are numbered per 8x8 cell
const BCELL: f64 = (4 * CELL) as f64;
/// A cell (4x4), for the index coordinates
const ACELL: f64 = (8 * CELL) as f64;
/// most common flag value in the original
const FLAGS: u32 = 0x280;
/// no house numbers
const NONE: u32 = 0xFFFF_7FFF;
/// address -> street, in units of 360/2^28 degrees (~0.15 m north-south)
const MAX_DIST: f64 = 100.0 / 0.149;
/// pieces of one street: ends closer than 60 m
const JOIN: f64 = 60.0 / 0.149;
const MAX_ALTS: usize = 12;
/// addr:city values per street ...
const MAX_CITIES: usize = 3;
/// ... with at least this share of its addresses
const CITY_SHARE: f64 = 0.25;
/// places of a street: at least this share of its length
const ALT_SHARE: f64 = 0.1;
/// a correlation this small carries no direction
const CORR_TOL: f64 = 1e-12;
/// north-south metres per unit
const M_PER_UNIT: f64 = 2.0 * PI * 6371000.0 / (1u64 << 28) as f64;

/// osm road class -> ta class
const TA_CLASS: [(i8, u32); 8] =
    [(0, 0), (1, 1), (2, 2), (3, 3), (4, 3), (6, 4), (7, 6), (8, 8)];
/// settlement types and their radius in metres
const SETTLEMENT_R: [(&str, f64); 4] =
    [("city", 12000.0), ("town", 5000.0), ("village", 2000.0), ("hamlet", 800.0)];
const SUBURB_R: [(&str, f64); 4] =
    [("suburb", 2500.0), ("borough", 4000.0), ("quarter", 1500.0), ("neighbourhood", 800.0)];
/// kept for the index as type 2, but never assigned to a street
const OTHER_PLACES: [&str; 5] = ["locality", "isolated_dwelling", "farm", "island", "islet"];
const LANGS: [(&[u8; 3], u8); 11] = [
    (b"DAN", 1),
    (b"GER", 2),
    (b"ENG", 3),
    (b"FIN", 4),
    (b"FRE", 5),
    (b"NOR", 6),
    (b"SPA", 7),
    (b"SWE", 8),
    (b"ITA", 9),
    (b"DUT", 10),
    (b"POR", 11),
];

// ---- small helpers -------------------------------------------------------

/// A map that keeps its insertion order, like a Python dict.  Several places
/// in the compiler depend on that order (`Counter` ties, the order of the
/// index results), so it cannot be a `HashMap`.
pub struct Ordered<K, V> {
    at: HashMap<K, usize>,
    pub items: Vec<(K, V)>,
}

impl<K: std::hash::Hash + Eq + Clone, V> Default for Ordered<K, V> {
    fn default() -> Self {
        Ordered { at: HashMap::new(), items: Vec::new() }
    }
}

impl<K: std::hash::Hash + Eq + Clone, V: Default> Ordered<K, V> {
    /// The slot of `k`, appending it if it is new; the key is cloned only
    /// then, which matters when this runs once per street piece.
    pub fn slot(&mut self, k: &K) -> usize {
        if let Some(&i) = self.at.get(k) {
            return i;
        }
        self.items.push((k.clone(), V::default()));
        self.at.insert(k.clone(), self.items.len() - 1);
        self.items.len() - 1
    }

    pub fn entry(&mut self, k: K) -> &mut V {
        let i = self.slot(&k);
        &mut self.items[i].1
    }

    pub fn get(&self, k: &K) -> Option<&V> {
        self.at.get(k).map(|&i| &self.items[i].1)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// A `collections.Counter`: counts in insertion order, so that
/// `most_common` breaks ties the way Python does.
pub struct Counter<K: std::hash::Hash + Eq + Clone>(Ordered<K, u64>);

impl<K: std::hash::Hash + Eq + Clone> Default for Counter<K> {
    fn default() -> Self {
        Counter(Ordered::default())
    }
}

impl<K: std::hash::Hash + Eq + Clone> Counter<K> {
    pub fn add(&mut self, k: K, n: u64) {
        *self.0.entry(k) += n;
    }

    pub fn total(&self) -> u64 {
        self.0.items.iter().map(|(_, n)| *n).sum()
    }

    /// Descending by count, ties in insertion order (stable, like Python).
    pub fn most_common(&self) -> Vec<(&K, u64)> {
        let mut v: Vec<(&K, u64)> = self.0.items.iter().map(|(k, n)| (k, *n)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        v
    }
}

/// `'12' -> [12]`, `'12a' -> [12]`, `'12-16' -> [12, 16]`, `'Flat 3' -> []`
pub fn house_numbers(s: &str) -> Vec<i64> {
    let s = s.trim();
    let digits = |c: &[char], i: &mut usize| -> Option<i64> {
        let a = *i;
        while *i < c.len() && c[*i].is_ascii_digit() {
            *i += 1;
        }
        if *i == a {
            return None;
        }
        c[a..*i].iter().collect::<String>().parse().ok()
    };
    let ws = |c: &[char], i: &mut usize| {
        while *i < c.len() && c[*i].is_whitespace() {
            *i += 1;
        }
    };
    let c: Vec<char> = s.chars().collect();
    // r"(\d+)\s*[A-Za-z]?\s*[-–/]\s*(\d+)\s*[A-Za-z]?" over the whole string
    let range = || -> Option<Vec<i64>> {
        let mut i = 0;
        let a = digits(&c, &mut i)?;
        ws(&c, &mut i);
        if i < c.len() && c[i].is_ascii_alphabetic() {
            i += 1;
        }
        ws(&c, &mut i);
        if !matches!(c.get(i), Some('-') | Some('\u{2013}') | Some('/')) {
            return None;
        }
        i += 1;
        ws(&c, &mut i);
        let b = digits(&c, &mut i)?;
        ws(&c, &mut i);
        if i < c.len() && c[i].is_ascii_alphabetic() {
            i += 1;
        }
        if i == c.len() {
            Some(vec![a, b])
        } else {
            None
        }
    };
    if let Some(v) = range() {
        return v;
    }
    // r"(\d+)(?:\s*[A-Za-z]{0,2})?$" anchored at the start
    let mut i = 0;
    let Some(a) = digits(&c, &mut i) else { return Vec::new() };
    ws(&c, &mut i);
    let mut letters = 0;
    while i < c.len() && c[i].is_ascii_alphabetic() && letters < 2 {
        i += 1;
        letters += 1;
    }
    if i == c.len() {
        vec![a]
    } else {
        Vec::new()
    }
}

/// What a postcode is searched by: the outward code of a British one (`SW1A`),
/// otherwise the code up to the first space (`46325`, `1234` of `1234 AB`,
/// `K1A` of `K1A 0B1`).  A value without a digit is no postcode.
pub fn postcode_key(pc: &str) -> Option<String> {
    if let Some(o) = outward(pc) {
        return Some(o);
    }
    let k = pc.split_whitespace().next()?.to_uppercase();
    (k.len() <= 10
        && k.chars().any(|c| c.is_ascii_digit())
        && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
    .then_some(k)
}

/// The outward code of a British postcode: `^([A-Z]{1,2}[0-9][A-Z0-9]?) ?[0-9][A-Z]{2}$`
pub fn outward(pc: &str) -> Option<String> {
    let c: Vec<char> = pc.trim().to_uppercase().chars().collect();
    let mut i = 0;
    while i < 2 && i < c.len() && c[i].is_ascii_uppercase() {
        i += 1;
    }
    if i == 0 || !c.get(i).is_some_and(|d| d.is_ascii_digit()) {
        return None;
    }
    i += 1;
    // the optional fourth character is only part of the outward code if what
    // follows still fits the inward code
    for take in [1usize, 0] {
        let mut j = i;
        if take == 1 {
            if !c.get(j).is_some_and(|d| d.is_ascii_alphanumeric()) {
                continue;
            }
            j += 1;
        }
        let out: String = c[..j].iter().collect();
        let mut k = j;
        if c.get(k) == Some(&' ') {
            k += 1;
        }
        if c.get(k).is_some_and(|d| d.is_ascii_digit())
            && c.get(k + 1).is_some_and(|d| d.is_ascii_uppercase())
            && c.get(k + 2).is_some_and(|d| d.is_ascii_uppercase())
            && k + 3 == c.len()
        {
            return Some(out);
        }
    }
    None
}

pub fn plen(p: &[(f64, f64)]) -> f64 {
    let mut s = 0.0;
    for i in 1..p.len() {
        s += (p[i].0 - p[i - 1].0).hypot(p[i].1 - p[i - 1].1);
    }
    s
}

/// Cut a polyline at the D cell borders -> [(cell, points)].
pub fn split_cells(pts: &[(f64, f64)]) -> Vec<((i64, i64), Vec<(f64, f64)>)> {
    let c = CELL as f64;
    let mut out: Vec<((i64, i64), Vec<(f64, f64)>)> = Vec::new();
    let mut cur: Vec<(f64, f64)> = vec![pts[0]];
    let mut cell: Option<(i64, i64)> = None;
    for i in 1..pts.len() {
        let ((ax, ay), (bx, by)) = (pts[i - 1], pts[i]);
        let mut ts: Vec<f64> = Vec::new();
        for (a, b) in [(ax, bx), (ay, by)] {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            let k0 = (lo / c).floor() as i64 + 1;
            let k1 = (hi / c).floor() as i64;
            for k in k0..=k1 {
                let v = k as f64 * c;
                if lo < v && v < hi {
                    ts.push((v - a) / (b - a));
                }
            }
        }
        ts.sort_by(f64::total_cmp);
        ts.dedup();
        ts.push(1.0);
        let mut t0 = 0.0;
        for &t in &ts {
            let mx = ax + (t0 + t) / 2.0 * (bx - ax);
            let my = ay + (t0 + t) / 2.0 * (by - ay);
            let here = ((mx / c).floor() as i64, (my / c).floor() as i64);
            let p = (ax + t * (bx - ax), ay + t * (by - ay));
            if cell.is_some() && Some(here) != cell {
                let last = *cur.last().unwrap();
                out.push((cell.unwrap(), std::mem::replace(&mut cur, vec![last])));
            }
            cell = Some(here);
            cur.push(p);
            t0 = t;
        }
    }
    out.push((cell.unwrap_or((0, 0)), cur));
    out.retain(|(_, p)| p.len() >= 2);
    out
}

/// Geometry words of one piece, clamped to the cell plus its margin.
fn encode(p: &[(f64, f64)], cell: (i64, i64)) -> Vec<u32> {
    let (ox, oy) = ((cell.0 * CELL) as f64, (cell.1 * CELL) as f64);
    let mut pts: Vec<u32> = Vec::with_capacity(p.len());
    for &(x, y) in p {
        let u = ((x - ox).round_ties_even() as i64).clamp(-MARGIN, CELL + MARGIN) + MARGIN;
        let v = ((y - oy).round_ties_even() as i64).clamp(-MARGIN, CELL + MARGIN) + MARGIN;
        let w = (v as u32) << 16 | u as u32;
        if pts.last() != Some(&w) {
            pts.push(w);
        }
    }
    if pts.len() < 2 {
        pts.push(pts[0]);
    }
    let mut out = vec![(pts.len() as u32) << 16 | pts.len() as u32];
    out.extend_from_slice(&pts);
    out
}

/// One side's numbers -> `from | to << 16`, bit 15 of `from` = both parities.
pub fn side_range(nums: &mut Vec<(f64, i64)>) -> u32 {
    if nums.is_empty() {
        return NONE;
    }
    nums.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut par: Counter<i64> = Counter::default();
    for &(_, n) in nums.iter() {
        par.add(n.rem_euclid(2), 1);
    }
    let two = par.0.len() == 2;
    let counts: Vec<u64> = par.0.items.iter().map(|(_, n)| *n).collect();
    let mixed = two && (*counts.iter().min().unwrap() as f64) > 0.1 * nums.len() as f64;
    if !mixed && two {
        let keep = *par.most_common()[0].0;
        nums.retain(|&(_, n)| n.rem_euclid(2) == keep);
    }
    let lo = nums.iter().map(|&(_, n)| n).min().unwrap();
    let hi = nums.iter().map(|&(_, n)| n).max().unwrap();
    let many_t = {
        let mut ts: Vec<f64> = nums.iter().map(|&(t, _)| t).collect();
        ts.sort_by(f64::total_cmp);
        ts.dedup();
        ts.len() > 1
    };
    let up = if many_t && lo != hi {
        nums.len() < 2 || corr(nums) >= -CORR_TOL
    } else {
        true
    };
    let (a, b) = if up { (lo, hi) } else { (hi, lo) };
    (a as u32 | if mixed { 0x8000 } else { 0 }) | (b as u32) << 16
}

/// numpy's `pairwise_sum` for float64, which is what `mean` uses: eight
/// partial sums, recursively halved above 128 elements.  The means have to
/// match bit for bit, because the covariance below cancels exactly when the
/// numbers run symmetrically along the piece -- and then its sign, which
/// decides whether the range counts up or down, hangs on the last bit.
fn np_sum(a: &[f64]) -> f64 {
    let n = a.len();
    if n < 8 {
        let mut res = 0.0;
        for &v in a {
            res += v;
        }
        res
    } else if n <= 128 {
        let mut r = [a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]];
        let mut i = 8;
        while i < n - n % 8 {
            for j in 0..8 {
                r[j] += a[i + j];
            }
            i += 8;
        }
        let mut res = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
        while i < n {
            res += a[i];
            i += 1;
        }
        res
    } else {
        let n2 = n / 2 - (n / 2) % 8;
        np_sum(&a[..n2]) + np_sum(&a[n2..])
    }
}

/// `np.corrcoef(ts, ns)[0, 1]`: the covariance over the two standard
/// deviations.  Only the sign is used, and only outside [`CORR_TOL`].
fn corr(nums: &[(f64, i64)]) -> f64 {
    let n = nums.len() as f64;
    let ts: Vec<f64> = nums.iter().map(|&(t, _)| t).collect();
    let ns: Vec<f64> = nums.iter().map(|&(_, v)| v as f64).collect();
    let mt = np_sum(&ts) / n;
    let mn = np_sum(&ns) / n;
    let dt: Vec<f64> = ts.iter().map(|t| t - mt).collect();
    let dn: Vec<f64> = ns.iter().map(|v| v - mn).collect();
    let prod: Vec<f64> = (0..nums.len()).map(|i| dt[i] * dn[i]).collect();
    let vt = np_sum(&dt.iter().map(|v| v * v).collect::<Vec<f64>>());
    let vn = np_sum(&dn.iter().map(|v| v * v).collect::<Vec<f64>>());
    np_sum(&prod) / (vt.sqrt() * vn.sqrt())
}

pub fn to_latlon(x: f64, y: f64) -> (f64, f64) {
    (90.0 - y / SCALE, x / SCALE - 180.0)
}

// ---- 1. street pieces ----------------------------------------------------

/// One street piece: it lies in exactly one D cell.
pub struct Piece {
    pub name: Box<str>,
    /// ta class
    pub cls: u32,
    pub cell: (i64, i64),
    pub pts: Vec<(f64, f64)>,
    /// length in Teasi units
    pub len: u32,
    /// OSM node id at the start / end, None at a cell border
    pub na: Option<i64>,
    pub nb: Option<i64>,
}

/// The named roads of the ta classes, cut at junctions and at the cell borders.
pub fn street_pieces(
    ways: &mut Ways,
    inside: &dyn Fn((i64, i64)) -> Result<bool>,
    log: &dyn Fn(&str),
) -> Result<Vec<Piece>> {
    let nw = ways.ids.len();
    let n = ways.w.len();
    let mut cls = vec![-1i8; nw];
    let mut names: Vec<Option<Box<str>>> = vec![None; nw];
    let mut nnamed = 0;
    for (k, at) in std::mem::take(&mut ways.attr).into_iter().enumerate() {
        if at.cls >= 0 {
            cls[k] = at.cls;
            if TA_CLASS.iter().any(|&(o, _)| o == at.cls) && !at.name.is_empty() {
                names[k] = Some(at.name.trim().into());
                nnamed += 1;
            }
        }
    }

    let (x, y, w, nid) = (&ways.x, &ways.y, &ways.w, &ways.nid);
    // nodes shared by more than one road way
    let mut rn: Vec<i64> =
        (0..n).filter(|&i| cls[w[i] as usize] >= 0).map(|i| nid[i]).collect();
    rn.sort_unstable();
    let mut shared: Vec<i64> = Vec::new();
    let mut i = 0;
    while i < rn.len() {
        let mut j = i + 1;
        while j < rn.len() && rn[j] == rn[i] {
            j += 1;
        }
        if j - i > 1 {
            shared.push(rn[i]);
        }
        i = j;
    }
    drop(rn);

    let mut cut = vec![false; n];
    for k in 0..nw {
        let (a, b) = ways.rows[k];
        if b > a {
            cut[a as usize] = true;
            cut[b as usize - 1] = true;
        }
    }
    for i in 0..n {
        cut[i] = (cut[i] || shared.binary_search(&nid[i]).is_ok())
            && names[w[i] as usize].is_some();
    }
    let cp: Vec<u32> = (0..n as u32).filter(|&i| cut[i as usize]).collect();
    drop(cut);
    let mut edges: Vec<(u32, u32)> = Vec::new();
    for j in 1..cp.len() {
        if w[cp[j] as usize] == w[cp[j - 1] as usize] {
            edges.push((cp[j - 1], cp[j]));
        }
    }
    log(&format!("{} named streets, {} edges", nnamed, edges.len()));

    // metres along the ways (haversine) for the length field
    let mut cum = vec![0f64; n];
    for i in 1..n {
        let d = if w[i] != w[i - 1] {
            0.0
        } else {
            let lat0 = (90.0 - y[i - 1] / SCALE).to_radians();
            let lat1 = (90.0 - y[i] / SCALE).to_radians();
            let dlat = lat1 - lat0;
            let dlon =
                (x[i] / SCALE - 180.0).to_radians() - (x[i - 1] / SCALE - 180.0).to_radians();
            let h = (dlat / 2.0).sin().powi(2)
                + lat0.cos() * lat1.cos() * (dlon / 2.0).sin().powi(2);
            2.0 * 6371000.0 * h.sqrt().asin()
        };
        cum[i] = cum[i - 1] + d;
    }

    let mut pieces: Vec<Piece> = Vec::new();
    for &(a, b) in &edges {
        let (a, b) = (a as usize, b as usize);
        let k = w[a] as usize;
        let pts: Vec<(f64, f64)> = (a..=b).map(|i| (x[i], y[i])).collect();
        let metres = cum[b] - cum[a];
        let planar = plen(&pts);
        let parts = split_cells(&pts);
        let last = parts.len() - 1;
        for (j, (cell, p)) in parts.into_iter().enumerate() {
            if !inside(cell)? {
                continue;
            }
            let ln = if planar != 0.0 { metres * plen(&p) / planar } else { 0.0 };
            pieces.push(Piece {
                name: names[k].clone().unwrap(),
                cls: TA_CLASS.iter().find(|&&(o, _)| o == cls[k]).unwrap().1,
                cell,
                len: (ln * LEN_FACTOR).round_ties_even() as u32,
                na: if j == 0 { Some(nid[a]) } else { None },
                nb: if j == last { Some(nid[b]) } else { None },
                pts: p,
            });
        }
    }
    log(&format!("{} pieces", pieces.len()));
    Ok(pieces)
}

// ---- 2. addresses -> pieces ---------------------------------------------

/// One house number on a piece.
pub struct Hit {
    /// position along the piece, 0..1
    pub t: f64,
    pub right: bool,
    pub n: i64,
    pub city: Option<Box<str>>,
    /// the postcode as it is searched for, see [`postcode_key`]
    pub pc: Option<Box<str>>,
}

struct Num<'a> {
    x: f64,
    y: f64,
    n: i64,
    street: &'a str,
    city: Option<Box<str>>,
    pc: Option<Box<str>>,
}

/// Match every house number to the nearest piece of the same name.
pub fn match_addresses(
    pieces: &[Piece],
    addr: &[Addr],
    interp: &[Interp],
    log: &dyn Fn(&str),
) -> Ordered<u32, Vec<Hit>> {
    let mut pts: Vec<Num> = Vec::new();
    for a in addr {
        let Some(street) = a.tags.street.as_deref().filter(|s| !s.is_empty()) else { continue };
        let pc = a.tags.postcode.as_deref().and_then(postcode_key);
        for n in house_numbers(a.tags.hn.as_deref().unwrap_or("")) {
            if 0 < n && n < 0x7FFF {
                pts.push(Num {
                    x: a.x,
                    y: a.y,
                    n,
                    street: street.trim(),
                    city: a.tags.city.as_deref().map(Into::into),
                    pc: pc.as_deref().map(Into::into),
                });
            }
        }
    }
    for it in interp {
        let step = match &*it.kind {
            "odd" | "even" => 2i64,
            "all" => 1,
            _ => continue,
        };
        let Some(street) = it.street.as_deref().filter(|s| !s.is_empty()) else { continue };
        let ns: Vec<(f64, f64, Vec<i64>)> = it
            .pts
            .iter()
            .map(|(x, y, h, _)| (*x, *y, house_numbers(h.as_deref().unwrap_or(""))))
            .collect();
        if ns.len() < 2 || ns[0].2.len() != 1 || ns[ns.len() - 1].2.len() != 1 {
            continue;
        }
        let (n0, n1) = (ns[0].2[0], ns[ns.len() - 1].2[0]);
        if n0 == n1 || (n1 - n0).abs() > 400 || (n1 - n0).rem_euclid(step) != 0 {
            continue;
        }
        let line: Vec<(f64, f64)> = ns.iter().map(|&(x, y, _)| (x, y)).collect();
        let total = plen(&line);
        let span = (n1 - n0).abs();
        for i in 1..span / step {
            let n = n0 + i * step * if n1 > n0 { 1 } else { -1 };
            let mut d = total * (i * step) as f64 / span as f64;
            for q in 1..line.len() {
                let ((ax, ay), (bx, by)) = (line[q - 1], line[q]);
                let s = (bx - ax).hypot(by - ay);
                if d <= s {
                    let f = if s != 0.0 { d / s } else { 0.0 };
                    pts.push(Num {
                        x: ax + f * (bx - ax),
                        y: ay + f * (by - ay),
                        n,
                        street: street.trim(),
                        city: None,
                        pc: None,
                    });
                    break;
                }
                d -= s;
            }
        }
    }
    log(&format!(
        "{} house numbers ({} addresses, {} interpolations)",
        pts.len(),
        addr.len(),
        interp.len()
    ));

    // the pieces by name (casefolded, as a number) and cell; a house number
    // looks at its own cell and the eight around it
    let mut ids: HashMap<String, u32> = HashMap::new();
    let mut id = |s: &str| {
        let n = ids.len() as u32;
        *ids.entry(crate::osm::casefold(s)).or_insert(n)
    };
    let mut by_cell: HashMap<(u32, (i64, i64)), Vec<u32>> = HashMap::new();
    for (i, p) in pieces.iter().enumerate() {
        by_cell
            .entry((id(&p.name), p.cell))
            .or_default()
            .push(i as u32);
    }
    let mut groups: Ordered<(u32, (i64, i64)), Vec<u32>> = Ordered::default();
    for (j, q) in pts.iter().enumerate() {
        let cell = (
            (q.x / CELL as f64).floor() as i64,
            (q.y / CELL as f64).floor() as i64,
        );
        groups.entry((id(q.street), cell)).push(j as u32);
    }
    drop(ids);

    let mut res: Ordered<u32, Vec<Hit>> = Ordered::default();
    let mut matched = 0usize;
    let mut cand: Vec<u32> = Vec::new();
    for ((name, (cx, cy)), js) in &groups.items {
        // in piece order, which decides between equally near pieces
        cand.clear();
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(v) = by_cell.get(&(*name, (cx + dx, cy + dy))) {
                    cand.extend_from_slice(v);
                }
            }
        }
        if cand.is_empty() {
            continue;
        }
        cand.sort_unstable();
        // the segments of all candidate pieces, with their offset along the piece
        let mut seg: Vec<(f64, f64, f64, f64)> = Vec::new();
        let mut owner: Vec<u32> = Vec::new();
        let mut before: Vec<f64> = Vec::new();
        let mut total: HashMap<u32, f64> = HashMap::new();
        for &i in &cand {
            let p = &pieces[i as usize].pts;
            let mut acc = 0.0;
            for q in 1..p.len() {
                seg.push((p[q - 1].0, p[q - 1].1, p[q].0, p[q].1));
                owner.push(i);
                before.push(acc);
                acc += (p[q].0 - p[q - 1].0).hypot(p[q].1 - p[q - 1].1);
            }
            total.insert(i, acc);
        }
        for &j in js {
            let q = &pts[j as usize];
            let mut best = usize::MAX;
            let mut bd = f64::INFINITY;
            let mut bt = 0.0;
            for (s, &(ax, ay, bx, by)) in seg.iter().enumerate() {
                let (dx, dy) = (bx - ax, by - ay);
                let l2 = (dx * dx + dy * dy).max(1e-9);
                let t = (((q.x - ax) * dx + (q.y - ay) * dy) / l2).clamp(0.0, 1.0);
                let d = (q.x - ax - t * dx).hypot(q.y - ay - t * dy);
                if d < bd {
                    bd = d;
                    best = s;
                    bt = t;
                }
            }
            if best == usize::MAX || bd > MAX_DIST {
                continue;
            }
            let (ax, ay, bx, by) = seg[best];
            let (dx, dy) = (bx - ax, by - ay);
            let l2 = (dx * dx + dy * dy).max(1e-9);
            let i = owner[best];
            let cross = dx * (q.y - ay) - dy * (q.x - ax);
            let tot = total[&i];
            let tt = if tot != 0.0 { (before[best] + bt * l2.sqrt()) / tot } else { 0.5 };
            res.entry(i).push(Hit {
                t: tt,
                right: cross > 0.0,
                n: q.n,
                city: q.city.clone(),
                pc: q.pc.clone(),
            });
            matched += 1;
        }
    }
    log(&format!("{} house numbers matched to {} pieces", matched, res.len()));
    res
}

// ---- 3. places -----------------------------------------------------------

/// One administrative level of the country as GEOS polygons, to look up
/// which area a point lies in.
struct Admin {
    names: Vec<Box<str>>,
    geoms: Vec<Geom>,
}

impl Admin {
    fn new(rs: &[Region], level: u8) -> Result<Admin> {
        let (mut names, mut geoms) = (Vec::new(), Vec::new());
        for r in rs.iter().filter(|r| r.level == level) {
            // even-odd over the rings: a hole (Bremen in Niedersachsen) is a
            // second ring around the same area
            let mut g: Option<Geom> = None;
            for ring in r.rings.iter().filter(|r| r.len() >= 4) {
                let p = Geom::polygon(ring, &[])?.valid()?;
                g = Some(match g {
                    None => p,
                    Some(a) => a.sym_difference(&p)?,
                });
            }
            if let Some(g) = g.filter(|g| !g.is_empty()) {
                names.push(r.name.clone());
                geoms.push(g);
            }
        }
        Ok(Admin { names, geoms })
    }
}

struct Lookup<'a> {
    names: &'a [Box<str>],
    prep: Vec<Prepared<'a>>,
    tree: Tree<'a>,
}

impl<'a> Lookup<'a> {
    fn new(a: &'a Admin) -> Result<Lookup<'a>> {
        let prep = a.geoms.iter().map(Prepared::new).collect::<Result<_>>()?;
        Ok(Lookup { names: &a.names, prep, tree: Tree::new(&a.geoms)? })
    }

    fn at(&self, x: f64, y: f64) -> Option<&'a str> {
        if self.names.is_empty() {
            return None;
        }
        let pt = Geom::rect(x, y, x + 1e-3, y + 1e-3).ok()?;
        let mut hits = self.tree.query(&pt);
        hits.sort_unstable();
        hits.into_iter().find(|&i| self.prep[i].intersects(&pt)).map(|i| &*self.names[i])
    }
}

/// Places of the same name and region farther apart than this are different
/// places and get the county (and the municipality) as well.
const SAME_PLACE: f64 = 5000.0 / 0.149;

/// The names places are shown and searched under, for every (name, point):
/// `Borken (Nordrhein-Westfalen)`, and where the region leaves several places
/// of one name, `Berg (Bayern, Landkreis Passau)` or, if the county does not
/// tell them apart either, `Berg (Bayern, Landkreis Passau, Fürstenzell)`.  A
/// name that already ends in parentheses (`Borken (Hessen)`) stays as it is.
fn place_names(items: &[(Box<str>, f64, f64)], regions: &[Region]) -> Result<Vec<Box<str>>> {
    let admin: Vec<Admin> =
        crate::addr::ADMIN_LEVELS.iter().map(|&l| Admin::new(regions, l)).collect::<Result<_>>()?;
    let look: Vec<Lookup> = admin.iter().map(Lookup::new).collect::<Result<_>>()?;
    let region: Vec<Option<&str>> = items.iter().map(|(_, x, y)| look[0].at(*x, *y)).collect();
    let mut extra: Vec<Vec<&str>> = vec![Vec::new(); items.len()];

    let mut groups: HashMap<(&str, Option<&str>), Vec<usize>> = HashMap::new();
    for (i, (name, _, _)) in items.iter().enumerate() {
        if !name.ends_with(')') {
            groups.entry((&**name, region[i])).or_default().push(i);
        }
    }
    // one cluster per place: members closer than SAME_PLACE to its first point
    let clusters = |members: &[usize]| -> Vec<usize> {
        let mut first: Vec<usize> = Vec::new();
        members
            .iter()
            .map(|&i| {
                let (_, x, y) = items[i];
                let near = first.iter().position(|&f| {
                    let (_, fx, fy) = items[f];
                    (fx - x).hypot(fy - y) < SAME_PLACE
                });
                near.unwrap_or_else(|| {
                    first.push(i);
                    first.len() - 1
                })
            })
            .collect()
    };
    let mut keys: Vec<_> = groups.keys().copied().collect();
    keys.sort_unstable();
    for key in keys {
        let members = &groups[&key];
        let cl = clusters(members);
        if cl.iter().all(|&c| c == 0) {
            continue;
        }
        // the levels below the region, per member; then the first choice of
        // levels that tells the places apart, or the one that comes closest
        let below: Vec<Vec<Option<&str>>> = members
            .iter()
            .map(|&i| {
                let (name, x, y) = &items[i];
                look[1..]
                    .iter()
                    .map(|lk| lk.at(*x, *y).filter(|n| *n != &**name && Some(*n) != region[i]))
                    .collect()
            })
            .collect();
        let clashes = |levels: &[usize]| -> usize {
            let mut seen: HashMap<Vec<Option<&str>>, usize> = HashMap::new();
            let mut n = 0;
            for (k, b) in below.iter().enumerate() {
                let label: Vec<Option<&str>> = levels.iter().map(|&l| b[l]).collect();
                if label.iter().all(|l| l.is_none()) {
                    n += 1;
                    continue;
                }
                n += (*seen.entry(label).or_insert(cl[k]) != cl[k]) as usize;
            }
            n
        };
        // indices into ADMIN_LEVELS[1..] = [5, 6, 7, 8]
        const CHOICES: [&[usize]; 8] = [&[1], &[2], &[3], &[0], &[1, 3], &[2, 3], &[0, 3], &[1, 2]];
        let best = CHOICES.iter().min_by_key(|c| clashes(c)).unwrap();
        for (k, &i) in members.iter().enumerate() {
            for &l in best.iter() {
                if let Some(n) = below[k][l] {
                    if extra[i].last() != Some(&n) {
                        extra[i].push(n);
                    }
                }
            }
        }
    }
    Ok(items
        .iter()
        .enumerate()
        .map(|(i, (name, _, _))| {
            let parts: Vec<&str> = region[i].into_iter().chain(extra[i].iter().copied()).collect();
            if name.ends_with(')') || parts.is_empty() {
                name.clone()
            } else {
                format!("{} ({})", name, parts.join(", ")).into()
            }
        })
        .collect())
}

/// The coordinates of a place node, as a hashable key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Anchor(u64, u64);

impl Anchor {
    fn of(x: f64, y: f64) -> Anchor {
        Anchor(x.to_bits(), y.to_bits())
    }

    fn xy(&self) -> (f64, f64) {
        (f64::from_bits(self.0), f64::from_bits(self.1))
    }
}

/// One place name a street can be looked up under, with the place node it
/// points at.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Alt {
    pub name: Box<str>,
    pub anchor: Option<Anchor>,
}

pub type Alts = Vec<Alt>;

type Cand = (f64, f64, Box<str>);

/// Per piece the place alternatives; the first is the full name "Town, Suburb".
fn assign_places(pieces: &[Piece], hn: &Ordered<u32, Vec<Hit>>, places: &[Place]) -> Vec<Alts> {
    let c = 54f64.to_radians().cos(); // x scale of the trees
    let mid: Vec<(f64, f64)> = pieces.iter().map(|p| p.pts[p.pts.len() / 2]).collect();
    let lat_scale: Vec<f64> =
        mid.iter().map(|&(_, y)| (90.0 - y / SCALE).to_radians().cos()).collect();
    let q: Vec<(f64, f64)> = mid.iter().map(|&(x, y)| (x * c, y)).collect();

    // the place with the smallest distance / radius < 1 per piece
    let best = |radii: &[(&str, f64)]| -> Vec<Option<Cand>> {
        let mut cands: Vec<Cand> = Vec::new();
        let mut score = vec![f64::INFINITY; pieces.len()];
        let mut which = vec![usize::MAX; pieces.len()];
        for &(typ, rm) in radii {
            let cand: Vec<Cand> = places
                .iter()
                .filter(|p| p.tags.place.as_deref() == Some(typ))
                .map(|p| (p.x, p.y, p.tags.name.clone().unwrap_or_default()))
                .collect();
            if cand.is_empty() {
                continue;
            }
            let base = cands.len();
            let arr: Vec<(f64, f64)> = cand.iter().map(|&(x, y, _)| (x, y)).collect();
            cands.extend(cand);
            let r = rm / M_PER_UNIT;
            let scaled: Vec<(f64, f64)> = arr.iter().map(|&(x, y)| (x * c, y)).collect();
            let g = Grid::new(&scaled);
            let near: Vec<Vec<(f64, u32)>> =
                q.par_iter().map(|&qq| g.knn(qq, 8, 1.2 * r)).collect();
            for kk in 0..8 {
                for row in 0..pieces.len() {
                    let Some(&(_, j)) = near[row].get(kk) else { continue };
                    let j = j as usize;
                    let sc = ((arr[j].0 - mid[row].0) * lat_scale[row])
                        .hypot(arr[j].1 - mid[row].1)
                        / r;
                    if sc < 1.0 && sc < score[row] {
                        score[row] = sc;
                        which[row] = base + j;
                    }
                }
            }
        }
        which.iter().map(|&w| cands.get(w).cloned()).collect()
    };
    let (town, sub) = (best(&SETTLEMENT_R), best(&SUBURB_R));

    // the addr:city values (postal towns) per street name and 4x4 cell
    let mut city: Ordered<(Box<str>, i64, i64), Counter<Box<str>>> = Ordered::default();
    for (i, lst) in &hn.items {
        let p = &pieces[*i as usize];
        let key = (p.name.clone(), p.cell.0.div_euclid(8), p.cell.1.div_euclid(8));
        for h in lst {
            if let Some(cty) = &h.city {
                city.entry(key.clone()).add(cty.trim().into(), 1);
            }
        }
    }
    // a postal town is anchored at the nearest place node of that name
    let mut by_name: HashMap<Box<str>, Vec<(f64, f64)>> = HashMap::new();
    for p in places {
        if let Some(n) = &p.tags.name {
            by_name.entry((&**n).into()).or_default().push((p.x, p.y));
        }
    }
    let trees: RefCell<HashMap<Box<str>, Grid>> = RefCell::new(HashMap::new());
    let anchor_of = |name: &str, x: f64, y: f64| -> Option<Anchor> {
        let pts = by_name.get(name)?;
        let mut t = trees.borrow_mut();
        let g = t.entry(name.into()).or_insert_with(|| {
            Grid::new(&pts.iter().map(|&(px, py)| (px * c, py)).collect::<Vec<_>>())
        });
        let (d, j) = g.nearest((x * c, y))?;
        (d * M_PER_UNIT < 30000.0).then(|| Anchor::of(pts[j as usize].0, pts[j as usize].1))
    };

    let mut out: Vec<Alts> = Vec::with_capacity(pieces.len());
    for (i, p) in pieces.iter().enumerate() {
        let (t, s) = (&town[i], &sub[i]);
        let mut alts: Alts = Vec::new();
        if let (Some(t), Some(s)) = (t, s) {
            if s.2 != t.2 {
                alts.push(Alt {
                    name: format!("{}, {}", t.2, s.2).into(),
                    anchor: Some(Anchor::of(s.0, s.1)),
                });
            }
        }
        if let Some(t) = t {
            alts.push(Alt { name: t.2.clone(), anchor: Some(Anchor::of(t.0, t.1)) });
        } else if let Some(s) = s {
            alts.push(Alt { name: s.2.clone(), anchor: Some(Anchor::of(s.0, s.1)) });
        }
        let key = (p.name.clone(), p.cell.0.div_euclid(8), p.cell.1.div_euclid(8));
        if let Some(cc) = city.get(&key) {
            let total = cc.total();
            for (name, n) in cc.most_common().into_iter().take(MAX_CITIES) {
                if n as f64 >= CITY_SHARE * total as f64
                    && !alts.iter().any(|a| a.name == *name)
                {
                    let anchor = anchor_of(name, mid[i].0, mid[i].1);
                    alts.push(Alt { name: name.clone(), anchor });
                }
            }
        }
        out.push(alts);
    }
    out
}

/// Pieces of the same name in a 4x4 cell whose ends are closer than [`JOIN`]
/// are one street and share their place alternatives.
fn merge_streets(pieces: &[Piece], grp: Vec<Alts>, log: &dyn Fn(&str)) -> Vec<Alts> {
    let mut by: Ordered<(Box<str>, i64, i64), Vec<u32>> = Ordered::default();
    for (i, p) in pieces.iter().enumerate() {
        by.entry((p.name.clone(), p.cell.0.div_euclid(8), p.cell.1.div_euclid(8)))
            .push(i as u32);
    }
    let mut out = grp.clone();
    let mut nstreets = 0usize;
    for (_, idx) in &by.items {
        let mut parent: Vec<usize> = (0..idx.len()).collect();
        fn root(parent: &mut Vec<usize>, mut k: usize) -> usize {
            while parent[k] != k {
                parent[k] = parent[parent[k]];
                k = parent[k];
            }
            k
        }
        if idx.len() > 1 {
            let ends: Vec<(f64, f64)> = idx
                .iter()
                .flat_map(|&i| {
                    let p = &pieces[i as usize].pts;
                    [p[0], p[p.len() - 1]]
                })
                .collect();
            for (a, b) in Grid::new(&ends).pairs(JOIN) {
                let (ra, rb) = (
                    root(&mut parent, a as usize / 2),
                    root(&mut parent, b as usize / 2),
                );
                if ra != rb {
                    parent[ra] = rb;
                }
            }
        }
        let mut comp: Ordered<usize, Vec<u32>> = Ordered::default();
        for (k, &i) in idx.iter().enumerate() {
            let r = root(&mut parent, k);
            comp.entry(r).push(i);
        }
        for (_, members) in &comp.items {
            nstreets += 1;
            let mut w: Counter<Alt> = Counter::default();
            let mut total = 0u64;
            for &i in members {
                let len = pieces[i as usize].len as u64 + 1;
                total += len;
                for alt in &grp[i as usize] {
                    w.add(alt.clone(), len);
                }
            }
            // the main "Town, Suburb" only (every suburb set would be its own
            // group and blow up the A records), plus the towns and postal towns
            // with at least ALT_SHARE of the street; canonical order, so equal
            // sets give one group
            let mut names: BTreeSet<Box<str>> = BTreeSet::new();
            let mut alts: Alts = Vec::new();
            let mut subname: Option<Alt> = None;
            for (k, (alt, n)) in w.most_common().into_iter().enumerate() {
                if names.contains(&alt.name)
                    || alts.len() >= MAX_ALTS
                    || (k > 0 && (n as f64) < ALT_SHARE * total as f64)
                {
                    continue;
                }
                if alt.name.contains(", ") {
                    if subname.is_none() {
                        subname = Some(alt.clone());
                        names.insert(alt.name.clone());
                    }
                    continue;
                }
                names.insert(alt.name.clone());
                alts.push(alt.clone());
            }
            alts.sort_by(|a, b| a.name.cmp(&b.name));
            let all: Alts = subname.into_iter().chain(alts).collect();
            for &i in members {
                out[i as usize] = all.clone();
            }
        }
    }
    log(&format!("{} streets", nstreets));
    out
}

// ---- 4. records and index ------------------------------------------------

/// One place group of an A record: the '|' joined names, (lat, lon) and the
/// street names in the group.
type Group = (String, (f64, f64), Vec<Box<str>>);

/// The A record of one 4x4 cell plus the blk5 offset per (group, street name).
fn build_a(
    cx: i64,
    cy: i64,
    groups: &[Group],
    country: &str,
) -> (Vec<u8>, Vec<HashMap<Box<str>, u32>>) {
    let mut woff: HashMap<Box<str>, u32> = HashMap::new();
    let mut blk6: Vec<u8> = Vec::new();
    let mut blk5: Vec<u32> = Vec::new();
    let mut index: Vec<HashMap<Box<str>, u32>> = Vec::with_capacity(groups.len());
    let mut s18: Vec<Sub18> = Vec::new();
    for (gi, (alts, (lat, lon), names)) in groups.iter().enumerate() {
        let _ = gi;
        let start = 4 * blk5.len() as u32;
        let mut sorted: Vec<(String, &Box<str>)> = {
            let mut uniq: Vec<&Box<str>> = names.iter().collect();
            uniq.sort();
            uniq.dedup();
            uniq.into_iter().map(|n| (sort_key(n), n)).collect()
        };
        sorted.sort();
        let count = sorted.len() as u32;
        index.push(HashMap::new());
        for (_, name) in &sorted {
            let ws: Vec<&str> = name.split_whitespace().collect();
            index[gi].insert((*name).clone(), 4 * blk5.len() as u32);
            for (j, w) in ws.iter().enumerate() {
                let off = *woff.entry((*w).into()).or_insert_with(|| {
                    let at = blk6.len() as u32;
                    let enc: Vec<u16> = w.encode_utf16().collect();
                    blk6.extend_from_slice(&(enc.len() as u16).to_le_bytes());
                    for u in &enc {
                        blk6.extend_from_slice(&u.to_le_bytes());
                    }
                    at
                });
                blk5.push((if j == 0 { ws.len() as u32 } else { 0 }) << 24 | off);
            }
        }
        let st = u16enc(alts);
        let fl = if alts.is_empty() {
            (0, 0)
        } else {
            ((*lat as f32).to_bits(), (*lon as f32).to_bits())
        };
        s18.push(Sub18 {
            s: vec![0, st.len() as u32, start, count, fl.0, fl.1],
            text: st,
        });
    }
    let cname = u16enc(country);
    let mut rec = ARec {
        hdr: vec![cx as u32, cy as u32, 0, 1, 0, 0, 0],
        items: vec![AItem {
            s: vec![0, cname.len() as u32, 1, s18.len() as u32, 0, 0, 0],
            name: cname,
            s18,
            s10: Vec::new(),
        }],
        blk5: Vec::new(),
        blk6: Vec::new(),
    };
    let head = layers::build_a(&rec).len() as u32;
    let mut b5 = Vec::with_capacity(4 * blk5.len());
    for v in &blk5 {
        b5.extend_from_slice(&v.to_le_bytes());
    }
    rec.hdr[5] = head;
    rec.hdr[6] = head + b5.len() as u32;
    rec.blk5 = b5;
    rec.blk6 = blk6;
    (layers::build_a(&rec), index)
}

/// One piece as it goes into a D record.
struct DPiece {
    off: u32,
    left: Vec<(f64, i64)>,
    right: Vec<(f64, i64)>,
    ia: u32,
    ib: u32,
    cls: u32,
    len: u32,
    pts: Vec<(f64, f64)>,
}

fn d_record(cell: (i64, i64), pcs: &mut [DPiece]) -> Vec<u8> {
    let mut items: Vec<Item> = Vec::with_capacity(pcs.len());
    for p in pcs.iter_mut() {
        let geo = encode(&p.pts, cell);
        items.push(Item {
            s: vec![
                p.off,
                p.off,
                side_range(&mut p.left),
                side_range(&mut p.right),
                p.ia,
                p.ib,
                FLAGS,
                p.cls << 27 | p.len.min(0xFFFFFF),
                geo.len() as u32,
                0,
            ],
            v: vec![Var::U32(geo)],
        });
    }
    items.sort_by_key(|it| (it.s[7] >> 27, it.s[0]));
    let n = items.len() as u32;
    let hdr = vec![n * 0x28, 0, 0, n, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let arrays = vec![items, Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    layers::build_arrays(&ArrayRec { hdr, arrays }, &D_SPEC)
}

/// The search keys of a place name: every word and every multi-word part.
pub fn keys(name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // the region in parentheses at the end is shown, not searched
    let name = match name.rfind(" (") {
        Some(i) if name.ends_with(')') => &name[..i],
        _ => name,
    };
    for comp in name.split(", ") {
        let f = ta_index::fold(comp).replace('\'', "").replace('\u{2019}', "");
        let cleaned: String = f
            .chars()
            .map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() { c } else { ' ' })
            .collect();
        let ws: Vec<&str> = cleaned.split_whitespace().collect();
        for w in &ws {
            out.push((*w).to_string());
        }
        if ws.len() > 1 {
            out.push(ws.join(" "));
        }
    }
    out.sort();
    out.dedup();
    out
}

fn insert_key(root: &mut Node, key: &str, r: &Res) {
    let mut nd = root;
    for ch in key.chars() {
        let c = ch as u16;
        let mut pos = nd.kids.iter().position(|k| k.ch == c);
        if pos.is_none() {
            nd.kids.push(Kid { ch: c, mask: ALL, node: Node { kids: Vec::new(), res: Vec::new() } });
            pos = Some(nd.kids.len() - 1);
        }
        nd = &mut nd.kids[pos.unwrap()].node;
    }
    nd.res.push(r.clone());
}

/// Sort every node's children by character, so the table is reproducible.
fn sort_kids(nd: &mut Node) {
    nd.kids.sort_by_key(|k| k.ch);
    for k in nd.kids.iter_mut() {
        sort_kids(&mut k.node);
    }
}

/// `np.mean(v, axis=0)` of a list of points: numpy accumulates the rows
/// sequentially into a two-element buffer.
fn mean_xy(v: &[(f64, f64)]) -> (f64, f64) {
    let (mut sx, mut sy) = (0.0, 0.0);
    for &(x, y) in v {
        sx += x;
        sy += y;
    }
    (sx / v.len() as f64, sy / v.len() as f64)
}

// ---- Python's repr, for the one tie break that needs it ------------------

fn py_repr_f64(v: f64) -> String {
    let s = format!("{}", v);
    if s.contains('.') || s.contains('e') || s.contains("inf") || s.contains("NaN") {
        s
    } else {
        format!("{}.0", s)
    }
}

fn py_repr_str(s: &str) -> String {
    let q = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(q);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == q => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push(q);
    out
}

/// `repr()` of the alternatives tuple, which Python uses as the last tie break
/// when two groups of a cell carry the same names but different place nodes.
fn py_repr_alts(a: &Alts) -> String {
    let inner: Vec<String> = a
        .iter()
        .map(|alt| {
            let an = match &alt.anchor {
                None => "None".to_string(),
                Some(p) => {
                    let (x, y) = p.xy();
                    format!("({}, {})", py_repr_f64(x), py_repr_f64(y))
                }
            };
            format!("({}, {})", py_repr_str(&alt.name), an)
        })
        .collect();
    if inner.len() == 1 {
        format!("({},)", inner[0])
    } else {
        format!("({})", inner.join(", "))
    }
}

/// Key of a street piece's start or end node; pieces cut at a cell border get
/// a key of their own.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum NKey {
    Node(i64),
    Border(u32, u8),
}

// ---- main build ----------------------------------------------------------

/// Canonical order of the extractor's output, see the module comment.
fn sort_extract(ad: &mut Extract) {
    fn s(o: &Option<Box<str>>) -> &str {
        o.as_deref().unwrap_or("")
    }
    ad.addr.sort_by(|a, b| {
        (a.x, a.y).partial_cmp(&(b.x, b.y)).unwrap().then_with(|| {
            (
                s(&a.tags.hn),
                s(&a.tags.street),
                s(&a.tags.postcode),
                s(&a.tags.city),
                s(&a.tags.place),
                s(&a.tags.suburb),
            )
                .cmp(&(
                    s(&b.tags.hn),
                    s(&b.tags.street),
                    s(&b.tags.postcode),
                    s(&b.tags.city),
                    s(&b.tags.place),
                    s(&b.tags.suburb),
                ))
        })
    });
    ad.places.sort_by(|a, b| {
        (a.x, a.y)
            .partial_cmp(&(b.x, b.y))
            .unwrap()
            .then_with(|| (s(&a.tags.place), s(&a.tags.name)).cmp(&(s(&b.tags.place), s(&b.tags.name))))
    });
    let ikey = |i: &Interp| {
        let f = i.pts.first().unwrap_or(&(0.0, 0.0, None, None));
        let l = i.pts.last().unwrap_or(&(0.0, 0.0, None, None));
        (f.0, f.1, l.0, l.1)
    };
    ad.interp.sort_by(|a, b| {
        ikey(a).partial_cmp(&ikey(b)).unwrap().then_with(|| {
            (&a.kind, s(&a.street), a.pts.len()).cmp(&(&b.kind, s(&b.street), b.pts.len()))
        })
    });
}

/// `p`: the area polygon in 360/2^28 units; cells that do not touch it plus
/// one D cell are dropped.
#[allow(clippy::too_many_arguments)]
pub fn build(
    mut ways: Ways,
    mut ad: Extract,
    p: &Geom,
    date: &[u8],
    country: u32,
    cname: &str,
    sign: &chart::Signer,
    log: &dyn Fn(&str),
) -> Result<Option<Vec<u8>>> {
    let t0 = std::time::Instant::now();
    let since = |s: String| format!("{} ({:.0} s)", s, t0.elapsed().as_secs_f32());
    let pb = p.buffer(CELL as f64)?;
    let prep = crate::geos::Prepared::new(&pb)?;
    let memo: RefCell<HashMap<(i64, i64), bool>> = RefCell::new(HashMap::new());
    let inside = |cell: (i64, i64)| -> Result<bool> {
        if let Some(&v) = memo.borrow().get(&cell) {
            return Ok(v);
        }
        let (x, y) = ((cell.0 * CELL) as f64, (cell.1 * CELL) as f64);
        let v = prep.intersects(&Geom::rect(x, y, x + CELL as f64, y + CELL as f64)?);
        memo.borrow_mut().insert(cell, v);
        Ok(v)
    };

    // the ways and the addresses are not needed past the matching, and they
    // are the biggest part of the memory
    let mut pieces = street_pieces(&mut ways, &inside, &|s| log(&since(s.to_string())))?;
    drop(ways);
    sort_extract(&mut ad);
    let hn = match_addresses(&pieces, &ad.addr, &ad.interp, &|s| {
        log(&since(s.to_string()))
    });
    let places: Vec<Place> = ad
        .places
        .into_iter()
        .filter(|p| {
            let t = p.tags.place.as_deref().unwrap_or("");
            SETTLEMENT_R.iter().any(|&(k, _)| k == t)
                || SUBURB_R.iter().any(|&(k, _)| k == t)
                || OTHER_PLACES.contains(&t)
        })
        .collect();
    ad.addr = Vec::new();
    ad.interp = Vec::new();
    let grp = merge_streets(&pieces, assign_places(&pieces, &hn, &places), &|s| {
        log(&since(s.to_string()))
    });
    // every (name, point) a place is shown under: the place node, or for an
    // alternative without one (a postal town far from it) the 4x4 cell of the
    // street, so that one street keeps one name
    let point = |a: &Alt, p: &Piece| {
        let ac = (p.cell.0.div_euclid(8), p.cell.1.div_euclid(8));
        a.anchor.map(|a| a.xy()).unwrap_or(((ac.0 as f64 + 0.5) * ACELL, (ac.1 as f64 + 0.5) * ACELL))
    };
    let mut item_of: HashMap<(Box<str>, u64, u64), usize> = HashMap::new();
    let mut items: Vec<(Box<str>, f64, f64)> = Vec::new();
    let mut item = |name: &Box<str>, (x, y): (f64, f64)| -> usize {
        *item_of.entry((name.clone(), x.to_bits(), y.to_bits())).or_insert_with(|| {
            items.push((name.clone(), x, y));
            items.len() - 1
        })
    };
    let alt_items: Vec<Vec<usize>> =
        grp.iter().zip(&pieces).map(|(alts, p)| alts.iter().map(|a| item(&a.name, point(a, p))).collect()).collect();
    let place_items: Vec<usize> = places
        .iter()
        .map(|pl| item(&pl.tags.name.clone().unwrap_or_default(), (pl.x, pl.y)))
        .collect();
    let named = place_names(&items, &ad.regions)?;
    let n_regions = ad.regions.iter().filter(|r| r.level == crate::addr::ADMIN_LEVELS[0]).count();
    let grp: Vec<Alts> = grp
        .into_iter()
        .zip(&alt_items)
        .map(|(alts, ix)| {
            alts.into_iter().zip(ix).map(|(a, &i)| Alt { name: named[i].clone(), anchor: a.anchor }).collect()
        })
        .collect();
    log(&since(format!(
        "places assigned: {} regions, {} areas below them",
        n_regions,
        ad.regions.len() - n_regions
    )));

    // A: the place groups of each 4x4 cell
    let mut acells: Ordered<(i64, i64), Ordered<Alts, Vec<Box<str>>>> = Ordered::default();
    let mut slots: Vec<(u32, u32)> = Vec::with_capacity(pieces.len());
    for (i, p) in pieces.iter().enumerate() {
        let ac = (p.cell.0.div_euclid(8), p.cell.1.div_euclid(8));
        let a = acells.slot(&ac);
        let g = acells.items[a].1.slot(&grp[i]);
        acells.items[a].1.items[g].1.push(p.name.clone());
        slots.push((a as u32, g as u32));
    }
    let mut arec: BTreeMap<(i64, i64), Vec<u8>> = BTreeMap::new();
    let mut aindex: Vec<Vec<HashMap<Box<str>, u32>>> = Vec::with_capacity(acells.len());
    // group number per (A cell slot, group slot), and the order they were made in
    let mut gi_of: Vec<Vec<usize>> = Vec::with_capacity(acells.len());
    let mut gorder: Vec<((i64, i64), Alts)> = Vec::new();
    for (ac, gs) in &acells.items {
        let mut order: Vec<usize> = (0..gs.items.len()).collect();
        let key = |i: &usize| {
            let a = &gs.items[*i].0;
            (
                !a.is_empty(),
                a.iter().map(|x| x.name.clone()).collect::<Vec<Box<str>>>(),
                py_repr_alts(a),
            )
        };
        order.sort_by_key(key);
        let mut groups: Vec<Group> = Vec::with_capacity(order.len());
        let mut gi = vec![0usize; order.len()];
        for (g, &slot) in order.iter().enumerate() {
            let (alts, names) = &gs.items[slot];
            let anchor = alts.iter().find_map(|a| a.anchor);
            let ll = match anchor {
                Some(a) => {
                    let (x, y) = a.xy();
                    to_latlon(x, y)
                }
                None => (0.0, 0.0),
            };
            let joined: Vec<&str> = alts.iter().map(|a| &*a.name).collect();
            groups.push((joined.join("|"), ll, names.clone()));
            gi[slot] = g;
            gorder.push((*ac, alts.clone()));
        }
        let (raw, ix) = build_a(ac.0, ac.1, &groups, cname);
        arec.insert(*ac, raw);
        // ix is indexed by group number; map it back to the slot order
        let mut by_slot: Vec<HashMap<Box<str>, u32>> = vec![HashMap::new(); order.len()];
        for (g, &slot) in order.iter().enumerate() {
            by_slot[slot] = ix[g].clone();
        }
        aindex.push(by_slot);
        gi_of.push(gi);
    }
    log(&since(format!("A built: {} records", arec.len())));

    // D: the pieces of each 32x32 cell
    let mut ids: HashMap<NKey, u32> = HashMap::new();
    let mut count: HashMap<(i64, i64), u32> = HashMap::new();
    let mut node_id = |key: NKey, x: f64, y: f64| -> u32 {
        *ids.entry(key).or_insert_with(|| {
            let bc = ((x / BCELL).floor() as i64, (y / BCELL).floor() as i64);
            let c = count.entry(bc).or_insert(0);
            *c += 1;
            *c
        })
    };
    let mut by_cell: Ordered<(i64, i64), Vec<DPiece>> = Ordered::default();
    for (i, p) in pieces.iter_mut().enumerate() {
        let (a, g) = (slots[i].0 as usize, slots[i].1 as usize);
        let off = aindex[a][g][&p.name];
        let (mut left, mut right) = (Vec::new(), Vec::new());
        if let Some(lst) = hn.get(&(i as u32)) {
            for h in lst {
                if h.right {
                    right.push((h.t, h.n));
                } else {
                    left.push((h.t, h.n));
                }
            }
        }
        let (first, last) = (p.pts[0], *p.pts.last().unwrap());
        let ia = node_id(
            p.na.map(NKey::Node).unwrap_or(NKey::Border(i as u32, 0)),
            p.pts[0].0,
            p.pts[0].1,
        );
        let ib =
            node_id(p.nb.map(NKey::Node).unwrap_or(NKey::Border(i as u32, 1)), last.0, last.1);
        by_cell.entry(p.cell).push(DPiece {
            off,
            left,
            right,
            ia,
            ib,
            cls: p.cls,
            len: p.len,
            // the postcode districts below only need the first point
            pts: std::mem::replace(&mut p.pts, vec![first]),
        });
    }
    drop(ids);
    let drec: BTreeMap<(i64, i64), Vec<u8>> = by_cell
        .items
        .into_par_iter()
        .map(|(cell, mut pcs)| (cell, d_record(cell, &mut pcs)))
        .collect();
    log(&since(format!("D built: {} records", drec.len())));

    // search index: one result per place name and place node
    let mut results: Ordered<(Box<str>, Option<Anchor>), BTreeSet<(i64, i64)>> =
        Ordered::default();
    for (ac, alts) in &gorder {
        for alt in alts {
            results.entry((alt.name.clone(), alt.anchor)).insert(*ac);
        }
    }
    let mut res: Vec<Res> = Vec::new();
    for ((name, anchor), cells) in &results.items {
        let (x, y) = match anchor {
            Some(a) => a.xy(),
            None => {
                let n = cells.len() as f64;
                let sx: i64 = cells.iter().map(|c| c.0).sum();
                let sy: i64 = cells.iter().map(|c| c.1).sum();
                ((sx as f64 / n + 0.5) * ACELL, (sy as f64 / n + 0.5) * ACELL)
            }
        };
        let (lat, lon) = to_latlon(x, y);
        res.push(Res {
            typ: 0,
            name: Name::Inline(name.as_bytes().to_vec()),
            lon: lon as f32,
            lat: lat as f32,
            cells: cells
                .iter()
                .map(|&(cx, cy)| Cell { x: cx as u16, y: cy as u16, offs: Vec::new() })
                .collect(),
        });
    }
    let n_with = res.len();
    let mut used: std::collections::HashSet<(Box<str>, Anchor)> = results
        .items
        .iter()
        .filter_map(|((n, a), _)| a.map(|a| (n.clone(), a)))
        .collect();
    for (pl, &ix) in places.iter().zip(&place_items) {
        let name = named[ix].clone();
        let key = (name.clone(), Anchor::of(pl.x, pl.y));
        if used.contains(&key) {
            continue;
        }
        let (lat, lon) = to_latlon(pl.x, pl.y);
        res.push(Res {
            typ: 2,
            name: Name::Inline(name.as_bytes().to_vec()),
            lon: lon as f32,
            lat: lat as f32,
            cells: Vec::new(),
        });
        used.insert(key);
    }
    let n_without = res.len() - n_with;

    // postcode districts: the streets with addresses of that outward code
    type Pcs = (Vec<(f64, f64)>, Ordered<(i64, i64), BTreeSet<u32>>);
    let mut pcs: Ordered<Box<str>, Pcs> = Ordered::default();
    for (i, lst) in &hn.items {
        let i = *i as usize;
        let p = &pieces[i];
        let ac = (p.cell.0.div_euclid(8), p.cell.1.div_euclid(8));
        let off = aindex[slots[i].0 as usize][slots[i].1 as usize][&p.name];
        for h in lst {
            if let Some(pc) = &h.pc {
                let e = pcs.entry(pc.clone());
                e.1.entry(ac).insert(off);
                e.0.push(p.pts[0]);
            }
        }
    }
    let n_pcs = pcs.len();
    for (pc, v) in &pcs.items {
        let (x, y) = mean_xy(&v.0);
        let (lat, lon) = to_latlon(x, y);
        let mut cells: Vec<&((i64, i64), BTreeSet<u32>)> = v.1.items.iter().collect();
        cells.sort_by_key(|(c, _)| *c);
        res.push(Res {
            typ: 1,
            name: Name::Inline(pc.as_bytes().to_vec()),
            lon: lon as f32,
            lat: lat as f32,
            cells: cells
                .into_iter()
                .map(|((cx, cy), offs)| Cell {
                    x: *cx as u16,
                    y: *cy as u16,
                    offs: offs.iter().copied().collect(),
                })
                .collect(),
        });
    }
    log(&since(format!(
        "index: {} places with streets, {} without, {} postcode districts",
        n_with, n_without, n_pcs
    )));

    let mut root = Node { kids: Vec::new(), res: Vec::new() };
    for r in &res {
        let Name::Inline(name) = &r.name else { continue };
        let name = String::from_utf8_lossy(name).into_owned();
        for k in keys(&name) {
            insert_key(&mut root, &k, r);
        }
    }
    sort_kids(&mut root);
    let index = ta_index::build(&Index {
        langs: LANGS.iter().map(|&(c, i)| (*c, i)).collect(),
        one: 1,
        country: cname.encode_utf16().collect(),
        root,
        pool: Vec::new(),
    })?;
    log(&since(format!("index {} B", index.len())));

    let mut tiles: BTreeMap<(i64, i64), Areas> = BTreeMap::new();
    for (area, recs, g) in [('A', arec, 4i64), ('D', drec, 32)] {
        for ((cx, cy), raw) in recs {
            let slot = (cx.rem_euclid(g) * g + cy.rem_euclid(g)) as usize;
            tiles
                .entry((cx.div_euclid(g), cy.div_euclid(g)))
                .or_default()
                .entry(area)
                .or_default()
                .insert(slot, raw);
        }
    }
    // no named street at all (Tokelau): a file without a record is not a
    // chart file, so there is none
    if tiles.is_empty() {
        return Ok(None);
    }
    let last = *tiles.keys().next_back().unwrap();
    let content: Vec<TileContent> = tiles
        .into_iter()
        .map(|(tile, areas)| TileContent {
            x: tile.0 as u16,
            y: tile.1 as u16,
            areas: Some(areas),
            tail: if tile == last { index.clone() } else { Vec::new() },
        })
        .collect();
    log(&since(format!("writing {} tiles", content.len())));
    let meta = Meta {
        date: date.to_vec(),
        typ: 2,
        layer: LAYER_TA,
        country,
        tail_tile: Some((last.0 as u16, last.1 as u16)),
    };
    writer::write_chart(&meta, &content, sign, None).map(Some)
}
