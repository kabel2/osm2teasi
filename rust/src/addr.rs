//! Addresses, places and interpolation ways from a .osm.pbf, for the ta layer.
//! Port of tools/osm_addr_extract.py.
//!
//! Three passes over the file, because a relation's geometry needs its member
//! ways and those need their nodes:
//!   1. relations: which multipolygons carry address or place tags
//!   2. ways: interpolation ways, closed ways (areas) and the member ways
//!   3. nodes: the results that are nodes, the coordinates the ways need and
//!      the house numbers of the interpolation nodes

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use osmpbf::PrimitiveBlock;

use crate::osm::{par_blocks, ring_centre, xy_dm, NodeIndex};

/// place=* values the Python extractor keeps.
pub const PLACES: [&str; 13] = [
    "city", "town", "village", "hamlet", "suburb", "quarter", "neighbourhood", "locality",
    "isolated_dwelling", "borough", "island", "islet", "farm",
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AddrTags {
    pub hn: Option<String>,
    pub street: Option<String>,
    pub postcode: Option<String>,
    pub city: Option<String>,
    pub place: Option<String>,
    pub suburb: Option<String>,
}

impl AddrTags {
    /// addr:housenumber plus a street or a place is what makes an address.
    pub fn is_addr(&self) -> bool {
        self.hn.is_some() && (self.street.is_some() || self.place.is_some())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlaceTags {
    pub place: Option<String>,
    pub name: Option<String>,
    pub name_en: Option<String>,
    pub population: Option<String>,
    pub is_in: Option<String>,
}

impl PlaceTags {
    pub fn is_place(&self) -> bool {
        self.name.is_some() && self.place.as_deref().is_some_and(|p| PLACES.contains(&p))
    }
}

/// Both tag sets plus what is needed to recognise an interpolation way.
#[derive(Clone, Debug, Default)]
struct Tags {
    addr: AddrTags,
    place: PlaceTags,
    interpolation: Option<String>,
    any: bool,
    multipolygon: bool,
}

fn tags_of<'a, I: Iterator<Item = (&'a str, &'a str)>>(it: I) -> Tags {
    let mut t = Tags::default();
    for (k, v) in it {
        t.any = true;
        let s = || Some(v.to_string());
        match k {
            "addr:housenumber" => t.addr.hn = s(),
            "addr:street" => t.addr.street = s(),
            "addr:postcode" => t.addr.postcode = s(),
            "addr:city" => t.addr.city = s(),
            "addr:place" => t.addr.place = s(),
            "addr:suburb" => t.addr.suburb = s(),
            "addr:interpolation" => t.interpolation = s(),
            "place" => t.place.place = s(),
            "name" => t.place.name = s(),
            "name:en" => t.place.name_en = s(),
            "population" => t.place.population = s(),
            "is_in" => t.place.is_in = s(),
            // libosmium's MultipolygonManager assembles both of these
            "type" => t.multipolygon = v == "multipolygon" || v == "boundary",
            _ => {}
        }
    }
    t
}

#[derive(Clone, Debug)]
pub struct Addr {
    pub x: f64,
    pub y: f64,
    pub tags: AddrTags,
}

#[derive(Clone, Debug)]
pub struct Place {
    pub x: f64,
    pub y: f64,
    pub tags: PlaceTags,
    pub area: bool,
}

#[derive(Clone, Debug)]
pub struct Interp {
    pub kind: String,
    pub street: Option<String>,
    /// (x, y, house number, street) per node of the way
    pub pts: Vec<(f64, f64, Option<String>, Option<String>)>,
}

#[derive(Default)]
pub struct Extract {
    pub addr: Vec<Addr>,
    pub places: Vec<Place>,
    pub interp: Vec<Interp>,
}

/// A way we have to look at in pass 3, with its node ids.
#[derive(Clone, Debug)]
struct WayGeom {
    id: i64,
    tags: Tags,
    refs: Vec<i64>,
}

#[derive(Default)]
struct Pass2 {
    /// closed ways with tags: an area of their own
    areas: Vec<WayGeom>,
    /// addr:interpolation ways
    interp: Vec<WayGeom>,
    /// ways that are members of a wanted relation
    members: Vec<WayGeom>,
}

fn merge2(mut a: Pass2, b: Pass2) -> Pass2 {
    a.areas.extend(b.areas);
    a.interp.extend(b.interp);
    a.members.extend(b.members);
    a
}

/// Relations with address or place tags; their outer ways form the geometry.
#[derive(Clone, Debug)]
struct RelCand {
    tags: Tags,
    ways: Vec<i64>,
}

fn pass1(path: &str) -> Result<Vec<RelCand>> {
    par_blocks(
        path,
        Vec::new,
        |acc: &mut Vec<RelCand>, block: &PrimitiveBlock| {
            for group in block.groups() {
                for r in group.relations() {
                    let tags = tags_of(r.tags());
                    if !tags.multipolygon || !(tags.addr.is_addr() || tags.place.is_place()) {
                        continue;
                    }
                    let ways: Vec<i64> = r
                        .members()
                        .filter(|m| {
                            m.member_type == osmpbf::RelMemberType::Way
                                && matches!(m.role(), Ok("outer") | Ok(""))
                        })
                        .map(|m| m.member_id)
                        .collect();
                    if !ways.is_empty() {
                        acc.push(RelCand { tags, ways });
                    }
                }
            }
        },
        |mut a, b| {
            a.extend(b);
            a
        },
    )
}

fn pass2(path: &str, wanted_ways: &HashSet<i64>) -> Result<Pass2> {
    par_blocks(
        path,
        Pass2::default,
        |acc: &mut Pass2, block: &PrimitiveBlock| {
            for group in block.groups() {
                for w in group.ways() {
                    let member = wanted_ways.contains(&w.id());
                    let tags = tags_of(w.tags());
                    let tagged = tags.addr.is_addr() || tags.place.is_place();
                    if !member && !tagged && tags.interpolation.is_none() {
                        continue; // the common case: do not even collect the nodes
                    }
                    let refs: Vec<i64> = w.refs().collect();
                    let closed = refs.len() >= 4 && refs[0] == refs[refs.len() - 1];
                    let is_area = closed && tagged;
                    let g = WayGeom { id: w.id(), tags, refs };
                    if g.tags.interpolation.is_some() && g.refs.len() > 1 {
                        acc.interp.push(g.clone());
                    }
                    if is_area {
                        acc.areas.push(g.clone());
                    }
                    if member {
                        acc.members.push(g);
                    }
                }
            }
        },
        merge2,
    )
}

/// What pass 3 collects besides the coordinates.
#[derive(Default)]
struct Pass3 {
    addr: Vec<Addr>,
    places: Vec<Place>,
    pos: Vec<(i64, i32, i32)>,
    /// house number and street of the nodes an interpolation way uses
    numbers: Vec<(i64, Option<String>, Option<String>)>,
}

fn merge3(mut a: Pass3, b: Pass3) -> Pass3 {
    a.addr.extend(b.addr);
    a.places.extend(b.places);
    a.pos.extend(b.pos);
    a.numbers.extend(b.numbers);
    a
}

pub fn extract(path: &str, log: &dyn Fn(&str)) -> Result<Extract> {
    let clock = std::time::Instant::now();
    let lap = || {
        let t = clock.elapsed().as_secs_f32();
        move |s: String| format!("{} [{:.1} s]", s, clock.elapsed().as_secs_f32() - t)
    };

    let since = lap();
    let rels = pass1(path)?;
    let wanted_ways: HashSet<i64> = rels.iter().flat_map(|r| r.ways.iter().copied()).collect();
    log(&since(format!(
        "pass 1: {} area relations, {} member ways",
        rels.len(),
        wanted_ways.len()
    )));

    let since = lap();
    let p2 = pass2(path, &wanted_ways)?;
    log(&since(format!(
        "pass 2: {} closed ways, {} interpolation ways, {} member ways found",
        p2.areas.len(),
        p2.interp.len(),
        p2.members.len()
    )));

    let mut ids: Vec<i64> = Vec::new();
    for g in p2.areas.iter().chain(p2.interp.iter()).chain(p2.members.iter()) {
        ids.extend(g.refs.iter().copied());
    }
    let since = lap();
    let mut index = NodeIndex::new(ids);
    let interp_nodes: HashSet<i64> =
        p2.interp.iter().flat_map(|g| g.refs.iter().copied()).collect();

    let p3 = par_blocks(
        path,
        Pass3::default,
        |acc: &mut Pass3, block: &PrimitiveBlock| {
            let mut probe = index.probe();
            let mut one = |id: i64, dm_lon: i32, dm_lat: i32, tags: Tags| {
                if probe.wants(id) {
                    acc.pos.push((id, dm_lon, dm_lat));
                }
                if interp_nodes.contains(&id) && tags.addr.hn.is_some() {
                    acc.numbers.push((id, tags.addr.hn.clone(), tags.addr.street.clone()));
                }
                if !(tags.addr.is_addr() || tags.place.is_place()) {
                    return;
                }
                let (x, y) = xy_dm(dm_lon, dm_lat);
                if tags.addr.is_addr() {
                    acc.addr.push(Addr { x, y, tags: tags.addr });
                }
                if tags.place.is_place() {
                    acc.places.push(Place { x, y, tags: tags.place, area: false });
                }
            };
            for group in block.groups() {
                for n in group.nodes() {
                    one(n.id(), n.decimicro_lon(), n.decimicro_lat(), tags_of(n.tags()));
                }
                for n in group.dense_nodes() {
                    one(n.id(), n.decimicro_lon(), n.decimicro_lat(), tags_of(n.tags()));
                }
            }
        },
        merge3,
    )?;
    for (id, lon, lat) in &p3.pos {
        index.set(*id, *lon, *lat);
    }
    log(&since(format!(
        "pass 3: {} address nodes, {} place nodes, {} of {} way nodes located",
        p3.addr.len(),
        p3.places.len(),
        index.len() - index.missing(),
        index.len()
    )));

    let since = lap();

    let mut out = Extract { addr: p3.addr, places: p3.places, interp: Vec::new() };

    // areas: closed ways and assembled multipolygons
    let by_id: HashMap<i64, &WayGeom> = p2.members.iter().map(|g| (g.id, g)).collect();
    let mut areas: Vec<(&Tags, Vec<i64>)> =
        p2.areas.iter().map(|g| (&g.tags, g.refs.clone())).collect();
    let mut open_rings = 0;
    for r in &rels {
        let mut refs = Vec::new();
        let mut ends: HashMap<i64, usize> = HashMap::new();
        let mut complete = true;
        for w in &r.ways {
            match by_id.get(w) {
                Some(g) if g.refs.len() >= 2 => {
                    refs.extend(g.refs.iter().copied());
                    *ends.entry(g.refs[0]).or_default() += 1;
                    *ends.entry(g.refs[g.refs.len() - 1]).or_default() += 1;
                }
                _ => complete = false,
            }
        }
        // libosmium only builds an area when the member ways close into rings;
        // then every way end is shared by an even number of ways.  Without this
        // test, relations whose members are missing from the extract would
        // produce a centre from a torn ring.
        if !complete || refs.is_empty() || ends.values().any(|n| n % 2 != 0) {
            open_rings += 1;
            continue;
        }
        areas.push((&r.tags, refs));
    }
    log(&since(format!(
        "areas: {} from closed ways, {} from relations ({} rejected as open)",
        p2.areas.len(),
        areas.len() - p2.areas.len(),
        open_rings
    )));
    for (tags, refs) in areas {
        let pts: Vec<(f64, f64)> = refs.iter().filter_map(|id| index.get(*id)).collect();
        let Some((x, y)) = ring_centre(&pts) else { continue };
        if tags.addr.is_addr() {
            out.addr.push(Addr { x, y, tags: tags.addr.clone() });
        }
        if tags.place.is_place() {
            out.places.push(Place { x, y, tags: tags.place.clone(), area: true });
        }
    }

    // interpolation ways with the house numbers of their nodes
    let numbers: HashMap<i64, (Option<String>, Option<String>)> =
        p3.numbers.into_iter().map(|(id, hn, st)| (id, (hn, st))).collect();
    for g in &p2.interp {
        let pts: Vec<(f64, f64, Option<String>, Option<String>)> = g
            .refs
            .iter()
            .filter_map(|id| {
                let (x, y) = index.get(*id)?;
                let (hn, st) = numbers.get(id).cloned().unwrap_or((None, None));
                Some((x, y, hn, st))
            })
            .collect();
        if pts.len() > 1 {
            out.interp.push(Interp {
                kind: g.tags.interpolation.clone().unwrap_or_default(),
                street: g.tags.addr.street.clone(),
                pts,
            });
        }
    }
    Ok(out)
}
