//! The osmarea layer: land use, water and sea (OSMAREA_FORMAT.md).
//! Port of tools/compile_osmarea.py, with GEOS doing the geometry.
//!
//! One deliberate difference to the first Python version: the areas are
//! processed in the order of their osmium id.  Python used to take them in the
//! order the extractor wrote them, which is the order libosmium happens to
//! flush its buffers in -- that order decides which polygon enters a union
//! first and so shows up in the output.  Sorting makes the layer reproducible
//! (tools/compile_osmarea.py sorts too now).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::{bail, Result};

use crate::area::{ATags, Extract};
use crate::chart::{self, Chart};
use crate::geos::{self, Geom, Tree};
use crate::layers::{self, u16enc, u16str, ArrayRec, Item, Var, C_HDR, C_SPEC, LAYER_OSMAREA};
use crate::writer::{self, Areas, Meta, TileContent};

/// slot area C: 4x4 cells per tile
pub const G: i64 = 4;
/// units per cell (360/2^25 degrees)
pub const CELL: i64 = 32768;
/// objects are cut into 8x8 blocks per cell
pub const BLOCK: i64 = 4096;
/// units^2 (~120 m2); smaller OSM polygons are dropped
pub const MIN_AREA: f64 = 150.0;
/// Douglas-Peucker, units
pub const TOLERANCE: f64 = 4.0;
/// hi = 14 - k for a shorter bbox side >= 33 * 2^k (k <= 5)
pub const LEVEL_BASE: i64 = 33;
pub const SEA_LEVEL: u32 = 9;

/// Target array and, for c5, the class in it.
pub type Key = (&'static str, Option<u32>);

const C3: Key = ("c3", None);
const WATER: Key = ("c6", None);

/// draw order of the c5 classes in a cell (roughly as in the original)
const CLASS_ORDER: [u32; 14] = [4, 3, 10, 1, 0, 9, 11, 2, 5, 17, 8, 6, 7, 16];

const LEISURE: [&str; 23] = [
    "park", "pitch", "playground", "garden", "sports_centre", "golf_course", "track",
    "miniature_golf", "marina", "common", "dog_park", "stadium", "horse_riding",
    "recreation_ground", "water_park", "fitness_station", "swimming_pool", "fishing",
    "sports_hall", "outdoor_seating", "slipway", "soccer_golf", "disc_golf_course",
];

fn landuse(v: &str) -> Option<Key> {
    Some(match v {
        "residential" => C3,
        "forest" | "scrub" => ("c5", Some(6)),
        "farmland" | "farmyard" | "allotments" | "orchard" | "vineyard" => ("c5", Some(8)),
        "meadow" | "grass" | "greenfield" | "village_green" | "greenhouse_horticulture" => {
            ("c5", Some(7))
        }
        "industrial" | "quarry" | "construction" | "railway" | "landfill" => ("c5", Some(4)),
        "commercial" | "retail" => ("c5", Some(1)),
        "military" => ("c5", Some(2)),
        "cemetery" => ("c5", Some(3)),
        "recreation_ground" => ("c5", Some(5)),
        "basin" | "reservoir" | "aquaculture" => WATER,
        "plant_nursery" | "brownfield" | "animal_keeping" | "fishfarm" | "garages" | "harbour"
        | "religious" | "scout_camp" => ("c5", Some(0)),
        _ => return None,
    })
}

fn natural(v: &str) -> Option<Key> {
    Some(match v {
        "water" => WATER,
        "wetland" => ("c5", Some(10)),
        "beach" => ("c5", Some(9)),
        "heath" => ("c5", Some(6)),
        _ => return None,
    })
}

fn man_made(v: &str) -> Option<Key> {
    Some(match v {
        "pier" => ("c5", Some(11)),
        "groyne" => ("c5", Some(16)),
        "breakwater" => ("c5", Some(17)),
        _ => return None,
    })
}

/// tags -> target array and class; `landuse` wins over `natural`.
pub fn area_class(t: &ATags) -> Option<Key> {
    for v in [t.landuse.as_deref().and_then(landuse), t.natural.as_deref().and_then(natural)] {
        if let Some(k) = v {
            // natural=water + water=river is not in the original
            if k == WATER && t.water.as_deref() == Some("river") {
                return None;
            }
            return Some(k);
        }
    }
    if t.leisure.as_deref().is_some_and(|v| LEISURE.contains(&v)) {
        return Some(("c5", Some(5)));
    }
    if let Some(k) = t.man_made.as_deref().and_then(man_made) {
        return Some(k);
    }
    if t.waterway.as_deref() == Some("dock") {
        return Some(WATER);
    }
    None
}

/// Python sorts the classes by `str(key)`, so "('c5', 10)" comes before "('c5', 2)".
fn key_str(k: Key) -> String {
    format!("('{}', {})", k.0, k.1.map_or("None".to_string(), |c| c.to_string()))
}

/// Detail level from which a ring is drawn (the firmware skips parts with hi
/// above the current level): 14 for small, down to 9 for large polygons.
fn level(b: (f64, f64, f64, f64)) -> u32 {
    let m = (b.2 - b.0).min(b.3 - b.1);
    let mut hi = 14i64;
    for k in 1..6 {
        if m >= (LEVEL_BASE << k) as f64 {
            hi = 14 - k;
        }
    }
    hi as u32
}

/// Area of a closed ring of integer points.
fn ring_area(r: &[(i32, i32)]) -> f64 {
    let mut s = 0.0;
    for i in 0..r.len() - 1 {
        let (x1, y1) = (f64::from(r[i].0), f64::from(r[i].1));
        let (x2, y2) = (f64::from(r[i + 1].0), f64::from(r[i + 1].1));
        s += x1 * y2 - x2 * y1;
    }
    (s / 2.0).abs()
}

fn fpts(r: &[(i32, i32)]) -> Vec<(f64, f64)> {
    r.iter().map(|&(x, y)| (f64::from(x), f64::from(y))).collect()
}

/// Signed area of a closed point list; shapely's `orient` looks at the sign.
fn signed(pts: &[(f64, f64)]) -> f64 {
    let mut s = 0.0;
    for i in 0..pts.len().saturating_sub(1) {
        s += pts[i].0 * pts[i + 1].1 - pts[i + 1].0 * pts[i].1;
    }
    s / 2.0
}

/// `shapely.geometry.polygon.orient(p, 1.0)` on a ring's points: an outer ring
/// gets a positive signed area, a hole a negative one, the first point stays.
fn orient(mut pts: Vec<(f64, f64)>, positive: bool) -> Vec<(f64, f64)> {
    if (signed(&pts) >= 0.0) != positive && pts.len() > 2 {
        let n = pts.len();
        pts[..n - 1].reverse();
        pts.rotate_right(1);
        let first = pts[0];
        pts[n - 1] = first;
    }
    pts
}

/// Shapely ring -> closed list of int points without repeated neighbours.
fn ring_points(coords: &[(f64, f64)]) -> Option<Vec<(i64, i64)>> {
    let mut out: Vec<(i64, i64)> = Vec::with_capacity(coords.len());
    for &(x, y) in coords {
        let q = (x.round_ties_even() as i64, y.round_ties_even() as i64);
        if out.last() != Some(&q) {
            out.push(q);
        }
    }
    if out.is_empty() {
        return None;
    }
    if out[0] != *out.last().unwrap() {
        out.push(out[0]);
    }
    if out.len() >= 4 {
        Some(out)
    } else {
        None
    }
}

/// -> (array, class) -> polygons, inside the bounding box of `p`.  Islets
/// (closed coastline ways with place=islet) are drawn as class 11 like piers.
fn load(ex: &Extract, p: &Geom, log: &dyn Fn(&str)) -> Result<Vec<(Key, Vec<Geom>)>> {
    let (bx0, by0, bx1, by1) = p.bounds();
    let mut groups: HashMap<Key, Vec<Geom>> = HashMap::new();
    let mut order: Vec<usize> = (0..ex.areas.len()).collect();
    order.sort_by_key(|&i| ex.areas[i].id);
    for i in order {
        let a = &ex.areas[i];
        let Some(key) = area_class(&a.tags) else { continue };
        for poly in &a.polys {
            if ring_area(&poly.outer) < MIN_AREA {
                continue;
            }
            let xs = poly.outer.iter().map(|q| f64::from(q.0));
            let ys = poly.outer.iter().map(|q| f64::from(q.1));
            let (mut xmin, mut xmax) = (f64::MAX, f64::MIN);
            for x in xs {
                xmin = xmin.min(x);
                xmax = xmax.max(x);
            }
            let (mut ymin, mut ymax) = (f64::MAX, f64::MIN);
            for y in ys {
                ymin = ymin.min(y);
                ymax = ymax.max(y);
            }
            if xmax < bx0 || xmin > bx1 || ymax < by0 || ymin > by1 {
                continue;
            }
            let holes: Vec<Vec<(f64, f64)>> = poly
                .inners
                .iter()
                .filter(|i| ring_area(i) >= MIN_AREA)
                .map(|r| fpts(r))
                .collect();
            let g = Geom::polygon(&fpts(&poly.outer), &holes)?;
            groups.entry(key).or_default().extend(g.valid()?.polygons()?);
        }
    }
    for c in &ex.coast {
        if c.islet && c.first == c.last && c.ring.len() >= 4 && ring_area(&c.ring) >= MIN_AREA {
            let g = Geom::polygon(&fpts(&c.ring), &[])?;
            groups.entry(("c5", Some(11))).or_default().extend(g.valid()?.polygons()?);
        }
    }
    let mut out: Vec<(Key, Vec<Geom>)> = groups.into_iter().collect();
    out.sort_by_key(|(k, _)| key_str(*k));
    log(&format!(
        "{} polygons in {} classes",
        out.iter().map(|(_, l)| l.len()).sum::<usize>(),
        out.len()
    ));
    Ok(out)
}

/// Union of polygons, done per group of intersecting polygons (connected
/// components of the STRtree intersection graph): the same result as one
/// union_all, but minutes faster for the large classes.
fn dissolve(lst: &[Geom]) -> Result<Vec<Geom>> {
    let tree = Tree::new(lst)?;
    let mut up: Vec<usize> = (0..lst.len()).collect();
    fn root(up: &mut Vec<usize>, mut i: usize) -> usize {
        while up[i] != i {
            up[i] = up[up[i]];
            i = up[i];
        }
        i
    }
    for (i, g) in lst.iter().enumerate() {
        let prep = geos::Prepared::new(g)?;
        for j in tree.query(g) {
            if j != i && prep.intersects(&lst[j]) {
                let (a, b) = (root(&mut up, i), root(&mut up, j));
                if a != b {
                    up[a.max(b)] = a.min(b);
                }
            }
        }
    }
    // components in the order of their smallest member, as scipy labels them
    let mut comps: Vec<Vec<usize>> = Vec::new();
    let mut at: HashMap<usize, usize> = HashMap::new();
    for i in 0..lst.len() {
        let r = root(&mut up, i);
        let slot = *at.entry(r).or_insert_with(|| {
            comps.push(Vec::new());
            comps.len() - 1
        });
        comps[slot].push(i);
    }
    let mut out = Vec::new();
    for c in &comps {
        if c.len() == 1 {
            out.extend(lst[c[0]].polygons()?);
        } else {
            out.extend(geos::union_all(c.iter().map(|&i| &lst[i]))?.polygons()?);
        }
    }
    Ok(out)
}

/// One ring of an object: block, detail level, points.
type Part = ((i64, i64), u32, Vec<(i64, i64)>);

/// Simplify, clip to `p` and cut into blocks.
fn cut(g: &Geom, hi: u32, p: &Geom) -> Result<Vec<Part>> {
    let s = g.simplify(TOLERANCE)?;
    let s = if p.contains(&s) { s } else { s.intersection(p)? };
    let mut out: Vec<Part> = Vec::new();
    for q in s.polygons()? {
        let (x0, y0, x1, y1) = q.bounds();
        for bx in (x0 as i64).div_euclid(BLOCK)..=(x1 as i64).div_euclid(BLOCK) {
            for by in (y0 as i64).div_euclid(BLOCK)..=(y1 as i64).div_euclid(BLOCK) {
                let r = (bx * BLOCK, by * BLOCK, bx * BLOCK + BLOCK, by * BLOCK + BLOCK);
                let whole = r.0 as f64 <= x0
                    && r.1 as f64 <= y0
                    && x1 <= r.2 as f64
                    && y1 <= r.3 as f64;
                let c = if whole {
                    q.clone_geom()?
                } else {
                    q.clip_by_rect(r.0 as f64, r.1 as f64, r.2 as f64, r.3 as f64)?
                };
                for piece in c.polygons()? {
                    if let Some(ring) = ring_points(&orient(piece.exterior(), true)) {
                        out.push(((bx, by), hi, ring));
                    }
                    for h in piece.interiors() {
                        let hb = Geom::polygon(&h, &[])?.bounds();
                        if let Some(ring) = ring_points(&orient(h, false)) {
                            out.push(((bx, by), level(hb), ring));
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

/// `natural=coastline` ways -> (land, water) polygons.  Land is left of the way,
/// so closed rings running anticlockwise (lon/lat) are land, clockwise ones
/// water in land.  Open chains (they end far outside the country) are closed
/// with a straight line.
fn land_polygons(ex: &Extract) -> Result<(Vec<Geom>, Vec<Geom>)> {
    // chains in insertion order, as in the Python dict
    let mut chains: Vec<Option<(i64, i64, Vec<usize>)>> = Vec::new();
    let mut start_at: HashMap<i64, usize> = HashMap::new();
    let mut ends: HashMap<i64, i64> = HashMap::new();
    for (i, w) in ex.coast.iter().enumerate() {
        let mut seq = vec![i];
        let (mut s, mut e) = (w.first, w.last);
        while e != s {
            let Some(slot) = start_at.remove(&e) else { break };
            let (_, e2, sq) = chains[slot].take().unwrap();
            ends.remove(&e2);
            seq.extend(sq);
            e = e2;
        }
        while s != e {
            let Some(s0) = ends.remove(&s) else { break };
            let slot = start_at.remove(&s0).unwrap();
            let (_, _, sq) = chains[slot].take().unwrap();
            let mut t = sq;
            t.extend(seq);
            seq = t;
            s = s0;
        }
        match start_at.get(&s) {
            Some(&slot) => chains[slot] = Some((s, e, seq)),
            None => {
                start_at.insert(s, chains.len());
                chains.push(Some((s, e, seq)));
            }
        }
        ends.insert(e, s);
    }
    let (mut land, mut water) = (Vec::new(), Vec::new());
    for ch in chains.iter().flatten() {
        let mut pts: Vec<(f64, f64)> = Vec::new();
        for (k, &i) in ch.2.iter().enumerate() {
            let r = &ex.coast[i].ring;
            pts.extend(fpts(if k == 0 { &r[..] } else { &r[1..] }));
        }
        if pts.len() < 4 {
            continue;
        }
        // lon/lat orientation: y points north
        let n = pts.len();
        let mut a = 0.0;
        for i in 0..n - 1 {
            a += pts[i].0 * -pts[i + 1].1 - pts[i + 1].0 * -pts[i].1;
        }
        a += pts[n - 1].0 * -pts[0].1 - pts[0].0 * -pts[n - 1].1;
        let g = Geom::polygon(&pts, &[])?.valid()?;
        if a > 0.0 {
            land.extend(g.polygons()?);
        } else {
            water.extend(g.polygons()?);
        }
    }
    Ok((land, water))
}

/// Cell box minus the land of the coastline within `clip` (None: whole cell).
fn sea_of_cell(
    cb: &Geom,
    clip: Option<&Geom>,
    land: (&[Geom], &Tree),
    water: (&[Geom], &Tree),
) -> Result<Geom> {
    let b = cb.bounds();
    let part = |ps: (&[Geom], &Tree)| -> Result<Geom> {
        let idx = ps.1.query(cb);
        if idx.is_empty() {
            return Geom::empty();
        }
        let clipped: Vec<Geom> = idx
            .iter()
            .map(|&i| ps.0[i].clip_by_rect(b.0, b.1, b.2, b.3))
            .collect::<Result<_>>()?;
        geos::union_all(&clipped)
    };
    let ground = part(land)?.difference(&part(water)?)?;
    let inside = match clip {
        None => cb.clone_geom()?,
        Some(p) => p.clip_by_rect(b.0, b.1, b.2, b.3)?,
    };
    inside.difference(&ground)?.simplify(TOLERANCE)
}

/// Even-odd combination of many rings.  Pairwise in a tree: the small islands
/// are combined with each other first and only once with the large sea ring.
fn xor_all(mut gs: Vec<Geom>) -> Result<Geom> {
    if gs.is_empty() {
        return Geom::empty();
    }
    while gs.len() > 1 {
        let odd = gs.len() % 2 == 1;
        let mut next = Vec::with_capacity(gs.len() / 2 + 1);
        for i in 0..gs.len() / 2 {
            next.push(gs[2 * i].sym_difference(&gs[2 * i + 1])?);
        }
        if odd {
            next.push(gs.pop().unwrap());
        }
        gs = next;
    }
    Ok(gs.pop().unwrap())
}

/// Every plaintext C record of a chart with its cell and slot.
fn c_records(c: &Chart) -> Result<Vec<(i64, i64, i64, i64, usize, Vec<u8>)>> {
    let mut out = Vec::new();
    for t in c.tiles() {
        if t.start == t.end {
            continue;
        }
        let rk = chart::record_key(&c.data, t.start, &c.key);
        for s in layers::slots(&c.data, &t, 'C') {
            let from = t.start + s.rec.rel + 8;
            let raw = chart::decode_record(&c.data[from..from + s.rec.len], s.rec.pltx, &rk)?;
            out.push((t.x as i64, t.y as i64, s.cx as i64, s.cy as i64, s.slot, raw));
        }
    }
    Ok(out)
}

/// Sea (#OW) of the original file per cell; its rings are combined with the
/// even-odd rule, like the firmware fills them.
fn original_sea(recs: &[(i64, i64, i64, i64, usize, Vec<u8>)]) -> Result<HashMap<(i64, i64), Geom>> {
    let mut sea = HashMap::new();
    for (_, _, cx, cy, _, raw) in recs {
        for it in layers::parse_c(raw)?.get("c6", &C_SPEC) {
            let Var::U16(name) = &it.v[0] else { bail!("c6 name") };
            if u16str(name) != "#OW" {
                continue;
            }
            let Var::U32(geom) = &it.v[1] else { bail!("c6 geometry") };
            let mut gs = Vec::new();
            for (_, pts) in layers::geometry_parts(geom) {
                if pts.len() >= 4 {
                    let ring: Vec<(f64, f64)> = pts
                        .iter()
                        .map(|q| {
                            (
                                (cx * CELL + (q & 0xFFFF) as i64) as f64,
                                (cy * CELL + (q >> 16) as i64) as f64,
                            )
                        })
                        .collect();
                    gs.push(Geom::polygon(&ring, &[])?.valid()?);
                }
            }
            sea.insert((*cx, *cy), xor_all(gs)?);
        }
    }
    Ok(sea)
}

fn packed(cell: (i64, i64), x: i64, y: i64) -> u32 {
    (((y - cell.1 * CELL) << 16) | (x - cell.0 * CELL)) as u32
}

/// One area object: name, class and the rings as a geometry array.
pub fn make_object(
    cell: (i64, i64),
    name: &str,
    cls: Option<u32>,
    rings: &[(u32, Vec<(i64, i64)>)],
    arr: &str,
) -> Item {
    let mut geom: Vec<u32> = Vec::new();
    let (mut x0, mut y0, mut x1, mut y1) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
    for (hi, r) in rings {
        geom.push(hi << 16 | r.len() as u32);
        for &(x, y) in r {
            geom.push(packed(cell, x, y));
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    let nm = u16enc(name);
    let (lo, hi_) = (packed(cell, x0, y0), packed(cell, x1, y1));
    let s = if arr == "c5" {
        vec![0, nm.len() as u32, cls.unwrap_or(0), lo, hi_, geom.len() as u32, 0]
    } else {
        vec![0, nm.len() as u32, lo, hi_, geom.len() as u32, 0]
    };
    Item { s, v: vec![Var::U16(nm), Var::U32(geom)] }
}

/// Plaintext C record from the objects of one cell.
pub fn record(cell: (i64, i64), objs: &BTreeMap<&str, Vec<Item>>) -> Vec<u8> {
    let empty: Vec<Item> = Vec::new();
    let get = |a: &str| objs.get(a).unwrap_or(&empty);
    let size = |a: &str| if a == "c5" { 0x1C } else { 0x18 };
    let first = ["c5", "c6", "c3"].into_iter().find(|a| !get(a).is_empty()).unwrap();
    let mut hdr = vec![0u32; C_HDR];
    hdr[0] = (get(first).len() * size(first)) as u32;
    hdr[1] = cell.1 as u32;
    hdr[7] = get("c3").len() as u32;
    hdr[11] = get("c5").len() as u32;
    hdr[13] = get("c6").len() as u32;
    let rec = ArrayRec {
        hdr,
        arrays: vec![
            Vec::new(),
            Vec::new(),
            get("c3").clone(),
            Vec::new(),
            get("c5").clone(),
            get("c6").clone(),
        ],
    };
    layers::build_arrays(&rec, &C_SPEC)
}

/// The boundary polygon of a .poly file in osmarea units.
pub fn boundary(rings: &[crate::poly::Ring]) -> Result<Geom> {
    boundary_at(rings, 8.0)
}

/// The same with a divisor: the `.poly` files are read in 2^28 units, the
/// osmarea layer wants 2^25 (divisor 8), the osm layer 2^28 (divisor 1).
pub fn boundary_at(rings: &[crate::poly::Ring], div: f64) -> Result<Geom> {
    let mut outer = Vec::new();
    let mut holes = Vec::new();
    for r in rings {
        let pts: Vec<(f64, f64)> = r.pts.iter().map(|&(x, y)| (x / div, y / div)).collect();
        let g = Geom::polygon(&pts, &[])?;
        if r.hole {
            holes.push(g);
        } else {
            outer.push(g);
        }
    }
    let p = geos::union_all(&outer)?;
    if holes.is_empty() {
        p.difference(&Geom::empty()?)
    } else {
        p.difference(&geos::union_all(&holes)?)
    }
}

/// Build the whole layer.  `orig` supplies the sea outside the boundary and the
/// tiles that lie completely outside it; `land` replaces the coastline sea with
/// the worldwide land polygons (land_extract.py) for countries without one.
#[allow(clippy::too_many_arguments)]
pub fn build(
    ex: &Extract,
    p: &Geom,
    orig: Option<&Chart>,
    land: Option<Vec<Geom>>,
    date: &[u8],
    country: u32,
    device: &[u8],
    log: &dyn Fn(&str),
) -> Result<Vec<u8>> {
    let t0 = std::time::Instant::now();
    let since = |s: String| format!("{} ({:.0} s)", s, t0.elapsed().as_secs_f32());
    let groups = load(ex, p, &|s| log(&since(s.to_string())))?;

    let mut blocks: BTreeMap<(Key, i64, i64), Vec<(u32, Vec<(i64, i64)>)>> = BTreeMap::new();
    for (key, lst) in &groups {
        let merged = dissolve(lst)?;
        for q in &merged {
            for (b, hi, r) in cut(q, level(q.bounds()), p)? {
                blocks.entry((*key, b.0, b.1)).or_default().push((hi, r));
            }
        }
        log(&since(format!("  {}: {} -> {} merged", key_str(*key), lst.len(), merged.len())));
    }

    // tiles: all tiles of the original plus those touching the boundary; tiles
    // with no cell inside the boundary (the Faroe Islands) are copied
    let orecs = match orig {
        Some(c) => c_records(c)?,
        None => Vec::new(),
    };
    let otiles: BTreeSet<(i64, i64)> = orecs.iter().map(|r| (r.0, r.1)).collect();
    let (x0, y0, x1, y1) = p.bounds();
    let mut ptiles: BTreeSet<(i64, i64)> = BTreeSet::new();
    for cx in (x0 as i64).div_euclid(CELL)..=(x1 as i64).div_euclid(CELL) {
        for cy in (y0 as i64).div_euclid(CELL)..=(y1 as i64).div_euclid(CELL) {
            let cb = Geom::rect(
                (cx * CELL) as f64,
                (cy * CELL) as f64,
                (cx * CELL + CELL) as f64,
                (cy * CELL + CELL) as f64,
            )?;
            if p.intersects(&cb) {
                ptiles.insert((cx.div_euclid(G), cy.div_euclid(G)));
            }
        }
    }
    let mut copied: HashMap<(i64, i64), BTreeMap<usize, Vec<u8>>> = HashMap::new();
    for (tx, ty, _, _, slot, raw) in &orecs {
        if otiles.contains(&(*tx, *ty)) && !ptiles.contains(&(*tx, *ty)) {
            copied.entry((*tx, *ty)).or_default().insert(*slot, raw.clone());
        }
    }

    let (land, water, clip) = match land {
        // worldwide land polygons: sea over the whole cell
        Some(l) => (l, Vec::new(), None),
        None => {
            let (l, w) = land_polygons(ex)?;
            (l, w, Some(p))
        }
    };
    let (lt, wt) = (Tree::new(&land)?, Tree::new(&water)?);
    let sea_old = original_sea(&orecs)?;
    log(&since(format!(
        "sea built ({} land, {} water, {} cells from the original)",
        land.len(),
        water.len(),
        sea_old.len()
    )));

    let mut cells: HashMap<(i64, i64), BTreeMap<&str, Vec<(usize, i64, i64, Option<u32>, &Vec<(u32, Vec<(i64, i64)>)>)>>> =
        HashMap::new();
    for ((key, bx, by), rings) in &blocks {
        let (arr, cls) = *key;
        let order = cls.map_or(0, |c| CLASS_ORDER.iter().position(|&o| o == c).unwrap());
        cells
            .entry((bx.div_euclid(8), by.div_euclid(8)))
            .or_default()
            .entry(arr)
            .or_default()
            .push((order, by.rem_euclid(8), bx.rem_euclid(8), cls, rings));
    }

    let mut content: Vec<TileContent> = Vec::new();
    // the original directory starts with an empty placeholder tile (0,0)
    content.push(TileContent { x: 0, y: 0, areas: None, tail: Vec::new() });
    let empty: BTreeMap<&str, Vec<(usize, i64, i64, Option<u32>, &Vec<(u32, Vec<(i64, i64)>)>)>> =
        BTreeMap::new();
    let mut tiles: Vec<(i64, i64)> = otiles.union(&ptiles).copied().collect();
    tiles.sort();
    for (tx, ty) in tiles {
        if let Some(raw) = copied.get(&(tx, ty)) {
            let mut areas = Areas::new();
            areas.insert('C', raw.clone());
            content.push(TileContent {
                x: tx as u16,
                y: ty as u16,
                areas: Some(areas),
                tail: Vec::new(),
            });
            continue;
        }
        let mut slots: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
        for k in 0..(G * G) as usize {
            let cell = (tx * G + (k as i64) / G, ty * G + (k as i64) % G);
            let cb = Geom::rect(
                (cell.0 * CELL) as f64,
                (cell.1 * CELL) as f64,
                (cell.0 * CELL + CELL) as f64,
                (cell.1 * CELL + CELL) as f64,
            )?;
            let old = match sea_old.get(&cell) {
                Some(g) => g.difference(p)?,
                None => Geom::empty()?,
            };
            let sea = sea_of_cell(&cb, clip, (&land, &lt), (&water, &wt))?.union(&old)?;
            let mut objs: BTreeMap<&str, Vec<Item>> = BTreeMap::new();
            let here = cells.get(&cell).unwrap_or(&empty);
            for arr in ["c3", "c5", "c6"] {
                let mut lst = here.get(arr).cloned().unwrap_or_default();
                lst.sort_by_key(|o| (o.0, o.1, o.2));
                objs.insert(
                    arr,
                    lst.iter().map(|o| make_object(cell, "", o.3, o.4, arr)).collect(),
                );
            }
            let mut rings: Vec<(u32, Vec<(i64, i64)>)> = Vec::new();
            for q in sea.polygons()? {
                let ext = orient(q.exterior(), true);
                let mut parts = vec![ext];
                for h in q.interiors() {
                    if Geom::polygon(&h, &[])?.area() >= MIN_AREA {
                        parts.push(orient(h, false));
                    }
                }
                for r in &parts {
                    if let Some(r) = ring_points(r) {
                        rings.push((SEA_LEVEL, r));
                    }
                }
            }
            if !rings.is_empty() {
                objs.get_mut("c6")
                    .unwrap()
                    .insert(0, make_object(cell, "#OW", None, &rings, "c6"));
            }
            if objs.values().any(|v| !v.is_empty()) {
                slots.insert(k, record(cell, &objs));
            }
        }
        let mut areas = Areas::new();
        areas.insert('C', slots);
        content.push(TileContent {
            x: tx as u16,
            y: ty as u16,
            areas: Some(areas),
            tail: Vec::new(),
        });
    }
    log(&since(format!("writing {} tiles", content.len())));
    let meta = Meta {
        date: date.to_vec(),
        typ: 1,
        layer: LAYER_OSMAREA,
        country,
        tail_tile: None,
    };
    writer::write_chart(&meta, &content, device, true, None)
}
