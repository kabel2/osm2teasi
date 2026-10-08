//! The ways the osm and ta layers are built from, read from a .osm.pbf.
//! Port of tools/osm_extract.py.
//!
//! Only ways that [`crate::osm::road_class`] or [`crate::osm::line_type`]
//! accept are kept -- that is what `--filter` does on the Python side, and it
//! is what Great Britain needs to fit into memory.  The other ways the Python
//! extractor keeps without the switch cannot reach the output: the compiler
//! looks at road and line ways only, and the node rows of the rest are never
//! indexed (verified below).
//!
//! Two passes: the ways with their node ids, then the coordinates of those
//! nodes.  Route relations come along in the first pass.

use std::collections::HashMap;

use anyhow::Result;
use osmpbf::PrimitiveBlock;

use crate::pbf::{par_blocks, xy_dm, NodeIndex, TagMap};

/// A route relation a way belongs to: `route`, `network`, `ref`.
pub type Route = (Box<str>, Option<Box<str>>, Option<Box<str>>);

/// `route=` values of the relations that set the network flags.
const ROUTES: [&str; 5] = ["bicycle", "mtb", "hiking", "foot", "ferry"];

/// The ways with one flat table of their node rows, as the Python pickle has
/// it: row `i` belongs to way `w[i]`, and the rows of a way are consecutive.
pub struct Ways {
    /// way ids, ascending
    pub ids: Vec<i64>,
    /// tags per way; the compiler drops them once it has the attributes
    pub tags: Vec<TagMap>,
    /// route relations by way id
    pub rels: HashMap<i64, Vec<Route>>,
    /// way index per node row
    pub w: Vec<u32>,
    /// node id per row
    pub nid: Vec<i64>,
    /// global Teasi coordinates (360/2^28 degrees) per row
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    /// first and last+1 row of each way (equal when it has no valid node)
    pub rows: Vec<(u32, u32)>,
}

impl Ways {
    pub fn nodes(&self, k: usize) -> std::ops::Range<usize> {
        let (a, b) = self.rows[k];
        a as usize..b as usize
    }
}

#[derive(Default)]
struct Acc {
    ways: Vec<(i64, TagMap, Vec<i64>)>,
    rels: Vec<(i64, Route)>,
}

fn block(acc: &mut Acc, b: &PrimitiveBlock) {
    for group in b.groups() {
        for w in group.ways() {
            let tags = TagMap::of(w.tags());
            if crate::osm::road_class(&tags).is_none() && crate::osm::line_type(&tags).is_none() {
                continue;
            }
            acc.ways.push((w.id(), tags, w.refs().collect()));
        }
        for r in group.relations() {
            let t = TagMap::of(r.tags());
            if t.get("type") != Some("route") {
                continue;
            }
            let Some(route) = t.get("route").filter(|v| ROUTES.contains(v)) else {
                continue;
            };
            let v: Route = (
                route.into(),
                t.get("network").map(Into::into),
                t.get("ref").map(Into::into),
            );
            for m in r.members() {
                if m.member_type == osmpbf::RelMemberType::Way {
                    acc.rels.push((m.member_id, v.clone()));
                }
            }
        }
    }
}

/// Read the ways, then the coordinates of their nodes.
pub fn extract(path: &str, log: &dyn Fn(String)) -> Result<Ways> {
    let mut acc = par_blocks(path, Acc::default, block, |mut a: Acc, b: Acc| {
        a.ways.extend(b.ways);
        a.rels.extend(b.rels);
        a
    })?;
    // The Python extractor takes the ways in the order libosmium hands them
    // over, which is the order of the file; the files from Geofabrik are sorted
    // by id.  The order decides how equal items sort inside a record, so it is
    // fixed here instead of inherited from the reader.
    acc.ways.sort_by_key(|(id, _, _)| *id);
    let mut rels: HashMap<i64, Vec<Route>> = HashMap::new();
    for (id, v) in acc.rels {
        rels.entry(id).or_default().push(v);
    }
    log(format!(
        "{} ways, {} ways in routes",
        acc.ways.len(),
        rels.len()
    ));

    let mut ids: Vec<i64> = acc.ways.iter().flat_map(|(_, _, r)| r.iter().copied()).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut index = NodeIndex::new(ids);
    index.fill(path)?;
    log(format!(
        "{} way nodes, {} without coordinates",
        index.len(),
        index.missing()
    ));

    let n = acc.ways.len();
    let mut out = Ways {
        ids: Vec::with_capacity(n),
        tags: Vec::with_capacity(n),
        rels,
        w: Vec::new(),
        nid: Vec::new(),
        x: Vec::new(),
        y: Vec::new(),
        rows: Vec::with_capacity(n),
    };
    for (k, (id, tags, refs)) in acc.ways.drain(..).enumerate() {
        let a = out.w.len() as u32;
        for r in refs {
            // nodes outside the extract have no location; libosmium skips them
            if let Some((x, y)) = index.get(r) {
                out.w.push(k as u32);
                out.nid.push(r);
                out.x.push(x);
                out.y.push(y);
            }
        }
        out.ids.push(id);
        out.tags.push(tags);
        out.rows.push((a, out.w.len() as u32));
    }
    Ok(out)
}

/// Decimicrodegrees -> global Teasi coordinates, as the extractor stores them.
pub fn xy(dm_lon: i32, dm_lat: i32) -> (f64, f64) {
    xy_dm(dm_lon, dm_lat)
}
