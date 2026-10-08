//! The osmpoi layer: tag rules, attribute string and the finished chart.
//! Port of tools/compile_osmpoi.py; format in docs/OSMPOI_FORMAT.md.

use std::collections::HashMap;

use anyhow::Result;

use crate::layers::{build_osmpoi, PoiItem, LAYER_OSMPOI};
use crate::poi::Cand;
use crate::poly::{self, Ring};
use crate::writer::{self, Areas, Meta, TileContent};

pub const G: usize = 32; // slot area D: 32 x 32 cells per tile
pub const CELL: i64 = 32768; // units per cell (360/2^28 deg)
pub const DEDUP: i64 = 500; // same type + name closer than this: keep one

// --------------------------------------------------------------------------
// the tags the rules and the attribute string look at
// --------------------------------------------------------------------------

/// Index into [`Tags`].  Only the keys the layer actually reads; the wider KEYS
/// list of tools/osm_poi_extract.py is a superset used for unfiltered extracts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum K {
    Amenity,
    Shop,
    Tourism,
    Leisure,
    Railway,
    Highway,
    Emergency,
    Religion,
    SeamarkType,
    ScfCategory,
    ManMade,
    TowerType,
    Bicycle,
    Aeroway,
    Aerialway,
    Historic,
    Name,
    Type,
    AddrStreet,
    AddrHousenumber,
    AddrPostcode,
    AddrCity,
    Website,
    Url,
    ContactWebsite,
    Phone,
    ContactPhone,
    OpeningHours,
    InternetAccess,
    Smoking,
    Wheelchair,
    Beds,
    Rooms,
    Cuisine,
    Capacity,
    Description,
    Fax,
    ContactFax,
    Email,
    ContactEmail,
    Operator,
    Access,
}

const NK: usize = K::Access as usize + 1;

/// Key name -> slot.  Everything else is dropped while the file is read.
fn key_index(k: &str) -> Option<usize> {
    use K::*;
    Some(match k {
        "amenity" => Amenity,
        "shop" => Shop,
        "tourism" => Tourism,
        "leisure" => Leisure,
        "railway" => Railway,
        "highway" => Highway,
        "emergency" => Emergency,
        "religion" => Religion,
        "seamark:type" => SeamarkType,
        "seamark:small_craft_facility:category" => ScfCategory,
        "man_made" => ManMade,
        "tower:type" => TowerType,
        "bicycle" => Bicycle,
        "aeroway" => Aeroway,
        "aerialway" => Aerialway,
        "historic" => Historic,
        "name" => Name,
        "type" => Type,
        "addr:street" => AddrStreet,
        "addr:housenumber" => AddrHousenumber,
        "addr:postcode" => AddrPostcode,
        "addr:city" => AddrCity,
        "website" => Website,
        "url" => Url,
        "contact:website" => ContactWebsite,
        "phone" => Phone,
        "contact:phone" => ContactPhone,
        "opening_hours" => OpeningHours,
        "internet_access" => InternetAccess,
        "smoking" => Smoking,
        "wheelchair" => Wheelchair,
        "beds" => Beds,
        "rooms" => Rooms,
        "cuisine" => Cuisine,
        "capacity" => Capacity,
        "description" => Description,
        "fax" => Fax,
        "contact:fax" => ContactFax,
        "email" => Email,
        "contact:email" => ContactEmail,
        "operator" => Operator,
        "access" => Access,
        _ => return None,
    } as usize)
}

/// The interesting tags of one object, borrowed from the PBF block.
pub struct Tags<'a>([Option<&'a str>; NK]);

impl<'a> Tags<'a> {
    pub fn get(&self, k: K) -> Option<&'a str> {
        self.0[k as usize]
    }

    /// Like Python's `t.get(k)` compared with a string.
    fn is(&self, k: K, v: &str) -> bool {
        self.get(k) == Some(v)
    }

    /// First of `keys` with a non-empty value, as `first()` does.
    fn first(&self, keys: &[K]) -> Option<&'a str> {
        keys.iter().filter_map(|&k| self.get(k)).find(|v| !v.is_empty())
    }

    pub fn name(&self) -> &'a str {
        self.get(K::Name).unwrap_or("")
    }

    /// libosmium's MultipolygonManager assembles both of these.
    pub fn multipolygon(&self) -> bool {
        matches!(self.get(K::Type), Some("multipolygon") | Some("boundary"))
    }
}

pub fn tags_of<'a, I: Iterator<Item = (&'a str, &'a str)>>(it: I) -> Tags<'a> {
    let mut t = Tags([None; NK]);
    for (k, v) in it {
        if let Some(i) = key_index(k) {
            t.0[i] = Some(v);
        }
    }
    t
}

// --------------------------------------------------------------------------
// tag -> POI type
// --------------------------------------------------------------------------

const RELIGION: [(&str, u8); 8] = [
    ("christian", 0x1D),
    ("muslim", 0x1E),
    ("buddhist", 0x1F),
    ("hindu", 0x20),
    ("shinto", 0x21),
    ("taoist", 0x22),
    ("sikh", 0x23),
    ("jewish", 0x24),
];

const SCF: [(&str, u8); 14] = [
    ("visitor_berth", 0x37),
    ("nautical_club", 0x38),
    ("boat_hoist", 0x39),
    ("boatyard", 0x3A),
    ("chandler", 0x3B),
    ("water_tap", 0x3C),
    ("fuel_station", 0x3D),
    ("electricity", 0x3E),
    ("showers", 0x3F),
    ("laundrette", 0x40),
    ("toilets", 0x41),
    ("pump-out", 0x42),
    ("slipway", 0x43),
    ("visitors_mooring", 0x44),
];

const AERIALWAY: [(&str, u8); 9] = [
    ("cable_car", 0x29),
    ("gondola", 0x2A),
    ("chair_lift", 0x2B),
    ("mixed_lift", 0x2C),
    ("t-bar", 0x2D),
    ("j-bar", 0x2E),
    ("platter", 0x2F),
    ("rope_tow", 0x30),
    ("magic_carpet", 0x31),
];

const HISTORIC_SKIP: [&str; 6] = ["railway", "hollow_way", "bunker", "wall", "aircraft", "no"];

/// (key, value, type); first match wins.
const RULES: [(K, &str, u8); 43] = [
    (K::Amenity, "fuel", 0x0B),
    (K::Amenity, "bicycle_rental", 0x26),
    (K::Shop, "bicycle", 0x01),
    (K::Amenity, "bank", 0x00),
    (K::Amenity, "atm", 0x03),
    (K::Amenity, "pharmacy", 0x0C),
    (K::Amenity, "doctors", 0x05),
    (K::Amenity, "hospital", 0x08),
    (K::Amenity, "police", 0x0D),
    (K::Emergency, "defibrillator", 0x06),
    (K::Amenity, "marketplace", 0x0A),
    (K::Amenity, "bus_station", 0x25),
    (K::Amenity, "cafe", 0x02),
    (K::Amenity, "bar", 0x02),
    (K::Amenity, "pub", 0x02),
    (K::Amenity, "restaurant", 0x0E),
    (K::Amenity, "fast_food", 0x0E),
    (K::Amenity, "shelter", 0x36),
    (K::Tourism, "museum", 0x49),
    (K::Tourism, "gallery", 0x49),
    (K::Tourism, "zoo", 0x4A),
    (K::Tourism, "aquarium", 0x4A),
    (K::Tourism, "attraction", 0x09),
    (K::Tourism, "viewpoint", 0x0F),
    (K::Tourism, "information", 0x13),
    (K::Tourism, "camp_site", 0x14),
    (K::Tourism, "hotel", 0x15),
    (K::Tourism, "hostel", 0x15),
    (K::Tourism, "motel", 0x15),
    (K::Tourism, "alpine_hut", 0x33),
    (K::Shop, "supermarket", 0x11),
    (K::Shop, "department_store", 0x04),
    (K::Shop, "convenience", 0x10),
    (K::Shop, "bakery", 0x10),
    (K::Shop, "butcher", 0x10),
    (K::Shop, "kiosk", 0x10),
    (K::Leisure, "sports_centre", 0x12),
    (K::Leisure, "stadium", 0x12),
    (K::Railway, "station", 0x1C),
    (K::Railway, "halt", 0x1C),
    (K::Railway, "tram_stop", 0x1A),
    (K::Railway, "subway_entrance", 0x1B),
    (K::Highway, "bus_stop", 0x19),
];

fn lookup(table: &[(&str, u8)], v: Option<&str>) -> Option<u8> {
    let v = v?;
    table.iter().find(|(k, _)| *k == v).map(|(_, t)| *t)
}

/// The POI type of an object, or None if it is not a POI.  `node` selects the
/// one rule that only applies to nodes.
pub fn poi_type(t: &Tags, node: bool) -> Option<u8> {
    // both of these return without falling through, as in the Python original
    if t.is(K::Amenity, "place_of_worship") {
        return lookup(&RELIGION, t.get(K::Religion));
    }
    if t.is(K::SeamarkType, "small_craft_facility") {
        return lookup(&SCF, t.get(K::ScfCategory));
    }
    for &(k, v, typ) in RULES.iter() {
        if t.is(k, v) {
            return Some(typ);
        }
    }
    if t.is(K::ManMade, "tower")
        && (t.is(K::TowerType, "observation") || t.is(K::Leisure, "bird_hide"))
    {
        return Some(0x0F);
    }
    if t.is(K::Amenity, "charging_station") && t.is(K::Bicycle, "yes") {
        return Some(0x28);
    }
    if t.is(K::Aeroway, "aerodrome") && node {
        return Some(0x27); // the original has only small airfields, mapped as nodes
    }
    if let Some(typ) = lookup(&AERIALWAY, t.get(K::Aerialway)) {
        return Some(typ);
    }
    if let Some(h) = t.get(K::Historic) {
        if !HISTORIC_SKIP.contains(&h) {
            return Some(0x48);
        }
    }
    None
}

/// Both values joined, empty ones dropped -- `" ".join(filter(None, ...))`.
fn join2(a: Option<&str>, b: Option<&str>, sep: &str) -> String {
    [a, b].iter().filter_map(|v| v.filter(|s| !s.is_empty())).collect::<Vec<_>>().join(sep)
}

/// `'NNvalue|NNvalue|...'` sorted by code (OSMPOI_FORMAT.md, "Attribut-String").
pub fn attributes(t: &Tags) -> String {
    let mut out: Vec<String> = Vec::new();
    let street = join2(t.get(K::AddrStreet), t.get(K::AddrHousenumber), " ");
    let city = join2(t.get(K::AddrPostcode), t.get(K::AddrCity), " ");
    if !street.is_empty() || !city.is_empty() {
        out.push(format!("00{}", join2(Some(&street), Some(&city), "\n")));
    }
    const ATTRS: [(&str, &[K]); 15] = [
        ("01", &[K::Website, K::Url, K::ContactWebsite]),
        ("02", &[K::Phone, K::ContactPhone]),
        ("03", &[K::OpeningHours]),
        ("04", &[K::InternetAccess]),
        ("05", &[K::Smoking]),
        ("06", &[K::Wheelchair]),
        ("07", &[K::Beds]),
        ("08", &[K::Rooms]),
        ("09", &[K::Cuisine]),
        ("10", &[K::Capacity]),
        ("11", &[K::Description]),
        ("16", &[K::Fax, K::ContactFax]),
        ("17", &[K::Email, K::ContactEmail]),
        ("18", &[K::Operator]),
        ("74", &[K::Access]),
    ];
    for (code, keys) in ATTRS.iter() {
        let Some(v) = t.first(keys) else { continue };
        let v = if *code == "03" { v.replace("; ", "\n").replace(';', "\n") } else { v.to_string() };
        out.push(format!("{}{}", code, v));
    }
    out.join("|")
}

// --------------------------------------------------------------------------
// collecting: position, country boundary, de-duplication
// --------------------------------------------------------------------------

/// What this layer takes from an object's tags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoiFields {
    pub typ: u8,
    pub name: String,
    pub attrs: String,
}

/// One POI as it goes into the chart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Poi {
    pub typ: u8,
    pub x: i64,
    pub y: i64,
    pub name: String,
    pub attrs: String,
}

/// Python's `round()`: halfway values go to the even side.
pub fn round_even(v: f64) -> i64 {
    v.round_ties_even() as i64
}

/// Sort, drop what is outside the boundary, then de-duplicate.
pub fn collect(cands: &[Cand], area: Option<&[Ring]>) -> Vec<Poi> {
    let mut pois: Vec<(bool, i64, u8, i64, i64, String, String)> = Vec::with_capacity(cands.len());
    for c in cands {
        let Some(f) = c.poi.as_ref() else { continue };
        // only a way uses its area centroid; a relation keeps the mean
        let (x, y) = match (c.kind, c.centre) {
            ('w', Some(p)) => p,
            _ => (c.x, c.y),
        };
        let (x, y) = (round_even(x), round_even(y));
        if let Some(rings) = area {
            if !poly::contains(rings, x as f64, y as f64) {
                continue;
            }
        }
        pois.push((c.way(), c.id, f.typ, x, y, f.name.clone(), f.attrs.clone()));
    }
    pois.sort();
    // same type and name within DEDUP units of an earlier one: drop it
    let mut near: HashMap<(u8, &str), Vec<(i64, i64)>> = HashMap::new();
    let mut keep = vec![true; pois.len()];
    for (i, p) in pois.iter().enumerate() {
        if p.5.is_empty() {
            continue;
        }
        let seen = near.entry((p.2, p.5.as_str())).or_default();
        if seen.iter().any(|(a, b)| (p.3 - a).abs() < DEDUP && (p.4 - b).abs() < DEDUP) {
            keep[i] = false;
        } else {
            seen.push((p.3, p.4));
        }
    }
    pois.into_iter()
        .zip(keep)
        .filter(|(_, k)| *k)
        .map(|(p, _)| Poi { typ: p.2, x: p.3, y: p.4, name: p.5, attrs: p.6 })
        .collect()
}

// --------------------------------------------------------------------------
// the chart
// --------------------------------------------------------------------------

/// Sort the POIs into cells of 32768 units, the cells into tiles of 32 x 32.
pub fn build(pois: &[Poi], date: &[u8], country: u32, device: &[u8]) -> Result<Vec<u8>> {
    let mut cells: HashMap<(i64, i64), Vec<PoiItem>> = HashMap::new();
    for p in pois {
        let (cx, cy) = (p.x.div_euclid(CELL), p.y.div_euclid(CELL));
        cells.entry((cx, cy)).or_default().push(PoiItem {
            typ: u32::from(p.typ),
            pos: (((p.y - cy * CELL) << 16) | (p.x - cx * CELL)) as u32,
            name: p.name.clone(),
            attrs: p.attrs.clone(),
        });
    }
    let mut tiles: std::collections::BTreeMap<(u16, u16), Areas> = Default::default();
    for ((cx, cy), items) in &cells {
        let g = G as i64;
        let slot = ((cx.rem_euclid(g)) * g + cy.rem_euclid(g)) as usize;
        tiles
            .entry(((cx.div_euclid(g)) as u16, (cy.div_euclid(g)) as u16))
            .or_default()
            .entry('D')
            .or_default()
            .insert(slot, build_osmpoi(items));
    }
    let content: Vec<TileContent> = tiles
        .into_iter()
        .map(|((x, y), areas)| TileContent { x, y, areas: Some(areas), tail: Vec::new() })
        .collect();
    let meta = Meta {
        date: date.to_vec(),
        typ: 1,
        layer: LAYER_OSMPOI,
        country,
        tail_tile: None,
    };
    writer::write_chart(&meta, &content, device, true, None)
}
