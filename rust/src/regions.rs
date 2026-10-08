//! Geofabrik's regions mapped to the firmware's country codes: the list
//! `tools/build_world.sh` works through (`teasi regions > tools/regions.tsv`).
//!
//! A region is taken when its parent is a continent.  A country is replaced by
//! its parts when the firmware has a code of its own for every one of them, the
//! way it has for the Canadian provinces; the US states are listed under the
//! continent by Geofabrik already.  Names are matched loosely (case, accents and
//! punctuation do not count), first the full name, then the part before a
//! bracket.  Geofabrik's ISO tags are not used: they put Pitcairn under the
//! Marshall Islands and six Pacific regions under Vanuatu.

use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use unicode_normalization::UnicodeNormalization;

use crate::chart::COUNTRIES;

pub const INDEX: &str = "https://download.geofabrik.de/index-v1.json";
const BASE: &str = "https://download.geofabrik.de/";

const CONTINENTS: [&str; 9] = [
    "africa",
    "antarctica",
    "asia",
    "australia-oceania",
    "central-america",
    "europe",
    "north-america",
    "russia",
    "south-america",
];

/// Regions that cover ground other regions already cover, or that the
/// firmware has no single code for.
const SKIP: [(&str, &str); 14] = [
    ("alps", "overlaps eight countries"),
    ("dach", "overlaps Germany, Austria and Switzerland"),
    ("britain-and-ireland", "overlaps united-kingdom and ireland"),
    ("great-britain", "part of united-kingdom"),
    ("sea", "overlaps eleven countries"),
    (
        "south-africa-and-lesotho",
        "overlaps south-africa and lesotho",
    ),
    ("us", "the states are listed one by one"),
    ("us-midwest", "part of us"),
    ("us-northeast", "part of us"),
    ("us-pacific", "part of us"),
    ("us-south", "part of us"),
    ("us-west", "part of us"),
    ("ile-de-clipperton", "not in the firmware's list"),
    (
        "american-oceania",
        "several US territories at once, no single code",
    ),
];

/// Where Geofabrik's name does not meet the firmware's, the parts of a country
/// that have no code of their own, and the regions of several countries, which
/// get the code of the biggest one.
const ALIAS: [(&str, &str); 32] = [
    ("azores", "Portugal"),
    ("canary-islands", "Spain"),
    ("kaliningrad", "Russia"),
    ("central-fed-district", "Russia"),
    ("crimean-fed-district", "Russia"),
    ("far-eastern-fed-district", "Russia"),
    ("north-caucasus-fed-district", "Russia"),
    ("northwestern-fed-district", "Russia"),
    ("siberian-fed-district", "Russia"),
    ("south-fed-district", "Russia"),
    ("ural-fed-district", "Russia"),
    ("volga-fed-district", "Russia"),
    ("comores", "Comoros"),
    ("guernsey-jersey", "Channel Islands"),
    ("polynesie-francaise", "French Polynesia"),
    ("wallis-et-futuna", "Wallis and Futuna"),
    ("pitcairn-islands", "Pitcairn"),
    ("bosnia-herzegovina", "Bosnia and Herzegovina"),
    ("congo-brazzaville", "Congo"),
    ("congo-democratic-republic", "Democratic Republic of Congo"),
    ("laos", "Lao People s Democratic Republic"),
    ("macedonia", "FYR of Macedonia"),
    ("micronesia", "Micronesia, Federated States of"),
    // the firmware's spelling
    ("seychelles", "Seychlles"),
    ("us-virgin-islands", "Virgin Islands, U.S."),
    ("yukon", "Yukon Territory (Canada)"),
    ("gcc-states", "Saudi Arabia"),
    ("haiti-and-domrep", "Haiti"),
    ("ireland-and-northern-ireland", "Ireland"),
    ("israel-and-palestine", "Israel"),
    ("malaysia-singapore-brunei", "Malaysia"),
    ("senegal-and-gambia", "Senegal"),
];

/// Lower case ASCII letters and digits only: "Québec (Canada)" -> "quebeccanada".
fn norm(s: &str) -> String {
    s.nfkd()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// One line of regions.tsv.
pub struct Region {
    pub continent: String,
    /// Geofabrik path without `-latest.osm.pbf`, e.g. `europe/andorra`
    pub path: String,
    pub code: u32,
}

struct Feature {
    id: String,
    name: String,
    parent: Option<String>,
    path: Option<String>,
}

fn features(json: &str) -> Result<Vec<Feature>> {
    let v: serde_json::Value = serde_json::from_str(json).context("Geofabrik index")?;
    let Some(list) = v["features"].as_array() else {
        bail!("Geofabrik index: no features")
    };
    let s = |p: &serde_json::Value, k: &str| p[k].as_str().map(str::to_string);
    Ok(list
        .iter()
        .map(|f| {
            let p = &f["properties"];
            Feature {
                id: s(p, "id").unwrap_or_default(),
                name: s(p, "name").unwrap_or_default(),
                parent: s(p, "parent"),
                path: p["urls"]["pbf"].as_str().and_then(|u| {
                    Some(
                        u.strip_prefix(BASE)?
                            .strip_suffix("-latest.osm.pbf")?
                            .to_string(),
                    )
                }),
            }
        })
        .collect())
}

/// The regions of a Geofabrik index (the JSON of [`INDEX`]), sorted by
/// continent and path.
pub fn regions(json: &str) -> Result<Vec<Region>> {
    let feats = features(json)?;
    // later entries win, as they always have
    let mut by_name: HashMap<String, u32> = HashMap::new();
    let mut by_head: HashMap<String, u32> = HashMap::new();
    for &(name, code, _, _) in COUNTRIES.iter() {
        by_name.insert(norm(name), code);
        by_head.insert(norm(name.split('(').next().unwrap_or(name)), code);
    }
    let alias: HashMap<&str, &str> = ALIAS.iter().copied().collect();
    let skip = |id: &str| {
        SKIP.iter()
            .any(|(s, _)| *s == id || id.rsplit('/').next() == Some(*s))
    };
    let find = |n: &str| {
        by_name
            .get(&norm(n))
            .or_else(|| by_head.get(&norm(n)))
            .copied()
    };
    let code_of = |f: &Feature| -> Option<u32> {
        let short = f.id.rsplit('/').next().unwrap_or(&f.id);
        if f.id.starts_with("us/") {
            // a state, not the country of the same name
            if let Some(&c) = by_name.get(&norm(&format!("{} (United States)", short))) {
                return Some(c);
            }
        }
        if let Some(a) = alias.get(short) {
            return find(a);
        }
        find(&f.name).or_else(|| find(&short.replace('-', " ")))
    };
    // the parts of a country, when every one has a code "<Part> (<Country>)"
    let parts = |f: &Feature| -> Option<Vec<(&Feature, u32)>> {
        let country = COUNTRIES.iter().find(|c| Some(c.1) == code_of(f))?.0;
        let kids: Vec<&Feature> = feats
            .iter()
            .filter(|k| k.parent.as_deref() == Some(&*f.id))
            .collect();
        if kids.is_empty() {
            return None;
        }
        kids.into_iter()
            .map(|k| {
                let short = k.id.rsplit('/').next().unwrap_or(&k.id);
                let name = alias.get(short).map_or(k.name.clone(), |a| a.to_string());
                let full = if name.ends_with(')') {
                    name
                } else {
                    format!("{} ({})", name, country)
                };
                by_name.get(&norm(&full)).map(|&c| (k, c))
            })
            .collect()
    };

    let mut out = Vec::new();
    let mut open = Vec::new();
    for f in &feats {
        let Some(parent) = f.parent.as_deref().filter(|p| CONTINENTS.contains(p)) else {
            continue;
        };
        if skip(&f.id) {
            continue;
        }
        let Some(code) = code_of(f) else {
            open.push(format!("{} ({})", f.id, f.name));
            continue;
        };
        let list = parts(f).unwrap_or_else(|| vec![(f, code)]);
        for (g, code) in list {
            let Some(path) = &g.path else {
                bail!("{}: no pbf download in the index", g.id)
            };
            out.push(Region {
                continent: parent.to_string(),
                path: path.clone(),
                code,
            });
        }
    }
    if !open.is_empty() {
        bail!("no country code for: {}", open.join(", "));
    }
    out.sort_by(|a, b| (&a.continent, &a.path).cmp(&(&b.continent, &b.path)));
    Ok(out)
}

/// regions.tsv: a header and one line per region.
pub fn tsv(regions: &[Region]) -> String {
    let mut n: HashMap<u32, usize> = HashMap::new();
    for r in regions {
        *n.entry(r.code).or_default() += 1;
    }
    let mut s = String::from(
        "# Geofabrik's regions mapped to Teasi country codes.\n\
         # Columns: continent, Geofabrik path (without -latest.osm.pbf or .poly), the\n\
         # country code of the chart header, the country as the firmware names it, and\n\
         # the prefix of the chart file names.\n\
         #\n\
         # A '*' before the code marks one that several regions share.  The device keeps\n\
         # only one file per layer and country, so those cannot be installed next to each\n\
         # other: the ten Russian federal districts, the Azores (Portugal) and the Canary\n\
         # Islands (Spain).  A region of several countries carries the code of the\n\
         # biggest: gcc-states is Saudi Arabia, israel-and-palestine Israel.\n\
         #\n\
         # Generated by `teasi regions` (rust/src/regions.rs, which says what is left out\n\
         # and why).\n\
         #continent\tregion\tcode\tcountry\tprefix\n",
    );
    for r in regions {
        let &(name, _, _, file) = COUNTRIES.iter().find(|c| c.1 == r.code).unwrap();
        let star = if n[&r.code] > 1 { "*" } else { "" };
        s.push_str(&format!(
            "{}\t{}\t{}{}\t{}\t{}\n",
            r.continent, r.path, star, r.code, name, file
        ));
    }
    s
}
