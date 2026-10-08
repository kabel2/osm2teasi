//! The osmpoint layer: seamarks from OSM / OpenSeaMap; format in
//! docs/OSMPOINT_FORMAT.md.
//!
//! The rules were calibrated against the original Danish file, see that
//! document's "Aus OSM erzeugen".  Everything here reads `seamark:*` tags, whose
//! key set is open-ended (`seamark:light:3:colour`), so the objects arrive with
//! all their tags in an [`osm::TagMap`] rather than a fixed array.

use std::collections::BTreeMap;

use anyhow::Result;

use crate::chart;
use crate::layers::{build_osmpoint, u16enc, PointObj, PointRec, Sector, LAYER_OSMPOINT};
use crate::pbf::TagMap;
use crate::osmpoi::round_even;
use crate::poi::Cand;
use crate::poly::{self, Ring};
use crate::writer::{self, Areas, Meta, TileContent};

pub const G: usize = 4; // slot area C: 4 x 4 cells per tile
pub const CELL: i64 = 32768; // units per cell (360/2^25 deg)

fn lk(table: &[(&str, u32)], v: Option<&str>) -> Option<u32> {
    let v = v?;
    table.iter().find(|(k, _)| *k == v).map(|(_, n)| *n)
}

/// Python's `str.capitalize()`: first character upper case, the rest lower.
fn capitalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, c) in s.chars().enumerate() {
        if i == 0 {
            out.extend(c.to_uppercase());
        } else {
            out.extend(c.to_lowercase());
        }
    }
    out
}

/// Python's `str.title()`: every run of letters starts with an upper-case one.
fn title(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_word = false;
    for c in s.chars() {
        if in_word {
            out.extend(c.to_lowercase());
        } else {
            out.extend(c.to_uppercase());
        }
        in_word = c.is_alphabetic();
    }
    out
}

/// Python's `num()`: a number without a trailing ".0", anything else unchanged.
fn num(v: &str) -> String {
    match v.parse::<f64>() {
        Ok(f) if f == f.trunc() && f.abs() < 1e15 => format!("{}", f as i64),
        Ok(f) => format!("{}", f),
        Err(_) => v.to_string(),
    }
}

// --------------------------------------------------------------------------
// category [2] = group << 16 | sub << 8 | class
// --------------------------------------------------------------------------
// class: buoy 0, beacon 1, light 3, landmark 4, harbour 6, anchorage 7,
// mooring 8, wreck 9, rock 0xC, bridge 0xD, radio station 0xE, signal station
// 0xF, platform 0x10, production area 0x11.  sub: shape or category.

const BUOY_SHAPE: [(&str, u32); 7] = [
    ("conical", 0),
    ("can", 1),
    ("spherical", 2),
    ("pillar", 3),
    ("spar", 4),
    ("barrel", 5),
    ("super-buoy", 6),
];
const BEACON_SHAPE: [(&str, u32); 6] =
    [("stake", 0), ("pole", 0), ("post", 0), ("tower", 2), ("pile", 3), ("lattice", 3)];
const LANDMARK: [(&str, u32); 20] = [
    ("cairn", 0),
    ("cemetery", 1),
    ("chimney", 2),
    ("dish_aerial", 3),
    ("flagstaff", 4),
    ("flare_stack", 5),
    ("mast", 6),
    ("windsock", 7),
    ("monument", 8),
    ("column", 9),
    ("memorial", 10),
    ("obelisk", 11),
    ("statue", 12),
    ("cross", 13),
    ("dome", 14),
    ("radar_scanner", 15),
    ("tower", 16),
    ("windmill", 17),
    ("windmotor", 18),
    ("spire", 19),
];
const HARBOUR: [(&str, u32); 3] =
    [("fishing", 0), ("marina", 1), ("marina_no_facilities", 2)];
const MOORING: [(&str, u32); 7] = [
    ("dolphin", 0),
    ("deviation_dolphin", 1),
    ("bollard", 2),
    ("wall", 3),
    ("post", 4),
    ("pile", 4),
    ("buoy", 5),
];
const ROCK: [(&str, u32); 2] = [("covers", 0), ("awash", 1)];
const WRECK: [(&str, u32); 3] = [("non-dangerous", 0), ("dangerous", 1), ("hull_showing", 2)];

/// A seamark's own tags: `seamark:<type>:<key>`.
fn own<'a>(t: &'a TagMap, ty: &str, key: &str) -> Option<&'a str> {
    t.get(&format!("seamark:{}:{}", ty, key))
}

pub fn category(ty: &str, t: &TagMap) -> Option<u32> {
    let kind = ty.split('_').next().unwrap_or("");
    if ty.starts_with("buoy_") || ty.starts_with("beacon_") {
        let group = match ty.split('_').nth(1) {
            Some("lateral") | Some("cardinal") => 0xA,
            Some("safe") => 0x9,
            _ => 0xC,
        };
        if kind == "buoy" {
            return Some(group << 16 | lk(&BUOY_SHAPE, own(t, ty, "shape")).unwrap_or(3) << 8);
        }
        return Some(group << 16 | lk(&BEACON_SHAPE, own(t, ty, "shape")).unwrap_or(3) << 8 | 1);
    }
    let cat = own(t, ty, "category");
    Some(match ty {
        "light_major" => 0x09_0003,
        "light_minor" => 0x09_0103,
        "landmark" => {
            let c = cat.unwrap_or("").split(';').next().unwrap_or("");
            match lk(&LANDMARK, Some(c)) {
                Some(n) => 0x0A_0004 | n << 8,
                None => 0x09_0003,
            }
        }
        "harbour" => 0x09_0006 | lk(&HARBOUR, cat).unwrap_or(3) << 8,
        "anchorage" => 0x0E_0007,
        "bridge" => 0x0E_000D,
        "mooring" => 0x0F_0008 | lk(&MOORING, cat).unwrap_or(0) << 8,
        "rock" => 0x0F_000C | lk(&ROCK, own(t, ty, "water_level")).unwrap_or(2) << 8,
        "wreck" => 0x0F_0009 | lk(&WRECK, cat).unwrap_or(1) << 8,
        "radio_station" => 0x0C_000E,
        "platform" => 0x0B_0010,
        "production_area" => 0x0B_0011,
        "light_vessel" => 0x0C_0202,
        _ if ty.starts_with("signal_station") => 0x0C_000F,
        _ => return None,
    })
}

// --------------------------------------------------------------------------
// body colour [4] and topmark [5]
// --------------------------------------------------------------------------

const COLOUR: [(&str, u32); 7] = [
    ("grey", 0),
    ("white", 1),
    ("black", 2),
    ("red", 3),
    ("green", 4),
    ("blue", 5),
    ("yellow", 8),
];
const TOPSHAPE: [(&str, u32); 16] = [
    ("cone, point up", 1),
    ("cone, point down", 2),
    ("sphere", 3),
    ("2 spheres", 4),
    ("cylinder", 5),
    ("board", 6),
    ("x-shape", 7),
    ("upright cross", 8),
    ("2 cones point together", 10),
    ("2 cones base together", 11),
    ("rhombus", 12),
    ("2 cones up", 13),
    ("2 cones down", 14),
    ("square", 17),
    ("triangle, point up", 18),
    ("triangle, point down", 19),
];

/// All known colours of a `;`-separated list; only the first two are stored.
fn colours(v: Option<&str>) -> Vec<u32> {
    v.unwrap_or("").split(';').filter_map(|c| lk(&COLOUR, Some(c))).collect()
}

/// [4]: bits 0-3 pattern, 4-6 number of colours, then 4-bit colours from bit 7.
pub fn body_colour(ty: &str, t: &TagMap) -> u32 {
    if !(ty.starts_with("buoy_") || ty.starts_with("beacon_")) {
        return 0;
    }
    let all = colours(own(t, ty, "colour"));
    if all.is_empty() {
        return 0;
    }
    // the only pattern the original uses
    let pattern = u32::from(own(t, ty, "colour_pattern") == Some("vertical")) * 4;
    let mut v = pattern | (all.len() as u32) << 4;
    for (i, c) in all.iter().take(2).enumerate() {
        v |= c << (7 + 4 * i);
    }
    v
}

/// [5]: bits 0-6 shape, 7-8 number of colours, then 4-bit colours from bit 9.
pub fn topmark(t: &TagMap) -> u32 {
    let shape = lk(&TOPSHAPE, t.get("seamark:topmark:shape")).unwrap_or(0);
    let all = colours(t.get("seamark:topmark:colour"));
    let mut v = shape | (all.len().min(3) as u32) << 7;
    for (i, c) in all.iter().take(2).enumerate() {
        v |= c << (9 + 4 * i);
    }
    v
}

// --------------------------------------------------------------------------
// texts
// --------------------------------------------------------------------------

const BUOY_KIND: [(&str, &str); 5] = [
    ("lateral", "Lateral"),
    ("cardinal", "Cardinal"),
    ("safe_water", "Safe Water"),
    ("special_purpose", "Special Purpose"),
    ("isolated_danger", "Isolated Danger"),
];
const LATERAL: [(&str, &str); 4] = [
    ("port", "Port-hand "),
    ("starboard", "Starboard-hand "),
    ("preferred_channel_port", "Preferred Channel to Port "),
    ("preferred_channel_starboard", "Preferred Channel to Starboard "),
];
const SPECIAL: [(&str, &str); 12] = [
    ("anchorage", "Anchorage mark "),
    ("cable", "Cable mark "),
    ("diving", "Diving mark "),
    ("leading", "Leading mark "),
    ("mooring", "Mooring mark "),
    ("no_entry", "Entry prohibited mark "),
    ("notice", "Notice mark "),
    ("outfall", "Outfall mark "),
    ("pipeline", "Pipeline mark "),
    ("unknown_purpose", "Mark with unknown purpose "),
    ("warning", "General warning mark "),
    ("yachting", "Yachting mark "),
];
const HARBOUR_TXT: [(&str, &str); 13] = [
    ("bulk", "Bulk terminal"),
    ("cargo", "General cargo terminal"),
    ("container", "Container terminal"),
    ("ferry", "Ferry terminal"),
    ("fishing", "Fishing harbour"),
    ("lay_up", "Lay up vessels berth"),
    ("marina", "Yacht harbour/marina"),
    ("marina_no_facilities", "Yacht berths without facilities"),
    ("naval", "Naval base"),
    ("passenger", "Passenger terminal"),
    ("roro", "RoRo-terminal"),
    ("service_repair", "Service and repair"),
    ("shipyard", "Shipyard"),
];
const MOORING_TXT: [(&str, &str); 6] = [
    ("dolphin", "Dolphin"),
    ("bollard", "Bollard"),
    ("wall", "Tie-up wall"),
    ("post", "Post or pile"),
    ("pile", "Post or pile"),
    ("buoy", "Mooring buoy"),
];
const WRECK_TXT: [(&str, &str); 3] = [
    ("dangerous", "Dangerous wreck"),
    ("non-dangerous", "Non-dangerous wreck"),
    ("hull_showing", "Wreck showing any portion of hull or superstructure"),
];
const ANCHORAGE_TXT: [(&str, &str); 2] = [
    ("unrestricted", "Unrestricted anchorage"),
    ("24_hour", "Anchorage for periods up to 24 hours"),
];

fn txt<'a>(table: &[(&str, &'a str)], v: Option<&str>) -> Option<&'a str> {
    let v = v?;
    table.iter().find(|(k, _)| *k == v).map(|(_, s)| *s)
}

fn light_category(t: &TagMap) -> Option<&str> {
    t.non_empty("seamark:light:category").or_else(|| t.non_empty("seamark:light:1:category"))
}

/// Attribute 19.
pub fn description(ty: &str, t: &TagMap) -> Option<String> {
    let cat = own(t, ty, "category").filter(|v| !v.is_empty());
    if ty.starts_with("buoy_") || ty.starts_with("beacon_") {
        let (kind, sub) = ty.split_once('_').unwrap();
        let base = format!("{} {}", txt(&BUOY_KIND, Some(sub)).unwrap_or(sub), capitalize(kind));
        return Some(match sub {
            "lateral" => txt(&LATERAL, cat).unwrap_or("").to_string() + &base,
            "cardinal" => match cat {
                Some(c) => format!("{} {}", capitalize(c), base),
                None => base,
            },
            "special_purpose" => txt(&SPECIAL, cat).unwrap_or("").to_string() + &base,
            _ => base,
        });
    }
    Some(match ty {
        "light_major" | "light_minor" => {
            let base = ty.replace('_', " ");
            match light_category(t) {
                Some(lc) => {
                    let lc: Vec<String> = lc
                        .split(';')
                        .map(|x| if x == "floodlight" { "flood".into() } else { x.replace('_', " ") })
                        .collect();
                    format!("{} {}", capitalize(&lc.join(",")), base)
                }
                None => capitalize(&base),
            }
        }
        "landmark" => capitalize(cat?.split(';').next().unwrap_or("")),
        "harbour" => {
            let cs: Vec<&str> =
                cat.unwrap_or("").split(';').filter_map(|x| txt(&HARBOUR_TXT, Some(x))).collect();
            match cs.len() {
                0 => "Harbour".to_string(),
                1 => format!("{}\n", cs[0]),
                _ => cs.iter().map(|x| format!("- {}\n", x)).collect(),
            }
        }
        "mooring" => txt(&MOORING_TXT, cat).unwrap_or("Dolphin").to_string(),
        "rock" => "Rock".to_string(),
        "wreck" => txt(&WRECK_TXT, cat)?.to_string(),
        "anchorage" => txt(&ANCHORAGE_TXT, cat)?.to_string(),
        "bridge" => match cat {
            Some(c) => format!("{} bridge", capitalize(c)),
            None => "Bridge".to_string(),
        },
        "platform" => match cat {
            Some(c) => format!("Platform({})", c),
            None => "Platform".to_string(),
        },
        "radio_station" => match cat {
            Some("dgps") => "Differential GPS".to_string(),
            _ => "Radio station".to_string(),
        },
        "signal_station_traffic" => match cat {
            Some(c) => format!("Traffic Signal Station: {}", title(&c.replace('_', " "))),
            None => "Traffic Signal Station".to_string(),
        },
        "signal_station_warning" => "Warning Signal Stations".to_string(),
        "light_vessel" => "Light vessel".to_string(),
        _ => return None,
    })
}

// --------------------------------------------------------------------------
// lights: seamark:light:* or seamark:light:<n>:*
// --------------------------------------------------------------------------

const LETTER: [(&str, &str); 7] = [
    ("white", "W"),
    ("red", "R"),
    ("green", "G"),
    ("yellow", "Y"),
    ("blue", "Bu"),
    ("orange", "Or"),
    ("violet", "Vi"),
];
const LETTER_ORDER: [&str; 7] = ["W", "R", "G", "Y", "Bu", "Or", "Vi"];

/// One light, keyed by what follows `seamark:light:` resp. `seamark:light:<n>:`.
pub type Light = TagMap;

pub fn lights(t: &TagMap) -> Vec<Light> {
    // numbered lights win; the keys are seamark:light:<n>:<key>
    let mut idx: Vec<u32> = t
        .iter()
        .filter_map(|(k, _)| {
            let rest = k.strip_prefix("seamark:light:")?;
            let (n, _) = rest.split_once(':')?;
            n.parse::<u32>().ok()
        })
        .collect();
    idx.sort_unstable();
    idx.dedup();
    if !idx.is_empty() {
        return idx
            .iter()
            .map(|i| {
                let prefix = format!("seamark:light:{}:", i);
                TagMap::of(
                    t.iter().filter_map(|(k, v)| k.strip_prefix(&prefix[..]).map(|r| (r, v))),
                )
            })
            .collect();
    }
    let one = TagMap::of(t.iter().filter_map(|(k, v)| {
        if k.starts_with("seamark:light:reference") {
            return None;
        }
        k.strip_prefix("seamark:light:").map(|r| (r, v))
    }));
    if one.iter().next().is_none() {
        Vec::new()
    } else {
        vec![one]
    }
}

/// Attribute 24 / label 5, e.g. `Fl.G.5s6m4M`, `Oc.WRG.12s9-12M`.
pub fn light_string(ls: &[Light]) -> String {
    let l0 = &ls[0];
    let mut cols: Vec<&str> = Vec::new();
    for l in ls {
        for c in l.get("colour").unwrap_or("").split(';') {
            if let Some(letter) = txt(&LETTER, Some(c)) {
                if !cols.contains(&letter) {
                    cols.push(letter);
                }
            }
        }
    }
    cols.sort_by_key(|c| LETTER_ORDER.iter().position(|o| o == c).unwrap());
    let mut s = format!("{}.{}", l0.get("character").unwrap_or(""), cols.concat());
    let mut rest = String::new();
    if let Some(p) = l0.non_empty("period") {
        rest += &(num(p) + "s");
    }
    if ls.len() == 1 && l0.get("sector_start").is_none() {
        if let Some(h) = l0.non_empty("height") {
            rest += &(num(h) + "m");
        }
    }
    // ranges in nautical miles, truncated; only plain numbers count
    let mut rs: Vec<f64> = ls
        .iter()
        .filter_map(|l| l.non_empty("range"))
        .filter(|r| r.chars().all(|c| c.is_ascii_digit() || c == '.'))
        .filter_map(|r| r.parse::<f64>().ok())
        .collect();
    rs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    rs.dedup();
    if !rs.is_empty() {
        let (lo, hi) = (rs[0] as i64, rs[rs.len() - 1] as i64);
        rest += &(if lo == hi { lo.to_string() } else { format!("{}-{}", lo, hi) } + "M");
    }
    if !rest.is_empty() {
        s += &format!(".{}", rest);
    }
    if let Some(cat) = ls[ls.len() - 1].non_empty("category") {
        let mut c = cat.chars();
        let first: String = c.next().unwrap().to_uppercase().collect();
        s += &format!("({}{})", first, c.as_str());
    }
    s
}

/// One sector per light, sorted by colour and then by the light's own order.
/// A light without a sector still produces an entry, with radius 0.
pub fn sectors(ls: &[Light]) -> Vec<(u32, f64, f64, u32, String)> {
    let mut out: Vec<(u32, usize, (u32, f64, f64, u32, String))> = Vec::new();
    for (i, l) in ls.iter().enumerate() {
        let col = match l.get("colour").unwrap_or("").split(';').next() {
            Some("amber") => 9,
            c => lk(&COLOUR, c).unwrap_or(0),
        };
        let angles = l
            .get("sector_start")
            .and_then(|a| a.parse::<f64>().ok())
            .zip(l.get("sector_end").and_then(|b| b.parse::<f64>().ok()));
        let Some((a, b)) = angles else {
            out.push((col, i, (col, 0.0, 0.0, 0, String::new())));
            continue;
        };
        // 4096 steps per turn, counted from south; stored in tenths of a degree
        let conv = |d: f64| ((d + 180.0).rem_euclid(360.0) * 4096.0 / 360.0).trunc() / 10.0;
        let mut lab = format!(
            "{}.{}",
            l.get("character").unwrap_or(""),
            txt(&LETTER, l.get("colour")).unwrap_or("")
        );
        if let Some(p) = l.non_empty("period") {
            lab += &format!(".{}s", num(p));
        }
        out.push((col, i, (col, conv(a), conv(b), 370, lab)));
    }
    out.sort_by_key(|x| (x.0, x.1));
    out.into_iter().map(|x| x.2).collect()
}

// --------------------------------------------------------------------------
// attribute string and label
// --------------------------------------------------------------------------

pub fn attributes(ty: &str, t: &TagMap, ls: &[Light]) -> String {
    let mut a: Vec<(&str, String)> = Vec::new();
    for (code, keys) in [
        ("01", &["website", "contact:website"][..]),
        ("02", &["phone", "contact:phone"]),
        ("04", &["internet_access"]),
        ("06", &["wheelchair"]),
        ("11", &["description"]),
        ("16", &["fax"]),
        ("17", &["email", "contact:email"]),
        ("18", &["operator"]),
    ] {
        if let Some(v) = t.first(keys) {
            a.push((code, v.to_string()));
        }
    }
    if let Some(d) = description(ty, t) {
        a.push(("19", d));
    }
    if let Some(v) = t.non_empty("seamark:information") {
        a.push(("20", v.to_string()));
    }
    if t.get("seamark:radar_transponder:category") == Some("racon") {
        a.push((
            "21",
            format!("Racon({})", t.get("seamark:radar_transponder:group").unwrap_or("")),
        ));
    }
    if let Some(fog) = t.non_empty("seamark:fog_signal:category") {
        let mut f = capitalize(fog);
        if let Some(g) = t.non_empty("seamark:fog_signal:group") {
            f += &format!("({})", g);
        }
        if let Some(p) = t.non_empty("seamark:fog_signal:period") {
            f += &(num(p) + "s");
        }
        a.push(("23", f));
    }
    if !ls.is_empty() {
        a.push(("24", light_string(ls)));
    }
    for (code, key) in [
        ("25", "seamark:light:reference"),
        ("26", &format!("seamark:{}:system", ty)[..]),
        ("27", "port_of_entry"),
    ] {
        if let Some(v) = t.non_empty(key) {
            a.push((code, v.to_string()));
        }
    }
    if let Some(v) = t.non_empty("seamark:landmark:height") {
        a.push(("35", num(v)));
    }
    if let Some(h) = ls.first().and_then(|l| l.non_empty("height")) {
        a.push(("36", num(h)));
    }
    if let Some(ex) = ls.first().and_then(|l| l.non_empty("exhibition")) {
        a.push(("37", ex.to_string()));
    }
    if ty == "rock" || ty == "wreck" {
        let wl = own(t, ty, "water_level").filter(|v| !v.is_empty());
        if wl.is_some() || ty == "rock" {
            a.push(("45", wl.unwrap_or("submerged").to_string()));
        }
    }
    a.iter().map(|(c, v)| format!("{}{}", c, v)).collect::<Vec<_>>().join("|")
}

/// The label line: racon and fog signal (code replaced by 0), then the rest.
pub fn label(ty: &str, t: &TagMap, ls: &[Light]) -> String {
    let all = attributes(ty, t, &[]);
    let mut fog: Vec<&str> =
        all.split('|').filter(|p| p.starts_with("21") || p.starts_with("23")).collect();
    fog.sort_unstable_by(|a, b| b.cmp(a));
    let mut parts: Vec<String> = fog.iter().map(|p| format!("0{}", &p[2..])).collect();
    if ty.starts_with("signal_station_warning") {
        parts.push("7SS".to_string());
    }
    if ty == "signal_station_traffic" {
        if let Some(c) = own(t, ty, "category").filter(|v| !v.is_empty()) {
            parts.push(format!("({})", title(&c.replace('_', " "))));
        }
    }
    if ty == "bridge" {
        if let Some(h) = t.non_empty("seamark:bridge:clearance_height") {
            parts.push(format!("4{}", num(h)));
        }
    }
    if !ls.is_empty() {
        parts.push(format!("5{}", light_string(ls)));
    }
    parts.join("|")
}

// --------------------------------------------------------------------------
// collecting and the chart
// --------------------------------------------------------------------------

/// One seamark as it goes into the chart; X, Y in 360/2^25 units.
#[derive(Clone, Debug)]
pub struct Point {
    pub x: i64,
    pub y: i64,
    pub cat: u32,
    pub col: u32,
    pub top: u32,
    pub name: String,
    pub attrs: String,
    pub label: String,
    /// colour, from, to (degrees), radius, label
    pub sectors: Vec<(u32, f64, f64, u32, String)>,
}

pub fn collect(cands: &[Cand], area: Option<&[Ring]>) -> Vec<Point> {
    let mut order: Vec<&Cand> = cands.iter().filter(|c| c.seamark.is_some()).collect();
    order.sort_by_key(|c| (c.way(), c.id));
    let mut out = Vec::new();
    for c in order {
        let t = c.seamark.as_ref().unwrap();
        let Some(ty) = t.non_empty("seamark:type") else { continue };
        let Some(cat) = category(ty, t) else { continue };
        // only a way uses its area centroid; a relation keeps the mean
        let (x, y) = match (c.kind, c.centre) {
            ('w', Some(p)) => p,
            _ => (c.x, c.y),
        };
        // 360/2^28 -> 360/2^25 units
        let (x, y) = (round_even(x / 8.0), round_even(y / 8.0));
        if let Some(rings) = area {
            if !poly::contains(rings, (x * 8) as f64, (y * 8) as f64) {
                continue;
            }
        }
        let ls = lights(t);
        out.push(Point {
            x,
            y,
            cat,
            col: body_colour(ty, t),
            top: topmark(t),
            name: t.get("seamark:name").unwrap_or("").to_string(),
            attrs: attributes(ty, t, &ls),
            label: label(ty, t, &ls),
            sectors: sectors(&ls),
        });
    }
    out
}

/// One cell's record: 12-word structs plus the strings.
fn record(points: &[&Point], cell_x: i64, cell_y: i64) -> Vec<u8> {
    let objs: Vec<PointObj> = points
        .iter()
        .map(|p| {
            let (name, attrs, label) = (u16enc(&p.name), u16enc(&p.attrs), u16enc(&p.label));
            let sectors: Vec<Sector> = p
                .sectors
                .iter()
                .map(|(col, from, to, radius, lab)| {
                    let text = u16enc(lab);
                    Sector {
                        w: vec![
                            *col as u16,
                            (from * 10.0).round_ties_even() as u16,
                            (to * 10.0).round_ties_even() as u16,
                            *radius as u16,
                        ],
                        a: 0,
                        len: text.len() as u32,
                        label: text,
                    }
                })
                .collect();
            let pos = (((p.y - cell_y * CELL) << 16) | (p.x - cell_x * CELL)) as u32;
            PointObj {
                s: vec![
                    0,
                    name.len() as u32,
                    p.cat,
                    pos,
                    p.col,
                    p.top,
                    0,
                    attrs.len() as u32,
                    0,
                    label.len() as u32,
                    sectors.len() as u32,
                    0,
                ],
                name,
                attrs,
                label,
                sectors,
            }
        })
        .collect();
    let hdr = vec![(objs.len() * 0x30) as u32, cell_y as u32, 0, objs.len() as u32, 0];
    build_osmpoint(&PointRec { hdr, objs })
}

/// Sort the seamarks into cells of 32768 units, the cells into tiles of 4 x 4.
pub fn build(points: &[Point], date: &[u8], country: u32, sign: &chart::Signer) -> Result<Vec<u8>> {
    let mut cells: BTreeMap<(i64, i64), Vec<&Point>> = BTreeMap::new();
    for p in points {
        cells.entry((p.x.div_euclid(CELL), p.y.div_euclid(CELL))).or_default().push(p);
    }
    let mut tiles: BTreeMap<(u16, u16), Areas> = BTreeMap::new();
    for ((cx, cy), lst) in &cells {
        let g = G as i64;
        let slot = (cx.rem_euclid(g) * g + cy.rem_euclid(g)) as usize;
        tiles
            .entry((cx.div_euclid(g) as u16, cy.div_euclid(g) as u16))
            .or_default()
            .entry('C')
            .or_default()
            .insert(slot, record(lst, *cx, *cy));
    }
    let content: Vec<TileContent> = tiles
        .into_iter()
        .map(|((x, y), areas)| TileContent { x, y, areas: Some(areas), tail: Vec::new() })
        .collect();
    let meta = Meta {
        date: date.to_vec(),
        typ: 1,
        layer: LAYER_OSMPOINT,
        country,
        tail_tile: None,
    };
    writer::write_chart(&meta, &content, sign, None)
}
