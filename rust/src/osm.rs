//! The osm layer: street net, names and the routing graph (OSM_FORMAT.md).
//!
//! Records built per tile: D (32x32 cells) with the road edges (a1) and the
//! unnamed and named lines (a3, a4), A (4x4) with the street name table the a1
//! items point into, B (8x8) with the routing graph, and C (4x4) with the
//! simplified overview lines of the road classes 0-3.
//!
//! Edges, graph nodes and their per-cell numbering are computed for the whole
//! country, the records then tile by tile, so the memory does not grow with the
//! country.  The tags are dropped as soon as the way attributes are known.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{ensure, Result};
use rayon::prelude::*;
use unicode_normalization::char::canonical_combining_class;
use unicode_normalization::UnicodeNormalization;

use crate::chart::{self, Chart};
use crate::geos::Geom;
use crate::heights::Heights;
use crate::layers::{
    self, u16enc, AItem, ARec, ArrayRec, Item, Sub18, Var, C_SPEC, D_SPEC, LAYER_OSM,
};
use crate::pbf::{TagMap, SCALE};
use crate::way::{Route, Ways};
use crate::writer::{self, Areas, Meta, TileContent};

/// units per cell (D: 360/2^28 degrees, A/C: 360/2^25)
pub const CELL: i64 = 32768;
/// the line layers store u = x + 512
pub const MARGIN: f64 = 512.0;
/// B nodes: 65536 units (360/2^27 degrees) per 8x8 cell
const BCELL: f64 = 65536.0;
/// length units per metre (~4.19)
pub const LEN_FACTOR: f64 = 5.0 * (1u64 << 25) as f64 / (2.0 * std::f64::consts::PI * 6371000.0);
/// Douglas-Peucker tolerance of the overview lines (360/2^25)
const C_TOLERANCE: f64 = 32.0;
/// cells are kept if they touch the area polygon plus this much
const BUFFER: f64 = 32768.0;
/// one tile in global units
const TS: i64 = CELL * 32;

/// road classes (a1[7] >> 27)
const CLASS: [(&str, u32); 22] = [
    ("motorway", 0),
    ("motorway_link", 0),
    ("trunk", 1),
    ("trunk_link", 1),
    ("primary", 2),
    ("secondary", 3),
    ("primary_link", 3),
    ("secondary_link", 4),
    ("tertiary", 6),
    ("tertiary_link", 6),
    ("service", 7),
    ("residential", 7),
    ("unclassified", 7),
    ("living_street", 7),
    ("road", 7),
    ("pedestrian", 8),
    ("cycleway", 10),
    ("footway", 11),
    ("track", 12),
    ("path", 13),
    ("bridleway", 13),
    ("steps", 14),
];
const FERRY: u32 = 9;
/// B way category (edge[0] bits 25-29) per class; class 7 is decided by the tag
const CATEGORY: [(u32, u32); 13] = [
    (0, 18),
    (1, 18),
    (2, 18),
    (3, 18),
    (4, 18),
    (6, 10),
    (8, 16),
    (9, 6),
    (10, 16),
    (11, 16),
    (12, 5),
    (13, 5),
    (14, 16),
];
/// motorways: node ids 0, not in the routing graph
const NOT_ROUTED: u32 = 0;
/// left turns are only looked at between roads of this class or lower
const TURN_CLASS: i8 = 7;
/// left turn: heading change of more than this (degrees)
const TURN_ANGLE: f64 = 35.0;

const PAVED: [&str; 7] = [
    "asphalt", "paved", "paving_stones", "concrete", "concrete:plates", "concrete:lanes",
    "chipseal",
];
const COBBLES: [&str; 3] = ["sett", "cobblestone", "unhewn_cobblestone"];
const RESTRICTED: [&str; 5] = ["private", "no", "destination", "customers", "forestry"];
const CYCLE_ON_ROAD: [&str; 9] = [
    "track", "lane", "shared_lane", "separate", "opposite_lane", "opposite_track", "opposite",
    "share_busway", "crossing",
];
const RAIL: [&str; 8] = [
    "rail", "light_rail", "subway", "narrow_gauge", "disused", "tram", "preserved", "miniature",
];
const A3_MAN_MADE: [(&str, u32); 3] = [("pier", 4), ("groyne", 5), ("breakwater", 6)];
const WATERWAY: [&str; 3] = ["stream", "river", "canal"];
/// only when named
const SMALL_WATERWAY: [&str; 2] = ["ditch", "drain"];

fn lookup<K: PartialEq>(table: &[(K, u32)], v: &K) -> Option<u32> {
    table.iter().find(|(k, _)| k == v).map(|(_, c)| *c)
}

/// The road class of a way, or None if it is not a road.
pub fn road_class(t: &TagMap) -> Option<u32> {
    if t.get("area") == Some("yes") {
        return None;
    }
    if t.get("route") == Some("ferry") && t.get("highway").is_none() {
        return Some(FERRY);
    }
    lookup(&CLASS, &t.get("highway")?)
}

fn category(cls: u32, t: &TagMap) -> u32 {
    if cls == 7 {
        return if t.get("highway") == Some("service") { 6 } else { 10 };
    }
    lookup(&CATEGORY, &cls).expect("class without category")
}

/// a1 word [6] / B edge word [1]: the attribute bits (OSM_FORMAT.md).
pub fn flags(t: &TagMap, rels: &[Route]) -> u32 {
    let mut f: u32 = 0;
    let junction = t.get("junction") == Some("roundabout");
    f |= junction as u32;
    let net = |n: &str| {
        rels.iter()
            .any(|(r, netw, _)| matches!(&**r, "bicycle" | "mtb") && netw.as_deref() == Some(n))
    };
    f |= (net("lcn") as u32) << 1 | (net("rcn") as u32) << 2 | (net("ncn") as u32) << 3;
    f |= (net("icn") as u32) << 27;
    f |= (rels.iter().any(|(r, _, _)| matches!(&**r, "hiking" | "foot")) as u32) << 18;
    f |= (t.get("access").is_some_and(|v| RESTRICTED.contains(&v)) as u32) << 4;
    f |= (t.get("mtb:scale").is_some() as u32) << 5;
    f |= ((t.get("highway") == Some("bridleway")) as u32) << 6;
    let surface = match t.get("surface") {
        None => 3,
        Some(s) if PAVED.contains(&s) => 1,
        Some(s) if COBBLES.contains(&s) => 2,
        Some(_) => 0,
    };
    f |= surface << 7;
    let cycle_road = t.get("cycleway").is_some_and(|v| CYCLE_ON_ROAD.contains(&v));
    let bicycle = t.get("bicycle");
    let b = match bicycle {
        Some("no") => 0,
        Some("designated") => 3,
        _ if cycle_road => 3,
        Some("yes") | Some("permissive") => 2,
        _ => 1,
    };
    f |= b << 9;
    let oneway = t.get("oneway");
    f |= ((matches!(oneway, Some("yes") | Some("true") | Some("1")) || junction) as u32) << 11;
    let lanes: i64 = t.get("lanes").unwrap_or("0").parse().unwrap_or(0);
    f |= (lanes.clamp(0, 7) as u32) << 13;
    f |= (cycle_road as u32) << 29;
    f |= ((bicycle == Some("dismount")) as u32) << 30;
    f |= (matches!(t.get("foot"), Some("yes") | Some("designated") | Some("permissive")) as u32)
        << 31;
    f
}

/// (forward, backward) passable for bicycles.
fn passable(t: &TagMap, cls: u32) -> (bool, bool) {
    if cls == FERRY {
        return (true, true);
    }
    let ow = t.get("oneway");
    if t.get("oneway:bicycle") == Some("no")
        || matches!(t.get("cycleway"), Some("opposite") | Some("opposite_lane") | Some("opposite_track"))
    {
        return (true, true);
    }
    if ow == Some("-1") {
        return (false, true);
    }
    if matches!(ow, Some("yes") | Some("true") | Some("1"))
        || t.get("junction") == Some("roundabout")
    {
        return (true, false);
    }
    (true, true)
}

/// A name with its German and English variants, in the original's notation.
fn multilingual(t: &TagMap) -> String {
    let name = t.get("name").unwrap_or("");
    let others: Vec<(&str, &str)> = [("GER", "name:de"), ("ENG", "name:en")]
        .iter()
        .filter_map(|(code, k)| {
            t.get(k).filter(|v| !v.is_empty() && *v != name).map(|v| (*code, v))
        })
        .collect();
    if name.is_empty() || others.is_empty() {
        return name.to_string();
    }
    let mut parts = vec![format!("DAN{}", name)];
    parts.extend(others.iter().map(|(c, n)| format!("{}{}", c, n)));
    format!("[{}]", parts.join("\u{a6}"))
}

/// Which line array a non-road way goes into, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LineType {
    /// a3: type only
    A3(u32),
    /// a4: type and name
    A4(u32, String),
}

pub fn line_type(t: &TagMap) -> Option<LineType> {
    if t.get("railway").is_some_and(|v| RAIL.contains(&v)) {
        return Some(LineType::A3(0));
    }
    if let Some(v) = t.get("man_made") {
        if let Some(ty) = lookup(&A3_MAN_MADE, &v) {
            return Some(LineType::A3(ty));
        }
    }
    if t.get("power") == Some("line") {
        return Some(LineType::A3(10));
    }
    let ww = t.get("waterway");
    if ww.is_some_and(|v| WATERWAY.contains(&v))
        || (ww.is_some_and(|v| SMALL_WATERWAY.contains(&v))
            && t.get("name").is_some_and(|n| !n.is_empty()))
    {
        return Some(LineType::A4(7, multilingual(t)));
    }
    let st = t.get("seamark:type")?;
    if st != "navigation_line" && st != "recommended_track" {
        return None;
    }
    let o = t
        .get(&format!("seamark:{}:orientation", st))
        .filter(|v| !v.is_empty())
        .and_then(|v| v.parse::<f64>().ok())
        .map(|v| format!("{}", v.round_ties_even() as i64))
        .unwrap_or_default();
    Some(if st == "navigation_line" {
        LineType::A4(8, format!("{}(leading)", o))
    } else {
        LineType::A4(9, format!("{}(fixed_marks)", o))
    })
}

// ---- geometry ------------------------------------------------------------

/// Clip a polyline to a closed rectangle (Liang-Barsky) -> the parts inside.
pub fn clip(pts: &[(f64, f64)], x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Vec<(f64, f64)>> {
    let mut parts: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut cur: Vec<(f64, f64)> = Vec::new();
    for i in 1..pts.len() {
        let ((ax, ay), (bx, by)) = (pts[i - 1], pts[i]);
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        let (dx, dy) = (bx - ax, by - ay);
        let mut ok = true;
        for (p, q) in [(-dx, ax - x0), (dx, x1 - ax), (-dy, ay - y0), (dy, y1 - ay)] {
            if p == 0.0 {
                if q < 0.0 {
                    ok = false;
                    break;
                }
            } else {
                let r = q / p;
                if p < 0.0 {
                    t0 = t0.max(r);
                } else {
                    t1 = t1.min(r);
                }
            }
        }
        if !ok || t0 > t1 {
            if !cur.is_empty() {
                parts.push(std::mem::take(&mut cur));
            }
            continue;
        }
        let a = (ax + t0 * dx, ay + t0 * dy);
        let b = (ax + t1 * dx, ay + t1 * dy);
        if cur.is_empty() {
            cur.push(a);
        }
        cur.push(b);
        if t1 < 1.0 {
            parts.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

/// Parts in global units -> the geometry words relative to (ox, oy).
pub fn encode_parts(parts: &[Vec<(f64, f64)>], ox: f64, oy: f64) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    for part in parts {
        let mut pts: Vec<u32> = Vec::with_capacity(part.len());
        for &(x, y) in part {
            let u = ((x - ox).round_ties_even() as i64 + MARGIN as i64) as u32;
            let v = ((y - oy).round_ties_even() as i64 + MARGIN as i64) as u32;
            let p = v << 16 | u;
            if pts.last() != Some(&p) {
                pts.push(p);
            }
        }
        if pts.len() >= 2 {
            out.push((pts.len() as u32) << 16 | pts.len() as u32);
            out.extend_from_slice(&pts);
        }
    }
    out
}

/// The 32x32 cells whose margin-extended box the bbox touches.
pub fn cells_of(x0: f64, x1: f64, y0: f64, y1: f64) -> Vec<(i64, i64)> {
    let c = CELL as f64;
    let cx0 = ((x0 - MARGIN) / c).floor() as i64;
    let cx1 = ((x1 + MARGIN) / c).floor() as i64;
    let cy0 = ((y0 - MARGIN) / c).floor() as i64;
    let cy1 = ((y1 + MARGIN) / c).floor() as i64;
    let mut out = Vec::new();
    for cx in cx0..=cx1 {
        for cy in cy0..=cy1 {
            out.push((cx, cy));
        }
    }
    out
}

/// Cut a polyline into the cells it touches; [(cell, geometry words)].
pub fn place(pts: &[(f64, f64)]) -> Vec<((i64, i64), Vec<u32>)> {
    let (mut x0, mut x1) = (f64::MAX, f64::MIN);
    let (mut y0, mut y1) = (f64::MAX, f64::MIN);
    for &(x, y) in pts {
        x0 = x0.min(x);
        x1 = x1.max(x);
        y0 = y0.min(y);
        y1 = y1.max(y);
    }
    let mut out = Vec::new();
    for (cx, cy) in cells_of(x0, x1, y0, y1) {
        let (ox, oy) = ((cx * CELL) as f64, (cy * CELL) as f64);
        let c = CELL as f64;
        let geo = if x0 >= ox - MARGIN && x1 <= ox + c + MARGIN && y0 >= oy - MARGIN
            && y1 <= oy + c + MARGIN
        {
            encode_parts(std::slice::from_ref(&pts.to_vec()), ox, oy)
        } else {
            encode_parts(&clip(pts, ox - MARGIN, oy - MARGIN, ox + c + MARGIN, oy + c + MARGIN), ox, oy)
        };
        if !geo.is_empty() {
            out.push(((cx, cy), geo));
        }
    }
    out
}

/// Direction (degrees, counter-clockwise from east) in which an edge leaves
/// its start node.
fn heading(x: &[f64], y: &[f64], a: usize, b: usize, forward: bool) -> f64 {
    let idx: Vec<usize> = if forward { (a..=b).collect() } else { (a..=b).rev().collect() };
    let i0 = idx[0];
    let (x0, y0) = (x[i0], y[i0]);
    let f = (90.0 - y0 / SCALE).to_radians().cos();
    let (mut dx, mut dy) = (0.0, 0.0);
    for &i in &idx[1..] {
        dx = (x[i] - x0) * f;
        dy = y[i] - y0;
        if dx != 0.0 || dy != 0.0 {
            break;
        }
    }
    (-dy).atan2(dx).to_degrees()
}

/// B node word [2]: the left turns of a node, from its outgoing edges.
fn turns(out: &[(Vec<u32>, i8, f64)]) -> u32 {
    let car: Vec<usize> =
        (0..out.len()).filter(|&i| out[i].1 <= TURN_CLASS).collect();
    if car.len() < 3 {
        return 0;
    }
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for &a in &car {
        for &c in &car {
            if a != c && (out[c].2 - out[a].2).rem_euclid(360.0) - 180.0 > TURN_ANGLE {
                pairs.push((a, c));
            }
        }
    }
    if pairs.len() > 5 || pairs.iter().any(|&(a, c)| a.max(c) > 7) {
        return 0;
    }
    pairs
        .iter()
        .enumerate()
        .map(|(j, &(a, c))| ((a | c << 3) as u32) << (6 * j))
        .sum()
}

// ---- names ---------------------------------------------------------------

/// `str.casefold()`: lower case, plus the two mappings that differ from it
/// and can turn up in a name.
pub fn casefold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c == '\u{3c2}' { '\u{3c3}' } else { c })
        .collect::<String>()
        .replace('\u{df}', "ss")
}

/// Sort key of a street name: case-folded, Danish letters spelled out, accents
/// dropped -- `unicodedata.normalize("NFKD", ...)` without the combining marks.
pub fn sort_key(name: &str) -> String {
    let folded = casefold(name).replace('\u{e6}', "ae").replace('\u{f8}', "o");
    folded.nfkd().filter(|&c| canonical_combining_class(c) == 0).collect()
}

/// -> (blk5, blk6, byte offset in blk5 per name)
pub fn name_table(names: &[String]) -> (Vec<u8>, Vec<u8>, HashMap<String, u32>) {
    let mut sorted: Vec<(String, &String)> = names.iter().map(|n| (sort_key(n), n)).collect();
    sorted.sort();
    let mut woff: HashMap<&str, u32> = HashMap::new();
    let mut blk6: Vec<u8> = Vec::new();
    let mut blk5: Vec<u32> = Vec::new();
    let mut index: HashMap<String, u32> = HashMap::new();
    for (_, name) in &sorted {
        let ws: Vec<&str> = name.split_whitespace().collect();
        index.insert((*name).clone(), 4 * blk5.len() as u32);
        for (j, w) in ws.iter().enumerate() {
            let off = *woff.entry(w).or_insert_with(|| {
                let at = blk6.len() as u32;
                // the words carry no NUL terminator, unlike layers::u16enc
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
    let mut b5 = Vec::with_capacity(4 * blk5.len());
    for v in &blk5 {
        b5.extend_from_slice(&v.to_le_bytes());
    }
    (b5, blk6, index)
}

/// The A record of one 4x4 cell: the country name plus the street name table.
pub fn build_a(cx: i64, cy: i64, names: &[String], country_name: &str) -> (Vec<u8>, HashMap<String, u32>) {
    let (blk5, blk6, index) = name_table(names);
    let country = u16enc(country_name);
    let item = AItem {
        s: vec![0, country.len() as u32, 2, 1, 0, 0, 0],
        name: country,
        s18: vec![Sub18 { s: vec![0, 0, 0, names.len() as u32, 0, 0], text: Vec::new() }],
        s10: Vec::new(),
    };
    let mut rec = ARec {
        hdr: vec![cx as u32, cy as u32, 0, 1, 0, 0, 0],
        items: vec![item],
        blk5: Vec::new(),
        blk6: Vec::new(),
    };
    let head = layers::build_a(&rec).len() as u32;
    rec.hdr[5] = head;
    rec.hdr[6] = head + blk5.len() as u32;
    rec.blk5 = blk5;
    rec.blk6 = blk6;
    (layers::build_a(&rec), index)
}

/// Header of a D record: [first array size, 0, 0, then the five counts].
fn d_header(arrays: &[Vec<Item>]) -> Vec<u32> {
    let first = D_SPEC
        .iter()
        .zip(arrays)
        .find(|(_, a)| !a.is_empty())
        .map(|(s, a)| (a.len() * 4 * s.words) as u32)
        .unwrap_or(0);
    let n = |i: usize| arrays[i].len() as u32;
    vec![first, 0, 0, n(0), 0, n(1), 0, n(2), 0, n(3), 0, n(4), 0]
}

/// Header of a C record; only c1 (new) and c2/c4 (from the original) are used.
fn c_header(cy: i64, arrays: &[Vec<Item>]) -> Vec<u32> {
    let first = C_SPEC
        .iter()
        .zip(arrays)
        .find(|(_, a)| !a.is_empty())
        .map(|(s, a)| (a.len() * 4 * s.words) as u32)
        .unwrap_or(0);
    let n = |i: usize| arrays[i].len() as u32;
    vec![first, cy as u32, 0, n(0), 0, n(1), 0, 0, 0, n(3), 0, 0, 0, 0, 0]
}

/// The plaintext records of one slot area of a chart, with their cells.
fn raw_records(c: &Chart, area: char) -> Result<Vec<(i64, i64, i64, i64, usize, Vec<u8>)>> {
    let mut out = Vec::new();
    for t in c.tiles() {
        if t.start == t.end {
            continue;
        }
        let rk = chart::record_key(&c.data, t.start, &c.key);
        for s in layers::slots(&c.data, &t, area) {
            let from = t.start + s.rec.rel + 8;
            let raw = chart::decode_record(&c.data[from..from + s.rec.len], s.rec.pltx, &rk)?;
            out.push((t.x as i64, t.y as i64, s.cx as i64, s.cy as i64, s.slot, raw));
        }
    }
    Ok(out)
}

/// c2/c4 (the boundaries) of the original C records, by 4x4 cell.
fn original_c(c: &Chart) -> Result<HashMap<(i64, i64), (Vec<Item>, Vec<Item>)>> {
    let mut out = HashMap::new();
    for (_, _, cx, cy, _, raw) in raw_records(c, 'C')? {
        let r = layers::parse_c(&raw)?;
        let (c2, c4) = (r.get("c2", &C_SPEC).to_vec(), r.get("c4", &C_SPEC).to_vec());
        if !c2.is_empty() || !c4.is_empty() {
            out.insert((cx, cy), (c2, c4));
        }
    }
    Ok(out)
}

/// All records of the original, by tile -- for the tiles that are copied over.
fn original_tiles(c: &Chart) -> Result<BTreeMap<(i64, i64), Areas>> {
    let mut out: BTreeMap<(i64, i64), Areas> = BTreeMap::new();
    for area in ['A', 'B', 'C', 'D'] {
        for (tx, ty, _, _, slot, raw) in raw_records(c, area)? {
            out.entry((tx, ty)).or_default().entry(area).or_default().insert(slot, raw);
        }
    }
    Ok(out)
}

// ---- main build ----------------------------------------------------------

/// `p`: the area polygon in 360/2^28 units; cells that do not touch it plus
/// [`BUFFER`] are dropped.  `orig` supplies c2/c4 and the copied tiles.
#[allow(clippy::too_many_arguments)]
pub fn build(
    ways: &mut Ways,
    p: &Geom,
    orig: Option<&Chart>,
    date: &[u8],
    heights: Option<&Heights>,
    country: u32,
    cname: &str,
    device: &[u8],
    log: &dyn Fn(&str),
) -> Result<Vec<u8>> {
    let t0 = std::time::Instant::now();
    let since = |s: String| format!("{} ({:.0} s)", s, t0.elapsed().as_secs_f32());
    let pb = p.buffer(BUFFER)?;
    let prep = crate::geos::Prepared::new(&pb)?;
    let memo: RefCell<HashMap<((i64, i64), i64), bool>> = RefCell::new(HashMap::new());
    let inside = |cell: (i64, i64), g: i64| -> Result<bool> {
        if let Some(&v) = memo.borrow().get(&(cell, g)) {
            return Ok(v);
        }
        let size = CELL * 32 / g;
        let (x, y) = ((cell.0 * size) as f64, (cell.1 * size) as f64);
        let v = prep.intersects(&Geom::rect(x, y, x + size as f64, y + size as f64)?);
        memo.borrow_mut().insert((cell, g), v);
        Ok(v)
    };

    let nw = ways.ids.len();
    let n = ways.w.len();

    // way attributes: roads get a class, flags, category, direction and name,
    // the other ways a line type
    let mut cls = vec![-1i8; nw];
    let mut fl = vec![0u32; nw];
    let mut cat = vec![0u8; nw];
    let mut fwd = vec![false; nw];
    let mut bwd = vec![false; nw];
    let mut names: Vec<String> = vec![String::new(); nw];
    let mut lines: Vec<(u32, LineType)> = Vec::new();
    for k in 0..nw {
        let (a, b) = ways.rows[k];
        if b - a < 2 {
            continue;
        }
        let t = &ways.tags[k];
        if let Some(c) = road_class(t) {
            cls[k] = c as i8;
            fl[k] = flags(t, ways.rels.get(&ways.ids[k]).map_or(&[][..], |v| &v[..]));
            cat[k] = category(c, t) as u8;
            (fwd[k], bwd[k]) = passable(t, c);
            names[k] = t.get("name").unwrap_or("").to_string();
        } else if let Some(lt) = line_type(t) {
            lines.push((k as u32, lt));
        }
    }
    ways.tags = Vec::new();
    ways.rels = HashMap::new();
    log(&since(format!(
        "{} roads, {} lines",
        cls.iter().filter(|&&c| c >= 0).count(),
        lines.len()
    )));

    let (x, y, w, nid) = (&ways.x, &ways.y, &ways.w, &ways.nid);
    let isroad = |i: usize| cls[w[i] as usize] >= 0;

    // segment lengths (haversine), cumulative over the node rows
    let mut cum = vec![0f64; n];
    {
        let seg: Vec<f64> = (0..n.saturating_sub(1))
            .into_par_iter()
            .map(|i| {
                if w[i + 1] != w[i] {
                    return 0.0;
                }
                let lat0 = (90.0 - y[i] / SCALE).to_radians();
                let lat1 = (90.0 - y[i + 1] / SCALE).to_radians();
                let dlat = lat1 - lat0;
                let dlon = (x[i + 1] / SCALE - 180.0).to_radians() - (x[i] / SCALE - 180.0).to_radians();
                let h = (dlat / 2.0).sin().powi(2)
                    + lat0.cos() * lat1.cos() * (dlon / 2.0).sin().powi(2);
                2.0 * 6371000.0 * h.sqrt().asin() * LEN_FACTOR
            })
            .collect();
        for i in 1..n {
            cum[i] = cum[i - 1] + seg[i - 1];
        }
    }

    // nodes shared by more than one road way: there the ways are cut
    let mut rn: Vec<i64> = (0..n).filter(|&i| isroad(i)).map(|i| nid[i]).collect();
    rn.par_sort_unstable();
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
    log(&since(format!("{} shared nodes", shared.len())));

    // ascent and descent (cm) per node row, cumulative
    let mut cup = vec![0f64; n];
    let mut cdown = vec![0f64; n];
    if let Some(hts) = heights {
        let idx: Vec<usize> = (0..n).filter(|&i| isroad(i)).collect();
        let hx: Vec<f64> = idx.iter().map(|&i| x[i]).collect();
        let hy: Vec<f64> = idx.iter().map(|&i| y[i]).collect();
        let v = hts.at(&hx, &hy);
        let mut h = vec![0f64; n];
        for (j, &i) in idx.iter().enumerate() {
            h[i] = v[j];
        }
        for i in 1..n {
            let d = if w[i] != w[i - 1] { 0.0 } else { h[i] - h[i - 1] };
            cup[i] = cup[i - 1] + d.max(0.0);
            cdown[i] = cdown[i - 1] + (-d).max(0.0);
        }
        log(&since("heights interpolated".to_string()));
    }

    // edges: consecutive cut points (way ends and shared nodes) of a road way
    let mut cutm = vec![false; n];
    for k in 0..nw {
        let (a, b) = ways.rows[k];
        if b > a {
            cutm[a as usize] = true;
            cutm[b as usize - 1] = true;
        }
    }
    for i in 0..n {
        cutm[i] = isroad(i) && (cutm[i] || shared.binary_search(&nid[i]).is_ok());
    }
    let cp: Vec<u32> = (0..n as u32).filter(|&i| cutm[i as usize]).collect();
    drop(cutm);
    let mut ea: Vec<u32> = Vec::new();
    let mut eb: Vec<u32> = Vec::new();
    for j in 1..cp.len() {
        if w[cp[j] as usize] == w[cp[j - 1] as usize] {
            ea.push(cp[j - 1]);
            eb.push(cp[j]);
        }
    }
    drop(cp);
    let ne = ea.len();
    let ek: Vec<u32> = ea.iter().map(|&a| w[a as usize]).collect();
    let routed: Vec<bool> = ek.iter().map(|&k| cls[k as usize] != NOT_ROUTED as i8).collect();
    log(&since(format!("{} edges", ne)));

    // graph nodes, numbered per 8x8 cell in (v, u, node id) order from 1
    let mut ends2: Vec<u32> = Vec::with_capacity(2 * ne);
    for e in 0..ne {
        if routed[e] {
            ends2.push(ea[e]);
            ends2.push(eb[e]);
        }
    }
    let mut ord: Vec<u32> = (0..ends2.len() as u32).collect();
    ord.par_sort_by_key(|&j| nid[ends2[j as usize] as usize]);
    let mut gid: Vec<i64> = Vec::new();
    let mut pos: Vec<u32> = Vec::new();
    for &j in &ord {
        let row = ends2[j as usize];
        let id = nid[row as usize];
        if gid.last() != Some(&id) {
            gid.push(id);
            pos.push(row);
        }
    }
    drop(ord);
    drop(ends2);
    let ng = gid.len();
    let gx: Vec<f64> = pos.iter().map(|&i| x[i as usize] / 2.0).collect();
    let gy: Vec<f64> = pos.iter().map(|&i| y[i as usize] / 2.0).collect();
    drop(pos);
    let gbx: Vec<i64> = gx.iter().map(|v| (v / BCELL).floor() as i64).collect();
    let gby: Vec<i64> = gy.iter().map(|v| (v / BCELL).floor() as i64).collect();
    // the original maps a cell onto 0..65535: floor(u * 65535/65536)
    let gu: Vec<u32> = (0..ng)
        .map(|i| ((gx[i] - gbx[i] as f64 * BCELL) * (BCELL - 1.0) / BCELL) as u32)
        .collect();
    let gv: Vec<u32> = (0..ng)
        .map(|i| ((gy[i] - gby[i] as f64 * BCELL) * (BCELL - 1.0) / BCELL) as u32)
        .collect();
    drop(gx);
    drop(gy);
    let mut o: Vec<u32> = (0..ng as u32).collect();
    o.par_sort_by_key(|&i| {
        let i = i as usize;
        (gbx[i], gby[i], gv[i], gu[i], gid[i])
    });
    let mut gj = vec![0u32; ng];
    let mut group = 0usize;
    for q in 0..ng {
        let (a, b) = (o[q] as usize, if q > 0 { o[q - 1] as usize } else { 0 });
        if q > 0 && (gbx[a], gby[a]) != (gbx[b], gby[b]) {
            group = q;
        }
        gj[a] = (q - group + 1) as u32;
    }
    let bcells: HashSet<(i64, i64)> = (0..ng).map(|i| (gbx[i], gby[i])).collect();
    let mut kb: HashSet<(i64, i64)> = HashSet::new();
    for bc in &bcells {
        if inside(*bc, 8)? {
            kb.insert(*bc);
        }
    }
    let gkeep: Vec<bool> = (0..ng).map(|i| kb.contains(&(gbx[i], gby[i]))).collect();
    let mut eu = vec![0u32; ne];
    let mut ev = vec![0u32; ne];
    for e in 0..ne {
        if routed[e] {
            eu[e] = gid.partition_point(|&v| v < nid[ea[e] as usize]) as u32;
            ev[e] = gid.partition_point(|&v| v < nid[eb[e] as usize]) as u32;
        }
    }
    drop(gid);
    log(&since(format!(
        "{} graph nodes in {} B cells, {} kept",
        ng,
        bcells.len(),
        kb.len()
    )));

    // directed edges (both ways), per source node in edge order
    let mut half: Vec<(u32, u32, bool)> = Vec::new();
    for e in 0..ne {
        let mut ok = routed[e] && gkeep[eu[e] as usize] && gkeep[ev[e] as usize];
        if ok {
            let (a, b) = (eu[e] as usize, ev[e] as usize);
            if (gbx[a] - gbx[b]).abs().max((gby[a] - gby[b]).abs()) > 63 {
                log(&format!(
                    "way {}: edge spans more than 63 B cells, not routed",
                    ways.ids[ek[e] as usize]
                ));
                ok = false;
            }
        }
        if ok {
            half.push((eu[e], e as u32, true));
            half.push((ev[e], e as u32, false));
        }
    }
    half.par_sort_by_key(|&(s, e, f)| (s, e, !f));
    let mut hstart = vec![0u32; ng + 1];
    for &(s, _, _) in &half {
        hstart[s as usize + 1] += 1;
    }
    for i in 0..ng {
        hstart[i + 1] += hstart[i];
    }

    let b_record = |bc: (i64, i64), sel: &[u32]| -> Result<Vec<u8>> {
        let mut nodes: Vec<Vec<u32>> = vec![vec![0, 0, 0]];
        let mut bedges: Vec<Vec<u32>> = Vec::new();
        for &g in sel {
            let mut out: Vec<(Vec<u32>, i8, f64)> = Vec::new();
            for h in hstart[g as usize]..hstart[g as usize + 1] {
                let (_, e, f) = half[h as usize];
                let e = e as usize;
                let k = ek[e] as usize;
                let (a, b) = (ea[e] as usize, eb[e] as usize);
                let c = cls[k];
                let dst = (if f { ev[e] } else { eu[e] }) as usize;
                let ln = ((cum[b] - cum[a]).round_ties_even() as i64).min(0xFFFFF) as u32;
                let (dx, dy) = (gbx[dst] - bc.0, gby[dst] - bc.1);
                let w0 = ln
                    | (c as u32) << 21
                    | (cat[k] as u32) << 25
                    | ((if f { fwd[k] } else { bwd[k] }) as u32) << 30
                    | 1 << 31;
                let w3 = ((dx + 64) as u32) << 25 | ((dy + 64) as u32) << 18 | gj[dst];
                let climb = (if f { cup[b] - cup[a] } else { cdown[b] - cdown[a] })
                    .round_ties_even() as u32;
                out.push((vec![w0, fl[k], climb, w3], c, heading(x, y, a, b, f)));
            }
            ensure!(out.len() < 64, "graph node with {} edges", out.len());
            nodes.push(vec![
                bedges.len() as u32 | (out.len() as u32) << 26,
                gv[g as usize] << 16 | gu[g as usize],
                turns(&out),
            ]);
            bedges.extend(out.into_iter().map(|(e, _, _)| e));
        }
        ensure!(bedges.len() < 1 << 19, "B cell with {} edges", bedges.len());
        let hdr = vec![0, 0, 0, nodes.len() as u32, 0, bedges.len() as u32, 0, 0, 0, 0, 0];
        Ok(layers::build_b(&layers::BRec { hdr, nodes, edges: bedges, b3: Vec::new(), extra: Vec::new() }))
    };

    // bounding boxes of the edges and the lines, for the tile selection
    let bbox = |a: usize, b: usize| {
        let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for i in a..=b {
            x0 = x0.min(x[i]);
            x1 = x1.max(x[i]);
            y0 = y0.min(y[i]);
            y1 = y1.max(y[i]);
        }
        (x0, x1, y0, y1)
    };
    let ebox: Vec<(f64, f64, f64, f64)> =
        (0..ne).into_par_iter().map(|e| bbox(ea[e] as usize, eb[e] as usize)).collect();
    let lbox: Vec<(f64, f64, f64, f64)> = lines
        .par_iter()
        .map(|&(k, _)| {
            let (a, b) = ways.rows[k as usize];
            bbox(a as usize, b as usize - 1)
        })
        .collect();

    // tiles touching the area
    let ts = TS as f64;
    let (bx0, by0, bx1, by1) = pb.bounds();
    let mut tlist: Vec<(i64, i64)> = Vec::new();
    for tx in (bx0 / ts).floor() as i64..=(bx1 / ts).floor() as i64 {
        for ty in (by0 / ts).floor() as i64..=(by1 / ts).floor() as i64 {
            if inside((tx, ty), 1)? {
                tlist.push((tx, ty));
            }
        }
    }
    let tset: HashSet<(i64, i64)> = tlist.iter().copied().collect();

    // which tile's D records an edge or a line can reach
    let spread = |b: &(f64, f64, f64, f64)| -> Vec<(i64, i64)> {
        let mut out = Vec::new();
        let r = |v: f64| (v / ts).floor() as i64;
        for tx in r(b.0 - MARGIN) - 1..=r(b.1 + MARGIN) + 1 {
            for ty in r(b.2 - MARGIN) - 1..=r(b.3 + MARGIN) + 1 {
                let (tx0, ty0) = (tx * TS as i64 - MARGIN as i64, ty * TS - MARGIN as i64);
                let (tx1, ty1) = (tx0 + TS + 2 * MARGIN as i64, ty0 + TS + 2 * MARGIN as i64);
                if b.1 >= tx0 as f64
                    && b.0 <= tx1 as f64
                    && b.3 >= ty0 as f64
                    && b.2 <= ty1 as f64
                    && tset.contains(&(tx, ty))
                {
                    out.push((tx, ty));
                }
            }
        }
        out
    };
    let mut tedges: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
    for e in 0..ne {
        for t in spread(&ebox[e]) {
            tedges.entry(t).or_default().push(e as u32);
        }
    }
    drop(ebox);
    let mut tlines: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
    for j in 0..lines.len() {
        for t in spread(&lbox[j]) {
            tlines.entry(t).or_default().push(j as u32);
        }
    }
    drop(lbox);

    // graph nodes per tile, already in (cell, number) order
    let mut tnodes: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
    for &q in &o {
        let i = q as usize;
        if gkeep[i] {
            tnodes.entry((gbx[i].div_euclid(8), gby[i].div_euclid(8))).or_default().push(q);
        }
    }
    drop(o);

    let empty: Vec<u32> = Vec::new();
    let mut brec: BTreeMap<(i64, i64), Vec<u8>> = BTreeMap::new();
    let mut arec: BTreeMap<(i64, i64), Vec<u8>> = BTreeMap::new();
    let mut drec: BTreeMap<(i64, i64), Vec<u8>> = BTreeMap::new();
    for (nt, &(tx, ty)) in tlist.iter().enumerate() {
        // B: the routing graph
        let sel = tnodes.get(&(tx, ty)).unwrap_or(&empty);
        let mut cells_b: Vec<((i64, i64), Vec<u32>)> = Vec::new();
        for &g in sel {
            let bc = (gbx[g as usize], gby[g as usize]);
            if cells_b.last().map(|(c, _)| *c) != Some(bc) {
                cells_b.push((bc, Vec::new()));
            }
            cells_b.last_mut().unwrap().1.push(g);
        }
        for (bc, lst) in &cells_b {
            brec.insert(*bc, b_record(*bc, lst)?);
        }

        // D: the road edges and the lines, cut at the cell margin
        let mut cells: HashMap<(i64, i64), Vec<DItem>> = HashMap::new();
        for &e in tedges.get(&(tx, ty)).unwrap_or(&empty) {
            let e = e as usize;
            let k = ek[e] as usize;
            let c = cls[k];
            let (a, b) = (ea[e] as usize, eb[e] as usize);
            let pts: Vec<(f64, f64)> = (a..=b).map(|i| (x[i], y[i])).collect();
            let (s_id, t_id) = if c == NOT_ROUTED as i8 {
                (0, 0)
            } else {
                let at = |g: usize| if gkeep[g] { gj[g] } else { 0 };
                (at(eu[e] as usize), at(ev[e] as usize))
            };
            let ln = ((cum[b] - cum[a]).round_ties_even() as i64).min(0xFFFFFF) as u32;
            let w7 = (c as u32) << 27 | ln;
            for (cell, geo) in place(&pts) {
                cells.entry(cell).or_default().push(DItem::A1 {
                    k: k as u32,
                    s: s_id,
                    t: t_id,
                    f: fl[k],
                    w7,
                    geo,
                });
            }
        }
        for &j in tlines.get(&(tx, ty)).unwrap_or(&empty) {
            let (k, lt) = &lines[j as usize];
            let (a, b) = ways.rows[*k as usize];
            let pts: Vec<(f64, f64)> = (a as usize..b as usize).map(|i| (x[i], y[i])).collect();
            for (cell, geo) in place(&pts) {
                cells.entry(cell).or_default().push(match lt {
                    LineType::A3(ty) => DItem::A3(*ty, geo),
                    LineType::A4(ty, nm) => DItem::A4(*ty, nm.clone(), geo),
                });
            }
        }
        let mut keep: Vec<(i64, i64)> = Vec::new();
        for cell in cells.keys() {
            if cell.0.div_euclid(32) == tx && cell.1.div_euclid(32) == ty && inside(*cell, 32)? {
                keep.push(*cell);
            }
        }

        // A: the name table per 4x4 cell, from the names of its 64 D cells
        let mut anames: HashMap<(i64, i64), HashSet<&str>> = HashMap::new();
        for cell in &keep {
            for it in &cells[cell] {
                if let DItem::A1 { k, .. } = it {
                    let nm = &names[*k as usize];
                    if !nm.is_empty() {
                        anames
                            .entry((cell.0.div_euclid(8), cell.1.div_euclid(8)))
                            .or_default()
                            .insert(nm);
                    }
                }
            }
        }
        let mut index: HashMap<(i64, i64), HashMap<String, u32>> = HashMap::new();
        for (ac, ns) in &anames {
            let v: Vec<String> = ns.iter().map(|s| s.to_string()).collect();
            let (raw, ix) = build_a(ac.0, ac.1, &v, cname);
            arec.insert(*ac, raw);
            index.insert(*ac, ix);
        }

        for cell in &keep {
            let idx = index.get(&(cell.0.div_euclid(8), cell.1.div_euclid(8)));
            let mut a1: Vec<Item> = Vec::new();
            let mut a3: Vec<Item> = Vec::new();
            let mut a4: Vec<Item> = Vec::new();
            for it in &cells[cell] {
                match it {
                    DItem::A1 { k, s, t, f, w7, geo } => {
                        let nm = &names[*k as usize];
                        let off = if nm.is_empty() {
                            0xFFFF_FFFF
                        } else {
                            *idx.and_then(|m| m.get(nm)).expect("name not in the table")
                        };
                        a1.push(Item {
                            s: vec![
                                off,
                                off,
                                0xFFFF_7FFF,
                                0xFFFF_7FFF,
                                *s,
                                *t,
                                *f,
                                *w7,
                                geo.len() as u32,
                                0,
                            ],
                            v: vec![Var::U32(geo.clone())],
                        });
                    }
                    DItem::A3(ty, geo) => a3.push(Item {
                        s: vec![*ty, geo.len() as u32, 0],
                        v: vec![Var::U32(geo.clone())],
                    }),
                    DItem::A4(ty, nm, geo) => {
                        let nm = u16enc(nm);
                        a4.push(Item {
                            s: vec![0, nm.len() as u32, *ty, geo.len() as u32, 0],
                            v: vec![Var::U16(nm), Var::U32(geo.clone())],
                        });
                    }
                }
            }
            a1.sort_by_key(|it| (it.s[7] >> 27, it.s[0]));
            a3.sort_by_key(|it| it.s[0]);
            a4.sort_by_key(|it| it.s[2]);
            let arrays = vec![a1, Vec::new(), a3, a4, Vec::new()];
            let hdr = d_header(&arrays);
            drec.insert(*cell, layers::build_arrays(&ArrayRec { hdr, arrays }, &D_SPEC));
        }
        log(&since(format!(
            "  tile {},{} ({}/{}): {} B, {} D records",
            tx,
            ty,
            nt + 1,
            tlist.len(),
            cells_b.len(),
            keep.len()
        )));
    }
    log(&since(format!(
        "A built: {} records, B built: {} records, D built: {} records",
        arec.len(),
        brec.len(),
        drec.len()
    )));

    // C: the overview lines of the classes 0-3, in 360/2^25 units
    let ocs = match orig {
        Some(c) => original_c(c)?,
        None => HashMap::new(),
    };
    let mut cit: BTreeMap<((i64, i64), u32), Vec<u32>> = BTreeMap::new();
    for c in 0..4i8 {
        let mut segs: Vec<Geom> = Vec::new();
        for k in 0..nw {
            if cls[k] == c {
                let (a, b) = ways.rows[k];
                let pts: Vec<(f64, f64)> =
                    (a as usize..b as usize).map(|i| (x[i] / 8.0, y[i] / 8.0)).collect();
                segs.push(Geom::linestring(&pts)?);
            }
        }
        if segs.is_empty() {
            continue;
        }
        let nseg = segs.len();
        let merged = Geom::multilinestring(segs)?.line_merge()?.simplify_dp(C_TOLERANCE)?;
        let parts = merged.parts()?;
        for ln in &parts {
            let pts = ln.coords();
            if pts.len() < 2 {
                continue;
            }
            for (cell, geo) in place(&pts) {
                if inside(cell, 4)? {
                    cit.entry((cell, c as u32)).or_default().extend(geo);
                }
            }
        }
        log(&since(format!("  class {}: {} ways -> {} lines", c, nseg, parts.len())));
    }
    let mut c1: HashMap<(i64, i64), Vec<Item>> = HashMap::new();
    for ((cell, c), geo) in &cit {
        c1.entry(*cell).or_default().push(Item {
            s: vec![0xFFFF_FFFF, *c, geo.len() as u32, 0],
            v: vec![Var::U32(geo.clone())],
        });
    }
    let mut crec: BTreeMap<(i64, i64), Vec<u8>> = BTreeMap::new();
    let mut ccells: Vec<(i64, i64)> = c1.keys().chain(ocs.keys()).copied().collect();
    ccells.sort();
    ccells.dedup();
    for cell in ccells {
        let (c2, c4) = ocs.get(&cell).cloned().unwrap_or_default();
        let arrays = vec![
            c1.get(&cell).cloned().unwrap_or_default(),
            c2,
            Vec::new(),
            c4,
            Vec::new(),
            Vec::new(),
        ];
        let hdr = c_header(cell.1, &arrays);
        crec.insert(cell, layers::build_arrays(&ArrayRec { hdr, arrays }, &C_SPEC));
    }
    log(&since(format!("C built: {} records", crec.len())));

    // tiles
    let mut tiles: BTreeMap<(i64, i64), Areas> = BTreeMap::new();
    for (area, recs, g) in
        [('A', &arec, 4i64), ('B', &brec, 8), ('C', &crec, 4), ('D', &drec, 32)]
    {
        for (&(cx, cy), raw) in recs {
            let slot = (cx.rem_euclid(g) * g + cy.rem_euclid(g)) as usize;
            tiles
                .entry((cx.div_euclid(g), cy.div_euclid(g)))
                .or_default()
                .entry(area)
                .or_default()
                .insert(slot, raw.clone());
        }
    }
    if let Some(c) = orig {
        for (tile, areas) in original_tiles(c)? {
            // west of 0 degrees without a new record: the Faroe Islands
            if !tiles.contains_key(&tile) && tile.0 < 128 {
                log(&format!("tile {},{} copied from the original", tile.0, tile.1));
                tiles.insert(tile, areas);
            }
        }
    }
    let content: Vec<TileContent> = tiles
        .into_iter()
        .map(|((tx, ty), areas)| TileContent {
            x: tx as u16,
            y: ty as u16,
            areas: Some(areas),
            tail: Vec::new(),
        })
        .collect();
    log(&since(format!("writing {} tiles", content.len())));
    let meta = Meta {
        date: date.to_vec(),
        typ: 1,
        layer: LAYER_OSM,
        country,
        tail_tile: None,
    };
    writer::write_chart(&meta, &content, device, true, None)
}

/// One item of a D record before the name offsets are known.
enum DItem {
    A1 { k: u32, s: u32, t: u32, f: u32, w7: u32, geo: Vec<u32> },
    A3(u32, Vec<u32>),
    A4(u32, String, Vec<u32>),
}
