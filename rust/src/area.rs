//! Areas (closed ways and multipolygons) and the coastline from a .osm.pbf,
//! the input of the osmarea layer.  Port of tools/osm_area_extract.py.
//!
//! Coordinates are the osmarea units straight away (360/2^25 degrees, rounded
//! half to even like numpy's rint), not the 2^28 units of the other layers.
//! Rings follow libosmium's conventions (see pbf.rs): they start at their
//! smallest vertex, repeat it at the end, and outer and inner are decided by
//! nesting depth -- here the direction matters too, outer rings run clockwise
//! in Teasi coordinates (counter-clockwise in lon/lat), holes the other way.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use osmpbf::PrimitiveBlock;

use crate::pbf::{assemble_segments, par_blocks, split_rings, NodeIndex};

/// Teasi osmarea units: 360 degrees in 2^25 steps.
pub const SCALE: f64 = (1u64 << 25) as f64 / 360.0;

/// Only areas carrying one of these keys are kept.
const KEYS: [&str; 10] = [
    "landuse", "natural", "leisure", "man_made", "waterway", "water", "military", "aeroway",
    "amenity", "tourism",
];

/// Decimicrodegrees -> osmarea units.
pub fn xy25(dm_lon: i32, dm_lat: i32) -> (f64, f64) {
    (
        (f64::from(dm_lon) / 1e7 + 180.0) * SCALE,
        (90.0 - f64::from(dm_lat) / 1e7) * SCALE,
    )
}

fn rint(dm: (i32, i32)) -> (i32, i32) {
    let (x, y) = xy25(dm.0, dm.1);
    (x.round_ties_even() as i32, y.round_ties_even() as i32)
}

pub type Ring = Vec<(i32, i32)>;

/// The tags the osmarea compiler reads.
#[derive(Clone, Default, Debug)]
pub struct ATags {
    /// `area=no` forbids an area, however the other tags look
    pub no_area: bool,
    pub landuse: Option<Box<str>>,
    pub natural: Option<Box<str>>,
    pub water: Option<Box<str>>,
    pub leisure: Option<Box<str>>,
    pub man_made: Option<Box<str>>,
    pub waterway: Option<Box<str>>,
}

impl ATags {
    /// The wanted values plus "has one of KEYS at all".
    pub fn of<'a, I: Iterator<Item = (&'a str, &'a str)>>(tags: I) -> (ATags, bool) {
        let (mut t, mut any) = (ATags::default(), false);
        for (k, v) in tags {
            any |= KEYS.contains(&k);
            t.no_area |= k == "area" && v == "no";
            let slot = match k {
                "landuse" => &mut t.landuse,
                "natural" => &mut t.natural,
                "water" => &mut t.water,
                "leisure" => &mut t.leisure,
                "man_made" => &mut t.man_made,
                "waterway" => &mut t.waterway,
                _ => continue,
            };
            *slot = Some(v.into());
        }
        (t, any)
    }

    pub fn coastline(&self) -> bool {
        self.natural.as_deref() == Some("coastline")
    }
}

/// One outer ring with its holes.
#[derive(Clone, Debug)]
pub struct Poly {
    pub outer: Ring,
    pub inners: Vec<Ring>,
}

/// An osmium area: id = 2 * way id, or 2 * relation id + 1.
#[derive(Clone, Debug)]
pub struct Area {
    pub id: i64,
    pub tags: ATags,
    pub polys: Vec<Poly>,
}

/// A `natural=coastline` way with its end nodes, so that chains can be joined.
#[derive(Clone, Debug)]
pub struct Coast {
    pub id: i64,
    pub first: i64,
    pub last: i64,
    pub ring: Ring,
    /// `place=islet`: a small island, drawn like a pier
    pub islet: bool,
}

#[derive(Default)]
pub struct Extract {
    pub areas: Vec<Area>,
    pub coast: Vec<Coast>,
}

/// Signed area of a closed location ring, in lon/lat orientation: positive is
/// counter-clockwise, which is how libosmium turns its outer rings.  The test
/// has to run on the locations -- rounded to osmarea units a tiny area collapses
/// and its direction would be decided by rounding alone.
fn signed_area(r: &[(i32, i32)]) -> f64 {
    let mut s = 0.0;
    for i in 0..r.len() {
        let (x1, y1) = (f64::from(r[i].0), f64::from(r[i].1));
        let (x2, y2) = (f64::from(r[(i + 1) % r.len()].0), f64::from(r[(i + 1) % r.len()].1));
        s += x1 * y2 - x2 * y1;
    }
    s / 2.0
}

/// Turn a ring the way libosmium hands it out, keeping its first vertex.
fn orient(mut r: Ring, ccw: f64, outer: bool) -> Ring {
    if (ccw > 0.0) != outer {
        let n = r.len();
        r[1..n - 1].reverse();
    }
    r
}

/// Node ids -> their decimicrodegree locations, None if one is missing.
fn locs(ids: &[i64], index: &NodeIndex) -> Option<Vec<(i32, i32)>> {
    ids.iter().map(|id| index.get_dm(*id)).collect()
}

/// A ring's locations -> its points, rotated to the smallest vertex and closed.
/// The comparison is on the decimicrodegree integers, as in libosmium.
fn ring_of(dm: &[(i32, i32)]) -> Option<((i32, i32), f64, Ring)> {
    let at = (0..dm.len()).min_by_key(|&i| dm[i])?;
    let mut pts: Ring = dm[at..].iter().chain(dm[..at].iter()).map(|&d| rint(d)).collect();
    pts.push(pts[0]);
    Some((dm[at], signed_area(dm), pts))
}

/// Crossing number test against a closed ring.
fn inside(ring: &[(i32, i32)], p: (i32, i32)) -> bool {
    let (x, y) = (f64::from(p.0), f64::from(p.1));
    let mut c = false;
    for i in 0..ring.len() - 1 {
        let (x1, y1) = (f64::from(ring[i].0), f64::from(ring[i].1));
        let (x2, y2) = (f64::from(ring[i + 1].0), f64::from(ring[i + 1].1));
        if (y1 > y) != (y2 > y) && x < x1 + (y - y1) * (x2 - x1) / (y2 - y1) {
            c = !c;
        }
    }
    c
}

/// Locations of the member ways -> the area's polygons: assembled from the
/// segments, split where a ring touches itself, grouped by nesting depth.
fn rings_of(ways: &[Vec<(i32, i32)>]) -> Option<Vec<Poly>> {
    let rings = assemble_segments(ways)?;
    let split: Vec<Vec<(i32, i32)>> = rings.iter().flat_map(|r| split_rings(r)).collect();
    Some(group(split.iter().map(|r| ring_of(r)).collect::<Option<Vec<_>>>()?))
}

/// Rings of one area -> outer rings with their holes, as osmium groups them:
/// rings in the order of their smallest vertex, nesting depth decides the role.
fn group(mut rings: Vec<((i32, i32), f64, Ring)>) -> Vec<Poly> {
    rings.sort_by_key(|(start, _, _)| *start);
    // two rings touching in a vertex make the test point ambiguous, so take the
    // first point of the ring that is not a corner of the ring tested against
    let is_in = |r: &Ring, o: &Ring| -> bool {
        let p = r.iter().take(r.len() - 1).find(|p| !o.contains(p)).unwrap_or(&r[0]);
        inside(o, *p)
    };
    let depth: Vec<usize> = (0..rings.len())
        .map(|i| {
            (0..rings.len()).filter(|&j| j != i && is_in(&rings[i].2, &rings[j].2)).count()
        })
        .collect();
    let mut polys: Vec<Poly> = Vec::new();
    let mut at: HashMap<usize, usize> = HashMap::new(); // ring index -> poly index
    for i in 0..rings.len() {
        if depth[i] % 2 == 0 {
            at.insert(i, polys.len());
            polys.push(Poly {
                outer: orient(rings[i].2.clone(), rings[i].1, true),
                inners: Vec::new(),
            });
        }
    }
    for i in 0..rings.len() {
        if depth[i] % 2 == 0 {
            continue;
        }
        // the hole belongs to the innermost ring around it, which is its parent
        let parent = (0..rings.len())
            .filter(|&j| j != i && depth[j] + 1 == depth[i] && is_in(&rings[i].2, &rings[j].2))
            .max_by_key(|&j| depth[j]);
        if let Some(p) = parent.and_then(|j| at.get(&j)) {
            polys[*p].inners.push(orient(rings[i].2.clone(), rings[i].1, false));
        }
    }
    polys
}

/// A way or relation whose geometry is still missing.
struct Cand {
    id: i64,
    tags: ATags,
    /// node ids (way) or member way ids (relation)
    refs: Vec<i64>,
}

#[derive(Default)]
struct Pass2 {
    ways: Vec<Cand>,
    coast: Vec<(i64, Vec<i64>, bool)>,
    members: Vec<(i64, Vec<i64>)>,
}

/// Everything the osmarea layer needs from the file.
pub fn extract(path: &str, log: &dyn Fn(&str)) -> Result<Extract> {
    let clock = std::time::Instant::now();
    let lap = || {
        let t = clock.elapsed().as_secs_f32();
        move |s: String| format!("{} [{:.1} s]", s, clock.elapsed().as_secs_f32() - t)
    };

    // pass 1: the relations osmium builds areas from
    let since = lap();
    let rels: Vec<Cand> = par_blocks(
        path,
        Vec::new,
        |acc: &mut Vec<Cand>, block: &PrimitiveBlock| {
            for group in block.groups() {
                for r in group.relations() {
                    let ty = r.tags().find(|(k, _)| *k == "type").map(|(_, v)| v);
                    if ty != Some("multipolygon") && ty != Some("boundary") {
                        continue;
                    }
                    let (tags, any) = ATags::of(r.tags());
                    if !any || tags.coastline() || tags.no_area {
                        continue;
                    }
                    let mut seen = HashSet::new();
                    let refs: Vec<i64> = r
                        .members()
                        .filter(|m| m.member_type == osmpbf::RelMemberType::Way)
                        .map(|m| m.member_id)
                        .filter(|id| seen.insert(*id))
                        .collect();
                    if !refs.is_empty() {
                        acc.push(Cand { id: r.id(), tags, refs });
                    }
                }
            }
        },
        |mut a, b| {
            a.extend(b);
            a
        },
    )?;
    let wanted: HashSet<i64> = rels.iter().flat_map(|r| r.refs.iter().copied()).collect();
    log(&since(format!("pass 1: {} relations, {} member ways", rels.len(), wanted.len())));

    // pass 2: closed ways that are areas, the coastline, and the member ways
    let since = lap();
    let p2 = par_blocks(
        path,
        Pass2::default,
        |acc: &mut Pass2, block: &PrimitiveBlock| {
            for group in block.groups() {
                for w in group.ways() {
                    let member = wanted.contains(&w.id());
                    let (tags, any) = ATags::of(w.tags());
                    let coast = tags.coastline();
                    if !member && !any {
                        continue; // the common case: do not even collect the nodes
                    }
                    let refs: Vec<i64> = w.refs().collect();
                    if member {
                        acc.members.push((w.id(), refs.clone()));
                    }
                    if coast {
                        if refs.len() > 1 {
                            let islet =
                                w.tags().any(|(k, v)| k == "place" && v == "islet");
                            acc.coast.push((w.id(), refs, islet));
                        }
                        continue; // coastline ways are no areas
                    }
                    if any && !tags.no_area && refs.len() > 3 && refs[0] == *refs.last().unwrap()
                    {
                        acc.ways.push(Cand { id: w.id(), tags, refs });
                    }
                }
            }
        },
        |mut a, b| {
            a.ways.extend(b.ways);
            a.coast.extend(b.coast);
            a.members.extend(b.members);
            a
        },
    )?;
    log(&since(format!(
        "pass 2: {} closed ways, {} coastline ways, {} member ways",
        p2.ways.len(),
        p2.coast.len(),
        p2.members.len()
    )));

    // pass 3: the coordinates of all those nodes
    let since = lap();
    let mut ids: Vec<i64> = Vec::new();
    for w in &p2.ways {
        ids.extend(w.refs.iter().copied());
    }
    for (_, refs, _) in &p2.coast {
        ids.extend(refs.iter().copied());
    }
    for (_, refs) in &p2.members {
        ids.extend(refs.iter().copied());
    }
    let mut index = NodeIndex::new(ids);
    let pos = par_blocks(
        path,
        Vec::new,
        |acc: &mut Vec<(i64, i32, i32)>, block: &PrimitiveBlock| {
            let mut probe = index.probe();
            for group in block.groups() {
                for n in group.nodes() {
                    if probe.wants(n.id()) {
                        acc.push((n.id(), n.decimicro_lon(), n.decimicro_lat()));
                    }
                }
                for n in group.dense_nodes() {
                    if probe.wants(n.id()) {
                        acc.push((n.id(), n.decimicro_lon(), n.decimicro_lat()));
                    }
                }
            }
        },
        |mut a, b| {
            a.extend(b);
            a
        },
    )?;
    for (id, lon, lat) in &pos {
        index.set(*id, *lon, *lat);
    }
    log(&since(format!(
        "pass 3: {} of {} way nodes located",
        index.len() - index.missing(),
        index.len()
    )));

    // geometry: closed ways are one ring, relations are assembled
    let since = lap();
    let mut out = Extract::default();
    for w in &p2.ways {
        let Some(dm) = locs(&w.refs, &index) else { continue };
        let Some(polys) = rings_of(&[dm]) else { continue };
        if !polys.is_empty() {
            out.areas.push(Area { id: 2 * w.id, tags: w.tags.clone(), polys });
        }
    }
    let n_ways = out.areas.len();
    let members: HashMap<i64, &Vec<i64>> =
        p2.members.iter().map(|(id, refs)| (*id, refs)).collect();
    let mut open = 0usize;
    for r in &rels {
        let ways: Vec<Vec<i64>> =
            r.refs.iter().filter_map(|id| members.get(id).map(|v| (*v).clone())).collect();
        let Some(dm) = ways.iter().map(|r| locs(r, &index)).collect::<Option<Vec<_>>>() else {
            open += 1;
            continue;
        };
        let Some(polys) = rings_of(&dm) else {
            open += 1;
            continue;
        };
        if !polys.is_empty() {
            out.areas.push(Area { id: 2 * r.id + 1, tags: r.tags.clone(), polys });
        } else {
            open += 1;
        }
    }
    for (id, refs, islet) in &p2.coast {
        let pts: Ring = refs.iter().filter_map(|i| index.get_dm(*i)).map(rint).collect();
        out.coast.push(Coast {
            id: *id,
            first: refs[0],
            last: *refs.last().unwrap(),
            ring: pts,
            islet: *islet,
        });
    }
    log(&since(format!(
        "geometry: {} way areas, {} relation areas ({} without rings), {} coastline ways",
        n_ways,
        out.areas.len() - n_ways,
        open,
        out.coast.len()
    )));
    Ok(out)
}
