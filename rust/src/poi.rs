//! POI candidates from a .osm.pbf: tagged nodes, tagged ways and tagged
//! multipolygon relations.  Port of tools/osm_poi_extract.py --filter, which
//! keeps exactly the objects osmpoi::poi_type accepts.
//!
//! Three passes, as in addr.rs: a relation needs its member ways, those need
//! their nodes, and the PBF has nodes first, relations last.

use std::collections::HashSet;

use anyhow::Result;
use osmpbf::PrimitiveBlock;

use crate::osm::{area_centre, area_rings, mean, par_blocks, xy_dm, NodeIndex};
use crate::osmpoi::{attributes, poi_type, tags_of, Cand};

/// A way or relation we still need the geometry of.
#[derive(Clone, Debug)]
struct Cand2 {
    id: i64,
    typ: u8,
    name: String,
    attrs: String,
    /// node ids (way) or member way ids (relation)
    refs: Vec<i64>,
}

#[derive(Default)]
struct Pass2 {
    /// ways that are POIs themselves
    ways: Vec<Cand2>,
    /// node ids of the ways a wanted relation is made of
    members: Vec<(i64, Vec<i64>)>,
}

/// Area centroid of a closed ring (shoelace), None if degenerate.
pub fn centroid(pts: &[(f64, f64)]) -> Option<(f64, f64)> {
    let (x0, y0) = *pts.first()?;
    let (mut a, mut cx, mut cy) = (0.0f64, 0.0f64, 0.0f64);
    for i in 0..pts.len() {
        let (x1, y1) = pts[i];
        let (x2, y2) = pts[(i + 1) % pts.len()];
        let (x1, y1, x2, y2) = (x1 - x0, y1 - y0, x2 - x0, y2 - y0);
        let c = x1 * y2 - x2 * y1;
        a += c;
        cx += (x1 + x2) * c;
        cy += (y1 + y2) * c;
    }
    if a.abs() < 1e-9 {
        return None;
    }
    Some((x0 + cx / (3.0 * a), y0 + cy / (3.0 * a)))
}

/// Everything osmpoi needs from the file.
pub fn extract(path: &str, log: &dyn Fn(&str)) -> Result<Vec<Cand>> {
    let clock = std::time::Instant::now();
    let lap = || {
        let t = clock.elapsed().as_secs_f32();
        move |s: String| format!("{} [{:.1} s]", s, clock.elapsed().as_secs_f32() - t)
    };

    // pass 1: multipolygon relations that are POIs
    let since = lap();
    let rels: Vec<Cand2> = par_blocks(
        path,
        Vec::new,
        |acc: &mut Vec<Cand2>, block: &PrimitiveBlock| {
            for group in block.groups() {
                for r in group.relations() {
                    let t = tags_of(r.tags());
                    if !t.multipolygon() {
                        continue;
                    }
                    let Some(typ) = poi_type(&t, false) else { continue };
                    // every way member, once: libosmium ignores the roles too, and
                    // a way listed twice contributes its ring only once
                    let mut seen = HashSet::new();
                    let refs: Vec<i64> = r
                        .members()
                        .filter(|m| m.member_type == osmpbf::RelMemberType::Way)
                        .map(|m| m.member_id)
                        .filter(|id| seen.insert(*id))
                        .collect();
                    if !refs.is_empty() {
                        acc.push(Cand2 {
                            id: r.id(),
                            typ,
                            name: t.name().to_string(),
                            attrs: attributes(&t),
                            refs,
                        });
                    }
                }
            }
        },
        |mut a, b| {
            a.extend(b);
            a
        },
    )?;
    let wanted_ways: HashSet<i64> = rels.iter().flat_map(|r| r.refs.iter().copied()).collect();
    log(&since(format!("pass 1: {} POI relations, {} member ways", rels.len(), wanted_ways.len())));

    // pass 2: ways that are POIs, plus the member ways of those relations
    let since = lap();
    let p2 = par_blocks(
        path,
        Pass2::default,
        |acc: &mut Pass2, block: &PrimitiveBlock| {
            for group in block.groups() {
                for w in group.ways() {
                    let member = wanted_ways.contains(&w.id());
                    let t = tags_of(w.tags());
                    let typ = poi_type(&t, false);
                    if !member && typ.is_none() {
                        continue; // the common case: do not even collect the nodes
                    }
                    let refs: Vec<i64> = w.refs().collect();
                    if member {
                        acc.members.push((w.id(), refs.clone()));
                    }
                    if let Some(typ) = typ {
                        acc.ways.push(Cand2 {
                            id: w.id(),
                            typ,
                            name: t.name().to_string(),
                            attrs: attributes(&t),
                            refs,
                        });
                    }
                }
            }
        },
        |mut a, b| {
            a.ways.extend(b.ways);
            a.members.extend(b.members);
            a
        },
    )?;
    log(&since(format!("pass 2: {} POI ways, {} member ways found", p2.ways.len(), p2.members.len())));

    // pass 3: POI nodes and the coordinates the ways need
    let since = lap();
    let mut ids: Vec<i64> = Vec::new();
    for w in &p2.ways {
        ids.extend(w.refs.iter().copied());
    }
    for (_, refs) in &p2.members {
        ids.extend(refs.iter().copied());
    }
    let mut index = NodeIndex::new(ids);
    let (nodes, pos) = par_blocks(
        path,
        || (Vec::new(), Vec::new()),
        |acc: &mut (Vec<Cand>, Vec<(i64, i32, i32)>), block: &PrimitiveBlock| {
            let mut probe = index.probe();
            let mut one = |id: i64, lon: i32, lat: i32, t: &crate::osmpoi::Tags| {
                if probe.wants(id) {
                    acc.1.push((id, lon, lat));
                }
                if let Some(typ) = poi_type(t, true) {
                    let (x, y) = xy_dm(lon, lat);
                    acc.0.push(Cand {
                        kind: 'n',
                        id,
                        typ,
                        x,
                        y,
                        centre: None,
                        name: t.name().to_string(),
                        attrs: attributes(t),
                    });
                }
            };
            for group in block.groups() {
                for n in group.nodes() {
                    one(n.id(), n.decimicro_lon(), n.decimicro_lat(), &tags_of(n.tags()));
                }
                for n in group.dense_nodes() {
                    one(n.id(), n.decimicro_lon(), n.decimicro_lat(), &tags_of(n.tags()));
                }
            }
        },
        |mut a, b| {
            a.0.extend(b.0);
            a.1.extend(b.1);
            a
        },
    )?;
    for (id, lon, lat) in &pos {
        index.set(*id, *lon, *lat);
    }
    log(&since(format!(
        "pass 3: {} POI nodes, {} of {} way nodes located",
        nodes.len(),
        index.len() - index.missing(),
        index.len()
    )));

    // ways: mean of their points, plus the area centroid of the closed ones
    let since = lap();
    let mut out = nodes;
    let n_nodes = out.len();
    for w in &p2.ways {
        let mut pts: Vec<(f64, f64)> = w.refs.iter().filter_map(|id| index.get(*id)).collect();
        if pts.len() > 1 && w.refs[0] == *w.refs.last().unwrap() {
            pts.pop();
        }
        let Some((x, y)) = mean(&pts) else { continue };
        out.push(Cand {
            kind: 'w',
            id: w.id,
            typ: w.typ,
            x,
            y,
            centre: centroid(&pts),
            name: w.name.clone(),
            attrs: w.attrs.clone(),
        });
    }
    let n_ways = out.len() - n_nodes;

    // relations: the mean over the points of all assembled rings
    let members: std::collections::HashMap<i64, &Vec<i64>> =
        p2.members.iter().map(|(id, refs)| (*id, refs)).collect();
    let mut open = 0usize;
    for r in &rels {
        let ways: Vec<Vec<i64>> =
            r.refs.iter().filter_map(|id| members.get(id).map(|v| (*v).clone())).collect();
        let Some((x, y)) = area_rings(&ways, &index).as_deref().and_then(area_centre) else {
            open += 1;
            continue;
        };
        out.push(Cand {
            kind: 'r',
            id: r.id,
            typ: r.typ,
            x,
            y,
            centre: None,
            name: r.name.clone(),
            attrs: r.attrs.clone(),
        });
    }
    log(&since(format!(
        "geometry: {} ways, {} relations ({} without a closed ring)",
        n_ways,
        out.len() - n_nodes - n_ways,
        open
    )));
    Ok(out)
}
