//! The ways the osm and ta layers are built from, read from a .osm.pbf.
//!
//! Only ways that [`crate::osm::road_class`] or [`crate::osm::line_type`]
//! accept are kept, and of those only what the compilers read: the attributes
//! are worked out from the tags while the file is read, and the tags are
//! dropped right there.  Kept as tags they were the biggest part of the memory
//! (about 500 bytes per way, against about 90 for the attributes).
//!
//! Two passes: the ways with their node ids, then the coordinates of those
//! nodes.  Route relations come along in the first pass.

use std::collections::HashMap;

use anyhow::Result;
use osmpbf::PrimitiveBlock;

use crate::osm::{self, LineType};
use crate::pbf::{par_blocks, xy_dm, NodeIndex, TagMap};

/// A route relation a way belongs to: `route`, `network`, `ref`.
pub type Route = (Box<str>, Option<Box<str>>, Option<Box<str>>);

/// `route=` values of the relations that set the network flags.
const ROUTES: [&str; 5] = ["bicycle", "mtb", "hiking", "foot", "ferry"];

/// What the compilers read of a way.
#[derive(Clone, Debug)]
pub struct Attr {
    /// road class, or -1 if the way is not a road
    pub cls: i8,
    /// roads: the attribute bits ([`osm::flags`]), route relations included
    pub flags: u32,
    pub cat: u8,
    /// roads: passable for bicycles forward and backward
    pub fwd: bool,
    pub bwd: bool,
    /// roads: the `name` tag as it is, "" without one
    pub name: Box<str>,
    /// the other ways: which line they are
    pub line: Option<LineType>,
}

impl Attr {
    fn of(t: &TagMap) -> Option<Attr> {
        if let Some(c) = osm::road_class(t) {
            let (fwd, bwd) = osm::passable(t, c);
            return Some(Attr {
                cls: c as i8,
                flags: osm::tag_flags(t),
                cat: osm::category(c, t) as u8,
                fwd,
                bwd,
                name: t.get("name").unwrap_or("").into(),
                line: None,
            });
        }
        let lt = osm::line_type(t)?;
        Some(Attr {
            cls: -1,
            flags: 0,
            cat: 0,
            fwd: false,
            bwd: false,
            name: "".into(),
            line: Some(lt),
        })
    }
}

/// The ways with one flat table of their node rows: row `i` belongs to way
/// `w[i]`, and the rows of a way are consecutive.
pub struct Ways {
    /// way ids, ascending
    pub ids: Vec<i64>,
    pub attr: Vec<Attr>,
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
    ways: Vec<(i64, Attr, Vec<i64>)>,
    rels: Vec<(i64, Route)>,
}

fn block(acc: &mut Acc, b: &PrimitiveBlock, keep: &(dyn Fn(&Attr) -> bool + Sync)) {
    for group in b.groups() {
        for w in group.ways() {
            let Some(a) = Attr::of(&TagMap::of(w.tags())) else {
                continue;
            };
            if keep(&a) {
                acc.ways.push((w.id(), a, w.refs().collect()));
            }
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

/// Read the ways `keep` accepts, then the coordinates of their nodes.
pub fn extract(
    path: &str,
    keep: &(dyn Fn(&Attr) -> bool + Sync),
    log: &dyn Fn(&str),
) -> Result<Ways> {
    let mut acc = par_blocks(
        path,
        Acc::default,
        |a: &mut Acc, b| block(a, b, keep),
        |mut a: Acc, b: Acc| {
            a.ways.extend(b.ways);
            a.rels.extend(b.rels);
            a
        },
    )?;
    // The ways are taken in file order, and the files from Geofabrik are
    // sorted by id.  The order decides how equal items sort inside a record,
    // so it is fixed here instead of inherited from the reader.
    acc.ways.sort_by_key(|(id, _, _)| *id);
    let mut rels: HashMap<i64, Vec<Route>> = HashMap::new();
    for (id, v) in acc.rels {
        rels.entry(id).or_default().push(v);
    }
    log(&format!(
        "{} ways, {} ways in routes",
        acc.ways.len(),
        rels.len()
    ));
    for (id, a, _) in acc.ways.iter_mut() {
        if a.cls >= 0 {
            a.flags |= osm::route_flags(rels.get(id).map_or(&[][..], |v| &v[..]));
        }
    }
    drop(rels);

    let mut ids: Vec<i64> = acc
        .ways
        .iter()
        .flat_map(|(_, _, r)| r.iter().copied())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let index = NodeIndex::new(ids);
    index.fill(path)?;
    log(&format!(
        "{} way nodes, {} without coordinates",
        index.len(),
        index.missing()
    ));

    let n = acc.ways.len();
    let rows: usize = acc.ways.iter().map(|(_, _, r)| r.len()).sum();
    let mut out = Ways {
        ids: Vec::with_capacity(n),
        attr: Vec::with_capacity(n),
        w: Vec::with_capacity(rows),
        nid: Vec::with_capacity(rows),
        x: Vec::with_capacity(rows),
        y: Vec::with_capacity(rows),
        rows: Vec::with_capacity(n),
    };
    for (k, (id, attr, refs)) in std::mem::take(&mut acc.ways).into_iter().enumerate() {
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
        out.attr.push(attr);
        out.rows.push((a, out.w.len() as u32));
    }
    Ok(out)
}

/// Decimicrodegrees -> global Teasi coordinates, as the extractor stores them.
pub fn xy(dm_lon: i32, dm_lat: i32) -> (f64, f64) {
    xy_dm(dm_lon, dm_lat)
}
